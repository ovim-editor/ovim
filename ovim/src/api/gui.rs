//! The GUI editor conversation, carried over the session API.
//!
//! `POST /v1/gui/command` takes one [`GuiCommand`] and answers with the
//! [`GuiReply`](crate::gui::GuiReply) that command's reply kind promises;
//! `GET /v1/gui/stream` is a Server-Sent Events feed of
//! [`GuiSnapshot`]s. Together they are the whole editor conversation, which is
//! what lets a frontend on another host drive a session over an SSH tunnel.
//!
//! Both routes are mounted inside the router that [`super::security`] wraps, so
//! they carry the same bearer capability and Host guard as every other route.

use crate::gui::server::GuiChannel;
use crate::gui::{GuiCommand, GuiSnapshot, SNAPSHOT_EVENT};
use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json, Response,
    },
    routing::{get, post},
    Json as JsonExtractor, Router,
};
use futures::stream::Stream;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::watch;

/// How often an idle stream emits a keep-alive comment.
///
/// A session can sit untouched for minutes. Without traffic, an SSH tunnel or
/// any other intermediary is free to reap the connection, and the frontend
/// would only discover it on the next edit.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

pub(super) fn router(gui: GuiChannel) -> Router {
    Router::new()
        .route("/gui/command", post(gui_command))
        .route("/gui/stream", get(gui_stream))
        .with_state(gui)
}

/// Handler for POST /v1/gui/command.
///
/// A command that failed *in the editor* still answers 200 with a reply
/// carrying the error, exactly as the in-process transport does; the status
/// codes here describe the conversation, not the command.
async fn gui_command(
    State(gui): State<GuiChannel>,
    JsonExtractor(command): JsonExtractor<GuiCommand>,
) -> Response {
    match gui.send(command).await {
        Ok(Some(reply)) => Json(reply).into_response(),
        // A fire-and-forget command is answered as soon as the editor has been
        // handed it. There is no reply to wait for, so waiting would hang.
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

/// Handler for GET /v1/gui/stream.
async fn gui_stream(
    State(gui): State<GuiChannel>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    Sse::new(snapshot_stream(gui.subscribe()))
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE_INTERVAL))
}

/// Turn the coalescing update channel into a stream of snapshot events.
///
/// The subscription's current value is offered first, so a client joining a
/// session that is already being watched gets a frame immediately instead of
/// waiting for the next edit. When there is no current value -- the session was
/// not projecting anything until this subscriber arrived -- the publisher sees
/// the new receiver on its next tick and the first frame follows from there.
fn snapshot_stream(
    updates: watch::Receiver<Option<GuiSnapshot>>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures::stream::unfold(
        (updates, true),
        |(mut updates, mut offer_current)| async move {
            loop {
                // The current value is taken as-is the first time round and
                // waited for afterwards.
                if !std::mem::take(&mut offer_current) && updates.changed().await.is_err() {
                    // The editor is gone; ending the stream closes the
                    // connection, which is how the client learns.
                    return None;
                }
                let frame = updates.borrow_and_update().clone();
                // An empty channel carries no frame: it starts out empty and is
                // cleared again when the last subscriber leaves.
                if let Some(snapshot) = frame {
                    return Some((snapshot_event(&snapshot), (updates, offer_current)));
                }
            }
        },
    )
}

fn snapshot_event(snapshot: &GuiSnapshot) -> Result<Event, Infallible> {
    // A snapshot is plain data with no map keys and no non-finite floats, so
    // serialization cannot fail. A comment frame is a harmless last resort,
    // which is better than panicking on a live connection.
    Ok(Event::default()
        .event(SNAPSHOT_EVENT)
        .json_data(snapshot)
        .unwrap_or_else(|_| Event::default().comment("snapshot could not be serialized")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::state::ApiState;
    use crate::api::{routes::create_router, security};
    use crate::editor::Editor;
    use crate::gui::server::{gui_channel, GuiServer};
    use crate::gui::GuiReply;
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
        Router,
    };
    use futures::StreamExt;
    use ovim_core::session::SessionCapability;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    const PORT: u16 = 4242;

    /// The real router behind the real security layer, plus the editor end of
    /// the conversation for the test to drive.
    ///
    /// The editor is never spawned onto another task: it embeds a Lua state and
    /// so is deliberately not `Send`, which is also why the real session drives
    /// it from its own event loop rather than from the server's tasks.
    fn secured_app() -> (Router, SessionCapability, GuiServer) {
        let capability = SessionCapability::generate();
        let (channel, server) = gui_channel((80, 24));
        let (tx, _rx) = mpsc::channel(8);
        let app = security::secure_router(
            create_router(ApiState::new(tx), channel),
            security::ApiSecurity::new(capability.clone(), PORT),
        );
        (app, capability, server)
    }

    fn command_request(capability: &SessionCapability, command: &GuiCommand) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/gui/command")
            .header("host", format!("127.0.0.1:{PORT}"))
            .header(
                "authorization",
                format!("Bearer {}", capability.expose_secret()),
            )
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(command).unwrap()))
            .unwrap()
    }

    fn stream_request(capability: &SessionCapability) -> Request<Body> {
        Request::builder()
            .uri("/v1/gui/stream")
            .header("host", format!("127.0.0.1:{PORT}"))
            .header(
                "authorization",
                format!("Bearer {}", capability.expose_secret()),
            )
            .body(Body::empty())
            .unwrap()
    }

    /// Answer one command the way `run_headless_loop` does, reporting whether
    /// the session should keep running.
    async fn serve_one(server: &mut GuiServer, editor: &mut Editor) -> bool {
        let request = server.recv().await.expect("a command should arrive");
        let keep_running = server.handle(request, editor).await;
        server.publish(editor);
        keep_running
    }

    /// Split one Server-Sent Events frame into its event name and its data.
    fn parse_event(frame: &str) -> (Option<String>, String) {
        let mut name = None;
        let mut data = String::new();
        for line in frame.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                name = Some(value.trim_start().to_string());
            } else if let Some(value) = line.strip_prefix("data:") {
                data.push_str(value.trim_start());
            }
        }
        (name, data)
    }

    #[tokio::test]
    async fn a_command_posted_over_the_api_comes_back_as_the_reply_it_declared() {
        let (app, capability, mut server) = secured_app();
        let mut editor = Editor::with_content("fn main() {}\n");
        let command = GuiCommand::EditorCommand {
            command: "set number".to_string(),
        };

        let (response, keep_running) = tokio::join!(
            app.oneshot(command_request(&capability, &command)),
            serve_one(&mut server, &mut editor)
        );

        assert!(keep_running);
        let response = response.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let reply: GuiReply = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply, GuiReply::Unit(Ok(())));
        // The command reached the real editor rather than being acknowledged
        // by the route.
        assert!(editor.options.number);
    }

    #[tokio::test]
    async fn every_route_answer_carries_the_shape_its_command_declared() {
        // The command fixes the reply shape at both ends. A route that guessed
        // instead would still return valid JSON, so the only thing that catches
        // it is comparing what came back against `reply_kind`.
        let (app, capability, mut server) = secured_app();
        let mut editor = Editor::with_content("fn main() {}\n");

        for command in [
            GuiCommand::Snapshot {
                columns: 100,
                rows: 30,
            },
            // The last three answer `Err` -- no vector document, no worktree --
            // but the shape is what is being pinned here, not the outcome.
            GuiCommand::VectorSource,
            GuiCommand::DiffReview { spec: None },
            GuiCommand::DiffFilePatch {
                spec: None,
                path: "src/main.rs".to_string(),
            },
            GuiCommand::SelectTab { index: 0 },
        ] {
            let (response, _) = tokio::join!(
                app.clone().oneshot(command_request(&capability, &command)),
                serve_one(&mut server, &mut editor)
            );

            let response = response.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{command:?}");
            let body = to_bytes(response.into_body(), 8 * 1024 * 1024)
                .await
                .unwrap();
            let reply: GuiReply = serde_json::from_slice(&body).unwrap();
            assert_eq!(reply.kind(), command.reply_kind(), "{command:?}");
        }
    }

    #[tokio::test]
    async fn a_fire_and_forget_command_is_answered_without_waiting_for_a_reply() {
        // `Shutdown` never answers, so a route that waited for one would hang
        // the request until the session died.
        let (app, capability, mut server) = secured_app();
        let mut editor = Editor::with_content("fn main() {}\n");

        let (response, keep_running) = tokio::join!(
            app.oneshot(command_request(&capability, &GuiCommand::Shutdown)),
            serve_one(&mut server, &mut editor)
        );

        assert_eq!(response.unwrap().status(), StatusCode::NO_CONTENT);
        assert!(!keep_running, "shutdown should stop the session");
    }

    #[tokio::test]
    async fn an_unauthenticated_command_never_reaches_the_editor() {
        let (app, _capability, mut server) = secured_app();

        // The app is cloned so the router -- and with it the sending end of the
        // conversation -- outlives the request; otherwise `recv` would return
        // on a closed channel rather than on a dispatched command.
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/gui/command")
                    .header("host", format!("127.0.0.1:{PORT}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&GuiCommand::Shutdown).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), server.recv())
                .await
                .is_err(),
            "a rejected command must not be dispatched"
        );
    }

    #[tokio::test]
    async fn an_unauthenticated_stream_is_refused_before_a_subscription_exists() {
        let (app, _capability, mut server) = secured_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/gui/stream")
                    .header("host", format!("127.0.0.1:{PORT}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        // A rejected request must not have started the session projecting.
        server.publish(&Editor::with_content("fn main() {}\n"));
        assert_eq!(server.frames_built(), 0);
    }

    #[tokio::test]
    async fn a_subscriber_receives_a_snapshot_that_an_unwatched_session_never_builds() {
        let (app, capability, mut server) = secured_app();
        let mut editor = Editor::with_content("fn main() {}\n");

        // Nothing is streaming yet, so ticking the publisher costs nothing.
        editor.set_status_message("saved".to_string());
        editor.mark_dirty();
        server.publish(&editor);
        assert_eq!(server.frames_built(), 0);

        let response = app.oneshot(stream_request(&capability)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );

        // The subscription exists from the moment the handler ran, so the very
        // next publish projects a frame for it even though the editor has not
        // changed since.
        server.publish(&editor);
        assert_eq!(server.frames_built(), 1);

        let mut body = response.into_body().into_data_stream();
        let chunk = tokio::time::timeout(Duration::from_secs(5), body.next())
            .await
            .expect("the first frame should arrive promptly")
            .expect("the stream should not end")
            .unwrap();

        let frame = String::from_utf8(chunk.to_vec()).unwrap();
        let (name, payload) = parse_event(&frame);
        assert_eq!(name.as_deref(), Some(SNAPSHOT_EVENT));
        let snapshot: GuiSnapshot = serde_json::from_str(&payload).unwrap();
        assert_eq!(snapshot.status_message, "saved");
    }

    /// A repository with one committed file and one uncommitted line added.
    fn repository_with_one_change() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let repository = git2::Repository::init(&root).unwrap();
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();

        let mut index = repository.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repository.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = git2::Signature::now("Ovim", "ovim@example.invalid").unwrap();
        repository
            .commit(Some("HEAD"), &signature, &signature, "first", &tree, &[])
            .unwrap();

        std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        (directory, root)
    }

    #[tokio::test]
    async fn the_diff_commands_compute_against_the_workspace_the_editor_is_in() {
        // This is the whole point of moving the diff server-side: the answer
        // describes the repository the editor is sitting in, and the caller
        // never needs to be able to reach it.
        let (_directory, root) = repository_with_one_change();
        let (app, capability, mut server) = secured_app();
        let mut editor = Editor::with_content("");
        editor.set_workspace_root(&root).unwrap();

        let (response, _) = tokio::join!(
            app.clone().oneshot(command_request(
                &capability,
                &GuiCommand::DiffReview { spec: None }
            )),
            serve_one(&mut server, &mut editor)
        );
        let body = to_bytes(response.unwrap().into_body(), 1024 * 1024)
            .await
            .unwrap();
        let GuiReply::DiffReview(review) = serde_json::from_slice(&body).unwrap() else {
            panic!("a DiffReview command must answer with a review");
        };
        let review = review.expect("the workspace is a Git worktree");
        assert_eq!(review.root, root);
        assert_eq!(
            review
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["a.txt"]
        );

        let (response, _) = tokio::join!(
            app.oneshot(command_request(
                &capability,
                &GuiCommand::DiffFilePatch {
                    spec: None,
                    path: "a.txt".to_string(),
                },
            )),
            serve_one(&mut server, &mut editor)
        );
        let body = to_bytes(response.unwrap().into_body(), 1024 * 1024)
            .await
            .unwrap();
        let GuiReply::DiffPatch(patch) = serde_json::from_slice(&body).unwrap() else {
            panic!("a DiffFilePatch command must answer with a patch");
        };
        let patch = patch.expect("a.txt is changed in the worktree");
        assert!(patch.contains("+three"), "{patch}");
    }

    /// Serve a router on a loopback port and report where to reach it.
    async fn serve(app: Router) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        address
    }

    #[tokio::test]
    async fn a_remote_transport_drives_a_real_session_over_a_socket() {
        // The whole conversation end to end: the real routes behind the real
        // security layer, over a real socket, driving a real editor -- and
        // dialled on a port that is not the one the session claims to listen
        // on, which is the shape an SSH tunnel produces.
        let (app, capability, mut server) = secured_app();
        let mut editor = Editor::with_content(
            "fn main() {}
",
        );
        let address = serve(app).await;
        assert_ne!(
            address.port(),
            PORT,
            "the ports must differ to prove anything"
        );

        let transport = crate::gui::RemoteTransport::open(crate::gui::RemoteEndpoint::new(
            address.to_string(),
            PORT,
            capability,
        ))
        .await
        .expect("the session should accept the transport");
        let bridge = crate::gui::GuiBridge::new(std::sync::Arc::new(transport));
        let mut updates = bridge.subscribe();

        // The editor is driven from this task rather than spawned: it embeds a
        // Lua state and so is deliberately not `Send`.
        let (result, _) = tokio::join!(
            bridge.editor_command("set number".to_string()),
            serve_one(&mut server, &mut editor)
        );
        result.expect("the command should have been carried out");
        assert!(editor.options.number, "the command reached the real editor");

        // Publishing happens on the session's tick; the subscription exists
        // from the moment the stream handler ran, so the frame that follows
        // the command travels the SSE feed to this subscriber.
        tokio::time::timeout(Duration::from_secs(5), updates.changed())
            .await
            .expect("a frame should arrive over the stream")
            .unwrap();
        let first = updates.borrow_and_update().clone().unwrap();
        let text: String = first.lines[0]
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect();
        assert_eq!(text, "fn main() {}");

        let (result, _) = tokio::join!(
            bridge.paste("edited".to_string()),
            serve_one(&mut server, &mut editor)
        );
        result.expect("the paste should have been carried out");
        tokio::time::timeout(Duration::from_secs(5), updates.changed())
            .await
            .expect("an edit should produce another frame")
            .unwrap();
        let second = updates.borrow_and_update().clone().unwrap();
        assert!(
            second.revision > first.revision,
            "{} should follow {}",
            second.revision,
            first.revision
        );
    }
}
