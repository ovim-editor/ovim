/// Context for command-line mode (`:` commands)
pub struct CommandContext {
    /// Current command being typed
    pub command_line: String,
    /// History of executed commands
    pub command_history: Vec<String>,
    /// Current position in history during navigation
    pub command_history_index: Option<usize>,
}

impl CommandContext {
    pub fn new() -> Self {
        Self {
            command_line: String::new(),
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
