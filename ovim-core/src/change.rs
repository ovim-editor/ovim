//! # Undo/Repeat Architecture
//!
//! Two patterns coexist on the same undo/redo stacks:
//!
//! ## Pattern A: Semantic Change (`add_change()`)
//! Use for operations that:
//! - Enter insert mode (ci", cw, cgn, R, s, S, o, O, a, A, i, I, C)
//! - Need ChangeBuilder composition (insert-mode keystroke batching)
//! - Need semantic post-processing on insert-mode exit (ChangeTextObject, ChangeWord, ChangeSearchMatch)
//! - Are complex bracket-matching operations (d%)
//!
//! Variants: InsertText, DeleteText, Composite, ChangeTextObject, ChangeWord,
//! ChangeSearchMatch, ReplaceMode, Recorded
//!
//! ## Pattern B: Recorded Undo + RepeatAction (`record()` + `push_recorded_undo()`)
//! **Default for normal-mode operations.** Use when:
//! - Repeat should re-evaluate at the current cursor position
//! - No insert-mode entry needed
//! - Undo should be mechanical (inverse the exact edits)
//!
//! Operations: x, X, dd, D/d$, dw, dj, dk, d}, d{, dl,
//! di"/di(/diw/dip/dap/dis/das/dit/dii/dif (all text object deletes),
//! df/dt/dF/dT, ~, J, gJ, >>, <<, Ctrl-A/X, p, P
//!
//! ## Mutual Exclusion
//! `last_change` (Pattern A) and `last_repeat_action` (Pattern B) are mutually
//! exclusive. Setting one clears the other. Dot-repeat checks RepeatAction first.
//!
//! ## Pattern choice guide
//! - Does it enter insert mode? → Pattern A
//! - Does it need semantic re-evaluation on repeat (ci", cw, cgn)? → Pattern A
//! - Does it store a replacement sequence (R mode)? → Pattern A
//! - Everything else → Pattern B

use crate::buffer::Buffer;
use crate::edit::Edit;
use crate::editor::motions::Motions;
use crate::repeat_action::RepeatAction;
use crate::search::Search;
use crate::textobjects::TextObjects;
use anyhow::Result;

/// Position in the buffer (line, column)
pub type Position = (usize, usize);

/// Types of text objects for semantic repeat
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextObjectType {
    /// Inner/around word (iw/aw)
    Word { inner: bool, big: bool },
    /// Quoted string with specific quote char (i"/a", i'/a', i`/a`)
    Quote { char: char, inner: bool },
    /// Paired delimiters (i(/a(, i[/a[, i{/a{, i</a<)
    Paired {
        open: char,
        close: char,
        inner: bool,
    },
    /// Inner/around paragraph (ip/ap)
    Paragraph { inner: bool },
    /// Inner/around sentence (is/as)
    Sentence { inner: bool },
    /// Tag text object (it/at)
    Tag { inner: bool },
    /// Inner/around indent (ii/ai)
    Indent { inner: bool, tab_width: usize },
    /// Inner/around function (if/af)
    Function { inner: bool },
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
        /// True when the deletion went backward from cursor (e.g. X command).
        /// Used by repeat() to determine direction — stored explicitly so it
        /// doesn't depend on the value of cursor_before.
        backwards: bool,
    },
    /// A composite of multiple changes (e.g., all changes during insert mode)
    Composite {
        changes: Vec<Change>,
        cursor_before: Position,
        cursor_after: Position,
        /// How insert mode was entered — tells dot repeat how to reposition.
        entry_mode: InsertEntryMode,
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
    /// Semantic search match change (cgn) - stores the replacement and search pattern
    /// On repeat, finds the next search match and replaces it
    ChangeSearchMatch {
        search_pattern: String,
        search_forward: bool,
        replacement: String,
        cursor_before: Position,
        cursor_after: Position,
        // Store original for undo
        old_text: String,
        old_range: Range,
    },
    /// Undo record backed by raw edits (from buffer recording).
    /// Undo applies inverse edits in reverse; redo replays forward.
    Recorded {
        edits: Vec<Edit>,
        cursor_before: Position,
        cursor_after: Position,
        /// Optional group ID for undo grouping (e.g., agent turns).
        /// Multiple Recorded changes with the same group_id are undone together.
        undo_group_id: Option<u64>,
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

    /// Creates a forward DeleteText change
    pub fn delete(range: Range, deleted_text: String, cursor_before: Position) -> Self {
        Self::DeleteText {
            range,
            deleted_text,
            cursor_before,
            backwards: false,
        }
    }

    /// Creates a backward DeleteText change (e.g. X command)
    pub fn delete_backward(range: Range, deleted_text: String, cursor_before: Position) -> Self {
        Self::DeleteText {
            range,
            deleted_text,
            cursor_before,
            backwards: true,
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

    /// Creates a ChangeSearchMatch change (for cgn)
    pub fn change_search_match(
        search_pattern: String,
        search_forward: bool,
        replacement: String,
        cursor_before: Position,
        cursor_after: Position,
        old_text: String,
        old_range: Range,
    ) -> Self {
        Self::ChangeSearchMatch {
            search_pattern,
            search_forward,
            replacement,
            cursor_before,
            cursor_after,
            old_text,
            old_range,
        }
    }

    /// Creates a Recorded change from raw buffer edits
    pub fn recorded(edits: Vec<Edit>, cursor_before: Position, cursor_after: Position) -> Self {
        Self::Recorded {
            edits,
            cursor_before,
            cursor_after,
            undo_group_id: None,
        }
    }

    /// Creates a Recorded change with an undo group ID for grouped undo.
    pub fn recorded_grouped(
        edits: Vec<Edit>,
        cursor_before: Position,
        cursor_after: Position,
        group_id: u64,
    ) -> Self {
        Self::Recorded {
            edits,
            cursor_before,
            cursor_after,
            undo_group_id: Some(group_id),
        }
    }

    /// Returns the undo group ID if this is a grouped Recorded change.
    pub fn undo_group_id(&self) -> Option<u64> {
        match self {
            Self::Recorded { undo_group_id, .. } => *undo_group_id,
            _ => None,
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
                let version_before = buffer.version();
                buffer.insert_text_at(line, col, text);
                // Keep cursor stable when insertion was blocked/no-op.
                if buffer.version() != version_before {
                    // Update cursor to end of inserted text
                    let end_pos = Self::calculate_end_position(*position, text);
                    buffer.cursor_mut().set_position(end_pos.0, end_pos.1);
                }
            }
            Self::DeleteText { range, .. } => {
                let (start_line, start_col) = range.start;
                let (end_line, end_col) = range.end;
                let version_before = buffer.version();
                buffer.delete_range(start_line, start_col, end_line, end_col);
                // Keep cursor stable when deletion was blocked/no-op.
                if buffer.version() != version_before {
                    // Position cursor at deletion start
                    buffer.cursor_mut().set_position(start_line, start_col);
                }
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
                buffer
                    .cursor_mut()
                    .set_position(cursor_after.0, cursor_after.1);
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
                buffer
                    .cursor_mut()
                    .set_position(cursor_after.0, cursor_after.1);
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
                buffer
                    .cursor_mut()
                    .set_position(cursor_after.0, cursor_after.1);
            }
            Self::ChangeSearchMatch {
                old_range,
                replacement,
                cursor_after,
                ..
            } => {
                // Delete old match and insert replacement
                let (start_line, start_col) = old_range.start;
                let (end_line, end_col) = old_range.end;
                buffer.delete_range(start_line, start_col, end_line, end_col);
                buffer.insert_text_at(start_line, start_col, replacement);
                buffer
                    .cursor_mut()
                    .set_position(cursor_after.0, cursor_after.1);
            }
            Self::Recorded {
                edits,
                cursor_after,
                ..
            } => {
                for edit in edits {
                    edit.apply(buffer);
                }
                buffer
                    .cursor_mut()
                    .set_position(cursor_after.0, cursor_after.1);
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
                ..
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
                let replacement_end =
                    Self::calculate_end_position((start_line, start_col), replacement);
                buffer.delete_range(start_line, start_col, replacement_end.0, replacement_end.1);
                buffer.insert_text_at(start_line, start_col, old_text);
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.0, cursor_before.1);
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
                let replacement_end =
                    Self::calculate_end_position((start_line, start_col), replacement);
                buffer.delete_range(start_line, start_col, replacement_end.0, replacement_end.1);
                buffer.insert_text_at(start_line, start_col, old_text);
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.0, cursor_before.1);
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
                let replacement_end =
                    Self::calculate_end_position((start_line, start_col), replacements);
                buffer.delete_range(start_line, start_col, replacement_end.0, replacement_end.1);
                buffer.insert_text_at(start_line, start_col, old_text);
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.0, cursor_before.1);
            }
            Self::ChangeSearchMatch {
                cursor_before,
                old_range,
                old_text,
                replacement,
                ..
            } => {
                // To undo: delete the replacement and restore old match text
                let (start_line, start_col) = old_range.start;
                let replacement_end =
                    Self::calculate_end_position((start_line, start_col), replacement);
                buffer.delete_range(start_line, start_col, replacement_end.0, replacement_end.1);
                buffer.insert_text_at(start_line, start_col, old_text);
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.0, cursor_before.1);
            }
            Self::Recorded {
                edits,
                cursor_before,
                ..
            } => {
                // Apply inverse edits in reverse order
                for edit in edits.iter().rev() {
                    edit.inverse().apply(buffer);
                }
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.0, cursor_before.1);
                buffer.validate_cursor_position();
            }
        }
    }

    /// Repeats this change at the current cursor position
    pub fn repeat(&mut self, buffer: &mut Buffer) {
        match self {
            Self::InsertText {
                text,
                position: self_pos,
                ..
            } => {
                // Insert the same text at current position
                let new_pos = (buffer.cursor().line(), buffer.cursor().col());
                // Update self so undo targets the new position, not the original
                *self_pos = new_pos;
                Self::InsertText {
                    position: new_pos,
                    text: text.clone(),
                    cursor_before: new_pos,
                }
                .apply(buffer);
            }
            Self::DeleteText {
                range,
                deleted_text,
                backwards,
                ..
            } => {
                // Apply the same deletion pattern from current position
                let cursor_pos = (buffer.cursor().line(), buffer.cursor().col());
                let offset_line = range.end.0 - range.start.0;
                let offset_col = if range.end.0 == range.start.0 {
                    range.end.1 - range.start.1
                } else {
                    range.end.1
                };

                let is_backwards = *backwards;

                let (start_line, start_col, end_line, end_col) = if is_backwards {
                    // For backwards deletion (X), treat current cursor as the END
                    // and calculate the start by going backwards
                    let new_start = if offset_line == 0 {
                        (cursor_pos.0, cursor_pos.1.saturating_sub(offset_col))
                    } else if cursor_pos.1 == 0 {
                        // Multi-line backwards deletion with cursor at col 0
                        // (e.g. backspace at col 0 joining lines via I<BS>)
                        let prev_line = cursor_pos.0.saturating_sub(offset_line);
                        let prev_line_len = buffer
                            .line(prev_line)
                            .map(|s| s.trim_end_matches('\n').chars().count())
                            .unwrap_or(0);
                        (prev_line, prev_line_len)
                    } else {
                        // Original was cross-line but cursor is mid-line now
                        // (e.g. i<BS> at col 0, then repeat at col 2).
                        // Constrain to same-line single-char delete — what BS
                        // would actually do at this cursor position.
                        (cursor_pos.0, cursor_pos.1.saturating_sub(1))
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

                let actual_deleted = buffer.delete_range(start_line, start_col, end_line, end_col);

                // Update range and deleted_text so undo reverses the actual
                // deletion, not the original one.
                *range = Range::new((start_line, start_col), (end_line, end_col));
                *deleted_text = actual_deleted;

                // Position cursor at the start of the deletion
                buffer.cursor_mut().set_position(start_line, start_col);
            }
            Self::Composite {
                changes,
                entry_mode,
                ..
            } => {
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
                            let col = content
                                .chars()
                                .position(|c| !c.is_whitespace())
                                .unwrap_or(0);
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
                    InsertEntryMode::OpenBelow | InsertEntryMode::OpenAbove => {}
                }

                // For open-line sessions (o/O), dot-repeat is handled by
                // RepeatAction::OpenLine in insert-mode exit.
                for change in changes.iter_mut() {
                    change.repeat(buffer);
                }
                // Move cursor back by 1 to match Esc behavior
                let cursor = buffer.cursor_mut();
                if cursor.col() > 0 {
                    cursor.move_left(1);
                }
            }
            Self::ChangeTextObject {
                object_type,
                replacement,
                ..
            } => {
                // Re-evaluate the text object at current cursor and apply replacement
                if let Some(range) = Self::find_text_object(buffer, object_type) {
                    // Delete the text object content
                    buffer.delete_range(
                        range.start_line,
                        range.start_col,
                        range.end_line,
                        range.end_col,
                    );
                    // Insert replacement
                    buffer.insert_text_at(range.start_line, range.start_col, replacement);
                    // Position cursor at end of inserted text (minus 1 for normal mode)
                    let end_pos = Self::calculate_end_position(
                        (range.start_line, range.start_col),
                        replacement,
                    );
                    let final_col = if end_pos.1 > 0 { end_pos.1 - 1 } else { 0 };
                    buffer.cursor_mut().set_position(end_pos.0, final_col);
                }
            }
            Self::ChangeWord { replacement, .. } => {
                // Replicate cw (ce) semantics: delete from cursor to word end, then insert
                let start_line = buffer.cursor().line();
                let start_col = buffer.cursor().col();

                // Move cursor to word end (ce motion)
                Motions::word_end_forward_prefer_current(buffer, 1);

                let end_line = buffer.cursor().line();
                let line_len = if let Some(line) = buffer.line(end_line) {
                    line.trim_end_matches('\n').chars().count()
                } else {
                    0
                };
                let end_col = (buffer.cursor().col() + 1).min(line_len);

                // Restore cursor to start and delete the range
                buffer.cursor_mut().set_position(start_line, start_col);
                buffer.delete_range(start_line, start_col, end_line, end_col);
                buffer.insert_text_at(start_line, start_col, replacement);

                // Position cursor at end of inserted text (minus 1 for normal mode)
                let end_pos = Self::calculate_end_position((start_line, start_col), replacement);
                let final_col = if end_pos.1 > 0 { end_pos.1 - 1 } else { 0 };
                buffer.cursor_mut().set_position(end_pos.0, final_col);
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
            Self::ChangeSearchMatch {
                search_pattern,
                search_forward,
                replacement,
                ..
            } => {
                // Find the next search match from current cursor and replace it
                let line_idx = buffer.cursor().line();
                let col = buffer.cursor().col();

                let mut search = Search::new_with_options(
                    search_pattern.clone(),
                    *search_forward,
                    true, // ignorecase
                    true, // smartcase
                );

                if let Some((match_line, match_col, match_text)) =
                    search.find_next(buffer, line_idx, col)
                {
                    let match_len = match_text.chars().count();
                    let match_end_col = match_col + match_len;

                    // Delete the match
                    buffer.delete_range(match_line, match_col, match_line, match_end_col);
                    // Insert replacement
                    buffer.insert_text_at(match_line, match_col, replacement);
                    // Position cursor at end of inserted text (minus 1 for normal mode)
                    let end_pos =
                        Self::calculate_end_position((match_line, match_col), replacement);
                    let final_col = if end_pos.1 > 0 { end_pos.1 - 1 } else { 0 };
                    buffer.cursor_mut().set_position(end_pos.0, final_col);
                }
            }
            Self::Recorded {
                edits,
                cursor_after,
                ..
            } => {
                // Re-execute by applying edits forward
                for edit in edits.iter() {
                    edit.apply(buffer);
                }
                buffer
                    .cursor_mut()
                    .set_position(cursor_after.0, cursor_after.1);
            }
        }
    }

    /// Finds a text object range at current cursor position based on type
    fn find_text_object(
        buffer: &Buffer,
        object_type: &TextObjectType,
    ) -> Option<crate::textobjects::TextObjectRange> {
        match object_type {
            TextObjectType::Word { inner, big } => match (*inner, *big) {
                (true, true) => TextObjects::inner_big_word(buffer),
                (true, false) => TextObjects::inner_word(buffer),
                (false, true) => TextObjects::around_big_word(buffer),
                (false, false) => TextObjects::around_word(buffer),
            },
            TextObjectType::Quote { char, inner } => {
                TextObjects::quoted_string(buffer, *char, !*inner)
            }
            TextObjectType::Paired { open, close, inner } => {
                TextObjects::paired_delimiters(buffer, *open, *close, !*inner)
            }
            TextObjectType::Paragraph { inner } => {
                if *inner {
                    TextObjects::inner_paragraph(buffer)
                } else {
                    TextObjects::around_paragraph(buffer)
                }
            }
            TextObjectType::Sentence { inner } => {
                if *inner {
                    TextObjects::inner_sentence(buffer)
                } else {
                    TextObjects::around_sentence(buffer)
                }
            }
            TextObjectType::Tag { inner } => TextObjects::tag(buffer, !*inner),
            TextObjectType::Indent { inner, tab_width } => {
                if *inner {
                    TextObjects::inner_indent(buffer, *tab_width)
                } else {
                    TextObjects::around_indent(buffer, *tab_width)
                }
            }
            TextObjectType::Function { inner } => {
                if *inner {
                    TextObjects::inner_function(buffer)
                } else {
                    TextObjects::around_function(buffer)
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
            Self::ChangeTextObject { replacement, .. } => replacement.clone(),
            Self::ChangeWord { replacement, .. } => replacement.clone(),
            Self::ReplaceMode { replacements, .. } => replacements.clone(),
            Self::ChangeSearchMatch { replacement, .. } => replacement.clone(),
            Self::Recorded { edits, .. } => {
                // Concatenate text from insert edits
                let mut result = String::new();
                for edit in edits {
                    if let Edit::Insert { text, .. } = edit {
                        result.push_str(text);
                    }
                }
                result
            }
        }
    }

    /// Gets the position where the actual edit occurred.
    /// For Composite changes (insert-mode sessions), this returns the first
    /// inner change's cursor_before — i.e., where the cursor was AFTER
    /// entry-mode repositioning (A, I, etc.) but before actual editing.
    /// Used by g; to navigate to the changelist position.
    pub fn edit_position(&self) -> Position {
        match self {
            Self::Composite {
                changes,
                cursor_before,
                ..
            } => changes
                .first()
                .map(|c| c.cursor_before())
                .unwrap_or(*cursor_before),
            Self::Recorded { cursor_before, .. } => *cursor_before,
            _ => self.cursor_before(),
        }
    }

    /// Gets the cursor position before this change
    pub fn cursor_before(&self) -> Position {
        match self {
            Self::InsertText { cursor_before, .. } => *cursor_before,
            Self::DeleteText { cursor_before, .. } => *cursor_before,
            Self::Composite { cursor_before, .. } => *cursor_before,
            Self::ChangeTextObject { cursor_before, .. } => *cursor_before,
            Self::ChangeWord { cursor_before, .. } => *cursor_before,
            Self::ReplaceMode { cursor_before, .. } => *cursor_before,
            Self::ChangeSearchMatch { cursor_before, .. } => *cursor_before,
            Self::Recorded { cursor_before, .. } => *cursor_before,
        }
    }

    /// Sets cursor_before on this change (used by repeat to record undo position).
    pub fn set_cursor_before(&mut self, pos: Position) {
        match self {
            Self::InsertText { cursor_before, .. } => *cursor_before = pos,
            Self::DeleteText { cursor_before, .. } => *cursor_before = pos,
            Self::Composite { cursor_before, .. } => *cursor_before = pos,
            Self::ChangeTextObject { cursor_before, .. } => *cursor_before = pos,
            Self::ChangeWord { cursor_before, .. } => *cursor_before = pos,
            Self::ReplaceMode { cursor_before, .. } => *cursor_before = pos,
            Self::ChangeSearchMatch { cursor_before, .. } => *cursor_before = pos,
            Self::Recorded { cursor_before, .. } => *cursor_before = pos,
        }
    }

    /// Sets cursor_after on this change (used by repeat to record redo position).
    pub fn set_cursor_after(&mut self, pos: Position) {
        match self {
            Self::InsertText { .. } => { /* InsertText has no cursor_after field */ }
            Self::DeleteText { .. } => { /* DeleteText has no cursor_after field */ }
            Self::Composite { cursor_after, .. } => *cursor_after = pos,
            Self::ChangeTextObject { cursor_after, .. } => *cursor_after = pos,
            Self::ChangeWord { cursor_after, .. } => *cursor_after = pos,
            Self::ReplaceMode { cursor_after, .. } => *cursor_after = pos,
            Self::ChangeSearchMatch { cursor_after, .. } => *cursor_after = pos,
            Self::Recorded { cursor_after, .. } => *cursor_after = pos,
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
        } else if self.changes.len() == 1 && matches!(self.entry_mode, InsertEntryMode::Insert) {
            // Only unwrap single changes when entry mode is plain Insert (i),
            // which doesn't reposition the cursor. All other entry modes (I, a,
            // A, o, O) need the Composite wrapper to preserve entry_mode so
            // dot-repeat repositions the cursor correctly.
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

/// Token returned by `push_change_returning_token`.
/// Stores the undo stack index at push time so `pop_by_token` can verify
/// that the expected change is still at the top of the stack.
#[derive(Debug, Clone, Copy)]
pub struct ChangeToken(usize);

/// Manages undo/redo history and change tracking
#[derive(Debug)]
pub struct ChangeManager {
    pub undo_stack: Vec<Change>,
    pub redo_stack: Vec<Change>,
    pub current_builder: Option<ChangeBuilder>,
    pub last_change: Option<Change>,
    /// Tracks the undo stack size at last save (None if never saved)
    pub save_point: Option<usize>,
    /// Last position where an edit occurred (for g; navigation)
    pub last_edit_position: Option<Position>,
    /// Changelist positions (older/newer navigation via g; / g,)
    pub change_list: Vec<Position>,
    /// Current index in changelist (None when empty)
    pub change_list_index: Option<usize>,
    /// Semantic repeat action for dot-repeat (mutually exclusive with last_change)
    pub last_repeat_action: Option<RepeatAction>,
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
            last_edit_position: None,
            change_list: Vec::new(),
            change_list_index: None,
            last_repeat_action: None,
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
    pub fn push_change(&mut self, change: Change) {
        self.note_edit_position(change.edit_position());
        self.undo_stack.push(change.clone());
        self.redo_stack.clear(); // Clear redo stack on new change
        self.last_change = Some(change);
        self.last_repeat_action = None; // Mutual exclusion: Change-based repeat wins
    }

    /// Records an edit position in the changelist and moves current index to newest.
    pub fn note_edit_position(&mut self, pos: Position) {
        self.last_edit_position = Some(pos);

        if let Some(idx) = self.change_list_index {
            if idx + 1 < self.change_list.len() {
                self.change_list.truncate(idx + 1);
            }
        }

        if self.change_list.last().copied() != Some(pos) {
            self.change_list.push(pos);
        }
        self.change_list_index = self.change_list.len().checked_sub(1);
    }

    /// Jump to an older entry in the changelist (g;).
    pub fn jump_change_older(&mut self, count: usize) -> Option<Position> {
        let len = self.change_list.len();
        if len == 0 {
            return None;
        }
        let idx = self.change_list_index.unwrap_or(len - 1);
        let next = idx.saturating_sub(count.max(1));
        self.change_list_index = Some(next);
        self.change_list.get(next).copied()
    }

    /// Jump to a newer entry in the changelist (g,).
    pub fn jump_change_newer(&mut self, count: usize) -> Option<Position> {
        let len = self.change_list.len();
        if len == 0 {
            return None;
        }
        let idx = self.change_list_index.unwrap_or(len - 1);
        let next = (idx + count.max(1)).min(len.saturating_sub(1));
        self.change_list_index = Some(next);
        self.change_list.get(next).copied()
    }

    /// Undoes the last change. If the change has an undo_group_id, keeps
    /// popping changes with the same group ID so one `u` undoes the whole group.
    pub fn undo(&mut self, buffer: &mut Buffer) -> bool {
        if let Some(change) = self.undo_stack.pop() {
            let group_id = change.undo_group_id();
            change.undo(buffer);
            self.redo_stack.push(change);

            // If this change was part of a group, undo all remaining changes in the group
            if let Some(gid) = group_id {
                while self.undo_stack.last().and_then(|c| c.undo_group_id()) == Some(gid) {
                    let grouped = self.undo_stack.pop().unwrap();
                    grouped.undo(buffer);
                    self.redo_stack.push(grouped);
                }
            }

            true
        } else {
            false
        }
    }

    /// Redoes the next change. If the change has an undo_group_id, keeps
    /// replaying changes with the same group ID so one redo restores the group.
    pub fn redo(&mut self, buffer: &mut Buffer) -> bool {
        if let Some(change) = self.redo_stack.pop() {
            let group_id = change.undo_group_id();
            change.apply(buffer);
            self.undo_stack.push(change);

            // If this change was part of a group, redo the rest of the group
            if let Some(gid) = group_id {
                while self.redo_stack.last().and_then(|c| c.undo_group_id()) == Some(gid) {
                    let grouped = self.redo_stack.pop().unwrap();
                    grouped.apply(buffer);
                    self.undo_stack.push(grouped);
                }
            }

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

    /// Pushes a change and returns a token that can be used with `pop_by_token`.
    pub fn push_change_returning_token(&mut self, change: Change) -> ChangeToken {
        let index = self.undo_stack.len();
        self.push_change(change);
        ChangeToken(index)
    }

    /// Pops a change only if the token matches the current stack top.
    /// Returns None if the token is stale (the expected change wasn't there).
    pub fn pop_by_token(&mut self, token: ChangeToken) -> Option<Change> {
        if !self.undo_stack.is_empty() && token.0 == self.undo_stack.len() - 1 {
            self.undo_stack.pop()
        } else {
            None
        }
    }
}

// ==============================================================================
// Number Operation Helper Functions
// ==============================================================================

/// Finds a number at or after the given column position.
/// Returns (start_col, end_col, number_string).
/// Handles cursor on hex digits (a-f) inside a 0x prefix number.
pub fn find_number_at_or_after(line: &str, col: usize) -> Option<(usize, usize, String)> {
    let chars: Vec<char> = line.chars().collect();

    if chars.is_empty() {
        return None;
    }

    // First, check if we're currently inside a number by searching backward
    let cursor_col = col.min(chars.len().saturating_sub(1));

    // Check if we're on a digit or hex digit that's part of a hex number
    let on_digit = cursor_col < chars.len() && chars[cursor_col].is_ascii_digit();
    let on_hex_digit = cursor_col < chars.len()
        && chars[cursor_col].is_ascii_hexdigit()
        && !chars[cursor_col].is_ascii_digit();

    // If we're on a hex digit (a-f/A-F), check if we're inside a hex number
    let in_hex_number = if on_hex_digit {
        let mut check = cursor_col;
        let mut found_hex = false;
        while check > 0 {
            let prev = chars[check - 1];
            if prev.is_ascii_hexdigit() || prev.is_ascii_digit() {
                check -= 1;
            } else if check >= 2 && (prev == 'x' || prev == 'X') && chars[check - 2] == '0' {
                found_hex = true;
                break;
            } else {
                break;
            }
        }
        found_hex
    } else {
        false
    };

    // If we're on a digit (or hex digit within a hex number), search backward to find the start
    if on_digit || in_hex_number {
        let mut start_col = cursor_col;

        while start_col > 0 {
            let prev_ch = chars[start_col - 1];
            if prev_ch.is_ascii_digit() {
                start_col -= 1;
            } else if in_hex_number && prev_ch.is_ascii_hexdigit() {
                // Only allow hex digits if we're in a hex number context
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
                && (prev_ch == 'x' || prev_ch == 'X')
                && chars[start_col - 2] == '0'
            {
                start_col -= 2;
                break;
            } else if start_col >= 2
                && (prev_ch == 'b' || prev_ch == 'o')
                && chars[start_col - 2] == '0'
            {
                start_col -= 2;
                break;
            } else {
                break;
            }
        }

        // Determine if this is a hex number (check for 0x prefix in collected range)
        let is_hex = start_col + 1 < chars.len()
            && chars[start_col] == '0'
            && (chars[start_col + 1] == 'x' || chars[start_col + 1] == 'X');

        let mut end_col = cursor_col + 1;
        while end_col < chars.len() {
            let ch = chars[end_col];
            if is_hex && ch.is_ascii_hexdigit() {
                end_col += 1;
            } else if ch.is_ascii_digit() {
                end_col += 1;
            } else {
                break;
            }
        }

        let number_str: String = chars[start_col..end_col].iter().collect();
        return Some((start_col, end_col, number_str));
    }

    // Not on a digit — search forward only (matches Vim behavior)
    let mut search_col = col;

    while search_col < chars.len() {
        let ch = chars[search_col];
        if ch.is_ascii_digit()
            || ch == '-'
            || ch == '+'
            || (search_col + 1 < chars.len()
                && ch == '0'
                && (chars[search_col + 1] == 'x'
                    || chars[search_col + 1] == 'X'
                    || chars[search_col + 1] == 'b'
                    || chars[search_col + 1] == 'B'
                    || chars[search_col + 1] == 'o'
                    || chars[search_col + 1] == 'O'))
        {
            break;
        }
        search_col += 1;
    }

    if search_col >= chars.len() {
        return None;
    }

    let mut start_col = search_col;

    // Check if we're on a sign, and if so, verify there's a digit after it
    if chars[start_col] == '-' || chars[start_col] == '+' {
        if start_col + 1 < chars.len() && chars[start_col + 1].is_ascii_digit() {
            // Keep the sign as part of the number
        } else {
            start_col += 1;
            if start_col >= chars.len() {
                return None;
            }
        }
    }
    let mut end_col = start_col;

    // Check for hex (0x), binary (0b), or octal (0o) prefix
    if chars[end_col] == '0' && end_col + 1 < chars.len() {
        let next = chars[end_col + 1];
        if next == 'x' || next == 'X' || next == 'b' || next == 'B' || next == 'o' || next == 'O' {
            end_col += 2;

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

    if end_col < chars.len() && (chars[end_col] == '-' || chars[end_col] == '+') {
        end_col += 1;
    }

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

/// Parses a number string, detecting the base from prefix.
/// Returns (value, base, prefix_length).
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

/// Formats a number with the given base.
/// Handles negative hex/bin/oct via sign + unsigned abs.
pub fn format_number(value: i64, base: u32, prefix_len: usize) -> String {
    match base {
        16 => {
            let abs = value.unsigned_abs();
            let sign = if value < 0 { "-" } else { "" };
            if prefix_len > 0 {
                format!("{sign}0x{abs:x}")
            } else {
                format!("{sign}{abs:x}")
            }
        }
        2 => {
            let abs = value.unsigned_abs();
            let sign = if value < 0 { "-" } else { "" };
            if prefix_len > 0 {
                format!("{sign}0b{abs:b}")
            } else {
                format!("{sign}{abs:b}")
            }
        }
        8 => {
            let abs = value.unsigned_abs();
            let sign = if value < 0 { "-" } else { "" };
            if prefix_len > 0 {
                format!("{sign}0o{abs:o}")
            } else {
                format!("{sign}{abs:o}")
            }
        }
        _ => format!("{}", value),
    }
}
