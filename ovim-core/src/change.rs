//! # Undo/Repeat Architecture
//!
//! `Change` has two variants:
//!
//! - `Recorded` — a `Vec<Edit>` of mechanical buffer mutations plus the
//!   cursor positions to restore on undo / redo. Insert sessions,
//!   normal-mode operators, LSP edits, and direct-path buffer helpers all
//!   land here.
//! - `ResourceOp` — filesystem snapshots for workspace LSP operations; not
//!   repeatable via `.`.
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
//! `Recorded` plus a `RepeatAction::InsertSession` for dot-repeat.

use crate::buffer::Buffer;
use crate::edit::Edit;
use crate::repeat_action::RepeatAction;
use crate::textobjects::{TextObjectRange, TextObjects};
use crate::unicode::{CharCol, GraphemeCol};
use std::io;
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

    /// Applies this change to the buffer.
    ///
    /// Returns `Err` if a `ResourceOp` filesystem write/remove fails. On
    /// failure, processing aborts at the offending snapshot — partial undo
    /// is worse than no undo, and the cursor is not advanced. Callers should
    /// surface the error to the user (the change has not fully applied).
    pub fn apply(&self, buffer: &mut Buffer) -> io::Result<()> {
        match self {
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
                Ok(())
            }
            Self::ResourceOp {
                snapshots,
                cursor_after,
                ..
            } => {
                for snap in snapshots {
                    Self::restore_file_snapshot(&snap.path, &snap.after)?;
                }
                buffer
                    .cursor_mut()
                    .set_position(cursor_after.line, cursor_after.col);
                Ok(())
            }
        }
    }

    /// Undoes this change on the buffer.
    ///
    /// Returns `Err` if a `ResourceOp` filesystem write/remove fails. On
    /// failure, processing aborts at the offending snapshot and the cursor
    /// is not advanced. The change should be left on the undo stack so a
    /// subsequent `u` can retry once the filesystem error is resolved.
    pub fn undo(&self, buffer: &mut Buffer) -> io::Result<()> {
        match self {
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
                Ok(())
            }
            Self::ResourceOp {
                snapshots,
                cursor_before,
                ..
            } => {
                for snap in snapshots.iter().rev() {
                    Self::restore_file_snapshot(&snap.path, &snap.before)?;
                }
                buffer
                    .cursor_mut()
                    .set_position(cursor_before.line, cursor_before.col);
                buffer.validate_cursor_position();
                Ok(())
            }
        }
    }

    /// Repeats this change at the current cursor position
    pub fn repeat(&mut self, buffer: &mut Buffer) {
        match self {
            Self::Recorded {
                edits,
                cursor_after,
                ..
            } => {
                // Re-execute by applying edits forward.
                //
                // NB: `edits` store absolute char offsets, so replay lands at
                // the original position — not re-anchored to the current
                // cursor. Callers that need cursor re-anchoring (e.g., the
                // Normal-mode bracketed paste, normal-mode operators) set a
                // `RepeatAction` instead; `repeat_last_change` routes through
                // that path first and only falls back here for fallthrough.
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

    /// Restores a file to a snapshot's bytes (or removes it when the snapshot
    /// is `None`, representing "did not exist before"). Returns `Err` on any
    /// filesystem failure so callers can surface the error rather than
    /// silently corrupting undo state.
    ///
    /// `create_dir_all` errors are propagated only when they actually prevent
    /// the subsequent write — a missing-parent path returns `AlreadyExists`
    /// for the create call, which is folded into Ok by the standard library.
    fn restore_file_snapshot(path: &Path, snapshot: &Option<Vec<u8>>) -> io::Result<()> {
        match snapshot {
            Some(bytes) => {
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(path, bytes)?;
                Ok(())
            }
            None => {
                if !path.exists() {
                    return Ok(());
                }
                if path.is_dir() {
                    std::fs::remove_dir_all(path)?;
                } else {
                    std::fs::remove_file(path)?;
                }
                Ok(())
            }
        }
    }

    /// Extracts the inserted text from this change (for the . register)
    pub fn get_inserted_text(&self) -> String {
        match self {
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
            Self::Recorded { cursor_before, .. } => *cursor_before,
            Self::ResourceOp { cursor_before, .. } => *cursor_before,
        }
    }

    /// Gets the cursor position after this change — the stored grapheme-space
    /// cursor snapshot captured when the change was recorded.
    pub fn cursor_after(&self) -> CursorPos {
        match self {
            Self::Recorded { cursor_after, .. } => *cursor_after,
            Self::ResourceOp { cursor_after, .. } => *cursor_after,
        }
    }

    /// Sets cursor_before on this change (used by repeat to record undo position).
    pub fn set_cursor_before(&mut self, pos: CursorPos) {
        match self {
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

/// Outcome of `ChangeManager::undo` / `redo`.
///
/// Splits the previous `bool` return into three states so callers can
/// distinguish "stack was empty" from "filesystem error during a
/// `ResourceOp` snapshot restore" — silent failure on the latter was
/// OV-00212.
#[derive(Debug)]
pub enum UndoOutcome {
    /// The undo/redo stack was empty — nothing to do.
    Nothing,
    /// The change applied successfully.
    Done,
    /// A `ResourceOp` snapshot restore failed. The change has been left on
    /// the stack it was popped from so the caller can retry once the
    /// filesystem condition is resolved.
    Failed(io::Error),
}

impl UndoOutcome {
    /// True if anything actually changed (success or partial failure).
    /// Used by code paths that need to invalidate caches even on failure.
    pub fn touched_buffer(&self) -> bool {
        matches!(self, Self::Done | Self::Failed(_))
    }

    /// True only on full success. Equivalent to the old `bool` semantics
    /// for callers that don't need to distinguish failure modes.
    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done)
    }
}

/// Token returned by `push_change_returning_token` / `push_recorded_undo`.
/// Stores the undo stack index at push time so `pop_by_token` can verify
/// that the expected change is still at the top of the stack.
#[derive(Debug, Clone, Copy)]
pub struct ChangeToken(usize);

impl ChangeToken {
    /// Constructs a token from a raw undo-stack index. Exposed for the
    /// editor-level `push_recorded_undo` helper; most callers should get
    /// their token from `push_recorded_undo` / `push_change_returning_token`.
    pub fn from_index(index: usize) -> Self {
        Self(index)
    }
}

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
    ///
    /// On `ResourceOp` filesystem failure the offending change is restored
    /// to the top of the undo stack (without rotating it onto the redo
    /// stack), so the user can retry `u` once the filesystem condition is
    /// fixed. Earlier changes from the same group that already undid
    /// successfully *do* get rotated to redo — partial failure mid-group is
    /// represented honestly rather than papered over.
    pub fn undo(&mut self, buffer: &mut Buffer) -> UndoOutcome {
        let Some(change) = self.undo_stack.pop() else {
            return UndoOutcome::Nothing;
        };

        if let Err(err) = change.undo(buffer) {
            // Put it back so the next `u` can retry.
            self.undo_stack.push(change);
            return UndoOutcome::Failed(err);
        }
        let group_id = change.undo_group_id();
        self.redo_stack.push(change);

        // If this change was part of a group, undo all remaining changes in the group.
        if let Some(gid) = group_id {
            while self.undo_stack.last().and_then(|c| c.undo_group_id()) == Some(gid) {
                let grouped = self.undo_stack.pop().unwrap();
                if let Err(err) = grouped.undo(buffer) {
                    self.undo_stack.push(grouped);
                    return UndoOutcome::Failed(err);
                }
                self.redo_stack.push(grouped);
            }
        }

        UndoOutcome::Done
    }

    /// Redoes the next change. If the change has an undo_group_id, keeps
    /// replaying changes with the same group ID so one redo restores the group.
    ///
    /// On `ResourceOp` filesystem failure the offending change is restored
    /// to the top of the redo stack (so a retry is possible) and the
    /// outcome is `Failed`. See `undo` for the symmetric rationale.
    pub fn redo(&mut self, buffer: &mut Buffer) -> UndoOutcome {
        let Some(change) = self.redo_stack.pop() else {
            return UndoOutcome::Nothing;
        };

        if let Err(err) = change.apply(buffer) {
            self.redo_stack.push(change);
            return UndoOutcome::Failed(err);
        }
        let group_id = change.undo_group_id();
        self.undo_stack.push(change);

        // If this change was part of a group, redo the rest of the group.
        if let Some(gid) = group_id {
            while self.redo_stack.last().and_then(|c| c.undo_group_id()) == Some(gid) {
                let grouped = self.redo_stack.pop().unwrap();
                if let Err(err) = grouped.apply(buffer) {
                    self.redo_stack.push(grouped);
                    return UndoOutcome::Failed(err);
                }
                self.undo_stack.push(grouped);
            }
        }

        UndoOutcome::Done
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
