use crate::ai::chat_types::{ChatFocus, ChatOpts, StreamChunk};
use crate::mode::Mode;
use std::collections::HashSet;

pub struct AiChatState {
    pub opts: ChatOpts,
    /// Buffer ID that was active when chat was opened.
    pub active_buffer_id: usize,
    /// Chat input text.
    pub input: String,
    /// Byte offset cursor in input.
    pub input_cursor: usize,
    /// Which zone has focus.
    pub focus: ChatFocus,
    /// Scroll offset in message history (0 = bottom, increases = older).
    pub message_scroll: usize,
    /// Whether assistant can suggest edits.
    pub allow_edits: bool,
    /// Waiting for AI response.
    pub waiting: bool,
    /// Pending async chat job.
    pub pending_job: Option<PendingAiChatJob>,
    /// Scratch buffer state for <C-g> editing.
    pub scratch: Option<ScratchBufferState>,
    /// Mode the editor was in before opening chat.
    pub mode_before_chat: Mode,
    /// Timestamp of last Esc press (for double-Esc detection).
    pub last_escape: Option<std::time::Instant>,
    /// Accumulated streaming content (committed on Done).
    pub streaming_content: Option<String>,
    /// Accumulated streaming thinking (committed on Done).
    pub streaming_thinking: Option<String>,
    /// Indices of thinking messages that are expanded in the UI.
    pub expanded_thinking: HashSet<usize>,
}

impl AiChatState {
    pub fn new(opts: ChatOpts, active_buffer_id: usize, mode_before: Mode) -> Self {
        let allow_edits = opts.allow_edits;
        Self {
            opts,
            active_buffer_id,
            input: String::new(),
            input_cursor: 0,
            focus: ChatFocus::TextInput,
            message_scroll: 0,
            allow_edits,
            waiting: false,
            pending_job: None,
            scratch: None,
            mode_before_chat: mode_before,
            last_escape: None,
            streaming_content: None,
            streaming_thinking: None,
            expanded_thinking: HashSet::new(),
        }
    }
}

pub struct PendingAiChatJob {
    pub receiver: tokio::sync::mpsc::UnboundedReceiver<StreamChunk>,
    pub task: tokio::task::JoinHandle<()>,
    pub profile_name: String,
    pub model_name: String,
}

pub struct ScratchBufferState {
    pub scratch_buffer_index: usize,
    pub original_buffer_index: usize,
    pub original_input: String,
}
