//! Input state machine for Normal mode key handling.
//!
//! This module defines the explicit state machine that tracks what the editor
//! is waiting for during multi-key command sequences. By using an enum instead
//! of scattered `pending_*` fields, we avoid collisions between different
//! input contexts (e.g., `<Space>t` vs `t` motion).

use super::operators::Operator;

/// The input state machine for Normal mode.
///
/// Each variant represents a distinct state the editor can be in while
/// waiting for additional input. The state determines how the next
/// keypress will be interpreted.
#[derive(Debug, Clone, PartialEq)]
pub enum InputState {
    /// Ready for any command. This is the default/idle state.
    Normal,

    /// Leader key (<Space>) was pressed, awaiting command sequence.
    ///
    /// Example sequences:
    /// - `<Space>th` → type hierarchy
    /// - `<Space>ca` → code actions
    /// - `<Space>e` → toggle file tree
    Leader {
        /// Keys pressed after leader (e.g., ['t'] waiting for 'h')
        keys: Vec<char>,
    },

    /// Awaiting target character for a character motion.
    ///
    /// Used for: f, t, F, T (find/till), r (replace), m (mark),
    /// ' and ` (jump to mark)
    AwaitingChar {
        /// The type of character motion
        motion: CharMotion,
        /// If preceded by an operator (d, c, y), apply it to the range
        operator: Option<Operator>,
    },

    /// Operator (d, c, y, >, <) pressed, awaiting motion or text object.
    ///
    /// Example sequences:
    /// - `dw` → delete word
    /// - `ci"` → change inside quotes
    /// - `yy` → yank line (operator repeated)
    OperatorPending {
        /// The operator waiting for a motion
        operator: Operator,
    },

    /// 'g' prefix pressed, awaiting second character.
    ///
    /// Example sequences:
    /// - `gg` → go to first line
    /// - `gd` → go to definition
    /// - `ge` → end of previous word
    /// - `gu{motion}` → lowercase
    /// - `gU{motion}` → uppercase
    GPrefix {
        /// If preceded by an operator (for dgg, cgg, ygg)
        operator: Option<Operator>,
    },

    /// 'z' prefix pressed, awaiting second character.
    ///
    /// Example sequences:
    /// - `zz` → center cursor line in viewport
    /// - `zt` → cursor line to top
    /// - `zb` → cursor line to bottom
    /// - `zo` → open fold
    /// - `zc` → close fold
    ZPrefix {
        /// If preceded by an operator (for zf fold motion)
        operator: Option<Operator>,
    },

    /// '[' or ']' prefix pressed, awaiting second character.
    ///
    /// Example sequences:
    /// - `[[` → previous section
    /// - `]]` → next section
    /// - `[m` → previous method
    /// - `]d` → next diagnostic
    BracketPrefix {
        /// Which bracket started the sequence
        bracket: char,
        /// If preceded by an operator
        operator: Option<Operator>,
    },

    /// Text object prefix (i/a) after operator.
    ///
    /// Example sequences:
    /// - `diw` → delete inner word
    /// - `ca"` → change around quotes
    /// - `yi(` → yank inner parentheses
    TextObjectPending {
        /// The operator to apply
        operator: Operator,
        /// Inner (i) or Around (a)
        prefix: TextObjectPrefix,
    },

    /// Window command prefix (Ctrl-W).
    ///
    /// Example sequences:
    /// - `<C-w>h` → move to left window
    /// - `<C-w>v` → vertical split
    /// - `<C-w>s` → horizontal split
    WindowCommand,

    /// Macro prefix (q for record, @ for playback).
    ///
    /// Example sequences:
    /// - `qa` → start recording macro to register 'a'
    /// - `@a` → play macro from register 'a'
    MacroPrefix {
        /// true = recording (q), false = playback (@)
        is_recording: bool,
    },

    /// Register selection prefix (").
    ///
    /// Example sequences:
    /// - `"ayy` → yank line to register 'a'
    /// - `"ap` → paste from register 'a'
    RegisterPending,
}

impl Default for InputState {
    fn default() -> Self {
        Self::Normal
    }
}

impl InputState {
    /// Returns true if the state is Normal (ready for any command).
    pub fn is_normal(&self) -> bool {
        matches!(self, Self::Normal)
    }

    /// Returns true if an operator is pending in this state.
    pub fn has_pending_operator(&self) -> bool {
        matches!(
            self,
            Self::OperatorPending { .. }
                | Self::AwaitingChar {
                    operator: Some(_),
                    ..
                }
                | Self::GPrefix {
                    operator: Some(_),
                    ..
                }
                | Self::ZPrefix {
                    operator: Some(_),
                    ..
                }
                | Self::BracketPrefix {
                    operator: Some(_),
                    ..
                }
                | Self::TextObjectPending { .. }
        )
    }

    /// Returns the pending operator, if any.
    pub fn pending_operator(&self) -> Option<Operator> {
        match self {
            Self::OperatorPending { operator } => Some(*operator),
            Self::AwaitingChar { operator, .. } => *operator,
            Self::GPrefix { operator, .. } => *operator,
            Self::ZPrefix { operator, .. } => *operator,
            Self::BracketPrefix { operator, .. } => *operator,
            Self::TextObjectPending { operator, .. } => Some(*operator),
            _ => None,
        }
    }

    /// Resets to Normal state.
    pub fn reset(&mut self) {
        *self = Self::Normal;
    }
}

/// Types of character-based motions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharMotion {
    /// `f{char}` - move cursor TO the next occurrence of char
    Find,
    /// `t{char}` - move cursor TILL (one before) the next occurrence
    Till,
    /// `F{char}` - move cursor TO the previous occurrence of char
    FindBack,
    /// `T{char}` - move cursor TILL (one after) the previous occurrence
    TillBack,
    /// `r{char}` - replace character under cursor
    Replace,
    /// `m{char}` - set mark at current position
    Mark,
    /// `'{char}` - jump to line of mark
    JumpMarkLine,
    /// `` `{char} `` - jump to exact position of mark
    JumpMarkExact,
}

impl CharMotion {
    /// Returns true if this is a find/till motion (not mark/replace).
    pub fn is_find_motion(&self) -> bool {
        matches!(
            self,
            Self::Find | Self::Till | Self::FindBack | Self::TillBack
        )
    }

    /// Returns the opposite direction motion.
    pub fn reversed(&self) -> Self {
        match self {
            Self::Find => Self::FindBack,
            Self::Till => Self::TillBack,
            Self::FindBack => Self::Find,
            Self::TillBack => Self::Till,
            other => *other, // Mark/Replace don't reverse
        }
    }

    /// Returns true if this motion searches backward.
    pub fn is_backward(&self) -> bool {
        matches!(self, Self::FindBack | Self::TillBack)
    }
}

/// Text object prefix type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectPrefix {
    /// `i` - inner (excludes delimiters)
    Inner,
    /// `a` - around (includes delimiters)
    Around,
}

impl TextObjectPrefix {
    /// Creates from a character ('i' or 'a').
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'i' => Some(Self::Inner),
            'a' => Some(Self::Around),
            _ => None,
        }
    }

    /// Returns the character representation.
    pub fn as_char(&self) -> char {
        match self {
            Self::Inner => 'i',
            Self::Around => 'a',
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_state_default() {
        assert_eq!(InputState::default(), InputState::Normal);
    }

    #[test]
    fn test_is_normal() {
        assert!(InputState::Normal.is_normal());
        assert!(!InputState::Leader { keys: vec![] }.is_normal());
    }

    #[test]
    fn test_has_pending_operator() {
        assert!(!InputState::Normal.has_pending_operator());

        assert!(InputState::OperatorPending {
            operator: Operator::Delete
        }
        .has_pending_operator());

        assert!(InputState::AwaitingChar {
            motion: CharMotion::Find,
            operator: Some(Operator::Delete),
        }
        .has_pending_operator());

        assert!(!InputState::AwaitingChar {
            motion: CharMotion::Find,
            operator: None,
        }
        .has_pending_operator());
    }

    #[test]
    fn test_char_motion_reversed() {
        assert_eq!(CharMotion::Find.reversed(), CharMotion::FindBack);
        assert_eq!(CharMotion::Till.reversed(), CharMotion::TillBack);
        assert_eq!(CharMotion::FindBack.reversed(), CharMotion::Find);
        assert_eq!(CharMotion::TillBack.reversed(), CharMotion::Till);
    }

    #[test]
    fn test_text_object_prefix_from_char() {
        assert_eq!(TextObjectPrefix::from_char('i'), Some(TextObjectPrefix::Inner));
        assert_eq!(TextObjectPrefix::from_char('a'), Some(TextObjectPrefix::Around));
        assert_eq!(TextObjectPrefix::from_char('x'), None);
    }
}
