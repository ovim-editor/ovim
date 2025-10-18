use std::collections::HashMap;

/// Manages registers for storing text (yank, delete, etc.)
#[derive(Debug, Clone)]
pub struct RegisterManager {
    /// Named registers (a-z)
    registers: HashMap<char, String>,
    /// The unnamed register (default)
    unnamed: String,
    /// The yank register (0)
    yank: String,
    /// Delete registers (1-9) - circular buffer of recent deletes
    delete_history: Vec<String>,
    /// Special registers
    current_file: String,        // % - current file name
    alternate_file: String,      // # - alternate file name
    last_inserted: String,       // . - last inserted text
    last_command: String,        // : - last command
    last_search: String,         // / - last search pattern
    clipboard: String,           // + and * - system clipboard
}

impl RegisterManager {
    /// Creates a new register manager
    pub fn new() -> Self {
        Self {
            registers: HashMap::new(),
            unnamed: String::new(),
            yank: String::new(),
            delete_history: Vec::new(),
            current_file: String::new(),
            alternate_file: String::new(),
            last_inserted: String::new(),
            last_command: String::new(),
            last_search: String::new(),
            clipboard: String::new(),
        }
    }

    /// Sets a register value
    pub fn set(&mut self, register: Option<char>, value: String) {
        match register {
            None => {
                // Unnamed register - also set as register "
                self.unnamed = value;
            }
            Some('"') => {
                self.unnamed = value;
            }
            Some('0') => {
                self.yank = value;
            }
            Some('+') | Some('*') => {
                // System clipboard
                self.clipboard = value;
                // TODO: Actually integrate with system clipboard using arboard or clipboard crate
            }
            Some('_') => {
                // Black hole register - do nothing
            }
            Some(c) if c.is_ascii_lowercase() => {
                self.registers.insert(c, value);
            }
            Some(c) if c.is_ascii_uppercase() => {
                // Uppercase appends to lowercase register
                let lowercase = c.to_ascii_lowercase();
                self.registers
                    .entry(lowercase)
                    .and_modify(|v| v.push_str(&value))
                    .or_insert(value);
            }
            _ => {}
        }
    }

    /// Gets a register value
    pub fn get(&self, register: Option<char>) -> &str {
        match register {
            None | Some('"') => &self.unnamed,
            Some('0') => &self.yank,
            Some('%') => &self.current_file,
            Some('#') => &self.alternate_file,
            Some('.') => &self.last_inserted,
            Some(':') => &self.last_command,
            Some('/') => &self.last_search,
            Some('+') | Some('*') => &self.clipboard,
            Some('_') => "", // Black hole register always returns empty
            Some(c) if c.is_ascii_digit() => {
                let idx = c.to_digit(10).unwrap() as usize;
                if idx > 0 && idx <= self.delete_history.len() {
                    &self.delete_history[idx - 1]
                } else {
                    ""
                }
            }
            Some(c) if c.is_ascii_lowercase() => {
                self.registers.get(&c).map(|s| s.as_str()).unwrap_or("")
            }
            Some(c) if c.is_ascii_uppercase() => {
                // Uppercase reads from lowercase register
                let lowercase = c.to_ascii_lowercase();
                self.registers.get(&lowercase).map(|s| s.as_str()).unwrap_or("")
            }
            _ => "",
        }
    }

    /// Stores text in the unnamed register and yank register
    pub fn yank(&mut self, text: String) {
        self.unnamed = text.clone();
        self.yank = text;
    }

    /// Stores deleted text in unnamed register and delete history
    pub fn delete(&mut self, text: String) {
        self.unnamed = text.clone();

        // Add to delete history (1-9)
        self.delete_history.insert(0, text);
        if self.delete_history.len() > 9 {
            self.delete_history.truncate(9);
        }
    }

    /// Gets the unnamed register content (for paste)
    pub fn get_default(&self) -> &str {
        &self.unnamed
    }

    /// Updates the current file name (% register)
    pub fn set_current_file(&mut self, path: String) {
        self.current_file = path;
    }

    /// Updates the alternate file name (# register)
    pub fn set_alternate_file(&mut self, path: String) {
        self.alternate_file = path;
    }

    /// Updates the last inserted text (. register)
    pub fn set_last_inserted(&mut self, text: String) {
        self.last_inserted = text;
    }

    /// Updates the last command (: register)
    pub fn set_last_command(&mut self, command: String) {
        self.last_command = command;
    }

    /// Updates the last search pattern (/ register)
    pub fn set_last_search(&mut self, pattern: String) {
        self.last_search = pattern;
    }

    /// Updates the clipboard registers (+ and *)
    pub fn set_clipboard(&mut self, text: String) {
        self.clipboard = text;
        // TODO: Actually integrate with system clipboard using arboard or clipboard crate
    }

    /// Gets the current file name
    pub fn get_current_file(&self) -> &str {
        &self.current_file
    }

    /// Gets the alternate file name
    pub fn get_alternate_file(&self) -> &str {
        &self.alternate_file
    }

    /// Gets the last inserted text
    pub fn get_last_inserted(&self) -> &str {
        &self.last_inserted
    }

    /// Gets the last command
    pub fn get_last_command(&self) -> &str {
        &self.last_command
    }

    /// Gets the last search pattern
    pub fn get_last_search(&self) -> &str {
        &self.last_search
    }

    /// Gets the clipboard content
    pub fn get_clipboard(&self) -> &str {
        &self.clipboard
    }
}

impl Default for RegisterManager {
    fn default() -> Self {
        Self::new()
    }
}
