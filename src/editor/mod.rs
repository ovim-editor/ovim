mod change;
mod completion;
mod filetree;
mod fold;
mod input;
mod lsp_state;
mod macros;
mod marks;
mod motions;
mod operators;
mod picker;
mod quickfix;
mod register;
mod search;
mod tabpage;
mod textobjects;
mod undo;
mod window;

pub use change::{Change, ChangeBuilder, ChangeManager, Position, Range};
pub use completion::CompletionMenu;
pub use filetree::{FileTree, TreeNode};
pub use fold::{Fold, FoldManager};
pub use input::InputHandler;
pub use lsp_state::{LspAction, LspResultType, LspState};
pub use macros::MacroManager;
pub use marks::{GlobalMark, JumpList, Mark, MarkManager};
pub use motions::Motions;
pub use operators::{Operator, Operators};
pub use picker::{Picker, PickerMode, PickerResult};
pub use quickfix::{LocationList, QuickfixEntry, QuickfixEntryType, QuickfixList};
pub use register::{RegisterManager, RegisterType};
pub use search::Search;
pub use tabpage::{TabPage, TabPageManager};
pub use textobjects::{TextObjectRange, TextObjects};
pub use undo::UndoManager;
pub use window::{SplitDirection, Window, WindowManager, WindowNode};

/// Editor options and settings
#[derive(Debug, Clone)]
pub struct EditorOptions {
    /// Width of tab character (default: 4)
    pub tab_width: usize,
    /// Number of spaces to use for autoindent (default: 4)
    pub shift_width: usize,
    /// Use spaces instead of tabs (default: true)
    pub expand_tab: bool,
    /// Show line numbers (default: false)
    pub number: bool,
    /// Show relative line numbers (default: false)
    pub relative_number: bool,
    /// Number of lines to scroll for half-page movements (default: None = calculate from viewport)
    pub scroll: Option<usize>,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            tab_width: 4,
            shift_width: 4,
            expand_tab: true,
            number: false,
            relative_number: false,
            scroll: None,
        }
    }
}

use crate::buffer::Buffer;
use crate::lsp::LspManager;
#[cfg(feature = "lua")]
use crate::lua::LuaContext;
use crate::mode::Mode;
use crate::syntax::{ColorScheme, ColorSchemeRegistry};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Commands sent from background tasks to the LSP manager via channel
#[derive(Debug)]
pub enum LspCommand {
    /// Start a language server
    StartServer {
        language: String,
        command: String,
        args: Vec<String>,
        root_path: std::path::PathBuf,
        response_tx: tokio::sync::oneshot::Sender<Result<()>>,
    },
    /// Send didOpen notification
    DidOpen {
        uri: lsp_types::Url,
        language_id: String,
        version: i32,
        text: String,
        response_tx: tokio::sync::oneshot::Sender<Result<()>>,
    },
    /// Start notification listener
    StartNotificationListener { language_id: String },
}

/// The main editor state
pub struct Editor {
    /// List of open buffers
    pub buffers: Vec<Buffer>,
    /// Index of the currently active buffer
    current_buffer_index: usize,
    /// Window manager for split windows
    window_manager: Option<WindowManager>,
    /// Current editing mode
    mode: Mode,
    /// Whether the editor should quit
    should_quit: bool,
    /// Count prefix for commands (e.g., 5j means move down 5 lines)
    count: Option<usize>,
    /// Pending operator (e.g., d for delete, waiting for motion)
    pending_operator: Option<Operator>,
    /// Pending command character (e.g., 'g' waiting for second character)
    pending_command: Option<char>,
    /// Pending register selection (e.g., 'a' from "a for next operation)
    pending_register: Option<char>,
    /// Register manager for yank/delete operations
    registers: RegisterManager,
    /// Visual mode selection start (line, col)
    visual_start: Option<(usize, usize)>,
    /// Visual block insert/append state: (start_line, end_line, col, is_append, move_to_end)
    /// move_to_end: true for I/A (cursor at end_line), false for c (cursor at start_line)
    visual_block_insert_state: Option<(usize, usize, usize, bool, bool)>,
    /// Command line buffer (for : commands)
    command_line: String,
    /// Command history for command line mode
    command_history: Vec<String>,
    /// Current position in command history (for up/down navigation)
    command_history_index: Option<usize>,
    /// Search buffer (for / and ? commands)
    search_buffer: String,
    /// Search direction: true for forward (/), false for backward (?)
    search_forward: bool,
    /// Current search state
    current_search: Option<Search>,
    /// Mark manager for buffer marks
    marks: MarkManager,
    /// Jump list for Ctrl-O and Ctrl-I
    jump_list: JumpList,
    /// Macro manager for recording and playback
    macro_manager: MacroManager,
    /// Last find motion (for ; and , repeat)
    /// (char, FindType::Find/Till, FindDirection::Forward/Backward)
    last_find: Option<(char, FindType, FindDirection)>,
    /// Picker for fuzzy finding files/grep
    picker: Option<Picker>,
    /// Leader key (default: space)
    leader_key: char,
    /// Waiting for leader sequence (e.g., after pressing space)
    pending_leader: bool,
    /// LSP-related state
    lsp_state: LspState,
    /// Channel sender for LSP commands from background tasks
    lsp_command_tx: Option<mpsc::UnboundedSender<LspCommand>>,
    /// Channel receiver for LSP commands from background tasks
    lsp_command_rx: Option<mpsc::UnboundedReceiver<LspCommand>>,
    /// Lua context for configuration and plugins (optional)
    #[cfg(feature = "lua")]
    lua_context: Option<LuaContext>,
    /// Bridge for Lua-Editor communication (optional)
    #[cfg(feature = "lua")]
    editor_bridge: Option<crate::lua::EditorBridge>,
    /// Last insert position (line, col) for gi command
    last_insert_position: Option<(usize, usize)>,
    /// Completion menu popup
    completion_menu: CompletionMenu,
    /// Preview cache for picker (file_path -> (content, syntax highlights))
    preview_cache: HashMap<String, PreviewCache>,
    /// Color scheme registry
    color_scheme_registry: ColorSchemeRegistry,
    /// Editor options and settings
    pub options: EditorOptions,
    /// Viewport height (rows) - updated from UI layer
    viewport_height: usize,
    /// Current color scheme name
    current_color_scheme: String,
    /// File tree explorer
    file_tree: FileTree,
    /// Quickfix list (global error/location list)
    quickfix_list: QuickfixList,
    /// Location list (per-window error/location list)
    location_list: LocationList,
    /// Whether quickfix window is open
    quickfix_window_open: bool,
    /// Whether location list window is open
    location_window_open: bool,
    /// Tab page manager
    tab_page_manager: TabPageManager,
    /// Last time picker query changed (for debouncing preview loading)
    last_picker_query_change: Option<std::time::Instant>,
    /// Last time picker selection moved (for debouncing preview loading)
    last_picker_selection_change: Option<std::time::Instant>,
    /// Currently loading preview path (to avoid duplicate requests)
    loading_preview: Option<String>,
    /// Last successfully shown preview path (to show while new one loads)
    pub last_shown_preview: Option<String>,
    /// Performance metrics: render count
    render_count: u64,
    /// Performance metrics: last render duration in microseconds
    last_render_duration_micros: Option<u64>,
    /// Performance metrics: last syntax highlighting duration in microseconds
    last_syntax_duration_micros: Option<u64>,
    /// Render dirty flag - set when UI needs redraw
    render_dirty: bool,
    /// Input latency samples in microseconds (circular buffer, max 1000 samples)
    input_latency_samples: Vec<u64>,
    /// Last LSP serialize (rope->string) duration in microseconds
    last_lsp_serialize_micros: Option<u64>,
    /// Last git status refresh duration in microseconds
    last_git_status_micros: Option<u64>,
    /// Last fold calculation duration in microseconds
    last_fold_calc_micros: Option<u64>,
    /// Last diagnostic query duration in microseconds
    last_diagnostic_query_micros: Option<u64>,
}

/// Maximum number of input latency samples to keep for percentile calculation
const MAX_LATENCY_SAMPLES: usize = 1000;

/// Cached preview data for the picker
#[derive(Clone)]
pub struct PreviewCache {
    /// File content
    pub content: String,
    /// Cached syntax-highlighted lines (line_idx -> highlights)
    /// Uses RefCell for interior mutability so we can cache highlights even with immutable reference
    pub highlighted_lines: std::cell::RefCell<
        HashMap<usize, Vec<(std::ops::Range<usize>, crate::syntax::HighlightGroup)>>,
    >,
    /// Detected language (if any)
    pub language: Option<crate::syntax::Language>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FindType {
    Find, // f/F - cursor lands on character
    Till, // t/T - cursor lands before/after character
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FindDirection {
    Forward,
    Backward,
}

impl Editor {
    /// Creates a new editor with an empty buffer
    pub fn new() -> Self {
        let buffer = Buffer::new();

        Self {
            buffers: vec![buffer],
            current_buffer_index: 0,
            window_manager: None, // Will be initialized when viewport size is known
            mode: Mode::default(),
            should_quit: false,
            count: None,
            pending_operator: None,
            pending_command: None,
            pending_register: None,
            registers: RegisterManager::new(),
            visual_start: None,
            visual_block_insert_state: None,
            command_line: String::new(),
            command_history: Vec::new(),
            command_history_index: None,
            search_buffer: String::new(),
            search_forward: true,
            current_search: None,
            marks: MarkManager::new(),
            jump_list: JumpList::new(),
            macro_manager: MacroManager::new(),
            last_find: None,
            picker: None,
            leader_key: ' ',
            pending_leader: false,
            lsp_state: LspState::new(),
            lsp_command_tx: None,
            lsp_command_rx: None,
            #[cfg(feature = "lua")]
            lua_context: None,
            #[cfg(feature = "lua")]
            editor_bridge: None,
            last_insert_position: None,
            completion_menu: CompletionMenu::new(),
            preview_cache: HashMap::new(),
            color_scheme_registry: ColorSchemeRegistry::new(),
            current_color_scheme: "tokyonight".to_string(),
            options: EditorOptions::default(),
            viewport_height: 24,
            file_tree: FileTree::new(),
            quickfix_list: QuickfixList::new(),
            location_list: LocationList::new(),
            quickfix_window_open: false,
            location_window_open: false,
            tab_page_manager: TabPageManager::new(),
            last_picker_query_change: None,
            last_picker_selection_change: None,
            loading_preview: None,
            last_shown_preview: None,
            render_count: 0,
            last_render_duration_micros: None,
            last_syntax_duration_micros: None,
            render_dirty: true, // Start dirty to force initial render
            input_latency_samples: Vec::new(),
            last_lsp_serialize_micros: None,
            last_git_status_micros: None,
            last_fold_calc_micros: None,
            last_diagnostic_query_micros: None,
        }
    }

    /// Creates an editor with initial content
    pub fn with_content(content: &str) -> Self {
        let buffer = Buffer::from_str(content);

        Self {
            buffers: vec![buffer],
            current_buffer_index: 0,
            window_manager: None, // Will be initialized when viewport size is known
            mode: Mode::default(),
            should_quit: false,
            count: None,
            pending_operator: None,
            pending_command: None,
            pending_register: None,
            registers: RegisterManager::new(),
            visual_start: None,
            visual_block_insert_state: None,
            command_line: String::new(),
            command_history: Vec::new(),
            command_history_index: None,
            search_buffer: String::new(),
            search_forward: true,
            current_search: None,
            marks: MarkManager::new(),
            jump_list: JumpList::new(),
            macro_manager: MacroManager::new(),
            last_find: None,
            picker: None,
            leader_key: ' ',
            pending_leader: false,
            lsp_state: LspState::new(),
            lsp_command_tx: None,
            lsp_command_rx: None,
            #[cfg(feature = "lua")]
            lua_context: None,
            #[cfg(feature = "lua")]
            editor_bridge: None,
            last_insert_position: None,
            completion_menu: CompletionMenu::new(),
            preview_cache: HashMap::new(),
            color_scheme_registry: ColorSchemeRegistry::new(),
            current_color_scheme: "tokyonight".to_string(),
            options: EditorOptions::default(),
            viewport_height: 24,
            file_tree: FileTree::new(),
            quickfix_list: QuickfixList::new(),
            location_list: LocationList::new(),
            quickfix_window_open: false,
            location_window_open: false,
            tab_page_manager: TabPageManager::new(),
            last_picker_query_change: None,
            last_picker_selection_change: None,
            loading_preview: None,
            last_shown_preview: None,
            render_count: 0,
            last_render_duration_micros: None,
            last_syntax_duration_micros: None,
            render_dirty: true, // Start dirty to force initial render
            input_latency_samples: Vec::new(),
            last_lsp_serialize_micros: None,
            last_git_status_micros: None,
            last_fold_calc_micros: None,
            last_diagnostic_query_micros: None,
        }
    }

    /// Enables LSP support
    pub fn enable_lsp(&mut self) {
        let (tx, rx) = mpsc::unbounded_channel();
        self.lsp_state.lsp_manager = Some(Arc::new(LspManager::new()));
        self.lsp_command_tx = Some(tx);
        self.lsp_command_rx = Some(rx);
    }

    /// Gets a reference to the LSP manager
    pub fn lsp_manager(&self) -> Option<Arc<LspManager>> {
        self.lsp_state.lsp_manager.clone()
    }

    /// Gets a reference to the LSP command sender for background tasks
    pub fn lsp_command_sender(&self) -> Option<mpsc::UnboundedSender<LspCommand>> {
        self.lsp_command_tx.clone()
    }

    /// Enables Lua support
    #[cfg(feature = "lua")]
    pub fn enable_lua(&mut self) -> Result<()> {
        if self.lua_context.is_none() {
            let mut context = LuaContext::new()?;
            // Create EditorBridge for Lua-Editor communication
            let bridge = crate::lua::EditorBridge::new();
            // Sync initial state to bridge
            self.sync_lua_bridge(&bridge);
            // Set up vim API with bridge
            crate::lua::setup_vim_api(context.lua(), bridge.clone())?;
            // Try to load config
            match context.load_config() {
                Ok(true) => {
                    // Config loaded successfully - process any commands that were queued
                    let commands = bridge.drain_commands();
                    for cmd in commands {
                        let _ = InputHandler::execute_command_string(self, &cmd);
                    }
                }
                Ok(false) => {
                    // No config file found - not an error
                }
                Err(e) => {
                    eprintln!("Warning: Error loading config: {}", e);
                }
            }
            self.lua_context = Some(context);
            self.editor_bridge = Some(bridge);
        }
        Ok(())
    }

    /// Reloads Lua configuration
    #[cfg(feature = "lua")]
    pub fn reload_lua_config(&mut self) -> Result<String> {
        if let Some(ref mut context) = self.lua_context {
            context.reload_config()?;
            // Process any commands that were queued
            if let Some(ref bridge) = self.editor_bridge {
                let commands = bridge.drain_commands();
                for cmd in commands {
                    InputHandler::execute_command_string(self, &cmd)?;
                }
            }
            Ok("Configuration reloaded".to_string())
        } else {
            Ok("Lua not enabled".to_string())
        }
    }

    /// Syncs the current editor state to the Lua bridge
    #[cfg(feature = "lua")]
    fn sync_lua_bridge(&self, bridge: &crate::lua::EditorBridge) {
        // Update cursor position
        let cursor = self.buffer().cursor();
        bridge.update_cursor(cursor.line(), cursor.col());
        // Update buffer content
        bridge.update_buffer(self.buffer().rope().to_string());
        // Update mode
        bridge.update_mode(format!("{:?}", self.mode));
    }

    /// Sync editor state to Lua bridge and get pending commands
    #[cfg(feature = "lua")]
    pub fn get_lua_commands(&self) -> Vec<String> {
        if let Some(ref bridge) = self.editor_bridge {
            // Sync state before getting commands
            self.sync_lua_bridge(bridge);
            // Get and return pending commands
            bridge.drain_commands()
        } else {
            Vec::new()
        }
    }

    /// Update Lua bridge after editor state changes
    #[cfg(feature = "lua")]
    pub fn update_lua_state(&self) {
        if let Some(ref bridge) = self.editor_bridge {
            self.sync_lua_bridge(bridge);
        }
    }

    /// Process pending Lua commands and execute them
    #[cfg(feature = "lua")]
    pub fn process_lua_commands(&mut self) -> Result<()> {
        let commands = self.get_lua_commands();
        for cmd in commands {
            // Execute each command using InputHandler
            InputHandler::execute_command_string(self, &cmd)?;
        }
        Ok(())
    }

    /// Gets a reference to the Lua context
    #[cfg(feature = "lua")]
    pub fn lua_context(&self) -> Option<&LuaContext> {
        self.lua_context.as_ref()
    }

    /// Gets a mutable reference to the Lua context
    #[cfg(feature = "lua")]
    pub fn lua_context_mut(&mut self) -> Option<&mut LuaContext> {
        self.lua_context.as_mut()
    }

    /// Executes Lua code
    #[cfg(feature = "lua")]
    pub fn execute_lua(&mut self, code: &str) -> Result<String> {
        if let Some(ref context) = self.lua_context {
            // Sync state to bridge before execution
            self.update_lua_state();
            // Execute Lua code
            let result = context.execute(code)?;
            Ok(crate::lua::lua_value_to_string(&result))
        } else {
            anyhow::bail!("Lua support not enabled")
        }
    }

    /// Executes a Lua file
    #[cfg(feature = "lua")]
    pub fn execute_lua_file(&mut self, path: &str) -> Result<()> {
        if let Some(ref mut context) = self.lua_context {
            context.execute_file(path)?;
            Ok(())
        } else {
            anyhow::bail!("Lua support not enabled")
        }
    }

    /// Gets the command line buffer
    pub fn command_line(&self) -> &str {
        &self.command_line
    }

    /// Clears the command line buffer
    pub fn clear_command_line(&mut self) {
        self.command_line.clear();
    }

    /// Appends a character to the command line
    pub fn append_to_command_line(&mut self, ch: char) {
        self.command_line.push(ch);
    }

    /// Removes the last character from the command line
    pub fn backspace_command_line(&mut self) {
        self.command_line.pop();
    }

    /// Adds current command to history
    pub fn add_command_to_history(&mut self) {
        let cmd = self.command_line.trim().to_string();
        if !cmd.is_empty() {
            // Don't add duplicate if it's the same as the last command
            if self.command_history.last() != Some(&cmd) {
                self.command_history.push(cmd);
                // Limit history size to 100 commands
                if self.command_history.len() > 100 {
                    self.command_history.drain(0..1);
                }
            }
        }
        self.command_history_index = None;
    }

    /// Navigate to previous command in history (up arrow)
    pub fn history_prev(&mut self) {
        if self.command_history.is_empty() {
            return;
        }

        let new_index = match self.command_history_index {
            None => {
                // First time pressing up - go to last command
                Some(self.command_history.len() - 1)
            }
            Some(idx) if idx > 0 => {
                // Go to previous command
                Some(idx - 1)
            }
            Some(_) => {
                // Already at oldest command
                return;
            }
        };

        if let Some(idx) = new_index {
            if let Some(cmd) = self.command_history.get(idx) {
                self.command_line = cmd.clone();
                self.command_history_index = Some(idx);
            }
        }
    }

    /// Navigate to next command in history (down arrow)
    pub fn history_next(&mut self) {
        if self.command_history.is_empty() {
            return;
        }

        let new_index = match self.command_history_index {
            None => {
                // Not navigating history, do nothing
                return;
            }
            Some(idx) if idx < self.command_history.len() - 1 => {
                // Go to next command
                Some(idx + 1)
            }
            Some(_) => {
                // At newest command, clear to empty line
                self.command_line.clear();
                self.command_history_index = None;
                return;
            }
        };

        if let Some(idx) = new_index {
            if let Some(cmd) = self.command_history.get(idx) {
                self.command_line = cmd.clone();
                self.command_history_index = Some(idx);
            }
        }
    }

    /// Gets the search buffer
    pub fn search_buffer(&self) -> &str {
        &self.search_buffer
    }

    /// Clears the search buffer
    pub fn clear_search_buffer(&mut self) {
        self.search_buffer.clear();
    }

    /// Appends a character to the search buffer
    pub fn append_to_search_buffer(&mut self, ch: char) {
        self.search_buffer.push(ch);
    }

    /// Removes the last character from the search buffer
    pub fn backspace_search_buffer(&mut self) {
        self.search_buffer.pop();
    }

    /// Sets the search direction
    pub fn set_search_forward(&mut self, forward: bool) {
        self.search_forward = forward;
    }

    /// Gets the search direction
    pub fn search_forward(&self) -> bool {
        self.search_forward
    }

    /// Gets the current search
    pub fn current_search(&self) -> Option<&Search> {
        self.current_search.as_ref()
    }

    /// Sets the current search
    pub fn set_current_search(&mut self, search: Search) {
        self.current_search = Some(search);
    }

    /// Clears the current search (stops highlighting)
    pub fn clear_search_highlight(&mut self) {
        self.current_search = None;
    }

    /// Executes the current search and moves cursor to first match
    pub fn execute_search(&mut self) {
        if self.search_buffer.is_empty() {
            return;
        }

        // Update the / register with the search pattern
        self.registers.set_last_search(self.search_buffer.clone());

        let mut search = Search::new(self.search_buffer.clone(), self.search_forward);
        let cursor = self.buffer().cursor();

        // Start search from current cursor position (inclusive)
        if let Some((line, col, _)) = search.find_next(self.buffer(), cursor.line(), cursor.col()) {
            self.buffer_mut().cursor_mut().set_position(line, col);
            self.current_search = Some(search);
        }
    }

    /// Finds the next search match (n command)
    pub fn search_next(&mut self) {
        // Get cursor position before borrowing
        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col();

        // Clone search to avoid borrow conflicts
        if let Some(ref search) = self.current_search {
            let is_forward = search.is_forward();
            let mut search_clone = search.clone();

            // For forward search, start from col+1; for backward, start from col-1 or col
            let search_col = if is_forward {
                cursor_col + 1
            } else {
                if cursor_col > 0 {
                    cursor_col - 1
                } else {
                    0
                }
            };

            if let Some((line, col, _)) =
                search_clone.find_next(self.buffer(), cursor_line, search_col)
            {
                self.buffer_mut().cursor_mut().set_position(line, col);
            }
        }
    }

    /// Finds the previous search match (N command)
    pub fn search_prev(&mut self) {
        if let Some(ref search) = self.current_search {
            // Create a reversed search
            let is_forward = search.is_forward();
            let mut rev_search = Search::new(search.pattern().to_string(), !is_forward);
            let cursor = self.buffer().cursor();

            // For reverse direction: if original was forward, now going backward (use col-1)
            // if original was backward, now going forward (use col+1)
            let search_col = if is_forward {
                // Original was forward, now backward
                if cursor.col() > 0 {
                    cursor.col() - 1
                } else {
                    0
                }
            } else {
                // Original was backward, now forward
                cursor.col() + 1
            };

            if let Some((line, col, _)) =
                rev_search.find_next(self.buffer(), cursor.line(), search_col)
            {
                self.buffer_mut().cursor_mut().set_position(line, col);
            }
        }
    }

    /// Sets a mark at the current cursor position
    pub fn set_mark(&mut self, name: char) -> bool {
        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col();
        let file_path = self.buffer().file_path().map(|s| s.to_string());
        self.marks
            .set_mark(name, cursor_line, cursor_col, file_path.as_deref())
    }

    /// Jumps to a mark (exact position with backtick)
    pub fn jump_to_mark(&mut self, name: char) -> bool {
        // Try local mark first (a-z)
        if name.is_ascii_lowercase() {
            if let Some(mark) = self.marks.get_mark(name) {
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(mark.line, mark.col);
                return true;
            }
        }

        // Try global mark (A-Z)
        if name.is_ascii_uppercase() {
            if let Some(global_mark) = self.marks.get_global_mark(name).cloned() {
                // Load the file if it's different from current file
                let current_file = self.buffer().file_path().map(|s| s.to_string());
                if current_file.as_deref() != Some(&global_mark.file_path) {
                    // Load the file (synchronously for now)
                    if let Ok(_) = self.load_file(&global_mark.file_path) {
                        // File loaded successfully
                    } else {
                        return false; // Failed to load file
                    }
                }

                // Validate and clamp mark position to buffer bounds
                let max_line = self.buffer().line_count().saturating_sub(1);
                let clamped_line = global_mark.line.min(max_line);

                let line_len = if let Some(line) = self.buffer().line(clamped_line) {
                    line.trim_end_matches('\n').chars().count()
                } else {
                    0
                };
                let clamped_col = global_mark.col.min(line_len);

                // Jump to the validated position
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(clamped_line, clamped_col);
                return true;
            }
        }

        false
    }

    /// Jumps to mark line (apostrophe - goes to first non-blank on line)
    pub fn jump_to_mark_line(&mut self, name: char) -> bool {
        // Try local mark first (a-z)
        if name.is_ascii_lowercase() {
            if let Some(mark) = self.marks.get_mark(name) {
                // Find first non-blank character on the line
                let first_non_blank = if let Some(line_text) = self.buffer().line(mark.line) {
                    line_text
                        .chars()
                        .position(|c| !c.is_whitespace())
                        .unwrap_or(0)
                } else {
                    0
                };

                self.buffer_mut()
                    .cursor_mut()
                    .set_position(mark.line, first_non_blank);
                return true;
            }
        }

        // Try global mark (A-Z)
        if name.is_ascii_uppercase() {
            if let Some(global_mark) = self.marks.get_global_mark(name).cloned() {
                // Load the file if it's different from current file
                let current_file = self.buffer().file_path().map(|s| s.to_string());
                if current_file.as_deref() != Some(&global_mark.file_path) {
                    // Load the file (synchronously for now)
                    if let Ok(_) = self.load_file(&global_mark.file_path) {
                        // File loaded successfully
                    } else {
                        return false; // Failed to load file
                    }
                }

                // Validate and clamp mark line to buffer bounds
                let max_line = self.buffer().line_count().saturating_sub(1);
                let clamped_line = global_mark.line.min(max_line);

                // Find first non-blank character on the line
                let first_non_blank = if let Some(line_text) = self.buffer().line(clamped_line) {
                    line_text
                        .chars()
                        .position(|c| !c.is_whitespace())
                        .unwrap_or(0)
                } else {
                    0
                };

                self.buffer_mut()
                    .cursor_mut()
                    .set_position(clamped_line, first_non_blank);
                return true;
            }
        }

        false
    }

    /// Adds current position to jump list
    pub fn add_jump(&mut self) {
        let cursor = self.buffer().cursor();
        self.jump_list.add_jump(cursor.line(), cursor.col());
    }

    /// Jumps back in the jump list (Ctrl-O)
    pub fn jump_back(&mut self) -> bool {
        if let Some((line, col)) = self.jump_list.jump_back() {
            self.buffer_mut().cursor_mut().set_position(line, col);
            true
        } else {
            false
        }
    }

    /// Jumps forward in the jump list (Ctrl-I)
    pub fn jump_forward(&mut self) -> bool {
        if let Some((line, col)) = self.jump_list.jump_forward() {
            self.buffer_mut().cursor_mut().set_position(line, col);
            true
        } else {
            false
        }
    }

    /// Starts recording a macro
    pub fn start_macro_recording(&mut self, register: char) -> bool {
        self.macro_manager.start_recording(register)
    }

    /// Stops macro recording
    pub fn stop_macro_recording(&mut self) {
        self.macro_manager.stop_recording();
    }

    /// Records a key event in the current macro
    pub fn record_macro_event(&mut self, event: crossterm::event::KeyEvent) {
        self.macro_manager.record_event(event);
    }

    /// Returns whether currently recording a macro
    pub fn is_recording_macro(&self) -> bool {
        self.macro_manager.is_recording()
    }

    /// Gets the register being recorded
    pub fn recording_register(&self) -> Option<char> {
        self.macro_manager.recording_register()
    }

    /// Gets a macro by register for playback
    pub fn get_macro(&self, register: char) -> Option<&Vec<crossterm::event::KeyEvent>> {
        self.macro_manager.get_macro(register)
    }

    /// Gets a reference to the current buffer
    pub fn buffer(&self) -> &Buffer {
        &self.buffers[self.current_buffer_index]
    }

    /// Gets a mutable reference to the current buffer
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current_buffer_index]
    }

    /// Gets the number of open buffers
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Gets the current buffer index (0-based)
    pub fn current_buffer_index(&self) -> usize {
        self.current_buffer_index
    }

    /// Gets a list of all buffer names (file paths or "[No Name]")
    pub fn buffer_names(&self) -> Vec<String> {
        self.buffers
            .iter()
            .map(|buf| {
                buf.file_path()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "[No Name]".to_string())
            })
            .collect()
    }

    /// Lists all buffers with their index and status
    pub fn list_buffers(&self) -> String {
        let mut result = String::new();
        for (i, buf) in self.buffers.iter().enumerate() {
            let current_marker = if i == self.current_buffer_index {
                "%"
            } else {
                " "
            };
            let modified_marker = if buf.is_modified() { "+" } else { " " };
            let name = buf.file_path().unwrap_or("[No Name]");
            result.push_str(&format!(
                "{}{} {}: {}\n",
                current_marker,
                modified_marker,
                i + 1,
                name
            ));
        }
        result
    }

    /// Switches to a buffer by index (0-based)
    pub fn switch_to_buffer(&mut self, index: usize) {
        if index < self.buffers.len() && index != self.current_buffer_index {
            // Save current file to alternate file register
            if let Some(current_path) = self.buffer().file_path() {
                self.registers.set_alternate_file(current_path.to_string());
            }

            self.current_buffer_index = index;
            self.lsp_state.needs_lsp_init = true;

            // Clear buffer-local marks (a-z) when switching files
            self.marks.clear();

            // Clear LSP UI state (hover, completions, etc.)
            self.clear_lsp_state();

            // Update current file register
            if let Some(new_path) = self.buffer().file_path() {
                self.registers.set_current_file(new_path.to_string());
            }
        }
    }

    /// Switches to the next buffer
    pub fn next_buffer(&mut self) {
        if self.buffers.len() > 1 {
            // Save current file to alternate file register
            if let Some(current_path) = self.buffer().file_path() {
                self.registers.set_alternate_file(current_path.to_string());
            }

            self.current_buffer_index = (self.current_buffer_index + 1) % self.buffers.len();
            self.lsp_state.needs_lsp_init = true;

            // Clear buffer-local marks (a-z) when switching files
            self.marks.clear();

            // Clear LSP UI state (hover, completions, etc.)
            self.clear_lsp_state();

            // Update current file register
            if let Some(new_path) = self.buffer().file_path() {
                self.registers.set_current_file(new_path.to_string());
            }
        }
    }

    /// Switches to the previous buffer
    pub fn prev_buffer(&mut self) {
        if self.buffers.len() > 1 {
            // Save current file to alternate file register
            if let Some(current_path) = self.buffer().file_path() {
                self.registers.set_alternate_file(current_path.to_string());
            }

            self.current_buffer_index = if self.current_buffer_index == 0 {
                self.buffers.len() - 1
            } else {
                self.current_buffer_index - 1
            };
            self.lsp_state.needs_lsp_init = true;

            // Clear buffer-local marks (a-z) when switching files
            self.marks.clear();

            // Clear LSP UI state (hover, completions, etc.)
            self.clear_lsp_state();

            // Update current file register
            if let Some(new_path) = self.buffer().file_path() {
                self.registers.set_current_file(new_path.to_string());
            }
        }
    }

    /// Deletes the current buffer and switches to another if available
    /// Returns true if the editor should quit (no more buffers)
    pub fn delete_current_buffer(&mut self) -> bool {
        if self.buffers.len() == 1 {
            // Last buffer - quit the editor
            return true;
        }

        // Remove current buffer (track sync state)
        if let Some(path) = self.buffer().file_path().map(|s| s.to_string()) {
            self.lsp_state.document_sync.remove(&path);
        }

        // Remove current buffer
        self.buffers.remove(self.current_buffer_index);

        // Adjust index if we were at the end
        if self.current_buffer_index >= self.buffers.len() {
            self.current_buffer_index = self.buffers.len() - 1;
        }

        self.lsp_state.needs_lsp_init = true;
        false
    }

    /// Adds a new buffer and switches to it
    pub fn add_buffer(&mut self, buffer: Buffer) {
        self.buffers.push(buffer);
        self.current_buffer_index = self.buffers.len() - 1;
        self.lsp_state.needs_lsp_init = true;
    }

    /// Finds the index of a buffer with the given file path
    /// Returns None if no buffer has that file path
    fn find_buffer_by_path(&self, file_path: &str) -> Option<usize> {
        // Normalize paths for comparison
        let target_path = std::path::Path::new(file_path).canonicalize().ok()?;

        for (index, buffer) in self.buffers.iter().enumerate() {
            if let Some(buf_path) = buffer.file_path() {
                if let Ok(buf_canonical) = std::path::Path::new(buf_path).canonicalize() {
                    if target_path == buf_canonical {
                        return Some(index);
                    }
                }
            }
        }
        None
    }

    /// Opens a file, switching to existing buffer if already open
    /// or creating a new buffer if not
    pub fn open_file<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        let path_str = path
            .as_ref()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?;

        // Check if file is already open
        if let Some(index) = self.find_buffer_by_path(path_str) {
            // Just switch to existing buffer
            self.current_buffer_index = index;
            return Ok(());
        }

        // File not open, load it
        let buffer = Buffer::load_file(path)?;
        self.add_buffer(buffer);
        Ok(())
    }

    /// Gets the current mode
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Sets the mode
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        // Clear count and pending operator when changing modes
        self.count = None;
        self.pending_operator = None;
        self.pending_command = None;
        self.pending_register = None;

        // Clear visual selection when leaving visual modes
        if !matches!(mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock) {
            self.visual_start = None;
        }
    }

    /// Gets the pending command
    pub fn pending_command(&self) -> Option<char> {
        self.pending_command
    }

    /// Sets the pending command
    pub fn set_pending_command(&mut self, cmd: char) {
        self.pending_command = Some(cmd);
    }

    /// Sets the viewport height (called from UI layer)
    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height;
    }

    /// Gets the viewport height
    pub fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    /// Calculates half-page scroll amount
    /// Uses options.scroll if set, otherwise viewport_height / 2
    pub fn half_page_scroll(&self) -> usize {
        self.options
            .scroll
            .unwrap_or_else(|| (self.viewport_height / 2).max(1))
    }

    /// Clears the pending command
    pub fn clear_pending_command(&mut self) {
        self.pending_command = None;
    }

    /// Returns whether the editor should quit
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Requests the editor to quit
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Sends didClose notification for current file (called on shutdown)
    pub async fn close_current_file_lsp(&mut self) {
        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return;
        };

        let file_path_string = file_path.to_string();
        let Ok(uri) = lsp_types::Url::from_file_path(std::path::Path::new(&file_path_string))
        else {
            return;
        };

        self.lsp_state.document_sync.remove(&file_path_string);

        // Detect language from file path
        let Some(language_id) =
            crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path_string)
        else {
            return;
        };

        let _ = lsp.did_close(uri, language_id).await;
    }

    /// Gets the current count prefix
    pub fn count(&self) -> Option<usize> {
        self.count
    }

    /// Sets the count prefix
    pub fn set_count(&mut self, count: usize) {
        self.count = Some(count);
    }

    /// Appends to the count (for multi-digit counts like 55)
    pub fn append_count(&mut self, digit: usize) {
        self.count = Some(self.count.unwrap_or(0) * 10 + digit);
    }

    /// Clears the count
    pub fn clear_count(&mut self) {
        self.count = None;
    }

    /// Gets the effective count (1 if not set)
    pub fn effective_count(&self) -> usize {
        self.count.unwrap_or(1)
    }

    /// Gets the pending operator
    pub fn pending_operator(&self) -> Option<Operator> {
        self.pending_operator
    }

    /// Sets the pending operator
    pub fn set_pending_operator(&mut self, op: Operator) {
        self.pending_operator = Some(op);
    }

    /// Clears the pending operator
    pub fn clear_pending_operator(&mut self) {
        self.pending_operator = None;
    }

    /// Gets a reference to the register manager
    pub fn registers(&self) -> &RegisterManager {
        &self.registers
    }

    /// Gets a mutable reference to the register manager
    pub fn registers_mut(&mut self) -> &mut RegisterManager {
        &mut self.registers
    }

    /// Gets a reference to the mark manager
    pub fn marks(&self) -> &MarkManager {
        &self.marks
    }

    /// Yanks text to the appropriate register (pending_register or default)
    pub fn yank_to_register(&mut self, text: String) {
        self.yank_to_register_with_type(text, RegisterType::Character);
    }

    /// Yanks text to the appropriate register with explicit type
    pub fn yank_to_register_with_type(&mut self, text: String, reg_type: RegisterType) {
        if let Some(reg) = self.pending_register {
            self.registers.set_with_type(Some(reg), text, reg_type);
            self.pending_register = None;
        } else {
            self.registers.yank_with_type(text, reg_type);
        }
    }

    /// Deletes text and stores in the appropriate register (pending_register or default)
    pub fn delete_to_register(&mut self, text: String) {
        self.delete_to_register_with_type(text, RegisterType::Character);
    }

    /// Deletes text and stores in the appropriate register with explicit type
    pub fn delete_to_register_with_type(&mut self, text: String, reg_type: RegisterType) {
        if let Some(reg) = self.pending_register {
            self.registers.set_with_type(Some(reg), text, reg_type);
            self.pending_register = None;
        } else {
            self.registers.delete_with_type(text, reg_type);
        }
    }

    /// Gets text from the appropriate register (pending_register or default)
    pub fn get_from_register(&mut self) -> String {
        let text = if let Some(reg) = self.pending_register {
            self.registers.get(Some(reg))
        } else {
            self.registers.get_default().to_string()
        };
        self.pending_register = None;
        text
    }

    /// Gets text and type from the appropriate register (pending_register or default)
    pub fn get_from_register_with_type(&mut self) -> (String, RegisterType) {
        let (text, reg_type) = if let Some(reg) = self.pending_register {
            self.registers.get_with_type(Some(reg))
        } else {
            let (t, rt) = self.registers.get_default_with_type();
            (t.to_string(), rt)
        };
        self.pending_register = None;
        (text, reg_type)
    }

    /// Gets the pending register for next operation
    pub fn pending_register(&self) -> Option<char> {
        self.pending_register
    }

    /// Sets the pending register for next operation
    pub fn set_pending_register(&mut self, reg: char) {
        self.pending_register = Some(reg);
    }

    /// Clears the pending register
    pub fn clear_pending_register(&mut self) {
        self.pending_register = None;
    }

    /// Gets the visual selection start position
    pub fn visual_start(&self) -> Option<(usize, usize)> {
        self.visual_start
    }

    /// Sets the visual selection start position
    pub fn set_visual_start(&mut self, line: usize, col: usize) {
        self.visual_start = Some((line, col));
    }

    /// Clears the visual selection
    pub fn clear_visual_start(&mut self) {
        self.visual_start = None;
    }

    /// Sets visual block insert/append state for replay on insert mode exit
    pub fn set_visual_block_insert_state(
        &mut self,
        state: Option<(usize, usize, usize, bool, bool)>,
    ) {
        self.visual_block_insert_state = state;
    }

    /// Gets visual block insert/append state
    pub fn visual_block_insert_state(&self) -> Option<(usize, usize, usize, bool, bool)> {
        self.visual_block_insert_state
    }

    /// Gets the visual selection range (start and end positions)
    /// Returns ((start_line, start_col), (end_line, end_col))
    /// Note: For VisualBlock, this returns the corners of the rectangle
    pub fn visual_selection(&self) -> Option<((usize, usize), (usize, usize))> {
        self.visual_start.map(|start| {
            let cursor = self.buffer().cursor();
            let mut end = (cursor.line(), cursor.col());

            match self.mode {
                Mode::VisualLine => {
                    // Get the length of the end line (excluding newline)
                    if let Some(line_text) = self.buffer().line(end.0) {
                        let line_len = line_text.trim_end_matches('\n').chars().count();
                        end.1 = if line_len > 0 { line_len - 1 } else { 0 };
                    }

                    // Also ensure start is at beginning of its line
                    let mut start = start;
                    start.1 = 0;

                    // Normalize so start is always before end
                    if start.0 <= end.0 {
                        (start, end)
                    } else {
                        // If cursor moved above start line, swap and adjust
                        let mut new_start = end;
                        new_start.1 = 0;
                        let mut new_end = start;
                        if let Some(line_text) = self.buffer().line(new_end.0) {
                            let line_len = line_text.trim_end_matches('\n').chars().count();
                            new_end.1 = if line_len > 0 { line_len - 1 } else { 0 };
                        }
                        (new_start, new_end)
                    }
                }
                Mode::VisualBlock => {
                    // Block mode: return corners of rectangle
                    // Normalize so start_line <= end_line and start_col <= end_col
                    eprintln!(
                        "[DEBUG visual_selection] start=({},{}), end=({},{})",
                        start.0, start.1, end.0, end.1
                    );
                    let (min_line, max_line) = if start.0 <= end.0 {
                        (start.0, end.0)
                    } else {
                        (end.0, start.0)
                    };

                    let (min_col, max_col) = if start.1 <= end.1 {
                        (start.1, end.1)
                    } else {
                        (end.1, start.1)
                    };

                    eprintln!(
                        "[DEBUG visual_selection] result: (({},{}), ({},{}))",
                        min_line, min_col, max_line, max_col
                    );
                    ((min_line, min_col), (max_line, max_col))
                }
                _ => {
                    // Normal visual mode behavior
                    // Normalize so start is always before end
                    if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
                        (start, end)
                    } else {
                        (end, start)
                    }
                }
            }
        })
    }

    /// Loads a file into the editor (async version)
    /// If the file is already open in a buffer, switches to that buffer
    /// Otherwise, adds it as a new buffer
    pub async fn load_file_async<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        let path_str = path.as_ref().to_string_lossy().to_string();

        // Check if file is already open in a buffer
        for (i, buf) in self.buffers.iter().enumerate() {
            if buf.file_path() == Some(&path_str) {
                // File already open - just switch to it
                // Save current file to alternate file register
                if let Some(current_path) = self.buffer().file_path() {
                    self.registers.set_alternate_file(current_path.to_string());
                }
                self.current_buffer_index = i;
                // Update current file register
                self.registers.set_current_file(path_str);
                // Still need to initialize LSP for this file if it hasn't been yet
                self.lsp_state.needs_lsp_init = true;
                return Ok(());
            }
        }

        // Store old file path before loading new file
        let old_file_path = self.buffer().file_path().map(|s| s.to_string());

        // Save current file to alternate file register
        if let Some(current_path) = old_file_path.as_ref() {
            self.registers.set_alternate_file(current_path.to_string());
        }

        // Load new buffer
        let new_buffer = Buffer::load_file_async(path).await?;
        self.add_buffer(new_buffer);

        // Update current file register
        self.registers.set_current_file(path_str);

        // Update tab title to match the loaded file
        self.update_current_tab_title();

        // Mark that we need to send didClose for the old file
        if old_file_path.is_some() {
            self.lsp_state.pending_did_close_file = old_file_path;
        }

        Ok(())
    }

    /// Loads a file into the editor (blocking wrapper)
    pub fn load_file<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.load_file_async(path))
        })
    }

    /// Checks if LSP initialization is needed and returns the file path
    pub fn needs_lsp_init(&self) -> Option<String> {
        if self.lsp_state.needs_lsp_init {
            self.buffer().file_path().map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Clears the LSP init flag (should be called after initializing LSP)
    pub fn clear_lsp_init_flag(&mut self) {
        self.lsp_state.needs_lsp_init = false;
    }

    /// Requests LSP initialization for the current buffer
    pub fn request_lsp_init(&mut self) {
        self.lsp_state.needs_lsp_init = true;
    }

    /// Starts building a composite change (e.g., when entering insert mode)
    pub fn start_change_building(&mut self, cursor_before: Position) {
        self.buffer_mut()
            .change_manager_mut()
            .start_building(cursor_before);
    }

    /// Adds a change to the change manager
    pub fn add_change(&mut self, change: Change) {
        self.buffer_mut().change_manager_mut().add_change(change);
        self.mark_buffer_modified(); // Mark for LSP didChange notification
    }

    /// Finalizes the current composite change
    pub fn finalize_change_building(&mut self) {
        let cursor_pos = (self.buffer().cursor().line(), self.buffer().cursor().col());
        self.buffer_mut()
            .change_manager_mut()
            .finalize_building_at(cursor_pos);
    }

    /// Gets a reference to the last change
    pub fn last_change(&self) -> Option<&Change> {
        self.buffer().change_manager().last_change()
    }

    /// Pops the last change from the undo stack (without undoing it)
    /// Used when replacing a change with a composite version
    pub fn pop_last_change(&mut self) -> Option<Change> {
        self.buffer_mut().change_manager_mut().pop_last_change()
    }

    /// Undoes the last change
    pub fn undo(&mut self) {
        self.buffer_mut().undo();
    }

    /// Redoes the next change
    pub fn redo(&mut self) {
        self.buffer_mut().redo();
    }

    /// Repeats the last change
    pub fn repeat_last_change(&mut self) {
        self.buffer_mut().repeat_last_change();
    }

    /// Updates the . register with the last inserted text
    pub fn update_last_inserted_register(&mut self) {
        if let Some(change) = self.buffer().change_manager().last_change() {
            let inserted_text = change.get_inserted_text();
            if !inserted_text.is_empty() {
                self.registers.set_last_inserted(inserted_text);
            }
        }
    }

    /// Checks if buffer is modified relative to last save
    pub fn is_modified(&self) -> bool {
        !self.buffer().change_manager().is_at_save_point()
    }

    /// Marks current state as saved
    pub fn mark_saved(&mut self) {
        self.buffer_mut().change_manager_mut().mark_saved();
        self.buffer_mut().mark_clean();
    }

    /// Runs the editor (main loop will be implemented later)
    pub fn run(&mut self) -> Result<()> {
        // Placeholder for now
        Ok(())
    }

    /// Sets the last find motion for ; and , repeat
    pub fn set_last_find(&mut self, ch: char, find_type: FindType, direction: FindDirection) {
        self.last_find = Some((ch, find_type, direction));
    }

    /// Gets the last find motion
    pub fn get_last_find(&self) -> Option<(char, FindType, FindDirection)> {
        self.last_find
    }

    /// Sets the picker
    pub fn set_picker(&mut self, picker: Picker) {
        self.picker = Some(picker);
    }

    /// Gets a reference to the picker
    pub fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }

    /// Gets a mutable reference to the picker
    pub fn picker_mut(&mut self) -> Option<&mut Picker> {
        self.picker.as_mut()
    }

    /// Marks that picker query was just changed (for debouncing preview loading)
    pub fn mark_picker_query_changed(&mut self) {
        self.last_picker_query_change = Some(std::time::Instant::now());
        // Clear loading flag since query changed
        self.loading_preview = None;
    }

    /// Marks that the picker selection moved (for debouncing preview loading)
    pub fn mark_picker_selection_changed(&mut self) {
        self.last_picker_selection_change = Some(std::time::Instant::now());
        // Allow new preview to load for the freshly selected entry
        self.loading_preview = None;
    }

    /// Checks if enough time has elapsed since picker query changed (for debouncing)
    /// Returns true if we should load preview now
    pub fn should_load_picker_preview(&self, debounce_ms: u64) -> bool {
        let mut last_change = self.last_picker_query_change;

        if let Some(selection_change) = self.last_picker_selection_change {
            last_change = match last_change {
                Some(existing) => Some(std::cmp::max(existing, selection_change)),
                None => Some(selection_change),
            };
        }

        match last_change {
            None => true, // No recent change, load immediately
            Some(last_change) => last_change.elapsed().as_millis() >= debounce_ms as u128,
        }
    }

    /// Gets the path that should be loaded for preview (if any)
    /// Returns None if already cached or already loading
    pub fn get_preview_to_load(&mut self) -> Option<String> {
        if let Some(picker) = self.picker() {
            if let Some(result) = picker.selected_result() {
                // Only for file picker modes
                if *picker.mode() != crate::editor::PickerMode::Custom
                    && *picker.mode() != crate::editor::PickerMode::Completion
                    && *picker.mode() != crate::editor::PickerMode::LspLocations
                {
                    let file_path = result.location.clone();

                    // Skip if already cached
                    if self.preview_cache.contains_key(&file_path) {
                        return None;
                    }

                    // Skip if currently loading
                    if self.loading_preview.as_ref() == Some(&file_path) {
                        return None;
                    }

                    // Mark as loading
                    self.loading_preview = Some(file_path.clone());
                    return Some(file_path);
                }
            }
        }
        None
    }

    /// Inserts a loaded preview into the cache
    pub fn insert_preview(&mut self, file_path: String, cache: PreviewCache) {
        self.preview_cache.insert(file_path.clone(), cache);
        // Clear loading flag
        if self.loading_preview.as_ref() == Some(&file_path) {
            self.loading_preview = None;
        }
        // Trim cache
        self.trim_preview_cache(50);
    }

    /// Closes the picker
    pub fn close_picker(&mut self) {
        self.picker = None;
        // Clear preview cache when closing picker to free memory
        self.preview_cache.clear();
        self.last_picker_selection_change = None;
    }

    /// Gets preview from cache or loads it (async version)
    pub async fn get_or_load_preview_async(&mut self, file_path: &str) -> Option<&PreviewCache> {
        // Check if already cached
        if self.preview_cache.contains_key(file_path) {
            return self.preview_cache.get(file_path);
        }

        // Check file size before loading (max 1MB for preview)
        const MAX_PREVIEW_SIZE: u64 = 1024 * 1024;
        if let Ok(metadata) = tokio::fs::metadata(file_path).await {
            if metadata.len() > MAX_PREVIEW_SIZE {
                // File too large, create a placeholder cache entry
                let cache = PreviewCache {
                    content: format!("File too large for preview ({} bytes)", metadata.len()),
                    highlighted_lines: std::cell::RefCell::new(HashMap::new()),
                    language: None,
                };
                self.preview_cache.insert(file_path.to_string(), cache);
                return self.preview_cache.get(file_path);
            }
        }

        // Load the file
        let content = match tokio::fs::read_to_string(file_path).await {
            Ok(c) => c,
            Err(_) => return None,
        };

        // Detect language
        let language = crate::syntax::LanguageRegistry::detect_from_path(file_path);

        // Create cache entry
        let cache = PreviewCache {
            content,
            highlighted_lines: std::cell::RefCell::new(HashMap::new()),
            language,
        };

        self.preview_cache.insert(file_path.to_string(), cache);
        self.preview_cache.get(file_path)
    }

    /// Gets preview from cache or loads it (blocking wrapper)
    pub fn get_or_load_preview(&mut self, file_path: &str) -> Option<&PreviewCache> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.get_or_load_preview_async(file_path))
        })
    }

    /// Gets cached preview if available
    pub fn get_preview_cache(&self, file_path: &str) -> Option<&PreviewCache> {
        self.preview_cache.get(file_path)
    }

    /// Gets preview with fallback - prefers current file, but shows last preview if loading
    /// This provides smooth transitions without "Loading..." flicker
    pub fn get_preview_with_fallback(
        &mut self,
        file_path: &str,
    ) -> Option<(&PreviewCache, String)> {
        // Try to get the requested preview
        if let Some(preview) = self.preview_cache.get(file_path) {
            // Update last shown
            self.last_shown_preview = Some(file_path.to_string());
            return Some((preview, file_path.to_string()));
        }

        // Fall back to last shown preview while new one loads
        if let Some(last_path) = &self.last_shown_preview {
            if let Some(preview) = self.preview_cache.get(last_path) {
                // Return the old preview (with its path for reference)
                return Some((preview, last_path.clone()));
            }
        }

        None
    }

    /// Gets the current color scheme
    pub fn get_color_scheme(&self) -> Option<&ColorScheme> {
        self.color_scheme_registry.get(&self.current_color_scheme)
    }

    /// Sets the color scheme by name
    pub fn set_color_scheme(&mut self, name: &str) -> Result<()> {
        if self.color_scheme_registry.get(name).is_some() {
            self.current_color_scheme = name.to_string();
            Ok(())
        } else {
            Err(anyhow::anyhow!("Color scheme '{}' not found", name))
        }
    }

    /// Lists all available color scheme names
    pub fn list_color_schemes(&self) -> Vec<&str> {
        self.color_scheme_registry.list_names()
    }

    /// Gets the current color scheme name
    pub fn current_color_scheme_name(&self) -> &str {
        &self.current_color_scheme
    }

    /// Limits preview cache size to prevent memory bloat
    pub fn trim_preview_cache(&mut self, max_entries: usize) {
        if self.preview_cache.len() > max_entries {
            // Keep only the most recent entries
            // Simple strategy: clear half when limit is exceeded
            let to_remove = self.preview_cache.len() - max_entries / 2;
            let keys_to_remove: Vec<String> =
                self.preview_cache.keys().take(to_remove).cloned().collect();
            for key in keys_to_remove {
                self.preview_cache.remove(&key);
            }
        }
    }

    /// Gets the leader key
    pub fn leader_key(&self) -> char {
        self.leader_key
    }

    /// Sets pending leader state
    pub fn set_pending_leader(&mut self, pending: bool) {
        self.pending_leader = pending;
    }

    /// Gets pending leader state
    pub fn pending_leader(&self) -> bool {
        self.pending_leader
    }

    /// Gets diagnostics for the current file (async helper for UI)
    /// Returns None if LSP is not enabled or file has no URI
    pub async fn get_current_file_diagnostics(&self) -> Option<Vec<lsp_types::Diagnostic>> {
        let lsp = self.lsp_state.lsp_manager.as_ref()?;
        let file_path = self.buffer().file_path()?;
        let uri = lsp_types::Url::from_file_path(file_path).ok()?;

        Some(lsp.get_diagnostics(&uri).await)
    }

    /// Gets diagnostic count for the current file (errors, warnings, info, hints)
    pub async fn get_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        if let Some(lsp) = &self.lsp_state.lsp_manager {
            if let Some(file_path) = self.buffer().file_path() {
                if let Ok(uri) = lsp_types::Url::from_file_path(file_path) {
                    return lsp.count_diagnostics(&uri).await;
                }
            }
        }
        (0, 0, 0, 0)
    }

    /// Updates the cached diagnostic count (should be called when diagnostics change)
    pub async fn update_diagnostic_cache(&mut self) {
        let start = std::time::Instant::now();
        self.lsp_state.diagnostic_count = self.get_diagnostic_count().await;
        let duration = start.elapsed().as_micros() as u64;
        self.record_diagnostic_query_duration(duration);
    }

    /// Gets the cached diagnostic count (sync, suitable for UI rendering)
    pub fn cached_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        self.lsp_state.diagnostic_count
    }

    /// Sets the LSP status message
    pub fn set_lsp_status(&mut self, status: String) {
        self.lsp_state.lsp_status = status;
    }

    /// Gets the LSP status message
    pub fn lsp_status(&self) -> &str {
        &self.lsp_state.lsp_status
    }

    /// Registers an active LSP server
    pub fn register_lsp_server(&mut self, language_id: String, server_name: String) {
        self.lsp_state.lsp_status = format!("LSP: {} ready", server_name);
        self.lsp_state
            .active_lsp_servers
            .insert(language_id, server_name);
    }

    /// Unregisters an LSP server
    pub fn unregister_lsp_server(&mut self, language_id: &str) {
        self.lsp_state.active_lsp_servers.remove(language_id);
        if self.lsp_state.active_lsp_servers.is_empty() {
            self.lsp_state.lsp_status.clear();
        }
    }

    /// Clears LSP UI state (hover, completions, code actions)
    /// Should be called when switching buffers or when state becomes stale
    fn clear_lsp_state(&mut self) {
        self.lsp_state.hover_info = None;
        self.lsp_state.hover_scroll = 0;
        self.lsp_state.available_code_actions.clear();
        self.lsp_state.available_completions.clear();
        self.lsp_state.pending_lsp_action = None;
        // Don't clear lsp_status - it's useful to keep for user feedback
    }

    /// Gets active LSP servers
    pub fn active_lsp_servers(&self) -> &HashMap<String, String> {
        &self.lsp_state.active_lsp_servers
    }

    /// Gets current LSP progress message
    pub fn lsp_progress_message(&self) -> Option<String> {
        if let Some(lsp_manager) = &self.lsp_state.lsp_manager {
            return lsp_manager.get_progress_message();
        }
        None
    }

    /// Gets comprehensive LSP information for debugging
    pub fn get_lsp_info(&self) -> String {
        let mut info = String::new();

        // LSP Manager status
        if self.lsp_state.lsp_manager.is_some() {
            info.push_str("LSP Manager: Active\n");
        } else {
            info.push_str("LSP Manager: Not initialized\n");
            return info;
        }

        // Active servers
        if self.lsp_state.active_lsp_servers.is_empty() {
            info.push_str("\nActive Servers: None\n");
        } else {
            info.push_str("\nActive Servers:\n");
            for (lang_id, server_name) in &self.lsp_state.active_lsp_servers {
                info.push_str(&format!("  {} -> {}\n", lang_id, server_name));
            }
        }

        // Current file
        if let Some(path) = self.buffer().file_path() {
            info.push_str(&format!("\nCurrent File: {}\n", path));
        }

        // Diagnostic counts
        let (errors, warnings, infos, hints) = self.lsp_state.diagnostic_count;
        info.push_str(&format!("\nDiagnostics:\n"));
        info.push_str(&format!("  Errors: {}\n", errors));
        info.push_str(&format!("  Warnings: {}\n", warnings));
        info.push_str(&format!("  Info: {}\n", infos));
        info.push_str(&format!("  Hints: {}\n", hints));

        // Current status
        if !self.lsp_state.lsp_status.is_empty() {
            info.push_str(&format!("\nStatus: {}\n", self.lsp_state.lsp_status));
        }

        info
    }

    /// Triggers async re-highlighting if needed
    pub async fn process_pending_rehighlight(&mut self) {
        // Check if buffer needs re-highlighting
        let Some((content, version, language)) = self.buffer().get_rehighlight_data() else {
            return;
        };

        // Spawn blocking task for CPU-intensive parsing
        let highlights = tokio::task::spawn_blocking(move || {
            // Create a new highlighter for this language
            let mut highlighter = match crate::syntax::SyntaxHighlighter::new(language) {
                Ok(h) => h,
                Err(_) => return None,
            };

            // Parse the content
            highlighter.parse(&content);

            // Build highlights for all lines using efficient single-pass method
            let all_highlights = highlighter.highlights_for_all_lines(&content);

            Some(all_highlights)
        })
        .await;

        // Apply highlights if successful and version still matches
        if let Ok(Some(highlights)) = highlights {
            self.buffer_mut().apply_highlights(highlights, version);
        }
    }

    /// Request go to definition (sets pending action)
    pub fn request_goto_definition(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::GoToDefinition);
    }

    /// Request go to implementation (sets pending action)
    pub fn request_goto_implementation(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::GoToImplementation);
    }

    /// Request go to type definition (sets pending action)
    pub fn request_goto_type(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::GoToType);
    }

    /// Requests hover information for current cursor position
    pub fn request_hover(&mut self) {
        crate::lsp_debug!(
            "LSP-HOVER",
            "request_hover() called - setting pending action"
        );
        self.lsp_state.pending_lsp_action = Some(LspAction::ShowHover);
    }

    /// Requests code completion for current cursor position
    pub fn request_completion(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::Completion);
    }

    /// Requests document formatting
    pub fn request_format_document(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::FormatDocument);
    }

    /// Requests code actions for current cursor position
    pub fn request_code_actions(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::CodeActions);
    }

    /// Requests call hierarchy incoming calls (who calls this method)
    pub fn request_call_hierarchy_incoming(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::CallHierarchyIncoming);
    }

    /// Requests call hierarchy outgoing calls (what this method calls)
    pub fn request_call_hierarchy_outgoing(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::CallHierarchyOutgoing);
    }

    /// Requests type hierarchy (superclasses/interfaces and subclasses/implementations)
    pub fn request_type_hierarchy(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::TypeHierarchy);
    }

    /// Requests organize imports command for Java
    pub fn request_organize_imports(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::OrganizeImports);
    }

    /// Requests find all references to symbol at cursor
    pub fn request_find_references(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::FindReferences);
    }

    /// Requests document symbols (outline)
    pub fn request_document_symbols(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::DocumentSymbols);
    }

    /// Requests workspace-wide symbol search
    pub fn request_workspace_symbols(&mut self) {
        self.lsp_state.pending_lsp_action = Some(LspAction::WorkspaceSymbols);
    }

    /// Requests to rename the symbol at cursor
    pub fn request_rename(&mut self, new_name: String) {
        self.lsp_state.pending_lsp_action = Some(LspAction::Rename(new_name));
    }

    /// Gets the current hover information (if any)
    pub fn hover_info(&self) -> Option<&str> {
        self.lsp_state.hover_info.as_deref()
    }

    /// Clears the hover information
    pub fn clear_hover(&mut self) {
        self.lsp_state.hover_info = None;
        self.lsp_state.hover_scroll = 0;
    }

    /// Gets the hover scroll offset
    pub fn hover_scroll(&self) -> usize {
        self.lsp_state.hover_scroll
    }

    /// Scrolls the hover window down
    pub fn scroll_hover_down(&mut self, lines: usize) {
        if self.lsp_state.hover_info.is_some() {
            self.lsp_state.hover_scroll = self.lsp_state.hover_scroll.saturating_add(lines);
        }
    }

    /// Scrolls the hover window up
    pub fn scroll_hover_up(&mut self, lines: usize) {
        self.lsp_state.hover_scroll = self.lsp_state.hover_scroll.saturating_sub(lines);
    }

    /// Get (or create) the document sync state for the current buffer
    fn document_sync_state_mut(&mut self) -> Option<&mut lsp_state::DocumentSyncState> {
        let file_path = self.buffer().file_path()?.to_string();
        Some(self.lsp_state.document_sync.entry(file_path).or_default())
    }

    /// Marks that the buffer was modified (for LSP notification)
    pub fn mark_buffer_modified(&mut self) {
        if let Some(state) = self.document_sync_state_mut() {
            state.buffer_modified = true;
        }
    }

    /// Marks that the buffer was saved (for LSP notification)
    pub fn mark_buffer_saved(&mut self) {
        if let Some(state) = self.document_sync_state_mut() {
            state.buffer_saved = true;
        }
    }

    /// Sets the last synced content for incremental LSP sync
    pub fn set_last_synced_content(&mut self, file_path: &str, content: Option<String>) {
        let state = self
            .lsp_state
            .document_sync
            .entry(file_path.to_string())
            .or_default();
        state.last_synced_content = content;
        // Mark document as opened since we're setting synced content (called after didOpen)
        state.did_open_sent = true;
    }

    /// Sends didChange notification if buffer was modified, then resets the flag
    pub async fn send_lsp_changes_if_modified(&mut self) {
        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return;
        };

        let file_path_string = file_path.to_string();

        let Ok(uri) = lsp_types::Url::from_file_path(std::path::Path::new(&file_path_string))
        else {
            return;
        };

        let state_key = file_path_string.clone();

        let old_content = {
            let state = self
                .lsp_state
                .document_sync
                .entry(state_key.clone())
                .or_default();

            if !state.buffer_modified {
                return;
            }

            state.last_synced_content.clone()
        };

        // Detect language from file path
        let Some(language_id) =
            crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path_string)
        else {
            return;
        };

        // Send document sync with debouncing (incremental if supported)
        let serialize_start = std::time::Instant::now();
        let content = self.buffer().rope().to_string();
        let serialize_duration = serialize_start.elapsed().as_micros() as u64;

        let result = lsp
            .did_change(uri, language_id, content.clone(), old_content)
            .await;

        // Record serialize duration after we're done using lsp reference
        self.record_lsp_serialize_duration(serialize_duration);

        let state = self.lsp_state.document_sync.entry(state_key).or_default();

        match result {
            Ok(_) => {
                state.buffer_modified = false;
                state.last_synced_content = Some(content);
            }
            Err(e) => {
                // Restore flag so we retry the sync
                state.buffer_modified = true;
                self.set_lsp_status(format!("LSP: didChange failed: {}", e));
            }
        }
    }

    /// Sends didSave notification if buffer was saved, then resets the flag
    pub async fn send_lsp_save_if_needed(&mut self) {
        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return;
        };

        let file_path_string = file_path.to_string();

        let Ok(uri) = lsp_types::Url::from_file_path(std::path::Path::new(&file_path_string))
        else {
            return;
        };

        let state_key = file_path_string.clone();

        {
            let state = self
                .lsp_state
                .document_sync
                .entry(state_key.clone())
                .or_default();
            if !state.buffer_saved {
                return;
            }
        }

        // Detect language from file path
        let Some(language_id) =
            crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path_string)
        else {
            return;
        };

        let text = Some(self.buffer().rope().to_string());

        let result = lsp.did_save(uri, language_id, text).await;

        let state = self.lsp_state.document_sync.entry(state_key).or_default();

        state.buffer_saved = false;

        if let Err(e) = result {
            // Restore flag so we retry and surface error
            state.buffer_saved = true;
            self.set_lsp_status(format!("LSP: didSave failed: {}", e));
        }
    }

    /// Ensures the current document state is synced with the LSP server
    async fn ensure_lsp_document_synced(&mut self) {
        self.send_lsp_changes_if_modified().await;
        self.send_lsp_save_if_needed().await;
    }

    /// Ensures the document is opened with the LSP server
    /// Returns Ok(true) if document is opened, Ok(false) if not applicable, Err on failure
    async fn ensure_document_opened(&mut self) -> Result<bool> {
        crate::lsp_debug!("LSP-HOVER", "ensure_document_opened() called");

        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            crate::lsp_debug!("LSP-HOVER", "No LSP manager available");
            return Ok(false);
        };

        let Some(file_path) = self.buffer().file_path() else {
            crate::lsp_debug!("LSP-HOVER", "No file path for buffer");
            return Ok(false);
        };

        let file_path_string = file_path.to_string();

        // Convert to absolute path
        let abs_path = if std::path::Path::new(&file_path_string).is_absolute() {
            file_path_string.clone()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(&file_path_string).to_string_lossy().to_string(),
                Err(_) => return Ok(false),
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        let state_key = abs_path.clone();

        // Check if document is already opened
        let is_opened = {
            let state = self
                .lsp_state
                .document_sync
                .entry(state_key.clone())
                .or_default();
            state.did_open_sent
        };

        if is_opened {
            crate::lsp_info!("LSP-HOVER", "Document already opened for: {}", state_key);
            return Ok(true);
        }

        crate::lsp_info!(
            "LSP-HOVER",
            "Document NOT opened yet for: {}, sending didOpen",
            state_key
        );

        // Document not opened - send didOpen now
        let Some(language_id) = crate::syntax::LanguageRegistry::get_lsp_language_id(&abs_path)
        else {
            crate::lsp_info!("LSP-HOVER", "Could not determine language ID for: {}", abs_path);
            return Ok(false);
        };

        let content = self.buffer().rope().to_string();
        crate::lsp_info!("LSP-HOVER", "Sending didOpen for language: {}, content length: {} bytes", language_id, content.len());

        match lsp
            .did_open(uri.clone(), language_id, 1, content.clone())
            .await
        {
            Ok(_) => {
                // Mark document as opened
                let state = self.lsp_state.document_sync.entry(state_key).or_default();
                state.did_open_sent = true;
                state.last_synced_content = Some(content);
                crate::lsp_info!("LSP-HOVER", "didOpen sent successfully");
                Ok(true)
            }
            Err(e) => {
                crate::lsp_info!("LSP-HOVER", "didOpen failed: {}", e);
                Err(e)
            }
        }
    }

    /// Sends didClose notification for pending file (when switching files)
    pub async fn send_lsp_close_if_needed(&mut self) {
        let Some(file_path) = self.lsp_state.pending_did_close_file.take() else {
            return;
        };

        let Some(ref lsp) = self.lsp_state.lsp_manager else {
            return;
        };

        let file_path_string = file_path.clone();

        let Ok(uri) = lsp_types::Url::from_file_path(std::path::Path::new(&file_path_string))
        else {
            return;
        };

        self.lsp_state.document_sync.remove(&file_path_string);

        // Detect language from file path
        let Some(language_id) =
            crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path_string)
        else {
            return;
        };

        let _ = lsp.did_close(uri, language_id).await;
    }

    /// Process any pending LSP actions
    pub async fn process_pending_lsp_actions(&mut self) {
        if let Some(action) = self.lsp_state.pending_lsp_action.take() {
            crate::lsp_debug!(
                "LSP-HOVER",
                "process_pending_lsp_actions() - processing action: {:?}",
                action
            );
            let result = match &action {
                LspAction::GoToDefinition => self.goto_definition_impl().await,
                LspAction::GoToImplementation => self.goto_implementation_impl().await,
                LspAction::GoToType => self.goto_type_impl().await,
                LspAction::ShowHover => {
                    crate::lsp_debug!("LSP-HOVER", "About to call hover_impl()");
                    self.hover_impl().await
                }
                LspAction::Completion => self.completion_impl().await,
                LspAction::FormatDocument => self.format_document_impl().await,
                LspAction::CodeActions => self.code_actions_impl().await,
                LspAction::TypeHierarchy => self.type_hierarchy_impl().await,
                LspAction::CallHierarchyIncoming => self.call_hierarchy_incoming_impl().await,
                LspAction::CallHierarchyOutgoing => self.call_hierarchy_outgoing_impl().await,
                LspAction::FindReferences => self.find_references_impl().await,
                LspAction::DocumentSymbols => self.document_symbols_impl().await,
                LspAction::WorkspaceSymbols => self.workspace_symbols_impl().await,
                LspAction::OrganizeImports => self.organize_imports_impl().await,
                LspAction::Rename(new_name) => self.rename_impl(new_name.clone()).await,
            };

            // Handle errors: update status and optionally retry
            match result {
                Ok(_) => {
                    // Success - status was already updated by the impl function
                }
                Err(e) => {
                    // Check if error message indicates we should retry (e.g., "LSP busy")
                    let error_msg = e.to_string();
                    let should_retry =
                        error_msg.contains("LSP busy") || error_msg.contains("couldn't get lock");

                    if should_retry {
                        // Retry silently - put action back
                        self.lsp_state.pending_lsp_action = Some(action);
                    } else {
                        // Permanent error - update status to show the error
                        let action_name = match &action {
                            LspAction::GoToDefinition => "Go to definition",
                            LspAction::GoToImplementation => "Go to implementation",
                            LspAction::GoToType => "Go to type",
                            LspAction::ShowHover => "Hover",
                            LspAction::Completion => "Completion",
                            LspAction::FormatDocument => "Format document",
                            LspAction::CodeActions => "Code actions",
                            LspAction::TypeHierarchy => "Type hierarchy",
                            LspAction::CallHierarchyIncoming => "Call hierarchy incoming",
                            LspAction::CallHierarchyOutgoing => "Call hierarchy outgoing",
                            LspAction::FindReferences => "Find references",
                            LspAction::DocumentSymbols => "Document symbols",
                            LspAction::WorkspaceSymbols => "Workspace symbols",
                            LspAction::OrganizeImports => "Organize imports",
                            LspAction::Rename(_) => "Rename",
                        };
                        self.set_lsp_status(format!("{} failed: {}", action_name, error_msg));
                    }
                }
            }
        }
    }

    /// Converts a column position to UTF-16 code units for LSP
    ///
    /// LSP spec requires character positions in UTF-16 code units, not byte offsets.
    /// This is critical for correct positioning with rust-analyzer and other LSP servers.
    fn col_to_utf16(&self, line: usize, col: usize) -> u32 {
        let rope = self.buffer().rope();
        if line >= rope.len_lines() {
            return 0;
        }

        let line_text = rope.line(line);

        // CRITICAL: rope.line() includes the trailing newline, but LSP positions
        // should NOT include it. Exclude newline when calculating char count
        // and when iterating for UTF-16 conversion to prevent off-by-one errors
        // at end-of-line positions (hover, goto definition, etc.)
        let chars_without_newline = line_text.chars().take_while(|&c| c != '\n').count();
        let safe_col = col.min(chars_without_newline);

        // Convert to UTF-16 code units, excluding the newline
        line_text
            .chars()
            .take_while(|&c| c != '\n')
            .take(safe_col)
            .map(|c| c.len_utf16() as u32)
            .sum()
    }

    /// Converts UTF-16 code units (from LSP) back to character column position
    ///
    /// LSP responses provide positions in UTF-16 code units. This converts them
    /// back to character positions for rope operations.
    fn utf16_to_col(&self, line: usize, utf16_col: u32) -> usize {
        let rope = self.buffer().rope();
        if line >= rope.len_lines() {
            return 0;
        }

        let line_text = rope.line(line);
        let mut utf16_offset = 0u32;
        let mut char_position = 0usize;

        for ch in line_text.chars() {
            if utf16_offset >= utf16_col {
                break;
            }
            utf16_offset += ch.len_utf16() as u32;
            char_position += 1;
        }

        char_position
    }

    /// Go to definition at current cursor position via LSP (implementation)
    async fn goto_definition_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use goto-definition".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Check if LSP server is ready
        if let Some(server) = lsp.get_server(language_id).await {
            if !server.is_ready().await {
                self.set_lsp_status(
                    "LSP server still initializing (try again in a moment)".to_string(),
                );
                return Ok(false);
            }
        } else {
            self.set_lsp_status("LSP server not started for this language".to_string());
            return Ok(false);
        }

        // Request definition
        self.set_lsp_status("Searching for definition...".to_string());

        // Ensure document is opened with LSP server
        if let Err(e) = self.ensure_document_opened().await {
            self.set_lsp_status(format!("Failed to open document with LSP: {}", e));
            return Ok(false);
        }

        // Make sure the LSP server has the latest buffer contents
        self.ensure_lsp_document_synced().await;

        // CRITICAL FIX: Flush pending changes before goto definition
        // Ensures LSP server has the latest content
        let _ = lsp.flush_pending_changes(&uri).await;

        // Adaptive delay based on language server processing time
        // rust-analyzer and jdtls need more time to index and process changes
        let delay_ms = match language_id {
            "rust" => 100, // rust-analyzer needs more time
            "java" => 150, // jdtls needs even more
            _ => 50,       // Other servers are faster
        };
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;

        let location = lsp
            .goto_definition(&uri, line, character, language_id)
            .await?;

        // Jump to definition if found
        if let Some(location) = location {
            let target_line = location.range.start.line as usize;
            let target_col = location.range.start.character as usize;

            // Save current position to jump list before jumping
            let current_line = self.buffer().cursor().line();
            let current_col = self.buffer().cursor().col();
            self.jump_list.add_jump(current_line, current_col);

            // Check if definition is in the same file
            if location.uri == uri {
                // Same file - jump directly
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(target_line, target_col);
                self.set_lsp_status(format!("Definition found at line {}", target_line + 1));
                return Ok(true);
            } else {
                // Different file - open it and jump
                match location.uri.to_file_path() {
                    Ok(target_path) => {
                        // Try to open the target file
                        match self.load_file_async(&target_path).await {
                            Ok(_) => {
                                self.buffer_mut()
                                    .cursor_mut()
                                    .set_position(target_line, target_col);
                                let file_name = target_path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("file");
                                self.set_lsp_status(format!(
                                    "Opened {} at line {}",
                                    file_name,
                                    target_line + 1
                                ));
                                return Ok(true);
                            }
                            Err(e) => {
                                self.set_lsp_status(format!("Failed to open file: {}", e));
                                return Ok(false);
                            }
                        }
                    }
                    Err(_) => {
                        self.set_lsp_status("Definition in invalid file path".to_string());
                        return Ok(false);
                    }
                }
            }
        }

        // No definition found
        self.set_lsp_status("No definition found".to_string());
        Ok(false)
    }

    /// Go to implementation at current cursor position via LSP (implementation)
    async fn goto_implementation_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use goto-implementation".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request implementation
        self.set_lsp_status("Searching for implementation...".to_string());

        self.ensure_lsp_document_synced().await;

        let location = lsp
            .implementation(&uri, line, character, language_id)
            .await?;

        // Jump to implementation if found
        if let Some(location) = location {
            let target_line = location.range.start.line as usize;
            let target_col = location.range.start.character as usize;

            // Save current position to jump list before jumping
            let current_line = self.buffer().cursor().line();
            let current_col = self.buffer().cursor().col();
            self.jump_list.add_jump(current_line, current_col);

            // Check if implementation is in the same file
            if location.uri == uri {
                // Same file - jump directly
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(target_line, target_col);
                self.set_lsp_status(format!("Implementation found at line {}", target_line + 1));
                return Ok(true);
            } else {
                // Different file - open it and jump
                match location.uri.to_file_path() {
                    Ok(target_path) => {
                        // Try to open the target file
                        match self.load_file_async(&target_path).await {
                            Ok(_) => {
                                self.buffer_mut()
                                    .cursor_mut()
                                    .set_position(target_line, target_col);
                                let file_name = target_path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("file");
                                self.set_lsp_status(format!(
                                    "Opened {} at line {}",
                                    file_name,
                                    target_line + 1
                                ));
                                return Ok(true);
                            }
                            Err(e) => {
                                self.set_lsp_status(format!("Failed to open file: {}", e));
                                return Ok(false);
                            }
                        }
                    }
                    Err(_) => {
                        self.set_lsp_status("Implementation in invalid file path".to_string());
                        return Ok(false);
                    }
                }
            }
        }

        // No implementation found
        self.set_lsp_status("No implementation found".to_string());
        Ok(false)
    }

    /// Go to type definition at current cursor position via LSP (implementation)
    async fn goto_type_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use goto-type".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request type definition
        self.set_lsp_status("Searching for type definition...".to_string());

        self.ensure_lsp_document_synced().await;

        let location = lsp
            .type_definition(&uri, line, character, language_id)
            .await?;

        // Jump to type definition if found
        if let Some(location) = location {
            let target_line = location.range.start.line as usize;
            let target_col = location.range.start.character as usize;

            // Save current position to jump list before jumping
            let current_line = self.buffer().cursor().line();
            let current_col = self.buffer().cursor().col();
            self.jump_list.add_jump(current_line, current_col);

            // Check if type definition is in the same file
            if location.uri == uri {
                // Same file - jump directly
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(target_line, target_col);
                self.set_lsp_status(format!("Type definition found at line {}", target_line + 1));
                return Ok(true);
            } else {
                // Different file - open it and jump
                match location.uri.to_file_path() {
                    Ok(target_path) => {
                        // Try to open the target file
                        match self.load_file_async(&target_path).await {
                            Ok(_) => {
                                self.buffer_mut()
                                    .cursor_mut()
                                    .set_position(target_line, target_col);
                                let file_name = target_path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("file");
                                self.set_lsp_status(format!(
                                    "Opened {} at line {}",
                                    file_name,
                                    target_line + 1
                                ));
                                return Ok(true);
                            }
                            Err(e) => {
                                self.set_lsp_status(format!("Failed to open file: {}", e));
                                return Ok(false);
                            }
                        }
                    }
                    Err(_) => {
                        self.set_lsp_status("Type definition in invalid file path".to_string());
                        return Ok(false);
                    }
                }
            }
        }

        // No type definition found
        self.set_lsp_status("No type definition found".to_string());
        Ok(false)
    }

    /// Find all references to symbol at current cursor position (implementation)
    async fn find_references_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use find references".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request references
        self.set_lsp_status("Searching for references...".to_string());

        self.ensure_lsp_document_synced().await;

        let locations = lsp
            .references(&uri, line, character, language_id, true)
            .await?;

        // Display results in picker
        if locations.is_empty() {
            self.set_lsp_status("No references found".to_string());
            return Ok(false);
        }

        // Store locations in storage vector
        self.lsp_state.available_references = locations.clone();
        self.lsp_state.active_lsp_result_type = Some(LspResultType::References);

        // Format locations as picker items
        let items: Vec<String> = locations
            .iter()
            .map(|loc| {
                let file_path = loc
                    .uri
                    .to_file_path()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "unknown".to_string());
                let line = loc.range.start.line + 1;
                let col = loc.range.start.character + 1;
                format!("{}:{}:{}", file_path, line, col)
            })
            .collect();

        self.set_lsp_status(format!("Found {} references", locations.len()));

        // Create picker with results
        let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let picker = crate::editor::Picker::new_lsp_locations(base_dir, items);
        self.set_picker(picker);
        self.set_mode(crate::mode::Mode::Picker);

        Ok(true)
    }

    /// Show document symbols (outline) (implementation)
    async fn document_symbols_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use document symbols".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request document symbols
        self.set_lsp_status("Loading document symbols...".to_string());

        self.ensure_lsp_document_synced().await;

        let symbols = lsp.document_symbols(&uri, language_id).await?;

        // Display results in picker
        if symbols.is_empty() {
            self.set_lsp_status("No symbols found".to_string());
            return Ok(false);
        }

        // Store symbols in storage vector
        self.lsp_state.available_document_symbols = symbols.clone();
        self.lsp_state.active_lsp_result_type = Some(LspResultType::DocumentSymbols);

        // Format symbols as picker items
        let items: Vec<String> = symbols
            .iter()
            .map(|sym| {
                let line = sym.range.start.line + 1;
                format!("{} ({:?}:{})", sym.name, sym.kind, line)
            })
            .collect();

        self.set_lsp_status(format!("Found {} symbols", symbols.len()));

        // Create picker with results
        let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let picker = crate::editor::Picker::new_lsp_locations(base_dir, items);
        self.set_picker(picker);
        self.set_mode(crate::mode::Mode::Picker);

        Ok(true)
    }

    /// Search workspace symbols (implementation)
    async fn workspace_symbols_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file path for language detection
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use workspace symbols".to_string());
            return Ok(false);
        };

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request workspace symbols with empty query (gets all symbols)
        self.set_lsp_status("Loading workspace symbols...".to_string());

        self.ensure_lsp_document_synced().await;

        let symbols = lsp.workspace_symbols(language_id, String::new()).await?;

        // Display results in picker
        if symbols.is_empty() {
            self.set_lsp_status("No workspace symbols found".to_string());
            return Ok(false);
        }

        // Store symbols in storage vector
        self.lsp_state.available_workspace_symbols = symbols.clone();
        self.lsp_state.active_lsp_result_type = Some(LspResultType::WorkspaceSymbols);

        // Format symbols as picker items
        let items: Vec<String> = symbols
            .iter()
            .map(|sym| {
                let file_name = sym
                    .location
                    .uri
                    .to_file_path()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "unknown".to_string());
                let line = sym.location.range.start.line + 1;
                format!("{} - {} ({:?}:{})", sym.name, file_name, sym.kind, line)
            })
            .collect();

        self.set_lsp_status(format!("Found {} workspace symbols", symbols.len()));

        // Create picker with results
        let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let picker = crate::editor::Picker::new_lsp_locations(base_dir, items);
        self.set_picker(picker);
        self.set_mode(crate::mode::Mode::Picker);

        Ok(true)
    }

    /// Show incoming call hierarchy (implementation)
    async fn call_hierarchy_incoming_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use call hierarchy".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request call hierarchy preparation
        self.set_lsp_status("Preparing call hierarchy...".to_string());

        self.ensure_lsp_document_synced().await;

        let items = lsp
            .prepare_call_hierarchy(uri, line, character, language_id)
            .await?;

        let items = match items {
            Some(items) if !items.is_empty() => items,
            _ => {
                self.set_lsp_status("No call hierarchy item at cursor".to_string());
                return Ok(false);
            }
        };

        // Get incoming calls for the first item
        // Safety: items vector is guaranteed non-empty by the check above (lines 3157-3163),
        // but we handle None defensively to avoid panics in edge cases
        let first_item = items
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No call hierarchy items available"))?;
        let incoming = lsp.incoming_calls(first_item, language_id).await?;

        // Display results in picker
        let calls = match incoming {
            Some(calls) if !calls.is_empty() => calls,
            _ => {
                self.set_lsp_status("No incoming calls found".to_string());
                return Ok(false);
            }
        };

        // Store call hierarchy data in storage vector
        self.lsp_state.available_call_hierarchy = calls
            .iter()
            .map(|call| {
                let name = call.from.name.clone();
                let location = lsp_types::Location {
                    uri: call.from.uri.clone(),
                    range: call.from.range,
                };
                (name, location)
            })
            .collect();
        self.lsp_state.active_lsp_result_type = Some(LspResultType::CallHierarchy);

        // Format calls as picker items
        let items: Vec<String> = calls
            .iter()
            .map(|call| {
                let name = &call.from.name;
                let file_path = call
                    .from
                    .uri
                    .to_file_path()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "unknown".to_string());
                let line = call.from.range.start.line + 1;
                format!("{} - {}:{}", name, file_path, line)
            })
            .collect();

        self.set_lsp_status(format!("Found {} incoming calls", calls.len()));

        // Create picker with results
        let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let picker = crate::editor::Picker::new_lsp_locations(base_dir, items);
        self.set_picker(picker);
        self.set_mode(crate::mode::Mode::Picker);

        Ok(true)
    }

    /// Show outgoing call hierarchy (implementation)
    async fn call_hierarchy_outgoing_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use call hierarchy".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request call hierarchy preparation
        self.set_lsp_status("Preparing call hierarchy...".to_string());

        self.ensure_lsp_document_synced().await;

        let items = lsp
            .prepare_call_hierarchy(uri, line, character, language_id)
            .await?;

        let items = match items {
            Some(items) if !items.is_empty() => items,
            _ => {
                self.set_lsp_status("No call hierarchy item at cursor".to_string());
                return Ok(false);
            }
        };

        // Get outgoing calls for the first item
        // Safety: items vector is guaranteed non-empty by the check above (lines 3276-3282),
        // but we handle None defensively to avoid panics in edge cases
        let first_item = items
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No call hierarchy items available"))?;
        let outgoing = lsp.outgoing_calls(first_item, language_id).await?;

        // Display results in picker
        let calls = match outgoing {
            Some(calls) if !calls.is_empty() => calls,
            _ => {
                self.set_lsp_status("No outgoing calls found".to_string());
                return Ok(false);
            }
        };

        // Store call hierarchy data in storage vector
        self.lsp_state.available_call_hierarchy = calls
            .iter()
            .map(|call| {
                let name = call.to.name.clone();
                let location = lsp_types::Location {
                    uri: call.to.uri.clone(),
                    range: call.to.range,
                };
                (name, location)
            })
            .collect();
        self.lsp_state.active_lsp_result_type = Some(LspResultType::CallHierarchy);

        // Format calls as picker items
        let items: Vec<String> = calls
            .iter()
            .map(|call| {
                let name = &call.to.name;
                let file_path = call
                    .to
                    .uri
                    .to_file_path()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "unknown".to_string());
                let line = call.to.range.start.line + 1;
                format!("{} - {}:{}", name, file_path, line)
            })
            .collect();

        self.set_lsp_status(format!("Found {} outgoing calls", calls.len()));

        // Create picker with results
        let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let picker = crate::editor::Picker::new_lsp_locations(base_dir, items);
        self.set_picker(picker);
        self.set_mode(crate::mode::Mode::Picker);

        Ok(true)
    }

    /// Show type hierarchy (implementation)
    async fn type_hierarchy_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use type hierarchy".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request type hierarchy preparation
        self.set_lsp_status("Preparing type hierarchy...".to_string());

        self.ensure_lsp_document_synced().await;

        let items = lsp
            .prepare_type_hierarchy(uri, line, character, language_id)
            .await?;

        let items = match items {
            Some(items) if !items.is_empty() => items,
            _ => {
                self.set_lsp_status("No type hierarchy item at cursor".to_string());
                return Ok(false);
            }
        };

        // Get supertypes and subtypes for the first item
        // Safety: items vector is guaranteed non-empty by the check above (lines 3395-3401),
        // but we handle None defensively to avoid panics in edge cases
        let first_item = items
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No type hierarchy items available"))?;
        let supertypes = lsp.supertypes(first_item.clone(), language_id).await?;
        let subtypes = lsp.subtypes(first_item, language_id).await?;

        // Combine results and store in storage vector
        let mut all_types_display = Vec::new();
        let mut all_types_data = Vec::new();

        if let Some(supers) = supertypes {
            for super_type in supers {
                let name = super_type.name.clone();
                let location = lsp_types::Location {
                    uri: super_type.uri.clone(),
                    range: super_type.range,
                };
                let file_name = super_type
                    .uri
                    .to_file_path()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "unknown".to_string());
                let line = super_type.range.start.line + 1;
                all_types_display.push(format!("↑ {} - {}:{}", name, file_name, line));
                all_types_data.push((format!("↑ {}", name), location));
            }
        }

        if let Some(subs) = subtypes {
            for sub_type in subs {
                let name = sub_type.name.clone();
                let location = lsp_types::Location {
                    uri: sub_type.uri.clone(),
                    range: sub_type.range,
                };
                let file_name = sub_type
                    .uri
                    .to_file_path()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "unknown".to_string());
                let line = sub_type.range.start.line + 1;
                all_types_display.push(format!("↓ {} - {}:{}", name, file_name, line));
                all_types_data.push((format!("↓ {}", name), location));
            }
        }

        if all_types_data.is_empty() {
            self.set_lsp_status("No type hierarchy found".to_string());
            return Ok(false);
        }

        // Store type hierarchy data in storage vector
        self.lsp_state.available_type_hierarchy = all_types_data;
        self.lsp_state.active_lsp_result_type = Some(LspResultType::TypeHierarchy);

        self.set_lsp_status(format!(
            "Found {} types in hierarchy",
            all_types_display.len()
        ));

        // Create picker with results
        let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let picker = crate::editor::Picker::new_lsp_locations(base_dir, all_types_display);
        self.set_picker(picker);
        self.set_mode(crate::mode::Mode::Picker);

        Ok(true)
    }

    /// Gets hover information at current cursor position via LSP (implementation)
    async fn hover_impl(&mut self) -> Result<bool> {
        crate::lsp_debug!("LSP-HOVER", "hover_impl() called");

        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                crate::lsp_debug!("LSP-HOVER", "No LSP manager in hover_impl");
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use hover".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Check if LSP server is ready
        if let Some(server) = lsp.get_server(language_id).await {
            if !server.is_ready().await {
                self.set_lsp_status(
                    "LSP server still initializing (try again in a moment)".to_string(),
                );
                return Ok(false);
            }
        } else {
            self.set_lsp_status("LSP server not started for this language".to_string());
            return Ok(false);
        }

        // Request hover information
        self.set_lsp_status("Requesting hover information...".to_string());

        // Ensure document is opened with LSP server
        if let Err(e) = self.ensure_document_opened().await {
            self.set_lsp_status(format!("Failed to open document with LSP: {}", e));
            return Ok(false);
        }

        self.ensure_lsp_document_synced().await;

        // CRITICAL FIX: Flush pending changes before hover
        // The didChange notifications are debounced (150ms), so we need to flush
        // to ensure the LSP server has the latest content
        // We do this WITHOUT holding the LspManager lock to avoid blocking
        crate::lsp_info!("LSP-HOVER", "Flushing pending changes before hover");
        let _ = lsp.flush_pending_changes(&uri).await;

        // Adaptive delay based on language server processing time
        // rust-analyzer and jdtls need more time to index and process changes
        let delay_ms = match language_id {
            "rust" => 100, // rust-analyzer needs more time
            "java" => 150, // jdtls needs even more
            _ => 50,       // Other servers are faster
        };
        crate::lsp_info!("LSP-HOVER", "Waiting {}ms before hover request", delay_ms);
        tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;

        crate::lsp_info!("LSP-HOVER", "Sending hover request for URI: {}, line: {}, char: {}, lang: {}", uri, line, character, language_id);
        let hover_text = lsp.hover(&uri, line, character, language_id).await?;

        // Store hover information and enter HoverWindow mode if available
        self.lsp_state.hover_info = hover_text;
        self.lsp_state.hover_scroll = 0; // Reset scroll position

        if self.lsp_state.hover_info.is_some() {
            self.set_mode(Mode::HoverWindow);
            self.set_lsp_status("Hover window opened (q to close, j/k to scroll)".to_string());
            Ok(true)
        } else {
            // No hover info - could be many reasons:
            // 1. Cursor is not on a valid symbol
            // 2. LSP server hasn't indexed this location yet
            // 3. This language/position doesn't have hover info available
            crate::lsp_info!("LSP-HOVER", "Hover returned empty for: URI={}, line={}, char={}", uri, line, character);
            self.set_lsp_status("No hover info at this location (may not be a symbol)".to_string());
            Ok(false)
        }
    }

    /// Requests code completion at current cursor position via LSP (implementation)
    async fn completion_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use completion".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request completion
        self.set_lsp_status("Requesting completion...".to_string());

        self.ensure_lsp_document_synced().await;

        let items = lsp.completion(&uri, line, character, language_id).await?;

        if items.is_empty() {
            self.set_lsp_status("No completion items found".to_string());
            return Ok(false);
        }

        // Store completion items
        self.lsp_state.available_completions = items.clone();

        // Get the current word prefix for filtering
        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col();
        let line_text = self.buffer().line(cursor_line).unwrap_or_default();

        // Find the start of the current word
        let mut trigger_col = cursor_col;
        let chars: Vec<char> = line_text.chars().collect();
        while trigger_col > 0 {
            let prev_char = chars.get(trigger_col - 1).copied().unwrap_or(' ');
            if prev_char.is_alphanumeric() || prev_char == '_' {
                trigger_col -= 1;
            } else {
                break;
            }
        }

        let trigger_prefix = chars[trigger_col..cursor_col].iter().collect::<String>();

        // Show the completion menu
        self.completion_menu
            .show(items, trigger_col, trigger_prefix);
        self.set_lsp_status(format!(
            "{} completions available",
            self.completion_menu.items().len()
        ));

        Ok(true)
    }

    /// Formats the current document via LSP (implementation)
    async fn format_document_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use formatting".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request formatting
        self.set_lsp_status("Formatting document...".to_string());

        self.ensure_lsp_document_synced().await;

        let edits = lsp
            .format_document(&uri, language_id, 4, true) // 4 spaces, insert spaces
            .await?;

        if !edits.is_empty() {
            // Apply text edits to buffer
            self.apply_lsp_edits(edits);
            self.set_lsp_status("Document formatted".to_string());
            Ok(true)
        } else {
            self.set_lsp_status("No formatting changes".to_string());
            Ok(false)
        }
    }

    /// Gets code actions at current cursor position via LSP (implementation)
    async fn code_actions_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use code actions".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Get diagnostics at cursor position for context
        self.set_lsp_status("Requesting code actions...".to_string());

        self.ensure_lsp_document_synced().await;

        let diagnostics = lsp.get_diagnostics_for_line(&uri, line).await;
        let actions = lsp
            .code_actions(&uri, line, character, language_id, diagnostics)
            .await?;

        if actions.is_empty() {
            self.set_lsp_status("No code actions available".to_string());
            return Ok(false);
        }

        // Store actions and create picker
        self.lsp_state.available_code_actions = actions.clone();

        // Extract action titles for picker
        let items: Vec<String> = actions
            .iter()
            .map(|action| match action {
                lsp_types::CodeActionOrCommand::CodeAction(ca) => ca.title.clone(),
                lsp_types::CodeActionOrCommand::Command(cmd) => cmd.title.clone(),
            })
            .collect();

        // Create picker with code action titles
        let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let mut picker = crate::editor::Picker::new_custom(base_dir, items);
        picker.set_prompt("Code Actions: ".to_string());

        self.set_picker(picker);
        self.set_mode(Mode::Picker);
        self.set_lsp_status(format!("{} code actions available", actions.len()));

        Ok(true)
    }

    /// Applies LSP text edits to the buffer
    fn apply_lsp_edits(&mut self, edits: Vec<lsp_types::TextEdit>) {
        // Sort edits in reverse order (bottom to top) to avoid position invalidation
        let mut sorted_edits = edits;
        sorted_edits.sort_by(|a, b| {
            b.range
                .start
                .line
                .cmp(&a.range.start.line)
                .then(b.range.start.character.cmp(&a.range.start.character))
        });

        for edit in sorted_edits {
            let start_line = edit.range.start.line as usize;
            // Convert UTF-16 positions to character positions
            let start_col = self.utf16_to_col(start_line, edit.range.start.character);
            let end_line = edit.range.end.line as usize;
            let end_col = self.utf16_to_col(end_line, edit.range.end.character);

            // Delete the range
            if start_line != end_line || start_col != end_col {
                self.buffer_mut()
                    .delete_range(start_line, start_col, end_line, end_col);
            }

            // Insert new text
            if !edit.new_text.is_empty() {
                self.buffer_mut()
                    .insert_text_at(start_line, start_col, &edit.new_text);
            }
        }
    }

    /// Applies a selected code action from the picker
    pub fn apply_code_action(&mut self, action_index: usize) {
        // Check if we have actions and the index is valid
        if action_index >= self.lsp_state.available_code_actions.len() {
            self.set_lsp_status("Invalid code action selection".to_string());
            return;
        }

        let action = self.lsp_state.available_code_actions[action_index].clone();

        match action {
            lsp_types::CodeActionOrCommand::CodeAction(code_action) => {
                let action_title = code_action.title.clone();

                // Apply the workspace edit if present
                if let Some(ref edit) = code_action.edit {
                    if let Some(ref changes) = edit.changes {
                        // Apply changes for current document
                        if let Some(file_path) = self.buffer().file_path() {
                            let abs_path = if std::path::Path::new(file_path).is_absolute() {
                                file_path.to_string()
                            } else {
                                match std::env::current_dir() {
                                    Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                                    Err(_) => {
                                        self.set_lsp_status(
                                            "Failed to resolve file path".to_string(),
                                        );
                                        return;
                                    }
                                }
                            };

                            if let Ok(uri) = lsp_types::Url::from_file_path(&abs_path) {
                                if let Some(edits) = changes.get(&uri) {
                                    self.apply_lsp_edits(edits.clone());
                                    self.set_lsp_status(format!("Applied: {}", action_title));
                                } else {
                                    self.set_lsp_status("No edits for current file".to_string());
                                }
                            } else {
                                self.set_lsp_status("Invalid file URI".to_string());
                            }
                        } else {
                            self.set_lsp_status("No file loaded".to_string());
                        }
                    } else if let Some(ref document_changes) = edit.document_changes {
                        // Handle document changes (more complex, includes creates/renames/deletes)
                        // DocumentChanges is an enum, extract the operations
                        match document_changes {
                            lsp_types::DocumentChanges::Edits(edits) => {
                                // Process text document edits
                                for text_doc_edit in edits {
                                    // Check if this is for the current document
                                    if let Some(file_path) = self.buffer().file_path() {
                                        let abs_path =
                                            if std::path::Path::new(file_path).is_absolute() {
                                                file_path.to_string()
                                            } else {
                                                match std::env::current_dir() {
                                                    Ok(cwd) => cwd
                                                        .join(file_path)
                                                        .to_string_lossy()
                                                        .to_string(),
                                                    Err(_) => continue,
                                                }
                                            };

                                        if let Ok(uri) = lsp_types::Url::from_file_path(&abs_path) {
                                            if text_doc_edit.text_document.uri == uri {
                                                self.apply_lsp_edits(
                                                    text_doc_edit
                                                        .edits
                                                        .iter()
                                                        .filter_map(|e| match e {
                                                            lsp_types::OneOf::Left(edit) => {
                                                                Some(edit.clone())
                                                            }
                                                            lsp_types::OneOf::Right(annot_edit) => {
                                                                Some(annot_edit.text_edit.clone())
                                                            }
                                                        })
                                                        .collect(),
                                                );
                                                self.set_lsp_status(format!(
                                                    "Applied: {}",
                                                    action_title
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                            lsp_types::DocumentChanges::Operations(ops) => {
                                // Handle mixed operations (edits, creates, renames, deletes)
                                for op in ops {
                                    match op {
                                        lsp_types::DocumentChangeOperation::Edit(text_doc_edit) => {
                                            // Check if this is for the current document
                                            if let Some(file_path) = self.buffer().file_path() {
                                                let abs_path = if std::path::Path::new(file_path)
                                                    .is_absolute()
                                                {
                                                    file_path.to_string()
                                                } else {
                                                    match std::env::current_dir() {
                                                        Ok(cwd) => cwd
                                                            .join(file_path)
                                                            .to_string_lossy()
                                                            .to_string(),
                                                        Err(_) => continue,
                                                    }
                                                };

                                                if let Ok(uri) =
                                                    lsp_types::Url::from_file_path(&abs_path)
                                                {
                                                    if text_doc_edit.text_document.uri == uri {
                                                        self.apply_lsp_edits(
                                                            text_doc_edit
                                                                .edits
                                                                .iter()
                                                                .filter_map(|e| match e {
                                                                    lsp_types::OneOf::Left(
                                                                        edit,
                                                                    ) => Some(edit.clone()),
                                                                    lsp_types::OneOf::Right(
                                                                        annot_edit,
                                                                    ) => Some(
                                                                        annot_edit
                                                                            .text_edit
                                                                            .clone(),
                                                                    ),
                                                                })
                                                                .collect(),
                                                        );
                                                        self.set_lsp_status(format!(
                                                            "Applied: {}",
                                                            action_title
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                        _ => {
                                            // Other operations (create, rename, delete) not supported yet
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        self.set_lsp_status("Code action has no edits".to_string());
                    }
                } else {
                    self.set_lsp_status("Code action has no edits".to_string());
                }
            }
            lsp_types::CodeActionOrCommand::Command(cmd) => {
                // Execute the command via LSP
                let command_title = cmd.title.clone();
                let command_name = cmd.command.clone();
                let arguments = cmd.arguments.clone();

                // Get language ID
                let Some(file_path) = self.buffer().file_path() else {
                    self.set_lsp_status("No file loaded".to_string());
                    return;
                };

                let Some(language_id) =
                    crate::syntax::LanguageRegistry::get_lsp_language_id(file_path)
                else {
                    self.set_lsp_status("Language not supported".to_string());
                    return;
                };

                // Execute command asynchronously
                if let Some(lsp) = self.lsp_state.lsp_manager.clone() {
                    self.set_lsp_status(format!("Executing: {}", command_title));

                    tokio::spawn(async move {
                        let _result = lsp
                            .execute_command(command_name, arguments, &language_id)
                            .await;
                        // Note: Result isn't sent back to editor - this is fire and forget
                        // A full implementation would use a channel to send results back
                    });
                } else {
                    self.set_lsp_status("LSP not available".to_string());
                }
            }
        }

        // Clear available actions after applying
        self.lsp_state.available_code_actions.clear();
    }

    /// Applies the selected completion item
    pub fn apply_completion(&mut self, completion_index: usize) {
        // Check if we have completions and the index is valid
        if completion_index >= self.lsp_state.available_completions.len() {
            self.set_lsp_status("Invalid completion selection".to_string());
            return;
        }

        // Clone the completion data we need before mutable borrow
        let completion = self.lsp_state.available_completions[completion_index].clone();
        let insert_text = completion
            .insert_text
            .as_ref()
            .unwrap_or(&completion.label)
            .clone();
        let label = completion.label.clone();

        // Insert the completion text at cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line();
        let col = cursor.col();

        // Get the line's char index
        let line_char_idx = self.buffer().rope().line_to_char(line);
        let insert_pos = line_char_idx + col;

        // Insert the text
        self.buffer_mut()
            .rope_mut()
            .insert(insert_pos, &insert_text);

        // Move cursor to end of inserted text
        let new_col = col + insert_text.chars().count();
        self.buffer_mut().cursor_mut().set_position(line, new_col);

        self.set_lsp_status(format!("Inserted: {}", label));

        // Clear available completions after applying
        self.lsp_state.available_completions.clear();
    }

    /// Navigates to an LSP location from the picker selection
    pub fn navigate_to_lsp_location(&mut self, index: usize) {
        // Determine which LSP result type we're viewing
        let result_type = match &self.lsp_state.active_lsp_result_type {
            Some(t) => t.clone(),
            None => {
                self.set_lsp_status("No active LSP results".to_string());
                return;
            }
        };

        // Get the location based on result type
        let location = match result_type {
            LspResultType::References => {
                if index >= self.lsp_state.available_references.len() {
                    self.set_lsp_status("Invalid reference selection".to_string());
                    return;
                }
                self.lsp_state.available_references[index].clone()
            }
            LspResultType::DocumentSymbols => {
                if index >= self.lsp_state.available_document_symbols.len() {
                    self.set_lsp_status("Invalid symbol selection".to_string());
                    return;
                }
                let symbol = &self.lsp_state.available_document_symbols[index];
                // For document symbols, the location is in the current file
                let file_path = self.buffer().file_path().unwrap_or("").to_string();
                let uri = match lsp_types::Url::from_file_path(&file_path) {
                    Ok(u) => u,
                    Err(_) => {
                        self.set_lsp_status("Invalid file path".to_string());
                        return;
                    }
                };
                lsp_types::Location {
                    uri,
                    range: symbol.range,
                }
            }
            LspResultType::WorkspaceSymbols => {
                if index >= self.lsp_state.available_workspace_symbols.len() {
                    self.set_lsp_status("Invalid symbol selection".to_string());
                    return;
                }
                self.lsp_state.available_workspace_symbols[index]
                    .location
                    .clone()
            }
            LspResultType::CallHierarchy | LspResultType::TypeHierarchy => {
                let storage = if result_type == LspResultType::CallHierarchy {
                    &self.lsp_state.available_call_hierarchy
                } else {
                    &self.lsp_state.available_type_hierarchy
                };

                if index >= storage.len() {
                    self.set_lsp_status("Invalid selection".to_string());
                    return;
                }
                storage[index].1.clone()
            }
        };

        // Convert LSP location to file path
        let file_path = match location.uri.to_file_path() {
            Ok(path) => path.to_string_lossy().to_string(),
            Err(_) => {
                self.set_lsp_status("Invalid file URI".to_string());
                return;
            }
        };

        // Load the file
        if let Err(e) = self.load_file(&file_path) {
            self.set_lsp_status(format!("Failed to load file: {}", e));
            return;
        }

        // Move cursor to the location
        let line = location.range.start.line as usize;
        let col = location.range.start.character as usize;
        self.buffer_mut().cursor_mut().set_position(line, col);

        let file_name = std::path::Path::new(&file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        self.set_lsp_status(format!("Opened {} at line {}", file_name, line + 1));
    }

    /// Organizes imports in the current file via LSP (implementation)
    async fn organize_imports_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use organize imports".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // For Java, use the organize imports command
        if language_id != "java" {
            self.set_lsp_status("Organize imports only supported for Java".to_string());
            return Ok(false);
        }

        // Request organize imports
        self.set_lsp_status("Organizing imports...".to_string());

        self.ensure_lsp_document_synced().await;

        // Execute the organize imports command
        // For jdtls, the command is "java.action.organizeImports"
        let command = "java.action.organizeImports".to_string();
        let arguments = Some(vec![serde_json::to_value(&uri)?]);

        let result = lsp.execute_command(command, arguments, language_id).await;

        match result {
            Ok(_) => {
                self.set_lsp_status("Imports organized".to_string());
                Ok(true)
            }
            Err(e) => {
                self.set_lsp_status(format!("Failed to organize imports: {}", e));
                Ok(false)
            }
        }
    }

    /// Performs LSP rename operation on the symbol at cursor
    async fn rename_impl(&mut self, new_name: String) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use rename".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = lsp_types::Url::from_file_path(&abs_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position (convert to UTF-16 for LSP)
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request rename
        self.set_lsp_status("Renaming...".to_string());

        self.ensure_lsp_document_synced().await;

        // Call the rename method with individual parameters (using UTF-16 character position)
        let result = lsp
            .rename(&uri, line, character, language_id, new_name.clone())
            .await;

        match result {
            Ok(Some(workspace_edit)) => {
                // Apply the workspace edit
                let applied = self.apply_workspace_edit(workspace_edit).await?;

                if applied {
                    self.set_lsp_status(format!("Renamed to '{}'", new_name));
                    Ok(true)
                } else {
                    self.set_lsp_status("Failed to apply rename edits".to_string());
                    Ok(false)
                }
            }
            Ok(None) => {
                self.set_lsp_status("No rename edits returned".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Rename failed: {}", e));
                Ok(false)
            }
        }
    }

    /// Applies a workspace edit from the LSP server
    /// Returns true if all edits were applied successfully
    pub async fn apply_workspace_edit(&mut self, edit: lsp_types::WorkspaceEdit) -> Result<bool> {
        // Track whether all edits were applied successfully
        let mut all_applied = true;

        // Handle changes (map of URI to TextEdit[])
        if let Some(ref changes) = edit.changes {
            for (uri, text_edits) in changes {
                // Check if this is the current document
                if let Some(file_path) = self.buffer().file_path() {
                    let abs_path = if std::path::Path::new(file_path).is_absolute() {
                        file_path.to_string()
                    } else {
                        match std::env::current_dir() {
                            Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                            Err(_) => {
                                all_applied = false;
                                continue;
                            }
                        }
                    };

                    if let Ok(current_uri) = lsp_types::Url::from_file_path(&abs_path) {
                        if current_uri == *uri {
                            // Apply edits to current buffer
                            self.apply_lsp_edits(text_edits.clone());
                        } else {
                            // Different file - would need to open and edit it
                            // For now, mark as not fully applied
                            all_applied = false;
                        }
                    } else {
                        all_applied = false;
                    }
                } else {
                    all_applied = false;
                }
            }
        }

        // Handle document_changes (more complex, includes creates/renames/deletes)
        if let Some(ref document_changes) = edit.document_changes {
            match document_changes {
                lsp_types::DocumentChanges::Edits(edits) => {
                    for text_doc_edit in edits {
                        // Check if this is for the current document
                        if let Some(file_path) = self.buffer().file_path() {
                            let abs_path = if std::path::Path::new(file_path).is_absolute() {
                                file_path.to_string()
                            } else {
                                match std::env::current_dir() {
                                    Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                                    Err(_) => {
                                        all_applied = false;
                                        continue;
                                    }
                                }
                            };

                            if let Ok(uri) = lsp_types::Url::from_file_path(&abs_path) {
                                if text_doc_edit.text_document.uri == uri {
                                    // Apply edits to current buffer
                                    let text_edits: Vec<lsp_types::TextEdit> = text_doc_edit
                                        .edits
                                        .iter()
                                        .filter_map(|e| match e {
                                            lsp_types::OneOf::Left(edit) => Some(edit.clone()),
                                            lsp_types::OneOf::Right(annot_edit) => {
                                                Some(annot_edit.text_edit.clone())
                                            }
                                        })
                                        .collect();
                                    self.apply_lsp_edits(text_edits);
                                } else {
                                    all_applied = false;
                                }
                            } else {
                                all_applied = false;
                            }
                        } else {
                            all_applied = false;
                        }
                    }
                }
                lsp_types::DocumentChanges::Operations(ops) => {
                    for op in ops {
                        match op {
                            lsp_types::DocumentChangeOperation::Edit(text_doc_edit) => {
                                // Check if this is for the current document
                                if let Some(file_path) = self.buffer().file_path() {
                                    let abs_path = if std::path::Path::new(file_path).is_absolute()
                                    {
                                        file_path.to_string()
                                    } else {
                                        match std::env::current_dir() {
                                            Ok(cwd) => {
                                                cwd.join(file_path).to_string_lossy().to_string()
                                            }
                                            Err(_) => {
                                                all_applied = false;
                                                continue;
                                            }
                                        }
                                    };

                                    if let Ok(uri) = lsp_types::Url::from_file_path(&abs_path) {
                                        if text_doc_edit.text_document.uri == uri {
                                            // Apply edits to current buffer
                                            let text_edits: Vec<lsp_types::TextEdit> =
                                                text_doc_edit
                                                    .edits
                                                    .iter()
                                                    .filter_map(|e| match e {
                                                        lsp_types::OneOf::Left(edit) => {
                                                            Some(edit.clone())
                                                        }
                                                        lsp_types::OneOf::Right(annot_edit) => {
                                                            Some(annot_edit.text_edit.clone())
                                                        }
                                                    })
                                                    .collect();
                                            self.apply_lsp_edits(text_edits);
                                        } else {
                                            all_applied = false;
                                        }
                                    } else {
                                        all_applied = false;
                                    }
                                } else {
                                    all_applied = false;
                                }
                            }
                            _ => {
                                // Other operations (create, rename, delete) not supported yet
                                all_applied = false;
                            }
                        }
                    }
                }
            }
        }

        Ok(all_applied)
    }

    /// Renders the editor to an in-memory buffer and returns ANSI output
    /// Used for headless mode to get pixel-perfect terminal representation
    pub fn render_to_ansi(&mut self, width: u16, height: u16) -> Result<String> {
        use crate::ui::buffer_to_ansi;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        // Create a test backend with specified dimensions
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend)?;

        // Render using the normal UI rendering code
        terminal.draw(|f| {
            crate::ui::Renderer::render_to_frame(f, self);
        })?;

        // Convert buffer to ANSI string
        let buffer = terminal.backend().buffer();
        Ok(buffer_to_ansi(buffer))
    }

    // === Window Management ===

    /// Gets a reference to the window manager
    pub fn window_manager(&self) -> Option<&WindowManager> {
        self.window_manager.as_ref()
    }

    /// Gets a mutable reference to the window manager
    pub fn window_manager_mut(&mut self) -> Option<&mut WindowManager> {
        self.window_manager.as_mut()
    }

    /// Initializes the window manager with the current viewport dimensions
    /// Call this once viewport size is known (typically from UI layer)
    pub fn init_window_manager(&mut self, width: u16, height: u16) {
        if self.window_manager.is_none() {
            self.window_manager = Some(WindowManager::new(0, width, height));
        }
    }

    /// Splits the current window horizontally (creates window above/below)
    pub fn split_window_horizontal(&mut self) {
        // Initialize window manager if needed (fallback dimensions)
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        if let Some(wm) = &mut self.window_manager {
            wm.split_focused(SplitDirection::Horizontal, 0);
        }
    }

    /// Splits the current window vertically (creates window left/right)
    pub fn split_window_vertical(&mut self) {
        // Initialize window manager if needed (fallback dimensions)
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        if let Some(wm) = &mut self.window_manager {
            wm.split_focused(SplitDirection::Vertical, 0);
        }
    }

    /// Moves focus to the next window
    pub fn focus_next_window(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            wm.focus_next();
        }
    }

    /// Moves focus to the previous window
    pub fn focus_prev_window(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            wm.focus_prev();
        }
    }

    /// Gets the current number of windows
    pub fn window_count(&self) -> usize {
        self.window_manager
            .as_ref()
            .map(|wm| wm.window_count())
            .unwrap_or(1)
    }

    // === Viewport Scrolling ===

    /// Scrolls viewport down N lines
    pub fn scroll_viewport_down(&mut self, lines: usize) {
        let buffer_line_count = self.buffer().line_count();
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.scroll_down(lines, buffer_line_count);
            }
        }
    }

    /// Scrolls viewport up N lines
    pub fn scroll_viewport_up(&mut self, lines: usize) {
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.scroll_up(lines);
            }
        }
    }

    /// Centers cursor in viewport
    pub fn center_cursor_in_viewport(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.center_cursor();
            }
        }
    }

    /// Moves cursor line to top of viewport
    pub fn move_cursor_line_to_top(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.move_cursor_to_top();
            }
        }
    }

    /// Moves cursor line to bottom of viewport
    pub fn move_cursor_line_to_bottom(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.move_cursor_to_bottom();
            }
        }
    }

    /// Scrolls down half a page (both viewport and cursor)
    pub fn scroll_half_page_down(&mut self) {
        // Extract window info first to avoid borrowing conflicts
        let (viewport_start, viewport_height) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.scroll_offset(), window.height() as usize)
            } else {
                return;
            }
        } else {
            return;
        };

        // Now we can mutably borrow buffer
        let new_viewport =
            Motions::scroll_half_page_down(self.buffer_mut(), viewport_start, viewport_height);

        // Finally update window scroll offset
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_viewport);
            }
        }
    }

    /// Scrolls up half a page (both viewport and cursor)
    pub fn scroll_half_page_up(&mut self) {
        // Extract window info first to avoid borrowing conflicts
        let (viewport_start, viewport_height) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.scroll_offset(), window.height() as usize)
            } else {
                return;
            }
        } else {
            return;
        };

        // Now we can mutably borrow buffer
        let new_viewport =
            Motions::scroll_half_page_up(self.buffer_mut(), viewport_start, viewport_height);

        // Finally update window scroll offset
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_viewport);
            }
        }
    }

    /// Scrolls forward (down) one page (both viewport and cursor)
    pub fn scroll_page_down(&mut self) {
        // Extract window info first to avoid borrowing conflicts
        let (viewport_start, viewport_height) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.scroll_offset(), window.height() as usize)
            } else {
                return;
            }
        } else {
            return;
        };

        // Now we can mutably borrow buffer
        let new_viewport =
            Motions::scroll_page_down(self.buffer_mut(), viewport_start, viewport_height);

        // Finally update window scroll offset
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_viewport);
            }
        }
    }

    /// Scrolls backward (up) one page (both viewport and cursor)
    pub fn scroll_page_up(&mut self) {
        // Extract window info first to avoid borrowing conflicts
        let (viewport_start, viewport_height) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.scroll_offset(), window.height() as usize)
            } else {
                return;
            }
        } else {
            return;
        };

        // Now we can mutably borrow buffer
        let new_viewport =
            Motions::scroll_page_up(self.buffer_mut(), viewport_start, viewport_height);

        // Finally update window scroll offset
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_viewport);
            }
        }
    }

    /// Gets a reference to the completion menu
    pub fn completion_menu(&self) -> &CompletionMenu {
        &self.completion_menu
    }

    /// Gets a mutable reference to the completion menu
    pub fn completion_menu_mut(&mut self) -> &mut CompletionMenu {
        &mut self.completion_menu
    }

    /// Hides the completion menu
    pub fn hide_completion_menu(&mut self) {
        self.completion_menu.hide();
    }

    /// Selects the next completion item
    pub fn completion_next(&mut self) {
        self.completion_menu.select_next();
    }

    /// Selects the previous completion item
    pub fn completion_previous(&mut self) {
        self.completion_menu.select_previous();
    }

    /// Accepts the currently selected completion
    pub fn accept_completion(&mut self) {
        if let Some(item) = self.completion_menu.selected_item() {
            // Get the text to insert (prefer insertText, fallback to label)
            let text_to_insert = if let Some(ref insert_text) = item.insert_text {
                insert_text.clone()
            } else {
                item.label.clone()
            };

            // Get cursor position
            let cursor_line = self.buffer().cursor().line();
            let cursor_col = self.buffer().cursor().col();

            // Calculate the range to replace
            let trigger_col = self.completion_menu.trigger_col();

            // Delete the partial word from trigger position to cursor
            if cursor_col > trigger_col {
                self.buffer_mut()
                    .delete_range(cursor_line, trigger_col, cursor_line, cursor_col);
            }

            // Insert the completion text
            self.buffer_mut()
                .insert_text_at(cursor_line, trigger_col, &text_to_insert);

            // Move cursor to end of inserted text
            let new_col = trigger_col + text_to_insert.chars().count();
            self.buffer_mut()
                .cursor_mut()
                .set_position(cursor_line, new_col);

            // Mark buffer as modified
            self.mark_buffer_modified();
        }

        // Hide the completion menu
        self.hide_completion_menu();
    }

    /// Gets the inlay hints for the current file
    pub fn inlay_hints(&self) -> &[lsp_types::InlayHint] {
        &self.lsp_state.inlay_hints
    }

    /// Gets the file tree
    pub fn file_tree(&self) -> &FileTree {
        &self.file_tree
    }

    /// Gets mutable file tree
    pub fn file_tree_mut(&mut self) -> &mut FileTree {
        &mut self.file_tree
    }

    /// Opens the file tree explorer at the current file's directory
    pub fn open_file_tree(&mut self) {
        // Extract file path first to avoid borrowing issues
        let file_path = self.buffer().file_path().map(|s| s.to_string());

        if let Some(file_path) = file_path {
            if let Some(parent) = std::path::Path::new(&file_path).parent() {
                self.file_tree.open(parent);
                return;
            }
        }

        // Fallback to current directory if no file path
        if let Ok(cwd) = std::env::current_dir() {
            self.file_tree.open(&cwd);
        }
    }

    /// Toggles the file tree visibility
    pub fn toggle_file_tree(&mut self) {
        if !self.file_tree.is_visible() {
            self.open_file_tree();
            self.set_mode(Mode::FileTree);
        } else {
            self.file_tree.toggle();
            self.set_mode(Mode::Normal);
        }
    }

    /// Opens the file selected in the file tree
    pub fn open_file_from_tree(&mut self) {
        if let Some(node) = self.file_tree.selected_node() {
            if node.is_dir() {
                // Toggle directory expansion
                self.file_tree.toggle_selected();
            } else {
                // Open file (checks for existing buffer)
                let path = node.path().to_path_buf();
                if let Ok(()) = self.open_file(&path) {
                    // Switch back to Normal mode and keep file tree visible
                    self.set_mode(Mode::Normal);
                }
            }
        }
    }

    /// Gets the quickfix list
    pub fn quickfix_list(&self) -> &QuickfixList {
        &self.quickfix_list
    }

    /// Gets mutable quickfix list
    pub fn quickfix_list_mut(&mut self) -> &mut QuickfixList {
        &mut self.quickfix_list
    }

    /// Sets the quickfix list entries
    pub fn set_quickfix_list(&mut self, entries: Vec<QuickfixEntry>, title: String) {
        self.quickfix_list.set_entries(entries, title);
    }

    /// Opens the quickfix window
    pub fn open_quickfix_window(&mut self) {
        self.quickfix_window_open = true;
    }

    /// Closes the quickfix window
    pub fn close_quickfix_window(&mut self) {
        self.quickfix_window_open = false;
    }

    /// Toggles the quickfix window
    pub fn toggle_quickfix_window(&mut self) {
        self.quickfix_window_open = !self.quickfix_window_open;
    }

    /// Whether the quickfix window is open
    pub fn is_quickfix_window_open(&self) -> bool {
        self.quickfix_window_open
    }

    /// Jumps to the current quickfix entry
    pub fn jump_to_quickfix_entry(&mut self) {
        // Extract values first to avoid borrow issues
        let (path, lnum, qcol) = if let Some(entry) = self.quickfix_list.current_entry() {
            (entry.filename.clone(), entry.lnum, entry.col)
        } else {
            return;
        };

        if let Some(path) = path {
            // Open file (checks for existing buffer)
            if let Ok(()) = self.open_file(&path) {
                // Move cursor to the location
                if lnum > 0 {
                    let line = lnum.saturating_sub(1);
                    let col = if qcol > 0 { qcol.saturating_sub(1) } else { 0 };
                    self.buffer_mut().cursor_mut().set_position(line, col);
                }
            }
        }
    }

    /// Gets the location list
    pub fn location_list(&self) -> &LocationList {
        &self.location_list
    }

    /// Gets mutable location list
    pub fn location_list_mut(&mut self) -> &mut LocationList {
        &mut self.location_list
    }

    /// Sets the location list entries
    pub fn set_location_list(&mut self, entries: Vec<QuickfixEntry>, title: String) {
        self.location_list.set_entries(entries, title);
    }

    /// Opens the location list window
    pub fn open_location_window(&mut self) {
        self.location_window_open = true;
    }

    /// Closes the location list window
    pub fn close_location_window(&mut self) {
        self.location_window_open = false;
    }

    /// Toggles the location list window
    pub fn toggle_location_window(&mut self) {
        self.location_window_open = !self.location_window_open;
    }

    /// Whether the location list window is open
    pub fn is_location_window_open(&self) -> bool {
        self.location_window_open
    }

    /// Jumps to the current location list entry
    pub fn jump_to_location_entry(&mut self) {
        // Extract values first to avoid borrow issues
        let (path, lnum, lcol) = if let Some(entry) = self.location_list.current_entry() {
            (entry.filename.clone(), entry.lnum, entry.col)
        } else {
            return;
        };

        if let Some(path) = path {
            // Open file (checks for existing buffer)
            if let Ok(()) = self.open_file(&path) {
                // Move cursor to the location
                if lnum > 0 {
                    let line = lnum.saturating_sub(1);
                    let col = if lcol > 0 { lcol.saturating_sub(1) } else { 0 };
                    self.buffer_mut().cursor_mut().set_position(line, col);
                }
            }
        }
    }

    /// Gets the tab page manager
    pub fn tab_page_manager(&self) -> &TabPageManager {
        &self.tab_page_manager
    }

    /// Gets mutable tab page manager
    pub fn tab_page_manager_mut(&mut self) -> &mut TabPageManager {
        &mut self.tab_page_manager
    }

    /// Creates a new tab page
    pub fn new_tab(&mut self, title: Option<String>) {
        // Default to "[No Name]" if no title provided
        let default_title = title.unwrap_or_else(|| "[No Name]".to_string());
        self.tab_page_manager.new_tab(Some(default_title));
    }

    /// Gets the display title for a tab at the given index
    /// Returns filename if file is open, otherwise "[No Name]"
    pub fn get_tab_title(&self, tab_index: usize) -> String {
        if let Some(tabs) = self.tab_page_manager.tabs().get(tab_index) {
            let title = tabs.title();
            // If title starts with "[", it's already a special marker like [No Name]
            // Otherwise it might be an old numeric title - treat as "[No Name]"
            if title.starts_with('[') {
                title.to_string()
            } else if title.len() <= 2 && title.chars().all(|c| c.is_numeric()) {
                // Old numeric title - return [No Name]
                "[No Name]".to_string()
            } else {
                // It's a filename
                title.to_string()
            }
        } else {
            "[No Name]".to_string()
        }
    }

    /// Updates the current tab's title to the current buffer's filename
    pub fn update_current_tab_title(&mut self) {
        let title = if let Some(path) = self.buffer().file_path() {
            // Extract filename from path
            std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("[No Name]")
                .to_string()
        } else {
            "[No Name]".to_string()
        };
        self.tab_page_manager_mut().set_current_tab_title(title);
    }

    /// Closes the current tab
    pub fn close_current_tab(&mut self) {
        self.tab_page_manager.close_current_tab();
    }

    /// Switches to the next tab
    pub fn next_tab(&mut self) {
        self.tab_page_manager.next_tab();
    }

    /// Switches to the previous tab
    pub fn previous_tab(&mut self) {
        self.tab_page_manager.previous_tab();
    }

    /// Switches to a specific tab by index (0-based)
    pub fn goto_tab(&mut self, index: usize) {
        self.tab_page_manager.switch_to_tab(index);
    }

    /// Switches to the first tab
    pub fn first_tab(&mut self) {
        self.tab_page_manager.first_tab();
    }

    /// Switches to the last tab
    pub fn last_tab(&mut self) {
        self.tab_page_manager.last_tab();
    }

    /// Gets the current tab index
    pub fn current_tab_index(&self) -> usize {
        self.tab_page_manager.current_tab_index()
    }

    /// Gets the number of tabs
    pub fn tab_count(&self) -> usize {
        self.tab_page_manager.tab_count()
    }

    /// Close all tabs except the current one
    pub fn close_other_tabs(&mut self) {
        self.tab_page_manager.close_other_tabs();
    }

    /// Performance metrics: increment render count
    pub fn increment_render_count(&mut self) {
        self.render_count = self.render_count.saturating_add(1);
    }

    /// Performance metrics: record render duration
    pub fn record_render_duration(&mut self, duration_micros: u64) {
        self.last_render_duration_micros = Some(duration_micros);
    }

    /// Performance metrics: record syntax highlighting duration
    pub fn record_syntax_duration(&mut self, duration_micros: u64) {
        self.last_syntax_duration_micros = Some(duration_micros);
    }

    /// Performance metrics: get render count
    pub fn render_count(&self) -> u64 {
        self.render_count
    }

    /// Performance metrics: get last render duration
    pub fn last_render_duration_micros(&self) -> Option<u64> {
        self.last_render_duration_micros
    }

    /// Performance metrics: get last syntax duration
    pub fn last_syntax_duration_micros(&self) -> Option<u64> {
        self.last_syntax_duration_micros
    }

    /// Performance metrics: record input latency sample
    pub fn record_input_latency(&mut self, latency_micros: u64) {
        self.input_latency_samples.push(latency_micros);
        // Keep only the most recent MAX_LATENCY_SAMPLES samples (circular buffer)
        if self.input_latency_samples.len() > MAX_LATENCY_SAMPLES {
            self.input_latency_samples.remove(0);
        }
    }

    /// Performance metrics: compute latency percentile
    fn compute_percentile(samples: &[u64], percentile: f64) -> Option<u64> {
        if samples.is_empty() {
            return None;
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let index = ((percentile / 100.0) * (sorted.len() as f64 - 1.0)) as usize;
        Some(sorted[index])
    }

    /// Performance metrics: get input latency p50
    pub fn input_latency_p50_micros(&self) -> Option<u64> {
        Self::compute_percentile(&self.input_latency_samples, 50.0)
    }

    /// Performance metrics: get input latency p95
    pub fn input_latency_p95_micros(&self) -> Option<u64> {
        Self::compute_percentile(&self.input_latency_samples, 95.0)
    }

    /// Performance metrics: get input latency p99
    pub fn input_latency_p99_micros(&self) -> Option<u64> {
        Self::compute_percentile(&self.input_latency_samples, 99.0)
    }

    /// Performance metrics: get number of input latency samples
    pub fn input_latency_sample_count(&self) -> usize {
        self.input_latency_samples.len()
    }

    /// Performance metrics: record LSP serialize duration
    pub fn record_lsp_serialize_duration(&mut self, duration_micros: u64) {
        self.last_lsp_serialize_micros = Some(duration_micros);
    }

    /// Performance metrics: get last LSP serialize duration
    pub fn last_lsp_serialize_micros(&self) -> Option<u64> {
        self.last_lsp_serialize_micros
    }

    /// Performance metrics: record git status duration
    pub fn record_git_status_duration(&mut self, duration_micros: u64) {
        self.last_git_status_micros = Some(duration_micros);
    }

    /// Performance metrics: get last git status duration
    pub fn last_git_status_micros(&self) -> Option<u64> {
        self.last_git_status_micros
    }

    /// Performance metrics: record fold calculation duration
    pub fn record_fold_calc_duration(&mut self, duration_micros: u64) {
        self.last_fold_calc_micros = Some(duration_micros);
    }

    /// Performance metrics: get last fold calculation duration
    pub fn last_fold_calc_micros(&self) -> Option<u64> {
        self.last_fold_calc_micros
    }

    /// Performance metrics: record diagnostic query duration
    pub fn record_diagnostic_query_duration(&mut self, duration_micros: u64) {
        self.last_diagnostic_query_micros = Some(duration_micros);
    }

    /// Performance metrics: get last diagnostic query duration
    pub fn last_diagnostic_query_micros(&self) -> Option<u64> {
        self.last_diagnostic_query_micros
    }

    /// Marks the editor as needing a redraw
    pub fn mark_dirty(&mut self) {
        self.render_dirty = true;
    }

    /// Checks if the editor needs a redraw
    pub fn is_dirty(&self) -> bool {
        self.render_dirty
    }

    /// Marks the editor as clean (just rendered)
    pub fn mark_clean(&mut self) {
        self.render_dirty = false;
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
