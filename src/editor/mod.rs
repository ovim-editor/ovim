mod input;
mod macros;
mod marks;
mod motions;
mod operators;
mod register;
mod search;
mod textobjects;
mod undo;

pub use input::InputHandler;
pub use macros::MacroManager;
pub use marks::{JumpList, Mark, MarkManager};
pub use motions::Motions;
pub use operators::{Operator, Operators};
pub use register::RegisterManager;
pub use search::Search;
pub use textobjects::{TextObjectRange, TextObjects};
pub use undo::UndoManager;

use crate::buffer::Buffer;
use crate::mode::Mode;
use anyhow::Result;

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
    /// Undo/redo manager
    undo_manager: UndoManager,
    /// Mark manager for buffer marks
    marks: MarkManager,
    /// Jump list for Ctrl-O and Ctrl-I
    jump_list: JumpList,
    /// Macro manager for recording and playback
    macro_manager: MacroManager,
}

impl Editor {
    /// Creates a new editor with an empty buffer
    pub fn new() -> Self {
        let buffer = Buffer::new();
        let undo_manager = UndoManager::new(&buffer);

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
            undo_manager,
            marks: MarkManager::new(),
            jump_list: JumpList::new(),
            macro_manager: MacroManager::new(),
        }
    }

    /// Creates an editor with initial content
    pub fn with_content(content: &str) -> Self {
        let buffer = Buffer::from_str(content);
        let undo_manager = UndoManager::new(&buffer);

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
            undo_manager,
            marks: MarkManager::new(),
            jump_list: JumpList::new(),
            macro_manager: MacroManager::new(),
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
            let end = (cursor.line(), cursor.col());

            // Normalize so start is always before end
            if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
                (start, end)
            } else {
                (end, start)
            }
        })
    }

    /// Loads a file into the editor
    pub fn load_file<P: AsRef<std::path::Path>>(&mut self, path: P) -> Result<()> {
        self.buffer = Buffer::load_file(path)?;
        self.undo_manager = UndoManager::new(&self.buffer);
        Ok(())
    }

    /// Saves the current buffer state for undo
    pub fn save_undo_state(&mut self) {
        self.undo_manager.save_state(&self.buffer);
    }

    /// Undoes the last change
    pub fn undo(&mut self) {
        if let Some((rope, cursor)) = self.undo_manager.undo() {
            *self.buffer.rope_mut() = rope;
            *self.buffer.cursor_mut() = cursor;
        }
    }

    /// Redoes the next change
    pub fn redo(&mut self) {
        if let Some((rope, cursor)) = self.undo_manager.redo() {
            *self.buffer.rope_mut() = rope;
            *self.buffer.cursor_mut() = cursor;
        }
    }

    /// Runs the editor (main loop will be implemented later)
    pub fn run(&mut self) -> Result<()> {
        // Placeholder for now
        Ok(())
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
