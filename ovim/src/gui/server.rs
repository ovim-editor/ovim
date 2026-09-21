//! The editor conversation, served from a headless session.
//!
//! The desktop GUI runs its own editor loop ([`super::run_editor`]) and answers
//! [`GuiRequest`]s from it directly. A headless session already owns an editor
//! for the automation API, so rather than starting a second loop it grows a
//! second conversation: [`GuiServer`] plugs the same request handling and the
//! same snapshot publishing into `run_headless_loop`, and [`GuiChannel`] is the
//! handle the HTTP layer talks to.
//!
//! Nothing here depends on Tauri. That is the point: the host that runs the
//! editor needs no window server, and a frontend on another machine reaches it
//! over the session API instead of over a pair of in-process channels.

use super::bridge::EDITOR_STOPPED;
use super::protocol::{GuiCommand, GuiReply, GuiSnapshot};
use super::{GuiReplySender, GuiRequest};
use crate::editor::Editor;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

/// Build the two ends of a headless GUI conversation.
///
/// `dimensions` is the viewport the session already established, so the first
/// projected frame matches what `/v1/resize` and `/v1/render` see.
pub fn gui_channel(dimensions: (u16, u16)) -> (GuiChannel, GuiServer) {
    let (requests, incoming) = mpsc::unbounded_channel();
    // The publishing end is shared rather than split in two. The event loop
    // sends frames through it; the HTTP layer subscribes through it and asks
    // it how many subscribers exist, which is the entire laziness signal.
    // Dropping the initial receiver is deliberate -- with it retained the
    // count would never fall back to zero.
    let updates = Arc::new(watch::channel(None).0);
    (
        GuiChannel {
            requests,
            updates: Arc::clone(&updates),
        },
        GuiServer {
            incoming,
            updates,
            dimensions,
            revision: 1,
            inputs: 0,
            last_snapshot: None,
            last_render_version: 0,
            frames_built: 0,
        },
    )
}

/// The caller's end of a headless GUI conversation.
///
/// Held by the Axum handlers, which are the only things on this side.
#[derive(Clone)]
pub struct GuiChannel {
    requests: mpsc::UnboundedSender<GuiRequest>,
    updates: Arc<watch::Sender<Option<GuiSnapshot>>>,
}

impl GuiChannel {
    /// Send one command and wait for the answer its
    /// [`reply_kind`](GuiCommand::reply_kind) promises.
    ///
    /// `Ok(None)` means the command is fire-and-forget and there was never an
    /// answer to wait for. The failure wording matches
    /// [`LocalTransport`](super::LocalTransport) so a remote frontend reads the
    /// same message as an in-process one.
    pub async fn send(&self, command: GuiCommand) -> Result<Option<GuiReply>, String> {
        let (reply, receiver) = GuiReplySender::channel(command.reply_kind());
        let request = GuiRequest::from_parts(command, reply)?;
        self.requests
            .send(request)
            .map_err(|_| EDITOR_STOPPED.to_string())?;
        receiver.recv().await
    }

    /// Watch coalesced editor-state changes.
    ///
    /// Subscribing is what makes the session start projecting frames at all,
    /// and dropping the receiver is what makes it stop.
    pub fn subscribe(&self) -> watch::Receiver<Option<GuiSnapshot>> {
        self.updates.subscribe()
    }
}

/// The editor's end of a headless GUI conversation.
///
/// Driven by `run_headless_loop`: one [`GuiServer::recv`] arm in its `select!`,
/// and a [`GuiServer::publish`] call after anything that could have changed the
/// projection.
pub struct GuiServer {
    incoming: mpsc::UnboundedReceiver<GuiRequest>,
    updates: Arc<watch::Sender<Option<GuiSnapshot>>>,
    dimensions: (u16, u16),
    revision: u64,
    /// How many commands this conversation has taken in, published on every
    /// frame so a frontend can tell which of its own keys a frame accounts for.
    inputs: u64,
    /// The last frame published, or `None` while nothing is being streamed.
    last_snapshot: Option<GuiSnapshot>,
    last_render_version: u64,
    frames_built: u64,
}

impl GuiServer {
    /// Await the next command, or `None` once every [`GuiChannel`] is gone.
    pub async fn recv(&mut self) -> Option<GuiRequest> {
        self.incoming.recv().await
    }

    /// Answer one command, reporting whether the session should keep running.
    ///
    /// `Shutdown` is the only command that stops it. A frontend closing its
    /// window must therefore not send one if it wants the session to survive
    /// for a later reconnect.
    pub async fn handle(&mut self, request: GuiRequest, editor: &mut Editor) -> bool {
        if matches!(request, GuiRequest::Shutdown) {
            return false;
        }
        super::handle_request(
            request,
            editor,
            &mut self.dimensions,
            &mut self.revision,
            &mut self.inputs,
        )
        .await;
        true
    }

    /// Publish a frame if the projection changed -- and only while somebody is
    /// watching.
    ///
    /// A headless session used purely for automation must not pay to build
    /// `GuiSnapshot`s nobody reads, so the subscriber count gates the
    /// projection itself and not merely the send. When the last subscriber
    /// leaves, the published frame is cleared: the next subscriber then cannot
    /// be handed a stale frame from a previous connection, and because no frame
    /// is on record any more the first tick after it arrives projects a fresh
    /// one regardless of whether the editor changed meanwhile.
    pub fn publish(&mut self, editor: &Editor) {
        if self.updates.receiver_count() == 0 {
            if self.last_snapshot.take().is_some() {
                self.updates.send_replace(None);
            }
            return;
        }

        let render_version = editor.render_input_version();
        if self.last_snapshot.is_some() && render_version == self.last_render_version {
            return;
        }
        self.last_render_version = render_version;
        // Project at the current revision so a runtime tick that did not touch
        // the visible state costs a comparison rather than a wire frame.
        self.frames_built += 1;
        let mut next = super::snapshot(editor, self.revision, self.inputs);
        if self.last_snapshot.as_ref() == Some(&next) {
            return;
        }
        self.revision = self.revision.wrapping_add(1);
        next.revision = self.revision;
        self.last_snapshot = Some(next.clone());
        self.updates.send_replace(Some(next));
    }

    /// How many frames this session has projected.
    ///
    /// The honest measure of laziness: an unwatched session must never raise
    /// it, however much the editor underneath is changing.
    pub fn frames_built(&self) -> u64 {
        self.frames_built
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::protocol::{GuiKeyInput, GuiReplyKind};

    /// Move the editor's visible state on, the way an edit would.
    fn changed(editor: &mut Editor, message: &str) {
        editor.set_status_message(message.to_string());
        editor.mark_dirty();
    }

    #[tokio::test]
    async fn a_session_nobody_is_watching_projects_no_frames() {
        let (_channel, mut server) = gui_channel((80, 24));
        let mut editor = Editor::with_content("fn main() {}\n");

        for index in 0..5 {
            changed(&mut editor, &format!("edit {index}"));
            server.publish(&editor);
        }

        assert_eq!(server.frames_built(), 0);
    }

    #[tokio::test]
    async fn a_subscriber_gets_a_frame_even_though_the_editor_did_not_change() {
        // The race a client would otherwise lose: it connects to a session
        // that was idle, and nothing edits the buffer afterwards.
        let (channel, mut server) = gui_channel((80, 24));
        let editor = Editor::with_content("fn main() {}\n");
        server.publish(&editor);
        assert_eq!(server.frames_built(), 0);

        let mut updates = channel.subscribe();
        server.publish(&editor);

        assert_eq!(server.frames_built(), 1);
        assert!(updates.borrow_and_update().is_some());
    }

    #[tokio::test]
    async fn the_last_subscriber_leaving_stops_projection_and_clears_the_frame() {
        // A frame kept past the last disconnect would be served to the next
        // subscriber as if it were current.
        let (channel, mut server) = gui_channel((80, 24));
        let mut editor = Editor::with_content("fn main() {}\n");
        let updates = channel.subscribe();
        server.publish(&editor);
        let published = server.frames_built();

        drop(updates);
        changed(&mut editor, "saved");
        server.publish(&editor);

        assert_eq!(server.frames_built(), published);
        assert!(channel.subscribe().borrow().is_none());
    }

    #[tokio::test]
    async fn a_command_reaches_the_editor_and_answers_with_its_declared_shape() {
        let (channel, mut server) = gui_channel((80, 24));
        let mut editor = Editor::with_content("fn main() {}\n");
        let sent = tokio::spawn(async move {
            channel
                .send(GuiCommand::EditorCommand {
                    command: "set number".to_string(),
                })
                .await
        });

        let request = server.recv().await.expect("the command should arrive");
        assert!(server.handle(request, &mut editor).await);

        let reply = sent.await.unwrap().unwrap().expect("a unit reply");
        assert_eq!(reply.kind(), GuiReplyKind::Unit);
    }

    #[tokio::test]
    async fn every_command_the_session_takes_in_raises_the_count_on_the_frame() {
        // The count is the fence a frontend measures its own speculation
        // against: a frame that has not counted a key cannot be describing the
        // buffer that key produced.
        let (channel, mut server) = gui_channel((80, 24));
        let mut editor = Editor::with_content("fn main() {}\n");
        let updates = channel.subscribe();
        server.publish(&editor);
        assert_eq!(
            updates.borrow().as_ref().map(|frame| frame.input_epoch),
            Some(0)
        );

        for key in ["i", "x"] {
            let input = GuiKeyInput {
                key: key.to_string(),
                shift: false,
                control: false,
                alt: false,
                meta: false,
            };
            let sent = {
                let channel = channel.clone();
                tokio::spawn(async move { channel.send(GuiCommand::Key { input }).await })
            };
            let request = server.recv().await.expect("the key should arrive");
            server.handle(request, &mut editor).await;
            sent.await.unwrap().expect("the key should be taken");
        }
        server.publish(&editor);

        let frame = updates.borrow().clone().expect("a frame");
        assert_eq!(frame.input_epoch, 2);
        // And the frame that counted them is one a frontend may speak for.
        assert!(frame.predictable_insert);
    }

    #[tokio::test]
    async fn a_shutdown_command_ends_the_session_without_an_answer() {
        let (channel, mut server) = gui_channel((80, 24));
        let mut editor = Editor::with_content("fn main() {}\n");
        let sent = tokio::spawn(async move { channel.send(GuiCommand::Shutdown).await });

        let request = server.recv().await.expect("the command should arrive");
        assert!(!server.handle(request, &mut editor).await);

        assert_eq!(sent.await.unwrap(), Ok(None));
    }

    #[tokio::test]
    async fn a_command_sent_to_a_stopped_session_reads_as_a_stopped_editor() {
        let (channel, server) = gui_channel((80, 24));
        drop(server);

        assert_eq!(
            channel.send(GuiCommand::SelectTab { index: 1 }).await,
            Err(EDITOR_STOPPED.to_string())
        );
    }
}
