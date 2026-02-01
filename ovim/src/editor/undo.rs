use crate::buffer::{Buffer, Cursor};
use ropey::Rope;

/// Represents a snapshot of the buffer state for undo/redo
#[derive(Clone)]
struct UndoState {
    rope: Rope,
    cursor: Cursor,
}

/// Manages undo/redo history
pub struct UndoManager {
    /// History of buffer states
    history: Vec<UndoState>,
    /// Current position in history
    current: usize,
}

impl UndoManager {
    /// Creates a new undo manager with an initial state
    pub fn new(buffer: &Buffer) -> Self {
        let initial_state = UndoState {
            rope: buffer.rope().clone(),
            cursor: *buffer.cursor(),
        };

        Self {
            history: vec![initial_state],
            current: 0,
        }
    }

    /// Saves the current buffer state
    pub fn save_state(&mut self, buffer: &Buffer) {
        // If we're not at the end of history, truncate everything after current
        self.history.truncate(self.current + 1);

        // Add new state
        let state = UndoState {
            rope: buffer.rope().clone(),
            cursor: *buffer.cursor(),
        };

        self.history.push(state);
        self.current += 1;

        // Limit history size to prevent excessive memory usage
        const MAX_HISTORY: usize = 1000;
        if self.history.len() > MAX_HISTORY {
            self.history.drain(0..1);
            self.current -= 1;
        }
    }

    /// Undoes the last change, returning the previous state
    pub fn undo(&mut self) -> Option<(Rope, Cursor)> {
        if self.current > 0 {
            self.current -= 1;
            let state = &self.history[self.current];
            Some((state.rope.clone(), state.cursor))
        } else {
            None
        }
    }

    /// Redoes the next change, returning the next state
    pub fn redo(&mut self) -> Option<(Rope, Cursor)> {
        if self.current + 1 < self.history.len() {
            self.current += 1;
            let state = &self.history[self.current];
            Some((state.rope.clone(), state.cursor))
        } else {
            None
        }
    }

    /// Returns whether undo is available
    pub fn can_undo(&self) -> bool {
        self.current > 0
    }

    /// Returns whether redo is available
    pub fn can_redo(&self) -> bool {
        self.current + 1 < self.history.len()
    }
}
