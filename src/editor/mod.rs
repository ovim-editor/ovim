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

use crate::buffer::Buffer;
use crate::lsp::LspManager;
use crate::mode::Mode;
use anyhow::Result;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspAction {
    GoToDefinition,
    ShowHover,
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

        if let Some((line, col, _)) = search.find_next(&self.buffer, cursor.line(), cursor.col() + 1) {
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
    pub fn visual_selection(&self) -> Option<((usize, usize), (usize, usize))> {
        self.visual_start.map(|start| {
            let cursor = self.buffer.cursor();
            let mut end = (cursor.line(), cursor.col());

            // In Visual Line mode, extend selection to end of line
            if self.mode == Mode::VisualLine {
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
            } else {
                // Normal visual mode behavior
                // Normalize so start is always before end
                if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
                    (start, end)
                } else {
                    (end, start)
                }
            }
        })
    }

    /// Loads a file into the editor
    pub fn load_file<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        self.buffer = Buffer::load_file(path)?;
        self.change_manager = ChangeManager::new();
        Ok(())
    }

    /// Starts building a composite change (e.g., when entering insert mode)
    pub fn start_change_building(&mut self, cursor_before: Position) {
        self.change_manager.start_building(cursor_before);
    }

    /// Adds a change to the change manager
    pub fn add_change(&mut self, change: Change) {
        self.change_manager.add_change(change);
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

        let lsp_guard = lsp.lock().await;
        Some(lsp_guard.get_diagnostics(&uri).await)
    }

    /// Gets diagnostic count for the current file (errors, warnings, info, hints)
    pub async fn get_diagnostic_count(&self) -> (usize, usize, usize, usize) {
        if let Some(lsp) = &self.lsp_manager {
            if let Some(file_path) = self.buffer.file_path() {
                if let Ok(uri) = lsp_types::Url::from_file_path(file_path) {
                    let lsp_guard = lsp.lock().await;
                    return lsp_guard.count_diagnostics(&uri).await;
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

    /// Gets the current hover information (if any)
    pub fn hover_info(&self) -> Option<&str> {
        self.hover_info.as_deref()
    }

    /// Clears the hover information
    pub fn clear_hover(&mut self) {
        self.hover_info = None;
    }

    /// Process any pending LSP actions
    pub async fn process_pending_lsp_actions(&mut self) {
        if let Some(action) = self.pending_lsp_action.take() {
            eprintln!("Processing LSP action: {:?}", action);
            match action {
                LspAction::GoToDefinition => {
                    let _ = self.goto_definition_impl().await;
                }
                LspAction::ShowHover => {
                    let _ = self.hover_impl().await;
                }
            }
        }
    }

    /// Go to definition at current cursor position via LSP (implementation)
    async fn goto_definition_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled
        let Some(ref lsp) = self.lsp_manager else {
            eprintln!("LSP not enabled");
            return Ok(false);
        };

        // Get current file URI
        let Some(file_path) = self.buffer.file_path() else {
            eprintln!("No file path");
            return Ok(false);
        };

        let uri = lsp_types::Url::from_file_path(file_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer.cursor();
        let line = cursor.line() as u32;
        let character = cursor.col() as u32;
        eprintln!("goto_definition: file={}, line={}, char={}", file_path, line, character);

        // Detect language from file extension
        let language_id = if file_path.ends_with(".rs") {
            "rust"
        } else if file_path.ends_with(".js") || file_path.ends_with(".ts") {
            "javascript"
        } else if file_path.ends_with(".py") {
            "python"
        } else {
            return Ok(false);
        };

        // Request definition
        eprintln!("Requesting definition from LSP for {}", language_id);
        let lsp_guard = lsp.lock().await;
        let location = lsp_guard
            .goto_definition(&uri, line, character, language_id)
            .await?;

        drop(lsp_guard);
        eprintln!("LSP response: location={:?}", location);

        // Jump to definition if found
        if let Some(location) = location {
            // For now, only handle same-file definitions
            if location.uri == uri {
                let target_line = location.range.start.line as usize;
                let target_col = location.range.start.character as usize;

                // Update cursor position
                self.buffer.cursor_mut().set_position(target_line, target_col);
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Gets hover information at current cursor position via LSP (implementation)
    async fn hover_impl(&mut self) -> Result<bool> {
        // Check if LSP is enabled
        let Some(ref lsp) = self.lsp_manager else {
            return Ok(false);
        };

        // Get current file URI
        let Some(file_path) = self.buffer.file_path() else {
            return Ok(false);
        };

        let uri = lsp_types::Url::from_file_path(file_path)
            .map_err(|_| anyhow::anyhow!("Invalid file path"))?;

        // Get cursor position
        let cursor = self.buffer.cursor();
        let line = cursor.line() as u32;
        let character = cursor.col() as u32;

        // Detect language from file extension
        let language_id = if file_path.ends_with(".rs") {
            "rust"
        } else if file_path.ends_with(".js") || file_path.ends_with(".ts") {
            "javascript"
        } else if file_path.ends_with(".py") {
            "python"
        } else {
            return Ok(false);
        };

        // Request hover information
        let lsp_guard = lsp.lock().await;
        let hover_text = lsp_guard
            .hover(&uri, line, character, language_id)
            .await?;

        drop(lsp_guard);

        // Store hover information
        self.hover_info = hover_text;

        Ok(self.hover_info.is_some())
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
