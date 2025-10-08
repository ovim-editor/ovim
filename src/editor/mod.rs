mod change;
mod input;
mod macros;
mod marks;
mod motions;
mod operators;
mod picker;
mod register;
mod search;
mod textobjects;
mod undo;
mod window;

pub use change::{Change, ChangeBuilder, ChangeManager, Position, Range};
pub use input::InputHandler;
pub use macros::MacroManager;
pub use marks::{JumpList, Mark, MarkManager};
pub use motions::Motions;
pub use operators::{Operator, Operators};
pub use picker::{Picker, PickerMode, PickerResult};
pub use register::RegisterManager;
pub use search::Search;
pub use textobjects::{TextObjectRange, TextObjects};
pub use undo::UndoManager;
pub use window::{SplitDirection, Window, WindowManager, WindowNode};

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

/// The main editor state
pub struct Editor {
    /// The text buffer
    buffer: Buffer,
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
    /// Preview cache for picker (file_path -> (content, syntax highlights))
    preview_cache: HashMap<String, PreviewCache>,
    /// Color scheme registry
    color_scheme_registry: ColorSchemeRegistry,
    /// Current color scheme name
    current_color_scheme: String,
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
    ShowHover,
    Completion,
    FormatDocument,
    CodeActions,
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
            buffer,
            mode: Mode::default(),
            should_quit: false,
            count: None,
            pending_operator: None,
            pending_command: None,
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
            #[cfg(feature = "lua")]
            lua_context: None,
            #[cfg(feature = "lua")]
            editor_bridge: None,
            last_insert_position: None,
            available_code_actions: Vec::new(),
            available_completions: Vec::new(),
            preview_cache: HashMap::new(),
            color_scheme_registry: ColorSchemeRegistry::new(),
            current_color_scheme: "tokyonight".to_string(),
        }
    }

    /// Creates an editor with initial content
    pub fn with_content(content: &str) -> Self {
        let buffer = Buffer::from_str(content);

        Self {
            buffer,
            mode: Mode::default(),
            should_quit: false,
            count: None,
            pending_operator: None,
            pending_command: None,
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
            #[cfg(feature = "lua")]
            lua_context: None,
            #[cfg(feature = "lua")]
            editor_bridge: None,
            last_insert_position: None,
            available_code_actions: Vec::new(),
            available_completions: Vec::new(),
            preview_cache: HashMap::new(),
            color_scheme_registry: ColorSchemeRegistry::new(),
            current_color_scheme: "tokyonight".to_string(),
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
            let _ = context.load_config();
            self.lua_context = Some(context);
            self.editor_bridge = Some(bridge);
        }
        Ok(())
    }

    /// Syncs the current editor state to the Lua bridge
    #[cfg(feature = "lua")]
    fn sync_lua_bridge(&self, bridge: &crate::lua::EditorBridge) {
        // Update cursor position
        let cursor = self.buffer.cursor();
        bridge.update_cursor(cursor.line(), cursor.col());
        // Update buffer content
        bridge.update_buffer(self.buffer.rope().to_string());
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
        let cursor = self.buffer.cursor();

        // Start search from current cursor position (inclusive)
        if let Some((line, col, _)) = search.find_next(&self.buffer, cursor.line(), cursor.col()) {
            self.buffer.cursor_mut().set_position(line, col);
            self.current_search = Some(search);
        }
    }

    /// Finds the next search match (n command)
    pub fn search_next(&mut self) {
        if let Some(ref mut search) = self.current_search {
            let cursor = self.buffer.cursor();
            let is_forward = search.is_forward();

            // For forward search, start from col+1; for backward, start from col-1 or col
            let search_col = if is_forward {
                cursor.col() + 1
            } else {
                if cursor.col() > 0 { cursor.col() - 1 } else { 0 }
            };

            if let Some((line, col, _)) = search.find_next(&self.buffer, cursor.line(), search_col) {
                self.buffer.cursor_mut().set_position(line, col);
            }
        }
    }

    /// Finds the previous search match (N command)
    pub fn search_prev(&mut self) {
        if let Some(ref search) = self.current_search {
            // Create a reversed search
            let is_forward = search.is_forward();
            let mut rev_search = Search::new(search.pattern().to_string(), !is_forward);
            let cursor = self.buffer.cursor();

            // For reverse direction: if original was forward, now going backward (use col-1)
            // if original was backward, now going forward (use col+1)
            let search_col = if is_forward {
                // Original was forward, now backward
                if cursor.col() > 0 { cursor.col() - 1 } else { 0 }
            } else {
                // Original was backward, now forward
                cursor.col() + 1
            };

            if let Some((line, col, _)) = rev_search.find_next(&self.buffer, cursor.line(), search_col) {
                self.buffer.cursor_mut().set_position(line, col);
            }
        }
    }

    /// Sets a mark at the current cursor position
    pub fn set_mark(&mut self, name: char) -> bool {
        let cursor = self.buffer.cursor();
        self.marks.set_mark(name, cursor.line(), cursor.col())
    }

    /// Jumps to a mark (exact position with backtick)
    pub fn jump_to_mark(&mut self, name: char) -> bool {
        if let Some(mark) = self.marks.get_mark(name) {
            self.buffer.cursor_mut().set_position(mark.line, mark.col);
            true
        } else {
            false
        }
    }

    /// Jumps to mark line (apostrophe - goes to first non-blank on line)
    pub fn jump_to_mark_line(&mut self, name: char) -> bool {
        if let Some(mark) = self.marks.get_mark(name) {
            self.buffer.cursor_mut().set_position(mark.line, 0);
            // TODO: Move to first non-blank character
            true
        } else {
            false
        }
    }

    /// Adds current position to jump list
    pub fn add_jump(&mut self) {
        let cursor = self.buffer.cursor();
        self.jump_list.add_jump(cursor.line(), cursor.col());
    }

    /// Jumps back in the jump list (Ctrl-O)
    pub fn jump_back(&mut self) -> bool {
        if let Some((line, col)) = self.jump_list.jump_back() {
            self.buffer.cursor_mut().set_position(line, col);
            true
        } else {
            false
        }
    }

    /// Jumps forward in the jump list (Ctrl-I)
    pub fn jump_forward(&mut self) -> bool {
        if let Some((line, col)) = self.jump_list.jump_forward() {
            self.buffer.cursor_mut().set_position(line, col);
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

    /// Gets a reference to the buffer
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// Gets a mutable reference to the buffer
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffer
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
            let cursor = self.buffer.cursor();
            let mut end = (cursor.line(), cursor.col());

            match self.mode {
                Mode::VisualLine => {
                    // Get the length of the end line (excluding newline)
                    if let Some(line_text) = self.buffer.line(end.0) {
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
                        if let Some(line_text) = self.buffer.line(new_end.0) {
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

    /// Loads a file into the editor
    pub fn load_file<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        self.buffer = Buffer::load_file(path)?;
        self.change_manager = ChangeManager::new();
        self.needs_lsp_init = true; // Flag that LSP needs initialization
        Ok(())
    }

    /// Checks if LSP initialization is needed and returns the file path
    pub fn needs_lsp_init(&self) -> Option<String> {
        if self.needs_lsp_init {
            self.buffer.file_path().map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Clears the LSP init flag (should be called after initializing LSP)
    pub fn clear_lsp_init_flag(&mut self) {
        self.needs_lsp_init = false;
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
        self.change_manager.undo(&mut self.buffer);
    }

    /// Redoes the next change
    pub fn redo(&mut self) {
        self.change_manager.redo(&mut self.buffer);
    }

    /// Repeats the last change
    pub fn repeat_last_change(&mut self) {
        self.change_manager.repeat_last(&mut self.buffer);
    }

    /// Checks if buffer is modified relative to last save
    pub fn is_modified(&self) -> bool {
        !self.change_manager.is_at_save_point()
    }

    /// Marks current state as saved
    pub fn mark_saved(&mut self) {
        self.change_manager.mark_saved();
        self.buffer.mark_clean();
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

    /// Closes the picker
    pub fn close_picker(&mut self) {
        self.picker = None;
        // Clear preview cache when closing picker to free memory
        self.preview_cache.clear();
    }

    /// Gets preview from cache or loads it
    pub fn get_or_load_preview(&mut self, file_path: &str) -> Option<&PreviewCache> {
        // Check if already cached
        if self.preview_cache.contains_key(file_path) {
            return self.preview_cache.get(file_path);
        }

        // Check file size before loading (max 1MB for preview)
        const MAX_PREVIEW_SIZE: u64 = 1024 * 1024;
        if let Ok(metadata) = std::fs::metadata(file_path) {
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
        let content = match std::fs::read_to_string(file_path) {
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

    /// Gets cached preview if available
    pub fn get_preview_cache(&self, file_path: &str) -> Option<&PreviewCache> {
        self.preview_cache.get(file_path)
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
        let file_path = self.buffer.file_path()?;
        let uri = lsp_types::Url::from_file_path(file_path).ok()?;

        // Try to get lock without blocking - return None if busy
        let lsp_guard = lsp.try_lock().ok()?;
        Some(lsp_guard.get_diagnostics(&uri).await)
    }

    /// Gets diagnostic count for the current file (errors, warnings, info, hints)
    pub async fn get_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        if let Some(lsp) = &self.lsp_manager {
            if let Some(file_path) = self.buffer.file_path() {
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
        if let Some(path) = self.buffer.file_path() {
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
        let Some((content, version, language)) = self.buffer.get_rehighlight_data() else {
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
            self.buffer.apply_highlights(highlights, version);
        }
    }

    /// Request go to definition (sets pending action)
    pub fn request_goto_definition(&mut self) {
        self.pending_lsp_action = Some(LspAction::GoToDefinition);
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

    /// Sends didChange notification if buffer was modified, then resets the flag
    pub async fn send_lsp_changes_if_modified(&mut self) {
        if !self.buffer_modified_this_iteration {
            return;
        }

        self.buffer_modified_this_iteration = false;

        let Some(ref lsp) = self.lsp_manager else {
            return;
        };

        let Some(file_path) = self.buffer.file_path() else {
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
        let content = self.buffer.rope().to_string();

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

        let Some(file_path) = self.buffer.file_path() else {
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

        let text = Some(self.buffer.rope().to_string());

        // Try to get lock without blocking - if LSP is busy, skip
        let Ok(lsp_guard) = lsp.try_lock() else {
            return; // LSP busy, save notification will be sent on next save
        };

        let _ = lsp_guard.did_save(uri, language_id, text).await;
    }

    /// Process any pending LSP actions
    pub async fn process_pending_lsp_actions(&mut self) {
        if let Some(action) = self.pending_lsp_action.take() {
            let retry = match action {
                LspAction::GoToDefinition => {
                    matches!(self.goto_definition_impl().await, Err(_))
                }
                LspAction::ShowHover => {
                    matches!(self.hover_impl().await, Err(_))
                }
                LspAction::Completion => {
                    matches!(self.completion_impl().await, Err(_))
                }
                LspAction::FormatDocument => {
                    matches!(self.format_document_impl().await, Err(_))
                }
                LspAction::CodeActions => {
                    matches!(self.code_actions_impl().await, Err(_))
                }
            };

            // If action returned error (e.g., couldn't get lock), put it back for retry
            if retry {
                self.pending_lsp_action = Some(action);
            }
        }
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
        let Some(file_path) = self.buffer.file_path() else {
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
        let cursor = self.buffer.cursor();
        let line = cursor.line() as u32;
        let character = cursor.col() as u32;

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
            let current_line = self.buffer.cursor().line();
            let current_col = self.buffer.cursor().col();
            self.jump_list.add_jump(current_line, current_col);

            // Check if definition is in the same file
            if location.uri == uri {
                // Same file - jump directly
                self.buffer.cursor_mut().set_position(target_line, target_col);
                self.set_lsp_status(format!("Definition found at line {}", target_line + 1));
                return Ok(true);
            } else {
                // Different file - open it and jump
                match location.uri.to_file_path() {
                    Ok(target_path) => {
                        // Try to open the target file
                        match self.load_file(&target_path) {
                            Ok(_) => {
                                self.buffer.cursor_mut().set_position(target_line, target_col);
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
        let Some(file_path) = self.buffer.file_path() else {
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
        let cursor = self.buffer.cursor();
        let line = cursor.line() as u32;
        let character = cursor.col() as u32;

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
            self.set_lsp_status("No hover information found".to_string());
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
        let Some(file_path) = self.buffer.file_path() else {
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
        let cursor = self.buffer.cursor();
        let line = cursor.line() as u32;
        let character = cursor.col() as u32;

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

        // Extract labels for picker
        let labels: Vec<String> = items
            .iter()
            .map(|item| {
                // Use label or insert_text, fallback to empty
                item.label.clone()
            })
            .collect();

        // Create picker with completion items
        let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let mut picker = crate::editor::Picker::new_completion(base_dir, labels);
        picker.set_prompt("Completion: ".to_string());

        self.set_picker(picker);
        self.set_mode(Mode::Picker);
        self.set_lsp_status(format!("{} completions available", items.len()));

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
        let Some(file_path) = self.buffer.file_path() else {
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
        let Some(file_path) = self.buffer.file_path() else {
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
        let cursor = self.buffer.cursor();
        let line = cursor.line() as u32;
        let character = cursor.col() as u32;

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
                self.buffer.delete_range(start_line, start_col, end_line, end_col);
            }

            // Insert new text
            if !edit.new_text.is_empty() {
                self.buffer.insert_text_at(start_line, start_col, &edit.new_text);
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
                        if let Some(file_path) = self.buffer.file_path() {
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
                                    if let Some(file_path) = self.buffer.file_path() {
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
                                            if let Some(file_path) = self.buffer.file_path() {
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
            lsp_types::CodeActionOrCommand::Command(_cmd) => {
                // Commands are not directly supported - would need to execute via LSP
                self.set_lsp_status("Command execution not yet supported".to_string());
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
        let cursor = self.buffer.cursor();
        let line = cursor.line();
        let col = cursor.col();

        // Get the line's char index
        let line_char_idx = self.buffer.rope().line_to_char(line);
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

    /// Renders the editor to an in-memory buffer and returns ANSI output
    /// Used for headless mode to get pixel-perfect terminal representation
    pub fn render_to_ansi(&self, width: u16, height: u16) -> Result<String> {
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
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
