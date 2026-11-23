//! Accessors for input-side editor state: macro recording, the input state
//! machine, counts, key mappings, the leader key and macro abort.

use super::{Editor, InputState, KeyMapManager, Operator};

impl Editor {
    pub fn start_macro_recording(&mut self, register: char) -> bool {
        self.macro_manager.start_recording(register)
    }

    /// Records a key event in the current macro
    pub fn record_macro_event(&mut self, event: crate::KeyEvent) {
        self.macro_manager.record_event(event);
    }

    /// Returns whether currently recording a macro
    pub fn is_recording_macro(&self) -> bool {
        self.macro_manager.is_recording()
    }

    /// Gets the register being recorded
    pub fn recording_register(&self) -> Option<char> {
        self.macro_manager.recording_register()
    }

    /// Sets the last played macro register (for @@)
    pub fn set_last_played_macro(&mut self, register: char) {
        self.macro_manager.set_last_played(register);
    }

    /// Gets the last played macro register (for @@)
    pub fn last_played_macro(&self) -> Option<char> {
        self.macro_manager.last_played()
    }

    /// Gets the current input state (new state machine)
    pub fn input_state(&self) -> &InputState {
        &self.input.input_state
    }

    /// Sets the input state (new state machine)
    pub fn set_input_state(&mut self, state: InputState) {
        self.input.input_state = state;
    }

    /// Resets input state to Normal
    pub fn reset_input_state(&mut self) {
        self.input.input_state = InputState::Normal;
    }

    /// Gets the current count
    pub fn count(&self) -> Option<usize> {
        self.input.count
    }

    /// Sets the count
    pub fn set_count(&mut self, count: usize) {
        self.input.count = Some(count);
    }

    /// Appends a digit to the count
    pub fn append_count(&mut self, digit: usize) {
        let current = self.input.count.unwrap_or(0);
        self.input.count = Some(current * 10 + digit);
    }

    /// Starts an operator (`d`, `y`, `gU`, ...): a count typed so far belongs to
    /// the operator, and digits typed next start the motion's own count.
    pub fn enter_operator_pending(&mut self, operator: Operator) {
        let count = self.input.count.take();
        self.set_input_state(InputState::OperatorPending { operator, count });
    }

    /// Takes the count typed so far, leaving none.
    pub fn take_count(&mut self) -> Option<usize> {
        self.input.count.take()
    }

    /// Clears the count
    pub fn clear_count(&mut self) {
        self.input.count = None;
    }

    /// Gets the effective count (count or 1)
    pub fn effective_count(&self) -> usize {
        self.input.count.unwrap_or(1)
    }

    /// Gets a reference to the keymaps
    pub fn keymaps(&self) -> &KeyMapManager {
        &self.keymaps
    }

    /// Gets a mutable reference to the keymaps
    pub fn keymaps_mut(&mut self) -> &mut KeyMapManager {
        &mut self.keymaps
    }

    /// Returns true when normal-mode keymap matching is waiting for more input.
    pub fn has_pending_mapping(&self) -> bool {
        !self.input.pending_mapping_sequence.is_empty()
    }

    /// Returns the pending normal-mode mapping key sequence.
    pub fn pending_mapping_sequence(&self) -> &str {
        &self.input.pending_mapping_sequence
    }

    /// Appends one encoded key token to the pending mapping sequence.
    pub fn append_pending_mapping(&mut self, token: &str, event: crate::KeyEvent) {
        self.input.pending_mapping_sequence.push_str(token);
        self.input.pending_mapping_events.push(event);
    }

    /// Clears all pending mapping state.
    pub fn clear_pending_mapping(&mut self) {
        self.input.pending_mapping_sequence.clear();
        self.input.pending_mapping_events.clear();
    }

    /// Drains pending mapping events and clears the pending sequence.
    pub fn take_pending_mapping_events(&mut self) -> Vec<crate::KeyEvent> {
        self.input.pending_mapping_sequence.clear();
        std::mem::take(&mut self.input.pending_mapping_events)
    }

    /// Gets the leader key (default: space)
    pub fn leader_key(&self) -> char {
        self.input.leader_key
    }

    /// Sets the leader key
    pub fn set_leader_key(&mut self, key: char) {
        self.input.leader_key = key;
    }

    /// Returns whether macro playback should abort (a motion failed to move).
    pub fn macro_aborted(&self) -> bool {
        self.macro_manager.aborted()
    }

    /// Signal that a motion failed (cursor didn't move), aborting macro playback.
    pub fn signal_macro_abort(&mut self) {
        self.macro_manager.signal_abort();
    }

    /// Clear the macro abort flag.
    pub fn clear_macro_abort(&mut self) {
        self.macro_manager.clear_abort();
    }
}
