//! The editor conversation and the transports that carry it.
//!
//! [`GuiBridge`] is the only thing the Tauri command layer talks to, and it
//! speaks nothing but [`GuiTransport`]. Everything below the trait is
//! replaceable: today the editor runs on a thread in this process
//! ([`LocalTransport`]), and a later transport can put it on another host
//! without any of the typed helpers on `GuiBridge` changing shape.

use super::protocol::{GuiClipboardText, GuiCommand, GuiReply, GuiReplyKind};
use super::reconnect::GuiConnection;
use super::{
    GuiChatInputState, GuiDiffCommentInput, GuiKeyInput, GuiMouseKind, GuiSnapshot,
    GuiTerminalRequest, GuiVectorSource,
};
use crate::cli::FileArg;
use crate::editor::EditorServices;
use anyhow::{Context, Result};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

/// The editor is gone and no command can reach it any more.
pub(crate) const EDITOR_STOPPED: &str = "The Ovim editor thread has stopped";
/// The editor took the command but dropped the answer.
pub(crate) const REPLY_CLOSED: &str = "The Ovim editor thread closed the response";

/// One editor request, with the channel its answer travels back on.
///
/// This is the in-process form: it pairs the serializable
/// [`GuiCommand`] payload with a `oneshot::Sender`, and
/// [`GuiRequest::into_parts`] separates the two again for a transport that
/// cannot carry a channel.
pub enum GuiRequest {
    CodeLens {
        pane: usize,
        file_path: String,
        version: usize,
        index: usize,
        generation: u64,
        mode: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Snapshot {
        columns: u16,
        rows: u16,
        reply: oneshot::Sender<Result<GuiSnapshot, String>>,
    },
    VectorSource {
        reply: oneshot::Sender<Result<GuiVectorSource, String>>,
    },
    VectorFeedback {
        feedback: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    WorkspaceDirectory {
        reply: oneshot::Sender<Result<PathBuf, String>>,
    },
    DiffComment {
        pane: usize,
        buffer_id: u64,
        comment: GuiDiffCommentInput,
        reply: oneshot::Sender<Result<(), String>>,
    },
    DiffAction {
        pane: usize,
        buffer_id: u64,
        action: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    OpenDiffSource {
        pane: usize,
        buffer_id: u64,
        path: String,
        line: usize,
        side: String,
        definition_column: Option<u32>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Key {
        input: GuiKeyInput,
        reply: oneshot::Sender<Result<(), String>>,
    },
    OpenAiChat {
        reply: oneshot::Sender<Result<(), String>>,
    },
    UpdateChatInput {
        expected_input: String,
        expected_cursor: usize,
        input: String,
        cursor: usize,
        action: Option<GuiKeyInput>,
        reply: oneshot::Sender<Result<GuiChatInputState, String>>,
    },
    SetChatInputCursor {
        offset: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SetChatInputWidth {
        columns: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    RemoveChatImage {
        index: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectAiProfile {
        profile: String,
        model: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectReasoningEffort {
        effort: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectPermissionMode {
        mode: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    ReplayChatTool {
        tool_call_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectChatMessage {
        index: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    ManageQueuedChatInput {
        id: u64,
        action: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    AiPolicy {
        action: String,
        target: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    EditorCommand {
        command: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectChatAgent {
        agent_id: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Paste {
        text: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    AttachImages {
        paths: Vec<PathBuf>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    AttachImageData {
        name: String,
        data: Vec<u8>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    PositionWalkthrough {
        current: usize,
        rows: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SetCursor {
        pane: usize,
        line: usize,
        display_column: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Primary-button pointer gesture on the buffer text: press, drag, release.
    Mouse {
        pane: usize,
        line: usize,
        display_column: usize,
        kind: GuiMouseKind,
        click_count: u8,
        shift: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectTab {
        index: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Scrolls one pane's viewport (positive `lines` = down) without keys or focus changes.
    Scroll {
        pane: usize,
        lines: i32,
        reply: oneshot::Sender<Result<(), String>>,
    },
    FocusPane {
        index: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectPicker {
        index: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectCompletion {
        index: usize,
        activate: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectFileTree {
        index: usize,
        activate: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectProblem {
        kind: String,
        index: usize,
        activate: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectLsp {
        index: usize,
        activate: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// A click in the replace-in-files panel: select row `index` and `toggle`
    /// its checkbox, or `apply` the review.
    SearchReplaceRow {
        index: usize,
        action: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    DebugPanelRow {
        index: usize,
        action: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    ToggleBreakpoint {
        line: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// A click on a fold marker in the gutter (`line` is 1-based).
    ToggleFold {
        line: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SelectDebugFrame {
        index: usize,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// The text the editor last put on its own host's system clipboard.
    ReadClipboard {
        reply: oneshot::Sender<Result<GuiClipboardText, String>>,
    },
    /// Put the frontend's clipboard into the editor's `+` register.
    WriteClipboard {
        text: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Shutdown,
}

/// The reply half of a [`GuiRequest`], separated from its payload.
///
/// A [`GuiCommand`] can cross a process boundary but a `oneshot::Sender`
/// cannot, so the two travel apart and are rejoined by
/// [`GuiRequest::from_parts`] on the editor's side of the transport.
pub enum GuiReplySender {
    None,
    Unit(oneshot::Sender<Result<(), String>>),
    Snapshot(oneshot::Sender<Result<GuiSnapshot, String>>),
    VectorSource(oneshot::Sender<Result<GuiVectorSource, String>>),
    WorkspaceDirectory(oneshot::Sender<Result<PathBuf, String>>),
    ChatInputState(oneshot::Sender<Result<GuiChatInputState, String>>),
    Clipboard(oneshot::Sender<Result<GuiClipboardText, String>>),
}

impl GuiReplySender {
    /// Build the reply channel that `kind` describes.
    ///
    /// Callers get the shape from [`GuiCommand::reply_kind`] rather than
    /// choosing it, which is what makes [`GuiRequest::from_parts`] unable to
    /// pair a command with the wrong channel in practice.
    pub fn channel(kind: GuiReplyKind) -> (Self, GuiReplyReceiver) {
        match kind {
            GuiReplyKind::None => (GuiReplySender::None, GuiReplyReceiver::None),
            GuiReplyKind::Unit => {
                let (tx, rx) = oneshot::channel();
                (GuiReplySender::Unit(tx), GuiReplyReceiver::Unit(rx))
            }
            GuiReplyKind::Snapshot => {
                let (tx, rx) = oneshot::channel();
                (GuiReplySender::Snapshot(tx), GuiReplyReceiver::Snapshot(rx))
            }
            GuiReplyKind::VectorSource => {
                let (tx, rx) = oneshot::channel();
                (
                    GuiReplySender::VectorSource(tx),
                    GuiReplyReceiver::VectorSource(rx),
                )
            }
            GuiReplyKind::WorkspaceDirectory => {
                let (tx, rx) = oneshot::channel();
                (
                    GuiReplySender::WorkspaceDirectory(tx),
                    GuiReplyReceiver::WorkspaceDirectory(rx),
                )
            }
            GuiReplyKind::ChatInputState => {
                let (tx, rx) = oneshot::channel();
                (
                    GuiReplySender::ChatInputState(tx),
                    GuiReplyReceiver::ChatInputState(rx),
                )
            }
            GuiReplyKind::Clipboard => {
                let (tx, rx) = oneshot::channel();
                (
                    GuiReplySender::Clipboard(tx),
                    GuiReplyReceiver::Clipboard(rx),
                )
            }
        }
    }

    pub fn kind(&self) -> GuiReplyKind {
        match self {
            GuiReplySender::None => GuiReplyKind::None,
            GuiReplySender::Unit(_) => GuiReplyKind::Unit,
            GuiReplySender::Snapshot(_) => GuiReplyKind::Snapshot,
            GuiReplySender::VectorSource(_) => GuiReplyKind::VectorSource,
            GuiReplySender::WorkspaceDirectory(_) => GuiReplyKind::WorkspaceDirectory,
            GuiReplySender::ChatInputState(_) => GuiReplyKind::ChatInputState,
            GuiReplySender::Clipboard(_) => GuiReplyKind::Clipboard,
        }
    }
}

/// The receiving end of a [`GuiReplySender`].
pub enum GuiReplyReceiver {
    None,
    Unit(oneshot::Receiver<Result<(), String>>),
    Snapshot(oneshot::Receiver<Result<GuiSnapshot, String>>),
    VectorSource(oneshot::Receiver<Result<GuiVectorSource, String>>),
    WorkspaceDirectory(oneshot::Receiver<Result<PathBuf, String>>),
    ChatInputState(oneshot::Receiver<Result<GuiChatInputState, String>>),
    Clipboard(oneshot::Receiver<Result<GuiClipboardText, String>>),
}

impl GuiReplyReceiver {
    /// Await the editor's answer.
    ///
    /// `Ok(None)` means the command is fire-and-forget and there is nothing to
    /// wait for; `Err` means the editor dropped the channel without answering.
    pub async fn recv(self) -> Result<Option<GuiReply>, String> {
        Ok(match self {
            GuiReplyReceiver::None => None,
            GuiReplyReceiver::Unit(rx) => Some(GuiReply::Unit(
                rx.await.map_err(|_| REPLY_CLOSED.to_string())?,
            )),
            GuiReplyReceiver::Snapshot(rx) => Some(GuiReply::Snapshot(Box::new(
                rx.await.map_err(|_| REPLY_CLOSED.to_string())?,
            ))),
            GuiReplyReceiver::VectorSource(rx) => Some(GuiReply::VectorSource(
                rx.await.map_err(|_| REPLY_CLOSED.to_string())?,
            )),
            GuiReplyReceiver::WorkspaceDirectory(rx) => Some(GuiReply::WorkspaceDirectory(
                rx.await.map_err(|_| REPLY_CLOSED.to_string())?,
            )),
            GuiReplyReceiver::ChatInputState(rx) => Some(GuiReply::ChatInputState(
                rx.await.map_err(|_| REPLY_CLOSED.to_string())?,
            )),
            GuiReplyReceiver::Clipboard(rx) => Some(GuiReply::Clipboard(
                rx.await.map_err(|_| REPLY_CLOSED.to_string())?,
            )),
        })
    }
}

impl GuiRequest {
    /// Rejoin a wire payload with a reply channel.
    ///
    /// The channel has to match [`GuiCommand::reply_kind`]. A caller that
    /// builds it with [`GuiReplySender::channel`] cannot get that wrong, so a
    /// mismatch is a transport bug and is reported rather than papered over by
    /// answering on a channel nobody is waiting on.
    pub fn from_parts(command: GuiCommand, reply: GuiReplySender) -> Result<Self, String> {
        Ok(match (command, reply) {
            (
                GuiCommand::CodeLens {
                    pane,
                    file_path,
                    version,
                    index,
                    generation,
                    mode,
                },
                GuiReplySender::Unit(reply),
            ) => GuiRequest::CodeLens {
                pane,
                file_path,
                version,
                index,
                generation,
                mode,
                reply,
            },
            (GuiCommand::Snapshot { columns, rows }, GuiReplySender::Snapshot(reply)) => {
                GuiRequest::Snapshot {
                    columns,
                    rows,
                    reply,
                }
            }
            (GuiCommand::VectorSource, GuiReplySender::VectorSource(reply)) => {
                GuiRequest::VectorSource { reply }
            }
            (GuiCommand::VectorFeedback { feedback }, GuiReplySender::Unit(reply)) => {
                GuiRequest::VectorFeedback { feedback, reply }
            }
            (GuiCommand::WorkspaceDirectory, GuiReplySender::WorkspaceDirectory(reply)) => {
                GuiRequest::WorkspaceDirectory { reply }
            }
            (
                GuiCommand::DiffComment {
                    pane,
                    buffer_id,
                    comment,
                },
                GuiReplySender::Unit(reply),
            ) => GuiRequest::DiffComment {
                pane,
                buffer_id,
                comment,
                reply,
            },
            (
                GuiCommand::DiffAction {
                    pane,
                    buffer_id,
                    action,
                },
                GuiReplySender::Unit(reply),
            ) => GuiRequest::DiffAction {
                pane,
                buffer_id,
                action,
                reply,
            },
            (
                GuiCommand::OpenDiffSource {
                    pane,
                    buffer_id,
                    path,
                    line,
                    side,
                    definition_column,
                },
                GuiReplySender::Unit(reply),
            ) => GuiRequest::OpenDiffSource {
                pane,
                buffer_id,
                path,
                line,
                side,
                definition_column,
                reply,
            },
            (GuiCommand::Key { input }, GuiReplySender::Unit(reply)) => {
                GuiRequest::Key { input, reply }
            }
            (GuiCommand::OpenAiChat, GuiReplySender::Unit(reply)) => {
                GuiRequest::OpenAiChat { reply }
            }
            (
                GuiCommand::UpdateChatInput {
                    expected_input,
                    expected_cursor,
                    input,
                    cursor,
                    action,
                },
                GuiReplySender::ChatInputState(reply),
            ) => GuiRequest::UpdateChatInput {
                expected_input,
                expected_cursor,
                input,
                cursor,
                action,
                reply,
            },
            (GuiCommand::SetChatInputCursor { offset }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SetChatInputCursor { offset, reply }
            }
            (GuiCommand::SetChatInputWidth { columns }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SetChatInputWidth { columns, reply }
            }
            (GuiCommand::RemoveChatImage { index }, GuiReplySender::Unit(reply)) => {
                GuiRequest::RemoveChatImage { index, reply }
            }
            (GuiCommand::SelectAiProfile { profile, model }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectAiProfile {
                    profile,
                    model,
                    reply,
                }
            }
            (GuiCommand::SelectReasoningEffort { effort }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectReasoningEffort { effort, reply }
            }
            (GuiCommand::SelectPermissionMode { mode }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectPermissionMode { mode, reply }
            }
            (GuiCommand::ReplayChatTool { tool_call_id }, GuiReplySender::Unit(reply)) => {
                GuiRequest::ReplayChatTool {
                    tool_call_id,
                    reply,
                }
            }
            (GuiCommand::SelectChatMessage { index }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectChatMessage { index, reply }
            }
            (GuiCommand::ManageQueuedChatInput { id, action }, GuiReplySender::Unit(reply)) => {
                GuiRequest::ManageQueuedChatInput { id, action, reply }
            }
            (GuiCommand::AiPolicy { action, target }, GuiReplySender::Unit(reply)) => {
                GuiRequest::AiPolicy {
                    action,
                    target,
                    reply,
                }
            }
            (GuiCommand::EditorCommand { command }, GuiReplySender::Unit(reply)) => {
                GuiRequest::EditorCommand { command, reply }
            }
            (GuiCommand::SelectChatAgent { agent_id }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectChatAgent { agent_id, reply }
            }
            (GuiCommand::Paste { text }, GuiReplySender::Unit(reply)) => {
                GuiRequest::Paste { text, reply }
            }
            (GuiCommand::AttachImages { paths }, GuiReplySender::Unit(reply)) => {
                GuiRequest::AttachImages { paths, reply }
            }
            (GuiCommand::AttachImageData { name, data }, GuiReplySender::Unit(reply)) => {
                GuiRequest::AttachImageData { name, data, reply }
            }
            (GuiCommand::PositionWalkthrough { current, rows }, GuiReplySender::Unit(reply)) => {
                GuiRequest::PositionWalkthrough {
                    current,
                    rows,
                    reply,
                }
            }
            (
                GuiCommand::SetCursor {
                    pane,
                    line,
                    display_column,
                },
                GuiReplySender::Unit(reply),
            ) => GuiRequest::SetCursor {
                pane,
                line,
                display_column,
                reply,
            },
            (
                GuiCommand::Mouse {
                    pane,
                    line,
                    display_column,
                    kind,
                    click_count,
                    shift,
                },
                GuiReplySender::Unit(reply),
            ) => GuiRequest::Mouse {
                pane,
                line,
                display_column,
                kind,
                click_count,
                shift,
                reply,
            },
            (GuiCommand::SelectTab { index }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectTab { index, reply }
            }
            (GuiCommand::Scroll { pane, lines }, GuiReplySender::Unit(reply)) => {
                GuiRequest::Scroll { pane, lines, reply }
            }
            (GuiCommand::FocusPane { index }, GuiReplySender::Unit(reply)) => {
                GuiRequest::FocusPane { index, reply }
            }
            (GuiCommand::SelectPicker { index }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectPicker { index, reply }
            }
            (GuiCommand::SelectCompletion { index, activate }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectCompletion {
                    index,
                    activate,
                    reply,
                }
            }
            (GuiCommand::SelectFileTree { index, activate }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectFileTree {
                    index,
                    activate,
                    reply,
                }
            }
            (
                GuiCommand::SelectProblem {
                    kind,
                    index,
                    activate,
                },
                GuiReplySender::Unit(reply),
            ) => GuiRequest::SelectProblem {
                kind,
                index,
                activate,
                reply,
            },
            (GuiCommand::SelectLsp { index, activate }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectLsp {
                    index,
                    activate,
                    reply,
                }
            }
            (GuiCommand::SearchReplaceRow { index, action }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SearchReplaceRow {
                    index,
                    action,
                    reply,
                }
            }
            (GuiCommand::DebugPanelRow { index, action }, GuiReplySender::Unit(reply)) => {
                GuiRequest::DebugPanelRow {
                    index,
                    action,
                    reply,
                }
            }
            (GuiCommand::ToggleBreakpoint { line }, GuiReplySender::Unit(reply)) => {
                GuiRequest::ToggleBreakpoint { line, reply }
            }
            (GuiCommand::ToggleFold { line }, GuiReplySender::Unit(reply)) => {
                GuiRequest::ToggleFold { line, reply }
            }
            (GuiCommand::SelectDebugFrame { index }, GuiReplySender::Unit(reply)) => {
                GuiRequest::SelectDebugFrame { index, reply }
            }
            (GuiCommand::ReadClipboard, GuiReplySender::Clipboard(reply)) => {
                GuiRequest::ReadClipboard { reply }
            }
            (GuiCommand::WriteClipboard { text }, GuiReplySender::Unit(reply)) => {
                GuiRequest::WriteClipboard { text, reply }
            }
            (GuiCommand::Shutdown, GuiReplySender::None) => GuiRequest::Shutdown,
            (command, reply) => {
                return Err(format!(
                    "A GUI command needing a {:?} reply was given a {:?} reply channel",
                    command.reply_kind(),
                    reply.kind()
                ))
            }
        })
    }

    /// Split a request back into the payload a transport can serialize and the
    /// reply channel it cannot.
    ///
    /// This is the exact inverse of [`GuiRequest::from_parts`]; the round-trip
    /// test in this module is what keeps a field from being dropped on the way
    /// through the wire format.
    pub fn into_parts(self) -> (GuiCommand, GuiReplySender) {
        match self {
            GuiRequest::CodeLens {
                pane,
                file_path,
                version,
                index,
                generation,
                mode,
                reply,
            } => (
                GuiCommand::CodeLens {
                    pane,
                    file_path,
                    version,
                    index,
                    generation,
                    mode,
                },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::Snapshot {
                columns,
                rows,
                reply,
            } => (
                GuiCommand::Snapshot { columns, rows },
                GuiReplySender::Snapshot(reply),
            ),
            GuiRequest::VectorSource { reply } => (
                GuiCommand::VectorSource,
                GuiReplySender::VectorSource(reply),
            ),
            GuiRequest::VectorFeedback { feedback, reply } => (
                GuiCommand::VectorFeedback { feedback },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::WorkspaceDirectory { reply } => (
                GuiCommand::WorkspaceDirectory,
                GuiReplySender::WorkspaceDirectory(reply),
            ),
            GuiRequest::DiffComment {
                pane,
                buffer_id,
                comment,
                reply,
            } => (
                GuiCommand::DiffComment {
                    pane,
                    buffer_id,
                    comment,
                },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::DiffAction {
                pane,
                buffer_id,
                action,
                reply,
            } => (
                GuiCommand::DiffAction {
                    pane,
                    buffer_id,
                    action,
                },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::OpenDiffSource {
                pane,
                buffer_id,
                path,
                line,
                side,
                definition_column,
                reply,
            } => (
                GuiCommand::OpenDiffSource {
                    pane,
                    buffer_id,
                    path,
                    line,
                    side,
                    definition_column,
                },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::Key { input, reply } => {
                (GuiCommand::Key { input }, GuiReplySender::Unit(reply))
            }
            GuiRequest::OpenAiChat { reply } => {
                (GuiCommand::OpenAiChat, GuiReplySender::Unit(reply))
            }
            GuiRequest::UpdateChatInput {
                expected_input,
                expected_cursor,
                input,
                cursor,
                action,
                reply,
            } => (
                GuiCommand::UpdateChatInput {
                    expected_input,
                    expected_cursor,
                    input,
                    cursor,
                    action,
                },
                GuiReplySender::ChatInputState(reply),
            ),
            GuiRequest::SetChatInputCursor { offset, reply } => (
                GuiCommand::SetChatInputCursor { offset },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SetChatInputWidth { columns, reply } => (
                GuiCommand::SetChatInputWidth { columns },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::RemoveChatImage { index, reply } => (
                GuiCommand::RemoveChatImage { index },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SelectAiProfile {
                profile,
                model,
                reply,
            } => (
                GuiCommand::SelectAiProfile { profile, model },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SelectReasoningEffort { effort, reply } => (
                GuiCommand::SelectReasoningEffort { effort },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SelectPermissionMode { mode, reply } => (
                GuiCommand::SelectPermissionMode { mode },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::ReplayChatTool {
                tool_call_id,
                reply,
            } => (
                GuiCommand::ReplayChatTool { tool_call_id },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SelectChatMessage { index, reply } => (
                GuiCommand::SelectChatMessage { index },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::ManageQueuedChatInput { id, action, reply } => (
                GuiCommand::ManageQueuedChatInput { id, action },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::AiPolicy {
                action,
                target,
                reply,
            } => (
                GuiCommand::AiPolicy { action, target },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::EditorCommand { command, reply } => (
                GuiCommand::EditorCommand { command },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SelectChatAgent { agent_id, reply } => (
                GuiCommand::SelectChatAgent { agent_id },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::Paste { text, reply } => {
                (GuiCommand::Paste { text }, GuiReplySender::Unit(reply))
            }
            GuiRequest::AttachImages { paths, reply } => (
                GuiCommand::AttachImages { paths },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::AttachImageData { name, data, reply } => (
                GuiCommand::AttachImageData { name, data },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::PositionWalkthrough {
                current,
                rows,
                reply,
            } => (
                GuiCommand::PositionWalkthrough { current, rows },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SetCursor {
                pane,
                line,
                display_column,
                reply,
            } => (
                GuiCommand::SetCursor {
                    pane,
                    line,
                    display_column,
                },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::Mouse {
                pane,
                line,
                display_column,
                kind,
                click_count,
                shift,
                reply,
            } => (
                GuiCommand::Mouse {
                    pane,
                    line,
                    display_column,
                    kind,
                    click_count,
                    shift,
                },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SelectTab { index, reply } => {
                (GuiCommand::SelectTab { index }, GuiReplySender::Unit(reply))
            }
            GuiRequest::Scroll { pane, lines, reply } => (
                GuiCommand::Scroll { pane, lines },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::FocusPane { index, reply } => {
                (GuiCommand::FocusPane { index }, GuiReplySender::Unit(reply))
            }
            GuiRequest::SelectPicker { index, reply } => (
                GuiCommand::SelectPicker { index },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SelectCompletion {
                index,
                activate,
                reply,
            } => (
                GuiCommand::SelectCompletion { index, activate },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SelectFileTree {
                index,
                activate,
                reply,
            } => (
                GuiCommand::SelectFileTree { index, activate },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SelectProblem {
                kind,
                index,
                activate,
                reply,
            } => (
                GuiCommand::SelectProblem {
                    kind,
                    index,
                    activate,
                },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SelectLsp {
                index,
                activate,
                reply,
            } => (
                GuiCommand::SelectLsp { index, activate },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::SearchReplaceRow {
                index,
                action,
                reply,
            } => (
                GuiCommand::SearchReplaceRow { index, action },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::DebugPanelRow {
                index,
                action,
                reply,
            } => (
                GuiCommand::DebugPanelRow { index, action },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::ToggleBreakpoint { line, reply } => (
                GuiCommand::ToggleBreakpoint { line },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::ToggleFold { line, reply } => {
                (GuiCommand::ToggleFold { line }, GuiReplySender::Unit(reply))
            }
            GuiRequest::SelectDebugFrame { index, reply } => (
                GuiCommand::SelectDebugFrame { index },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::ReadClipboard { reply } => {
                (GuiCommand::ReadClipboard, GuiReplySender::Clipboard(reply))
            }
            GuiRequest::WriteClipboard { text, reply } => (
                GuiCommand::WriteClipboard { text },
                GuiReplySender::Unit(reply),
            ),
            GuiRequest::Shutdown => (GuiCommand::Shutdown, GuiReplySender::None),
        }
    }
}

/// A future produced by a [`GuiTransport`] method.
///
/// The future is boxed rather than written as `async fn` in the trait,
/// because an `async fn` in a trait is not dyn-compatible and the entire point
/// of this trait is `Arc<dyn GuiTransport>`. `ovim-core` already spells its
/// object-safe async traits this way (`ai::auto_classifier`,
/// `agent_runtime::loop_runner`), so this matches the codebase instead of
/// pulling in `async-trait` for one trait.
pub type GuiTransportFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Where an editor conversation actually happens.
///
/// [`GuiBridge`] speaks only this trait, so moving the editor to another
/// process or another host is a matter of supplying a different
/// implementation rather than touching the Tauri commands above it.
pub trait GuiTransport: Send + Sync {
    /// Send one command and wait for the answer its
    /// [`reply_kind`](GuiCommand::reply_kind) promises.
    ///
    /// `Ok(None)` means the command is fire-and-forget and there was never an
    /// answer to wait for. `Err` means the conversation itself failed -- the
    /// editor is gone, or the link to it is.
    fn send(&self, command: GuiCommand)
        -> GuiTransportFuture<'_, Result<Option<GuiReply>, String>>;

    /// Send a command that answers nothing, without awaiting.
    ///
    /// Shutdown arrives from Tauri's synchronous exit handler, where there is
    /// no runtime to await on. Only commands whose
    /// [`reply_kind`](GuiCommand::reply_kind) is [`GuiReplyKind::None`] belong
    /// here; anything else is rejected rather than silently losing its answer.
    fn send_oneway(&self, command: GuiCommand) -> Result<(), String>;

    /// Subscribe to coalesced editor-state changes.
    ///
    /// A watch channel rather than a stream of every frame: a slow consumer
    /// keeps only the newest snapshot instead of building an unbounded queue,
    /// which is what both a slow webview and a saturated network link need.
    fn subscribe(&self) -> watch::Receiver<Option<GuiSnapshot>>;

    /// Watch for terminal launches the editor asks its frontend to make.
    ///
    /// The GUI's terminal panel runs a shell on *this* machine, so only an
    /// in-process editor has terminals to hand over. A remote transport keeps
    /// the default: a channel whose sender is already gone, which never
    /// fires. (The remote session itself declines `:terminal`, since it has
    /// no terminal of its own.)
    fn subscribe_terminal_requests(&self) -> watch::Receiver<Option<GuiTerminalRequest>> {
        watch::channel(None).1
    }

    /// Watch the state of the link to the editor.
    ///
    /// Separate from [`GuiTransport::subscribe`] on purpose. A snapshot only
    /// arrives while the link works, so connection state carried on one could
    /// never change at the moment it has something to say -- which is exactly
    /// when the link has just died. A transport with no link to lose leaves
    /// this at [`GuiConnection::Connected`] and drops the sender, so the
    /// frontend reads one value and never hears from it again.
    fn connection(&self) -> watch::Receiver<GuiConnection> {
        watch::channel(GuiConnection::Connected).1
    }

    /// Ask for another attempt at the link, now.
    ///
    /// `allow_new_session` is the one thing reconnection will not decide by
    /// itself: replacing a session that has ended discards its undo history
    /// and its warm language servers, so it happens only when a user says so.
    fn request_reconnect(&self, _allow_new_session: bool) -> Result<(), String> {
        Err("This editor runs in this window, so there is no link to reconnect.".to_string())
    }

    /// Whether a keystroke has to cross a network to reach the editor.
    ///
    /// The frontend asks once, at startup, and speculates locally only when the
    /// answer is yes. An in-process editor answers within a frame, so
    /// predicting for it would be a way of being wrong occasionally in exchange
    /// for nothing at all.
    fn is_remote(&self) -> bool {
        false
    }

    /// This window's name in the session's clipboard attribution; see
    /// [`GuiClipboard::writer`](super::GuiClipboard::writer). Only a remote
    /// transport has one.
    fn client_id(&self) -> Option<String> {
        None
    }
}

/// The in-process transport: the editor runs on a thread in this process and
/// the conversation is a pair of Tokio channels.
pub struct LocalTransport {
    requests: mpsc::UnboundedSender<GuiRequest>,
    updates: watch::Sender<Option<GuiSnapshot>>,
    terminal_requests: watch::Sender<Option<GuiTerminalRequest>>,
}

impl LocalTransport {
    /// Start an editor on its own thread and return the transport to it.
    ///
    /// Blocks until the editor reports that it opened its file and is ready
    /// to answer, so the first snapshot request never races startup.
    pub fn spawn(file: Option<FileArg>, resume: bool, services: EditorServices) -> Result<Self> {
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (update_tx, _) = watch::channel(None);
        let (terminal_tx, _) = watch::channel(None);
        let (ready_tx, ready_rx) = std_mpsc::sync_channel(1);
        let editor_updates = update_tx.clone();
        let editor_terminals = terminal_tx.clone();

        std::thread::Builder::new()
            .name("ovim-gui-editor".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build();
                let Ok(runtime) = runtime else {
                    let _ = ready_tx.send(Err("Failed to create GUI runtime".to_string()));
                    return;
                };
                runtime.block_on(super::run_editor(
                    file,
                    resume,
                    services,
                    request_rx,
                    editor_updates,
                    editor_terminals,
                    ready_tx,
                ));
            })
            .context("Failed to start the GUI editor thread")?;

        match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => Ok(Self {
                requests: request_tx,
                updates: update_tx,
                terminal_requests: terminal_tx,
            }),
            Ok(Err(error)) => anyhow::bail!(error),
            Err(error) => anyhow::bail!("GUI editor initialization timed out: {error}"),
        }
    }

    /// Wrap channels whose editor loop somebody else owns.
    ///
    /// The headless server drives its own event loop rather than spawning a
    /// thread, and tests substitute a scripted responder for the editor.
    pub fn new(
        requests: mpsc::UnboundedSender<GuiRequest>,
        updates: watch::Sender<Option<GuiSnapshot>>,
    ) -> Self {
        Self {
            requests,
            updates,
            terminal_requests: watch::channel(None).0,
        }
    }

    /// Hand the request to the editor loop, mapping a closed channel to the
    /// one wording the frontend already knows.
    fn dispatch(&self, command: GuiCommand, reply: GuiReplySender) -> Result<(), String> {
        let request = GuiRequest::from_parts(command, reply)?;
        self.requests
            .send(request)
            .map_err(|_| EDITOR_STOPPED.to_string())
    }
}

impl GuiTransport for LocalTransport {
    fn send(
        &self,
        command: GuiCommand,
    ) -> GuiTransportFuture<'_, Result<Option<GuiReply>, String>> {
        let (reply, receiver) = GuiReplySender::channel(command.reply_kind());
        // Queue the request eagerly so that a stopped editor is reported the
        // moment the caller asks, exactly as the direct channel send did,
        // rather than only once the returned future is first polled.
        let queued = self.dispatch(command, reply);
        Box::pin(async move {
            queued?;
            receiver.recv().await
        })
    }

    fn send_oneway(&self, command: GuiCommand) -> Result<(), String> {
        if command.reply_kind() != GuiReplyKind::None {
            return Err(format!(
                "A GUI command expecting a {:?} reply cannot be sent one-way",
                command.reply_kind()
            ));
        }
        let (reply, _receiver) = GuiReplySender::channel(GuiReplyKind::None);
        self.dispatch(command, reply)
    }

    fn subscribe(&self) -> watch::Receiver<Option<GuiSnapshot>> {
        self.updates.subscribe()
    }

    fn subscribe_terminal_requests(&self) -> watch::Receiver<Option<GuiTerminalRequest>> {
        self.terminal_requests.subscribe()
    }
}

/// A transport answered with a reply of a shape the command never asked for.
///
/// Unreachable with a correct transport, since [`GuiCommand::reply_kind`]
/// fixes the shape at both ends. A remote transport talking to a mismatched
/// server version can still produce it, and that has to read as an error
/// rather than a panic in a Tauri command.
fn mismatched_reply(expected: GuiReplyKind, actual: Option<&GuiReply>) -> String {
    match actual {
        Some(reply) => format!(
            "The Ovim editor answered with a {:?} reply where a {expected:?} was expected",
            reply.kind()
        ),
        None => format!("The Ovim editor did not answer, but a {expected:?} reply was expected"),
    }
}

/// The largest dropped image sent over a remote link, matching the limit the
/// editor applies to a pasted image.
const MAX_REMOTE_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Read a dropped image on this machine so it can cross a remote link.
///
/// The same extension check the editor applies to a local drop, plus a size
/// bound checked before the file is read rather than after.
fn read_local_image(path: &std::path::Path) -> Result<(String, Vec<u8>), String> {
    let supported = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp"
            )
        });
    if !supported {
        return Err(format!("Unsupported image format: {}", path.display()));
    }
    let size = std::fs::metadata(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?
        .len();
    if size > MAX_REMOTE_IMAGE_BYTES {
        return Err("Image exceeds the 20 MiB limit".to_string());
    }
    let data = std::fs::read(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_string();
    Ok((name, data))
}

/// Unwrap the answer to a command whose only reply is success or a message.
fn unit_reply(reply: Option<GuiReply>) -> Result<(), String> {
    match reply {
        Some(GuiReply::Unit(result)) => result,
        other => Err(mismatched_reply(GuiReplyKind::Unit, other.as_ref())),
    }
}

/// Send-side handle stored as Tauri application state.
///
/// Every method here is a typed name for one [`GuiCommand`]; the transport
/// underneath decides where that command is carried out.
#[derive(Clone)]
pub struct GuiBridge {
    transport: Arc<dyn GuiTransport>,
    /// How many state-changing commands this bridge has sent, shared by every
    /// clone. The clipboard mirror accepts a write only after this window has
    /// acted, which a remote cannot fake.
    commands_sent: Arc<std::sync::atomic::AtomicU64>,
}

impl GuiBridge {
    /// Run the editor in this process and talk to it over channels.
    pub fn spawn(file: Option<FileArg>, resume: bool, services: EditorServices) -> Result<Self> {
        Ok(Self::new(Arc::new(LocalTransport::spawn(
            file, resume, services,
        )?)))
    }

    /// Talk to an editor over an arbitrary transport.
    pub fn new(transport: Arc<dyn GuiTransport>) -> Self {
        Self {
            transport,
            commands_sent: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Hand a command to the transport, counting it if it changes state.
    ///
    /// Not `async`: the transport takes the command's place in the sequence
    /// when this is called, not when the result is first polled.
    fn send(
        &self,
        command: GuiCommand,
    ) -> GuiTransportFuture<'_, Result<Option<GuiReply>, String>> {
        if command.must_keep_its_place() {
            self.commands_sent
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        self.transport.send(command)
    }

    /// How many state-changing commands this window has sent.
    pub fn commands_sent(&self) -> u64 {
        self.commands_sent.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// This window's name in clipboard attribution, for a remote editor.
    pub fn client_id(&self) -> Option<String> {
        self.transport.client_id()
    }

    /// Put `text` in the editor's `+` register, then deliver `input`.
    ///
    /// Both commands take their places in the ordered lane before either is
    /// awaited, so nothing the window sends meanwhile can land between them:
    /// the key that pastes always reads the text sent for it.
    pub fn clipboard_then_key(
        &self,
        text: String,
        input: GuiKeyInput,
    ) -> impl std::future::Future<Output = Result<(), String>> + '_ {
        let write = self.send(GuiCommand::WriteClipboard { text });
        let key = self.send(GuiCommand::Key { input });
        async move {
            let wrote = unit_reply(write.await?);
            let typed = unit_reply(key.await?);
            wrote.and(typed)
        }
    }

    /// Subscribe to coalesced editor-state changes.
    ///
    /// Tauri turns this watch stream into an IPC channel. Slow webviews only
    /// retain the newest snapshot instead of building an unbounded queue.
    pub fn subscribe(&self) -> watch::Receiver<Option<GuiSnapshot>> {
        self.transport.subscribe()
    }

    /// Watch for terminal launches the editor hands to this frontend.
    pub fn subscribe_terminal_requests(&self) -> watch::Receiver<Option<GuiTerminalRequest>> {
        self.transport.subscribe_terminal_requests()
    }

    /// Watch the state of the link to the editor, which for an in-process one
    /// is a single [`GuiConnection::Connected`] that never changes.
    pub fn connection(&self) -> watch::Receiver<GuiConnection> {
        self.transport.connection()
    }

    /// Ask the transport for another attempt at the link.
    pub fn request_reconnect(&self, allow_new_session: bool) -> Result<(), String> {
        self.transport.request_reconnect(allow_new_session)
    }

    /// Whether keystrokes cross a network on their way to the editor.
    pub fn is_remote(&self) -> bool {
        self.transport.is_remote()
    }

    pub async fn snapshot(&self, columns: u16, rows: u16) -> Result<GuiSnapshot, String> {
        match self
            .transport
            .send(GuiCommand::Snapshot { columns, rows })
            .await?
        {
            Some(GuiReply::Snapshot(result)) => *result,
            other => Err(mismatched_reply(GuiReplyKind::Snapshot, other.as_ref())),
        }
    }

    pub async fn vector_source(&self) -> Result<GuiVectorSource, String> {
        match self.send(GuiCommand::VectorSource).await? {
            Some(GuiReply::VectorSource(result)) => result,
            other => Err(mismatched_reply(GuiReplyKind::VectorSource, other.as_ref())),
        }
    }

    pub async fn vector_feedback(&self, feedback: String) -> Result<(), String> {
        self.unit(GuiCommand::VectorFeedback { feedback }).await
    }

    /// The editor's workspace directory, which is a path on the editor's
    /// host: see [`GuiBridge::is_remote`] before using it locally.
    pub async fn workspace_directory(&self) -> Result<PathBuf, String> {
        match self.send(GuiCommand::WorkspaceDirectory).await? {
            Some(GuiReply::WorkspaceDirectory(result)) => result,
            other => Err(mismatched_reply(
                GuiReplyKind::WorkspaceDirectory,
                other.as_ref(),
            )),
        }
    }

    pub async fn diff_comment(
        &self,
        pane: usize,
        buffer_id: u64,
        comment: GuiDiffCommentInput,
    ) -> Result<(), String> {
        self.unit(GuiCommand::DiffComment {
            pane,
            buffer_id,
            comment,
        })
        .await
    }

    pub async fn diff_action(
        &self,
        pane: usize,
        buffer_id: u64,
        action: String,
    ) -> Result<(), String> {
        self.unit(GuiCommand::DiffAction {
            pane,
            buffer_id,
            action,
        })
        .await
    }

    pub async fn open_diff_source(
        &self,
        pane: usize,
        buffer_id: u64,
        path: String,
        line: usize,
        side: String,
        definition_column: Option<u32>,
    ) -> Result<(), String> {
        self.unit(GuiCommand::OpenDiffSource {
            pane,
            buffer_id,
            path,
            line,
            side,
            definition_column,
        })
        .await
    }

    pub async fn key(&self, input: GuiKeyInput) -> Result<(), String> {
        self.unit(GuiCommand::Key { input }).await
    }

    pub async fn open_ai_chat(&self) -> Result<(), String> {
        self.unit(GuiCommand::OpenAiChat).await
    }

    pub async fn paste(&self, text: String) -> Result<(), String> {
        self.unit(GuiCommand::Paste { text }).await
    }

    pub async fn set_chat_input_cursor(&self, offset: usize) -> Result<(), String> {
        self.unit(GuiCommand::SetChatInputCursor { offset }).await
    }

    pub async fn update_chat_input(
        &self,
        expected_input: String,
        expected_cursor: usize,
        input: String,
        cursor: usize,
        action: Option<GuiKeyInput>,
    ) -> Result<GuiChatInputState, String> {
        match self
            .transport
            .send(GuiCommand::UpdateChatInput {
                expected_input,
                expected_cursor,
                input,
                cursor,
                action,
            })
            .await?
        {
            Some(GuiReply::ChatInputState(result)) => result,
            other => Err(mismatched_reply(
                GuiReplyKind::ChatInputState,
                other.as_ref(),
            )),
        }
    }

    pub async fn set_chat_input_width(&self, columns: usize) -> Result<(), String> {
        self.unit(GuiCommand::SetChatInputWidth { columns }).await
    }

    pub async fn remove_chat_image(&self, index: usize) -> Result<(), String> {
        self.unit(GuiCommand::RemoveChatImage { index }).await
    }

    /// Attach dropped image files to the chat composer.
    ///
    /// The paths name files on *this* machine. An in-process editor reads them
    /// itself; an editor on another host would look for them on its own disk,
    /// so over a remote link the bytes are read here and sent instead.
    pub async fn attach_images(&self, paths: Vec<PathBuf>) -> Result<(), String> {
        if !self.is_remote() {
            return self.unit(GuiCommand::AttachImages { paths }).await;
        }
        for path in paths {
            let (name, data) = read_local_image(&path)?;
            self.attach_image_data(name, data).await?;
        }
        Ok(())
    }

    pub async fn attach_image_data(&self, name: String, data: Vec<u8>) -> Result<(), String> {
        self.unit(GuiCommand::AttachImageData { name, data }).await
    }

    pub async fn position_walkthrough(&self, current: usize, rows: usize) -> Result<(), String> {
        self.unit(GuiCommand::PositionWalkthrough { current, rows })
            .await
    }

    pub async fn set_cursor(
        &self,
        pane: usize,
        line: usize,
        display_column: usize,
    ) -> Result<(), String> {
        self.unit(GuiCommand::SetCursor {
            pane,
            line,
            display_column,
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn code_lens(
        &self,
        pane: usize,
        file_path: String,
        version: usize,
        index: usize,
        generation: u64,
        mode: String,
    ) -> Result<(), String> {
        self.unit(GuiCommand::CodeLens {
            pane,
            file_path,
            version,
            index,
            generation,
            mode,
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn mouse(
        &self,
        pane: usize,
        line: usize,
        display_column: usize,
        kind: GuiMouseKind,
        click_count: u8,
        shift: bool,
    ) -> Result<(), String> {
        self.unit(GuiCommand::Mouse {
            pane,
            line,
            display_column,
            kind,
            click_count,
            shift,
        })
        .await
    }

    pub async fn select_tab(&self, index: usize) -> Result<(), String> {
        self.unit(GuiCommand::SelectTab { index }).await
    }

    pub async fn scroll(&self, pane: usize, lines: i32) -> Result<(), String> {
        self.unit(GuiCommand::Scroll { pane, lines }).await
    }

    pub async fn focus_pane(&self, index: usize) -> Result<(), String> {
        self.unit(GuiCommand::FocusPane { index }).await
    }

    pub async fn select_picker(&self, index: usize) -> Result<(), String> {
        self.unit(GuiCommand::SelectPicker { index }).await
    }

    pub async fn select_completion(&self, index: usize, activate: bool) -> Result<(), String> {
        self.unit(GuiCommand::SelectCompletion { index, activate })
            .await
    }

    pub async fn select_file_tree(&self, index: usize, activate: bool) -> Result<(), String> {
        self.unit(GuiCommand::SelectFileTree { index, activate })
            .await
    }

    pub async fn select_problem(
        &self,
        kind: String,
        index: usize,
        activate: bool,
    ) -> Result<(), String> {
        self.unit(GuiCommand::SelectProblem {
            kind,
            index,
            activate,
        })
        .await
    }

    pub async fn search_replace_row(&self, index: usize, action: String) -> Result<(), String> {
        self.unit(GuiCommand::SearchReplaceRow { index, action })
            .await
    }

    pub async fn select_lsp(&self, index: usize, activate: bool) -> Result<(), String> {
        self.unit(GuiCommand::SelectLsp { index, activate }).await
    }

    /// A click in the debug panel: `activate`, `delete` or `toggle` on row `index`.
    pub async fn debug_panel_row(&self, index: usize, action: String) -> Result<(), String> {
        self.unit(GuiCommand::DebugPanelRow { index, action }).await
    }

    /// A click in the gutter: toggle the breakpoint on `line` (1-based).
    pub async fn toggle_breakpoint(&self, line: usize) -> Result<(), String> {
        self.unit(GuiCommand::ToggleBreakpoint { line }).await
    }

    /// A click on a fold marker in the gutter: toggle the fold on `line` (1-based).
    pub async fn toggle_fold(&self, line: usize) -> Result<(), String> {
        self.unit(GuiCommand::ToggleFold { line }).await
    }

    pub async fn select_debug_frame(&self, index: usize) -> Result<(), String> {
        self.unit(GuiCommand::SelectDebugFrame { index }).await
    }

    pub async fn select_ai_profile(
        &self,
        profile: String,
        model: Option<String>,
    ) -> Result<(), String> {
        self.unit(GuiCommand::SelectAiProfile { profile, model })
            .await
    }

    pub async fn select_reasoning_effort(&self, effort: String) -> Result<(), String> {
        self.unit(GuiCommand::SelectReasoningEffort { effort })
            .await
    }

    pub async fn select_permission_mode(&self, mode: String) -> Result<(), String> {
        self.unit(GuiCommand::SelectPermissionMode { mode }).await
    }

    pub async fn replay_chat_tool(&self, tool_call_id: String) -> Result<(), String> {
        self.unit(GuiCommand::ReplayChatTool { tool_call_id }).await
    }

    pub async fn select_chat_message(&self, index: usize) -> Result<(), String> {
        self.unit(GuiCommand::SelectChatMessage { index }).await
    }

    pub async fn manage_queued_chat_input(&self, id: u64, action: String) -> Result<(), String> {
        self.unit(GuiCommand::ManageQueuedChatInput { id, action })
            .await
    }

    pub async fn ai_policy(&self, action: String, target: Option<String>) -> Result<(), String> {
        self.unit(GuiCommand::AiPolicy { action, target }).await
    }

    pub async fn editor_command(&self, command: String) -> Result<(), String> {
        self.unit(GuiCommand::EditorCommand { command }).await
    }

    pub async fn select_chat_agent(&self, agent_id: Option<String>) -> Result<(), String> {
        self.unit(GuiCommand::SelectChatAgent { agent_id }).await
    }

    /// The text the editor last put on its own host's system clipboard.
    ///
    /// Only worth asking when [`GuiSnapshot::clipboard`]'s generation has
    /// moved; see [`crate::gui::clipboard`] for the policy around it.
    pub async fn read_clipboard(&self) -> Result<GuiClipboardText, String> {
        match self.send(GuiCommand::ReadClipboard).await? {
            Some(GuiReply::Clipboard(result)) => result,
            other => Err(mismatched_reply(GuiReplyKind::Clipboard, other.as_ref())),
        }
    }

    /// Put this machine's clipboard into the editor's `+` register.
    pub async fn write_clipboard(&self, text: String) -> Result<(), String> {
        self.unit(GuiCommand::WriteClipboard { text }).await
    }

    /// Ask the editor to stop, from a context that cannot await.
    ///
    /// The editor is on its way out either way, so a transport that has
    /// already gone is not worth reporting.
    pub fn shutdown(&self) {
        let _ = self.transport.send_oneway(GuiCommand::Shutdown);
    }

    /// Send a command whose only answer is success or a message.
    async fn unit(&self, command: GuiCommand) -> Result<(), String> {
        unit_reply(self.send(command).await?)
    }
}

#[cfg(test)]
// `pub(crate)` so the remote transport can be driven through the very same
// sweep as the local one; a helper wired to the wrong command variant has to
// fail identically over either transport.
pub(crate) mod tests {
    use super::*;
    use crate::editor::Editor;
    use crate::gui::protocol;
    use std::collections::HashSet;
    use std::mem::discriminant;
    use std::sync::Mutex;

    #[test]
    fn every_command_rebuilds_the_request_it_came_from_without_losing_a_field() {
        // `into_parts` is the inverse of `from_parts`, so a payload field that
        // either conversion forgets shows up as an inequality here.
        for command in protocol::sample_commands() {
            let (reply, _receiver) = GuiReplySender::channel(command.reply_kind());
            let request = GuiRequest::from_parts(command.clone(), reply)
                .unwrap_or_else(|error| panic!("{command:?} could not be rebuilt: {error}"));

            let (recovered, reply) = request.into_parts();

            assert_eq!(recovered, command);
            assert_eq!(reply.kind(), command.reply_kind());
        }
    }

    #[test]
    fn a_reply_channel_of_the_wrong_shape_is_refused_rather_than_mis_wired() {
        let (reply, _receiver) = GuiReplySender::channel(GuiReplyKind::Unit);

        let error = GuiRequest::from_parts(
            GuiCommand::Snapshot {
                columns: 80,
                rows: 24,
            },
            reply,
        )
        .err()
        .expect("a mismatched reply channel must be refused");

        assert!(error.contains("Snapshot"), "{error}");
        assert!(error.contains("Unit"), "{error}");
    }

    #[tokio::test]
    async fn a_rebuilt_request_answers_on_the_channel_its_command_asked_for() {
        let command = GuiCommand::UpdateChatInput {
            expected_input: "before".to_string(),
            expected_cursor: 3,
            input: "after".to_string(),
            cursor: 5,
            action: None,
        };
        let (reply, receiver) = GuiReplySender::channel(command.reply_kind());
        let request = GuiRequest::from_parts(command, reply).unwrap();

        let GuiRequest::UpdateChatInput { input, reply, .. } = request else {
            panic!("an UpdateChatInput command must rebuild an UpdateChatInput request");
        };
        assert_eq!(input, "after");
        let state = GuiChatInputState {
            input,
            cursor: 5,
            revision: 9,
        };
        reply.send(Ok(state.clone())).unwrap();

        assert_eq!(
            receiver.recv().await.unwrap(),
            Some(GuiReply::ChatInputState(Ok(state)))
        );
    }

    #[tokio::test]
    async fn a_fire_and_forget_command_has_nothing_to_wait_for() {
        let command = GuiCommand::Shutdown;
        let (reply, receiver) = GuiReplySender::channel(command.reply_kind());

        let request = GuiRequest::from_parts(command, reply).unwrap();

        assert!(matches!(request, GuiRequest::Shutdown));
        assert_eq!(receiver.recv().await.unwrap(), None);
    }

    /// A stand-in for the editor loop that answers whatever shape it is asked
    /// for and reports back what it was asked.
    ///
    /// Keeping the responder scripted rather than running a real editor keeps
    /// these tests about the bridge and the transport underneath it.
    fn spawn_scripted_editor(
        mut requests: mpsc::UnboundedReceiver<GuiRequest>,
        snapshot: GuiSnapshot,
    ) -> tokio::task::JoinHandle<Vec<GuiCommand>> {
        tokio::spawn(async move {
            let mut received = Vec::new();
            while let Some(request) = requests.recv().await {
                let (command, reply) = request.into_parts();
                let stopping = matches!(command, GuiCommand::Shutdown);
                received.push(command);
                match reply {
                    GuiReplySender::None => {}
                    GuiReplySender::Unit(tx) => {
                        let _ = tx.send(Ok(()));
                    }
                    GuiReplySender::Snapshot(tx) => {
                        let _ = tx.send(Ok(snapshot.clone()));
                    }
                    GuiReplySender::VectorSource(tx) => {
                        let _ = tx.send(Ok(GuiVectorSource {
                            source: "documentsize 24x24\n".to_string(),
                            file_name: "close.strok".to_string(),
                        }));
                    }
                    GuiReplySender::WorkspaceDirectory(tx) => {
                        let _ = tx.send(Ok(PathBuf::from("/srv/project")));
                    }
                    GuiReplySender::Clipboard(tx) => {
                        let _ = tx.send(Ok(GuiClipboardText {
                            generation: 1,
                            text: "yanked".to_string(),
                            writer: None,
                        }));
                    }
                    GuiReplySender::ChatInputState(tx) => {
                        let _ = tx.send(Ok(GuiChatInputState {
                            input: "draft".to_string(),
                            cursor: 5,
                            revision: 2,
                        }));
                    }
                }
                if stopping {
                    break;
                }
            }
            received
        })
    }

    fn local_bridge(
        snapshot: GuiSnapshot,
    ) -> (GuiBridge, tokio::task::JoinHandle<Vec<GuiCommand>>) {
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (update_tx, _) = watch::channel(None);
        let editor = spawn_scripted_editor(request_rx, snapshot);
        (
            GuiBridge::new(Arc::new(LocalTransport::new(request_tx, update_tx))),
            editor,
        )
    }

    #[tokio::test]
    async fn a_bridge_over_the_local_transport_gets_every_reply_shape_back() {
        // One call per reply shape, because the shape is what the bridge has
        // to unwrap; the fire-and-forget shutdown is included because it is
        // the only path where "no answer" is the correct answer.
        let projected = super::super::snapshot(&Editor::with_content("fn main() {}\n"), 7, 0);
        let (bridge, editor) = local_bridge(projected.clone());

        assert_eq!(bridge.snapshot(120, 40).await.unwrap(), projected);
        assert_eq!(
            bridge.vector_source().await.unwrap().file_name,
            "close.strok"
        );
        assert_eq!(
            bridge.workspace_directory().await.unwrap(),
            PathBuf::from("/srv/project")
        );
        bridge
            .editor_command("set number".to_string())
            .await
            .unwrap();
        bridge.shutdown();

        let received = editor.await.unwrap();
        assert_eq!(
            received,
            vec![
                GuiCommand::Snapshot {
                    columns: 120,
                    rows: 40
                },
                GuiCommand::VectorSource,
                GuiCommand::WorkspaceDirectory,
                GuiCommand::EditorCommand {
                    command: "set number".to_string()
                },
                GuiCommand::Shutdown,
            ]
        );
    }

    #[tokio::test]
    async fn the_local_transport_publishes_the_snapshots_its_editor_sends() {
        let (request_tx, _request_rx) = mpsc::unbounded_channel();
        let (update_tx, _) = watch::channel(None);
        let bridge = GuiBridge::new(Arc::new(LocalTransport::new(request_tx, update_tx.clone())));
        let mut updates = bridge.subscribe();

        let projected = super::super::snapshot(&Editor::with_content("hello\n"), 3, 0);
        update_tx.send_replace(Some(projected.clone()));

        updates.changed().await.unwrap();
        assert_eq!(updates.borrow_and_update().clone(), Some(projected));
    }

    /// A transport that records the commands handed to it and answers each one
    /// with a canned value of the shape the command declared.
    ///
    /// This is what catches a typed helper wired to the wrong variant, and it
    /// is the harness a remote transport will be tested against.
    pub(crate) struct RecordingTransport {
        received: Mutex<Vec<GuiCommand>>,
        snapshot: GuiSnapshot,
        updates: watch::Sender<Option<GuiSnapshot>>,
        /// An answer for a command whose canned reply is not specific enough.
        #[allow(clippy::type_complexity)]
        scripted: Option<Box<dyn Fn(&GuiCommand) -> Option<GuiReply> + Send + Sync>>,
        remote: bool,
        connection: watch::Sender<GuiConnection>,
    }

    impl RecordingTransport {
        pub(crate) fn new() -> Arc<Self> {
            let (updates, _) = watch::channel(None);
            Arc::new(Self {
                received: Mutex::new(Vec::new()),
                snapshot: super::super::snapshot(&Editor::with_content("fn main() {}\n"), 1, 0),
                updates,
                scripted: None,
                remote: false,
                connection: watch::channel(GuiConnection::Connected).0,
            })
        }

        /// A transport that reports itself remote and scripts some answers.
        pub(crate) fn remote_answering(
            answer: impl Fn(&GuiCommand) -> Option<GuiReply> + Send + Sync + 'static,
        ) -> Self {
            let (updates, _) = watch::channel(None);
            Self {
                received: Mutex::new(Vec::new()),
                snapshot: super::super::snapshot(&Editor::with_content("fn main() {}\n"), 1, 0),
                updates,
                scripted: Some(Box::new(answer)),
                remote: true,
                connection: watch::channel(GuiConnection::Connected).0,
            }
        }

        pub(crate) fn received(&self) -> Vec<GuiCommand> {
            self.received.lock().unwrap().clone()
        }

        /// Publish `frame` as the latest one, the way a stream would.
        pub(crate) fn show(&self, frame: GuiSnapshot) {
            self.updates.send_replace(Some(frame));
        }

        /// Move the connection state, the way the supervisor would.
        pub(crate) fn set_connection(&self, state: GuiConnection) {
            self.connection.send_replace(state);
        }
    }

    impl GuiTransport for RecordingTransport {
        fn send(
            &self,
            command: GuiCommand,
        ) -> GuiTransportFuture<'_, Result<Option<GuiReply>, String>> {
            let kind = command.reply_kind();
            let scripted = self.scripted.as_ref().and_then(|answer| answer(&command));
            self.received.lock().unwrap().push(command);
            let snapshot = self.snapshot.clone();
            Box::pin(async move {
                if let Some(scripted) = scripted {
                    return Ok(Some(scripted));
                }
                Ok(match kind {
                    GuiReplyKind::None => None,
                    GuiReplyKind::Unit => Some(GuiReply::Unit(Ok(()))),
                    GuiReplyKind::Snapshot => Some(GuiReply::Snapshot(Box::new(Ok(snapshot)))),
                    GuiReplyKind::VectorSource => {
                        Some(GuiReply::VectorSource(Ok(GuiVectorSource {
                            source: "documentsize 24x24\n".to_string(),
                            file_name: "close.strok".to_string(),
                        })))
                    }
                    GuiReplyKind::WorkspaceDirectory => Some(GuiReply::WorkspaceDirectory(Ok(
                        PathBuf::from("/srv/project"),
                    ))),
                    GuiReplyKind::ChatInputState => {
                        Some(GuiReply::ChatInputState(Ok(GuiChatInputState {
                            input: "draft".to_string(),
                            cursor: 5,
                            revision: 2,
                        })))
                    }
                    GuiReplyKind::Clipboard => {
                        Some(GuiReply::Clipboard(Ok(protocol::GuiClipboardText {
                            generation: 1,
                            text: "yanked".to_string(),
                            writer: None,
                        })))
                    }
                })
            })
        }

        fn send_oneway(&self, command: GuiCommand) -> Result<(), String> {
            self.received.lock().unwrap().push(command);
            Ok(())
        }

        fn subscribe(&self) -> watch::Receiver<Option<GuiSnapshot>> {
            self.updates.subscribe()
        }

        fn is_remote(&self) -> bool {
            self.remote
        }

        fn connection(&self) -> watch::Receiver<GuiConnection> {
            self.connection.subscribe()
        }

        fn client_id(&self) -> Option<String> {
            self.remote.then(|| THIS_WINDOW.to_string())
        }
    }

    /// The client id a remote [`RecordingTransport`] reports.
    pub(crate) const THIS_WINDOW: &str = "this-window";

    /// What [`exercise_every_helper`] should put on the wire, in order.
    ///
    /// `sample_commands` is the protocol's own list of one value per variant,
    /// built with the same field values the sweep uses, so deriving the
    /// expectation from it keeps the two from drifting apart. Its deliberate
    /// trailing duplicate is dropped here; everything else must appear once.
    ///
    /// Over a remote link `attach_images` sends the files' bytes rather than
    /// their local paths, so the path-carrying `AttachImages` never reaches
    /// the wire from the sweep; `remote` drops it from the expectation.
    pub(crate) fn every_command_once(remote: bool) -> Vec<GuiCommand> {
        let mut seen = HashSet::new();
        protocol::sample_commands()
            .into_iter()
            .filter(|command| seen.insert(discriminant(command)))
            .filter(|command| !(remote && matches!(command, GuiCommand::AttachImages { .. })))
            .collect()
    }

    /// Call every typed helper once, with a distinct argument per field.
    pub(crate) async fn exercise_every_helper(bridge: &GuiBridge) {
        let key = GuiKeyInput {
            key: "j".to_string(),
            shift: true,
            control: false,
            alt: true,
            meta: false,
        };
        let action = GuiKeyInput {
            key: "Enter".to_string(),
            shift: false,
            control: true,
            alt: false,
            meta: true,
        };
        bridge
            .code_lens(1, "src/lib.rs".to_string(), 17, 2, 9, "run".to_string())
            .await
            .unwrap();
        bridge.snapshot(132, 44).await.unwrap();
        bridge.vector_source().await.unwrap();
        bridge
            .vector_feedback("lighter stroke".to_string())
            .await
            .unwrap();
        bridge.workspace_directory().await.unwrap();
        bridge
            .diff_comment(
                2,
                31,
                GuiDiffCommentInput {
                    id: Some(4),
                    path: "src/main.rs".to_string(),
                    line: 12,
                    side: "new".to_string(),
                    before: true,
                    text: "why?".to_string(),
                },
            )
            .await
            .unwrap();
        bridge
            .diff_action(3, 32, "toggleLayout".to_string())
            .await
            .unwrap();
        bridge
            .open_diff_source(
                1,
                33,
                "src/main.rs".to_string(),
                40,
                "old".to_string(),
                Some(6),
            )
            .await
            .unwrap();
        bridge.key(key).await.unwrap();
        bridge.open_ai_chat().await.unwrap();
        bridge
            .update_chat_input(
                "before".to_string(),
                3,
                "after".to_string(),
                5,
                Some(action),
            )
            .await
            .unwrap();
        bridge.set_chat_input_cursor(7).await.unwrap();
        bridge.set_chat_input_width(96).await.unwrap();
        bridge.remove_chat_image(2).await.unwrap();
        bridge
            .select_ai_profile("anthropic".to_string(), Some("claude-opus".to_string()))
            .await
            .unwrap();
        bridge
            .select_reasoning_effort("high".to_string())
            .await
            .unwrap();
        bridge
            .select_permission_mode("plan".to_string())
            .await
            .unwrap();
        bridge.replay_chat_tool("call-8".to_string()).await.unwrap();
        bridge.select_chat_message(4).await.unwrap();
        bridge
            .manage_queued_chat_input(11, "cancel".to_string())
            .await
            .unwrap();
        bridge
            .ai_policy("allow".to_string(), Some("bash".to_string()))
            .await
            .unwrap();
        bridge
            .editor_command("set number".to_string())
            .await
            .unwrap();
        bridge
            .select_chat_agent(Some("agent-9".to_string()))
            .await
            .unwrap();
        bridge.paste("pasted text".to_string()).await.unwrap();
        if !bridge.is_remote() {
            bridge
                .attach_images(vec![
                    PathBuf::from("diagram.png"),
                    PathBuf::from("photo.jpeg"),
                ])
                .await
                .unwrap();
        }
        bridge
            .attach_image_data("pasted-image.png".to_string(), vec![137, 80, 78, 71])
            .await
            .unwrap();
        bridge.position_walkthrough(3, 12).await.unwrap();
        bridge.set_cursor(1, 42, 8).await.unwrap();
        bridge
            .mouse(2, 5, 14, GuiMouseKind::Drag, 2, true)
            .await
            .unwrap();
        bridge.select_tab(3).await.unwrap();
        bridge.scroll(1, -7).await.unwrap();
        bridge.focus_pane(2).await.unwrap();
        bridge.select_picker(6).await.unwrap();
        bridge.select_completion(1, true).await.unwrap();
        bridge.select_file_tree(12, false).await.unwrap();
        bridge
            .select_problem("diagnostics".to_string(), 5, true)
            .await
            .unwrap();
        bridge.select_lsp(9, false).await.unwrap();
        bridge
            .search_replace_row(8, "toggle".to_string())
            .await
            .unwrap();
        bridge
            .debug_panel_row(13, "expand".to_string())
            .await
            .unwrap();
        bridge.toggle_breakpoint(21).await.unwrap();
        bridge.toggle_fold(30).await.unwrap();
        bridge.select_debug_frame(2).await.unwrap();
        bridge.read_clipboard().await.unwrap();
        bridge
            .write_clipboard("clipboard text".to_string())
            .await
            .unwrap();
        bridge.shutdown();
    }

    #[tokio::test]
    async fn a_remote_bridge_sends_a_dropped_image_as_bytes_not_as_a_local_path() {
        // The path names a file on this machine; an editor on another host
        // would look for it on its own disk.
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("diagram.png");
        std::fs::write(&image, [137, 80, 78, 71, 1, 2, 3]).unwrap();
        let transport = Arc::new(RecordingTransport::remote_answering(|_| None));
        let bridge = GuiBridge::new(transport.clone());

        bridge.attach_images(vec![image]).await.unwrap();
        assert_eq!(
            transport.received(),
            vec![GuiCommand::AttachImageData {
                name: "diagram.png".to_string(),
                data: vec![137, 80, 78, 71, 1, 2, 3],
            }]
        );

        let error = bridge
            .attach_images(vec![directory.path().join("notes.txt")])
            .await
            .unwrap_err();
        assert!(error.contains("Unsupported image format"), "{error}");
    }

    #[tokio::test]
    async fn every_typed_helper_sends_the_command_variant_it_is_named_for() {
        // A helper wired to the wrong variant -- `select_lsp` sending
        // `SelectFileTree`, say -- still compiles and still returns `Ok`, so
        // the recorded conversation is the only thing that catches it.
        let transport = RecordingTransport::new();
        let bridge = GuiBridge::new(transport.clone());

        exercise_every_helper(&bridge).await;

        let expected = every_command_once(false);
        assert_eq!(
            expected.len(),
            protocol::COMMAND_VARIANTS,
            "the sweep should reach every variant"
        );
        assert_eq!(transport.received(), expected);
    }

    #[tokio::test]
    async fn a_stopped_editor_reads_the_same_on_every_reply_shape() {
        // The wording reaches the user as a Tauri command error, so it is part
        // of the interface and not an implementation detail.
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (update_tx, _) = watch::channel(None);
        let bridge = GuiBridge::new(Arc::new(LocalTransport::new(request_tx, update_tx)));
        drop(request_rx);

        assert_eq!(
            bridge.snapshot(80, 24).await,
            Err(EDITOR_STOPPED.to_string())
        );
        assert_eq!(bridge.vector_source().await.unwrap_err(), EDITOR_STOPPED);
        assert_eq!(
            bridge.workspace_directory().await.unwrap_err(),
            EDITOR_STOPPED
        );
        assert_eq!(bridge.select_tab(1).await, Err(EDITOR_STOPPED.to_string()));
        // Shutdown swallows the failure: the editor is already gone, which is
        // what shutdown was asking for.
        bridge.shutdown();
    }

    #[tokio::test]
    async fn an_editor_that_drops_the_answer_reads_as_a_closed_response() {
        let (request_tx, mut request_rx) = mpsc::unbounded_channel();
        let (update_tx, _) = watch::channel(None);
        let bridge = GuiBridge::new(Arc::new(LocalTransport::new(request_tx, update_tx)));
        tokio::spawn(async move {
            // Take the request and drop it, reply channel and all, the way a
            // panicking editor loop would.
            let _ = request_rx.recv().await;
        });

        assert_eq!(
            bridge
                .key(GuiKeyInput {
                    key: "j".to_string(),
                    shift: false,
                    control: false,
                    alt: false,
                    meta: false,
                })
                .await,
            Err(REPLY_CLOSED.to_string())
        );
    }

    /// A transport that answers everything with a unit reply, standing in for
    /// a peer whose protocol has drifted.
    struct WrongShapeTransport(watch::Sender<Option<GuiSnapshot>>);

    impl GuiTransport for WrongShapeTransport {
        fn send(
            &self,
            _command: GuiCommand,
        ) -> GuiTransportFuture<'_, Result<Option<GuiReply>, String>> {
            Box::pin(async { Ok(Some(GuiReply::Unit(Ok(())))) })
        }

        fn send_oneway(&self, _command: GuiCommand) -> Result<(), String> {
            Ok(())
        }

        fn subscribe(&self) -> watch::Receiver<Option<GuiSnapshot>> {
            self.0.subscribe()
        }
    }

    #[tokio::test]
    async fn a_reply_of_the_wrong_shape_is_reported_rather_than_panicked_on() {
        let (updates, _) = watch::channel(None);
        let bridge = GuiBridge::new(Arc::new(WrongShapeTransport(updates)));

        let error = bridge.snapshot(80, 24).await.unwrap_err();

        assert!(error.contains("Unit"), "{error}");
        assert!(error.contains("Snapshot"), "{error}");
    }

    #[test]
    fn a_command_that_expects_an_answer_cannot_be_sent_one_way() {
        // `send_oneway` skips the reply channel entirely, so letting a
        // command through it would drop that command's answer in silence.
        let (request_tx, _request_rx) = mpsc::unbounded_channel();
        let (update_tx, _) = watch::channel(None);
        let transport = LocalTransport::new(request_tx, update_tx);

        let error = transport
            .send_oneway(GuiCommand::SelectTab { index: 1 })
            .unwrap_err();

        assert!(error.contains("Unit"), "{error}");
    }
}
