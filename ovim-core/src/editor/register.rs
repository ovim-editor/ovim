use std::collections::HashMap;

use super::clipboard::{Clipboard, ClipboardExecutionScope, ExternalClipboardScope};

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

/// In-memory register contents, independent of external clipboard access.
#[derive(Debug, Clone)]
struct RegisterStorage {
    /// Named registers (a-z)
    registers: HashMap<char, RegisterContent>,
    /// The unnamed register (default)
    unnamed: RegisterContent,
    /// The yank register (0)
    yank: RegisterContent,
    /// Delete registers (1-9) - circular buffer of recent deletes
    delete_history: Vec<RegisterContent>,
    /// Small delete register (-) for characterwise deletes without a line break
    small_delete: RegisterContent,
    /// Special registers
    current_file: String, // % - current file name
    alternate_file: String, // # - alternate file name
    last_inserted: String,  // . - last inserted text
    last_command: String,   // : - last command
    last_search: String,    // / - last search pattern
}

/// Register facade coordinating owned register contents and clipboard access.
#[derive(Debug, Clone)]
pub struct RegisterManager {
    storage: RegisterStorage,
    /// The + and * registers share the platform's existing clipboard target.
    clipboard: Clipboard,
}

impl RegisterManager {
    /// Register names accepted by the input prefix in normal and visual modes.
    pub fn is_valid_name(register: char) -> bool {
        register.is_ascii_alphanumeric()
            || matches!(
                register,
                '"' | '_' | '-' | '+' | '*' | '%' | '.' | ':' | '#' | '/'
            )
    }

    /// Creates a new register manager
    pub fn new() -> Self {
        Self {
            storage: RegisterStorage {
                registers: HashMap::new(),
                unnamed: RegisterContent::new(String::new(), RegisterType::Character),
                yank: RegisterContent::new(String::new(), RegisterType::Character),
                delete_history: Vec::new(),
                small_delete: RegisterContent::new(String::new(), RegisterType::Character),
                current_file: String::new(),
                alternate_file: String::new(),
                last_inserted: String::new(),
                last_command: String::new(),
                last_search: String::new(),
            },
            clipboard: Clipboard::new(),
        }
    }

    /// Enter a command execution scope. Its owned guard publishes completed
    /// effects on drop, even if this register manager is replaced meanwhile.
    pub(crate) fn execution_scope(&mut self) -> ClipboardExecutionScope {
        self.clipboard.execution_scope()
    }

    /// Publish pending writes before external code runs, then invalidate its
    /// snapshot when the returned guard drops.
    pub(crate) fn external_clipboard_scope(&mut self) -> ExternalClipboardScope {
        self.clipboard.external_scope()
    }

    /// Replaces the OS clipboard behind `"+`/`"*` (and `clipboard=unnamed[plus]`)
    /// with a process-local one, so tests can run with the default clipboard option.
    pub fn use_memory_clipboard(&mut self) {
        self.clipboard = Clipboard::in_memory();
    }

    #[cfg(test)]
    pub(crate) fn with_clipboard_backend(
        backend: std::sync::Arc<dyn super::clipboard::ClipboardBackend>,
    ) -> Self {
        let mut manager = Self::new();
        manager.clipboard = Clipboard::with_backend(backend);
        manager
    }

    /// Sets a register value (defaults to Character type for backward compatibility)
    pub fn set(&mut self, register: Option<char>, value: String) {
        self.set_with_type(register, value, RegisterType::Character);
    }

    /// Returns true if the register is read-only (%, ., :, /, #).
    /// Writes to these registers should be silently ignored.
    pub fn is_read_only(register: char) -> bool {
        matches!(register, '%' | '.' | ':' | '/' | '#')
    }

    /// Sets a register value with explicit type.
    ///
    /// This is a raw-API safety net: it silently blocks writes to read-only
    /// registers (%, ., :, /, #) so that no code path — even one that bypasses
    /// the higher-level `yank_to_register` / `delete_to_register` helpers — can
    /// corrupt read-only state.  The higher-level helpers additionally implement
    /// Vim's "fall back to unnamed register" semantics for read-only targets;
    /// callers wanting that behavior should go through those helpers instead.
    pub fn set_with_type(&mut self, register: Option<char>, value: String, reg_type: RegisterType) {
        // Silently ignore writes to read-only registers
        if let Some(reg) = register {
            if Self::is_read_only(reg) {
                return;
            }
        }
        let content = RegisterContent::new(value.clone(), reg_type);
        match register {
            None => {
                // Unnamed register - also set as register "
                self.storage.unnamed = content;
            }
            Some('"') => {
                self.storage.unnamed = content;
            }
            Some('0') => {
                self.storage.yank = content;
            }
            Some('+') | Some('*') => {
                // System clipboard - sync with system
                self.clipboard.write(value);
            }
            Some('-') => {
                self.storage.small_delete = content;
            }
            Some('_') => {
                // Black hole register - do nothing
            }
            Some(c) if c.is_ascii_lowercase() => {
                self.storage.registers.insert(c, content);
            }
            Some(c) if c.is_ascii_uppercase() => {
                // Uppercase appends to lowercase register
                let lowercase = c.to_ascii_lowercase();
                self.storage
                    .registers
                    .entry(lowercase)
                    .and_modify(|v| {
                        v.text.push_str(&value);
                        v.reg_type = reg_type;
                    })
                    .or_insert(content);
            }
            _ => {}
        }
    }

    /// Gets a register value (text only, for backward compatibility)
    pub fn get(&self, register: Option<char>) -> String {
        match register {
            None | Some('"') => self.storage.unnamed.text.clone(),
            Some('0') => self.storage.yank.text.clone(),
            Some('%') => self.storage.current_file.clone(),
            Some('#') => self.storage.alternate_file.clone(),
            Some('.') => self.storage.last_inserted.clone(),
            Some(':') => self.storage.last_command.clone(),
            Some('/') => self.storage.last_search.clone(),
            Some('+') | Some('*') => self.clipboard.read(),
            Some('_') => String::new(), // Black hole register always returns empty
            Some('-') => self.storage.small_delete.text.clone(),
            Some(c) if c.is_ascii_digit() => {
                let idx = c.to_digit(10).unwrap() as usize;
                if idx > 0 && idx <= self.storage.delete_history.len() {
                    self.storage.delete_history[idx - 1].text.clone()
                } else {
                    String::new()
                }
            }
            Some(c) if c.is_ascii_lowercase() => self
                .storage
                .registers
                .get(&c)
                .map(|c| c.text.clone())
                .unwrap_or_default(),
            Some(c) if c.is_ascii_uppercase() => {
                // Uppercase reads from lowercase register
                let lowercase = c.to_ascii_lowercase();
                self.storage
                    .registers
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
            None | Some('"') => (
                self.storage.unnamed.text.clone(),
                self.storage.unnamed.reg_type,
            ),
            Some('0') => (self.storage.yank.text.clone(), self.storage.yank.reg_type),
            Some('%') => (self.storage.current_file.clone(), RegisterType::Character),
            Some('#') => (self.storage.alternate_file.clone(), RegisterType::Character),
            Some('.') => (self.storage.last_inserted.clone(), RegisterType::Character),
            Some(':') => (self.storage.last_command.clone(), RegisterType::Character),
            Some('/') => (self.storage.last_search.clone(), RegisterType::Character),
            Some('+') | Some('*') => (self.clipboard.read(), RegisterType::Character),
            Some('_') => (String::new(), RegisterType::Character),
            Some('-') => (
                self.storage.small_delete.text.clone(),
                self.storage.small_delete.reg_type,
            ),
            Some(c) if c.is_ascii_digit() => {
                let idx = c.to_digit(10).unwrap() as usize;
                if idx > 0 && idx <= self.storage.delete_history.len() {
                    let entry = &self.storage.delete_history[idx - 1];
                    (entry.text.clone(), entry.reg_type)
                } else {
                    (String::new(), RegisterType::Character)
                }
            }
            Some(c) if c.is_ascii_lowercase() => self
                .storage
                .registers
                .get(&c)
                .map(|c| (c.text.clone(), c.reg_type))
                .unwrap_or_else(|| (String::new(), RegisterType::Character)),
            Some(c) if c.is_ascii_uppercase() => {
                let lowercase = c.to_ascii_lowercase();
                self.storage
                    .registers
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
        self.storage.unnamed = content.clone();
        self.storage.yank = content;
    }

    /// Stores deleted text in the unnamed and appropriate implicit delete
    /// register (defaults to Character type).
    pub fn delete(&mut self, text: String) {
        self.delete_with_type(text, RegisterType::Character);
    }

    /// Stores deleted text in the unnamed register, plus either the small
    /// delete register or numbered delete history according to its shape.
    pub fn delete_with_type(&mut self, text: String, reg_type: RegisterType) {
        let content = RegisterContent::new(text, reg_type);
        self.storage.unnamed = content.clone();

        if reg_type == RegisterType::Character && !content.text.contains('\n') {
            self.storage.small_delete = content;
        } else {
            // Linewise and multiline deletes rotate through delete history (1-9).
            self.storage.delete_history.insert(0, content);
            if self.storage.delete_history.len() > 9 {
                self.storage.delete_history.truncate(9);
            }
        }
    }

    /// Gets the unnamed register content (for paste)
    pub fn get_default(&self) -> &str {
        &self.storage.unnamed.text
    }

    /// Gets the unnamed register content with type
    pub fn get_default_with_type(&self) -> (&str, RegisterType) {
        (&self.storage.unnamed.text, self.storage.unnamed.reg_type)
    }

    /// Updates the current file name (% register)
    pub fn set_current_file(&mut self, path: String) {
        self.storage.current_file = path;
    }

    /// Updates the alternate file name (# register)
    pub fn set_alternate_file(&mut self, path: String) {
        self.storage.alternate_file = path;
    }

    /// Updates the last inserted text (. register)
    pub fn set_last_inserted(&mut self, text: String) {
        self.storage.last_inserted = text;
    }

    /// Updates the last command (: register)
    pub fn set_last_command(&mut self, command: String) {
        self.storage.last_command = command;
    }

    /// Updates the last search pattern (/ register)
    pub fn set_last_search(&mut self, pattern: String) {
        self.storage.last_search = pattern;
    }

    /// Updates the clipboard registers (+ and *)
    pub fn set_clipboard(&mut self, text: String) {
        self.clipboard.write(text);
    }

    /// Gets the current file name
    pub fn get_current_file(&self) -> &str {
        &self.storage.current_file
    }

    /// Gets the alternate file name
    pub fn get_alternate_file(&self) -> &str {
        &self.storage.alternate_file
    }

    /// Gets the last inserted text
    pub fn get_last_inserted(&self) -> &str {
        &self.storage.last_inserted
    }

    /// Gets the last command
    pub fn get_last_command(&self) -> &str {
        &self.storage.last_command
    }

    /// Gets the last search pattern
    pub fn get_last_search(&self) -> &str {
        &self.storage.last_search
    }

    /// Gets the clipboard content (reads from system clipboard with fallback to cache)
    pub fn get_clipboard(&self) -> String {
        self.clipboard.read()
    }

    /// Lists all non-empty registers as (name, content) pairs
    /// Truncates content for display
    pub fn list_registers(&self) -> Vec<(String, String)> {
        let mut result = Vec::new();

        // Helper to truncate content for display
        fn truncate(s: &str, max_chars: usize) -> String {
            let s = s.replace('\n', "^J");
            let char_count = s.chars().count();
            if char_count > max_chars {
                let truncated: String = s.chars().take(max_chars).collect();
                format!("{}...", truncated)
            } else {
                s
            }
        }

        // Unnamed register
        if !self.storage.unnamed.text.is_empty() {
            result.push(("\"\"".to_string(), truncate(&self.storage.unnamed.text, 50)));
        }

        // Yank register (0)
        if !self.storage.yank.text.is_empty() {
            result.push(("\"0".to_string(), truncate(&self.storage.yank.text, 50)));
        }

        // Delete registers (1-9)
        for (i, entry) in self.storage.delete_history.iter().enumerate() {
            if !entry.text.is_empty() {
                result.push((format!("\"{}", i + 1), truncate(&entry.text, 50)));
            }
        }

        if !self.storage.small_delete.text.is_empty() {
            result.push((
                "\"-".to_string(),
                truncate(&self.storage.small_delete.text, 50),
            ));
        }

        // Named registers (a-z)
        let mut names: Vec<_> = self.storage.registers.keys().copied().collect();
        names.sort();
        for name in names {
            if let Some(content) = self.storage.registers.get(&name) {
                if !content.text.is_empty() {
                    result.push((format!("\"{}", name), truncate(&content.text, 50)));
                }
            }
        }

        // Special registers
        if !self.storage.current_file.is_empty() {
            result.push(("\"%".to_string(), truncate(&self.storage.current_file, 50)));
        }
        if !self.storage.last_search.is_empty() {
            result.push(("\"/".to_string(), truncate(&self.storage.last_search, 50)));
        }
        if !self.storage.last_command.is_empty() {
            result.push(("\":".to_string(), truncate(&self.storage.last_command, 50)));
        }

        // Clipboard
        let clipboard = self.clipboard.read();
        if !clipboard.is_empty() {
            result.push(("\"+".to_string(), truncate(&clipboard, 50)));
        }

        result
    }
}

impl Drop for RegisterManager {
    fn drop(&mut self) {
        // Replacing the register manager relinquishes this editing state's
        // ownership. Publish its prior effects now, so a surviving scope guard
        // cannot overwrite clipboard writes made by the replacement later.
        self.clipboard.flush();
    }
}

impl Default for RegisterManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::clipboard::test_support::FakeClipboard;

    #[test]
    fn register_storage_operations_do_not_access_external_clipboard() {
        let backend = FakeClipboard::new("external");
        let mut registers = RegisterManager::with_clipboard_backend(backend.clone());
        registers.yank_with_type("line\n".into(), RegisterType::Line);
        assert_eq!(registers.get_with_type(Some('0')).1, RegisterType::Line);
        registers.delete_with_type("old\n".into(), RegisterType::Line);
        registers.delete_with_type("new\n".into(), RegisterType::Line);
        registers.delete("small".into());
        assert_eq!(registers.get(Some('1')), "new\n");
        assert_eq!(registers.get(Some('2')), "old\n");
        assert_eq!(registers.get(Some('-')), "small");
        assert_eq!(registers.get(Some('0')), "line\n");
        registers.set(Some('a'), "first".into());
        registers.set(Some('A'), "second".into());
        assert_eq!(registers.get(Some('a')), "firstsecond");
        registers.set(Some('_'), "discarded".into());
        assert_eq!(registers.get(Some('_')), "");
        assert_eq!(registers.get_default(), "small");
        registers.set_current_file("file.rs".into());
        registers.set(Some('%'), "ignored".into());
        assert_eq!(registers.get(Some('%')), "file.rs");
        let state = backend.state.lock().unwrap();
        assert_eq!(state.reads, 0);
        assert!(state.writes.is_empty());
    }

    #[test]
    fn clipboard_register_aliases_and_inspection_observe_pending_writes() {
        let backend = FakeClipboard::new("external");
        let mut registers = RegisterManager::with_clipboard_backend(backend.clone());
        let scope = registers.execution_scope();
        registers.set_with_type(Some('+'), "local\n".into(), RegisterType::Line);
        assert_eq!(registers.get(Some('*')), "local\n");
        // Preserve existing explicit clipboard register type semantics.
        assert_eq!(
            registers.get_with_type(Some('+')).1,
            RegisterType::Character
        );
        assert!(registers
            .list_registers()
            .contains(&("\"+".into(), "local^J".into())));
        assert!(backend.state.lock().unwrap().writes.is_empty());
        drop(scope);
        let state = backend.state.lock().unwrap();
        assert_eq!(state.reads, 0);
        assert_eq!(state.writes, ["local\n"]);
    }
}
