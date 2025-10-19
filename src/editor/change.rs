use crate::buffer::Buffer;

/// Position in the buffer (line, column)
pub type Position = (usize, usize);

/// Range in the buffer
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }

    /// Creates a range for a single position (empty range)
    pub fn at(position: Position) -> Self {
        Self {
            start: position,
            end: position,
        }
    }
}

/// Represents a semantic change to the buffer
#[derive(Clone, Debug)]
pub enum Change {
    /// Insert text at a position
    InsertText {
        position: Position,
        text: String,
        cursor_before: Position,
    },
    /// Delete text in a range
    DeleteText {
        range: Range,
        deleted_text: String, // Stored for undo
        cursor_before: Position,
    },
    /// A composite of multiple changes (e.g., all changes during insert mode)
    Composite {
        changes: Vec<Change>,
        cursor_before: Position,
        cursor_after: Position,
    },
}

impl Change {
    /// Creates an InsertText change
    pub fn insert(position: Position, text: String, cursor_before: Position) -> Self {
        Self::InsertText { position, text, cursor_before }
    }

    /// Creates a DeleteText change
    pub fn delete(range: Range, deleted_text: String, cursor_before: Position) -> Self {
        Self::DeleteText {
            range,
            deleted_text,
            cursor_before,
        }
    }

    /// Creates a Composite change
    pub fn composite(changes: Vec<Change>, cursor_before: Position, cursor_after: Position) -> Self {
        Self::Composite { changes, cursor_before, cursor_after }
    }

    /// Applies this change to the buffer
    pub fn apply(&self, buffer: &mut Buffer) {
        match self {
            Self::InsertText { position, text, .. } => {
                let (line, col) = *position;
                buffer.insert_text_at(line, col, text);
                // Update cursor to end of inserted text
                let end_pos = Self::calculate_end_position(*position, text);
                buffer.cursor_mut().set_position(end_pos.0, end_pos.1);
            }
            Self::DeleteText { range, .. } => {
                let (start_line, start_col) = range.start;
                let (end_line, end_col) = range.end;
                buffer.delete_range(start_line, start_col, end_line, end_col);
                // Position cursor at deletion start
                buffer.cursor_mut().set_position(start_line, start_col);
            }
            Self::Composite { changes, cursor_after, .. } => {
                for change in changes {
                    change.apply(buffer);
                }
                // Restore cursor to final position after composite operation
                buffer.cursor_mut().set_position(cursor_after.0, cursor_after.1);
            }
        }
    }

    /// Undoes this change on the buffer
    pub fn undo(&self, buffer: &mut Buffer) {
        match self {
            Self::InsertText { position, text, cursor_before } => {
                // To undo an insert, delete the inserted text
                // We need to use the rope state AFTER the insert to find the correct positions
                let (start_line, start_col) = *position;

                // Convert position to absolute char index using current rope
                let start_char = if start_line < buffer.rope().len_lines() {
                    buffer.rope().line_to_char(start_line) + start_col
                } else {
                    buffer.rope().len_chars()
                };

                // Calculate end char position by adding text length
                let text_len = text.chars().count();
                let end_char = (start_char + text_len).min(buffer.rope().len_chars());

                // Convert end_char back to (line, col)
                let end_line = buffer.rope().char_to_line(end_char);
                let end_line_start = buffer.rope().line_to_char(end_line);
                let end_col = end_char - end_line_start;

                buffer.delete_range(start_line, start_col, end_line, end_col);
                // Restore cursor to where it was before the change
                buffer.cursor_mut().set_position(cursor_before.0, cursor_before.1);
            }
            Self::DeleteText {
                range,
                deleted_text,
                cursor_before,
            } => {
                // To undo a delete, re-insert the deleted text
                let (line, col) = range.start;
                buffer.insert_text_at(line, col, deleted_text);
                // Restore cursor to where it was before the change
                buffer.cursor_mut().set_position(cursor_before.0, cursor_before.1);
            }
            Self::Composite { changes, cursor_before, .. } => {
                // Undo changes in reverse order
                for change in changes.iter().rev() {
                    change.undo(buffer);
                }
                // Restore cursor to where it was before the composite change
                buffer.cursor_mut().set_position(cursor_before.0, cursor_before.1);
            }
        }
    }

    /// Repeats this change at the current cursor position
    pub fn repeat(&self, buffer: &mut Buffer) {
        match self {
            Self::InsertText { text, .. } => {
                // Insert the same text at current position
                let position = (buffer.cursor().line(), buffer.cursor().col());
                let cursor_before = position;
                Self::InsertText {
                    position,
                    text: text.clone(),
                    cursor_before,
                }
                .apply(buffer);
            }
            Self::DeleteText { range, .. } => {
                // Apply the same deletion pattern from current position
                let cursor_pos = (buffer.cursor().line(), buffer.cursor().col());
                let offset_line = range.end.0 - range.start.0;
                let offset_col = if range.end.0 == range.start.0 {
                    range.end.1 - range.start.1
                } else {
                    range.end.1
                };

                let new_end = if offset_line == 0 {
                    (cursor_pos.0, cursor_pos.1 + offset_col)
                } else {
                    (cursor_pos.0 + offset_line, offset_col)
                };

                let (start_line, start_col) = cursor_pos;
                let (end_line, end_col) = new_end;
                let _deleted = buffer.delete_range(start_line, start_col, end_line, end_col);
                // Cursor is already positioned correctly by delete_range
            }
            Self::Composite { changes, .. } => {
                // For composite changes (like insert mode), replay all sub-changes
                for change in changes {
                    change.repeat(buffer);
                }
            }
        }
    }

    /// Helper to calculate end position after inserting text
    fn calculate_end_position(start: Position, text: &str) -> Position {
        let mut line = start.0;
        let mut col = start.1;

        for ch in text.chars() {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }

        (line, col)
    }

    /// Extracts the inserted text from this change (for the . register)
    pub fn get_inserted_text(&self) -> String {
        match self {
            Self::InsertText { text, .. } => text.clone(),
            Self::Composite { changes, .. } => {
                // Concatenate all inserted text from sub-changes
                let mut result = String::new();
                for change in changes {
                    result.push_str(&change.get_inserted_text());
                }
                result
            }
            Self::DeleteText { .. } => String::new(),
        }
    }
}

/// Builder for accumulating changes during insert mode
#[derive(Debug)]
pub struct ChangeBuilder {
    changes: Vec<Change>,
    cursor_before: Position,
    cursor_after: Option<Position>,
}

impl ChangeBuilder {
    pub fn new(cursor_before: Position) -> Self {
        Self {
            changes: Vec::new(),
            cursor_before,
            cursor_after: None,
        }
    }

    /// Adds a change to the builder
    pub fn add(&mut self, change: Change) {
        self.changes.push(change);
    }

    /// Sets the final cursor position after all changes
    pub fn set_cursor_after(&mut self, cursor_after: Position) {
        self.cursor_after = Some(cursor_after);
    }

    /// Finalizes the builder into a Change
    pub fn build(self, buffer_cursor: Position) -> Option<Change> {
        if self.changes.is_empty() {
            None
        } else if self.changes.len() == 1 {
            Some(self.changes.into_iter().next().unwrap())
        } else {
            // Use explicitly set cursor_after, or fall back to current buffer cursor
            let cursor_after = self.cursor_after.unwrap_or(buffer_cursor);
            Some(Change::Composite {
                changes: self.changes,
                cursor_before: self.cursor_before,
                cursor_after,
            })
        }
    }

    /// Returns true if the builder has no changes
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Manages undo/redo history and change tracking
#[derive(Debug)]
pub struct ChangeManager {
    pub(crate) undo_stack: Vec<Change>,
    pub(crate) redo_stack: Vec<Change>,
    pub(crate) current_builder: Option<ChangeBuilder>,
    pub(crate) last_change: Option<Change>,
    /// Tracks the undo stack size at last save (None if never saved)
    pub(crate) save_point: Option<usize>,
}

impl ChangeManager {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            current_builder: None,
            last_change: None,
            save_point: Some(0), // Start at save point (empty buffer is saved)
        }
    }

    /// Starts building a composite change (e.g., when entering insert mode)
    pub fn start_building(&mut self, cursor_before: Position) {
        self.current_builder = Some(ChangeBuilder::new(cursor_before));
    }

    /// Adds a change to the current builder, or pushes directly if not building
    pub fn add_change(&mut self, change: Change) {
        if let Some(builder) = &mut self.current_builder {
            builder.add(change);
        } else {
            // Direct change (not building), push to undo stack
            self.push_change(change);
        }
    }

    /// Finalizes the current builder and pushes the composite change
    pub fn finalize_building_at(&mut self, cursor_pos: Position) {
        if let Some(builder) = self.current_builder.take() {
            if let Some(change) = builder.build(cursor_pos) {
                self.push_change(change);
            }
        }
    }

    /// Pushes a change to the undo stack
    fn push_change(&mut self, change: Change) {
        self.undo_stack.push(change.clone());
        self.redo_stack.clear(); // Clear redo stack on new change
        self.last_change = Some(change);
    }

    /// Undoes the last change
    pub fn undo(&mut self, buffer: &mut Buffer) -> bool {
        if let Some(change) = self.undo_stack.pop() {
            change.undo(buffer);
            self.redo_stack.push(change);
            true
        } else {
            false
        }
    }

    /// Redoes the next change
    pub fn redo(&mut self, buffer: &mut Buffer) -> bool {
        if let Some(change) = self.redo_stack.pop() {
            change.apply(buffer);
            self.undo_stack.push(change);
            true
        } else {
            false
        }
    }

    /// Repeats the last change at the current cursor position
    pub fn repeat_last(&mut self, buffer: &mut Buffer) -> bool {
        if let Some(ref change) = self.last_change {
            let repeated_change = change.clone();
            repeated_change.repeat(buffer);
            // When repeating, we create a new change
            self.push_change(repeated_change);
            true
        } else {
            false
        }
    }

    /// Returns whether currently building a composite change
    pub fn is_building(&self) -> bool {
        self.current_builder.is_some()
    }

    /// Marks the current position as saved (after :w)
    pub fn mark_saved(&mut self) {
        self.save_point = Some(self.undo_stack.len());
    }

    /// Checks if we're at the save point (buffer is unmodified)
    pub fn is_at_save_point(&self) -> bool {
        self.save_point == Some(self.undo_stack.len())
    }

    /// Clears the save point (when loading a new file)
    pub fn clear_save_point(&mut self) {
        self.save_point = None;
    }

    /// Gets a reference to the last change
    pub fn last_change(&self) -> Option<&Change> {
        self.last_change.as_ref()
    }
}
