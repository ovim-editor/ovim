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
#[derive(Debug, Clone, PartialEq, Default)]
pub enum InputState {
    /// Ready for any command. This is the default/idle state.
    #[default]
    Normal,

    /// Leader key (<Space>) was pressed, awaiting command sequence.
    ///
    /// Example sequences:
    /// - `<Space>th` → type hierarchy
    /// - `<Space>ca` → code actions
    /// - `<Space>e` → show diagnostic at cursor
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

    /// Operator (d, c, y, >, <, =, gu, gU, g~, zf) pressed, awaiting a
    /// motion or text object.
    ///
    /// Example sequences: `dw`, `yy` (operator repeated), `2d3w`.
    OperatorPending {
        /// The operator waiting for a motion
        operator: Operator,
        /// Count typed before the operator (`2` in `2d3w`). Digits typed after
        /// it accumulate in the editor's count; the two multiply when the
        /// motion arrives.
        count: Option<usize>,
    },

    /// 'g' prefix pressed, awaiting second character (`gg`, `gd`, `gu`...).
    GPrefix {
        /// If preceded by an operator (`dgg`, `cgn`)
        operator: Option<Operator>,
    },

    /// `gr` pressed, awaiting the LSP command key (`grr`, `grn`, `gra`...).
    LspPrefix,

    /// 'z' prefix pressed, awaiting second character (`zz`, `zt`, `zo`, `zf`...).
    ZPrefix,

    /// 'Z' pressed, awaiting `Z` (`ZZ` = `:x`) or `Q` (`ZQ` = `:q!`).
    QuitPrefix,

    /// '[' or ']' prefix pressed, awaiting second character (`[[`, `]d`...).
    BracketPrefix {
        /// Which bracket started the sequence
        bracket: char,
    },

    /// Text object prefix (i/a), awaiting the object key.
    ///
    /// Example sequences: `diw`, `ca"`, `yi(`; in Visual mode `viw`.
    TextObjectPending {
        /// The operator to apply (`None` in Visual mode)
        operator: Option<Operator>,
        /// Inner (i) or Around (a)
        prefix: TextObjectPrefix,
    },

    /// Window command prefix (Ctrl-W), awaiting `w`, `v`, `h`...
    WindowCommand,

    /// Macro prefix: `q` (record) or `@` (play), awaiting the register.
    MacroPrefix {
        /// true = recording (q), false = playback (@)
        is_recording: bool,
    },

    /// Register selection prefix (`"`), awaiting the register name.
    RegisterPending,
}

impl InputState {
    /// Returns true if the state is Normal (ready for any command).
    pub fn is_normal(&self) -> bool {
        matches!(self, Self::Normal)
    }

    /// Returns the pending operator, if any.
    pub fn pending_operator(&self) -> Option<Operator> {
        match self {
            Self::OperatorPending { operator, .. } => Some(*operator),
            Self::AwaitingChar { operator, .. }
            | Self::GPrefix { operator }
            | Self::TextObjectPending { operator, .. } => *operator,
            _ => None,
        }
    }

    /// The key that opened a pending prefix: `g`, `z`, `Z`, `[`/`]`,
    /// `i`/`a` (text object), `"`, `q`/`@`; `R` after `gr` and `W` after
    /// Ctrl-W. `None` when no prefix is pending.
    pub fn prefix_key(&self) -> Option<char> {
        Some(match self {
            Self::GPrefix { .. } => 'g',
            Self::LspPrefix => 'R',
            Self::ZPrefix => 'z',
            Self::QuitPrefix => 'Z',
            Self::BracketPrefix { bracket } => *bracket,
            Self::TextObjectPending { prefix, .. } => prefix.as_char(),
            Self::WindowCommand => 'W',
            Self::MacroPrefix { is_recording: true } => 'q',
            Self::MacroPrefix {
                is_recording: false,
            } => '@',
            Self::RegisterPending => '"',
            Self::Normal
            | Self::Leader { .. }
            | Self::AwaitingChar { .. }
            | Self::OperatorPending { .. } => return None,
        })
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

    /// Returns true for forward motions (f/t).
    pub fn is_forward(&self) -> bool {
        matches!(self, Self::Find | Self::Till)
    }

    /// Returns true for till motions (t/T).
    pub fn is_till(&self) -> bool {
        matches!(self, Self::Till | Self::TillBack)
    }

    /// Returns the corresponding `FindType`.
    pub fn find_type(&self) -> crate::editor::FindType {
        if self.is_till() {
            crate::editor::FindType::Till
        } else {
            crate::editor::FindType::Find
        }
    }

    /// Returns the corresponding `FindDirection`.
    pub fn direction(&self) -> crate::editor::FindDirection {
        if self.is_backward() {
            crate::editor::FindDirection::Backward
        } else {
            crate::editor::FindDirection::Forward
        }
    }

    /// Executes this motion on the buffer. Returns true if the cursor moved.
    pub fn execute(&self, buffer: &mut crate::buffer::Buffer, target: char, count: usize) -> bool {
        match self {
            Self::Find => crate::editor::Motions::find_char_forward(buffer, target, count),
            Self::Till => crate::editor::Motions::till_char_forward(buffer, target, count),
            Self::FindBack => crate::editor::Motions::find_char_backward(buffer, target, count),
            Self::TillBack => crate::editor::Motions::till_char_backward(buffer, target, count),
            _ => false,
        }
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
    fn pending_operator_and_prefix_key_project_the_state() {
        let delete = Some(Operator::Delete);
        let cases = [
            (InputState::Normal, None, None),
            (
                InputState::OperatorPending {
                    operator: Operator::Delete,
                    count: None,
                },
                delete,
                None,
            ),
            (InputState::GPrefix { operator: delete }, delete, Some('g')),
            (
                InputState::TextObjectPending {
                    operator: None,
                    prefix: TextObjectPrefix::Around,
                },
                None,
                Some('a'),
            ),
            (
                InputState::AwaitingChar {
                    motion: CharMotion::Find,
                    operator: delete,
                },
                delete,
                None,
            ),
            (InputState::LspPrefix, None, Some('R')),
            (InputState::BracketPrefix { bracket: ']' }, None, Some(']')),
            (
                InputState::MacroPrefix {
                    is_recording: false,
                },
                None,
                Some('@'),
            ),
        ];
        for (state, operator, key) in cases {
            assert_eq!(state.pending_operator(), operator, "{state:?}");
            assert_eq!(state.prefix_key(), key, "{state:?}");
        }
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
        assert_eq!(
            TextObjectPrefix::from_char('i'),
            Some(TextObjectPrefix::Inner)
        );
        assert_eq!(
            TextObjectPrefix::from_char('a'),
            Some(TextObjectPrefix::Around)
        );
        assert_eq!(TextObjectPrefix::from_char('x'), None);
    }
}
