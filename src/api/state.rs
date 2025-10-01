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
    ExecuteCommand(String, oneshot::Sender<ApiResponse>),
}

/// Response types that can be returned from the editor
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ApiResponse {
    Snapshot(EditorSnapshot),
    Buffer(BufferInfo),
    Cursor(CursorPosition),
    Mode(ModeInfo),
    Success(SuccessResponse),
    Error(ErrorResponse),
}

/// Complete snapshot of editor state
#[derive(Debug, Clone, Serialize)]
pub struct EditorSnapshot {
    pub buffer: BufferInfo,
    pub cursor: CursorPosition,
    pub mode: String,
    pub visual_selection: Option<VisualSelection>,
    pub registers: HashMap<String, String>,
    pub marks: HashMap<String, CursorPosition>,
}

/// Buffer information
#[derive(Debug, Clone, Serialize)]
pub struct BufferInfo {
    pub content: String,
    pub line_count: usize,
    pub file_path: Option<String>,
}

/// Cursor position
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPosition {
    pub line: usize,
    pub column: usize,
}

/// Mode information
#[derive(Debug, Clone, Serialize)]
pub struct ModeInfo {
    pub mode: String,
}

/// Visual selection range
#[derive(Debug, Clone, Serialize)]
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
pub fn parse_key_string(s: &str) -> Vec<KeyEvent> {
    let mut events = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Handle special keys
        if c == '<' {
            // Find the closing >
            if let Some(end) = s[i..].find('>') {
                let key_name = &s[i+1..i+end];
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

    events
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
