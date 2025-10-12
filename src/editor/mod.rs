mod change;
mod completion;
mod filetree;
mod fold;
mod input;
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
pub use macros::MacroManager;
pub use marks::{JumpList, Mark, MarkManager};
pub use motions::Motions;
pub use operators::{Operator, Operators};
pub use picker::{Picker, PickerMode, PickerResult};
pub use quickfix::{LocationList, QuickfixEntry, QuickfixEntryType, QuickfixList};
pub use register::RegisterManager;
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
use crate::syntax::{ColorScheme, ColorSchemeRegistry};
#[cfg(feature = "lua")]
use crate::lua::LuaContext;
use crate::mode::Mode;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

/// Type of LSP result currently being displayed in picker
#[derive(Debug, Clone, PartialEq)]
enum LspResultType {
    References,
    DocumentSymbols,
    WorkspaceSymbols,
    CallHierarchy,
    TypeHierarchy,
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
    /// Command line buffer (for : commands)
    command_line: String,
    /// Search buffer (for / and ? commands)
    search_buffer: String,
    /// Search direction: true for forward (/), false for backward (?)
    search_forward: bool,
    /// Current search state
    current_search: Option<Search>,
    /// Change manager for undo/redo and repeat
    change_manager: ChangeManager,
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
    /// LSP manager (optional, only if LSP is enabled)
    lsp_manager: Option<Arc<TokioMutex<LspManager>>>,
    /// Cached diagnostic count (errors, warnings, info, hints) for status line display
    diagnostic_count: (usize, usize, usize, usize),
    /// Pending LSP action to execute in async context
    pending_lsp_action: Option<LspAction>,
    /// Hover information to display (from LSP)
    hover_info: Option<String>,
    /// Scroll offset for hover window (line number)
    hover_scroll: usize,
    /// Flag to track if buffer was modified this iteration (for LSP didChange)
    buffer_modified_this_iteration: bool,
    /// Flag to track if buffer was saved this iteration (for LSP didSave)
    buffer_saved_this_iteration: bool,
    /// Last synced buffer content for incremental LSP sync (None = use full sync)
    last_synced_content: Option<String>,
    /// LSP status message (errors, warnings, or info)
    lsp_status: String,
    /// Active LSP servers (language_id -> server_name)
    active_lsp_servers: HashMap<String, String>,
    /// Flag to indicate LSP needs initialization for current file
    needs_lsp_init: bool,
    /// File path that needs didClose notification (set when switching files)
    pending_did_close_file: Option<String>,
    /// Lua context for configuration and plugins (optional)
    #[cfg(feature = "lua")]
    lua_context: Option<LuaContext>,
    /// Bridge for Lua-Editor communication (optional)
    #[cfg(feature = "lua")]
    editor_bridge: Option<crate::lua::EditorBridge>,
    /// Last insert position (line, col) for gi command
    last_insert_position: Option<(usize, usize)>,
    /// Available code actions at current cursor position
    available_code_actions: Vec<lsp_types::CodeActionOrCommand>,
    /// Available completion items at current cursor position
    available_completions: Vec<lsp_types::CompletionItem>,
    /// Completion menu popup
    completion_menu: CompletionMenu,
    /// Available LSP references at current cursor position
    available_references: Vec<lsp_types::Location>,
    /// Available document symbols for current file
    available_document_symbols: Vec<lsp_types::DocumentSymbol>,
    /// Available workspace symbols
    available_workspace_symbols: Vec<lsp_types::SymbolInformation>,
    /// Available call hierarchy items (incoming or outgoing)
    available_call_hierarchy: Vec<(String, lsp_types::Location)>,
    /// Available type hierarchy items (supertypes and subtypes)
    available_type_hierarchy: Vec<(String, lsp_types::Location)>,
    /// Inlay hints for the visible region
    inlay_hints: Vec<lsp_types::InlayHint>,
    /// Currently active LSP result type (for picker navigation)
    active_lsp_result_type: Option<LspResultType>,
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
    /// Currently loading preview path (to avoid duplicate requests)
    loading_preview: Option<String>,
    /// Last successfully shown preview path (to show while new one loads)
    last_shown_preview: Option<String>,
}

/// Cached preview data for the picker
#[derive(Clone)]
pub struct PreviewCache {
    /// File content
    pub content: String,
    /// Cached syntax-highlighted lines (line_idx -> highlights)
    /// Uses RefCell for interior mutability so we can cache highlights even with immutable reference
    pub highlighted_lines: std::cell::RefCell<HashMap<usize, Vec<(std::ops::Range<usize>, crate::syntax::HighlightGroup)>>>,
    /// Detected language (if any)
    pub language: Option<crate::syntax::Language>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspAction {
    GoToDefinition,
    GoToImplementation,
    GoToType,
    ShowHover,
    Completion,
    FormatDocument,
    CodeActions,
    TypeHierarchy,
    CallHierarchyIncoming,
    CallHierarchyOutgoing,
    FindReferences,
    DocumentSymbols,
    WorkspaceSymbols,
    OrganizeImports,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FindType {
    Find,  // f/F - cursor lands on character
    Till,  // t/T - cursor lands before/after character
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
            command_line: String::new(),
            search_buffer: String::new(),
            search_forward: true,
            current_search: None,
            change_manager: ChangeManager::new(),
            marks: MarkManager::new(),
            jump_list: JumpList::new(),
            macro_manager: MacroManager::new(),
            last_find: None,
            picker: None,
            leader_key: ' ',
            pending_leader: false,
            lsp_manager: None,
            diagnostic_count: (0, 0, 0, 0),
            pending_lsp_action: None,
            hover_info: None,
            hover_scroll: 0,
            buffer_modified_this_iteration: false,
            buffer_saved_this_iteration: false,
            last_synced_content: None,
            lsp_status: String::new(),
            active_lsp_servers: HashMap::new(),
            needs_lsp_init: false,
            pending_did_close_file: None,
            #[cfg(feature = "lua")]
            lua_context: None,
            #[cfg(feature = "lua")]
            editor_bridge: None,
            last_insert_position: None,
            available_code_actions: Vec::new(),
            available_completions: Vec::new(),
            completion_menu: CompletionMenu::new(),
            available_references: Vec::new(),
            available_document_symbols: Vec::new(),
            available_workspace_symbols: Vec::new(),
            available_call_hierarchy: Vec::new(),
            available_type_hierarchy: Vec::new(),
            inlay_hints: Vec::new(),
            active_lsp_result_type: None,
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
            loading_preview: None,
            last_shown_preview: None,
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
            command_line: String::new(),
            search_buffer: String::new(),
            search_forward: true,
            current_search: None,
            change_manager: ChangeManager::new(),
            marks: MarkManager::new(),
            jump_list: JumpList::new(),
            macro_manager: MacroManager::new(),
            last_find: None,
            picker: None,
            leader_key: ' ',
            pending_leader: false,
            lsp_manager: None,
            diagnostic_count: (0, 0, 0, 0),
            pending_lsp_action: None,
            hover_info: None,
            hover_scroll: 0,
            buffer_modified_this_iteration: false,
            buffer_saved_this_iteration: false,
            last_synced_content: None,
            lsp_status: String::new(),
            active_lsp_servers: HashMap::new(),
            needs_lsp_init: false,
            pending_did_close_file: None,
            #[cfg(feature = "lua")]
            lua_context: None,
            #[cfg(feature = "lua")]
            editor_bridge: None,
            last_insert_position: None,
            available_code_actions: Vec::new(),
            available_completions: Vec::new(),
            completion_menu: CompletionMenu::new(),
            available_references: Vec::new(),
            available_document_symbols: Vec::new(),
            available_workspace_symbols: Vec::new(),
            available_call_hierarchy: Vec::new(),
            available_type_hierarchy: Vec::new(),
            inlay_hints: Vec::new(),
            active_lsp_result_type: None,
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
            loading_preview: None,
            last_shown_preview: None,
        }
    }

    /// Enables LSP support
    pub fn enable_lsp(&mut self) {
        self.lsp_manager = Some(Arc::new(TokioMutex::new(LspManager::new())));
    }

    /// Gets a reference to the LSP manager
    pub fn lsp_manager(&self) -> Option<Arc<TokioMutex<LspManager>>> {
        self.lsp_manager.clone()
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
                if cursor_col > 0 { cursor_col - 1 } else { 0 }
            };

            if let Some((line, col, _)) = search_clone.find_next(self.buffer(), cursor_line, search_col) {
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
                if cursor.col() > 0 { cursor.col() - 1 } else { 0 }
            } else {
                // Original was backward, now forward
                cursor.col() + 1
            };

            if let Some((line, col, _)) = rev_search.find_next(self.buffer(), cursor.line(), search_col) {
                self.buffer_mut().cursor_mut().set_position(line, col);
            }
        }
    }

    /// Sets a mark at the current cursor position
    pub fn set_mark(&mut self, name: char) -> bool {
        let cursor = self.buffer().cursor();
        self.marks.set_mark(name, cursor.line(), cursor.col())
    }

    /// Jumps to a mark (exact position with backtick)
    pub fn jump_to_mark(&mut self, name: char) -> bool {
        if let Some(mark) = self.marks.get_mark(name) {
            self.buffer_mut().cursor_mut().set_position(mark.line, mark.col);
            true
        } else {
            false
        }
    }

    /// Jumps to mark line (apostrophe - goes to first non-blank on line)
    pub fn jump_to_mark_line(&mut self, name: char) -> bool {
        if let Some(mark) = self.marks.get_mark(name) {
            self.buffer_mut().cursor_mut().set_position(mark.line, 0);
            // TODO: Move to first non-blank character
            true
        } else {
            false
        }
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

    /// Switches to the next buffer
    pub fn next_buffer(&mut self) {
        if self.buffers.len() > 1 {
            self.current_buffer_index = (self.current_buffer_index + 1) % self.buffers.len();
            self.needs_lsp_init = true;
        }
    }

    /// Switches to the previous buffer
    pub fn prev_buffer(&mut self) {
        if self.buffers.len() > 1 {
            self.current_buffer_index = if self.current_buffer_index == 0 {
                self.buffers.len() - 1
            } else {
                self.current_buffer_index - 1
            };
            self.needs_lsp_init = true;
        }
    }

    /// Deletes the current buffer and switches to another if available
    /// Returns true if the editor should quit (no more buffers)
    pub fn delete_current_buffer(&mut self) -> bool {
        if self.buffers.len() == 1 {
            // Last buffer - quit the editor
            return true;
        }

        // Remove current buffer
        self.buffers.remove(self.current_buffer_index);

        // Adjust index if we were at the end
        if self.current_buffer_index >= self.buffers.len() {
            self.current_buffer_index = self.buffers.len() - 1;
        }

        self.needs_lsp_init = true;
        false
    }

    /// Adds a new buffer and switches to it
    pub fn add_buffer(&mut self, buffer: Buffer) {
        self.buffers.push(buffer);
        self.current_buffer_index = self.buffers.len() - 1;
        self.change_manager = ChangeManager::new();
        self.needs_lsp_init = true;
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
        self.options.scroll.unwrap_or_else(|| {
            (self.viewport_height / 2).max(1)
        })
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
        let Some(ref lsp) = self.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return;
        };

        let Ok(uri) = lsp_types::Url::from_file_path(file_path) else {
            return;
        };

        // Detect language from file extension
        let language_id = if file_path.ends_with(".rs") {
            "rust"
        } else if file_path.ends_with(".js") || file_path.ends_with(".ts") {
            "javascript"
        } else if file_path.ends_with(".py") {
            "python"
        } else if file_path.ends_with(".java") {
            "java"
        } else {
            return;
        };

        // Try to get lock without blocking
        let Ok(lsp_guard) = lsp.try_lock() else {
            return;
        };

        let _ = lsp_guard.did_close(uri, language_id).await;
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

    /// Yanks text to the appropriate register (pending_register or default)
    pub fn yank_to_register(&mut self, text: String) {
        if let Some(reg) = self.pending_register {
            self.registers.set(Some(reg), text);
            self.pending_register = None;
        } else {
            self.registers.yank(text);
        }
    }

    /// Deletes text and stores in the appropriate register (pending_register or default)
    pub fn delete_to_register(&mut self, text: String) {
        if let Some(reg) = self.pending_register {
            self.registers.set(Some(reg), text);
            self.pending_register = None;
        } else {
            self.registers.delete(text);
        }
    }

    /// Gets text from the appropriate register (pending_register or default)
    pub fn get_from_register(&mut self) -> String {
        let text = if let Some(reg) = self.pending_register {
            self.registers.get(Some(reg)).to_string()
        } else {
            self.registers.get_default().to_string()
        };
        self.pending_register = None;
        text
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
                self.current_buffer_index = i;
                return Ok(());
            }
        }

        // Store old file path before loading new file
        let old_file_path = self.buffer().file_path().map(|s| s.to_string());

        // Load new buffer
        let new_buffer = Buffer::load_file_async(path).await?;
        self.add_buffer(new_buffer);

        // Mark that we need to send didClose for the old file
        if old_file_path.is_some() {
            self.pending_did_close_file = old_file_path;
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
        if self.needs_lsp_init {
            self.buffer().file_path().map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Clears the LSP init flag (should be called after initializing LSP)
    pub fn clear_lsp_init_flag(&mut self) {
        self.needs_lsp_init = false;
    }

    /// Requests LSP initialization for the current buffer
    pub fn request_lsp_init(&mut self) {
        self.needs_lsp_init = true;
    }

    /// Starts building a composite change (e.g., when entering insert mode)
    pub fn start_change_building(&mut self, cursor_before: Position) {
        self.change_manager.start_building(cursor_before);
    }

    /// Adds a change to the change manager
    pub fn add_change(&mut self, change: Change) {
        self.change_manager.add_change(change);
        self.mark_buffer_modified(); // Mark for LSP didChange notification
    }

    /// Finalizes the current composite change
    pub fn finalize_change_building(&mut self) {
        self.change_manager.finalize_building();
    }

    /// Undoes the last change
    pub fn undo(&mut self) {
        let buffer = &mut self.buffers[self.current_buffer_index];
        self.change_manager.undo(buffer);
    }

    /// Redoes the next change
    pub fn redo(&mut self) {
        let buffer = &mut self.buffers[self.current_buffer_index];
        self.change_manager.redo(buffer);
    }

    /// Repeats the last change
    pub fn repeat_last_change(&mut self) {
        let buffer = &mut self.buffers[self.current_buffer_index];
        self.change_manager.repeat_last(buffer);
    }

    /// Checks if buffer is modified relative to last save
    pub fn is_modified(&self) -> bool {
        !self.change_manager.is_at_save_point()
    }

    /// Marks current state as saved
    pub fn mark_saved(&mut self) {
        self.change_manager.mark_saved();
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

    /// Checks if enough time has elapsed since picker query changed (for debouncing)
    /// Returns true if we should load preview now
    pub fn should_load_picker_preview(&self, debounce_ms: u64) -> bool {
        match self.last_picker_query_change {
            None => true, // No previous change, load immediately
            Some(last_change) => {
                let elapsed = last_change.elapsed();
                elapsed.as_millis() >= debounce_ms as u128
            }
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
                    && *picker.mode() != crate::editor::PickerMode::LspLocations {
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
    pub fn get_preview_with_fallback(&mut self, file_path: &str) -> Option<(&PreviewCache, String)> {
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
            let keys_to_remove: Vec<String> = self.preview_cache
                .keys()
                .take(to_remove)
                .cloned()
                .collect();
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
        let lsp = self.lsp_manager.as_ref()?;
        let file_path = self.buffer().file_path()?;
        let uri = lsp_types::Url::from_file_path(file_path).ok()?;

        // Try to get lock without blocking - return None if busy
        let lsp_guard = lsp.try_lock().ok()?;
        Some(lsp_guard.get_diagnostics(&uri).await)
    }

    /// Gets diagnostic count for the current file (errors, warnings, info, hints)
    pub async fn get_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        if let Some(lsp) = &self.lsp_manager {
            if let Some(file_path) = self.buffer().file_path() {
                if let Ok(uri) = lsp_types::Url::from_file_path(file_path) {
                    // Try to get lock without blocking - return (0,0,0,0) if busy
                    if let Ok(lsp_guard) = lsp.try_lock() {
                        return lsp_guard.count_diagnostics(&uri).await;
                    }
                }
            }
        }
        (0, 0, 0, 0)
    }

    /// Updates the cached diagnostic count (should be called when diagnostics change)
    pub async fn update_diagnostic_cache(&mut self) {
        self.diagnostic_count = self.get_diagnostic_count().await;
    }

    /// Gets the cached diagnostic count (sync, suitable for UI rendering)
    pub fn cached_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        self.diagnostic_count
    }

    /// Sets the LSP status message
    pub fn set_lsp_status(&mut self, status: String) {
        self.lsp_status = status;
    }

    /// Gets the LSP status message
    pub fn lsp_status(&self) -> &str {
        &self.lsp_status
    }

    /// Registers an active LSP server
    pub fn register_lsp_server(&mut self, language_id: String, server_name: String) {
        self.lsp_status = format!("LSP: {} ready", server_name);
        self.active_lsp_servers.insert(language_id, server_name);
    }

    /// Unregisters an LSP server
    pub fn unregister_lsp_server(&mut self, language_id: &str) {
        self.active_lsp_servers.remove(language_id);
        if self.active_lsp_servers.is_empty() {
            self.lsp_status.clear();
        }
    }

    /// Gets active LSP servers
    pub fn active_lsp_servers(&self) -> &HashMap<String, String> {
        &self.active_lsp_servers
    }

    /// Gets current LSP progress message
    pub fn lsp_progress_message(&self) -> Option<String> {
        if let Some(lsp_manager) = &self.lsp_manager {
            if let Ok(lsp) = lsp_manager.try_lock() {
                return lsp.get_progress_message();
            }
        }
        None
    }

    /// Gets comprehensive LSP information for debugging
    pub fn get_lsp_info(&self) -> String {
        let mut info = String::new();

        // LSP Manager status
        if self.lsp_manager.is_some() {
            info.push_str("LSP Manager: Active\n");
        } else {
            info.push_str("LSP Manager: Not initialized\n");
            return info;
        }

        // Active servers
        if self.active_lsp_servers.is_empty() {
            info.push_str("\nActive Servers: None\n");
        } else {
            info.push_str("\nActive Servers:\n");
            for (lang_id, server_name) in &self.active_lsp_servers {
                info.push_str(&format!("  {} -> {}\n", lang_id, server_name));
            }
        }

        // Current file
        if let Some(path) = self.buffer().file_path() {
            info.push_str(&format!("\nCurrent File: {}\n", path));
        }

        // Diagnostic counts
        let (errors, warnings, infos, hints) = self.diagnostic_count;
        info.push_str(&format!("\nDiagnostics:\n"));
        info.push_str(&format!("  Errors: {}\n", errors));
        info.push_str(&format!("  Warnings: {}\n", warnings));
        info.push_str(&format!("  Info: {}\n", infos));
        info.push_str(&format!("  Hints: {}\n", hints));

        // Current status
        if !self.lsp_status.is_empty() {
            info.push_str(&format!("\nStatus: {}\n", self.lsp_status));
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

            // Build highlights for all lines
            let lines: Vec<&str> = content.lines().collect();
            let mut all_highlights = Vec::with_capacity(lines.len());

            for line_idx in 0..lines.len() {
                let line_highlights = highlighter.highlights_for_line(line_idx, &content);
                all_highlights.push(line_highlights);
            }

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
        self.pending_lsp_action = Some(LspAction::GoToDefinition);
    }

    /// Request go to implementation (sets pending action)
    pub fn request_goto_implementation(&mut self) {
        self.pending_lsp_action = Some(LspAction::GoToImplementation);
    }

    /// Request go to type definition (sets pending action)
    pub fn request_goto_type(&mut self) {
        self.pending_lsp_action = Some(LspAction::GoToType);
    }

    /// Requests hover information for current cursor position
    pub fn request_hover(&mut self) {
        self.pending_lsp_action = Some(LspAction::ShowHover);
    }

    /// Requests code completion for current cursor position
    pub fn request_completion(&mut self) {
        self.pending_lsp_action = Some(LspAction::Completion);
    }

    /// Requests document formatting
    pub fn request_format_document(&mut self) {
        self.pending_lsp_action = Some(LspAction::FormatDocument);
    }

    /// Requests code actions for current cursor position
    pub fn request_code_actions(&mut self) {
        self.pending_lsp_action = Some(LspAction::CodeActions);
    }

    /// Requests call hierarchy incoming calls (who calls this method)
    pub fn request_call_hierarchy_incoming(&mut self) {
        self.pending_lsp_action = Some(LspAction::CallHierarchyIncoming);
    }

    /// Requests call hierarchy outgoing calls (what this method calls)
    pub fn request_call_hierarchy_outgoing(&mut self) {
        self.pending_lsp_action = Some(LspAction::CallHierarchyOutgoing);
    }

    /// Requests type hierarchy (superclasses/interfaces and subclasses/implementations)
    pub fn request_type_hierarchy(&mut self) {
        self.pending_lsp_action = Some(LspAction::TypeHierarchy);
    }

    /// Requests organize imports command for Java
    pub fn request_organize_imports(&mut self) {
        self.pending_lsp_action = Some(LspAction::OrganizeImports);
    }

    /// Requests find all references to symbol at cursor
    pub fn request_find_references(&mut self) {
        self.pending_lsp_action = Some(LspAction::FindReferences);
    }

    /// Requests document symbols (outline)
    pub fn request_document_symbols(&mut self) {
        self.pending_lsp_action = Some(LspAction::DocumentSymbols);
    }

    /// Requests workspace-wide symbol search
    pub fn request_workspace_symbols(&mut self) {
        self.pending_lsp_action = Some(LspAction::WorkspaceSymbols);
    }

    /// Gets the current hover information (if any)
    pub fn hover_info(&self) -> Option<&str> {
        self.hover_info.as_deref()
    }

    /// Clears the hover information
    pub fn clear_hover(&mut self) {
        self.hover_info = None;
        self.hover_scroll = 0;
    }

    /// Gets the hover scroll offset
    pub fn hover_scroll(&self) -> usize {
        self.hover_scroll
    }

    /// Scrolls the hover window down
    pub fn scroll_hover_down(&mut self, lines: usize) {
        if self.hover_info.is_some() {
            self.hover_scroll = self.hover_scroll.saturating_add(lines);
        }
    }

    /// Scrolls the hover window up
    pub fn scroll_hover_up(&mut self, lines: usize) {
        self.hover_scroll = self.hover_scroll.saturating_sub(lines);
    }

    /// Marks that the buffer was modified (for LSP notification)
    pub fn mark_buffer_modified(&mut self) {
        self.buffer_modified_this_iteration = true;
    }

    /// Marks that the buffer was saved (for LSP notification)
    pub fn mark_buffer_saved(&mut self) {
        self.buffer_saved_this_iteration = true;
    }

    /// Sets the last synced content for incremental LSP sync
    pub fn set_last_synced_content(&mut self, content: Option<String>) {
        self.last_synced_content = content;
    }

    /// Sends didChange notification if buffer was modified, then resets the flag
    pub async fn send_lsp_changes_if_modified(&mut self) {
        if !self.buffer_modified_this_iteration {
            return;
        }

        self.buffer_modified_this_iteration = false;

        let Some(ref lsp) = self.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return;
        };

        let Ok(uri) = lsp_types::Url::from_file_path(file_path) else {
            return;
        };

        // Detect language from file extension
        let language_id = if file_path.ends_with(".rs") {
            "rust"
        } else if file_path.ends_with(".js") || file_path.ends_with(".ts") {
            "javascript"
        } else if file_path.ends_with(".py") {
            "python"
        } else {
            return;
        };

        // Send document sync with debouncing (incremental if supported)
        let content = self.buffer().rope().to_string();

        // Pass last_synced_content for incremental sync
        let old_content = self.last_synced_content.clone();

        // Try to get lock without blocking - if LSP is busy, skip this iteration
        let Ok(lsp_guard) = lsp.try_lock() else {
            return; // LSP busy, will sync on next change
        };

        let _ = lsp_guard.did_change(uri, language_id, content.clone(), old_content).await;
        drop(lsp_guard);

        // Update last_synced_content after successful sync
        self.last_synced_content = Some(content);
    }

    /// Sends didSave notification if buffer was saved, then resets the flag
    pub async fn send_lsp_save_if_needed(&mut self) {
        if !self.buffer_saved_this_iteration {
            return;
        }

        self.buffer_saved_this_iteration = false;

        let Some(ref lsp) = self.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer().file_path() else {
            return;
        };

        let Ok(uri) = lsp_types::Url::from_file_path(file_path) else {
            return;
        };

        // Detect language from file extension
        let language_id = if file_path.ends_with(".rs") {
            "rust"
        } else if file_path.ends_with(".js") || file_path.ends_with(".ts") {
            "javascript"
        } else if file_path.ends_with(".py") {
            "python"
        } else {
            return;
        };

        let text = Some(self.buffer().rope().to_string());

        // Try to get lock without blocking - if LSP is busy, skip
        let Ok(lsp_guard) = lsp.try_lock() else {
            return; // LSP busy, save notification will be sent on next save
        };

        let _ = lsp_guard.did_save(uri, language_id, text).await;
    }

    /// Sends didClose notification for pending file (when switching files)
    pub async fn send_lsp_close_if_needed(&mut self) {
        let Some(file_path) = self.pending_did_close_file.take() else {
            return;
        };

        let Some(ref lsp) = self.lsp_manager else {
            return;
        };

        let Ok(uri) = lsp_types::Url::from_file_path(&file_path) else {
            return;
        };

        // Detect language from file extension
        let language_id = if file_path.ends_with(".rs") {
            "rust"
        } else if file_path.ends_with(".js") || file_path.ends_with(".ts") {
            "javascript"
        } else if file_path.ends_with(".py") {
            "python"
        } else if file_path.ends_with(".java") {
            "java"
        } else {
            return;
        };

        // Try to get lock without blocking - if LSP is busy, put it back for retry
        let Ok(lsp_guard) = lsp.try_lock() else {
            self.pending_did_close_file = Some(file_path);
            return;
        };

        let _ = lsp_guard.did_close(uri, language_id).await;
    }

    /// Process any pending LSP actions
    pub async fn process_pending_lsp_actions(&mut self) {
        if let Some(action) = self.pending_lsp_action.take() {
            let result = match action {
                LspAction::GoToDefinition => self.goto_definition_impl().await,
                LspAction::GoToImplementation => self.goto_implementation_impl().await,
                LspAction::GoToType => self.goto_type_impl().await,
                LspAction::ShowHover => self.hover_impl().await,
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
            };

            // Handle errors: update status and optionally retry
            match result {
                Ok(_) => {
                    // Success - status was already updated by the impl function
                }
                Err(e) => {
                    // Check if error message indicates we should retry (e.g., "LSP busy")
                    let error_msg = e.to_string();
                    let should_retry = error_msg.contains("LSP busy") || error_msg.contains("couldn't get lock");

                    if should_retry {
                        // Retry silently - put action back
                        self.pending_lsp_action = Some(action);
                    } else {
                        // Permanent error - update status to show the error
                        let action_name = match action {
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
        let char_count = line_text.chars().count();

        // Clamp col to valid range
        let safe_col = col.min(char_count);

        // Convert to UTF-16 code units
        line_text.chars()
            .take(safe_col)
            .map(|c| c.len_utf16() as u32)
            .sum()
    }

    /// Go to definition at current cursor position via LSP (implementation)
    async fn goto_definition_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_manager {
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

        // Request definition
        self.set_lsp_status("Searching for definition...".to_string());

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                // LSP manager is busy (e.g., Java initialization), will retry next iteration
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let location = lsp_guard
            .goto_definition(&uri, line, character, language_id)
            .await?;

        drop(lsp_guard);

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
                self.buffer_mut().cursor_mut().set_position(target_line, target_col);
                self.set_lsp_status(format!("Definition found at line {}", target_line + 1));
                return Ok(true);
            } else {
                // Different file - open it and jump
                match location.uri.to_file_path() {
                    Ok(target_path) => {
                        // Try to open the target file
                        match self.load_file_async(&target_path).await {
                            Ok(_) => {
                                self.buffer_mut().cursor_mut().set_position(target_line, target_col);
                                let file_name = target_path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("file");
                                self.set_lsp_status(format!("Opened {} at line {}", file_name, target_line + 1));
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
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                // LSP manager is busy (e.g., Java initialization), will retry next iteration
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let location = lsp_guard
            .implementation(&uri, line, character, language_id)
            .await?;

        drop(lsp_guard);

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
                self.buffer_mut().cursor_mut().set_position(target_line, target_col);
                self.set_lsp_status(format!("Implementation found at line {}", target_line + 1));
                return Ok(true);
            } else {
                // Different file - open it and jump
                match location.uri.to_file_path() {
                    Ok(target_path) => {
                        // Try to open the target file
                        match self.load_file_async(&target_path).await {
                            Ok(_) => {
                                self.buffer_mut().cursor_mut().set_position(target_line, target_col);
                                let file_name = target_path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("file");
                                self.set_lsp_status(format!("Opened {} at line {}", file_name, target_line + 1));
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
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                // LSP manager is busy (e.g., Java initialization), will retry next iteration
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let location = lsp_guard
            .type_definition(&uri, line, character, language_id)
            .await?;

        drop(lsp_guard);

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
                self.buffer_mut().cursor_mut().set_position(target_line, target_col);
                self.set_lsp_status(format!("Type definition found at line {}", target_line + 1));
                return Ok(true);
            } else {
                // Different file - open it and jump
                match location.uri.to_file_path() {
                    Ok(target_path) => {
                        // Try to open the target file
                        match self.load_file_async(&target_path).await {
                            Ok(_) => {
                                self.buffer_mut().cursor_mut().set_position(target_line, target_col);
                                let file_name = target_path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("file");
                                self.set_lsp_status(format!("Opened {} at line {}", file_name, target_line + 1));
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
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let locations = lsp_guard
            .references(&uri, line, character, language_id, true)
            .await?;

        drop(lsp_guard);

        // Display results in picker
        if locations.is_empty() {
            self.set_lsp_status("No references found".to_string());
            return Ok(false);
        }

        // Store locations in storage vector
        self.available_references = locations.clone();
        self.active_lsp_result_type = Some(LspResultType::References);

        // Format locations as picker items
        let items: Vec<String> = locations
            .iter()
            .map(|loc| {
                let file_path = loc.uri.to_file_path().ok()
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
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let symbols = lsp_guard
            .document_symbols(&uri, language_id)
            .await?;

        drop(lsp_guard);

        // Display results in picker
        if symbols.is_empty() {
            self.set_lsp_status("No symbols found".to_string());
            return Ok(false);
        }

        // Store symbols in storage vector
        self.available_document_symbols = symbols.clone();
        self.active_lsp_result_type = Some(LspResultType::DocumentSymbols);

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
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let symbols = lsp_guard
            .workspace_symbols(language_id, String::new())
            .await?;

        drop(lsp_guard);

        // Display results in picker
        if symbols.is_empty() {
            self.set_lsp_status("No workspace symbols found".to_string());
            return Ok(false);
        }

        // Store symbols in storage vector
        self.available_workspace_symbols = symbols.clone();
        self.active_lsp_result_type = Some(LspResultType::WorkspaceSymbols);

        // Format symbols as picker items
        let items: Vec<String> = symbols
            .iter()
            .map(|sym| {
                let file_name = sym.location.uri.to_file_path().ok()
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
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let items = lsp_guard
            .prepare_call_hierarchy(uri, line, character, language_id)
            .await?;

        let items = match items {
            Some(items) if !items.is_empty() => items,
            _ => {
                drop(lsp_guard);
                self.set_lsp_status("No call hierarchy item at cursor".to_string());
                return Ok(false);
            }
        };

        // Get incoming calls for the first item
        let first_item = items.into_iter().next().unwrap();
        let incoming = lsp_guard
            .incoming_calls(first_item, language_id)
            .await?;

        drop(lsp_guard);

        // Display results in picker
        let calls = match incoming {
            Some(calls) if !calls.is_empty() => calls,
            _ => {
                self.set_lsp_status("No incoming calls found".to_string());
                return Ok(false);
            }
        };

        // Store call hierarchy data in storage vector
        self.available_call_hierarchy = calls
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
        self.active_lsp_result_type = Some(LspResultType::CallHierarchy);

        // Format calls as picker items
        let items: Vec<String> = calls
            .iter()
            .map(|call| {
                let name = &call.from.name;
                let file_path = call.from.uri.to_file_path().ok()
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
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let items = lsp_guard
            .prepare_call_hierarchy(uri, line, character, language_id)
            .await?;

        let items = match items {
            Some(items) if !items.is_empty() => items,
            _ => {
                drop(lsp_guard);
                self.set_lsp_status("No call hierarchy item at cursor".to_string());
                return Ok(false);
            }
        };

        // Get outgoing calls for the first item
        let first_item = items.into_iter().next().unwrap();
        let outgoing = lsp_guard
            .outgoing_calls(first_item, language_id)
            .await?;

        drop(lsp_guard);

        // Display results in picker
        let calls = match outgoing {
            Some(calls) if !calls.is_empty() => calls,
            _ => {
                self.set_lsp_status("No outgoing calls found".to_string());
                return Ok(false);
            }
        };

        // Store call hierarchy data in storage vector
        self.available_call_hierarchy = calls
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
        self.active_lsp_result_type = Some(LspResultType::CallHierarchy);

        // Format calls as picker items
        let items: Vec<String> = calls
            .iter()
            .map(|call| {
                let name = &call.to.name;
                let file_path = call.to.uri.to_file_path().ok()
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
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let items = lsp_guard
            .prepare_type_hierarchy(uri, line, character, language_id)
            .await?;

        let items = match items {
            Some(items) if !items.is_empty() => items,
            _ => {
                drop(lsp_guard);
                self.set_lsp_status("No type hierarchy item at cursor".to_string());
                return Ok(false);
            }
        };

        // Get supertypes and subtypes for the first item
        let first_item = items.into_iter().next().unwrap();
        let supertypes = lsp_guard
            .supertypes(first_item.clone(), language_id)
            .await?;
        let subtypes = lsp_guard
            .subtypes(first_item, language_id)
            .await?;

        drop(lsp_guard);

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
                let file_name = super_type.uri.to_file_path().ok()
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
                let file_name = sub_type.uri.to_file_path().ok()
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
        self.available_type_hierarchy = all_types_data;
        self.active_lsp_result_type = Some(LspResultType::TypeHierarchy);

        self.set_lsp_status(format!("Found {} types in hierarchy", all_types_display.len()));

        // Create picker with results
        let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let picker = crate::editor::Picker::new_lsp_locations(base_dir, all_types_display);
        self.set_picker(picker);
        self.set_mode(crate::mode::Mode::Picker);

        Ok(true)
    }

    /// Gets hover information at current cursor position via LSP (implementation)
    async fn hover_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
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

        crate::lsp_debug!("HOVER-DEBUG", "Cursor: line={}, col={} | UTF-16 char={}", cursor.line(), cursor.col(), character);

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Request hover information
        self.set_lsp_status("Requesting hover information...".to_string());

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                // LSP manager is busy (e.g., Java initialization), will retry next iteration
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let hover_text = lsp_guard
            .hover(&uri, line, character, language_id)
            .await?;

        drop(lsp_guard);

        // Store hover information and enter HoverWindow mode if available
        self.hover_info = hover_text;
        self.hover_scroll = 0; // Reset scroll position

        if self.hover_info.is_some() {
            self.set_mode(Mode::HoverWindow);
            self.set_lsp_status("Hover window opened (q to close, j/k to scroll)".to_string());
            Ok(true)
        } else {
            // Provide helpful status message - LSP might still be indexing
            self.set_lsp_status("No hover info (try again, LSP may be indexing...)".to_string());
            Ok(false)
        }
    }

    /// Requests code completion at current cursor position via LSP (implementation)
    async fn completion_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                // LSP manager is busy (e.g., Java initialization), will retry next iteration
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let items = lsp_guard
            .completion(&uri, line, character, language_id)
            .await?;

        drop(lsp_guard);

        if items.is_empty() {
            self.set_lsp_status("No completion items found".to_string());
            return Ok(false);
        }

        // Store completion items
        self.available_completions = items.clone();

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
        self.completion_menu.show(items, trigger_col, trigger_prefix);
        self.set_lsp_status(format!("{} completions available", self.completion_menu.items().len()));

        Ok(true)
    }

    /// Formats the current document via LSP (implementation)
    async fn format_document_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                // LSP manager is busy (e.g., Java initialization), will retry next iteration
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let edits = lsp_guard
            .format_document(&uri, language_id, 4, true) // 4 spaces, insert spaces
            .await?;

        drop(lsp_guard);

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
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                // LSP manager is busy (e.g., Java initialization), will retry next iteration
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        let diagnostics = lsp_guard.get_diagnostics_for_line(&uri, line).await;
        let actions = lsp_guard
            .code_actions(&uri, line, character, language_id, diagnostics)
            .await?;

        drop(lsp_guard);

        if actions.is_empty() {
            self.set_lsp_status("No code actions available".to_string());
            return Ok(false);
        }

        // Store actions and create picker
        self.available_code_actions = actions.clone();

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
            b.range.start.line.cmp(&a.range.start.line)
                .then(b.range.start.character.cmp(&a.range.start.character))
        });

        for edit in sorted_edits {
            let start_line = edit.range.start.line as usize;
            let start_col = edit.range.start.character as usize;
            let end_line = edit.range.end.line as usize;
            let end_col = edit.range.end.character as usize;

            // Delete the range
            if start_line != end_line || start_col != end_col {
                self.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
            }

            // Insert new text
            if !edit.new_text.is_empty() {
                self.buffer_mut().insert_text_at(start_line, start_col, &edit.new_text);
            }
        }
    }

    /// Applies a selected code action from the picker
    pub fn apply_code_action(&mut self, action_index: usize) {
        // Check if we have actions and the index is valid
        if action_index >= self.available_code_actions.len() {
            self.set_lsp_status("Invalid code action selection".to_string());
            return;
        }

        let action = self.available_code_actions[action_index].clone();

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
                                        self.set_lsp_status("Failed to resolve file path".to_string());
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
                                        let abs_path = if std::path::Path::new(file_path).is_absolute() {
                                            file_path.to_string()
                                        } else {
                                            match std::env::current_dir() {
                                                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                                                Err(_) => continue,
                                            }
                                        };

                                        if let Ok(uri) = lsp_types::Url::from_file_path(&abs_path) {
                                            if text_doc_edit.text_document.uri == uri {
                                                self.apply_lsp_edits(text_doc_edit.edits.iter().filter_map(|e| {
                                                    match e {
                                                        lsp_types::OneOf::Left(edit) => Some(edit.clone()),
                                                        lsp_types::OneOf::Right(annot_edit) => Some(annot_edit.text_edit.clone()),
                                                    }
                                                }).collect());
                                                self.set_lsp_status(format!("Applied: {}", action_title));
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
                                                let abs_path = if std::path::Path::new(file_path).is_absolute() {
                                                    file_path.to_string()
                                                } else {
                                                    match std::env::current_dir() {
                                                        Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
                                                        Err(_) => continue,
                                                    }
                                                };

                                                if let Ok(uri) = lsp_types::Url::from_file_path(&abs_path) {
                                                    if text_doc_edit.text_document.uri == uri {
                                                        self.apply_lsp_edits(text_doc_edit.edits.iter().filter_map(|e| {
                                                            match e {
                                                                lsp_types::OneOf::Left(edit) => Some(edit.clone()),
                                                                lsp_types::OneOf::Right(annot_edit) => Some(annot_edit.text_edit.clone()),
                                                            }
                                                        }).collect());
                                                        self.set_lsp_status(format!("Applied: {}", action_title));
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

                let Some(language_id) = crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) else {
                    self.set_lsp_status("Language not supported".to_string());
                    return;
                };

                // Execute command asynchronously
                if let Some(lsp) = self.lsp_manager.clone() {
                    self.set_lsp_status(format!("Executing: {}", command_title));

                    tokio::spawn(async move {
                        let guard = lsp.lock().await;
                        let _result = guard.execute_command(command_name, arguments, &language_id).await;
                        // Note: Result isn't sent back to editor - this is fire and forget
                        // A full implementation would use a channel to send results back
                    });
                } else {
                    self.set_lsp_status("LSP not available".to_string());
                }
            }
        }

        // Clear available actions after applying
        self.available_code_actions.clear();
    }

    /// Applies the selected completion item
    pub fn apply_completion(&mut self, completion_index: usize) {
        // Check if we have completions and the index is valid
        if completion_index >= self.available_completions.len() {
            self.set_lsp_status("Invalid completion selection".to_string());
            return;
        }

        // Clone the completion data we need before mutable borrow
        let completion = self.available_completions[completion_index].clone();
        let insert_text = completion.insert_text.as_ref()
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
        self.buffer_mut().rope_mut().insert(insert_pos, &insert_text);

        // Move cursor to end of inserted text
        let new_col = col + insert_text.chars().count();
        self.buffer_mut().cursor_mut().set_position(line, new_col);

        self.set_lsp_status(format!("Inserted: {}", label));

        // Clear available completions after applying
        self.available_completions.clear();
    }

    /// Navigates to an LSP location from the picker selection
    pub fn navigate_to_lsp_location(&mut self, index: usize) {
        // Determine which LSP result type we're viewing
        let result_type = match &self.active_lsp_result_type {
            Some(t) => t.clone(),
            None => {
                self.set_lsp_status("No active LSP results".to_string());
                return;
            }
        };

        // Get the location based on result type
        let location = match result_type {
            LspResultType::References => {
                if index >= self.available_references.len() {
                    self.set_lsp_status("Invalid reference selection".to_string());
                    return;
                }
                self.available_references[index].clone()
            }
            LspResultType::DocumentSymbols => {
                if index >= self.available_document_symbols.len() {
                    self.set_lsp_status("Invalid symbol selection".to_string());
                    return;
                }
                let symbol = &self.available_document_symbols[index];
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
                if index >= self.available_workspace_symbols.len() {
                    self.set_lsp_status("Invalid symbol selection".to_string());
                    return;
                }
                self.available_workspace_symbols[index].location.clone()
            }
            LspResultType::CallHierarchy | LspResultType::TypeHierarchy => {
                let storage = if result_type == LspResultType::CallHierarchy {
                    &self.available_call_hierarchy
                } else {
                    &self.available_type_hierarchy
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
        let lsp = match &self.lsp_manager {
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

        // Try to get lock without blocking - if LSP is busy, retry later
        let lsp_guard = match lsp.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                // LSP manager is busy (e.g., Java initialization), will retry next iteration
                return Err(anyhow::anyhow!("LSP busy"));
            }
        };

        // Execute the organize imports command
        // For jdtls, the command is "java.action.organizeImports"
        let command = "java.action.organizeImports".to_string();
        let arguments = Some(vec![serde_json::to_value(&uri)?]);

        let result = lsp_guard
            .execute_command(command, arguments, language_id)
            .await;

        drop(lsp_guard);

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

    /// Applies a workspace edit from the LSP server
    /// Returns true if all edits were applied successfully
    pub async fn apply_workspace_edit(
        &mut self,
        edit: lsp_types::WorkspaceEdit,
    ) -> Result<bool> {
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
                                    let text_edits: Vec<lsp_types::TextEdit> = text_doc_edit.edits.iter().filter_map(|e| {
                                        match e {
                                            lsp_types::OneOf::Left(edit) => Some(edit.clone()),
                                            lsp_types::OneOf::Right(annot_edit) => Some(annot_edit.text_edit.clone()),
                                        }
                                    }).collect();
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
                                            let text_edits: Vec<lsp_types::TextEdit> = text_doc_edit.edits.iter().filter_map(|e| {
                                                match e {
                                                    lsp_types::OneOf::Left(edit) => Some(edit.clone()),
                                                    lsp_types::OneOf::Right(annot_edit) => Some(annot_edit.text_edit.clone()),
                                                }
                                            }).collect();
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
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use crate::ui::buffer_to_ansi;

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
                self.buffer_mut().delete_range(cursor_line, trigger_col, cursor_line, cursor_col);
            }

            // Insert the completion text
            self.buffer_mut().insert_text_at(cursor_line, trigger_col, &text_to_insert);

            // Move cursor to end of inserted text
            let new_col = trigger_col + text_to_insert.chars().count();
            self.buffer_mut().cursor_mut().set_position(cursor_line, new_col);

            // Mark buffer as modified
            self.buffer_modified_this_iteration = true;
        }

        // Hide the completion menu
        self.hide_completion_menu();
    }

    /// Gets the inlay hints for the current file
    pub fn inlay_hints(&self) -> &[lsp_types::InlayHint] {
        &self.inlay_hints
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
            self.mode = Mode::FileTree;
        } else {
            self.file_tree.toggle();
            self.mode = Mode::Normal;
        }
    }

    /// Opens the file selected in the file tree
    pub fn open_file_from_tree(&mut self) {
        if let Some(node) = self.file_tree.selected_node() {
            if node.is_dir() {
                // Toggle directory expansion
                self.file_tree.toggle_selected();
            } else {
                // Open file
                let path = node.path().to_path_buf();
                // Load file into buffer (reuse existing buffer loading logic)
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let mut buffer = Buffer::from_str(&content);
                    buffer.set_file_path(path.to_str().unwrap_or("").to_string());
                    self.buffers.push(buffer);
                    self.current_buffer_index = self.buffers.len() - 1;
                    self.needs_lsp_init = true;
                    // Switch back to Normal mode and keep file tree visible
                    self.mode = Mode::Normal;
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
        if let Some(entry) = self.quickfix_list.current_entry() {
            if let Some(ref path) = entry.filename {
                // Load the file if needed
                if let Ok(content) = std::fs::read_to_string(path) {
                    let mut buffer = Buffer::from_str(&content);
                    buffer.set_file_path(path.to_str().unwrap_or("").to_string());
                    self.buffers.push(buffer);
                    self.current_buffer_index = self.buffers.len() - 1;
                    self.needs_lsp_init = true;

                    // Move cursor to the location
                    if entry.lnum > 0 {
                        let line = entry.lnum.saturating_sub(1);
                        let col = if entry.col > 0 {
                            entry.col.saturating_sub(1)
                        } else {
                            0
                        };
                        self.buffer_mut().cursor_mut().set_position(line, col);
                    }
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
        if let Some(entry) = self.location_list.current_entry() {
            if let Some(ref path) = entry.filename {
                // Load the file if needed
                if let Ok(content) = std::fs::read_to_string(path) {
                    let mut buffer = Buffer::from_str(&content);
                    buffer.set_file_path(path.to_str().unwrap_or("").to_string());
                    self.buffers.push(buffer);
                    self.current_buffer_index = self.buffers.len() - 1;
                    self.needs_lsp_init = true;

                    // Move cursor to the location
                    if entry.lnum > 0 {
                        let line = entry.lnum.saturating_sub(1);
                        let col = if entry.col > 0 {
                            entry.col.saturating_sub(1)
                        } else {
                            0
                        };
                        self.buffer_mut().cursor_mut().set_position(line, col);
                    }
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
        self.tab_page_manager.new_tab(title);
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
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
