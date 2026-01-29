use crate::buffer::Buffer;
use crate::editor::textobjects::TextObjects;
use anyhow::Result;

/// Position in the buffer (line, column)
pub type Position = (usize, usize);

/// Types of text objects for semantic repeat
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextObjectType {
    /// Inner/around word (iw/aw)
    Word { inner: bool },
    /// Quoted string with specific quote char (i"/a", i'/a', i`/a`)
    Quote { char: char, inner: bool },
    /// Paired delimiters (i(/a(, i[/a[, i{/a{, i</a<)
    Paired { open: char, close: char, inner: bool },
}

/// Range in the buffer
#[derive(Clone, Debug, PartialEq, Eq, Default)]
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

/// How insert mode was entered — used by dot repeat to reposition the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InsertEntryMode {
    /// `i` — insert at cursor (no repositioning needed)
    Insert,
    /// `a` — append after cursor
    Append,
    /// `I` — insert at first non-blank of line
    FirstNonBlank,
    /// `A` — append at end of line
    EndOfLine,
    /// `o` — open line below (handled separately)
    OpenBelow,
    /// `O` — open line above (handled separately)
    OpenAbove,
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
        /// How insert mode was entered — tells dot repeat how to reposition.
        entry_mode: InsertEntryMode,
    },
    /// Number operation (increment/decrement) - stores the operation, not the text change
    /// This allows dot-repeat to work correctly on different numbers
    NumberOperation {
        delta: i64, // +1 for Ctrl-A, -1 for Ctrl-X (multiplied by count)
        cursor_before: Position,
        cursor_after: Position,
        // Store the original change for undo (the actual text that was changed)
        old_text: String,
        old_range: Range,
    },
    /// Join lines operation - stores the count and whether to add space
    /// This allows dot-repeat to work correctly
    JoinLines {
        count: usize,
        add_space: bool,
        cursor_before: Position,
        cursor_after: Position,
        // Store the original lines for undo
        old_text: String,
        old_range: Range,
    },
    /// Semantic text object change (ci", ci(, etc.)
    /// On repeat, re-evaluates the text object at current cursor position
    ChangeTextObject {
        object_type: TextObjectType,
        replacement: String,
        cursor_before: Position,
        cursor_after: Position,
        // Store original for undo
        old_text: String,
        old_range: Range,
    },
    /// Semantic text object delete (di", di(, etc.)
    /// On repeat, re-evaluates the text object at current cursor position
    DeleteTextObject {
        object_type: TextObjectType,
        cursor_before: Position,
        cursor_after: Position,
        // Store original for undo
        old_text: String,
        old_range: Range,
    },
    /// Semantic word change (cw, cW)
    /// On repeat, changes the word at current cursor position
    ChangeWord {
        replacement: String,
        cursor_before: Position,
        cursor_after: Position,
        // Store original for undo
        old_text: String,
        old_range: Range,
    },
    /// Replace mode operation - stores the full replacement sequence
    /// On repeat, replays the entire replacement at current cursor position
    ReplaceMode {
        replacements: String, // The characters that were typed in replace mode
        cursor_before: Position,
        cursor_after: Position,
        // Store original for undo
        old_text: String,
        old_range: Range,
    },
}

impl Change {
    /// Creates an InsertText change
    pub fn insert(position: Position, text: String, cursor_before: Position) -> Self {
        Self::InsertText {
            position,
            text,
            cursor_before,
        }
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
    pub fn composite(
        changes: Vec<Change>,
        cursor_before: Position,
        cursor_after: Position,
    ) -> Self {
        Self::Composite {
            changes,
            cursor_before,
            cursor_after,
            entry_mode: InsertEntryMode::Insert,
        }
    }

    /// Creates a NumberOperation change
    pub fn number_operation(
        delta: i64,
        cursor_before: Position,
        cursor_after: Position,
        old_text: String,
        old_range: Range,
    ) -> Self {
        Self::NumberOperation {
            delta,
            cursor_before,
            cursor_after,
            old_text,
            old_range,
        }
    }

    /// Creates a JoinLines change
    pub fn join_lines(
        count: usize,
        add_space: bool,
        cursor_before: Position,
        cursor_after: Position,
        old_text: String,
        old_range: Range,
    ) -> Self {
        Self::JoinLines {
            count,
            add_space,
            cursor_before,
            cursor_after,
            old_text,
            old_range,
        }
    }

    /// Creates a ChangeTextObject change (for ci", ci(, etc.)
    pub fn change_text_object(
        object_type: TextObjectType,
        replacement: String,
        cursor_before: Position,
        cursor_after: Position,
        old_text: String,
        old_range: Range,
    ) -> Self {
        Self::ChangeTextObject {
            object_type,
            replacement,
            cursor_before,
            cursor_after,
            old_text,
            old_range,
        }
    }

    /// Creates a DeleteTextObject change (for di", di(, etc.)
    pub fn delete_text_object(
        object_type: TextObjectType,
        cursor_before: Position,
        cursor_after: Position,
        old_text: String,
        old_range: Range,
    ) -> Self {
        Self::DeleteTextObject {
            object_type,
            cursor_before,
            cursor_after,
            old_text,
            old_range,
        }
    }

    /// Creates a ChangeWord change (for cw)
    pub fn change_word(
        replacement: String,
        cursor_before: Position,
        cursor_after: Position,
        old_text: String,
        old_range: Range,
    ) -> Self {
        Self::ChangeWord {
            replacement,
            cursor_before,
            cursor_after,
            old_text,
            old_range,
        }
    }

    /// Creates a ReplaceMode change (for R command)
    pub fn replace_mode(
        replacements: String,
        cursor_before: Position,
        cursor_after: Position,
        old_text: String,
        old_range: Range,
    ) -> Self {
        Self::ReplaceMode {
            replacements,
            cursor_before,
            cursor_after,
            old_text,
            old_range,
        }
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
            Self::Composite {
                changes,
                cursor_after,
                ..
            } => {
                for change in changes {
                    change.apply(buffer);
                }
                // Restore cursor to final position after composite operation
                buffer
                    .cursor_mut()
                    .set_position(cursor_after.0, cursor_after.1);
            }
            Self::NumberOperation {
                delta,
                cursor_after,
                old_range,
                ..
            } => {
                // For apply (redo), we need to re-execute the number operation
                let (line_idx, _col) = cursor_after;
                if let Some(line) = buffer.line(*line_idx) {
                    let line_text = line.trim_end_matches('\n');
                    let col = old_range.start.1;

                    if let Some((start_col, end_col, number_str)) =
                        find_number_at_or_after(line_text, col)
                    {
                        if let Ok((mut value, base, prefix_len)) = parse_number(&number_str) {
                            value += delta;
                            let new_number_str = format_number(value, base, prefix_len);

                            // Delete old number and insert new one
                            buffer.delete_range(*line_idx, start_col, *line_idx, end_col);
                            buffer.insert_text_at(*line_idx, start_col, &new_number_str);

                            // Position cursor on last digit
                            let new_end_col = start_col + new_number_str.len() - 1;
                            buffer.cursor_mut().set_position(*line_idx, new_end_col);
                        }
                    }
                }
            }
            Self::JoinLines {
                count,
                add_space,
                cursor_after,
                ..
            } => {
                // For apply (redo), we need to re-execute the join operation
                let _ = if *add_space {
                    buffer.join_lines(*count)
                } else {
                    buffer.join_lines_no_space(*count)
                };
                // Position cursor at the final position
                buffer.cursor_mut().set_position(cursor_after.0, cursor_after.1);
            }
            Self::ChangeTextObject {
                old_range,
                replacement,
                cursor_after,
                ..
            } => {
                // Delete old text and insert replacement
                let (start_line, start_col) = old_range.start;
                let (end_line, end_col) = old_range.end;
                buffer.delete_range(start_line, start_col, end_line, end_col);
                buffer.insert_text_at(start_line, start_col, replacement);
                buffer.cursor_mut().set_position(cursor_after.0, cursor_after.1);
            }
            Self::DeleteTextObject {
                old_range,
                cursor_after,
                ..
            } => {
                // Delete the text object content
                let (start_line, start_col) = old_range.start;
                let (end_line, end_col) = old_range.end;
                buffer.delete_range(start_line, start_col, end_line, end_col);
                buffer.cursor_mut().set_position(cursor_after.0, cursor_after.1);
            }
            Self::ChangeWord {
                old_range,
                replacement,
                cursor_after,
                ..
            } => {
                // Delete old word and insert replacement
                let (start_line, start_col) = old_range.start;
                let (end_line, end_col) = old_range.end;
                buffer.delete_range(start_line, start_col, end_line, end_col);
                buffer.insert_text_at(start_line, start_col, replacement);
                buffer.cursor_mut().set_position(cursor_after.0, cursor_after.1);
            }
            Self::ReplaceMode {
                old_range,
                replacements,
                cursor_after,
                ..
            } => {
                // Delete old text and insert replacements
                let (start_line, start_col) = old_range.start;
                let (end_line, end_col) = old_range.end;
                buffer.delete_range(start_line, start_col, end_line, end_col);
                buffer.insert_text_at(start_line, start_col, replacements);
                buffer.cursor_mut().set_position(cursor_after.0, cursor_after.1);
            }
        }
    }

    /// Undoes this change on the buffer
    pub fn undo(&self, buffer: &mut Buffer) {
        match self {
            Self::InsertText {
                position,
                text,
                cursor_before,
            } => {
                // To undo an insert, delete the inserted text using absolute
                // char positions. We can't use delete_range(line, col) because
                // it clamps columns via line_len() (which excludes newlines),
                // but insertions can target the newline position (e.g., line
                // paste inserts at rope().line().len_chars() which includes \n).
                let (start_line, start_col) = *position;

                let start_char = if start_line < buffer.rope().len_lines() {
                    buffer.rope().line_to_char(start_line) + start_col
                } else {
                    buffer.rope().len_chars()
                };

                let text_len = text.chars().count();
                let end_char = (start_char + text_len).min(buffer.rope().len_chars());

                buffer.delete_char_range(start_char, end_char);
                // Restore cursor to where it was before the change
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.0, cursor_before.1);
                // Validate cursor position in case line no longer exists
                buffer.validate_cursor_position();
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
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.0, cursor_before.1);
                // Validate cursor position in case line no longer exists
                buffer.validate_cursor_position();
            }
            Self::Composite {
                changes,
                cursor_before,
                ..
            } => {
                // Undo changes in reverse order
                for change in changes.iter().rev() {
                    change.undo(buffer);
                }
                // Restore cursor to where it was before the composite change
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.0, cursor_before.1);
                // Validate cursor position after composite undo - intermediate undos
                // may have deleted lines that the final cursor position refers to
                buffer.validate_cursor_position();
            }
            Self::NumberOperation {
                cursor_before,
                old_range,
                old_text,
                ..
            } => {
                // To undo a number operation, apply the negative delta
                // Or more reliably: restore the old text
                let (line_idx, start_col) = old_range.start;
                let (_, end_col) = old_range.end;

                // Delete the current number and insert the old text
                buffer.delete_range(line_idx, start_col, line_idx, end_col);
                buffer.insert_text_at(line_idx, start_col, old_text);

                // Restore cursor to where it was before
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.0, cursor_before.1);
            }
            Self::JoinLines {
                cursor_before,
                old_range,
                old_text,
                ..
            } => {
                // To undo a join operation, restore the old text
                let (start_line, start_col) = old_range.start;
                let (end_line, end_col) = old_range.end;

                // Delete the joined line and insert the old multi-line text
                buffer.delete_range(start_line, start_col, end_line, end_col);
                buffer.insert_text_at(start_line, start_col, old_text);

                // Restore cursor to where it was before
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.0, cursor_before.1);
            }
            Self::ChangeTextObject {
                cursor_before,
                old_range,
                old_text,
                replacement,
                ..
            } => {
                // To undo: delete the replacement and restore old text
                let (start_line, start_col) = old_range.start;
                // Calculate where replacement ends
                let replacement_end = Self::calculate_end_position((start_line, start_col), replacement);
                buffer.delete_range(start_line, start_col, replacement_end.0, replacement_end.1);
                buffer.insert_text_at(start_line, start_col, old_text);
                buffer.cursor_mut().set_position(cursor_before.0, cursor_before.1);
            }
            Self::DeleteTextObject {
                cursor_before,
                old_range,
                old_text,
                ..
            } => {
                // To undo: re-insert the deleted text
                let (start_line, start_col) = old_range.start;
                buffer.insert_text_at(start_line, start_col, old_text);
                buffer.cursor_mut().set_position(cursor_before.0, cursor_before.1);
            }
            Self::ChangeWord {
                cursor_before,
                old_range,
                old_text,
                replacement,
                ..
            } => {
                // To undo: delete the replacement and restore old word
                let (start_line, start_col) = old_range.start;
                let replacement_end = Self::calculate_end_position((start_line, start_col), replacement);
                buffer.delete_range(start_line, start_col, replacement_end.0, replacement_end.1);
                buffer.insert_text_at(start_line, start_col, old_text);
                buffer.cursor_mut().set_position(cursor_before.0, cursor_before.1);
            }
            Self::ReplaceMode {
                cursor_before,
                old_range,
                old_text,
                replacements,
                ..
            } => {
                // To undo: delete the replacements and restore old text
                let (start_line, start_col) = old_range.start;
                let replacement_end = Self::calculate_end_position((start_line, start_col), replacements);
                buffer.delete_range(start_line, start_col, replacement_end.0, replacement_end.1);
                buffer.insert_text_at(start_line, start_col, old_text);
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
            Self::DeleteText { range, cursor_before, .. } => {
                // Apply the same deletion pattern from current position
                let cursor_pos = (buffer.cursor().line(), buffer.cursor().col());
                let offset_line = range.end.0 - range.start.0;
                let offset_col = if range.end.0 == range.start.0 {
                    range.end.1 - range.start.1
                } else {
                    range.end.1
                };

                // Detect if this was a backwards deletion (like X command)
                // where cursor_before was at the end of the range
                let is_backwards = cursor_before == &range.end;

                let (start_line, start_col, end_line, end_col) = if is_backwards {
                    // For backwards deletion (X), treat current cursor as the END
                    // and calculate the start by going backwards
                    let new_start = if offset_line == 0 {
                        (cursor_pos.0, cursor_pos.1.saturating_sub(offset_col))
                    } else {
                        // Multi-line backwards deletion (shouldn't happen for X, but handle it)
                        ((cursor_pos.0).saturating_sub(offset_line), 0)
                    };
                    (new_start.0, new_start.1, cursor_pos.0, cursor_pos.1)
                } else {
                    // For forward deletion (x, d, etc), treat current cursor as the START
                    let new_end = if offset_line == 0 {
                        (cursor_pos.0, cursor_pos.1 + offset_col)
                    } else {
                        (cursor_pos.0 + offset_line, offset_col)
                    };
                    (cursor_pos.0, cursor_pos.1, new_end.0, new_end.1)
                };

                let _deleted = buffer.delete_range(start_line, start_col, end_line, end_col);
                // Position cursor at the start of the deletion
                // For backwards deletion (X), cursor should be one position before the deletion
                let final_col = if is_backwards && start_col > 0 {
                    start_col - 1
                } else {
                    start_col
                };
                buffer.cursor_mut().set_position(start_line, final_col);
            }
            Self::Composite { changes, entry_mode, .. } => {
                // Position cursor according to how insert mode was originally entered.
                match entry_mode {
                    InsertEntryMode::Insert => {
                        // i — cursor stays where it is
                    }
                    InsertEntryMode::Append => {
                        // a — move cursor right by 1
                        let cursor = buffer.cursor_mut();
                        cursor.move_right(1);
                    }
                    InsertEntryMode::FirstNonBlank => {
                        // I — move to first non-blank of current line
                        let line_idx = buffer.cursor().line();
                        if let Some(line) = buffer.line(line_idx) {
                            let content = line.trim_end_matches('\n');
                            let col = content.chars().position(|c| !c.is_whitespace()).unwrap_or(0);
                            buffer.cursor_mut().set_col(col);
                        }
                    }
                    InsertEntryMode::EndOfLine => {
                        // A — move to end of line
                        let line_idx = buffer.cursor().line();
                        if let Some(line) = buffer.line(line_idx) {
                            let line_len = line.trim_end_matches('\n').chars().count();
                            buffer.cursor_mut().set_col(line_len);
                        }
                    }
                    InsertEntryMode::OpenBelow => {
                        // o — insert newline below, position on new line
                        // Replicate insert_line_below logic using CURRENT line's indent
                        if !changes.is_empty() {
                            let line_idx = buffer.cursor().line();
                            let line_text = buffer.line(line_idx).unwrap_or_default();
                            let indent: String = line_text
                                .chars()
                                .take_while(|c| c.is_whitespace() && *c != '\n')
                                .collect();

                            let (insert_pos, text) = if line_text.ends_with('\n') {
                                ((line_idx + 1, 0), format!("{}\n", indent))
                            } else {
                                let line_len = line_text.chars().count();
                                ((line_idx, line_len), format!("\n{}\n", indent))
                            };

                            buffer.insert_text_at(insert_pos.0, insert_pos.1, &text);
                            buffer.cursor_mut().set_position(line_idx + 1, indent.chars().count());
                        }
                        // Replay remaining sub-changes (skip first which was the newline)
                        for change in changes.iter().skip(1) {
                            change.repeat(buffer);
                        }
                        let cursor = buffer.cursor_mut();
                        if cursor.col() > 0 {
                            cursor.move_left(1);
                        }
                        return;
                    }
                    InsertEntryMode::OpenAbove => {
                        // O — insert newline above, position on new line
                        // Replicate insert_line_above logic using CURRENT line's indent
                        if !changes.is_empty() {
                            let line_idx = buffer.cursor().line();
                            let line_text = buffer.line(line_idx).unwrap_or_default();
                            let indent: String = line_text
                                .chars()
                                .take_while(|c| c.is_whitespace() && *c != '\n')
                                .collect();

                            let text = format!("{}\n", indent);
                            buffer.insert_text_at(line_idx, 0, &text);
                            buffer.cursor_mut().set_position(line_idx, indent.chars().count());
                        }
                        // Replay remaining sub-changes (skip first which was the newline)
                        for change in changes.iter().skip(1) {
                            change.repeat(buffer);
                        }
                        let cursor = buffer.cursor_mut();
                        if cursor.col() > 0 {
                            cursor.move_left(1);
                        }
                        return;
                    }
                }

                // For non-o/O modes: replay all sub-changes at repositioned cursor
                for change in changes {
                    change.repeat(buffer);
                }
                // Move cursor back by 1 to match Esc behavior
                let cursor = buffer.cursor_mut();
                if cursor.col() > 0 {
                    cursor.move_left(1);
                }
            }
            Self::NumberOperation { delta, .. } => {
                // For dot-repeat, find number at current cursor and apply the same delta
                let line_idx = buffer.cursor().line();
                let col = buffer.cursor().col();

                if let Some(line) = buffer.line(line_idx) {
                    let line_text = line.trim_end_matches('\n');

                    if let Some((start_col, end_col, number_str)) =
                        find_number_at_or_after(line_text, col)
                    {
                        if let Ok((mut value, base, prefix_len)) = parse_number(&number_str) {
                            value += delta;
                            let new_number_str = format_number(value, base, prefix_len);

                            // Delete old number and insert new one
                            buffer.delete_range(line_idx, start_col, line_idx, end_col);
                            buffer.insert_text_at(line_idx, start_col, &new_number_str);

                            // Position cursor on last digit
                            let new_end_col = start_col + new_number_str.len() - 1;
                            buffer.cursor_mut().set_position(line_idx, new_end_col);
                        }
                    }
                }
            }
            Self::JoinLines { count, add_space, .. } => {
                // For dot-repeat, execute the join operation at current cursor
                let _ = if *add_space {
                    buffer.join_lines(*count)
                } else {
                    buffer.join_lines_no_space(*count)
                };
            }
            Self::ChangeTextObject { object_type, replacement, .. } => {
                // Re-evaluate the text object at current cursor and apply replacement
                if let Some(range) = Self::find_text_object(buffer, object_type) {
                    // Delete the text object content
                    buffer.delete_range(range.start_line, range.start_col, range.end_line, range.end_col);
                    // Insert replacement
                    buffer.insert_text_at(range.start_line, range.start_col, replacement);
                    // Position cursor at end of inserted text (minus 1 for normal mode)
                    let end_pos = Self::calculate_end_position((range.start_line, range.start_col), replacement);
                    let final_col = if end_pos.1 > 0 { end_pos.1 - 1 } else { 0 };
                    buffer.cursor_mut().set_position(end_pos.0, final_col);
                }
            }
            Self::DeleteTextObject { object_type, .. } => {
                // Re-evaluate the text object at current cursor and delete it
                if let Some(range) = Self::find_text_object(buffer, object_type) {
                    buffer.delete_range(range.start_line, range.start_col, range.end_line, range.end_col);
                    buffer.cursor_mut().set_position(range.start_line, range.start_col);
                }
            }
            Self::ChangeWord { replacement, .. } => {
                // Find word at current cursor and replace it
                if let Some(range) = TextObjects::inner_word(buffer) {
                    buffer.delete_range(range.start_line, range.start_col, range.end_line, range.end_col);
                    buffer.insert_text_at(range.start_line, range.start_col, replacement);
                    // Position cursor at end of inserted text (minus 1 for normal mode)
                    let end_pos = Self::calculate_end_position((range.start_line, range.start_col), replacement);
                    let final_col = if end_pos.1 > 0 { end_pos.1 - 1 } else { 0 };
                    buffer.cursor_mut().set_position(end_pos.0, final_col);
                }
            }
            Self::ReplaceMode { replacements, .. } => {
                // Replay the entire replacement sequence at current cursor
                let line_idx = buffer.cursor().line();
                let col = buffer.cursor().col();
                let replacement_len = replacements.chars().count();

                if let Some(line) = buffer.line(line_idx) {
                    let line_text = line.trim_end_matches('\n');
                    let line_len = line_text.chars().count();

                    // Calculate how much to delete (min of replacement length and remaining line)
                    let delete_len = replacement_len.min(line_len.saturating_sub(col));
                    let end_col = col + delete_len;

                    // Delete the characters that will be replaced
                    buffer.delete_range(line_idx, col, line_idx, end_col);
                    // Insert the replacement text
                    buffer.insert_text_at(line_idx, col, replacements);
                    // Position cursor at end of replacements (minus 1 for normal mode)
                    let final_col = col + replacement_len.saturating_sub(1);
                    buffer.cursor_mut().set_position(line_idx, final_col);
                }
            }
        }
    }

    /// Finds a text object range at current cursor position based on type
    fn find_text_object(buffer: &Buffer, object_type: &TextObjectType) -> Option<crate::editor::textobjects::TextObjectRange> {
        match object_type {
            TextObjectType::Word { inner } => {
                if *inner {
                    TextObjects::inner_word(buffer)
                } else {
                    TextObjects::around_word(buffer)
                }
            }
            TextObjectType::Quote { char, inner } => {
                TextObjects::quoted_string(buffer, *char, !*inner)
            }
            TextObjectType::Paired { open, close, inner } => {
                TextObjects::paired_delimiters(buffer, *open, *close, !*inner)
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
            Self::NumberOperation { .. } => String::new(),
            Self::JoinLines { .. } => String::new(),
            Self::ChangeTextObject { replacement, .. } => replacement.clone(),
            Self::DeleteTextObject { .. } => String::new(),
            Self::ChangeWord { replacement, .. } => replacement.clone(),
            Self::ReplaceMode { replacements, .. } => replacements.clone(),
        }
    }

    /// Gets the cursor position before this change
    pub fn cursor_before(&self) -> Position {
        match self {
            Self::InsertText { cursor_before, .. } => *cursor_before,
            Self::DeleteText { cursor_before, .. } => *cursor_before,
            Self::Composite { cursor_before, .. } => *cursor_before,
            Self::NumberOperation { cursor_before, .. } => *cursor_before,
            Self::JoinLines { cursor_before, .. } => *cursor_before,
            Self::ChangeTextObject { cursor_before, .. } => *cursor_before,
            Self::DeleteTextObject { cursor_before, .. } => *cursor_before,
            Self::ChangeWord { cursor_before, .. } => *cursor_before,
            Self::ReplaceMode { cursor_before, .. } => *cursor_before,
        }
    }

    /// Sets cursor_before on this change (used by repeat to record undo position).
    pub fn set_cursor_before(&mut self, pos: Position) {
        match self {
            Self::InsertText { cursor_before, .. } => *cursor_before = pos,
            Self::DeleteText { cursor_before, .. } => *cursor_before = pos,
            Self::Composite { cursor_before, .. } => *cursor_before = pos,
            Self::NumberOperation { cursor_before, .. } => *cursor_before = pos,
            Self::JoinLines { cursor_before, .. } => *cursor_before = pos,
            Self::ChangeTextObject { cursor_before, .. } => *cursor_before = pos,
            Self::DeleteTextObject { cursor_before, .. } => *cursor_before = pos,
            Self::ChangeWord { cursor_before, .. } => *cursor_before = pos,
            Self::ReplaceMode { cursor_before, .. } => *cursor_before = pos,
        }
    }

    /// Sets cursor_after on this change (used by repeat to record redo position).
    pub fn set_cursor_after(&mut self, pos: Position) {
        match self {
            Self::InsertText { .. } => { /* InsertText has no cursor_after field */ }
            Self::DeleteText { .. } => { /* DeleteText has no cursor_after field */ }
            Self::Composite { cursor_after, .. } => *cursor_after = pos,
            Self::NumberOperation { cursor_after, .. } => *cursor_after = pos,
            Self::JoinLines { cursor_after, .. } => *cursor_after = pos,
            Self::ChangeTextObject { cursor_after, .. } => *cursor_after = pos,
            Self::DeleteTextObject { cursor_after, .. } => *cursor_after = pos,
            Self::ChangeWord { cursor_after, .. } => *cursor_after = pos,
            Self::ReplaceMode { cursor_after, .. } => *cursor_after = pos,
        }
    }
}

/// Builder for accumulating changes during insert mode
#[derive(Debug)]
pub struct ChangeBuilder {
    changes: Vec<Change>,
    cursor_before: Position,
    cursor_after: Option<Position>,
    entry_mode: InsertEntryMode,
}

impl ChangeBuilder {
    pub fn new(cursor_before: Position) -> Self {
        Self {
            changes: Vec::new(),
            cursor_before,
            cursor_after: None,
            entry_mode: InsertEntryMode::Insert,
        }
    }

    /// Sets how insert mode was entered (for dot repeat cursor positioning).
    pub fn set_entry_mode(&mut self, mode: InsertEntryMode) {
        self.entry_mode = mode;
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
        } else if self.changes.len() == 1
            && !matches!(self.entry_mode, InsertEntryMode::OpenBelow | InsertEntryMode::OpenAbove)
        {
            // Unwrap single changes for most entry modes. OpenBelow/OpenAbove need
            // the Composite wrapper to preserve entry_mode for dot-repeat.
            Some(self.changes.into_iter().next().unwrap())
        } else {
            // Use explicitly set cursor_after, or fall back to current buffer cursor
            let cursor_after = self.cursor_after.unwrap_or(buffer_cursor);
            Some(Change::Composite {
                changes: self.changes,
                cursor_before: self.cursor_before,
                cursor_after,
                entry_mode: self.entry_mode,
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

impl Default for ChangeManager {
    fn default() -> Self {
        Self::new()
    }
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

    /// Sets the entry mode on the current builder (for dot repeat cursor positioning).
    pub fn set_entry_mode(&mut self, mode: InsertEntryMode) {
        if let Some(builder) = &mut self.current_builder {
            builder.set_entry_mode(mode);
        }
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
            let mut repeated_change = change.clone();
            // Store the current cursor as this repeat's cursor_before so undo
            // returns to the right place (not the original change's position).
            let current_pos = (buffer.cursor().line(), buffer.cursor().col());
            repeated_change.set_cursor_before(current_pos);
            repeated_change.repeat(buffer);
            // Record cursor_after for redo
            let after_pos = (buffer.cursor().line(), buffer.cursor().col());
            repeated_change.set_cursor_after(after_pos);
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

    /// Pops the last change from the undo stack (without applying undo)
    /// Used when replacing a change with a composite version
    pub fn pop_last_change(&mut self) -> Option<Change> {
        self.undo_stack.pop()
    }
}

// ==============================================================================
// Number Operation Helper Functions
// ==============================================================================

/// Finds a number at or after the given column position
/// Returns (start_col, end_col, number_string)
pub fn find_number_at_or_after(line: &str, col: usize) -> Option<(usize, usize, String)> {
    let chars: Vec<char> = line.chars().collect();

    if chars.is_empty() {
        return None;
    }

    // First, check if we're currently inside a number by searching backward
    let cursor_col = col.min(chars.len().saturating_sub(1));

    // If we're on a digit, search backward to find the start of the number
    if cursor_col < chars.len() && chars[cursor_col].is_ascii_digit() {
        let mut start_col = cursor_col;

        // Search backward to find the start of the number
        while start_col > 0 {
            let prev_ch = chars[start_col - 1];
            if prev_ch.is_ascii_digit() {
                start_col -= 1;
            } else if prev_ch == '-' || prev_ch == '+' {
                // Check if this sign is part of the number
                if start_col > 1
                    && !chars[start_col - 2].is_whitespace()
                    && chars[start_col - 2] != '('
                    && chars[start_col - 2] != '['
                {
                    // Not a sign, just adjacent character
                    break;
                }
                start_col -= 1;
                break;
            } else if start_col >= 2 && prev_ch == 'x' && chars[start_col - 2] == '0' {
                // Hex prefix
                start_col -= 2;
                break;
            } else if start_col >= 2
                && (prev_ch == 'b' || prev_ch == 'o')
                && chars[start_col - 2] == '0'
            {
                // Binary or octal prefix
                start_col -= 2;
                break;
            } else {
                break;
            }
        }

        // Now find the end of the number
        let mut end_col = cursor_col + 1;
        while end_col < chars.len() && chars[end_col].is_ascii_digit() {
            end_col += 1;
        }

        let number_str: String = chars[start_col..end_col].iter().collect();
        return Some((start_col, end_col, number_str));
    }

    // Not on a digit, so search backward first, then forward
    // This matches Vim behavior: search backward on current line, then forward

    // Try searching backward from cursor
    if cursor_col > 0 {
        let mut back_col = cursor_col;
        while back_col > 0 {
            back_col -= 1;
            if chars[back_col].is_ascii_digit() {
                // Found a digit backward, now find the start and end of this number
                let mut start_col = back_col;
                while start_col > 0 {
                    let prev_ch = chars[start_col - 1];
                    if prev_ch.is_ascii_digit() {
                        start_col -= 1;
                    } else if prev_ch == '-' || prev_ch == '+' {
                        if start_col > 1
                            && !chars[start_col - 2].is_whitespace()
                            && chars[start_col - 2] != '('
                            && chars[start_col - 2] != '['
                        {
                            break;
                        }
                        start_col -= 1;
                        break;
                    } else if start_col >= 2
                        && (prev_ch == 'x' || prev_ch == 'b' || prev_ch == 'o')
                        && chars[start_col - 2] == '0'
                    {
                        start_col -= 2;
                        break;
                    } else {
                        break;
                    }
                }

                let mut end_col = back_col + 1;
                while end_col < chars.len() && chars[end_col].is_ascii_digit() {
                    end_col += 1;
                }

                let number_str: String = chars[start_col..end_col].iter().collect();
                return Some((start_col, end_col, number_str));
            }
        }
    }

    // No number found backward, search forward from cursor position
    let mut search_col = col;

    // Skip non-digit/non-hex characters to find start of number
    while search_col < chars.len()
        && !chars[search_col].is_ascii_digit()
        && chars[search_col] != '-'
        && chars[search_col] != '+'
    {
        search_col += 1;
    }

    if search_col >= chars.len() {
        return None;
    }

    let start_col = search_col;
    let mut end_col = start_col;

    // Check for hex (0x), binary (0b), or octal (0o) prefix
    if chars[end_col] == '0' && end_col + 1 < chars.len() {
        let next = chars[end_col + 1];
        if next == 'x' || next == 'X' || next == 'b' || next == 'B' || next == 'o' || next == 'O' {
            end_col += 2;

            // Collect hex/binary/octal digits
            let is_hex = next == 'x' || next == 'X';
            let is_binary = next == 'b' || next == 'B';

            while end_col < chars.len() {
                let ch = chars[end_col];
                let valid_digit = (is_hex && ch.is_ascii_hexdigit())
                    || (is_binary && (ch == '0' || ch == '1'))
                    || (!is_hex && !is_binary && ch.is_ascii_digit());
                if valid_digit {
                    end_col += 1;
                } else {
                    break;
                }
            }

            if end_col > start_col + 2 {
                let number_str: String = chars[start_col..end_col].iter().collect();
                return Some((start_col, end_col, number_str));
            }
        }
    }

    // Regular decimal number (may have sign)
    end_col = start_col;

    // Skip optional sign
    if end_col < chars.len() && (chars[end_col] == '-' || chars[end_col] == '+') {
        end_col += 1;
    }

    // Collect digits
    while end_col < chars.len() && chars[end_col].is_ascii_digit() {
        end_col += 1;
    }

    if end_col > start_col {
        let number_str: String = chars[start_col..end_col].iter().collect();
        Some((start_col, end_col, number_str))
    } else {
        None
    }
}

/// Parses a number string, detecting the base from prefix
/// Returns (value, base, prefix_length)
pub fn parse_number(s: &str) -> Result<(i64, u32, usize)> {
    if s.len() >= 3 {
        let prefix = &s[0..2];
        let digits = &s[2..];

        match prefix {
            "0x" | "0X" => {
                let value = i64::from_str_radix(digits, 16).unwrap_or(0);
                return Ok((value, 16, 2));
            }
            "0b" | "0B" => {
                let value = i64::from_str_radix(digits, 2).unwrap_or(0);
                return Ok((value, 2, 2));
            }
            "0o" | "0O" => {
                let value = i64::from_str_radix(digits, 8).unwrap_or(0);
                return Ok((value, 8, 2));
            }
            _ => {}
        }
    }

    // Regular decimal
    let value = s.parse::<i64>().unwrap_or(0);
    Ok((value, 10, 0))
}

/// Formats a number with the given base
pub fn format_number(value: i64, base: u32, prefix_len: usize) -> String {
    match base {
        16 => {
            if prefix_len > 0 {
                format!("0x{:x}", value)
            } else {
                format!("{:x}", value)
            }
        }
        2 => {
            if prefix_len > 0 {
                format!("0b{:b}", value)
            } else {
                format!("{:b}", value)
            }
        }
        8 => {
            if prefix_len > 0 {
                format!("0o{:o}", value)
            } else {
                format!("{:o}", value)
            }
        }
        _ => format!("{}", value),
    }
}
