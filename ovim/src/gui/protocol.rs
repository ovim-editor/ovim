//! The serializable half of the GUI protocol.
//!
//! Everything a frontend can ask Ovim to do is expressed here as plain data:
//! [`GuiCommand`] is a request with its reply channel removed, and
//! [`GuiReply`] is the answer. The frame itself, [`GuiSnapshot`], is declared
//! beside the projection that builds it in `gui::mod`; every type in it
//! derives `Deserialize` as well as `Serialize` so a frontend on another host
//! can read it back. Because nothing here touches Tauri, an editor loop can
//! produce these values with the GUI feature switched off, which is what lets
//! the same conversation run over a socket instead of an in-process channel.
//!
//! The wire format is JSON and it is consumed by the TypeScript frontend in
//! `ovim/gui/src`, so field renames here are breaking changes there.

use super::diff::GuiDiffCommentInput;
use super::{GuiChatInputState, GuiKeyInput, GuiMouseKind, GuiSnapshot, GuiVectorSource};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The Server-Sent Events name every snapshot frame is published under.
///
/// Shared by the route that writes the stream and the transport that reads it,
/// so a rename cannot leave one end silently ignoring the other.
pub const SNAPSHOT_EVENT: &str = "snapshot";

/// The version of the conversation this build speaks.
///
/// Raised whenever a [`GuiCommand`], a [`GuiReply`] or any type inside a frame
/// changes shape. Both ends compare it before anything else crosses the link:
/// serde ignores fields it does not know and a frame that fails to decode is
/// only logged, so without this a client and a session from different builds
/// can stay "connected" while frames go missing or lose fields silently.
pub const GUI_PROTOCOL_VERSION: u32 = 1;

/// The header a client names its protocol version in.
pub const PROTOCOL_HEADER: &str = "x-ovim-gui-protocol";

/// The header a client names itself in, so the session can say which client a
/// clipboard write came from. An opaque random value per window; it proves
/// nothing and is only compared for equality.
pub const CLIENT_HEADER: &str = "x-ovim-gui-client";

/// What `GET /v1/gui/protocol` answers: the handshake a client makes before it
/// subscribes, on every connect and every reconnect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiProtocolInfo {
    pub protocol: u32,
    /// The Ovim release on that side, only for the error message.
    pub ovim_version: String,
}

impl GuiProtocolInfo {
    /// This build's side of the handshake.
    pub fn current() -> Self {
        Self {
            protocol: GUI_PROTOCOL_VERSION,
            ovim_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// The refusal to show when `theirs` is not this build's protocol.
    pub fn mismatch(theirs: &Self) -> Option<String> {
        let ours = Self::current();
        (theirs.protocol != ours.protocol).then(|| {
            format!(
                "The remote Ovim session speaks GUI protocol {} (Ovim {}), and this window speaks \
                 protocol {} (Ovim {}). They cannot understand each other's frames. Install the \
                 same Ovim release on both machines, then start a new session with --fresh.",
                theirs.protocol, theirs.ovim_version, ours.protocol, ours.ovim_version
            )
        })
    }
}

/// The editor's system clipboard, as seen from a frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiClipboard {
    /// How many times the editor has written the system clipboard.
    ///
    /// Rises only for a write the editor made -- so a yank, or a delete under
    /// `unnamedplus`, or an explicit `"+y` whatever the option says. It does
    /// not rise for a clipboard some other program on the editor's host
    /// changed, which is the difference between mirroring the editor and
    /// synchronising two machines' clipboards.
    pub generation: u64,
    /// The client (its [`CLIENT_HEADER`] value) whose command made the write
    /// counted by `generation`, or `None` when it came from anywhere else --
    /// another API caller, a timer, or the editor itself. A frontend mirrors
    /// only its own writes onto its machine's clipboard.
    #[serde(default)]
    pub writer: Option<String>,
}

/// The text the editor last put on its host's system clipboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiClipboardText {
    /// The generation this text belongs to, which a frontend records so a
    /// yank that lands between the frame and this answer is still noticed.
    pub generation: u64,
    pub text: String,
    /// Who made this write; see [`GuiClipboard::writer`].
    #[serde(default)]
    pub writer: Option<String>,
}

/// A [`GuiRequest`](super::GuiRequest) payload with its reply channel removed.
///
/// `GuiRequest` embeds a `oneshot::Sender`, so it can never leave the process.
/// `GuiCommand` carries the same information as data; a transport pairs it
/// with a reply channel of its own to rebuild the request on the editor side.
/// The two enums are kept variant-for-variant identical on purpose — the
/// conversions in `gui::bridge` are exhaustive, so adding a request variant
/// without a command variant fails to compile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "command",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GuiCommand {
    CodeLens {
        pane: usize,
        file_path: String,
        version: usize,
        index: usize,
        generation: u64,
        mode: String,
    },
    Snapshot {
        columns: u16,
        rows: u16,
    },
    VectorSource,
    VectorFeedback {
        feedback: String,
    },
    /// The directory the editor treats as its workspace.
    ///
    /// A path on the editor's host: a frontend on another machine must not
    /// open it locally (the GUI terminal refuses to, over a remote link).
    WorkspaceDirectory,
    DiffComment {
        pane: usize,
        buffer_id: u64,
        comment: GuiDiffCommentInput,
    },
    DiffAction {
        pane: usize,
        buffer_id: u64,
        action: String,
    },
    OpenDiffSource {
        pane: usize,
        buffer_id: u64,
        path: String,
        line: usize,
        side: String,
        definition_column: Option<u32>,
    },
    Key {
        input: GuiKeyInput,
    },
    OpenAiChat,
    UpdateChatInput {
        expected_input: String,
        expected_cursor: usize,
        input: String,
        cursor: usize,
        action: Option<GuiKeyInput>,
    },
    SetChatInputCursor {
        offset: usize,
    },
    SetChatInputWidth {
        columns: usize,
    },
    RemoveChatImage {
        index: usize,
    },
    SelectAiProfile {
        profile: String,
        model: Option<String>,
    },
    SelectReasoningEffort {
        effort: String,
    },
    SelectPermissionMode {
        mode: String,
    },
    ReplayChatTool {
        tool_call_id: String,
    },
    SelectChatMessage {
        index: usize,
    },
    ManageQueuedChatInput {
        id: u64,
        action: String,
    },
    AiPolicy {
        action: String,
        target: Option<String>,
    },
    EditorCommand {
        command: String,
    },
    SelectChatAgent {
        agent_id: Option<String>,
    },
    Paste {
        text: String,
    },
    /// Paths naming files on the host that *runs the editor*.
    ///
    /// A remote frontend cannot use this: its drag-and-drop paths are local.
    /// [`GuiCommand::AttachImageData`] is the transport-independent form, and
    /// the bridge switches to it when the link is remote.
    AttachImages {
        paths: Vec<PathBuf>,
    },
    AttachImageData {
        name: String,
        data: Vec<u8>,
    },
    PositionWalkthrough {
        current: usize,
        rows: usize,
    },
    SetCursor {
        pane: usize,
        line: usize,
        display_column: usize,
    },
    Mouse {
        pane: usize,
        line: usize,
        display_column: usize,
        kind: GuiMouseKind,
        click_count: u8,
        shift: bool,
    },
    SelectTab {
        index: usize,
    },
    Scroll {
        pane: usize,
        lines: i32,
    },
    FocusPane {
        index: usize,
    },
    SelectPicker {
        index: usize,
    },
    SelectCompletion {
        index: usize,
        activate: bool,
    },
    SelectFileTree {
        index: usize,
        activate: bool,
    },
    SelectProblem {
        kind: String,
        index: usize,
        activate: bool,
    },
    SelectLsp {
        index: usize,
        activate: bool,
    },
    SearchReplaceRow {
        index: usize,
        action: String,
    },
    DebugPanelRow {
        index: usize,
        action: String,
    },
    ToggleBreakpoint {
        line: usize,
    },
    ToggleFold {
        line: usize,
    },
    SelectDebugFrame {
        index: usize,
    },
    /// The text the editor last put on its own host's system clipboard.
    ///
    /// Asked for when [`GuiClipboard::generation`] on a frame differs from the
    /// one already mirrored, so the payload crosses the link once per yank
    /// rather than once per frame.
    ReadClipboard,
    /// Put the frontend's system clipboard into the editor's `+` register.
    ///
    /// Sent only immediately ahead of a key the user typed to paste from the
    /// clipboard (see `gui::clipboard::PasteDetector`), never on its own.
    WriteClipboard {
        text: String,
    },
    Shutdown,
}

impl GuiCommand {
    /// The reply this command produces.
    ///
    /// A transport needs the shape before it sends anything, so it can build a
    /// correctly typed reply channel. Deriving it from the command rather than
    /// letting the caller choose keeps the two from drifting apart.
    pub fn reply_kind(&self) -> GuiReplyKind {
        match self {
            GuiCommand::Snapshot { .. } => GuiReplyKind::Snapshot,
            GuiCommand::VectorSource => GuiReplyKind::VectorSource,
            GuiCommand::WorkspaceDirectory => GuiReplyKind::WorkspaceDirectory,
            GuiCommand::UpdateChatInput { .. } => GuiReplyKind::ChatInputState,
            GuiCommand::ReadClipboard => GuiReplyKind::Clipboard,
            GuiCommand::Shutdown => GuiReplyKind::None,
            GuiCommand::CodeLens { .. }
            | GuiCommand::VectorFeedback { .. }
            | GuiCommand::DiffComment { .. }
            | GuiCommand::DiffAction { .. }
            | GuiCommand::OpenDiffSource { .. }
            | GuiCommand::Key { .. }
            | GuiCommand::OpenAiChat
            | GuiCommand::SetChatInputCursor { .. }
            | GuiCommand::SetChatInputWidth { .. }
            | GuiCommand::RemoveChatImage { .. }
            | GuiCommand::SelectAiProfile { .. }
            | GuiCommand::SelectReasoningEffort { .. }
            | GuiCommand::SelectPermissionMode { .. }
            | GuiCommand::ReplayChatTool { .. }
            | GuiCommand::SelectChatMessage { .. }
            | GuiCommand::ManageQueuedChatInput { .. }
            | GuiCommand::AiPolicy { .. }
            | GuiCommand::EditorCommand { .. }
            | GuiCommand::SelectChatAgent { .. }
            | GuiCommand::Paste { .. }
            | GuiCommand::AttachImages { .. }
            | GuiCommand::AttachImageData { .. }
            | GuiCommand::PositionWalkthrough { .. }
            | GuiCommand::SetCursor { .. }
            | GuiCommand::Mouse { .. }
            | GuiCommand::SelectTab { .. }
            | GuiCommand::Scroll { .. }
            | GuiCommand::FocusPane { .. }
            | GuiCommand::SelectPicker { .. }
            | GuiCommand::SelectCompletion { .. }
            | GuiCommand::SelectFileTree { .. }
            | GuiCommand::SelectProblem { .. }
            | GuiCommand::SelectLsp { .. }
            | GuiCommand::SearchReplaceRow { .. }
            | GuiCommand::DebugPanelRow { .. }
            | GuiCommand::ToggleBreakpoint { .. }
            | GuiCommand::ToggleFold { .. }
            | GuiCommand::SelectDebugFrame { .. }
            | GuiCommand::WriteClipboard { .. } => GuiReplyKind::Unit,
        }
    }

    /// Whether this command has to reach the editor in the order it was issued.
    ///
    /// In a modal editor almost every command means whatever the mode, pending
    /// operator, count and cursor make it mean *when it arrives*: `d` before
    /// its motion and `d` after it delete different things, and two characters
    /// that swap places corrupt the buffer with nothing on screen to say so.
    /// So the answer is yes by default, and a new variant added later inherits
    /// the safe side of it.
    ///
    /// The exceptions are the read-only queries. None of them touches editor
    /// state, so where they sit in the sequence cannot change what any other
    /// command does; and their own answer already describes the editor at
    /// whatever moment the session got round to them, which no client could
    /// pin down anyway.
    pub fn must_keep_its_place(&self) -> bool {
        !matches!(
            self,
            GuiCommand::VectorSource | GuiCommand::WorkspaceDirectory | GuiCommand::ReadClipboard
        )
    }
}

/// Which [`GuiReply`] a [`GuiCommand`] answers with.
///
/// `None` is not a reply variant: it means the command is fire-and-forget and
/// the editor never answers at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuiReplyKind {
    None,
    Unit,
    Snapshot,
    VectorSource,
    WorkspaceDirectory,
    ChatInputState,
    Clipboard,
}

/// The answer to a [`GuiCommand`].
///
/// Most commands share a handful of reply shapes, so the reply is tagged by
/// shape rather than by command. Both this enum and [`GuiCommand`] use
/// adjacent tagging: an internal tag would collide with the payload's own
/// fields (`EditorCommand` has a `command` field) and cannot wrap a `Result`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", content = "result", rename_all = "camelCase")]
pub enum GuiReply {
    Unit(Result<(), String>),
    /// Boxed because a projected frame dwarfs the other replies, and every
    /// command that returns one of those would otherwise pay for its size.
    /// `Box` is transparent to serde, so the JSON is unaffected.
    Snapshot(Box<Result<GuiSnapshot, String>>),
    VectorSource(Result<GuiVectorSource, String>),
    /// A directory on the editor's host.
    WorkspaceDirectory(Result<PathBuf, String>),
    /// The composer draft after an atomic edit.
    ChatInputState(Result<GuiChatInputState, String>),
    /// The text the editor last put on its own host's system clipboard.
    Clipboard(Result<GuiClipboardText, String>),
}

impl GuiReply {
    /// The shape of this reply, for matching against [`GuiCommand::reply_kind`].
    pub fn kind(&self) -> GuiReplyKind {
        match self {
            GuiReply::Unit(_) => GuiReplyKind::Unit,
            GuiReply::Snapshot(_) => GuiReplyKind::Snapshot,
            GuiReply::VectorSource(_) => GuiReplyKind::VectorSource,
            GuiReply::WorkspaceDirectory(_) => GuiReplyKind::WorkspaceDirectory,
            GuiReply::ChatInputState(_) => GuiReplyKind::ChatInputState,
            GuiReply::Clipboard(_) => GuiReplyKind::Clipboard,
        }
    }
}

/// A JSON round trip, as a transport would perform it.
///
/// Value equality after the trip is the assertion that matters: a field the
/// wire format drops either fails to deserialize or comes back different.
#[cfg(test)]
pub(super) fn round_trip<T>(value: &T) -> T
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let json = serde_json::to_string(value).expect("GUI protocol values serialize as JSON");
    serde_json::from_str(&json)
        .unwrap_or_else(|error| panic!("GUI protocol value did not deserialize: {error}\n{json}"))
}

/// One value of every [`GuiCommand`] variant.
///
/// Every field carries a distinct, non-default value so that a round trip
/// which loses one is visible rather than accidentally correct.
#[cfg(test)]
pub(super) fn sample_commands() -> Vec<GuiCommand> {
    let plain_key = GuiKeyInput {
        key: "j".to_string(),
        shift: true,
        control: false,
        alt: true,
        meta: false,
    };
    let modified_key = GuiKeyInput {
        key: "Enter".to_string(),
        shift: false,
        control: true,
        alt: false,
        meta: true,
    };
    vec![
        GuiCommand::CodeLens {
            pane: 1,
            file_path: "src/lib.rs".to_string(),
            version: 17,
            index: 2,
            generation: 9,
            mode: "run".to_string(),
        },
        GuiCommand::Snapshot {
            columns: 132,
            rows: 44,
        },
        GuiCommand::VectorSource,
        GuiCommand::VectorFeedback {
            feedback: "lighter stroke".to_string(),
        },
        GuiCommand::WorkspaceDirectory,
        GuiCommand::DiffComment {
            pane: 2,
            buffer_id: 31,
            comment: GuiDiffCommentInput {
                id: Some(4),
                path: "src/main.rs".to_string(),
                line: 12,
                side: "new".to_string(),
                before: true,
                text: "why?".to_string(),
            },
        },
        GuiCommand::DiffAction {
            pane: 3,
            buffer_id: 32,
            action: "toggleLayout".to_string(),
        },
        GuiCommand::OpenDiffSource {
            pane: 1,
            buffer_id: 33,
            path: "src/main.rs".to_string(),
            line: 40,
            side: "old".to_string(),
            definition_column: Some(6),
        },
        GuiCommand::Key {
            input: plain_key.clone(),
        },
        GuiCommand::OpenAiChat,
        GuiCommand::UpdateChatInput {
            expected_input: "before".to_string(),
            expected_cursor: 3,
            input: "after".to_string(),
            cursor: 5,
            action: Some(modified_key),
        },
        GuiCommand::SetChatInputCursor { offset: 7 },
        GuiCommand::SetChatInputWidth { columns: 96 },
        GuiCommand::RemoveChatImage { index: 2 },
        GuiCommand::SelectAiProfile {
            profile: "anthropic".to_string(),
            model: Some("claude-opus".to_string()),
        },
        GuiCommand::SelectReasoningEffort {
            effort: "high".to_string(),
        },
        GuiCommand::SelectPermissionMode {
            mode: "plan".to_string(),
        },
        GuiCommand::ReplayChatTool {
            tool_call_id: "call-8".to_string(),
        },
        GuiCommand::SelectChatMessage { index: 4 },
        GuiCommand::ManageQueuedChatInput {
            id: 11,
            action: "cancel".to_string(),
        },
        GuiCommand::AiPolicy {
            action: "allow".to_string(),
            target: Some("bash".to_string()),
        },
        GuiCommand::EditorCommand {
            command: "set number".to_string(),
        },
        GuiCommand::SelectChatAgent {
            agent_id: Some("agent-9".to_string()),
        },
        GuiCommand::Paste {
            text: "pasted text".to_string(),
        },
        GuiCommand::AttachImages {
            paths: vec![PathBuf::from("diagram.png"), PathBuf::from("photo.jpeg")],
        },
        GuiCommand::AttachImageData {
            name: "pasted-image.png".to_string(),
            data: vec![137, 80, 78, 71],
        },
        GuiCommand::PositionWalkthrough {
            current: 3,
            rows: 12,
        },
        GuiCommand::SetCursor {
            pane: 1,
            line: 42,
            display_column: 8,
        },
        GuiCommand::Mouse {
            pane: 2,
            line: 5,
            display_column: 14,
            kind: GuiMouseKind::Drag,
            click_count: 2,
            shift: true,
        },
        GuiCommand::SelectTab { index: 3 },
        GuiCommand::Scroll { pane: 1, lines: -7 },
        GuiCommand::FocusPane { index: 2 },
        GuiCommand::SelectPicker { index: 6 },
        GuiCommand::SelectCompletion {
            index: 1,
            activate: true,
        },
        GuiCommand::SelectFileTree {
            index: 12,
            activate: false,
        },
        GuiCommand::SelectProblem {
            kind: "diagnostics".to_string(),
            index: 5,
            activate: true,
        },
        GuiCommand::SelectLsp {
            index: 9,
            activate: false,
        },
        GuiCommand::SearchReplaceRow {
            index: 8,
            action: "toggle".to_string(),
        },
        GuiCommand::DebugPanelRow {
            index: 13,
            action: "expand".to_string(),
        },
        GuiCommand::ToggleBreakpoint { line: 21 },
        GuiCommand::ToggleFold { line: 30 },
        GuiCommand::SelectDebugFrame { index: 2 },
        GuiCommand::ReadClipboard,
        GuiCommand::WriteClipboard {
            text: "clipboard text".to_string(),
        },
        GuiCommand::Shutdown,
        GuiCommand::Key { input: plain_key },
    ]
}

/// The number of [`GuiCommand`] variants, which `sample_commands` must cover.
#[cfg(test)]
pub(super) const COMMAND_VARIANTS: usize = 45;

#[cfg(test)]
mod tests {
    use super::super::{GuiCodeExplanationDiscussion, GuiCodeExplanationPage};
    use super::*;
    use std::collections::HashSet;
    use std::mem::discriminant;

    #[test]
    fn the_command_sample_covers_every_variant_exactly_once() {
        // `GuiCommand` mirrors `GuiRequest` one for one, so this count is what
        // stops a new variant from being added without round-trip coverage.
        // The trailing duplicate in the sample is a second `Key` with
        // different modifiers, which is deliberate.
        let commands = sample_commands();
        let distinct: HashSet<_> = commands.iter().map(discriminant).collect();
        assert_eq!(distinct.len(), COMMAND_VARIANTS);
    }

    #[test]
    fn every_command_variant_survives_a_json_round_trip() {
        for command in sample_commands() {
            assert_eq!(round_trip(&command), command);
        }
    }

    #[test]
    fn a_command_is_tagged_by_name_with_its_payload_beside_it() {
        // Adjacent tagging is load-bearing: `EditorCommand` owns a field
        // called `command`, which an internal tag of the same name would
        // shadow, and it is the reason the payload sits under its own key.
        let json = serde_json::to_value(GuiCommand::EditorCommand {
            command: "set number".to_string(),
        })
        .unwrap();

        assert_eq!(json["command"], "editorCommand");
        assert_eq!(json["payload"]["command"], "set number");
    }

    #[test]
    fn a_command_payload_uses_camel_case_field_names() {
        let json = serde_json::to_value(GuiCommand::SetCursor {
            pane: 1,
            line: 42,
            display_column: 8,
        })
        .unwrap();

        assert_eq!(json["payload"]["displayColumn"], 8);
    }

    #[test]
    fn a_command_that_leaves_the_ordered_queue_is_one_that_only_reads() {
        // Ordering is the default so that a variant added later cannot opt out
        // of it by being forgotten. The exceptions are exactly the queries,
        // and each of those answers with a computed value rather than the
        // `Unit` an editor-state change comes back as.
        for command in sample_commands() {
            let reads_only = matches!(
                command.reply_kind(),
                GuiReplyKind::VectorSource
                    | GuiReplyKind::WorkspaceDirectory
                    | GuiReplyKind::Clipboard
            );
            assert_eq!(command.must_keep_its_place(), !reads_only, "{command:?}");
        }
    }

    #[test]
    fn every_reply_variant_survives_a_json_round_trip() {
        for reply in [
            GuiReply::Unit(Ok(())),
            GuiReply::Unit(Err("no chat is open".to_string())),
            GuiReply::VectorSource(Ok(GuiVectorSource {
                source: "documentsize 24x24\n".to_string(),
                file_name: "close.strok".to_string(),
            })),
            GuiReply::VectorSource(Err("not a vector buffer".to_string())),
            GuiReply::WorkspaceDirectory(Ok(PathBuf::from("/srv/project"))),
            GuiReply::WorkspaceDirectory(Err("no workspace".to_string())),
            GuiReply::ChatInputState(Ok(GuiChatInputState {
                input: "draft".to_string(),
                cursor: 3,
                revision: 12,
            })),
            GuiReply::ChatInputState(Err("chat is closed".to_string())),
            GuiReply::Snapshot(Box::new(Err("the editor stopped".to_string()))),
            GuiReply::Clipboard(Ok(GuiClipboardText {
                generation: 7,
                text: "yanked".to_string(),
                writer: Some("0123456789abcdef".to_string()),
            })),
            GuiReply::Clipboard(Err("the yank is too large to bridge".to_string())),
        ] {
            assert_eq!(round_trip(&reply), reply);
        }
    }

    #[test]
    fn a_reply_reports_the_shape_its_command_asked_for() {
        for command in sample_commands() {
            let kind = command.reply_kind();
            let reply = match kind {
                GuiReplyKind::None => continue,
                GuiReplyKind::Unit => GuiReply::Unit(Ok(())),
                GuiReplyKind::Snapshot => GuiReply::Snapshot(Box::new(Err("unused".to_string()))),
                GuiReplyKind::VectorSource => GuiReply::VectorSource(Err("unused".to_string())),
                GuiReplyKind::WorkspaceDirectory => {
                    GuiReply::WorkspaceDirectory(Err("unused".to_string()))
                }
                GuiReplyKind::ChatInputState => GuiReply::ChatInputState(Err("unused".to_string())),
                GuiReplyKind::Clipboard => GuiReply::Clipboard(Err("unused".to_string())),
            };
            assert_eq!(reply.kind(), kind, "{command:?}");
        }
    }

    #[test]
    fn internally_tagged_panels_round_trip_through_their_own_tag_field() {
        // These two enums are the only snapshot types that carry an internal
        // tag, and internal tagging is the serde form most likely to break
        // when `Deserialize` is added, so they are checked directly rather
        // than only through whatever a live editor happens to produce.
        for page in [
            GuiCodeExplanationPage::Concept {
                title: "Ropes".to_string(),
                body: "Text is stored as a rope.".to_string(),
            },
            GuiCodeExplanationPage::Code {
                path: "ovim-core/src/buffer/mod.rs".to_string(),
                start_line: 10,
                end_line: 24,
                comment: "The rope lives here.".to_string(),
            },
            GuiCodeExplanationPage::Diff {
                title: "Rename".to_string(),
                old_path: Some("src/lib.rs".to_string()),
                new_path: None,
                comment: "Moved.".to_string(),
            },
        ] {
            assert_eq!(round_trip(&page), page);
        }

        for discussion in [
            GuiCodeExplanationDiscussion::Navigating {
                question_count: 2,
                exchange_total: 5,
                latest_question: Some("Why a rope?".to_string()),
                latest_answer: Some("Cheap edits.".to_string()),
                latest_failed: true,
            },
            GuiCodeExplanationDiscussion::Composing {
                input: "How does undo work".to_string(),
                cursor: 4,
                question_count: 3,
            },
            GuiCodeExplanationDiscussion::Answering {
                question: "How does undo work?".to_string(),
                answer: "It snapshots the rope.".to_string(),
                question_count: 3,
            },
        ] {
            assert_eq!(round_trip(&discussion), discussion);
        }
    }

    #[test]
    fn a_key_input_keeps_modifiers_that_default_to_false_when_absent() {
        // The modifier fields carry `#[serde(default)]`, so dropping one from
        // the wire format would silently deserialize as `false` instead of
        // failing. Asserting on a value with mixed modifiers is what makes
        // that visible.
        let input = GuiKeyInput {
            key: "k".to_string(),
            shift: true,
            control: false,
            alt: true,
            meta: false,
        };

        assert_eq!(round_trip(&input), input);
        assert!(serde_json::to_string(&input)
            .unwrap()
            .contains("\"alt\":true"));
    }
}
