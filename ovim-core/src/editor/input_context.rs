//! Input context for editor command parsing.
//!
//! This module contains the InputContext struct, which holds all input-related
//! state for command parsing: count, register, the input state machine
//! (pending operator / prefix / character argument) and mapping keys.

use crate::editor::input_state::InputState;
use crate::KeyEvent;

/// Context for input state machine (counts, operators, pending commands).
///
/// This struct encapsulates all the transient state needed to parse multi-key
/// command sequences in Normal mode. By grouping these fields together, we
/// make it easier to reason about input handling and avoid scattered state
/// across the Editor struct.
#[derive(Debug)]
pub struct InputContext {
    /// Count prefix for commands (e.g., 5j means move down 5 lines)
    pub count: Option<usize>,

    /// Pending register selection (e.g., 'a' from "a for next operation)
    pub pending_register: Option<char>,

    /// What a multi-key sequence is waiting for (pending operator, prefix
    /// key, character argument, leader keys).
    pub input_state: InputState,

    /// Leader key (default: space)
    pub leader_key: char,

    /// Pending raw key sequence being considered for normal-mode mappings
    pub pending_mapping_sequence: String,

    /// Original key events that produced pending_mapping_sequence
    pub pending_mapping_events: Vec<KeyEvent>,

    /// How deeply ex command lines are nested right now (see `Editor::nested`)
    pub(super) ex_line_depth: usize,

    /// How deeply `:normal` commands are nested right now
    pub(super) normal_depth: usize,
}

impl InputContext {
    /// Creates a new InputContext with default values.
    pub fn new() -> Self {
        Self {
            count: None,
            pending_register: None,
            input_state: InputState::Normal,
            leader_key: ' ', // default space
            pending_mapping_sequence: String::new(),
            pending_mapping_events: Vec::new(),
            ex_line_depth: 0,
            normal_depth: 0,
        }
    }
}

impl Default for InputContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_context() {
        let ctx = InputContext::new();
        assert_eq!(ctx.count, None);
        assert_eq!(ctx.pending_register, None);
        assert_eq!(ctx.input_state, InputState::Normal);
        assert_eq!(ctx.leader_key, ' ');
        assert!(ctx.pending_mapping_sequence.is_empty());
        assert!(ctx.pending_mapping_events.is_empty());
    }
}
