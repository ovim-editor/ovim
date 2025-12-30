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

    /// Repeats the last change
    pub fn repeat_last_change(&mut self) {
        self.buffer_mut().repeat_last_change();
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
