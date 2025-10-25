use std::collections::HashMap;

/// System clipboard provider using arboard
/// This is a helper to gracefully handle clipboard operations
/// which might fail in certain environments (e.g., SSH, headless)
#[derive(Debug, Clone)]
struct ClipboardProvider {
    /// Last synced clipboard content (fallback if clipboard is unavailable)
    cached: String,
}

impl ClipboardProvider {
    fn new() -> Self {
        Self {
            cached: String::new(),
        }
    }

    /// Write text to system clipboard with fallback to cache
    fn write(&mut self, text: String) {
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                if let Err(_e) = clipboard.set_text(text.clone()) {
                    // Clipboard write failed, but still cache the value
                    self.cached = text;
                } else {
                    self.cached = text;
                }
            }
            Err(_e) => {
                // Clipboard unavailable (SSH, headless, etc.), cache the value
                self.cached = text;
            }
        }
    }

    /// Read text from system clipboard with fallback to cache
    fn read(&self) -> String {
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => match clipboard.get_text() {
                Ok(text) => text,
                Err(_e) => {
                    // Clipboard read failed, return cached value
                    self.cached.clone()
                }
            },
            Err(_e) => {
                // Clipboard unavailable, return cached value
                self.cached.clone()
            }
        }
    }
}

/// Type of content stored in a register
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterType {
    /// Character-wise (normal yank/delete)
    Character,
    /// Line-wise (yy, dd, etc.)
    Line,
    /// Block-wise (visual block yank)
    Block,
}

/// Content stored in a register (text + type)
#[derive(Debug, Clone)]
struct RegisterContent {
    text: String,
    reg_type: RegisterType,
}

impl RegisterContent {
    fn new(text: String, reg_type: RegisterType) -> Self {
        Self { text, reg_type }
    }
}

/// Manages registers for storing text (yank, delete, etc.)
#[derive(Debug, Clone)]
pub struct RegisterManager {
    /// Named registers (a-z)
    registers: HashMap<char, RegisterContent>,
    /// The unnamed register (default)
    unnamed: RegisterContent,
    /// The yank register (0)
    yank: RegisterContent,
    /// Delete registers (1-9) - circular buffer of recent deletes
    delete_history: Vec<String>,
    /// Special registers
    current_file: String, // % - current file name
    alternate_file: String, // # - alternate file name
    last_inserted: String,  // . - last inserted text
    last_command: String,   // : - last command
    last_search: String,    // / - last search pattern
    /// System clipboard provider (+ and * registers)
    clipboard: ClipboardProvider,
}

impl RegisterManager {
    /// Creates a new register manager
    pub fn new() -> Self {
        Self {
            registers: HashMap::new(),
            unnamed: RegisterContent::new(String::new(), RegisterType::Character),
            yank: RegisterContent::new(String::new(), RegisterType::Character),
            delete_history: Vec::new(),
            current_file: String::new(),
            alternate_file: String::new(),
            last_inserted: String::new(),
            last_command: String::new(),
            last_search: String::new(),
            clipboard: ClipboardProvider::new(),
        }
    }

    /// Sets a register value (defaults to Character type for backward compatibility)
    pub fn set(&mut self, register: Option<char>, value: String) {
        self.set_with_type(register, value, RegisterType::Character);
    }

    /// Sets a register value with explicit type
    pub fn set_with_type(
        &mut self,
        register: Option<char>,
        value: String,
        reg_type: RegisterType,
    ) {
        let content = RegisterContent::new(value.clone(), reg_type);
        match register {
            None => {
                // Unnamed register - also set as register "
                self.unnamed = content;
            }
            Some('"') => {
                self.unnamed = content;
            }
            Some('0') => {
                self.yank = content;
            }
            Some('+') | Some('*') => {
                // System clipboard - sync with system
                self.clipboard.write(value);
            }
            Some('_') => {
                // Black hole register - do nothing
            }
            Some(c) if c.is_ascii_lowercase() => {
                self.registers.insert(c, content);
            }
            Some(c) if c.is_ascii_uppercase() => {
                // Uppercase appends to lowercase register
                let lowercase = c.to_ascii_lowercase();
                self.registers
                    .entry(lowercase)
                    .and_modify(|v| v.text.push_str(&value))
                    .or_insert(content);
            }
            _ => {}
        }
    }

    /// Gets a register value (text only, for backward compatibility)
    pub fn get(&self, register: Option<char>) -> String {
        match register {
            None | Some('"') => self.unnamed.text.clone(),
            Some('0') => self.yank.text.clone(),
            Some('%') => self.current_file.clone(),
            Some('#') => self.alternate_file.clone(),
            Some('.') => self.last_inserted.clone(),
            Some(':') => self.last_command.clone(),
            Some('/') => self.last_search.clone(),
            Some('+') | Some('*') => self.clipboard.read(),
            Some('_') => String::new(), // Black hole register always returns empty
            Some(c) if c.is_ascii_digit() => {
                let idx = c.to_digit(10).unwrap() as usize;
                if idx > 0 && idx <= self.delete_history.len() {
                    self.delete_history[idx - 1].clone()
                } else {
                    String::new()
                }
            }
            Some(c) if c.is_ascii_lowercase() => self
                .registers
                .get(&c)
                .map(|c| c.text.clone())
                .unwrap_or_default(),
            Some(c) if c.is_ascii_uppercase() => {
                // Uppercase reads from lowercase register
                let lowercase = c.to_ascii_lowercase();
                self.registers
                    .get(&lowercase)
                    .map(|c| c.text.clone())
                    .unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    /// Gets a register value with its type
    /// Note: Returns owned String for clipboard to support dynamic reads
    pub fn get_with_type(&self, register: Option<char>) -> (String, RegisterType) {
        match register {
            None | Some('"') => (self.unnamed.text.clone(), self.unnamed.reg_type),
            Some('0') => (self.yank.text.clone(), self.yank.reg_type),
            Some('%') => (self.current_file.clone(), RegisterType::Character),
            Some('#') => (self.alternate_file.clone(), RegisterType::Character),
            Some('.') => (self.last_inserted.clone(), RegisterType::Character),
            Some(':') => (self.last_command.clone(), RegisterType::Character),
            Some('/') => (self.last_search.clone(), RegisterType::Character),
            Some('+') | Some('*') => (self.clipboard.read(), RegisterType::Character),
            Some('_') => (String::new(), RegisterType::Character),
            Some(c) if c.is_ascii_digit() => (self.get(Some(c)), RegisterType::Character),
            Some(c) if c.is_ascii_lowercase() => self
                .registers
                .get(&c)
                .map(|c| (c.text.clone(), c.reg_type))
                .unwrap_or_else(|| (String::new(), RegisterType::Character)),
            Some(c) if c.is_ascii_uppercase() => {
                let lowercase = c.to_ascii_lowercase();
                self.registers
                    .get(&lowercase)
                    .map(|c| (c.text.clone(), c.reg_type))
                    .unwrap_or_else(|| (String::new(), RegisterType::Character))
            }
            _ => (String::new(), RegisterType::Character),
        }
    }

    /// Stores text in the unnamed register and yank register (defaults to Character type)
    pub fn yank(&mut self, text: String) {
        self.yank_with_type(text, RegisterType::Character);
    }

    /// Stores text in the unnamed register and yank register with explicit type
    pub fn yank_with_type(&mut self, text: String, reg_type: RegisterType) {
        let content = RegisterContent::new(text, reg_type);
        self.unnamed = content.clone();
        self.yank = content;
    }

    /// Stores deleted text in unnamed register and delete history (defaults to Character type)
    pub fn delete(&mut self, text: String) {
        self.delete_with_type(text, RegisterType::Character);
    }

    /// Stores deleted text with explicit type
    pub fn delete_with_type(&mut self, text: String, reg_type: RegisterType) {
        self.unnamed = RegisterContent::new(text.clone(), reg_type);

        // Add to delete history (1-9)
        self.delete_history.insert(0, text);
        if self.delete_history.len() > 9 {
            self.delete_history.truncate(9);
        }
    }

    /// Gets the unnamed register content (for paste)
    pub fn get_default(&self) -> &str {
        &self.unnamed.text
    }

    /// Gets the unnamed register content with type
    pub fn get_default_with_type(&self) -> (&str, RegisterType) {
        (&self.unnamed.text, self.unnamed.reg_type)
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
        self.clipboard.write(text);
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

    /// Gets the clipboard content (reads from system clipboard with fallback to cache)
    pub fn get_clipboard(&self) -> String {
        self.clipboard.read()
    }
}

impl Default for RegisterManager {
    fn default() -> Self {
        Self::new()
    }
}
