use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

/// Request types that can be sent to the editor
#[derive(Debug)]
pub enum ApiRequest {
    GetSnapshot(oneshot::Sender<ApiResponse>),
    SendKeys(String, oneshot::Sender<ApiResponse>),
    GetBuffer(oneshot::Sender<ApiResponse>),
    SetBuffer(String, oneshot::Sender<ApiResponse>),
    GetCursor(oneshot::Sender<ApiResponse>),
    GetMode(oneshot::Sender<ApiResponse>),
    SetMode(String, oneshot::Sender<ApiResponse>),
    ExecuteCommand(String, oneshot::Sender<ApiResponse>),
    GetRender(oneshot::Sender<ApiResponse>),
    GetLspStatus(oneshot::Sender<ApiResponse>),
    GetHealth(oneshot::Sender<ApiResponse>),
    GetMetrics(oneshot::Sender<ApiResponse>),
    GetContextWindow(oneshot::Sender<ApiResponse>),
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
    Success(SuccessResponse),
    Error(ErrorResponse),
}

/// Complete snapshot of editor state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorSnapshot {
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

/// Visual selection range
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualSelection {
    pub start: CursorPosition,
    pub end: CursorPosition,
}

/// Success response
#[derive(Debug, Clone, Serialize)]
pub struct SuccessResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_count: Option<usize>,
}

/// Error response
#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Shared API state
#[derive(Clone)]
pub struct ApiState {
    pub tx: mpsc::UnboundedSender<ApiRequest>,
}

impl ApiState {
    pub fn new(tx: mpsc::UnboundedSender<ApiRequest>) -> Self {
        Self { tx }
    }
}

/// Parse a key string into KeyEvent
/// Maximum allowed length for key string input to prevent DoS
const MAX_KEY_STRING_LENGTH: usize = 1024;

pub fn parse_key_string(s: &str) -> Result<Vec<KeyEvent>, String> {
    // First, validate input length
    if s.len() > MAX_KEY_STRING_LENGTH {
        return Err(format!(
            "Key string too long. Max length is {} characters",
            MAX_KEY_STRING_LENGTH
        ));
    }

    let mut events = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Handle escape sequences: \e, \c, \n, \\
        if c == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            match next {
                'e' => {
                    // \e = Escape
                    events.push(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                    i += 2;
                    continue;
                }
                'c' => {
                    // \c = Ctrl+C
                    events.push(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
                    i += 2;
                    continue;
                }
                'n' => {
                    // \n = Enter/newline
                    events.push(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    i += 2;
                    continue;
                }
                '\\' => {
                    // \\ = Literal backslash
                    events.push(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::NONE));
                    i += 2;
                    continue;
                }
                _ => {
                    // Not a recognized escape sequence, treat backslash as literal
                    events.push(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::NONE));
                    i += 1;
                    continue;
                }
            }
        }

        // Handle special keys with <> notation
        if c == '<' {
            // Find the closing >
            if let Some(end) = s[i..].find('>') {
                let key_name = &s[i + 1..i + end];
                // Additional length check for special key names
                if key_name.len() > 32 {
                    return Err("Special key name too long".to_string());
                }
                if let Some(event) = parse_special_key(key_name) {
                    events.push(event);
                    i += end + 1;
                    continue;
                }
            }
        }

        // Regular character
        events.push(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        i += 1;
    }

    Ok(events)
}

/// Parse special key names like "CR", "Esc", "C-w"
fn parse_special_key(key_name: &str) -> Option<KeyEvent> {
    // Handle Ctrl- prefix
    if key_name.starts_with("C-") && key_name.len() == 3 {
        let c = key_name.chars().nth(2)?;
        return Some(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    // Handle common special keys
    match key_name {
        "CR" | "Enter" => Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        "Esc" => Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        "Tab" => Some(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        "BS" | "Backspace" => Some(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        "Del" | "Delete" => Some(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        "Up" => Some(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        "Down" => Some(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        "Left" => Some(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        "Right" => Some(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        _ => None,
    }
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
    let lines: Vec<&str> = buffer_content.lines().collect();
    let total_lines = lines.len();

    // Calculate visible range: 10 lines above, current, 10 below
    let start_line = if cursor_line > 10 { cursor_line - 10 } else { 0 };
    let end_line = (cursor_line + 11).min(total_lines);

    // Determine max line number width for padding
    let max_line_num = (total_lines - 1).max(cursor_line);
    let line_num_width = max_line_num.to_string().len();

    // Build header
    let file_display = file_path
        .and_then(|p| p.split('/').last())
        .unwrap_or("unnamed");
    let header = format!(
        "[ovim: {} | {} | L{}:C{}]",
        file_display, mode, cursor_line + 1, cursor_column + 1
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
                format!("{}...", &content[..77])
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
