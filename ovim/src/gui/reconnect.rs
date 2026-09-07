//! Surviving a dropped link.
//!
//! A remote Ovim session keeps running when its window goes away. That is the
//! whole reason the editor lives on the far side of the link: warm language
//! servers, undo history, a test run in progress and an AI conversation
//! mid-answer all survive a closed laptop lid, and none of them could survive
//! it if the buffer were local. This module is what turns that property into a
//! feature the user can feel -- the link comes back on its own, attached to the
//! same session, and says so honestly while it is trying.
//!
//! Three decisions shape it.
//!
//! * **Connection state does not ride on the snapshot.** A snapshot only
//!   arrives when the link works, so a field on it could not change at the one
//!   moment it has something to say. It travels on its own channel, and the
//!   frontend renders it separately from the editor's own status line.
//! * **Reconnecting reattaches; it never quietly starts a session.** A
//!   replacement session would come up empty and looking identical, which is
//!   the worst possible outcome: the user carries on typing into an editor
//!   that has silently lost everything they were doing.
//! * **Input during an outage is dropped, not queued.** See
//!   [`RemoteTransport::send`](super::remote::RemoteTransport::send) for why.

use super::protocol::{GuiCommand, GuiReply, GuiSnapshot};
use super::remote::{connect_stream, post_command, pump_snapshots, Wire};
use super::ssh::{LinkFailureKind, RemoteLink};
use rand::Rng;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::{oneshot, watch, Notify};

/// The wait before the first retry.
///
/// Short enough that the common case -- a forward restarted under a session
/// that never noticed -- is over before the user has finished reading the
/// indicator.
const BACKOFF_BASE: Duration = Duration::from_millis(500);
/// The longest wait between two attempts.
///
/// A ceiling rather than unbounded doubling: the thing being waited for is
/// usually a laptop waking up or a wifi network reappearing, and both of those
/// happen at a moment unrelated to how long the wait has already been.
const BACKOFF_CEILING: Duration = Duration::from_secs(15);
/// How long the automatic attempts go on before the user is asked.
///
/// A closed lid is minutes, not seconds. Ten of them is long enough to cover a
/// meeting, a train tunnel or a hotel network coming back, and short enough
/// that an indicator saying "reconnecting" is not left lying for an hour.
const RETRY_BUDGET: Duration = Duration::from_secs(600);
/// The fraction of each wait that is randomised.
///
/// Two windows on the same host that lost the same wifi would otherwise wake
/// up in lockstep and reconnect in lockstep for as long as the outage lasts.
const BACKOFF_JITTER: f64 = 0.25;

/// What the frontend is told about the link to the editor.
///
/// Deliberately more than a boolean. "Not connected" covers a hiccup that will
/// be over before the sentence is read and a session that has ended and taken
/// an afternoon's undo history with it, and a user who cannot tell those apart
/// cannot decide what to do about either.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum GuiConnection {
    /// The editor is answering.
    Connected,
    /// The link dropped and is being rebuilt.
    #[serde(rename_all = "camelCase")]
    Reconnecting {
        /// Which attempt is about to be made, counting from one.
        attempt: u32,
        /// How long until it is made.
        retry_in_ms: u64,
        /// What went wrong last time, in the words the user would get anyway.
        detail: String,
    },
    /// Nothing automatic is being tried any more. Always recoverable by hand:
    /// a dead end that can only be left by relaunching is a dead end that
    /// throws away the session it was trying to protect.
    #[serde(rename_all = "camelCase")]
    Lost {
        reason: ConnectionLoss,
        detail: String,
        /// Whether a new session could be started from here, which is only
        /// true when this process owns the SSH link.
        can_start_a_session: bool,
    },
}

impl GuiConnection {
    pub fn is_connected(&self) -> bool {
        matches!(self, GuiConnection::Connected)
    }
}

/// Why the automatic attempts stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionLoss {
    /// The retry budget ran out. Trying again may still work; the network just
    /// did not come back inside the window this code was willing to wait.
    GaveUp,
    /// The remote session is not there any more. Its state went with it, so
    /// whether to start a replacement is the user's decision.
    SessionGone,
    /// The far side refused the credentials, or its identity did not check
    /// out. Repeating the attempt only repeats the refusal.
    Authentication,
    /// Something about this pair of machines has to change first.
    Unusable,
}

/// What a failed attempt says about whether another one could work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Recovery {
    /// Wait and try again.
    Retry,
    /// Stop, for this reason.
    Stop(ConnectionLoss),
}

/// A failed attempt, kept with what to do about it.
#[derive(Clone, Debug)]
pub(super) struct Failure {
    pub(super) recovery: Recovery,
    pub(super) detail: String,
}

impl Failure {
    pub(super) fn retryable(detail: impl Into<String>) -> Self {
        Self {
            recovery: Recovery::Retry,
            detail: detail.into(),
        }
    }

    pub(super) fn stop(reason: ConnectionLoss, detail: impl Into<String>) -> Self {
        Self {
            recovery: Recovery::Stop(reason),
            detail: detail.into(),
        }
    }

    /// Read a rejected request for what it says about trying again.
    ///
    /// The session answers `503` when its editor has stopped, which is the
    /// only way a client with no tunnel of its own can learn that the far side
    /// is gone rather than merely unreachable. `401` and `403` are the bearer
    /// capability and the Host guard: both are settings, and no amount of
    /// waiting changes a setting.
    pub(super) fn from_status(status: StatusCode, detail: impl Into<String>) -> Self {
        let recovery = match status {
            StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::PROXY_AUTHENTICATION_REQUIRED => {
                Recovery::Stop(ConnectionLoss::Authentication)
            }
            StatusCode::SERVICE_UNAVAILABLE => Recovery::Stop(ConnectionLoss::SessionGone),
            // A route that is not there means the far side is an Ovim that
            // does not serve this protocol, which is a version problem.
            StatusCode::NOT_FOUND => Recovery::Stop(ConnectionLoss::Unusable),
            // Anything else in the 500s is the session having a bad moment,
            // and a proxy in the way can produce almost any 4xx.
            status if status.is_server_error() => Recovery::Retry,
            _ => Recovery::Stop(ConnectionLoss::Unusable),
        };
        Self {
            recovery,
            detail: detail.into(),
        }
    }

    /// Read a failure to rebuild the link the same way.
    fn from_link(kind: LinkFailureKind, detail: impl Into<String>) -> Self {
        let recovery = match kind {
            LinkFailureKind::Unreachable => Recovery::Retry,
            LinkFailureKind::Authentication => Recovery::Stop(ConnectionLoss::Authentication),
            LinkFailureKind::SessionGone => Recovery::Stop(ConnectionLoss::SessionGone),
            LinkFailureKind::Unusable => Recovery::Stop(ConnectionLoss::Unusable),
        };
        Self {
            recovery,
            detail: detail.into(),
        }
    }
}

/// Capped exponential backoff with jitter, bounded by a total budget.
#[derive(Debug)]
pub(super) struct Backoff {
    attempt: u32,
    spent: Duration,
}

impl Backoff {
    pub(super) fn new() -> Self {
        Self {
            attempt: 0,
            spent: Duration::ZERO,
        }
    }

    /// The unjittered wait before attempt `attempt`, counting from one.
    ///
    /// Separated from the randomisation so the shape of the curve can be
    /// asserted on rather than sampled.
    pub(super) fn planned(attempt: u32) -> Duration {
        let doublings = attempt.saturating_sub(1).min(16);
        BACKOFF_BASE
            .saturating_mul(1u32 << doublings)
            .min(BACKOFF_CEILING)
    }

    /// The next attempt number and how long to wait for it, or `None` once the
    /// budget is spent.
    pub(super) fn next(&mut self) -> Option<(u32, Duration)> {
        if self.spent >= RETRY_BUDGET {
            return None;
        }
        self.attempt += 1;
        let planned = Self::planned(self.attempt);
        self.spent += planned;
        Some((self.attempt, jitter(planned)))
    }
}

/// Spread a wait by up to [`BACKOFF_JITTER`] either way.
fn jitter(delay: Duration) -> Duration {
    let factor = rand::thread_rng().gen_range(1.0 - BACKOFF_JITTER..=1.0 + BACKOFF_JITTER);
    delay.mul_f64(factor)
}

/// Everything the transport and its supervisor share.
pub(super) struct Link {
    /// Replaced whenever the endpoint changes, read by every request.
    wire: RwLock<Arc<Wire>>,
    connection: watch::Sender<GuiConnection>,
    updates: watch::Sender<Option<GuiSnapshot>>,
    /// The highest revision handed to a subscriber. See [`Link::publish`].
    revision_floor: Mutex<u64>,
    /// The viewport the frontend last asked for, replayed on reattach.
    viewport: Mutex<Option<(u16, u16)>>,
    source: Arc<dyn RemoteLink>,
    /// A manual retry request, and whether it may start a new session.
    retry: Notify,
    retry_fresh: AtomicBool,
}

impl Link {
    pub(super) fn new(wire: Wire, source: Arc<dyn RemoteLink>) -> Self {
        Self {
            wire: RwLock::new(Arc::new(wire)),
            connection: watch::channel(GuiConnection::Connected).0,
            updates: watch::channel(None).0,
            revision_floor: Mutex::new(0),
            viewport: Mutex::new(None),
            source,
            retry: Notify::new(),
            retry_fresh: AtomicBool::new(false),
        }
    }

    pub(super) fn wire(&self) -> Arc<Wire> {
        Arc::clone(&self.wire.read().expect("the wire lock is never poisoned"))
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<Option<GuiSnapshot>> {
        self.updates.subscribe()
    }

    pub(super) fn watch_connection(&self) -> watch::Receiver<GuiConnection> {
        self.connection.subscribe()
    }

    pub(super) fn is_connected(&self) -> bool {
        self.connection.borrow().is_connected()
    }

    fn set_connection(&self, state: GuiConnection) {
        self.connection.send_replace(state);
    }

    /// Remember the viewport a `Snapshot` command carried.
    ///
    /// The frontend sends one when it subscribes and on every resize, so this
    /// is always the size the window last believed it had -- which is what a
    /// reattach has to ask for, because the window may well have been resized
    /// while the link was down.
    pub(super) fn note_viewport(&self, command: &GuiCommand) {
        if let GuiCommand::Snapshot { columns, rows } = command {
            *self
                .viewport
                .lock()
                .expect("the viewport lock is never poisoned") = Some((*columns, *rows));
        }
    }

    /// Hand a frame to subscribers, never letting the revision go backwards.
    ///
    /// `shouldAcceptRevision` in the frontend drops any frame whose revision is
    /// below the newest it has seen. Within one session that filter is exactly
    /// right and never fires, because the session's counter only ever rises.
    /// Across a reconnect it becomes a hazard: a session that was replaced
    /// starts counting from one again, and every frame it sends would be
    /// silently discarded, leaving a window that looks connected and never
    /// redraws. The number carries no meaning beyond ordering, so raising it
    /// here preserves what the frontend relies on. Nothing is rewritten while
    /// the far side is the monotonic one.
    pub(super) fn publish(&self, mut snapshot: GuiSnapshot) {
        let mut floor = self
            .revision_floor
            .lock()
            .expect("the revision lock is never poisoned");
        if snapshot.revision <= *floor {
            snapshot.revision = floor.wrapping_add(1);
        }
        *floor = snapshot.revision;
        // `send_replace` rather than `send`: a frame is worth keeping even
        // while no part of the frontend is listening yet.
        self.updates.send_replace(Some(snapshot));
    }

    /// Ask the supervisor to try now rather than wait out its backoff.
    pub(super) fn request_reconnect(&self, allow_new_session: bool) -> Result<(), String> {
        if allow_new_session && !self.source.can_start_a_session() {
            return Err(
                "This window did not bring the remote session up, so it cannot start a new one. \
                 Launch Ovim again with --remote to start one."
                    .to_string(),
            );
        }
        if allow_new_session {
            self.retry_fresh.store(true, Ordering::SeqCst);
        }
        self.retry.notify_one();
        Ok(())
    }
}

/// Keep the link to the session up for as long as this transport lives.
///
/// The first attempt is startup and reports itself through `ready`, because a
/// window that cannot connect at all should fail as a launch error rather than
/// open and sit in a reconnection loop. Every attempt after that speaks only
/// through the connection state.
pub(super) async fn supervise(link: Arc<Link>, ready: oneshot::Sender<Result<(), String>>) {
    let mut ready = Some(ready);
    let mut backoff = Backoff::new();
    let mut first = true;
    loop {
        let failure = match attach(&link, &mut ready, std::mem::take(&mut first)).await {
            Ok(reason) => {
                // The link worked and then stopped. The next outage starts
                // from a full budget: it is a new outage, not a continuation
                // of whatever came before it.
                backoff = Backoff::new();
                ovim_core::log_warn!("gui", "Remote snapshot stream ended: {}", reason);
                Failure::retryable(reason)
            }
            Err(failure) => failure,
        };
        if let Some(ready) = ready.take() {
            // Startup never had a working link, so the launch reports this and
            // there is no window for a reconnection loop to speak to.
            let _ = ready.send(Err(failure.detail));
            return;
        }
        hold(&link, failure, &mut backoff).await;
    }
}

/// Rebuild the link if needed, then pump snapshots until the stream ends.
///
/// Returns why the stream ended, or the failure that stopped it starting.
async fn attach(
    link: &Arc<Link>,
    ready: &mut Option<oneshot::Sender<Result<(), String>>>,
    first: bool,
) -> Result<String, Failure> {
    // The caller already built the first wire; every later attempt has to
    // rebuild whatever was carrying it.
    if !first {
        rebuild(link).await?;
    }
    let response = connect_stream(&link.wire()).await?;
    if let Some(ready) = ready.take() {
        let _ = ready.send(Ok(()));
    }
    link.set_connection(GuiConnection::Connected);
    // Asked for as the stream comes up rather than after it: the session only
    // publishes when its projection changes, so a buffer nobody touched during
    // the outage would produce no frame at all and the window would keep
    // showing the one it had when the link died. Not on the first attach --
    // there is no frame to be stale yet, and the frontend asks for one itself
    // as it subscribes.
    let refresh = (!first).then(|| tokio::spawn(force_snapshot(Arc::clone(link))));
    let reason = pump_snapshots(response, link).await;
    if let Some(refresh) = refresh {
        refresh.abort();
    }
    Ok(reason)
}

/// Re-establish whatever carries the link and point the wire at it.
async fn rebuild(link: &Arc<Link>) -> Result<(), Failure> {
    let source = Arc::clone(&link.source);
    // Only ever true because a user asked, and consumed here so that one
    // answer does not apply to every later attempt.
    let allow_new_session = link.retry_fresh.swap(false, Ordering::SeqCst);
    // Rebuilding an SSH tunnel spawns processes and waits on them, which must
    // not happen on a runtime worker.
    let endpoint = tokio::task::spawn_blocking(move || source.reconnect(allow_new_session))
        .await
        .map_err(|_| {
            Failure::stop(
                ConnectionLoss::Unusable,
                "The reconnection worker stopped before it could rebuild the link",
            )
        })?
        .map_err(|failure| Failure::from_link(failure.kind, failure.error.to_string()))?;
    let wire = Wire::new(&endpoint)
        .map_err(|error| Failure::stop(ConnectionLoss::Unusable, error.to_string()))?;
    *link.wire.write().expect("the wire lock is never poisoned") = Arc::new(wire);
    Ok(())
}

/// Ask for a frame now that the stream is back, and publish it.
async fn force_snapshot(link: Arc<Link>) {
    let viewport = *link
        .viewport
        .lock()
        .expect("the viewport lock is never poisoned");
    let Some((columns, rows)) = viewport else {
        // Nothing has ever asked for a size, so this is the first attach and
        // the stream's own first frame is already the fresh one.
        return;
    };
    let wire = link.wire();
    match post_command(&wire, GuiCommand::Snapshot { columns, rows }).await {
        Ok(Some(GuiReply::Snapshot(result))) => match *result {
            Ok(snapshot) => link.publish(snapshot),
            Err(error) => {
                ovim_core::log_warn!("gui", "Reattached session refused a snapshot: {}", error)
            }
        },
        Ok(_) => {}
        Err(error) => ovim_core::log_warn!(
            "gui",
            "Reattached session did not answer a snapshot: {}",
            error
        ),
    }
}

/// Publish what is happening and wait until it is worth trying again.
///
/// Returns when the next attempt should be made -- after a backoff, or as soon
/// as a user asks. It never returns "give up": giving up is a state the user
/// can leave, so it parks here until they do. The task itself is aborted when
/// the transport is dropped.
async fn hold(link: &Link, failure: Failure, backoff: &mut Backoff) {
    let wait = match failure.recovery {
        Recovery::Retry => backoff.next(),
        Recovery::Stop(_) => None,
    };
    let Some((attempt, delay)) = wait else {
        let reason = match failure.recovery {
            Recovery::Stop(reason) => reason,
            Recovery::Retry => ConnectionLoss::GaveUp,
        };
        link.set_connection(GuiConnection::Lost {
            reason,
            detail: failure.detail,
            can_start_a_session: link.source.can_start_a_session(),
        });
        link.retry.notified().await;
        *backoff = Backoff::new();
        return;
    };
    link.set_connection(GuiConnection::Reconnecting {
        attempt,
        retry_in_ms: delay.as_millis() as u64,
        detail: failure.detail,
    });
    tokio::select! {
        _ = tokio::time::sleep(delay) => {}
        // A user who asks for a retry should not then wait out the backoff
        // they were watching count down.
        _ = link.retry.notified() => *backoff = Backoff::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_and_then_stops_growing() {
        // Pinned rather than sampled: the point of a cap is that a long outage
        // does not turn into a ten-minute wait between two attempts.
        let plan: Vec<u64> = (1..=8)
            .map(|attempt| Backoff::planned(attempt).as_millis() as u64)
            .collect();

        assert_eq!(plan, vec![500, 1000, 2000, 4000, 8000, 15000, 15000, 15000]);
    }

    #[test]
    fn jitter_moves_a_wait_without_reordering_the_curve() {
        // Jitter exists so two windows on the same lost network do not retry
        // in lockstep, and it must not be large enough to make a late attempt
        // land before an early one.
        for _ in 0..200 {
            let spread = jitter(Duration::from_secs(4));
            assert!(spread >= Duration::from_secs(3), "{spread:?}");
            assert!(spread <= Duration::from_secs(5), "{spread:?}");
        }
    }

    #[test]
    fn retrying_gives_up_only_after_minutes_of_trying() {
        // A closed laptop lid is minutes. A backoff that gave up in seconds
        // would turn every one of them into a relaunch.
        let mut backoff = Backoff::new();
        let mut attempts = 0;
        let mut total = Duration::ZERO;
        while let Some((attempt, _)) = backoff.next() {
            attempts = attempt;
            total += Backoff::planned(attempt);
            assert!(attempts < 1000, "the budget must actually run out");
        }

        assert!(total >= RETRY_BUDGET, "{total:?}");
        assert!(attempts > 30, "gave up after only {attempts} attempts");
        // And the budget is spent, so the next call still says stop.
        assert_eq!(backoff.next(), None);
    }

    #[test]
    fn a_manual_retry_starts_the_budget_over() {
        let mut backoff = Backoff::new();
        while backoff.next().is_some() {}

        backoff = Backoff::new();

        assert_eq!(backoff.next().map(|(attempt, _)| attempt), Some(1));
    }

    #[test]
    fn each_way_of_being_refused_routes_to_the_answer_that_fits_it() {
        // The four classes the feature promises to tell apart, at the one
        // place a status code is turned into a decision.
        let cases = [
            (
                StatusCode::UNAUTHORIZED,
                Recovery::Stop(ConnectionLoss::Authentication),
            ),
            (
                StatusCode::FORBIDDEN,
                Recovery::Stop(ConnectionLoss::Authentication),
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Recovery::Stop(ConnectionLoss::SessionGone),
            ),
            (
                StatusCode::NOT_FOUND,
                Recovery::Stop(ConnectionLoss::Unusable),
            ),
            (StatusCode::BAD_GATEWAY, Recovery::Retry),
            (StatusCode::INTERNAL_SERVER_ERROR, Recovery::Retry),
        ];

        for (status, expected) in cases {
            assert_eq!(
                Failure::from_status(status, "detail").recovery,
                expected,
                "{status}"
            );
        }
    }

    #[test]
    fn a_link_that_cannot_be_rebuilt_says_whether_waiting_would_help() {
        let cases = [
            (LinkFailureKind::Unreachable, Recovery::Retry),
            (
                LinkFailureKind::Authentication,
                Recovery::Stop(ConnectionLoss::Authentication),
            ),
            (
                LinkFailureKind::SessionGone,
                Recovery::Stop(ConnectionLoss::SessionGone),
            ),
            (
                LinkFailureKind::Unusable,
                Recovery::Stop(ConnectionLoss::Unusable),
            ),
        ];

        for (kind, expected) in cases {
            assert_eq!(
                Failure::from_link(kind, "detail").recovery,
                expected,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn connection_state_is_serialized_in_the_shape_the_frontend_reads() {
        // The webview receives this over Tauri's IPC channel, so the tag and
        // the field names are part of the contract with `connection.ts`.
        let reconnecting = serde_json::to_value(GuiConnection::Reconnecting {
            attempt: 3,
            retry_in_ms: 2000,
            detail: "the stream failed".to_string(),
        })
        .unwrap();

        assert_eq!(
            reconnecting,
            serde_json::json!({
                "state": "reconnecting",
                "attempt": 3,
                "retryInMs": 2000,
                "detail": "the stream failed",
            })
        );
        assert_eq!(
            serde_json::to_value(GuiConnection::Lost {
                reason: ConnectionLoss::SessionGone,
                detail: "it ended".to_string(),
                can_start_a_session: true,
            })
            .unwrap(),
            serde_json::json!({
                "state": "lost",
                "reason": "sessionGone",
                "detail": "it ended",
                "canStartASession": true,
            })
        );
        assert_eq!(
            serde_json::to_value(GuiConnection::Connected).unwrap(),
            serde_json::json!({ "state": "connected" })
        );
    }
}
