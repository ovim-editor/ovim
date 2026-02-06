//! Change tracking and undo/redo operations

use super::{Change, Editor, Position};
use crate::edit::Edit;

impl Editor {
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
    }

    /// Redoes the next change
    pub fn redo(&mut self) {
        self.buffer_mut().redo();
        self.invalidate_hover_cache();
        self.mark_buffer_modified();
    }

    /// Repeats the last change with proper cursor position tracking.
    ///
    /// Records cursor_before/cursor_after so undo after dot-repeat restores
    /// the cursor to where the repeat happened, not the original change.
    /// Buffer mutations are captured via `record()` so the undo entry uses
    /// mechanical inverse edits rather than semantic replay.
    pub fn repeat_last_change(&mut self) {
        let buf = self.buffer_mut();
        if let Some(change) = buf.change_manager().last_change().cloned() {
            let mut repeated = change;
            let before = (buf.cursor().line(), buf.cursor().col());

            // Record the repeat's buffer mutations for mechanical undo
            let ((), edits) = buf.record(|b| {
                // Call repeat() BEFORE set_cursor_before() — repeat() uses the
                // original cursor_before to detect deletion direction (forward vs
                // backward).  It also updates range/deleted_text so undo works.
                repeated.repeat(b);
            });

            let after = (buf.cursor().line(), buf.cursor().col());

            // Push recorded undo (mechanical) — single `u` undoes the whole repeat
            let undo_change = Change::recorded(edits, before, after);
            buf.change_manager_mut().undo_stack.push(undo_change);
            buf.change_manager_mut().redo_stack.clear();

            // Update repeat template positions for next repeat
            repeated.set_cursor_before(before);
            repeated.set_cursor_after(after);
            buf.change_manager_mut().last_change = Some(repeated);
        }
    }

    /// Pushes a recorded undo entry without setting the repeat register.
    /// Use for compound operations (join, case change, indent) where the
    /// dot-repeat change is set separately.
    pub fn push_recorded_undo(
        &mut self,
        edits: Vec<Edit>,
        cursor_before: Position,
        cursor_after: Position,
    ) {
        let change = Change::recorded(edits, cursor_before, cursor_after);
        let cm = self.buffer_mut().change_manager_mut();
        cm.undo_stack.push(change);
        cm.redo_stack.clear();
    }

    /// Sets the dot-repeat register without pushing to the undo stack.
    pub fn set_repeat_change(&mut self, change: Change) {
        self.buffer_mut().change_manager_mut().last_change = Some(change);
    }

    /// Returns the current cursor position as (line, col).
    pub fn cursor_position(&self) -> Position {
        (self.buffer().cursor().line(), self.buffer().cursor().col())
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
}
