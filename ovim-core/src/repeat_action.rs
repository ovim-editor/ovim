use crate::buffer::Buffer;
use crate::change::TextObjectType;

/// Semantic repeat actions for dot-repeat (Pattern B).
///
/// Unlike `Change` (which handles both undo and repeat), `RepeatAction`
/// captures only the intent needed to re-execute an operation at the
/// current cursor position. Undo is handled separately via `Change::Recorded`.
///
/// Use Pattern B for normal-mode-only operations where repeat is
/// position-independent and no insert-mode entry is needed.
/// See the module doc in `change.rs` for the full boundary guide.
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
    /// Ctrl-A / Ctrl-X — increment/decrement number
    NumberOperation { delta: i64 },
    /// di" / di( / diw — delete text object
    DeleteTextObject { object_type: TextObjectType },
    /// df / dt / dF / dT — delete to character motion
    DeleteCharMotion {
        target: char,
        forward: bool,
        till: bool,
        count: usize,
    },
    /// x — delete character(s) forward
    DeleteCharForward { count: usize },
    /// X — delete character(s) backward
    DeleteCharBackward { count: usize },
    /// dd — delete line(s)
    DeleteLines { count: usize },
    /// D / d$ — delete to end of line
    DeleteToEndOfLine,
    /// dw — delete word forward
    DeleteWordForward { count: usize },
    /// dj — delete current + count lines down
    DeleteLineDown { count: usize },
    /// dk — delete current + count lines up
    DeleteLineUp { count: usize },
    /// d} — delete to paragraph forward
    DeleteParagraphForward { count: usize },
    /// d{ — delete to paragraph backward
    DeleteParagraphBackward { count: usize },
    /// dG — delete to last line (or target line)
    DeleteToLastLine { target_line: usize },
    /// dgg — delete to first line (or target line)
    DeleteToFirstLine { target_line: usize },
    /// d% — delete to matching bracket
    DeleteToMatchingBracket,
    /// r — replace character(s) at cursor
    ReplaceChar { ch: char, count: usize },
    /// p — paste after cursor
    PasteAfter,
    /// P — paste before cursor
    PasteBefore,
    /// Change operator — semantic delete + insert text (cc, C, s, S, cj, ck, etc.)
    Change {
        delete: Box<RepeatAction>,
        inserted_text: String,
        linewise: bool,
    },
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
            Self::NumberOperation { delta } => {
                buffer.modify_number_at_cursor(*delta);
            }
            Self::DeleteTextObject { object_type } => {
                buffer.delete_text_object(object_type);
            }
            Self::DeleteCharMotion {
                target,
                forward,
                till,
                count,
            } => {
                buffer.delete_char_motion(*target, *forward, *till, *count);
            }
            Self::DeleteCharForward { count } => {
                buffer.delete_chars_forward(*count);
            }
            Self::DeleteCharBackward { count } => {
                buffer.delete_chars_backward(*count);
            }
            Self::DeleteLines { count } => {
                buffer.delete_lines(*count);
            }
            Self::DeleteToEndOfLine => {
                buffer.delete_to_end_of_line();
            }
            Self::DeleteWordForward { count } => {
                buffer.delete_word_forward(*count);
            }
            Self::DeleteLineDown { count } => {
                buffer.delete_line_down(*count);
            }
            Self::DeleteLineUp { count } => {
                buffer.delete_line_up(*count);
            }
            Self::DeleteParagraphForward { count } => {
                buffer.delete_paragraph_forward(*count);
            }
            Self::DeleteParagraphBackward { count } => {
                buffer.delete_paragraph_backward(*count);
            }
            Self::DeleteToLastLine { target_line } => {
                buffer.delete_to_last_line(*target_line);
            }
            Self::DeleteToFirstLine { target_line } => {
                buffer.delete_to_first_line(*target_line);
            }
            Self::DeleteToMatchingBracket => {
                buffer.delete_to_matching_bracket();
            }
            Self::ReplaceChar { ch, count } => {
                buffer.replace_chars_at_cursor(*ch, *count);
            }
            Self::PasteAfter | Self::PasteBefore => {
                // Intentional no-op: paste repeat is intercepted in repeat_last_change()
                // before execute() is called, because it needs Editor-level register access.
            }
            Self::Change {
                delete,
                inserted_text,
                linewise,
            } => {
                // Save cursor position before delete (inline changes insert at original pos)
                let pre_delete_line = buffer.cursor().line();
                let pre_delete_col = buffer.cursor().col();

                // Phase 1: Execute the semantic delete at current cursor position
                delete.execute(buffer);

                if *linewise {
                    // Open a new line for the insertion (like cc after delete)
                    let line = buffer.cursor().line();
                    let insert_at = line.min(buffer.line_count());
                    buffer.insert_text_at(insert_at, 0, "\n");
                    buffer.cursor_mut().set_position(insert_at, 0);
                } else {
                    // Inline change: restore cursor to pre-delete column
                    // (delete methods may clamp cursor to normal-mode bounds)
                    buffer
                        .cursor_mut()
                        .set_position(pre_delete_line, pre_delete_col);
                }

                // Phase 2: Insert the captured text
                if !inserted_text.is_empty() {
                    let line = buffer.cursor().line();
                    let col = buffer.cursor().col();
                    buffer.insert_text_at(line, col, inserted_text);

                    // Position cursor at end of inserted text - 1 (Vim Esc behavior)
                    let text_chars: usize = inserted_text.chars().count();
                    if text_chars > 0 {
                        // Calculate final position by walking through inserted text
                        let mut final_line = line;
                        let mut final_col = col;
                        for ch in inserted_text.chars() {
                            if ch == '\n' {
                                final_line += 1;
                                final_col = 0;
                            } else {
                                final_col += 1;
                            }
                        }
                        // Back up one (Vim positions cursor on last inserted char)
                        if final_col > 0 {
                            final_col -= 1;
                        }
                        buffer.cursor_mut().set_position(final_line, final_col);
                    }
                }
            }
        }
    }
}
