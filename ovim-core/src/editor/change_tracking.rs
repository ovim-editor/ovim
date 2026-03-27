//! Change tracking and undo/redo operations

use super::{Change, Editor, Position};
use crate::change::ChangeToken;
use crate::edit::Edit;
use crate::repeat_action::RepeatAction;

impl Editor {
    /// Records a buffer mutation with undo tracking and optional dot-repeat.
    ///
    /// Captures cursor_before/after, records edits via `buffer.record()`,
    /// pushes an undo entry, sets the repeat action, and marks the buffer
    /// modified for LSP sync. Returns the closure's result so callers can
    /// use it (e.g., deleted text for registers).
    pub fn record_operation<R>(
        &mut self,
        f: impl FnOnce(&mut crate::buffer::Buffer) -> R,
        repeat_action: Option<RepeatAction>,
    ) -> R {
        let cursor_before = self.cursor_position();
        let (result, edits) = self.buffer_mut().record(f);
        if !edits.is_empty() {
            let cursor_after = self.cursor_position();
            // push_recorded_undo() calls mark_buffer_modified() internally
            self.push_recorded_undo(edits, cursor_before, cursor_after);
            if let Some(action) = repeat_action {
                self.set_repeat_action(action);
            }
        }
        result
    }

    /// Pops the last change from the undo stack (without undoing it)
    /// Used when replacing a change with a composite version
    pub fn pop_last_change(&mut self) -> Option<Change> {
        self.buffer_mut().change_manager_mut().pop_last_change()
    }

    /// Undoes the last change
    pub fn undo(&mut self) {
        self.buffer_mut().undo();
        self.invalidate_hover_cache();
        self.mark_buffer_modified();
        self.mark_dirty();
    }

    /// Redoes the next change
    pub fn redo(&mut self) {
        self.buffer_mut().redo();
        self.invalidate_hover_cache();
        self.mark_buffer_modified();
        self.mark_dirty();
    }

    /// Repeats the last change with proper cursor position tracking.
    ///
    /// Records cursor_before/cursor_after so undo after dot-repeat restores
    /// the cursor to where the repeat happened, not the original change.
    /// Buffer mutations are captured via `record()` so the undo entry uses
    /// mechanical inverse edits rather than semantic replay.
    pub fn repeat_last_change(&mut self) {
        // Try RepeatAction first (semantic repeat for Pattern B operations)
        if let Some(action) = self.buffer().change_manager().last_repeat_action.clone() {
            // Paste repeat needs Editor-level access (registers), handle specially
            match &action {
                RepeatAction::PasteAfter { count } | RepeatAction::PasteBefore { count } => {
                    let count = *count;
                    let is_after = matches!(action, RepeatAction::PasteAfter { .. });
                    let _ = if is_after {
                        crate::editor::input::helpers::paste_after(self, count)
                    } else {
                        crate::editor::input::helpers::paste_before(self, count)
                    };
                    return;
                }
                _ => {}
            }

            let (before, after, edits) = {
                let buf = self.buffer_mut();
                let before = (buf.cursor().line(), buf.cursor().col());
                let ((), edits) = buf.record(|b| {
                    action.execute(b);
                });
                let after = (buf.cursor().line(), buf.cursor().col());
                (before, after, edits)
            };

            if !edits.is_empty() {
                self.push_recorded_undo(edits, before, after);
            }
            return;
        }

        // Fall back to Change-based repeat
        if let Some(mut repeated) = self.buffer().change_manager().last_change().cloned() {
            let (before, after, edits) = {
                let buf = self.buffer_mut();
                let before = (buf.cursor().line(), buf.cursor().col());

                // Record the repeat's buffer mutations for mechanical undo.
                let ((), edits) = buf.record(|b| {
                    // Call repeat() BEFORE set_cursor_before() — repeat() uses the
                    // original cursor_before to detect deletion direction (forward vs
                    // backward). It also updates range/deleted_text so undo works.
                    repeated.repeat(b);
                });

                let after = (buf.cursor().line(), buf.cursor().col());
                (before, after, edits)
            };

            if !edits.is_empty() {
                // Push recorded undo (mechanical) — single `u` undoes the whole repeat.
                self.push_recorded_undo(edits, before, after);

                // Update repeat template positions for next repeat.
                repeated.set_cursor_before(before);
                repeated.set_cursor_after(after);
                self.buffer_mut().change_manager_mut().last_change = Some(repeated);
            }
        }
    }

    /// Pushes a recorded undo entry without setting the repeat register.
    /// Use for compound operations (join, case change, indent) where the
    /// dot-repeat change is set separately.
    ///
    /// If an AI chat undo group is active, the change is stamped with the group ID
    /// so that `u` undoes the entire agent turn at once.
    pub fn push_recorded_undo(
        &mut self,
        edits: Vec<Edit>,
        cursor_before: Position,
        cursor_after: Position,
    ) {
        // Adjust decoration positions to follow the edits.
        // This keeps inlay hints at correct positions between the edit
        // and the next LSP response (~500ms), avoiding stale coordinates.
        for edit in &edits {
            self.adjust_decorations_for_edit(edit);
        }

        let group_id = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| c.current_undo_group);

        let change = if let Some(gid) = group_id {
            Change::recorded_grouped(edits, cursor_before, cursor_after, gid)
        } else {
            Change::recorded(edits, cursor_before, cursor_after)
        };
        let cm = self.buffer_mut().change_manager_mut();
        cm.push_undo_change_preserving_repeat(change);
        // Ensure LSP is notified of buffer changes — callers that use record()
        // directly instead of record_operation() were previously missing this.
        self.mark_buffer_modified();
    }

    /// Like `push_recorded_undo` but returns a `ChangeToken` that can later
    /// be redeemed with `pop_by_token` to safely retrieve this exact entry.
    pub fn push_recorded_undo_returning_token(
        &mut self,
        edits: Vec<Edit>,
        cursor_before: Position,
        cursor_after: Position,
    ) -> ChangeToken {
        let change = Change::recorded(edits, cursor_before, cursor_after);
        let token = self
            .buffer_mut()
            .change_manager_mut()
            .push_change_returning_token(change);
        self.mark_buffer_modified();
        token
    }

    /// Pops a change only if the token matches the current stack top.
    /// Returns None if the token is stale.
    pub fn pop_by_token(&mut self, token: ChangeToken) -> Option<Change> {
        self.buffer_mut().change_manager_mut().pop_by_token(token)
    }

    /// Sets a semantic repeat action for dot-repeat (mutually exclusive with last_change).
    pub fn set_repeat_action(&mut self, action: RepeatAction) {
        let cm = self.buffer_mut().change_manager_mut();
        cm.last_repeat_action = Some(action);
        cm.last_change = None; // Mutual exclusion: RepeatAction wins
    }

    /// Returns the current cursor position as (line, col).
    pub fn cursor_position(&self) -> Position {
        (self.buffer().cursor().line(), self.buffer().cursor().col())
    }

    /// Returns the last position where an edit occurred (for g; navigation).
    pub fn last_edit_position(&self) -> Option<Position> {
        self.buffer().change_manager().last_edit_position
    }

    /// Jump to older changelist position (g;).
    pub fn jump_change_older(&mut self, count: usize) -> Option<Position> {
        self.buffer_mut()
            .change_manager_mut()
            .jump_change_older(count)
    }

    /// Jump to newer changelist position (g,).
    pub fn jump_change_newer(&mut self, count: usize) -> Option<Position> {
        self.buffer_mut()
            .change_manager_mut()
            .jump_change_newer(count)
    }

    /// Updates the . register with the last inserted text
    pub fn update_last_inserted_register(&mut self) {
        if let Some(change) = self.buffer().change_manager().last_change() {
            let inserted_text = change.get_inserted_text();
            if !inserted_text.is_empty() {
                self.registers.set_last_inserted(inserted_text);
            }
        }
    }

    /// Adjust decoration positions to track a buffer edit.
    ///
    /// Converts the absolute char-offset `Edit` into line/col deltas and
    /// calls `DecorationMap::adjust_for_edit`.  The rope must still reflect
    /// the **post-edit** state (this runs after `buffer.record()` applies
    /// the edit).
    fn adjust_decorations_for_edit(&mut self, edit: &Edit) {
        let rope = self.buffer().rope().clone();
        match edit {
            Edit::Insert { offset, text } => {
                let newlines = text.chars().filter(|&c| c == '\n').count();
                // The offset is where text was inserted (in the NOW-modified rope).
                // We need the pre-insert position.  Since the insert already
                // happened, `offset` points into the new rope.  The line/col of
                // the insert point is the same in both old and new rope (text was
                // added AFTER this point).
                let insert_offset = (*offset).min(rope.len_chars());
                let edit_line = rope.char_to_line(insert_offset);
                let line_start = rope.line_to_char(edit_line);
                let edit_col = insert_offset - line_start;

                if newlines > 0 {
                    self.decorations.adjust_for_edit(
                        edit_line,
                        edit_col,
                        newlines as isize,
                        0,
                    );
                } else {
                    let chars_inserted = text.chars().count() as isize;
                    self.decorations
                        .adjust_for_edit(edit_line, edit_col, 0, chars_inserted);
                }
            }
            Edit::Delete { offset, text } => {
                let newlines = text.chars().filter(|&c| c == '\n').count();
                // After deletion, `offset` points to where the deleted text
                // was.  The rope is already modified (text removed).
                let del_offset = (*offset).min(rope.len_chars());
                let edit_line = rope.char_to_line(del_offset);
                let line_start = rope.line_to_char(edit_line);
                let edit_col = del_offset - line_start;

                if newlines > 0 {
                    self.decorations.adjust_for_edit(
                        edit_line,
                        edit_col,
                        -(newlines as isize),
                        0,
                    );
                } else {
                    let chars_deleted = text.chars().count() as isize;
                    self.decorations
                        .adjust_for_edit(edit_line, edit_col, 0, -chars_deleted);
                }
            }
        }
    }

    /// Checks if buffer is modified relative to last save
    pub fn is_modified(&self) -> bool {
        !self.buffer().change_manager().is_at_save_point()
    }

    /// Marks current state as saved
    pub fn mark_saved(&mut self) {
        self.buffer_mut().change_manager_mut().mark_saved();
        self.buffer_mut().mark_clean();
    }
}
