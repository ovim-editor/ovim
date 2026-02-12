//! Input context for editor command parsing.
//!
//! This module contains the InputContext struct, which holds all input-related
//! state for command parsing, including counts, operators, pending commands,
//! registers, and the input state machine.

use crate::editor::input_state::InputState;
use crate::editor::operators::Operator;
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

    /// Pending operator (e.g., d for delete, waiting for motion)
    /// Note: This is being phased out in favor of input_state which embeds
    /// the operator in its state variants (OperatorPending, AwaitingChar, etc.)
    pub pending_operator: Option<Operator>,

    /// Pending command character (e.g., 'g' waiting for second character)
    /// Note: This is being phased out in favor of input_state which has
    /// explicit states like GPrefix, ZPrefix, BracketPrefix, etc.
    pub pending_command: Option<char>,

    /// Pending register selection (e.g., 'a' from "a for next operation)
    pub pending_register: Option<char>,

    /// Input state machine for Normal mode (new architecture)
    /// This will eventually replace pending_command, pending_operator, etc.
    pub input_state: InputState,

    /// Leader key (default: space)
    pub leader_key: char,

    /// Pending raw key sequence being considered for normal-mode mappings
    pub pending_mapping_sequence: String,

    /// Original key events that produced pending_mapping_sequence
    pub pending_mapping_events: Vec<KeyEvent>,
}

impl InputContext {
    /// Creates a new InputContext with default values.
    pub fn new() -> Self {
        Self {
            count: None,
            pending_operator: None,
            pending_command: None,
            pending_register: None,
            input_state: InputState::Normal,
            leader_key: ' ', // default space
            pending_mapping_sequence: String::new(),
            pending_mapping_events: Vec::new(),
        }
    }

    /// Resets all pending input state (count, operator, command, register).
    /// This is typically called after executing a command or on Escape.
    pub fn reset(&mut self) {
        self.count = None;
        self.pending_operator = None;
        self.pending_command = None;
        self.pending_register = None;
        self.input_state.reset();
        self.pending_mapping_sequence.clear();
        self.pending_mapping_events.clear();
    }

    /// Returns true if any input is currently pending.
    pub fn has_pending_input(&self) -> bool {
        self.count.is_some()
            || self.pending_operator.is_some()
            || self.pending_command.is_some()
            || self.pending_register.is_some()
            || !self.input_state.is_normal()
            || !self.pending_mapping_sequence.is_empty()
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
        assert_eq!(ctx.pending_operator, None);
        assert_eq!(ctx.pending_command, None);
        assert_eq!(ctx.pending_register, None);
        assert_eq!(ctx.input_state, InputState::Normal);
        assert_eq!(ctx.leader_key, ' ');
        assert!(ctx.pending_mapping_sequence.is_empty());
        assert!(ctx.pending_mapping_events.is_empty());
    }

    #[test]
    fn test_default() {
        let ctx = InputContext::default();
        assert_eq!(ctx.count, None);
    }

    #[test]
    fn test_reset() {
        let mut ctx = InputContext::new();
        ctx.count = Some(5);
        ctx.pending_operator = Some(Operator::Delete);
        ctx.pending_command = Some('g');
        ctx.pending_register = Some('a');
        ctx.input_state = InputState::OperatorPending {
            operator: Operator::Delete,
        };
        ctx.pending_mapping_sequence = "jk".to_string();
        ctx.pending_mapping_events = vec![KeyEvent::new(
            crate::KeyCode::Char('j'),
            crate::Modifiers::NONE,
        )];

        ctx.reset();

        assert_eq!(ctx.count, None);
        assert_eq!(ctx.pending_operator, None);
        assert_eq!(ctx.pending_command, None);
        assert_eq!(ctx.pending_register, None);
        assert_eq!(ctx.input_state, InputState::Normal);
        assert!(ctx.pending_mapping_sequence.is_empty());
        assert!(ctx.pending_mapping_events.is_empty());
    }

    #[test]
    fn test_has_pending_input() {
        let mut ctx = InputContext::new();
        assert!(!ctx.has_pending_input());

        ctx.count = Some(5);
        assert!(ctx.has_pending_input());

        ctx.reset();
        ctx.pending_operator = Some(Operator::Delete);
        assert!(ctx.has_pending_input());

        ctx.reset();
        ctx.pending_command = Some('g');
        assert!(ctx.has_pending_input());

        ctx.reset();
        ctx.pending_register = Some('a');
        assert!(ctx.has_pending_input());

        ctx.reset();
        ctx.input_state = InputState::OperatorPending {
            operator: Operator::Delete,
        };
        assert!(ctx.has_pending_input());

        ctx.reset();
        ctx.pending_mapping_sequence = "jk".to_string();
        assert!(ctx.has_pending_input());
    }
}
