//! # Undo/Repeat Architecture
//!
//! Two patterns coexist on the same undo/redo stacks. They are mutually
//! exclusive for dot-repeat: `last_change` (Pattern A) and `last_repeat_action`
//! (Pattern B) clear each other. Dot-repeat checks RepeatAction first.
//!
//! ## Pattern A: Insert-mode keystroke batching (`InsertText`, `DeleteText`, `Composite`)
//!
//! **Now used exclusively for `i`/`a`/`I`/`A` insert-mode sessions.**
//!
//! `ChangeBuilder` accumulates individual `InsertText`/`DeleteText` changes
//! during an insert session, then `build()` wraps them in a `Composite` (or
//! unwraps a single change when entry_mode is plain `Insert`). This gives
//! per-keystroke undo granularity *within* the builder while the session is
//! active, and a single undo unit once finalized.
//!
//! Key asymmetry: `InsertText.apply()` uses line/col coordinates (via
//! `buffer.insert_text_at`), but `InsertText.undo()` converts to absolute
//! char offsets (via `buffer.delete_char_range`) because line/col clamping
//! can't address newline positions that were valid at insert time.
//!
//! `Composite.repeat(&mut self)` mutates in place — each sub-change's
//! `repeat()` updates its own position/deleted_text fields so that the
//! mutated `Composite` becomes a valid undo entry for the repeated edit.
//! The entry_mode field drives cursor repositioning (I→first non-blank,
//! A→end of line, a→right by 1) *before* replaying sub-changes.
//!
//! Note: `o`/`O` sessions start as Pattern A composites but are converted
//! to `RepeatAction::OpenLine` on insert-mode exit, so their dot-repeat
//! follows Pattern B. Change operators (`cc`, `cw`, etc.) similarly exit
//! insert mode with a `RepeatAction::Change`.
//!
//! ## Pattern B: Recorded Undo + RepeatAction (`record()` + `push_recorded_undo()`)
//!
//! **Default for all other operations.** Undo is mechanical (inverse the exact
//! `Edit`s in reverse). Repeat is semantic via `RepeatAction` at the current
//! cursor position.
//!
//! Operations: x, X, dd, D/d$, dw, dj, dk, d}, d{, dl,
//! di"/di(/diw/dip/dap/dis/das/dit/dii/dif (all text object deletes),
//! df/dt/dF/dT, ~, J, gJ, >>, <<, Ctrl-A/X, p, P,
//! cc/C/s/S/cw/cgn/text-object changes, R, o/O, visual-block change
//!
//! ## Pattern choice guide
//! - Does repeat need semantic intent at current cursor? → Pattern B
//! - Is this a direct insert-mode editing session (`i/a/A/I`) with
//!   per-keystroke batching? → Pattern A
//! - Is this an infrastructure push of an already-built change? → Pattern A
//!   (`add_change`)
//!
//! Also on the undo stack but outside the A/B repeat dichotomy:
//! - `Recorded` — mechanical undo from `buffer.record()` (Pattern B's undo half)
//! - `ResourceOp` — filesystem snapshots for LSP workspace operations (non-repeatable)
//!
//! ---
//!
//! ## Investigation: migrating insert-mode to Pattern B
//!
//! Could insert-mode sessions use `buffer.record()` to capture keystrokes as
//! `Edit`s, store the batch as `Change::Recorded`, and use a new
//! `RepeatAction::InsertSession { entry_mode, keystrokes }` for repeat?
//!
//! **Assessment: feasible, but non-trivial. Not a quick refactor.**
//!
//! ### How ChangeBuilder works today
//!
//! `ChangeBuilder` accumulates individual `Change::InsertText`/`DeleteText`
//! entries via `add()`. On `build()`, if there's exactly one change and
//! entry_mode is plain `Insert`, it unwraps the single change (avoiding a
//! Composite wrapper). Otherwise it wraps in `Composite` with `entry_mode`
//! and `cursor_before`/`cursor_after`. The builder is started when entering
//! insert mode (`start_change_building`) and finalized on exit
//! (`finalize_change_building`).
//!
//! ### How Composite.repeat() works
//!
//! `repeat(&mut self)` first repositions the cursor based on `entry_mode`
//! (I→first non-blank, A→end of line, a→right by 1, etc.), then iterates
//! `changes.iter_mut()` calling `repeat()` on each sub-change. Each
//! `InsertText.repeat()` mutates its own `position` field to the current
//! cursor, then applies. Each `DeleteText.repeat()` recalculates the
//! deletion range from the current cursor and mutates `range` and
//! `deleted_text` to match what was actually deleted. This mutation is
//! critical: the repeated `Composite` becomes a valid undo entry because
//! its sub-changes now reflect actual positions. Finally, cursor moves
//! left by 1 to simulate Esc.
//!
//! ### How insert mode creates changes
//!
//! In `helpers.rs`, `insert_char()`, `insert_newline()`,
//! `delete_char_before_cursor()`, etc. each create a `Change::InsertText`
//! or `Change::DeleteText` and call `editor.apply_change_and_record()`.
//! When a builder is active (insert-mode session), `add_change()` routes
//! to `builder.add()` instead of pushing directly to the undo stack.
//!
//! ### The entry_mode cursor positioning
//!
//! `Composite.repeat()` handles cursor repositioning before replay. A
//! `RepeatAction::InsertSession` could do the same — it just needs the
//! `InsertEntryMode` enum value and would reposition before replaying
//! keystrokes. This is straightforward.
//!
//! ### Per-keystroke undo within insert mode
//!
//! There is NO per-keystroke undo during an active insert session. The
//! builder accumulates changes, but they're not on the undo stack until
//! `finalize_building_at()` is called on Esc. Backspace during insert
//! mode is handled by `delete_char_before_cursor()` adding a
//! `DeleteText` to the builder — not by popping from undo. So the
//! builder's per-change granularity is only used for replay ordering,
//! not for mid-session undo.
//!
//! ### Migration path
//!
//! 1. Wrap the insert session in `buffer.record()` instead of using
//!    `ChangeBuilder`. Each `insert_char`/`insert_newline`/`delete_char`
//!    call would go through `buffer.insert_text_at()`/`buffer.delete_range()`
//!    directly (they already do — `Change.apply()` calls these).
//!    The `record()` closure would capture all `Edit`s.
//!
//! 2. On exit, push `Change::Recorded { edits, ... }` for undo.
//!
//! 3. For repeat, store `RepeatAction::InsertSession { entry_mode,
//!    keystrokes: Vec<KeyEvent> }`. Repeat would: reposition cursor per
//!    entry_mode, then replay each keystroke through the insert-mode
//!    handler (which re-derives indentation, completion, etc.).
//!
//! ### Tricky parts
//!
//! - **Keystroke replay vs. edit replay**: The current `Composite.repeat()`
//!   replays *edits* (insert "x" at position, delete range, etc.), not
//!   keystrokes. This is simpler but loses context (auto-indent on Enter
//!   bakes in the indent string). A `RepeatAction` replaying keystrokes
//!   would be more correct (re-derive indent for the new context) but
//!   requires capturing the raw `KeyEvent` sequence.
//!
//! - **buffer.record() scoping**: Currently `record()` takes a closure.
//!   An insert session spans many event-loop ticks. We'd need
//!   `buffer.start_recording()` / `buffer.stop_recording()` (a stateful
//!   recording mode) rather than the current closure-based API.
//!
//! - **Completion and snippets**: `accept_completion()` does multi-step
//!   edits (delete prefix, insert completion text). These currently
//!   produce `InsertText`/`DeleteText` changes. Under Pattern B they'd
//!   just be recorded edits, which is fine for undo but means the
//!   keystroke log needs a "completion accepted" marker for faithful
//!   replay.
//!
//! - **Visual block insert replay**: `exit_insert_mode()` replays the
//!   first line's changes on subsequent lines. This currently clones
//!   `Change` objects. Under Pattern B, it could replay the same
//!   keystrokes or the same edits (offset-adjusted) on each line.
//!
//! - **Whitespace cleanup**: `cleanup_whitespace_only_line()` adds a
//!   `DeleteText` to the builder before finalize. Under Pattern B this
//!   would just be another recorded edit — simpler.
//!
//! **Bottom line**: The migration is feasible and would unify the undo
//! model. The main prerequisite is a stateful recording API on Buffer
//! (start/stop instead of closure). The repeat side needs a keystroke
//! capture mechanism. Neither is architecturally risky, but it touches
//! insert mode, undo, repeat, completion, and visual block — so it
//! should be its own focused sprint, not a drive-by refactor.

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
    /// A composite of multiple changes (e.g., all changes during insert mode)
    Composite {
        changes: Vec<Change>,
        cursor_before: CursorPos,
        cursor_after: CursorPos,
        /// How insert mode was entered — tells dot repeat how to reposition.
        entry_mode: InsertEntryMode,
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

    /// Creates a Composite change
    pub fn composite(
        changes: Vec<Change>,
        cursor_before: CursorPos,
        cursor_after: CursorPos,
    ) -> Self {
        Self::Composite {
            changes,
            cursor_before,
            cursor_after,
            entry_mode: InsertEntryMode::Insert,
        }
    }

    /// Creates a Recorded change from raw buffer edits
    pub fn recorded(edits: Vec<Edit>, cursor_before: CursorPos, cursor_after: CursorPos) -> Self {
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
        cursor_before: CursorPos,
        cursor_after: CursorPos,
        group_id: u64,
    ) -> Self {
        Self::Recorded {
            edits,
            cursor_before,
            cursor_after,
            undo_group_id: Some(group_id),
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
                    .set_position(cursor_after.line, cursor_after.col);
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
                    .set_position(cursor_before.line, cursor_before.col);
                // Validate cursor position after composite undo - intermediate undos
                // may have deleted lines that the final cursor position refers to
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
                            buffer.cursor_mut().set_col(GraphemeCol(col));
                        }
                    }
                    InsertEntryMode::EndOfLine => {
                        // A — move to end of line
                        let line_idx = buffer.cursor().line();
                        if let Some(line) = buffer.line(line_idx) {
                            let line_len = line.trim_end_matches('\n').chars().count();
                            buffer.cursor_mut().set_col(GraphemeCol(line_len));
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
                if buffer.cursor_char_col() > 0 {
                    buffer.cursor_mut().move_left(1);
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
            Self::Composite { changes, .. } => {
                // Concatenate all inserted text from sub-changes
                let mut result = String::new();
                for change in changes {
                    result.push_str(&change.get_inserted_text());
                }
                result
            }
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
    /// For Composite changes (insert-mode sessions), this returns the first
    /// inner change's cursor_before — i.e., where the cursor was AFTER
    /// entry-mode repositioning (A, I, etc.) but before actual editing.
    /// Used by g; to navigate to the changelist position.
    pub fn edit_position(&self) -> CursorPos {
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
    pub fn cursor_before(&self) -> CursorPos {
        match self {
            Self::InsertText { cursor_before, .. } => *cursor_before,
            Self::DeleteText { cursor_before, .. } => *cursor_before,
            Self::Composite { cursor_before, .. } => *cursor_before,
            Self::Recorded { cursor_before, .. } => *cursor_before,
            Self::ResourceOp { cursor_before, .. } => *cursor_before,
        }
    }

    /// Gets the cursor position after this change.
    ///
    /// For `Composite`/`Recorded`/`ResourceOp` this is the stored grapheme-space
    /// cursor snapshot. For `InsertText`/`DeleteText` the value is derived from
    /// the char-space `position`/`range` by interpreting char indices as grapheme
    /// indices — this matches the legacy behavior (correct for ASCII, slightly
    /// wrong for multi-char graphemes) and is load-bearing for mark `'.` / `'^`.
    /// A future sprint should either store a real grapheme-space cursor on these
    /// variants or require `&Buffer` here to do a faithful conversion.
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
            Self::Composite { cursor_after, .. } => *cursor_after,
            Self::Recorded { cursor_after, .. } => *cursor_after,
            Self::ResourceOp { cursor_after, .. } => *cursor_after,
        }
    }

    /// Sets cursor_before on this change (used by repeat to record undo position).
    pub fn set_cursor_before(&mut self, pos: CursorPos) {
        match self {
            Self::InsertText { cursor_before, .. } => *cursor_before = pos,
            Self::DeleteText { cursor_before, .. } => *cursor_before = pos,
            Self::Composite { cursor_before, .. } => *cursor_before = pos,
            Self::Recorded { cursor_before, .. } => *cursor_before = pos,
            Self::ResourceOp { cursor_before, .. } => *cursor_before = pos,
        }
    }

    /// Sets cursor_after on this change (used by repeat to record redo position).
    pub fn set_cursor_after(&mut self, pos: CursorPos) {
        match self {
            Self::InsertText { .. } => { /* InsertText has no cursor_after field */ }
            Self::DeleteText { .. } => { /* DeleteText has no cursor_after field */ }
            Self::Composite { cursor_after, .. } => *cursor_after = pos,
            Self::Recorded { cursor_after, .. } => *cursor_after = pos,
            Self::ResourceOp { cursor_after, .. } => *cursor_after = pos,
        }
    }
}

/// Builder for accumulating changes during insert mode
#[derive(Debug)]
pub struct ChangeBuilder {
    changes: Vec<Change>,
    cursor_before: CursorPos,
    cursor_after: Option<CursorPos>,
    entry_mode: InsertEntryMode,
}

impl ChangeBuilder {
    pub fn new(cursor_before: CursorPos) -> Self {
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
    pub fn set_cursor_after(&mut self, cursor_after: CursorPos) {
        self.cursor_after = Some(cursor_after);
    }

    /// Finalizes the builder into a Change
    pub fn build(self, buffer_cursor: CursorPos) -> Option<Change> {
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
    pub fn finalize_building_at(&mut self, cursor_pos: CursorPos) {
        if let Some(builder) = self.current_builder.take() {
            if let Some(change) = builder.build(cursor_pos) {
                self.push_change(change);
            }
        }
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
