use crate::buffer::Buffer;

/// Semantic repeat actions for dot-repeat.
///
/// Unlike `Change` (which handles both undo and repeat), RepeatAction
/// captures only the intent needed to re-execute an operation at the
/// current cursor position. Undo is handled separately via `Change::Recorded`.
#[derive(Clone, Debug)]
pub enum RepeatAction {
    /// J / gJ — join lines
    JoinLines { count: usize, add_space: bool },
    /// >> — indent lines
    IndentLines { line_count: usize, tab_width: usize },
    /// << — dedent lines
    DedentLines { line_count: usize, tab_width: usize },
    /// ~ — toggle case at cursor
    ToggleCase { count: usize },
}

impl RepeatAction {
    /// Execute this action at the current cursor position.
    /// Caller is responsible for wrapping in `buffer.record()`.
    pub fn execute(&self, buffer: &mut Buffer) {
        match self {
            Self::JoinLines { count, add_space } => {
                if *add_space {
                    let _ = buffer.join_lines(*count);
                } else {
                    let _ = buffer.join_lines_no_space(*count);
                }
            }
            Self::IndentLines {
                line_count,
                tab_width,
            } => {
                let start = buffer.cursor().line();
                let end = start + line_count;
                buffer.indent_lines_at(start, end, *tab_width);
                // Position cursor at first indented line, col = tab_width
                if start < end.min(buffer.line_count()) {
                    buffer.cursor_mut().set_position(start, *tab_width);
                }
            }
            Self::DedentLines {
                line_count,
                tab_width,
            } => {
                let start = buffer.cursor().line();
                let end = start + line_count;
                buffer.dedent_lines_at(start, end, *tab_width);
                buffer.clamp_cursor_col();
            }
            Self::ToggleCase { count } => {
                for _ in 0..*count {
                    if !buffer.toggle_char_at_cursor() {
                        break;
                    }
                }
            }
        }
    }
}
