//! Change tracking and undo/redo operations

use super::{Change, Editor};

impl Editor {
    /// Pops the last change from the undo stack (without undoing it)
    /// Used when replacing a change with a composite version
    pub fn pop_last_change(&mut self) -> Option<Change> {
        self.buffer_mut().change_manager_mut().pop_last_change()
    }

    /// Undoes the last change
    pub fn undo(&mut self) {
        self.buffer_mut().undo();
    }

    /// Redoes the next change
    pub fn redo(&mut self) {
        self.buffer_mut().redo();
    }

    /// Repeats the last change with proper cursor position tracking.
    ///
    /// Records cursor_before/cursor_after so undo after dot-repeat restores
    /// the cursor to where the repeat happened, not the original change.
    pub fn repeat_last_change(&mut self) {
        let buf = self.buffer_mut();
        if let Some(change) = buf.change_manager().last_change().cloned() {
            let mut repeated = change;
            let before = (buf.cursor().line(), buf.cursor().col());
            // Call repeat() BEFORE set_cursor_before() — repeat() uses the
            // original cursor_before to detect deletion direction (forward vs
            // backward).  It also updates range/deleted_text so undo works.
            repeated.repeat(buf);
            let after = (buf.cursor().line(), buf.cursor().col());
            repeated.set_cursor_before(before);
            repeated.set_cursor_after(after);
            buf.change_manager_mut().push_change(repeated);
        }
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
