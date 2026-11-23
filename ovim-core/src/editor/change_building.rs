//! Building the undo entry for an insert session and pending change repeats.

use super::{Change, CursorPos, Editor, InsertEntryMode, PendingChangeRepeat};
use crate::repeat_action::RepeatAction;

impl Editor {
    /// Starts building a composite change (e.g., when entering insert mode)
    pub fn start_change_building(&mut self, cursor_before: CursorPos) {
        self.buffer_mut()
            .change_manager_mut()
            .start_building(cursor_before);
        // Open the stateful recording session that `finalize_change_building`
        // will close — this feeds `RepeatAction::InsertSession` alongside the
        // `Recorded` undo entry. Session origin is captured lazily on the first
        // edit via `record_edit`.
        if !self.buffer().is_recording() {
            self.buffer_mut().begin_recording();
        }
    }

    /// Sets how insert mode was entered on the current change builder (for dot repeat).
    pub fn set_change_entry_mode(&mut self, mode: InsertEntryMode) {
        self.buffer_mut().change_manager_mut().set_entry_mode(mode);
    }

    /// Finalizes an Insert recording that was left open on a buffer other than
    /// the current one, and returns whether there was one.
    ///
    /// Some paths switch buffers without leaving Insert mode (a tab click in
    /// the GUI), so the recording stays on the buffer typed into before. The
    /// typing there becomes its own undo step.
    pub(super) fn finalize_recording_left_on_other_buffer(&mut self) -> bool {
        let Some(index) = (0..self.buffers.len()).find(|&index| {
            index != self.current_buffer_index && self.buffers[index].is_recording()
        }) else {
            return false;
        };
        let current = std::mem::replace(&mut self.current_buffer_index, index);
        self.finalize_change_building();
        self.current_buffer_index = current;
        true
    }

    /// Runs `edit` between two Insert-mode undo steps, as `<C-g>u` does in Vim.
    ///
    /// Use it for an edit that does not come from typing (an AI tool, an LSP
    /// result, an API request), because such an edit can arrive while Insert
    /// mode is recording the current buffer. The text typed so far becomes
    /// its own undo step, `edit` runs and records its own, and Insert mode
    /// starts a new recording at the cursor. Outside Insert mode, `edit` just
    /// runs.
    ///
    /// The open recording cannot simply stay open around `edit`: its edits
    /// are stored at char offsets that no longer match the text once `edit`
    /// lands, so undoing them would corrupt the buffer.
    pub fn with_insert_undo_break<R>(&mut self, edit: impl FnOnce(&mut Self) -> R) -> R {
        if !self.buffer().is_recording() {
            return edit(self);
        }
        let mode = self.mode();
        let entry_mode = self
            .buffer()
            .change_manager()
            .current_builder
            .as_ref()
            .map(|builder| builder.entry_mode().clone());
        self.finalize_change_building();
        let result = edit(self);
        if let Some(entry_mode) = entry_mode.filter(|_| self.mode() == mode) {
            self.start_change_building(self.cursor_position());
            self.set_change_entry_mode(entry_mode);
        }
        result
    }

    /// Finalizes the current insert-session change.
    ///
    /// Closes the stateful recording session and, if it produced edits,
    /// pushes a single `Change::Recorded` as the session's undo entry.
    /// Also installs a `RepeatAction::InsertSession` for dot-repeat. The
    /// legacy `ChangeBuilder`-built `Composite` is no longer produced here;
    /// the builder's role is now purely to carry `entry_mode` and
    /// `cursor_before` across the session.
    ///
    /// Returns the token of the pushed undo entry, for flows that merge the
    /// session with a preceding delete or replicate it (c, visual-block I/A).
    pub fn finalize_change_building(&mut self) -> Option<crate::change::ChangeToken> {
        if !self.buffer().is_recording() {
            self.finalize_recording_left_on_other_buffer();
        }
        let cursor_after =
            CursorPos::new(self.buffer().cursor().line(), self.buffer().cursor().col());

        // Read session metadata from the builder, then drop it — no more
        // Pattern A Composite.
        let (cursor_before, entry_mode) = match self
            .buffer_mut()
            .change_manager_mut()
            .current_builder
            .take()
        {
            Some(builder) => (builder.cursor_before(), builder.entry_mode().clone()),
            None => {
                // No active session — still make sure recording is closed so
                // a leaked `begin_recording` doesn't trip later record() calls.
                let _ = self.buffer_mut().end_recording();
                return None;
            }
        };

        let origin = self.buffer().recording_origin();
        let origin_cursor = self.buffer().recording_origin_cursor();
        let edits = self.buffer_mut().end_recording();

        // The `.` register: what was typed (empty for an insert that typed
        // nothing, as in vim). For o/O the first edit is the opened line.
        let typed = match entry_mode {
            InsertEntryMode::OpenBelow | InsertEntryMode::OpenAbove => edits.get(1..),
            _ => Some(&edits[..]),
        };
        let inserted = crate::edit::surviving_inserted_text(typed.unwrap_or_default());
        self.registers.set_last_inserted(inserted);
        if edits.is_empty() {
            // `i<Esc>` still redefines `.` (vim repeats the empty insert), so the
            // command before it is not replayed instead.
            if !matches!(
                entry_mode,
                InsertEntryMode::OpenBelow | InsertEntryMode::OpenAbove
            ) {
                self.buffer_mut()
                    .change_manager_mut()
                    .set_repeat_action(Some(RepeatAction::InsertSession {
                        count: 1,
                        entry_mode,
                        origin_offset: 0,
                        edits,
                    }));
            }
            return None;
        }

        // Push the session as a mechanical-undo `Recorded` entry. The
        // `edit_start` override makes `g;` land at the first-edit cursor
        // (post-entry-mode) rather than the pre-entry-mode `cursor_before`
        // that undo restores to.
        let change = match origin_cursor {
            Some(edit_start) => Change::recorded_with_edit_start(
                edits.clone(),
                cursor_before,
                cursor_after,
                edit_start,
            ),
            None => Change::recorded(edits.clone(), cursor_before, cursor_after),
        };
        let token = self.buffer_mut().change_manager_mut().push_change(change);

        // Install dot-repeat. Session edits always start at a recorded
        // origin; without one there is nothing to re-anchor, so `.` must not
        // keep repeating the command before this insert either.
        let cm = self.buffer_mut().change_manager_mut();
        cm.set_repeat_action(origin.map(|origin_offset| RepeatAction::InsertSession {
            count: 1,
            entry_mode,
            origin_offset,
            edits,
        }));
        Some(token)
    }

    /// Sets a pending change repeat (for cc, C, s, cj, etc. dot-repeat)
    pub fn set_pending_change_repeat(&mut self, pending: PendingChangeRepeat) {
        self.editing.pending_change_repeat = Some(pending);
    }

    /// Takes and clears the pending change repeat
    pub fn take_pending_change_repeat(&mut self) -> Option<PendingChangeRepeat> {
        self.editing.pending_change_repeat.take()
    }
}
