use anyhow::Result;
use std::sync::{Arc, Mutex};

/// A thread-safe bridge between Lua and the Editor
/// Allows Lua code to safely interact with editor state
#[derive(Clone)]
pub struct EditorBridge {
    inner: Arc<Mutex<EditorBridgeInner>>,
}

struct EditorBridgeInner {
    /// Commands to execute on the editor
    pending_commands: Vec<String>,
    /// Current cursor position (line, column)
    cursor_pos: Option<(usize, usize)>,
    /// Current buffer content (cached)
    buffer_content: Option<String>,
    /// Current mode
    mode: Option<String>,
}

impl EditorBridge {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(EditorBridgeInner {
                pending_commands: Vec::new(),
                cursor_pos: None,
                buffer_content: None,
                mode: None,
            })),
        }
    }

    /// Queue a command to be executed on the editor
    pub fn execute_command(&self, command: String) -> Result<()> {
        // Handle mutex poisoning gracefully by recovering the guard
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.pending_commands.push(command);
        Ok(())
    }

    /// Update the cached cursor position
    pub fn update_cursor(&self, line: usize, column: usize) {
        // Handle mutex poisoning gracefully by recovering the guard
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.cursor_pos = Some((line, column));
    }

    /// Get the current cursor position
    pub fn get_cursor(&self) -> Option<(usize, usize)> {
        // Handle mutex poisoning gracefully by recovering the guard
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.cursor_pos
    }

    /// Update the cached buffer content
    pub fn update_buffer(&self, content: String) {
        // Handle mutex poisoning gracefully by recovering the guard
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.buffer_content = Some(content);
    }

    /// Get the current buffer content
    pub fn get_buffer(&self) -> Option<String> {
        // Handle mutex poisoning gracefully by recovering the guard
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.buffer_content.clone()
    }

    /// Update the cached mode
    pub fn update_mode(&self, mode: String) {
        // Handle mutex poisoning gracefully by recovering the guard
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.mode = Some(mode);
    }

    /// Get the current mode
    pub fn get_mode(&self) -> Option<String> {
        // Handle mutex poisoning gracefully by recovering the guard
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.mode.clone()
    }

    /// Get all pending commands and clear the queue
    pub fn drain_commands(&self) -> Vec<String> {
        // Handle mutex poisoning gracefully by recovering the guard
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.pending_commands.drain(..).collect()
    }

    /// Get a specific line from the buffer
    pub fn get_line(&self, line: usize) -> Option<String> {
        // Handle mutex poisoning gracefully by recovering the guard
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(ref content) = inner.buffer_content {
            content.lines().nth(line).map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Get the number of lines in the buffer
    pub fn get_line_count(&self) -> usize {
        // Handle mutex poisoning gracefully by recovering the guard
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(ref content) = inner.buffer_content {
            content.lines().count()
        } else {
            0
        }
    }
}

impl Default for EditorBridge {
    fn default() -> Self {
        Self::new()
    }
}
