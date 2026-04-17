//! Change tracking and undo/redo operations

use super::{Change, CursorPos, Editor};
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
        let (did_undo, edits) = self.buffer_mut().undo();
        if did_undo && !edits.is_empty() {
            let rope = self.buffer().rope().clone();
            let new_version = self.buffer().version() as u64;
            self.decorations
                .adjust_for_edits(&edits, &rope, new_version);
        }
        self.invalidate_hover_cache();
        self.mark_buffer_modified();
        self.mark_dirty();
    }

    /// Redoes the next change
    pub fn redo(&mut self) {
        let (did_redo, edits) = self.buffer_mut().redo();
        if did_redo && !edits.is_empty() {
            let rope = self.buffer().rope().clone();
            let new_version = self.buffer().version() as u64;
            self.decorations
                .adjust_for_edits(&edits, &rope, new_version);
        }
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
                let before = CursorPos::new(buf.cursor().line(), buf.cursor().col());
                let ((), edits) = buf.record(|b| {
                    action.execute(b);
                });
                let after = CursorPos::new(buf.cursor().line(), buf.cursor().col());
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
                let before = CursorPos::new(buf.cursor().line(), buf.cursor().col());

                // Record the repeat's buffer mutations for mechanical undo.
                let ((), edits) = buf.record(|b| {
                    // Call repeat() BEFORE set_cursor_before() — repeat() uses the
                    // original cursor_before to detect deletion direction (forward vs
                    // backward). It also updates range/deleted_text so undo works.
                    repeated.repeat(b);
                });

                let after = CursorPos::new(buf.cursor().line(), buf.cursor().col());
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
        cursor_before: CursorPos,
        cursor_after: CursorPos,
    ) {
        // Adjust decoration char_offsets to follow the edits.
        // The rope is already in post-edit state; the arithmetic adjustment
        // uses only the edit offsets/lengths, not line/col, so this is correct.
        let rope = self.buffer().rope().clone();
        let new_version = self.buffer().version() as u64;
        self.decorations
            .adjust_for_edits(&edits, &rope, new_version);

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
        cursor_before: CursorPos,
        cursor_after: CursorPos,
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

    /// Returns the current cursor position (grapheme-space).
    pub fn cursor_position(&self) -> CursorPos {
        CursorPos::new(self.buffer().cursor().line(), self.buffer().cursor().col())
    }

    /// Returns the last position where an edit occurred (for g; navigation).
    pub fn last_edit_position(&self) -> Option<CursorPos> {
        self.buffer().change_manager().last_edit_position
    }

    /// Jump to older changelist position (g;).
    pub fn jump_change_older(&mut self, count: usize) -> Option<CursorPos> {
        self.buffer_mut()
            .change_manager_mut()
            .jump_change_older(count)
    }

    /// Jump to newer changelist position (g,).
    pub fn jump_change_newer(&mut self, count: usize) -> Option<CursorPos> {
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

    /// Checks if buffer is modified relative to last save
    pub fn is_modified(&self) -> bool {
        !self.buffer().change_manager().is_at_save_point()
    }

    /// Marks current state as saved
    pub fn mark_saved(&mut self) {
        self.buffer_mut().change_manager_mut().mark_saved();
        self.buffer_mut().mark_clean();
    }

    /// Post-mutation fixup for call sites that bypass `record()`.
    ///
    /// Clears the buffer's `edit_log` (its projection is no longer sound)
    /// and invalidates inlay-hint and diagnostic slots so the next event-loop
    /// tick re-pulls them. Also routes through `mark_buffer_modified` so
    /// LSP didChange fires.
    ///
    /// Use this from bypass sites — direct `insert_text_at`/`delete_range`
    /// calls outside a `record()` session — to preserve the invariant that
    /// decoration positions stay aligned with buffer content.
    pub fn fixup_after_bypass_mutation(&mut self) {
        self.buffer_mut().edit_log_mut().clear();
        self.mark_buffer_modified();
    }
}
