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
}

impl RegisterManager {
    /// Creates a new register manager
    pub fn new() -> Self {
        Self {
            registers: HashMap::new(),
            unnamed: String::new(),
            yank: String::new(),
            delete_history: Vec::new(),
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
}

impl Default for RegisterManager {
    fn default() -> Self {
        Self::new()
    }
}
