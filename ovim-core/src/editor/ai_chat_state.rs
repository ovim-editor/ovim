use crate::ai::chat_types::{ChatFocus, ChatOpts};
use crate::mode::Mode;

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
        }
    }
}

pub struct PendingAiChatJob {
    pub receiver: tokio::sync::oneshot::Receiver<anyhow::Result<String>>,
    pub task: tokio::task::JoinHandle<anyhow::Result<String>>,
    pub profile_name: String,
    pub model_name: String,
}

pub struct ScratchBufferState {
    pub scratch_buffer_index: usize,
    pub original_buffer_index: usize,
    pub original_input: String,
}
