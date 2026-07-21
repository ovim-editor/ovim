use super::SingleLineInput;

/// Context for command-line mode (`:` commands)
pub struct CommandContext {
    /// Current command text and its UTF-8-safe cursor.
    pub input: SingleLineInput,
    /// History of executed commands
    pub command_history: Vec<String>,
    /// Current position in history during navigation
    pub command_history_index: Option<usize>,
}

impl CommandContext {
    pub fn new() -> Self {
        Self {
            input: SingleLineInput::default(),
            command_history: Vec::new(),
            command_history_index: None,
        }
    }
}

impl Default for CommandContext {
    fn default() -> Self {
        Self::new()
    }
}
