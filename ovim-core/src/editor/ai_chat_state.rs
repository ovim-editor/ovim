use crate::ai::chat_types::{ChatFocus, ChatOpts, NodeId, StreamChunk, ToolCallInfo};
use crate::mode::Mode;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// A paused tool call that requires explicit user approval to access
/// paths outside the active project boundary.
pub struct PendingToolApproval {
    pub tool_call: ToolCallInfo,
    pub remaining_tool_calls: Vec<ToolCallInfo>,
    pub model_name: String,
    pub requested_path: PathBuf,
    pub approval_root: PathBuf,
}

/// Viewport state for chat history rendering.
pub struct ChatViewportState {
    /// Row scroll offset from bottom (0 = latest).
    pub row_scroll_from_bottom: usize,
    /// Whether viewport should track latest output automatically.
    pub follow_latest: bool,
    /// Total rendered row count when pinning started.
    pub pinned_base_total_rows: Option<usize>,
}

impl Default for ChatViewportState {
    fn default() -> Self {
        Self {
            row_scroll_from_bottom: 0,
            follow_latest: true,
            pinned_base_total_rows: None,
        }
    }
}

/// Selection state for message-history interactions.
pub struct ChatHistoryState {
    /// Selected node in the active branch, if any.
    ///
    /// Using node identity keeps selection stable when new messages append.
    pub selected_node_id: Option<NodeId>,
}

impl Default for ChatHistoryState {
    fn default() -> Self {
        Self {
            selected_node_id: None,
        }
    }
}

pub struct AiChatState {
    pub opts: ChatOpts,
    /// Buffer ID where the chat was originally opened.
    /// Used for the conversation key — never changes during the session.
    pub origin_buffer_id: usize,
    /// Buffer ID that mutations target. Updated by `open_file`.
    pub active_buffer_id: usize,
    /// Chat input text.
    pub input: String,
    /// Byte offset cursor in input.
    pub input_cursor: usize,
    /// Which zone has focus.
    pub focus: ChatFocus,
    /// Viewport behavior for chat history.
    pub viewport: ChatViewportState,
    /// Message-history selection state.
    pub history: ChatHistoryState,
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
    /// Node IDs of thinking messages that are expanded in the UI.
    pub expanded_thinking: HashSet<NodeId>,
    /// Whether the tree panel sidebar is open.
    pub tree_panel_open: bool,
    /// Cursor index into the flattened DFS tree list.
    pub tree_panel_cursor: usize,
    /// Tool calls accumulated during streaming.
    pub streaming_tool_calls: Vec<ToolCallInfo>,
    /// Number of individual tool calls executed in current turn.
    pub tool_call_count: u16,
    /// Paused tool call awaiting user approval for outside-project access.
    pub pending_tool_approval: Option<PendingToolApproval>,
    /// First-chat-open prompt when session starts outside a git repo.
    pub pending_no_repo_folder_approval: Option<PathBuf>,
    /// Session-scoped roots explicitly approved for outside-project tool access.
    pub approved_external_roots: Vec<PathBuf>,
    /// Tracks which lines each agent turn modified, per buffer.
    pub agent_edits: AgentEditTracker,
    /// Whether the buffer was clean when chat opened (for auto-save guard).
    pub buffer_was_clean_at_chat_start: bool,
    /// Whether review mode is active (chat hidden, buffer full screen).
    pub review_mode: bool,
    /// Current undo group ID for grouping agent edits per turn.
    pub current_undo_group: Option<u64>,
    /// Next undo group ID to assign.
    pub next_undo_group_id: u64,
}

impl AiChatState {
    /// Number of lines in the input text (at least 1).
    pub fn input_line_count(&self) -> usize {
        self.input.lines().count().max(1)
    }

    pub fn new(opts: ChatOpts, active_buffer_id: usize, mode_before: Mode) -> Self {
        let allow_edits = opts.allow_edits;
        Self {
            opts,
            origin_buffer_id: active_buffer_id,
            active_buffer_id,
            input: String::new(),
            input_cursor: 0,
            focus: ChatFocus::TextInput,
            viewport: ChatViewportState::default(),
            history: ChatHistoryState::default(),
            allow_edits,
            waiting: false,
            pending_job: None,
            scratch: None,
            mode_before_chat: mode_before,
            last_escape: None,
            streaming_content: None,
            streaming_thinking: None,
            expanded_thinking: HashSet::new(),
            tree_panel_open: false,
            tree_panel_cursor: 0,
            streaming_tool_calls: Vec::new(),
            tool_call_count: 0,
            pending_tool_approval: None,
            pending_no_repo_folder_approval: None,
            approved_external_roots: Vec::new(),
            agent_edits: AgentEditTracker::new(),
            buffer_was_clean_at_chat_start: false,
            review_mode: false,
            current_undo_group: None,
            next_undo_group_id: 0,
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

/// Tracks which lines the agent modified, per buffer.
/// Ranges are 0-indexed, inclusive: (start_line, end_line).
pub struct AgentEditTracker {
    /// Per-buffer modified line ranges: sorted, non-overlapping.
    pub all_edits: HashMap<usize, Vec<(usize, usize)>>,
}

impl AgentEditTracker {
    pub fn new() -> Self {
        Self {
            all_edits: HashMap::new(),
        }
    }

    /// Record that lines [start..=end] (0-indexed) were modified in the given buffer.
    pub fn record_edit(&mut self, buffer_index: usize, start_line: usize, end_line: usize) {
        let ranges = self.all_edits.entry(buffer_index).or_default();
        ranges.push((start_line, end_line));
        Self::merge_ranges(ranges);
    }

    /// Adjust all tracked ranges after lines were inserted.
    /// `after_line` is 0-indexed: N new lines inserted after this line shift
    /// all ranges with start > after_line by +count.
    pub fn adjust_for_insert(&mut self, buffer_index: usize, after_line: usize, count: usize) {
        if let Some(ranges) = self.all_edits.get_mut(&buffer_index) {
            for range in ranges.iter_mut() {
                if range.0 > after_line {
                    range.0 += count;
                    range.1 += count;
                } else if range.1 > after_line {
                    // Range overlaps the insertion point — extend it
                    range.1 += count;
                }
            }
        }
    }

    /// Adjust all tracked ranges after lines were deleted.
    /// Lines [start..=end] (0-indexed) were removed.
    pub fn adjust_for_delete(&mut self, buffer_index: usize, start_line: usize, end_line: usize) {
        let count = end_line - start_line + 1;
        if let Some(ranges) = self.all_edits.get_mut(&buffer_index) {
            ranges.retain_mut(|range| {
                if range.1 < start_line {
                    // Entirely before deletion — unchanged
                    true
                } else if range.0 > end_line {
                    // Entirely after deletion — shift down
                    range.0 -= count;
                    range.1 -= count;
                    true
                } else if range.0 >= start_line && range.1 <= end_line {
                    // Entirely within deletion — remove
                    false
                } else {
                    // Partially overlapping — shrink
                    if range.0 < start_line {
                        range.1 = start_line.saturating_sub(1);
                    } else {
                        range.0 = start_line;
                        range.1 = range.1.saturating_sub(count);
                    }
                    range.0 <= range.1
                }
            });
        }
    }

    /// Check if a line (0-indexed) in the given buffer was modified by the agent.
    pub fn is_line_modified(&self, buffer_index: usize, line: usize) -> bool {
        if let Some(ranges) = self.all_edits.get(&buffer_index) {
            for &(start, end) in ranges {
                if line >= start && line <= end {
                    return true;
                }
                if start > line {
                    break; // ranges are sorted
                }
            }
        }
        false
    }

    /// Get the next agent edit boundary from a given line (forward or backward).
    pub fn next_edit_boundary(
        &self,
        buffer_index: usize,
        from_line: usize,
        forward: bool,
    ) -> Option<usize> {
        let ranges = self.all_edits.get(&buffer_index)?;
        if forward {
            for &(start, _end) in ranges {
                if start > from_line {
                    return Some(start);
                }
            }
            // Wrap around to first
            ranges.first().map(|&(start, _)| start)
        } else {
            for &(start, _end) in ranges.iter().rev() {
                if start < from_line {
                    return Some(start);
                }
            }
            // Wrap around to last
            ranges.last().map(|&(start, _)| start)
        }
    }

    /// Total number of edit ranges across all buffers.
    pub fn total_edit_count(&self) -> usize {
        self.all_edits.values().map(|v| v.len()).sum()
    }

    /// Number of buffers that have agent edits.
    pub fn edited_buffer_count(&self) -> usize {
        self.all_edits.values().filter(|v| !v.is_empty()).count()
    }

    /// Clear all tracked edits.
    pub fn clear(&mut self) {
        self.all_edits.clear();
    }

    /// Merge overlapping/adjacent ranges in a sorted list.
    fn merge_ranges(ranges: &mut Vec<(usize, usize)>) {
        ranges.sort_by_key(|r| r.0);
        let mut i = 0;
        while i + 1 < ranges.len() {
            // Merge if overlapping or adjacent (end + 1 >= next start)
            if ranges[i].1 + 1 >= ranges[i + 1].0 {
                ranges[i].1 = ranges[i].1.max(ranges[i + 1].1);
                ranges.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }
}
