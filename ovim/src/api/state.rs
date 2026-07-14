use ovim_core::{KeyCode, KeyEvent, Modifiers};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

/// Request types that can be sent to the editor
#[derive(Debug)]
pub enum ApiRequest {
    GetSnapshot(oneshot::Sender<ApiResponse>),
    /// Lightweight snapshot: returns mode, cursor, hover_info but skips buffer content
    GetSnapshotLight(oneshot::Sender<ApiResponse>),
    SendKeys(String, oneshot::Sender<ApiResponse>),
    Paste(String, oneshot::Sender<ApiResponse>),
    Resize {
        width: u16,
        height: u16,
        tx: oneshot::Sender<ApiResponse>,
    },
    GetBuffer(oneshot::Sender<ApiResponse>),
    SetBuffer(String, oneshot::Sender<ApiResponse>),
    GetCursor(oneshot::Sender<ApiResponse>),
    GetMode(oneshot::Sender<ApiResponse>),
    SetMode(String, oneshot::Sender<ApiResponse>),
    ExecuteCommand(String, oneshot::Sender<ApiResponse>),
    GetRender {
        width: u16,
        height: u16,
        plain: bool,
        tx: oneshot::Sender<ApiResponse>,
    },
    GetLspStatus(oneshot::Sender<ApiResponse>),
    GetHealth(oneshot::Sender<ApiResponse>),
    GetMetrics(oneshot::Sender<ApiResponse>),
    GetContextWindow(oneshot::Sender<ApiResponse>),
    GetOutline(oneshot::Sender<ApiResponse>),
    SearchSymbol(String, oneshot::Sender<ApiResponse>),
    GetTrace(oneshot::Sender<ApiResponse>),
    GetDiagnostics(oneshot::Sender<ApiResponse>),
    EditLine {
        line: Option<usize>,
        old: String,
        new: String,
        tx: oneshot::Sender<ApiResponse>,
    },
    InsertLines {
        line: usize,
        before: bool,
        text: String,
        tx: oneshot::Sender<ApiResponse>,
    },
    DeleteLines {
        from: usize,
        to: usize,
        tx: oneshot::Sender<ApiResponse>,
    },
    ReadLines {
        from: usize,
        to: usize,
        tx: oneshot::Sender<ApiResponse>,
    },
}

/// Response types that can be returned from the editor
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ApiResponse {
    Snapshot(EditorSnapshot),
    Buffer(BufferInfo),
    Cursor(CursorPosition),
    Mode(ModeInfo),
    Render(RenderInfo),
    LspStatus(LspStatusInfo),
    Health(HealthInfo),
    Metrics(MetricsInfo),
    ContextWindow(ContextWindowInfo),
    SendKeysResult(SendKeysResult),
    Outline(OutlineInfo),
    SymbolSearch(SymbolSearchInfo),
    Trace(TraceInfo),
    Diagnostics(DiagnosticsInfo),
    Lines(LinesResponse),
    Success(SuccessResponse),
    Error(ErrorResponse),
}

/// Result of send_keys operation with context window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendKeysResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Context window showing result of key operation
    pub context: ContextWindowInfo,
}

/// Complete snapshot of editor state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorSnapshot {
    /// Snapshot schema version. Fields are additive within a version.
    #[serde(default = "default_snapshot_schema_version")]
    pub schema_version: u32,
    pub buffer: BufferInfo,
    pub cursor: CursorPosition,
    pub mode: String,
    pub visual_selection: Option<VisualSelection>,
    pub registers: HashMap<String, String>,
    pub marks: HashMap<String, CursorPosition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picker: Option<PickerInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover_info: Option<String>,
    /// Active AI chat state, including hidden chats that continue running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_chat: Option<AiChatSnapshot>,
    /// Virtual-text decorations (inlay hints, diagnostic EOL markers) currently
    /// attached to the buffer. Emitted as a flat, position-sorted list rather
    /// than a per-line map so consumers can trivially `jq '.decorations'` it.
    ///
    /// Empty when no decorations are active (e.g. before LSP inlay hints
    /// arrive, or for file types without LSP support).
    #[serde(default)]
    pub decorations: Vec<DecorationInfo>,
    /// UI state needed to compare a headless session with the interactive TUI.
    #[serde(default)]
    pub view: ViewSnapshot,
}

fn default_snapshot_schema_version() -> u32 {
    1
}

pub const SNAPSHOT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ViewSnapshot {
    pub viewport_width: Option<u16>,
    pub viewport_height: Option<u16>,
    pub scroll_offset: usize,
    pub scroll_subrow: usize,
    pub tab_count: usize,
    pub current_tab: usize,
    pub window_count: usize,
    pub file_tree_visible: bool,
    pub command_line: String,
    pub command_cursor: usize,
    pub search_query: String,
    pub search_forward: bool,
    pub status: String,
    pub active_session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatSnapshot {
    pub waiting: bool,
    pub input: String,
    #[serde(default)]
    pub input_cursor: usize,
    #[serde(default)]
    pub focus: String,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub review_mode: bool,
    #[serde(default)]
    pub tree_panel_open: bool,
    pub pending_approval: Option<String>,
    pub queued: Vec<QueuedChatSnapshot>,
    pub messages: Vec<AiChatMessageSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedChatSnapshot {
    pub kind: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatMessageSnapshot {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolCallSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallSnapshot {
    pub name: String,
    pub summary: String,
    pub expanded: bool,
    /// Arguments are only included when the tool event is expanded in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
}

/// A single virtual-text decoration projected for external consumers.
///
/// This is a serialization-friendly view of `ovim_core::editor::decoration::Decoration`.
/// Positions are reported in both rope-absolute (`char_offset`) and
/// line-relative (`line`, `col`) forms so callers can pick whichever matches
/// their mental model.
///
/// `source_version` is the buffer version the decoration was anchored to when
/// it was created and is **never mutated** — the renderer projects positions
/// on demand via `project_offset`. Always populated as of phase-05 Step C.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecorationInfo {
    /// 0-indexed line number, derived from `char_offset` via the rope.
    pub line: usize,
    /// Absolute char offset into the rope.
    pub char_offset: usize,
    /// 0-indexed char column within the line.
    pub col: usize,
    /// The virtual text rendered at this position.
    pub text: String,
    /// Producer of this decoration: `"inlay_hint"` or `"diagnostic"`.
    pub source: String,
    /// Where the text is rendered relative to the buffer: `"inline"` or `"eol"`.
    pub placement: String,
    /// Buffer version the decoration is anchored to.  Populated from the
    /// originating LSP request's `buffer_version` and never mutated.
    pub source_version: u64,
}

/// Picker state information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickerInfo {
    pub mode: String,
    pub query: String,
    pub results: Vec<PickerResultInfo>,
    pub selected_index: usize,
}

/// Picker result information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickerResultInfo {
    pub display: String,
    pub location: String,
    pub line: usize,
    pub col: usize,
}

/// Buffer information
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BufferInfo {
    pub content: String,
    pub line_count: usize,
    pub file_path: Option<String>,
}

/// Cursor position
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CursorPosition {
    pub line: usize,
    pub column: usize,
}

/// Mode information
#[derive(Debug, Clone, Serialize, Default)]
pub struct ModeInfo {
    pub mode: String,
}

/// Rendered output with ANSI codes
#[derive(Debug, Clone, Serialize, Default)]
pub struct RenderInfo {
    pub width: u16,
    pub height: u16,
    pub ansi: String,
}

/// LSP status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspStatusInfo {
    pub servers: Vec<LspServerInfoItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<String>,
}

/// Information about a single LSP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerInfoItem {
    pub language: String,
    pub command: String,
    pub state: String,
    pub pending_requests: usize,
    pub has_capabilities: bool,
}

/// Health check information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthInfo {
    pub status: String,
    pub uptime_seconds: u64,
    pub file: Option<String>,
    pub lsp_servers: HashMap<String, String>,
    pub ready: bool,
}

/// Performance metrics information
#[derive(Debug, Clone, Serialize)]
pub struct MetricsInfo {
    pub buffer_line_count: usize,
    pub buffer_byte_size: usize,
    pub syntax_enabled: bool,
    pub is_large_file: bool,
    pub render_count: u64,
    pub last_render_duration_micros: Option<u64>,
    pub last_syntax_duration_micros: Option<u64>,
    pub memory_usage_mb: f64,
    // Input latency percentiles (microseconds)
    pub input_latency_p50_micros: Option<u64>,
    pub input_latency_p95_micros: Option<u64>,
    pub input_latency_p99_micros: Option<u64>,
    pub input_latency_samples: usize,
    // Operation timings (microseconds)
    pub last_lsp_serialize_micros: Option<u64>,
    pub last_git_status_micros: Option<u64>,
    pub last_fold_calc_micros: Option<u64>,
    pub last_diagnostic_query_micros: Option<u64>,
}

/// Context window information (21-line view around cursor)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWindowInfo {
    /// Formatted context window with line numbers and cursor markers
    pub context: String,
    /// Current file name or path
    pub file: Option<String>,
    /// Current mode (NORMAL, INSERT, etc)
    pub mode: String,
    /// Current cursor line (0-indexed)
    pub line: usize,
    /// Current cursor column (0-indexed)
    pub column: usize,
}

// Re-export navigation types from ovim-core
pub use ovim_core::navigation_types::{
    OutlineInfo, OutlineSymbol, SymbolSearchInfo, SymbolSearchResult, TraceInfo, TraceNode,
};

/// Diagnostics information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsInfo {
    pub file: Option<String>,
    pub diagnostics: Vec<DiagnosticItem>,
    pub counts: DiagnosticCounts,
}

/// A single diagnostic item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticItem {
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub severity: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// Diagnostic counts by severity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticCounts {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub hints: usize,
}

/// Response for read-lines: a slice of the buffer with line numbers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinesResponse {
    pub lines: Vec<LineEntry>,
    pub total_lines: usize,
}

/// A single line entry with its 1-indexed line number
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineEntry {
    pub number: usize,
    pub text: String,
}

/// Visual selection range
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualSelection {
    pub start: CursorPosition,
    pub end: CursorPosition,
}

// Re-export from ovim-core
pub use ovim_core::command_result::{ErrorResponse, SuccessResponse};

impl From<ovim_core::CommandResult> for ApiResponse {
    fn from(cr: ovim_core::CommandResult) -> Self {
        match cr {
            ovim_core::CommandResult::Success(s) => ApiResponse::Success(s),
            ovim_core::CommandResult::Error(e) => ApiResponse::Error(e),
        }
    }
}

/// Shared API state
#[derive(Clone)]
pub struct ApiState {
    pub tx: mpsc::Sender<ApiRequest>,
}

impl ApiState {
    pub fn new(tx: mpsc::Sender<ApiRequest>) -> Self {
        Self { tx }
    }
}

/// Parse a key string into KeyEvent
/// Maximum allowed length for key string input to prevent DoS
pub const MAX_KEY_STRING_LENGTH: usize = 100_000;

pub fn parse_key_string(s: &str) -> Result<Vec<KeyEvent>, String> {
    // First, validate input length
    if s.len() > MAX_KEY_STRING_LENGTH {
        return Err(format!(
            "Key string too long. Max length is {} characters",
            MAX_KEY_STRING_LENGTH
        ));
    }

    let mut events = Vec::new();
    let mut chars = s.char_indices().peekable();

    while let Some((byte_index, c)) = chars.next() {
        // Handle escape sequences: \e, \c, \n, \\
        if c == '\\' {
            let Some(&(_, next)) = chars.peek() else {
                events.push(KeyEvent::new(KeyCode::Char('\\'), Modifiers::NONE));
                continue;
            };
            match next {
                'e' => {
                    // \e = Escape
                    events.push(KeyEvent::new(KeyCode::Esc, Modifiers::NONE));
                    chars.next();
                    continue;
                }
                'c' => {
                    // \c = Ctrl+C
                    events.push(KeyEvent::new(KeyCode::Char('c'), Modifiers::CONTROL));
                    chars.next();
                    continue;
                }
                'n' => {
                    // \n = Enter/newline
                    events.push(KeyEvent::new(KeyCode::Enter, Modifiers::NONE));
                    chars.next();
                    continue;
                }
                '\\' => {
                    // \\ = Literal backslash
                    events.push(KeyEvent::new(KeyCode::Char('\\'), Modifiers::NONE));
                    chars.next();
                    continue;
                }
                _ => {
                    // Not a recognized escape sequence, treat backslash as literal
                    events.push(KeyEvent::new(KeyCode::Char('\\'), Modifiers::NONE));
                    continue;
                }
            }
        }

        // Handle special keys with <> notation
        if c == '<' {
            // Find the closing >
            let token_start = byte_index + c.len_utf8();
            if let Some(relative_end) = s[token_start..].find('>') {
                let token_end = token_start + relative_end;
                let key_name = &s[token_start..token_end];
                // Additional length check for special key names
                if key_name.len() > 32 {
                    return Err("Special key name too long".to_string());
                }
                if let Some(event) = parse_special_key(key_name) {
                    events.push(event);
                    while chars.peek().is_some_and(|(index, _)| *index <= token_end) {
                        chars.next();
                    }
                    continue;
                }
            }
        }

        // Regular character
        events.push(KeyEvent::new(KeyCode::Char(c), Modifiers::NONE));
    }

    Ok(events)
}

/// Parse special key names like "CR", "Esc", "C-w"
fn parse_special_key(key_name: &str) -> Option<KeyEvent> {
    let parts: Vec<&str> = key_name.split('-').collect();
    let (modifier_parts, base_name) = parts.split_at(parts.len().saturating_sub(1));
    let base_name = *base_name.first()?;
    let mut modifiers = Modifiers::NONE;
    for modifier in modifier_parts {
        modifiers |= match *modifier {
            "C" | "Ctrl" => Modifiers::CONTROL,
            "S" | "Shift" => Modifiers::SHIFT,
            "A" | "M" | "Alt" => Modifiers::ALT,
            "D" | "Cmd" | "Super" => Modifiers::SUPER,
            _ => return None,
        };
    }

    // Handle function keys: F1-F12
    let code = if let Some(num) = base_name.strip_prefix('F') {
        if let Ok(n) = num.parse::<u8>() {
            if (1..=12).contains(&n) {
                Some(KeyCode::F(n))
            } else {
                None
            }
        } else {
            None
        }
    } else if base_name.chars().count() == 1 {
        Some(KeyCode::Char(base_name.chars().next()?))
    } else {
        match base_name {
            "CR" | "Enter" => Some(KeyCode::Enter),
            "Esc" => Some(KeyCode::Esc),
            "Tab" if modifiers.contains(Modifiers::SHIFT) => {
                modifiers.remove(Modifiers::SHIFT);
                Some(KeyCode::BackTab)
            }
            "Tab" => Some(KeyCode::Tab),
            "BackTab" => Some(KeyCode::BackTab),
            "BS" | "Backspace" => Some(KeyCode::Backspace),
            "Del" | "Delete" => Some(KeyCode::Delete),
            "Up" => Some(KeyCode::Up),
            "Down" => Some(KeyCode::Down),
            "Left" => Some(KeyCode::Left),
            "Right" => Some(KeyCode::Right),
            "Space" => Some(KeyCode::Char(' ')),
            "Home" => Some(KeyCode::Home),
            "End" => Some(KeyCode::End),
            "PageUp" => Some(KeyCode::PageUp),
            "PageDown" => Some(KeyCode::PageDown),
            "Null" => Some(KeyCode::Null),
            _ => None,
        }
    }?;

    Some(KeyEvent::new(code, modifiers))
}

/// Format a 21-line context window around the cursor
///
/// Shows 10 lines above, current line (with >> marker), and 10 lines below
/// Includes line numbers, cursor position indicator (^), and truncates long lines
pub fn format_context_window(
    buffer_content: &str,
    cursor_line: usize,
    cursor_column: usize,
    file_path: Option<&str>,
    mode: &str,
) -> String {
    // `str::lines` returns no items for an empty buffer and drops the logical
    // empty line after a trailing newline. An editor buffer always has at
    // least one logical line, so preserve those rows explicitly.
    let lines: Vec<&str> = buffer_content.split('\n').collect();
    let total_lines = lines.len();

    // Calculate visible range: 10 lines above, current, 10 below
    let start_line = cursor_line.saturating_sub(10);
    let end_line = (cursor_line + 11).min(total_lines);

    // Determine max line number width for padding
    let max_line_num = total_lines.saturating_sub(1).max(cursor_line);
    let line_num_width = max_line_num.to_string().len();

    // Build header
    let file_display = file_path
        .and_then(|p| p.split('/').next_back())
        .unwrap_or("unnamed");
    let header = format!(
        "[ovim: {} | {} | L{}:C{}]",
        file_display,
        mode,
        cursor_line + 1,
        cursor_column + 1
    );

    let mut result = String::new();
    result.push_str(&header);
    result.push('\n');

    // Show context lines
    for line_idx in start_line..end_line {
        let is_current = line_idx == cursor_line;
        let marker = if is_current { ">>" } else { "  " };

        // Line number
        let line_num_str = format!("{:width$}", line_idx + 1, width = line_num_width);
        result.push_str(&format!("{} {} | ", marker, line_num_str));

        // Line content with truncation
        let line = if line_idx < lines.len() {
            let content = lines[line_idx];
            if content.len() > 80 {
                let truncate_at = content
                    .char_indices()
                    .map(|(i, _)| i)
                    .take_while(|&i| i <= 77)
                    .last()
                    .unwrap_or(0);
                format!("{}...", &content[..truncate_at])
            } else {
                content.to_string()
            }
        } else {
            String::new()
        };
        result.push_str(&line);
        result.push('\n');

        // Add cursor indicator for current line
        if is_current && cursor_column <= lines[line_idx].len() {
            let spaces = " ".repeat(marker.len() + 1 + line_num_width + 3 + cursor_column);
            result.push_str(&format!("{}{}\n", spaces, "^"));
        }
    }

    // Add FILE END marker if we're showing the end
    if end_line >= total_lines && total_lines > 0 {
        result.push_str("FILE END\n");
    }

    result
}

#[cfg(test)]
mod context_window_tests {
    use super::{format_context_window, parse_key_string};
    use ovim_core::{KeyCode, Modifiers};

    #[test]
    fn unicode_before_special_key_is_byte_safe() {
        let events = parse_key_string("héllo<Esc>").unwrap();
        assert_eq!(events.len(), 6);
        assert_eq!(events[0].code, KeyCode::Char('h'));
        assert_eq!(events[1].code, KeyCode::Char('é'));
        assert_eq!(events[5].code, KeyCode::Esc);
    }

    #[test]
    fn parses_combined_terminal_modifiers() {
        let events = parse_key_string("<C-S-x><A-Left><D-1><S-Tab>").unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].code, KeyCode::Char('x'));
        assert!(events[0].modifiers.contains(Modifiers::CONTROL));
        assert!(events[0].modifiers.contains(Modifiers::SHIFT));
        assert_eq!(events[1].code, KeyCode::Left);
        assert!(events[1].modifiers.contains(Modifiers::ALT));
        assert_eq!(events[2].code, KeyCode::Char('1'));
        assert!(events[2].modifiers.contains(Modifiers::SUPER));
        assert_eq!(events[3].code, KeyCode::BackTab);
    }

    #[test]
    fn empty_buffer_has_a_current_logical_line() {
        let context = format_context_window("", 0, 0, Some("new.txt"), "NORMAL");
        assert!(context.contains(">> 1 | \n"), "{context}");
        assert!(context.contains('^'), "{context}");
        assert!(context.contains("FILE END"), "{context}");
    }

    #[test]
    fn trailing_newline_preserves_the_empty_final_line() {
        let context = format_context_window("first\n", 1, 0, Some("file.txt"), "NORMAL");
        assert!(context.contains(">> 2 | \n"), "{context}");
    }
}
