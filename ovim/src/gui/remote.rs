//! Driving an editor that runs in another process.
//!
//! [`RemoteTransport`] carries the same conversation as
//! [`LocalTransport`](super::LocalTransport), except that the editor is
//! reached over the session API instead of over a pair of in-process
//! channels: commands go out as `POST /v1/gui/command` and snapshots come
//! back on the `GET /v1/gui/stream` Server-Sent Events feed. Nothing above
//! [`GuiTransport`] changes shape, so every Tauri command keeps working
//! against an editor on another host.
//!
//! Two properties of that API shape this module:
//!
//! * **The session authenticates by Host as well as by capability.**
//!   `ApiSecurity` compares the `Host` header against the port the *server*
//!   listens on. Under `ssh -L` the forwarded local port differs from it, so a
//!   header derived from the URL is refused with `403`. [`RemoteEndpoint`]
//!   therefore keeps the address dialled and the session's own port as two
//!   separate values and always sets `Host` from the latter.
//! * **Status describes the conversation, not the command.** A command that
//!   failed inside the editor still answers `200` with an `Err` reply, so this
//!   transport passes replies through untouched and only invents an error when
//!   the link itself failed.

use super::bridge::{GuiTransport, GuiTransportFuture, EDITOR_STOPPED};
use super::protocol::{GuiCommand, GuiReply, GuiReplyKind, GuiSnapshot, SNAPSHOT_EVENT};
use super::reconnect::{supervise, Failure, GuiConnection, Link};
use super::ssh::RemoteLaunch;
use anyhow::{Context, Result};
use futures::StreamExt;
use ovim_core::session::{SessionCapability, SessionInfo};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, HOST};
use reqwest::StatusCode;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, watch};

/// The link to the editor failed, so the command never reached it.
///
/// [`EDITOR_STOPPED`] and `REPLY_CLOSED` are the local transport's equivalents;
/// these read the same way on purpose, because the wording reaches the user as
/// a Tauri command error and the user should not have to know which transport
/// is underneath.
const EDITOR_UNREACHABLE: &str = "The Ovim editor could not be reached";
/// The editor answered, but not with something this frontend understands.
const REPLY_UNREADABLE: &str = "The Ovim editor answered in a form this frontend cannot read";
/// What a command is answered with while the link is down.
///
/// Not queued for later: see [`RemoteTransport::send`].
pub(super) const EDITOR_DISCONNECTED: &str =
    "The remote Ovim session is not reachable right now, so the input was dropped";
/// A `403` from the Host guard is the one failure whose cause is impossible to
/// guess from the status alone.
const HOST_HINT: &str =
    "the session accepts only its own port in the Host header, not the port dialled";

/// The environment variable that makes a loopback session feel like a distant
/// one.
///
/// A round-trip time in milliseconds, applied as half of it in each direction:
/// a command waits on its way out and its answer waits on the way back, and
/// every snapshot frame waits on the way in. It exists so predictive echo can
/// be argued about with numbers instead of with adjectives -- measuring it
/// otherwise needs two machines and a link whose latency nobody controls.
///
/// Read once. Nothing in a shipped build sets it, and a session that does not
/// set it pays a single relaxed atomic load per request.
pub const LATENCY_VARIABLE: &str = "OVIM_REMOTE_LATENCY_MS";

/// Half the injected round trip, or `None` when the knob is unset.
fn injected_delay() -> Option<Duration> {
    static DELAY: std::sync::OnceLock<Option<Duration>> = std::sync::OnceLock::new();
    *DELAY.get_or_init(|| {
        std::env::var(LATENCY_VARIABLE)
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|milliseconds| *milliseconds > 0)
            .map(|milliseconds| Duration::from_micros(milliseconds * 500))
    })
}

/// Wait out one direction of the injected round trip.
async fn cross_the_wire() {
    if let Some(delay) = injected_delay() {
        tokio::time::sleep(delay).await;
    }
}

/// How long the initial stream handshake may take before startup gives up.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long [`GuiTransport::send_oneway`] blocks a non-runtime caller.
///
/// It runs from the window's exit handler, so a hung link must not keep the
/// process alive noticeably longer than a working one would.
const ONEWAY_TIMEOUT: Duration = Duration::from_secs(2);

/// Where a remote editor is, and what proves the right to talk to it.
///
/// The address dialled and the session's own port are separate fields with no
/// default relationship between them, because under an SSH tunnel they differ
/// and getting that wrong produces a `403` whose cause is invisible.
#[derive(Clone, Debug)]
pub struct RemoteEndpoint {
    /// `host:port` this process connects to.
    address: String,
    /// The port the session believes it is listening on, which is the only
    /// value its Host guard accepts.
    session_port: u16,
    capability: SessionCapability,
}

impl RemoteEndpoint {
    /// Address to dial, the session's own port, and its capability.
    pub fn new(
        address: impl Into<String>,
        session_port: u16,
        capability: SessionCapability,
    ) -> Self {
        Self {
            address: address.into(),
            session_port,
            capability,
        }
    }

    /// Build an endpoint from the descriptor the remote session wrote.
    ///
    /// The descriptor is the authority for both the capability and the port,
    /// so neither is ever typed on a command line. `address` is only the place
    /// to dial: omitted it means the session is on this machine, and under a
    /// forwarded port it is the local end of the tunnel.
    pub fn from_session(session: &SessionInfo, address: Option<&str>) -> Result<Self> {
        anyhow::ensure!(
            session.capability.is_configured(),
            "The remote session descriptor carries no capability, so the API would refuse every request"
        );
        let address = match address {
            Some(address) => parse_address(address)?,
            None => format!("127.0.0.1:{}", session.port),
        };
        Ok(Self::new(address, session.port, session.capability.clone()))
    }

    /// Read a session descriptor from disk and build an endpoint from it.
    ///
    /// A file rather than an argument or an environment variable: the
    /// capability would otherwise be readable in the process table and land in
    /// shell history, and the GUI spawns child processes that would inherit an
    /// exported variable.
    pub fn from_session_file(path: &Path, address: Option<&str>) -> Result<Self> {
        let descriptor = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read the session descriptor {}", path.display()))?;
        let session: SessionInfo = serde_json::from_str(&descriptor)
            .with_context(|| format!("{} is not an Ovim session descriptor", path.display()))?;
        Self::from_session(&session, address)
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}/v1{path}", self.address)
    }

    /// The only Host header the session's guard accepts.
    fn host_header(&self) -> String {
        format!("127.0.0.1:{}", self.session_port)
    }

    fn client(&self) -> Result<reqwest::Client> {
        let mut headers = HeaderMap::new();
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {}", self.capability.expose_secret()))
                .context("The session capability is not usable as an HTTP header")?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        // Pinned here rather than at each call site: an HTTP client fills in
        // `Host` from the URL when the request does not carry one, which is
        // exactly the failure this transport exists to avoid.
        headers.insert(
            HOST,
            HeaderValue::from_str(&self.host_header())
                .context("The session port is not usable in a Host header")?,
        );
        reqwest::Client::builder()
            .default_headers(headers)
            // Only connecting is given a deadline. A request has no overall
            // one because the snapshot stream is a request that stays open for
            // as long as the session does, and an editor can sit untouched for
            // far longer than any timeout worth choosing.
            .connect_timeout(HANDSHAKE_TIMEOUT)
            .build()
            .context("Failed to build the remote session HTTP client")
    }
}

/// Accept `host:port`, or a bare port meaning loopback.
fn parse_address(address: &str) -> Result<String> {
    let address = address.trim();
    if let Ok(port) = address.parse::<u16>() {
        return Ok(format!("127.0.0.1:{port}"));
    }
    let (host, port) = address
        .rsplit_once(':')
        .with_context(|| format!("{address} is not a HOST:PORT address"))?;
    anyhow::ensure!(!host.is_empty(), "{address} has no host");
    port.parse::<u16>()
        .with_context(|| format!("{address} does not end in a port number"))?;
    Ok(address.to_string())
}

/// One reachable set of URLs and the client that speaks to them.
///
/// Replaced wholesale on every reconnection rather than mutated: an SSH
/// forward that is rebuilt lands on a different local port, so the address,
/// the URLs derived from it, and the client that pins the Host header all
/// change together or not at all.
pub(super) struct Wire {
    client: reqwest::Client,
    command_url: String,
    stream_url: String,
}

impl Wire {
    pub(super) fn new(endpoint: &RemoteEndpoint) -> Result<Self> {
        Ok(Self {
            client: endpoint.client()?,
            command_url: endpoint.url("/gui/command"),
            stream_url: endpoint.url("/gui/stream"),
        })
    }
}

/// The transport: a supervised link to a session, and the runtime it runs on.
pub struct RemoteTransport {
    link: Arc<Link>,
    supervisor: tokio::task::JoinHandle<()>,
    /// Owned so that requests never depend on the caller's runtime.
    ///
    /// Tauri polls `send` futures on its own runtime, but `send_oneway` runs
    /// from a plain thread where there is none at all. Owning one runtime
    /// means both paths, and the snapshot stream, use the same reactor.
    /// `Option` only so that [`Drop`] can hand it back without blocking.
    runtime: Option<tokio::runtime::Runtime>,
}

impl RemoteTransport {
    /// Connect from a plain thread, blocking until the session answers.
    ///
    /// Failing here rather than in the window is the point: a wrong
    /// capability, a wrong port, or a tunnel that is not up should read as a
    /// startup error and not as an editor that never draws. Once the first
    /// handshake has worked, no later failure comes back this way -- it goes
    /// to the connection state instead, and the reconnection loop takes over.
    pub fn connect(launch: RemoteLaunch) -> Result<Self> {
        let (transport, ready) = Self::start(launch)?;
        let handshake = transport
            .runtime()
            .block_on(ready)
            .unwrap_or_else(|_| Err(EDITOR_UNREACHABLE.to_string()));
        handshake.map_err(anyhow::Error::msg)?;
        Ok(transport)
    }

    /// Connect from inside a runtime, where [`Self::connect`] would panic.
    pub async fn open(endpoint: RemoteEndpoint) -> Result<Self> {
        Self::open_launch(RemoteLaunch::preconnected(endpoint)).await
    }

    /// [`Self::open`], for a launch that knows how to rebuild its own link.
    pub async fn open_launch(launch: RemoteLaunch) -> Result<Self> {
        let (transport, ready) = Self::start(launch)?;
        // Only a channel is awaited here; the request itself is made by the
        // supervisor on this transport's own runtime, so the socket never
        // outlives the reactor that registered it.
        let handshake = ready
            .await
            .unwrap_or_else(|_| Err(EDITOR_UNREACHABLE.to_string()));
        handshake.map_err(anyhow::Error::msg)?;
        Ok(transport)
    }

    /// Build the transport and start its supervisor, without waiting for it.
    fn start(launch: RemoteLaunch) -> Result<(Self, oneshot::Receiver<Result<(), String>>)> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("ovim-gui-remote")
            .build()
            .context("Failed to create the remote GUI runtime")?;
        let link = Arc::new(Link::new(Wire::new(&launch.endpoint)?, launch.link));
        let (ready_tx, ready_rx) = oneshot::channel();
        let supervisor = runtime.spawn(supervise(Arc::clone(&link), ready_tx));
        Ok((
            Self {
                link,
                supervisor,
                runtime: Some(runtime),
            },
            ready_rx,
        ))
    }

    fn runtime(&self) -> &tokio::runtime::Runtime {
        self.runtime
            .as_ref()
            .expect("the runtime is taken only while dropping the transport")
    }

    /// Ask the supervisor to try again now instead of waiting out its backoff.
    ///
    /// `allow_new_session` is the user answering the one question this code
    /// must not answer for them: the session they were editing has ended, and
    /// a replacement would come up with none of its state.
    pub fn request_reconnect(&self, allow_new_session: bool) -> Result<(), String> {
        self.link.request_reconnect(allow_new_session)
    }

    /// Start the request immediately and hand back where its answer will land.
    ///
    /// Eager like the local transport's send: the caller's first `await` is
    /// not what puts the command on the wire.
    fn dispatch(&self, command: GuiCommand) -> oneshot::Receiver<Result<Option<GuiReply>, String>> {
        let (answer_tx, answer_rx) = oneshot::channel();
        self.link.note_viewport(&command);
        let link = Arc::clone(&self.link);
        self.runtime().spawn(async move {
            let _ = answer_tx.send(post_command(&link.wire(), command).await);
        });
        answer_rx
    }
}

impl GuiTransport for RemoteTransport {
    /// Send one command, or drop it if there is nowhere to send it.
    ///
    /// **Keystrokes typed while the link is down are dropped, never queued.**
    /// Queueing reads as the kinder option and is the dangerous one: in a modal
    /// editor a key means whatever the editor's mode, pending operator, count
    /// and cursor position make it mean at the moment it arrives. Replaying
    /// `dd` against a buffer whose cursor moved -- or whose mode changed,
    /// because the session kept running and an agent or a test runner touched
    /// it -- deletes the wrong line, and the user has no way to know it
    /// happened. Dropping loses only the keys nobody could see the effect of
    /// anyway; the connection indicator says so while it is happening, so the
    /// loss is visible rather than silent.
    ///
    /// Failing here instead of letting the request time out matters for the
    /// same reason: a pile of pending requests released at once on reconnect
    /// would be a replay by another name.
    fn send(
        &self,
        command: GuiCommand,
    ) -> GuiTransportFuture<'_, Result<Option<GuiReply>, String>> {
        if !self.link.is_connected() {
            return Box::pin(std::future::ready(Err(EDITOR_DISCONNECTED.to_string())));
        }
        let answer = self.dispatch(command);
        Box::pin(async move {
            answer
                .await
                // The task that owns the answer is gone, which can only mean
                // the runtime is shutting down under it.
                .unwrap_or_else(|_| Err(EDITOR_UNREACHABLE.to_string()))
        })
    }

    fn send_oneway(&self, command: GuiCommand) -> Result<(), String> {
        if command.reply_kind() != GuiReplyKind::None {
            return Err(format!(
                "A GUI command expecting a {:?} reply cannot be sent one-way",
                command.reply_kind()
            ));
        }
        if !self.link.is_connected() {
            return Err(EDITOR_DISCONNECTED.to_string());
        }
        let answer = self.dispatch(command);
        // The trait method is synchronous and HTTP is not. Blocking the
        // *calling* thread is safe when it is a plain thread -- the request
        // runs on this transport's own runtime, so nothing that has to make
        // progress is being held up -- and that path returns the real outcome.
        if tokio::runtime::Handle::try_current().is_ok() {
            // Inside a runtime, blocking would stall a worker that the request
            // may itself need, so the outcome is logged instead of dropped.
            self.runtime().spawn(async move {
                if let Ok(Err(error)) = answer.await {
                    ovim_core::log_warn!("gui", "One-way GUI command failed: {}", error);
                }
            });
            return Ok(());
        }
        self.runtime().block_on(async move {
            match tokio::time::timeout(ONEWAY_TIMEOUT, answer).await {
                Ok(Ok(result)) => result.map(|_| ()),
                Ok(Err(_)) => Err(EDITOR_UNREACHABLE.to_string()),
                Err(_) => Err(format!("{EDITOR_UNREACHABLE}: it did not answer in time")),
            }
        })
    }

    fn subscribe(&self) -> watch::Receiver<Option<GuiSnapshot>> {
        self.link.subscribe()
    }

    fn connection(&self) -> watch::Receiver<GuiConnection> {
        self.link.watch_connection()
    }

    fn request_reconnect(&self, allow_new_session: bool) -> Result<(), String> {
        RemoteTransport::request_reconnect(self, allow_new_session)
    }

    fn is_remote(&self) -> bool {
        true
    }
}

impl Drop for RemoteTransport {
    fn drop(&mut self) {
        self.supervisor.abort();
        // Dropping a runtime from inside another one panics, and a GUI shell
        // may well be tearing this down from an async context. Handing the
        // runtime its own shutdown avoids waiting for tasks here.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// Post one command and translate the answer.
///
/// `204` is a fire-and-forget command, `200` is a reply the editor produced --
/// including one carrying an editor-level `Err`, which is passed through
/// untouched -- and anything else is a failure of the link rather than of the
/// command.
pub(super) async fn post_command(
    wire: &Wire,
    command: GuiCommand,
) -> Result<Option<GuiReply>, String> {
    cross_the_wire().await;
    let response = wire
        .client
        .post(&wire.command_url)
        .json(&command)
        .send()
        .await
        .map_err(|error| format!("{EDITOR_UNREACHABLE}: {}", terse(&error)))?;
    cross_the_wire().await;
    let status = response.status();
    if status == StatusCode::NO_CONTENT {
        return Ok(None);
    }
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("{EDITOR_UNREACHABLE}: {}", terse(&error)))?;
    if status.is_success() {
        return serde_json::from_slice::<GuiReply>(&body)
            .map(Some)
            .map_err(|error| format!("{REPLY_UNREADABLE}: {error}"));
    }
    Err(request_failure(status, &body))
}

/// Phrase a non-success answer.
fn request_failure(status: StatusCode, body: &[u8]) -> String {
    let reported = error_field(body);
    if status == StatusCode::SERVICE_UNAVAILABLE {
        // The session has already phrased this in the wording the local
        // transport uses, so repeating it verbatim keeps a stopped editor
        // reading the same over either transport.
        return reported.unwrap_or_else(|| EDITOR_STOPPED.to_string());
    }
    let detail = match (status, reported) {
        (StatusCode::FORBIDDEN, _) => format!("it answered {status} -- {HOST_HINT}"),
        (_, Some(reported)) => format!("it answered {status}: {reported}"),
        (_, None) => format!("it answered {status}"),
    };
    format!("{EDITOR_UNREACHABLE}: {detail}")
}

/// The `error` field every rejection in this API carries.
fn error_field(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .as_str()
        .map(str::to_string)
}

/// A network error without the chain of `source` prefixes users cannot act on.
fn terse(error: &reqwest::Error) -> String {
    if error.is_connect() {
        return "the connection was refused".to_string();
    }
    if error.is_timeout() {
        return "the connection timed out".to_string();
    }
    // What a forward dying under an open stream looks like from here. reqwest
    // words it as "error decoding response body", which is true and tells a
    // user nothing about what happened to their laptop's network.
    if error.is_body() || error.is_decode() {
        return "the connection dropped".to_string();
    }
    error.to_string()
}

/// Open the snapshot stream, saying what to do about it if it will not open.
///
/// The status is carried into the failure rather than only its wording,
/// because the four things that can go wrong here want four different answers
/// and only the status tells them apart.
pub(super) async fn connect_stream(wire: &Wire) -> Result<reqwest::Response, Failure> {
    let response =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, wire.client.get(&wire.stream_url).send())
            .await
        {
            Err(_) => {
                return Err(Failure::retryable(format!(
                    "{EDITOR_UNREACHABLE}: it did not answer in time"
                )))
            }
            Ok(Err(error)) => {
                return Err(Failure::retryable(format!(
                    "{EDITOR_UNREACHABLE}: {}",
                    terse(&error)
                )))
            }
            Ok(Ok(response)) => response,
        };
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.bytes().await.unwrap_or_default();
    Err(Failure::from_status(status, request_failure(status, &body)))
}

/// Publish every snapshot frame, returning why the stream ended.
pub(super) async fn pump_snapshots(response: reqwest::Response, link: &Link) -> String {
    let mut frames = response.bytes_stream();
    let mut decoder = SseDecoder::default();
    while let Some(chunk) = frames.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => return format!("the stream failed: {}", terse(&error)),
        };
        for frame in decoder.push(&chunk) {
            if frame.event != SNAPSHOT_EVENT {
                continue;
            }
            match serde_json::from_str::<GuiSnapshot>(&frame.data) {
                Ok(snapshot) => {
                    cross_the_wire().await;
                    link.publish(snapshot);
                }
                Err(error) => {
                    ovim_core::log_warn!("gui", "Unreadable remote snapshot: {}", error);
                }
            }
        }
    }
    "the session closed the snapshot stream".to_string()
}

/// One decoded Server-Sent Events frame.
#[derive(Debug, PartialEq)]
struct SseFrame {
    event: String,
    data: String,
}

/// A Server-Sent Events decoder, fed one network chunk at a time.
///
/// Hand-written rather than pulled from a crate: both ends of this stream are
/// ours, the format in use is three field names and a blank line, and the
/// framing edge cases that matter -- a chunk boundary inside an event,
/// keep-alive comments, multi-line data -- are cheaper to test here than to
/// take on a dependency for.
#[derive(Default)]
struct SseDecoder {
    /// Bytes received but not yet terminated by a newline.
    pending: Vec<u8>,
    event: String,
    data: String,
}

impl SseDecoder {
    /// Feed one chunk, returning every frame it completed.
    fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        self.pending.extend_from_slice(chunk);
        let mut frames = Vec::new();
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=end).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            // A complete line cannot end mid-character, so lossy decoding here
            // cannot mangle a multi-byte character split across two chunks.
            if let Some(frame) = self.line(&String::from_utf8_lossy(&line)) {
                frames.push(frame);
            }
        }
        frames
    }

    /// Apply one complete line, returning a frame if it ended one.
    fn line(&mut self, line: &str) -> Option<SseFrame> {
        if line.is_empty() {
            let data = std::mem::take(&mut self.data);
            let event = std::mem::take(&mut self.event);
            // A blank line after nothing but comments is not an event.
            let mut data = if data.is_empty() { return None } else { data };
            // Every data line contributed a trailing newline; the last one is
            // a separator that was never part of the payload.
            data.pop();
            return Some(SseFrame { event, data });
        }
        if line.starts_with(':') {
            // A comment, which is what a keep-alive is.
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => {
                self.event.clear();
                self.event.push_str(value);
            }
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
            }
            // `id` and `retry` belong to reconnection, which is R6; anything
            // else is a field this version does not know.
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::security::{secure_router, ApiSecurity};
    use crate::editor::Editor;
    use crate::gui::bridge::tests::{every_command_once, exercise_every_helper};
    use crate::gui::protocol::GuiVectorSource;
    use crate::gui::reconnect::ConnectionLoss;
    use crate::gui::{protocol, GuiBridge};
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode as AxumStatus};
    use axum::response::sse::{Event, KeepAlive, Sse};
    use axum::response::{IntoResponse, Json, Response};
    use axum::routing::{get, post};
    use axum::{Json as JsonExtractor, Router};
    use std::convert::Infallible;
    use std::sync::Mutex;
    use tokio::net::TcpListener;

    /// The port the stub session claims to listen on.
    ///
    /// Deliberately not the port it is bound to: that gap is exactly what an
    /// `ssh -L` tunnel produces, and it is what the Host guard trips over.
    const SESSION_PORT: u16 = 4242;

    /// A command whose stub answer is an editor-level failure inside a 200.
    const FAILING_COMMAND: &str = "make the editor refuse";
    /// A command the stub answers as a broken conversation instead.
    const UNAVAILABLE_COMMAND: &str = "make the conversation fail";

    /// A session that answers like the real one without running an editor.
    struct StubSession {
        received: Mutex<Vec<GuiCommand>>,
        hosts: Mutex<Vec<String>>,
        snapshot: GuiSnapshot,
        updates: watch::Sender<Option<GuiSnapshot>>,
        /// Bumped to end every open stream, the way a dropped link would.
        /// A counter rather than a flag so the same session can be cut, come
        /// back, and be cut again.
        epoch: watch::Sender<u64>,
        /// How many streams have been opened, which is how a test sees that a
        /// client really did reattach rather than never having left.
        streams: Mutex<u64>,
        /// What the stream route answers with instead of a stream.
        refuse_stream: Mutex<Option<AxumStatus>>,
        /// The revision the next published frame carries, so a test can make
        /// the session look like a replacement that started counting again.
        revision: Mutex<u64>,
    }

    impl StubSession {
        fn received(&self) -> Vec<GuiCommand> {
            self.received.lock().unwrap().clone()
        }

        fn streams(&self) -> u64 {
            *self.streams.lock().unwrap()
        }

        /// Push a frame to whoever is streaming.
        fn publish(&self, status_message: &str) {
            let mut revision = self.revision.lock().unwrap();
            *revision += 1;
            let mut snapshot = self.snapshot.clone();
            snapshot.revision = *revision;
            snapshot.status_message = status_message.to_string();
            self.updates.send_replace(Some(snapshot));
        }

        /// Start counting frames again from one, as a new session process would.
        fn restart_revisions(&self) {
            *self.revision.lock().unwrap() = 0;
        }

        /// End every open snapshot stream the way a dropped link would.
        fn close_stream(&self) {
            self.epoch.send_modify(|epoch| *epoch += 1);
        }

        /// Answer the stream route with `status` until told otherwise.
        fn refuse_stream(&self, status: Option<AxumStatus>) {
            *self.refuse_stream.lock().unwrap() = status;
        }
    }

    /// A forwarder in front of a stub session, which a test can cut and mend.
    ///
    /// This is the shape of the failure R6 exists for: an `ssh -L` or a `socat`
    /// dies while the session on the far side of it carries on. It accepts
    /// connections either way, so a cut link fails the way a dead forward does
    /// -- the socket closes under the request -- rather than the way a port
    /// nobody has bound does.
    struct Forwarder {
        address: std::net::SocketAddr,
        open: Arc<std::sync::atomic::AtomicBool>,
        live: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    }

    impl Forwarder {
        async fn in_front_of(target: std::net::SocketAddr) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let open = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let live: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> =
                Arc::new(Mutex::new(Vec::new()));
            let accepting = Arc::clone(&open);
            let pumps = Arc::clone(&live);
            tokio::spawn(async move {
                while let Ok((mut client, _)) = listener.accept().await {
                    if !accepting.load(std::sync::atomic::Ordering::SeqCst) {
                        continue;
                    }
                    pumps.lock().unwrap().push(tokio::spawn(async move {
                        if let Ok(mut session) = tokio::net::TcpStream::connect(target).await {
                            let _ = tokio::io::copy_bidirectional(&mut client, &mut session).await;
                        }
                    }));
                }
            });
            Self {
                address,
                open,
                live,
            }
        }

        /// Kill the forward: existing connections die and new ones go nowhere.
        fn cut(&self) {
            self.open.store(false, std::sync::atomic::Ordering::SeqCst);
            for pump in self.live.lock().unwrap().drain(..) {
                pump.abort();
            }
        }

        /// Bring the forward back.
        fn mend(&self) {
            self.open.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    async fn stub_command(
        State(stub): State<Arc<StubSession>>,
        headers: HeaderMap,
        JsonExtractor(command): JsonExtractor<GuiCommand>,
    ) -> Response {
        stub.hosts.lock().unwrap().push(
            headers
                .get(HOST)
                .and_then(|host| host.to_str().ok())
                .unwrap_or_default()
                .to_string(),
        );
        let kind = command.reply_kind();
        let scripted = match &command {
            GuiCommand::EditorCommand { command } => command.clone(),
            _ => String::new(),
        };
        stub.received.lock().unwrap().push(command);
        if scripted == UNAVAILABLE_COMMAND {
            return (
                AxumStatus::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "error": EDITOR_STOPPED })),
            )
                .into_response();
        }
        if scripted == FAILING_COMMAND {
            return Json(GuiReply::Unit(Err(
                "Not an editor command: nope".to_string()
            )))
            .into_response();
        }
        match kind {
            GuiReplyKind::None => AxumStatus::NO_CONTENT.into_response(),
            GuiReplyKind::Unit => Json(GuiReply::Unit(Ok(()))).into_response(),
            GuiReplyKind::Snapshot => {
                Json(GuiReply::Snapshot(Box::new(Ok(stub.snapshot.clone())))).into_response()
            }
            GuiReplyKind::VectorSource => Json(GuiReply::VectorSource(Ok(GuiVectorSource {
                source: "documentsize 24x24\n".to_string(),
                file_name: "close.strok".to_string(),
            })))
            .into_response(),
            GuiReplyKind::DiffReview => {
                Json(GuiReply::DiffReview(Ok(protocol::sample_diff_review()))).into_response()
            }
            GuiReplyKind::DiffPatch => Json(GuiReply::DiffPatch(Ok(
                "@@ -1 +1 @@\n-old\n+new\n".to_string()
            )))
            .into_response(),
        }
    }

    async fn stub_stream(State(stub): State<Arc<StubSession>>) -> Response {
        if let Some(status) = *stub.refuse_stream.lock().unwrap() {
            return (status, Json(serde_json::json!({ "error": EDITOR_STOPPED }))).into_response();
        }
        *stub.streams.lock().unwrap() += 1;
        let updates = stub.updates.subscribe();
        let mut epochs = stub.epoch.subscribe();
        let opened = *epochs.borrow_and_update();
        let frames = futures::stream::unfold(
            (updates, epochs),
            move |(mut updates, mut epochs)| async move {
                loop {
                    if *epochs.borrow_and_update() != opened {
                        return None;
                    }
                    tokio::select! {
                        _ = epochs.changed() => return None,
                        changed = updates.changed() => changed.ok()?,
                    }
                    let frame = updates.borrow_and_update().clone();
                    if let Some(snapshot) = frame {
                        let event = Event::default()
                            .event(SNAPSHOT_EVENT)
                            .json_data(&snapshot)
                            .expect("a snapshot serializes");
                        return Some((Ok::<Event, Infallible>(event), (updates, epochs)));
                    }
                }
            },
        );
        Sse::new(frames)
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
            .into_response()
    }

    /// Serve a stub session on a loopback port, behind the real security layer.
    ///
    /// The security layer is the real one on purpose: the Host guard is the
    /// single most confusing way a remote transport can fail, and a stub guard
    /// would only prove that the test agrees with itself.
    async fn stub_session() -> (Arc<StubSession>, RemoteEndpoint) {
        let (stub, address, capability) = serve_stub().await;
        (
            stub,
            RemoteEndpoint::new(address.to_string(), SESSION_PORT, capability),
        )
    }

    /// The same stub, reached through a forwarder a test can cut.
    async fn forwarded_stub_session() -> (Arc<StubSession>, Forwarder, RemoteEndpoint) {
        let (stub, address, capability) = serve_stub().await;
        let forwarder = Forwarder::in_front_of(address).await;
        let endpoint = RemoteEndpoint::new(forwarder.address.to_string(), SESSION_PORT, capability);
        (stub, forwarder, endpoint)
    }

    async fn serve_stub() -> (Arc<StubSession>, std::net::SocketAddr, SessionCapability) {
        let capability = SessionCapability::generate();
        let stub = Arc::new(StubSession {
            received: Mutex::new(Vec::new()),
            hosts: Mutex::new(Vec::new()),
            snapshot: super::super::snapshot(&Editor::with_content("fn main() {}\n"), 1, 0),
            updates: watch::channel(None).0,
            epoch: watch::channel(0).0,
            streams: Mutex::new(0),
            refuse_stream: Mutex::new(None),
            revision: Mutex::new(0),
        });
        let routes = Router::new()
            .route("/gui/command", post(stub_command))
            .route("/gui/stream", get(stub_stream))
            .with_state(Arc::clone(&stub));
        let app = secure_router(
            Router::new().nest("/v1", routes),
            ApiSecurity::new(capability.clone(), SESSION_PORT),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (stub, address, capability)
    }

    /// The next frame a subscriber is handed, with a deadline.
    async fn next_frame(
        updates: &mut watch::Receiver<Option<GuiSnapshot>>,
        within: Duration,
    ) -> GuiSnapshot {
        tokio::time::timeout(within, updates.changed())
            .await
            .expect("a frame should arrive")
            .expect("the subscription should stay open");
        updates
            .borrow_and_update()
            .clone()
            .expect("a published frame is never empty")
    }

    /// Wait for the connection state to satisfy `ready`, and return it.
    async fn connection_reaches(
        connection: &mut watch::Receiver<GuiConnection>,
        ready: impl Fn(&GuiConnection) -> bool,
    ) -> GuiConnection {
        for _ in 0..400 {
            let state = connection.borrow_and_update().clone();
            if ready(&state) {
                return state;
            }
            let _ = tokio::time::timeout(Duration::from_millis(50), connection.changed()).await;
        }
        panic!(
            "the connection never reached the expected state (last: {:?})",
            connection.borrow().clone()
        );
    }

    /// Wait for a condition the stub reaches on another task.
    async fn eventually(mut ready: impl FnMut() -> bool) {
        for _ in 0..200 {
            if ready() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the stub session never reached the expected state");
    }

    #[tokio::test]
    async fn a_command_comes_back_as_the_reply_shape_it_declared() {
        let (stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();

        for command in [
            GuiCommand::Snapshot {
                columns: 120,
                rows: 40,
            },
            GuiCommand::VectorSource,
            GuiCommand::DiffReview { spec: None },
            GuiCommand::DiffFilePatch {
                spec: None,
                path: "src/main.rs".to_string(),
            },
            GuiCommand::SelectTab { index: 3 },
        ] {
            let reply = transport
                .send(command.clone())
                .await
                .unwrap_or_else(|error| panic!("{command:?}: {error}"));
            assert_eq!(
                reply.as_ref().map(GuiReply::kind),
                Some(command.reply_kind()),
                "{command:?}"
            );
        }
        assert_eq!(stub.received().len(), 5);
    }

    #[tokio::test]
    async fn an_editor_level_failure_arrives_as_the_error_the_editor_wrote() {
        // The session answers 200 for a command the editor refused, because
        // the status describes the conversation and not the command. A
        // transport that read the status instead would turn a typo in an ex
        // command into a connection error.
        let (_stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let bridge = GuiBridge::new(Arc::new(transport));

        let error = bridge
            .editor_command(FAILING_COMMAND.to_string())
            .await
            .unwrap_err();

        assert_eq!(error, "Not an editor command: nope");
    }

    #[tokio::test]
    async fn a_fire_and_forget_command_is_answered_with_no_reply_at_all() {
        let (stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();

        let reply = transport.send(GuiCommand::Shutdown).await.unwrap();

        assert_eq!(reply, None);
        assert_eq!(stub.received(), vec![GuiCommand::Shutdown]);
    }

    #[tokio::test]
    async fn a_broken_conversation_reads_exactly_as_a_stopped_local_editor_does() {
        // The wording reaches the user as a Tauri command error, and the user
        // should not have to know which transport is underneath.
        let (_stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let bridge = GuiBridge::new(Arc::new(transport));

        let error = bridge
            .editor_command(UNAVAILABLE_COMMAND.to_string())
            .await
            .unwrap_err();

        assert_eq!(error, EDITOR_STOPPED);
    }

    #[tokio::test]
    async fn a_session_that_is_not_listening_is_reported_at_startup() {
        // Binding and dropping a listener yields a port nothing answers on.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let endpoint = RemoteEndpoint::new(
            address.to_string(),
            SESSION_PORT,
            SessionCapability::generate(),
        );

        let error = RemoteTransport::open(endpoint)
            .await
            .err()
            .expect("a port nothing listens on cannot be connected to")
            .to_string();

        assert!(error.starts_with(EDITOR_UNREACHABLE), "{error}");
    }

    #[tokio::test]
    async fn every_typed_helper_reaches_the_session_as_the_command_it_is_named_for() {
        // The same sweep the local transport is held to, run over a socket:
        // anything the wire format loses or reorders shows up as a difference
        // against the protocol's own list of commands.
        let (stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let bridge = GuiBridge::new(Arc::new(transport));

        exercise_every_helper(&bridge).await;

        // The sweep ends with a one-way shutdown, which by definition is not
        // waited for.
        let expected = every_command_once();
        let arrived = Arc::clone(&stub);
        eventually(|| arrived.received().len() == expected.len()).await;
        assert_eq!(stub.received(), expected);
    }

    #[tokio::test]
    async fn a_command_that_expects_an_answer_cannot_be_sent_one_way() {
        // Same contract as the local transport: a one-way send has nowhere to
        // put an answer, so a command that has one must be refused rather than
        // have it dropped in silence.
        let (stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();

        let error = transport
            .send_oneway(GuiCommand::SelectTab { index: 1 })
            .unwrap_err();

        assert!(error.contains("Unit"), "{error}");
        assert!(stub.received().is_empty(), "nothing should have been sent");
    }

    #[tokio::test]
    async fn the_host_header_names_the_port_the_session_listens_on_not_the_one_dialled() {
        // The single most likely thing to regress here, and its failure mode
        // is a 403 that says nothing about ports. The stub is bound to an
        // ephemeral port and only claims SESSION_PORT, so a Host derived from
        // the URL would never have got past the guard at all.
        let (stub, endpoint) = stub_session().await;
        assert_ne!(
            endpoint.address,
            format!("127.0.0.1:{SESSION_PORT}"),
            "the test only means something while the two ports differ"
        );
        let transport = RemoteTransport::open(endpoint).await.unwrap();

        transport
            .send(GuiCommand::SelectTab { index: 0 })
            .await
            .unwrap();

        assert_eq!(
            stub.hosts.lock().unwrap().clone(),
            vec![format!("127.0.0.1:{SESSION_PORT}")]
        );
    }

    #[tokio::test]
    async fn a_transport_that_names_the_wrong_port_is_refused_and_says_why() {
        let (_stub, endpoint) = stub_session().await;
        let mistaken = RemoteEndpoint::new(
            endpoint.address.clone(),
            SESSION_PORT + 1,
            endpoint.capability.clone(),
        );

        let error = RemoteTransport::open(mistaken)
            .await
            .err()
            .expect("the Host guard refuses a port the session does not listen on")
            .to_string();

        assert!(error.contains("403"), "{error}");
        assert!(error.contains("Host header"), "{error}");
    }

    #[tokio::test]
    async fn a_snapshot_published_by_the_session_reaches_a_subscriber() {
        let (stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let mut updates = transport.subscribe();

        stub.publish("saved");

        tokio::time::timeout(Duration::from_secs(5), updates.changed())
            .await
            .expect("a published frame should arrive")
            .unwrap();
        let frame = updates.borrow_and_update().clone().unwrap();
        assert_eq!(frame.status_message, "saved");
        assert_eq!(frame.lines, stub.snapshot.lines);
    }

    #[tokio::test]
    async fn a_stream_that_dies_mid_session_comes_back_with_fresh_frames() {
        // The payoff case, end to end: the forward dies under a session that
        // never notices, and the window carries on against the same session.
        let (stub, forwarder, endpoint) = forwarded_stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let mut updates = transport.subscribe();
        let mut connection = transport.connection();
        stub.publish("before");
        assert_eq!(
            next_frame(&mut updates, Duration::from_secs(5))
                .await
                .status_message,
            "before"
        );

        forwarder.cut();
        connection_reaches(&mut connection, |state| !state.is_connected()).await;
        // The session kept working while nobody could see it, which is the
        // property the whole architecture exists for.
        stub.publish("while the link was down");
        forwarder.mend();

        connection_reaches(&mut connection, GuiConnection::is_connected).await;
        stub.publish("after");
        let mut frame = next_frame(&mut updates, Duration::from_secs(10)).await;
        while frame.status_message != "after" {
            frame = next_frame(&mut updates, Duration::from_secs(10)).await;
        }
        assert!(stub.streams() >= 2, "the client should have reattached");
    }

    #[tokio::test]
    async fn a_reattached_client_is_handed_a_frame_even_though_nothing_changed() {
        // The quiet way to show a stale window: the editor was untouched
        // during the outage, so a session that only publishes on change would
        // publish nothing, and the frontend would keep rendering the frame it
        // had when the link died. The reattach asks for one.
        let (stub, forwarder, endpoint) = forwarded_stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let mut connection = transport.connection();
        // The viewport a reattach replays is the one the frontend last asked
        // for, which it does on subscribe and on every resize.
        transport
            .send(GuiCommand::Snapshot {
                columns: 120,
                rows: 40,
            })
            .await
            .unwrap();
        let mut updates = transport.subscribe();

        forwarder.cut();
        connection_reaches(&mut connection, |state| !state.is_connected()).await;
        forwarder.mend();
        connection_reaches(&mut connection, GuiConnection::is_connected).await;

        let frame = next_frame(&mut updates, Duration::from_secs(10)).await;
        assert_eq!(frame.lines, stub.snapshot.lines);
        assert!(
            stub.received()
                .iter()
                .filter(|command| matches!(
                    command,
                    GuiCommand::Snapshot {
                        columns: 120,
                        rows: 40
                    }
                ))
                .count()
                >= 2,
            "the reattach should have replayed the viewport: {:?}",
            stub.received()
        );
    }

    #[tokio::test]
    async fn revisions_never_go_backwards_across_a_reconnect() {
        // The frontend drops any frame whose revision is below the newest it
        // has seen. A session that was replaced starts counting from one
        // again, and without this the window would look connected and never
        // redraw again.
        let (stub, forwarder, endpoint) = forwarded_stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let mut updates = transport.subscribe();
        let mut connection = transport.connection();
        for _ in 0..5 {
            stub.publish("before");
        }
        let before = next_frame(&mut updates, Duration::from_secs(5))
            .await
            .revision;

        forwarder.cut();
        connection_reaches(&mut connection, |state| !state.is_connected()).await;
        stub.restart_revisions();
        forwarder.mend();
        connection_reaches(&mut connection, GuiConnection::is_connected).await;
        stub.publish("after");

        let after = next_frame(&mut updates, Duration::from_secs(10)).await;
        assert!(
            after.revision > before,
            "revision went from {before} to {}",
            after.revision
        );
    }

    #[tokio::test]
    async fn keystrokes_typed_while_the_link_is_down_are_dropped_rather_than_queued() {
        // Replaying keys against a buffer that moved under them is how a
        // modal editor deletes the wrong line. They are refused at once, and
        // the connection state is what tells the user why.
        let (stub, forwarder, endpoint) = forwarded_stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let mut connection = transport.connection();
        let sent_before = stub.received().len();

        forwarder.cut();
        connection_reaches(&mut connection, |state| !state.is_connected()).await;
        let refusal = transport
            .send(GuiCommand::EditorCommand {
                command: "d".to_string(),
            })
            .await
            .unwrap_err();
        forwarder.mend();
        connection_reaches(&mut connection, GuiConnection::is_connected).await;

        assert_eq!(refusal, EDITOR_DISCONNECTED);
        // And nothing arrives late: the command was never on the wire, so
        // there is nothing for the reconnection to deliver.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !stub.received()[sent_before..]
                .iter()
                .any(|command| matches!(
                    command,
                    GuiCommand::EditorCommand { command } if command == "d"
                )),
            "a dropped command must not arrive later: {:?}",
            stub.received()
        );
    }

    #[tokio::test]
    async fn a_session_that_has_ended_is_reported_instead_of_being_replaced() {
        // The session's own way of saying its editor has stopped. Retrying
        // cannot bring back what it was holding, and starting a replacement
        // would look like a recovery while discarding all of it, so the loop
        // stops and hands the decision over.
        let (stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let mut connection = transport.connection();

        stub.refuse_stream(Some(AxumStatus::SERVICE_UNAVAILABLE));
        stub.close_stream();

        let state = connection_reaches(&mut connection, |state| {
            matches!(state, GuiConnection::Lost { .. })
        })
        .await;
        assert!(
            matches!(
                state,
                GuiConnection::Lost {
                    reason: ConnectionLoss::SessionGone,
                    can_start_a_session: false,
                    ..
                }
            ),
            "{state:?}"
        );
    }

    #[tokio::test]
    async fn credentials_that_stop_working_stop_the_retrying_too() {
        // A capability the session no longer accepts is a setting, not a
        // network condition. Retrying it every fifteen seconds for ten minutes
        // helps nobody, and on a host that counts failures it makes things
        // worse.
        let (stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let mut connection = transport.connection();

        stub.refuse_stream(Some(AxumStatus::UNAUTHORIZED));
        stub.close_stream();

        let state = connection_reaches(&mut connection, |state| {
            matches!(state, GuiConnection::Lost { .. })
        })
        .await;
        assert!(
            matches!(
                state,
                GuiConnection::Lost {
                    reason: ConnectionLoss::Authentication,
                    ..
                }
            ),
            "{state:?}"
        );
    }

    #[tokio::test]
    async fn an_unreachable_session_keeps_being_retried_with_a_growing_wait() {
        // The laptop-lid case: nothing answers, and the only right response is
        // to keep trying for long enough that a lid opened five minutes later
        // finds the window still attached.
        let (_stub, forwarder, endpoint) = forwarded_stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let mut connection = transport.connection();

        forwarder.cut();

        let first = connection_reaches(&mut connection, |state| {
            matches!(state, GuiConnection::Reconnecting { .. })
        })
        .await;
        let later = connection_reaches(
            &mut connection,
            |state| matches!(state, GuiConnection::Reconnecting { attempt, .. } if *attempt >= 3),
        )
        .await;

        let (
            GuiConnection::Reconnecting {
                attempt: first_attempt,
                retry_in_ms: first_wait,
                ..
            },
            GuiConnection::Reconnecting {
                attempt: later_attempt,
                retry_in_ms: later_wait,
                ..
            },
        ) = (&first, &later)
        else {
            panic!("{first:?} / {later:?}")
        };
        assert_eq!(*first_attempt, 1);
        assert!(later_attempt >= &3, "{later:?}");
        assert!(later_wait > first_wait, "{first_wait} then {later_wait}");
    }

    #[tokio::test]
    async fn giving_up_leaves_a_door_open_rather_than_a_dead_window() {
        // Whatever stops the automatic attempts, a manual one must still be
        // possible: a state that can only be left by relaunching throws away
        // the session it was trying to protect.
        let (stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();
        let mut connection = transport.connection();
        stub.refuse_stream(Some(AxumStatus::UNAUTHORIZED));
        stub.close_stream();
        connection_reaches(&mut connection, |state| {
            matches!(state, GuiConnection::Lost { .. })
        })
        .await;

        stub.refuse_stream(None);
        transport.request_reconnect(false).unwrap();

        connection_reaches(&mut connection, GuiConnection::is_connected).await;
    }

    #[tokio::test]
    async fn a_window_that_did_not_start_the_session_cannot_start_a_replacement() {
        // `--remote-session` points at a session somebody else arranged. There
        // is no bootstrap here to run, and pretending otherwise would end in a
        // window that says it recovered and did not.
        let (_stub, endpoint) = stub_session().await;
        let transport = RemoteTransport::open(endpoint).await.unwrap();

        let refusal = transport.request_reconnect(true).unwrap_err();

        assert!(refusal.contains("--remote"), "{refusal}");
    }

    #[test]
    fn an_address_may_be_a_bare_port_a_host_and_port_or_nothing_usable() {
        assert_eq!(parse_address("9000").unwrap(), "127.0.0.1:9000");
        assert_eq!(parse_address(" 127.0.0.1:9000 ").unwrap(), "127.0.0.1:9000");
        assert_eq!(parse_address("build-host:9000").unwrap(), "build-host:9000");
        assert!(parse_address("build-host").is_err());
        assert!(parse_address("build-host:ssh").is_err());
    }

    #[test]
    fn a_descriptor_without_a_capability_cannot_produce_an_endpoint() {
        // Such a session predates authentication or was hand-written; dialling
        // it could only end in a 401, so it is refused where the file is read.
        let session = SessionInfo {
            capability: SessionCapability::default(),
            ..SessionInfo::new(9000, None, "dev".to_string())
        };

        let error = RemoteEndpoint::from_session(&session, None).unwrap_err();

        assert!(error.to_string().contains("capability"), "{error}");
    }

    #[test]
    fn an_endpoint_defaults_to_the_port_the_session_reported() {
        let session = SessionInfo::new(9000, None, "dev".to_string())
            .with_capability(SessionCapability::generate());

        let endpoint = RemoteEndpoint::from_session(&session, None).unwrap();

        assert_eq!(endpoint.address, "127.0.0.1:9000");
        assert_eq!(endpoint.host_header(), "127.0.0.1:9000");
    }

    #[test]
    fn a_forwarded_port_changes_where_to_dial_but_never_the_host_header() {
        let session = SessionInfo::new(9000, None, "dev".to_string())
            .with_capability(SessionCapability::generate());

        let endpoint = RemoteEndpoint::from_session(&session, Some("41000")).unwrap();

        assert_eq!(endpoint.address, "127.0.0.1:41000");
        assert_eq!(endpoint.host_header(), "127.0.0.1:9000");
    }

    fn decode(decoder: &mut SseDecoder, chunk: &str) -> Vec<SseFrame> {
        decoder.push(chunk.as_bytes())
    }

    #[test]
    fn an_event_split_across_chunks_is_decoded_once_the_blank_line_arrives() {
        let mut decoder = SseDecoder::default();

        assert!(decode(&mut decoder, "event: snap").is_empty());
        assert!(decode(&mut decoder, "shot\ndata: {\"a\"").is_empty());
        // The event is complete only at the blank line, not at the newline
        // that ends its last field.
        assert!(decode(&mut decoder, ":1}\n").is_empty());
        assert_eq!(
            decode(&mut decoder, "\n"),
            vec![SseFrame {
                event: "snapshot".to_string(),
                data: "{\"a\":1}".to_string(),
            }]
        );
    }

    #[test]
    fn keep_alive_comments_and_unknown_fields_are_ignored() {
        let mut decoder = SseDecoder::default();

        // A keep-alive is a comment followed by a blank line, which must not
        // be mistaken for an event with empty data.
        assert!(decode(&mut decoder, ":\n\n").is_empty());
        assert_eq!(
            decode(
                &mut decoder,
                "id: 7\nretry: 500\nevent: snapshot\ndata: x\n\n"
            ),
            vec![SseFrame {
                event: "snapshot".to_string(),
                data: "x".to_string(),
            }]
        );
    }

    #[test]
    fn a_multi_line_data_field_is_rejoined_with_the_newlines_it_encoded() {
        let mut decoder = SseDecoder::default();

        // Carriage returns are line terminators here, not payload, and two
        // events in one chunk are two events.
        let frames = decode(
            &mut decoder,
            "event: snapshot\r\ndata: one\r\ndata: two\r\n\r\nevent: snapshot\ndata: three\n\n",
        );

        assert_eq!(
            frames,
            vec![
                SseFrame {
                    event: "snapshot".to_string(),
                    data: "one\ntwo".to_string(),
                },
                SseFrame {
                    event: "snapshot".to_string(),
                    data: "three".to_string(),
                },
            ]
        );
    }
}
