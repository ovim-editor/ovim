//! # Undo/Repeat Architecture
//!
//! Undo entries are `Change::Recorded` — a `Vec<Edit>` of mechanical buffer
//! mutations plus the cursor positions to restore on undo / redo. Insert
//! sessions, normal-mode operators, LSP edits, and the like all land here.
//! `ResourceOp` carries filesystem snapshots for workspace LSP operations
//! and is non-repeatable.
//!
//! Dot-repeat goes through `RepeatAction` (see `repeat_action.rs`), not
//! through `Change`. `last_change` and `last_repeat_action` are mutually
//! exclusive: pushing a `Change` clears `last_repeat_action`, setting a
//! `RepeatAction` clears `last_change`. Dot-repeat checks `RepeatAction`
//! first and falls back to replaying the recorded edits forward.
//!
//! Insert-mode sessions (`i` / `a` / `I` / `A` / `o` / `O`) open a
//! `ChangeBuilder` to remember `entry_mode` + `cursor_before` across event
//! loop ticks. The actual edits flow into the buffer's stateful recording,
//! and `finalize_change_building` packages everything into a single
//! `Recorded` plus a `RepeatAction::InsertSession` for dot-repeat. The
//! `InsertText` / `DeleteText` variants below survive only as transient
//! cursor-positioning wrappers used by `apply_change_and_record`; they
//! never reach the undo stack from insert sessions.

use crate::buffer::Buffer;
use crate::edit::Edit;
use crate::repeat_action::RepeatAction;
use crate::textobjects::{TextObjectRange, TextObjects};
use crate::unicode::{CharCol, GraphemeCol};
use std::path::{Path, PathBuf};

/// A cursor snapshot: where the cursor sits in grapheme-space.
///
/// Cursor indices throughout ovim are grapheme-space (what users perceive as
/// characters). `cursor_before`/`cursor_after` fields on `Change` store this
/// type so it is never confused with the char-space positions where a
/// `Change` applies to the rope. See `ApplyPos` for that counterpart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CursorPos {
    pub line: usize,
    pub col: GraphemeCol,
}

impl CursorPos {
    pub const ZERO: CursorPos = CursorPos {
        line: 0,
        col: GraphemeCol::ZERO,
    };

    #[inline]
    pub fn new(line: usize, col: GraphemeCol) -> Self {
        Self { line, col }
    }
}

/// Where a `Change` applies to the rope: char-space.
///
/// Rope operations (`insert_text_at`, `delete_range`) expect char indices.
/// `InsertText.position` and the endpoints of `Range` on `DeleteText` store
/// this type so callers can't silently feed grapheme indices into the rope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplyPos {
    pub line: usize,
    pub col: CharCol,
}

impl ApplyPos {
    pub const ZERO: ApplyPos = ApplyPos {
        line: 0,
        col: CharCol::ZERO,
    };

    #[inline]
    pub fn new(line: usize, col: CharCol) -> Self {
        Self { line, col }
    }
}

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

impl TextObjectType {
    /// Resolve this text object to a range at the current cursor position.
    pub fn resolve(&self, buffer: &Buffer) -> Option<TextObjectRange> {
        match self {
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
}

/// Range in the buffer
/// A filesystem snapshot for undo/redo of LSP resource operations.
///
/// Captures file contents before and after an operation so that
/// undo can restore `before` and redo can restore `after`.
#[derive(Clone, Debug)]
pub struct ResourceSnapshot {
    pub path: PathBuf,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Range {
    pub start: ApplyPos,
    pub end: ApplyPos,
}

impl Range {
    pub fn new(start: ApplyPos, end: ApplyPos) -> Self {
        Self { start, end }
    }

    /// Creates a range for a single position (empty range)
    pub fn at(position: ApplyPos) -> Self {
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
        position: ApplyPos,
        text: String,
        cursor_before: CursorPos,
    },
    /// Delete text in a range
    DeleteText {
        range: Range,
        deleted_text: String, // Stored for undo
        cursor_before: CursorPos,
        /// True when the deletion went backward from cursor (e.g. X command).
        /// Used by repeat() to determine direction — stored explicitly so it
        /// doesn't depend on the value of cursor_before.
        backwards: bool,
    },
    /// Undo record backed by raw edits (from buffer recording).
    /// Undo applies inverse edits in reverse; redo replays forward.
    Recorded {
        edits: Vec<Edit>,
        cursor_before: CursorPos,
        cursor_after: CursorPos,
        /// Optional group ID for undo grouping (e.g., agent turns).
        /// Multiple Recorded changes with the same group_id are undone together.
        undo_group_id: Option<u64>,
        /// Optional override for `edit_position()`. Used by insert sessions
        /// where `cursor_before` is the pre-entry-mode cursor (for undo
        /// restore) but the actual edit landed at the post-entry-mode
        /// cursor (what `g;` should navigate to). `None` falls back to
        /// `cursor_before`.
        edit_start: Option<CursorPos>,
    },
    /// Filesystem snapshots for LSP workspace `ResourceOp` (create/rename/delete).
    /// Undo restores `before` bytes; redo applies `after` bytes.
    ResourceOp {
        snapshots: Vec<ResourceSnapshot>,
        cursor_before: CursorPos,
        cursor_after: CursorPos,
    },
}

impl Change {
    /// Creates an InsertText change
    pub fn insert(position: ApplyPos, text: String, cursor_before: CursorPos) -> Self {
        Self::InsertText {
            position,
            text,
            cursor_before,
        }
    }

    /// Creates a forward DeleteText change
    pub fn delete(range: Range, deleted_text: String, cursor_before: CursorPos) -> Self {
        Self::DeleteText {
            range,
            deleted_text,
            cursor_before,
            backwards: false,
        }
    }

    /// Creates a backward DeleteText change (e.g. X command)
    pub fn delete_backward(range: Range, deleted_text: String, cursor_before: CursorPos) -> Self {
        Self::DeleteText {
            range,
            deleted_text,
            cursor_before,
            backwards: true,
        }
    }

    /// Creates a Recorded change from raw buffer edits
    pub fn recorded(edits: Vec<Edit>, cursor_before: CursorPos, cursor_after: CursorPos) -> Self {
        Self::Recorded {
            edits,
            cursor_before,
            cursor_after,
            undo_group_id: None,
            edit_start: None,
        }
    }

    /// Creates a Recorded change with an explicit edit-start override used
    /// by `edit_position()`. `cursor_before` still governs undo cursor
    /// restore; `edit_start` is where `g;` / the changelist should land.
    pub fn recorded_with_edit_start(
        edits: Vec<Edit>,
        cursor_before: CursorPos,
        cursor_after: CursorPos,
        edit_start: CursorPos,
    ) -> Self {
        Self::Recorded {
            edits,
            cursor_before,
            cursor_after,
            undo_group_id: None,
            edit_start: Some(edit_start),
        }
    }

    /// Creates a Recorded change with an undo group ID for grouped undo.
    pub fn recorded_grouped(
        edits: Vec<Edit>,
        cursor_before: CursorPos,
        cursor_after: CursorPos,
        group_id: u64,
    ) -> Self {
        Self::Recorded {
            edits,
            cursor_before,
            cursor_after,
            undo_group_id: Some(group_id),
            edit_start: None,
        }
    }

    /// Creates a single resource snapshot entry.
    pub fn resource_snapshot(
        path: PathBuf,
        before: Option<Vec<u8>>,
        after: Option<Vec<u8>>,
    ) -> ResourceSnapshot {
        ResourceSnapshot {
            path,
            before,
            after,
        }
    }

    /// Creates a ResourceOp snapshot change (filesystem-only, no buffer edits).
    pub fn resource_op(
        snapshots: Vec<ResourceSnapshot>,
        cursor_before: CursorPos,
        cursor_after: CursorPos,
    ) -> Self {
        Self::ResourceOp {
            snapshots,
            cursor_before,
            cursor_after,
        }
    }

    /// Returns the undo group ID if this is a grouped Recorded change.
    pub fn undo_group_id(&self) -> Option<u64> {
        match self {
            Self::Recorded { undo_group_id, .. } => *undo_group_id,
            _ => None,
        }
    }

    /// Applies this change to the buffer
    pub fn apply(&self, buffer: &mut Buffer) {
        match self {
            Self::InsertText { position, text, .. } => {
                let version_before = buffer.version();
                buffer.insert_text_at(position.line, position.col, text);
                // Keep cursor stable when insertion was blocked/no-op.
                if buffer.version() != version_before {
                    // Update cursor to end of inserted text
                    // calculate_end_position returns a char-space ApplyPos,
                    // so use set_cursor_char_col which converts to grapheme.
                    let end_pos = Self::calculate_end_position(*position, text);
                    buffer.set_cursor_char_col(end_pos.line, end_pos.col);
                }
            }
            Self::DeleteText { range, .. } => {
                let version_before = buffer.version();
                buffer.delete_range(
                    range.start.line,
                    range.start.col,
                    range.end.line,
                    range.end.col,
                );
                // Keep cursor stable when deletion was blocked/no-op.
                if buffer.version() != version_before {
                    // Position cursor at deletion start.
                    // range.start is char-space; set_cursor_char_col converts to grapheme.
                    buffer.set_cursor_char_col(range.start.line, range.start.col);
                }
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
                    .set_position(cursor_after.line, cursor_after.col);
            }
            Self::ResourceOp {
                snapshots,
                cursor_after,
                ..
            } => {
                for snap in snapshots {
                    Self::restore_file_snapshot(&snap.path, &snap.after);
                }
                buffer
                    .cursor_mut()
                    .set_position(cursor_after.line, cursor_after.col);
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
                let start_char = if position.line < buffer.rope().len_lines() {
                    buffer.rope().line_to_char(position.line) + position.col.0
                } else {
                    buffer.rope().len_chars()
                };

                let text_len = text.chars().count();
                let end_char = (start_char + text_len).min(buffer.rope().len_chars());

                buffer.delete_char_range(start_char, end_char);
                // Restore cursor to where it was before the change
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.line, cursor_before.col);
                // Validate cursor position in case line no longer exists
                buffer.validate_cursor_position();
            }
            Self::DeleteText {
                range,
                deleted_text,
                cursor_before,
                ..
            } => {
                // To undo a delete, re-insert the deleted text.
                buffer.insert_text_at(range.start.line, range.start.col, deleted_text);
                // Restore cursor to where it was before the change
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.line, cursor_before.col);
                // Validate cursor position in case line no longer exists
                buffer.validate_cursor_position();
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
                    .set_position(cursor_before.line, cursor_before.col);
                buffer.validate_cursor_position();
            }
            Self::ResourceOp {
                snapshots,
                cursor_before,
                ..
            } => {
                for snap in snapshots.iter().rev() {
                    Self::restore_file_snapshot(&snap.path, &snap.before);
                }
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.line, cursor_before.col);
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
                // Insert the same text at current position.
                let new_pos = ApplyPos {
                    line: buffer.cursor().line(),
                    col: buffer.cursor_char_col(),
                };
                let new_cursor = CursorPos {
                    line: buffer.cursor().line(),
                    col: buffer.cursor().col(),
                };
                // Update self so undo targets the new position, not the original
                *self_pos = new_pos;
                Self::InsertText {
                    position: new_pos,
                    text: text.clone(),
                    cursor_before: new_cursor,
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
                let cursor_line = buffer.cursor().line();
                let cursor_col = buffer.cursor_char_col();
                let offset_line = range.end.line - range.start.line;
                let offset_col = if range.end.line == range.start.line {
                    range.end.col.0 - range.start.col.0
                } else {
                    range.end.col.0
                };

                let is_backwards = *backwards;

                let (start_line, start_col, end_line, end_col): (usize, CharCol, usize, CharCol) =
                    if is_backwards {
                        // For backwards deletion (X), treat current cursor as the END
                        // and calculate the start by going backwards
                        let new_start: (usize, CharCol) = if offset_line == 0 {
                            (cursor_line, cursor_col.saturating_sub(offset_col))
                        } else if cursor_col == CharCol::ZERO {
                            // Multi-line backwards deletion with cursor at col 0
                            // (e.g. backspace at col 0 joining lines via I<BS>)
                            let prev_line = cursor_line.saturating_sub(offset_line);
                            let prev_line_len = buffer
                                .line(prev_line)
                                .map(|s| s.trim_end_matches('\n').chars().count())
                                .unwrap_or(0);
                            (prev_line, CharCol(prev_line_len))
                        } else {
                            // Original was cross-line but cursor is mid-line now
                            // (e.g. i<BS> at col 0, then repeat at col 2).
                            // Constrain to same-line single-char delete — what BS
                            // would actually do at this cursor position.
                            (cursor_line, cursor_col.saturating_sub(1))
                        };
                        (new_start.0, new_start.1, cursor_line, cursor_col)
                    } else {
                        // For forward deletion (x, d, etc), treat current cursor as the START
                        let new_end: (usize, CharCol) = if offset_line == 0 {
                            (cursor_line, cursor_col + offset_col)
                        } else {
                            (cursor_line + offset_line, CharCol(offset_col))
                        };
                        (cursor_line, cursor_col, new_end.0, new_end.1)
                    };

                let actual_deleted = buffer.delete_range(start_line, start_col, end_line, end_col);

                // Update range and deleted_text so undo reverses the actual
                // deletion, not the original one.
                *range = Range::new(
                    ApplyPos::new(start_line, start_col),
                    ApplyPos::new(end_line, end_col),
                );
                *deleted_text = actual_deleted;

                // Position cursor at the start of the deletion
                buffer.set_cursor_char_col(start_line, start_col);
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
                    .set_position(cursor_after.line, cursor_after.col);
            }
            Self::ResourceOp { .. } => {
                // Intentionally non-repeatable via `.`.
            }
        }
    }

    pub(crate) fn snapshot_file(path: &Path) -> Option<Vec<u8>> {
        if !path.exists() || path.is_dir() {
            return None;
        }
        std::fs::read(path).ok()
    }

    fn restore_file_snapshot(path: &Path, snapshot: &Option<Vec<u8>>) {
        match snapshot {
            Some(bytes) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(path, bytes);
            }
            None => {
                if !path.exists() {
                    return;
                }
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(path);
                } else {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }

    /// Helper to calculate end position after inserting text.
    /// Both input and output are char-space (ApplyPos) — the counting iterates
    /// over chars, not graphemes.
    fn calculate_end_position(start: ApplyPos, text: &str) -> ApplyPos {
        let mut line = start.line;
        let mut col = start.col.0;

        for ch in text.chars() {
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }

        ApplyPos::new(line, CharCol(col))
    }

    /// Extracts the inserted text from this change (for the . register)
    pub fn get_inserted_text(&self) -> String {
        match self {
            Self::InsertText { text, .. } => text.clone(),
            Self::DeleteText { .. } => String::new(),
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
            Self::ResourceOp { .. } => String::new(),
        }
    }

    /// Gets the position where the actual edit occurred.
    ///
    /// For insert-mode `Recorded` sessions, `cursor_before` is the
    /// pre-entry-mode cursor (so undo lands there). The post-entry-mode
    /// cursor — where editing actually began — is stored in `edit_start`
    /// and is what `g;` / the changelist should navigate to.
    pub fn edit_position(&self) -> CursorPos {
        match self {
            Self::Recorded {
                cursor_before,
                edit_start,
                ..
            } => edit_start.unwrap_or(*cursor_before),
            _ => self.cursor_before(),
        }
    }

    /// Gets the cursor position before this change
    pub fn cursor_before(&self) -> CursorPos {
        match self {
            Self::InsertText { cursor_before, .. } => *cursor_before,
            Self::DeleteText { cursor_before, .. } => *cursor_before,
            Self::Recorded { cursor_before, .. } => *cursor_before,
            Self::ResourceOp { cursor_before, .. } => *cursor_before,
        }
    }

    /// Gets the cursor position after this change.
    ///
    /// For `Recorded` / `ResourceOp` this is the stored grapheme-space
    /// cursor snapshot. For `InsertText` / `DeleteText` the value is derived
    /// from the char-space `position` / `range` by interpreting char indices
    /// as grapheme indices — pre-existing legacy behavior (correct for
    /// ASCII, slightly wrong for multi-char graphemes) that is load-bearing
    /// for mark `'.` / `'^`.
    pub fn cursor_after(&self) -> CursorPos {
        match self {
            Self::InsertText { position, text, .. } => {
                let mut line = position.line;
                let mut col = position.col.0;
                for ch in text.chars() {
                    if ch == '\n' {
                        line += 1;
                        col = 0;
                    } else {
                        col += 1;
                    }
                }
                CursorPos::new(line, GraphemeCol(col.saturating_sub(1)))
            }
            Self::DeleteText { range, .. } => {
                // Char-index repurposed as grapheme-index (legacy behavior).
                CursorPos::new(range.start.line, GraphemeCol(range.start.col.0))
            }
            Self::Recorded { cursor_after, .. } => *cursor_after,
            Self::ResourceOp { cursor_after, .. } => *cursor_after,
        }
    }

    /// Sets cursor_before on this change (used by repeat to record undo position).
    pub fn set_cursor_before(&mut self, pos: CursorPos) {
        match self {
            Self::InsertText { cursor_before, .. } => *cursor_before = pos,
            Self::DeleteText { cursor_before, .. } => *cursor_before = pos,
            Self::Recorded { cursor_before, .. } => *cursor_before = pos,
            Self::ResourceOp { cursor_before, .. } => *cursor_before = pos,
        }
    }

    /// Consumes this change and returns its edit list when it is a
    /// `Recorded`, or `None` otherwise. Used by flows that need to merge a
    /// popped insert-session change's edits into a new Recorded (e.g.,
    /// pending_change_repeat, visual-block insert replay).
    pub fn into_edits(self) -> Option<Vec<Edit>> {
        match self {
            Self::Recorded { edits, .. } => Some(edits),
            _ => None,
        }
    }

    /// Sets cursor_after on this change (used by repeat to record redo position).
    pub fn set_cursor_after(&mut self, pos: CursorPos) {
        match self {
            Self::InsertText { .. } => { /* InsertText has no cursor_after field */ }
            Self::DeleteText { .. } => { /* DeleteText has no cursor_after field */ }
            Self::Recorded { cursor_after, .. } => *cursor_after = pos,
            Self::ResourceOp { cursor_after, .. } => *cursor_after = pos,
        }
    }
}

/// Tracks insert-session metadata across the event-loop ticks between
/// `start_change_building` and `finalize_change_building`. The actual edit
/// capture lives in `Buffer::recording`; this struct only carries the
/// pre-entry-mode cursor and how the session was entered.
#[derive(Debug)]
pub struct ChangeBuilder {
    cursor_before: CursorPos,
    entry_mode: InsertEntryMode,
}

impl ChangeBuilder {
    pub fn new(cursor_before: CursorPos) -> Self {
        Self {
            cursor_before,
            entry_mode: InsertEntryMode::Insert,
        }
    }

    /// Sets how insert mode was entered (for dot repeat cursor positioning).
    pub fn set_entry_mode(&mut self, mode: InsertEntryMode) {
        self.entry_mode = mode;
    }

    /// Returns the cursor position captured when the builder was opened —
    /// i.e., where to restore the cursor on undo of the insert session.
    pub fn cursor_before(&self) -> CursorPos {
        self.cursor_before
    }

    /// Returns how the insert session was entered. Used by dot-repeat
    /// (via `RepeatAction::InsertSession`) and by the o/O promotion check.
    pub fn entry_mode(&self) -> &InsertEntryMode {
        &self.entry_mode
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
    pub last_edit_position: Option<CursorPos>,
    /// Changelist positions (older/newer navigation via g; / g,)
    pub change_list: Vec<CursorPos>,
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
    pub fn start_building(&mut self, cursor_before: CursorPos) {
        self.current_builder = Some(ChangeBuilder::new(cursor_before));
    }

    /// Sets the entry mode on the current builder (for dot repeat cursor positioning).
    pub fn set_entry_mode(&mut self, mode: InsertEntryMode) {
        if let Some(builder) = &mut self.current_builder {
            builder.set_entry_mode(mode);
        }
    }

    /// Adds a change to the undo stack when no insert session is active.
    ///
    /// During an active insert session the buffer's recording captures the
    /// edits and `finalize_change_building` pushes a single
    /// `Change::Recorded` covering the whole session — so this method is a
    /// no-op while building. Direct (non-session) callers still land their
    /// change on the undo stack as `last_change`.
    pub fn add_change(&mut self, change: Change) {
        if self.current_builder.is_some() {
            return;
        }
        self.push_change(change);
    }

    /// Pushes a change to the undo stack
    pub fn push_change(&mut self, change: Change) {
        self.push_undo_change_preserving_repeat(change.clone());
        self.last_change = Some(change);
        self.last_repeat_action = None; // Mutual exclusion: Change-based repeat wins
    }

    /// Pushes an undo entry while preserving current dot-repeat templates.
    ///
    /// This is for non-repeat operations (LSP edits, replayed recorded undo, resource ops)
    /// that must be undoable without becoming the new `.` target.
    pub fn push_undo_change_preserving_repeat(&mut self, change: Change) {
        self.note_edit_position(change.edit_position());
        self.undo_stack.push(change);
        self.redo_stack.clear();
    }

    /// Records an edit position in the changelist and moves current index to newest.
    pub fn note_edit_position(&mut self, pos: CursorPos) {
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
    pub fn jump_change_older(&mut self, count: usize) -> Option<CursorPos> {
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
    pub fn jump_change_newer(&mut self, count: usize) -> Option<CursorPos> {
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
