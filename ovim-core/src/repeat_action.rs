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
    IndentLines {
        line_count: usize,
        shift_width: usize,
        expand_tab: bool,
    },
    /// << — dedent lines
    DedentLines {
        line_count: usize,
        shift_width: usize,
    },
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
    /// db — delete word backward
    DeleteWordBackward { count: usize },
    /// de — delete to end of word (inclusive)
    DeleteWordEnd { count: usize },
    /// dB — delete WORD backward
    DeleteWordBackwardBig { count: usize },
    /// dE — delete to end of WORD (inclusive)
    DeleteWordEndBig { count: usize },
    /// dh — delete character left
    DeleteCharLeft { count: usize },
    /// d0 — delete to start of line
    DeleteToStartOfLine,
    /// d^ — delete to first non-blank
    DeleteToFirstNonBlank,
    /// dW — delete WORD forward
    DeleteWordForwardBig { count: usize },
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
    PasteAfter { count: usize },
    /// P — paste before cursor
    PasteBefore { count: usize },
    /// o/O — open a line below/above, then replay inserted text
    OpenLine {
        above: bool,
        inserted_text: String,
        shift_width: usize,
        expand_tab: bool,
    },
    /// Visual-mode character-wise delete (v...d/x)
    DeleteVisualChar {
        line_delta: usize,
        offset_col: usize,
    },
    /// Visual-line delete (V...d/x)
    DeleteVisualLine { line_count: usize },
    /// Visual-block delete (Ctrl-V...d/x)
    DeleteVisualBlock { line_count: usize, width: usize },
    /// Visual-block change (Ctrl-V...c): delete block then insert on each line
    ChangeVisualBlock {
        line_count: usize,
        width: usize,
        inserted_text: String,
    },
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
                shift_width,
                expand_tab,
            } => {
                let start = buffer.cursor().line();
                let end = start + line_count;
                buffer.indent_lines_at(start, end, *shift_width, *expand_tab);
                let first_nb = buffer.first_non_blank_col(start);
                buffer.cursor_mut().set_position(start, first_nb);
            }
            Self::DedentLines {
                line_count,
                shift_width,
            } => {
                let start = buffer.cursor().line();
                let end = start + line_count;
                buffer.dedent_lines_at(start, end, *shift_width);
                let first_nb = buffer.first_non_blank_col(start);
                buffer.cursor_mut().set_position(start, first_nb);
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
            Self::DeleteWordBackward { count } => {
                buffer.delete_word_backward(*count);
            }
            Self::DeleteWordEnd { count } => {
                buffer.delete_word_end(*count);
            }
            Self::DeleteWordBackwardBig { count } => {
                buffer.delete_word_backward_big(*count);
            }
            Self::DeleteWordEndBig { count } => {
                buffer.delete_word_end_big(*count);
            }
            Self::DeleteCharLeft { count } => {
                buffer.delete_char_left(*count);
            }
            Self::DeleteToStartOfLine => {
                buffer.delete_to_start_of_line();
            }
            Self::DeleteToFirstNonBlank => {
                buffer.delete_to_first_non_blank();
            }
            Self::DeleteWordForwardBig { count } => {
                buffer.delete_word_forward_big(*count);
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
            Self::PasteAfter { .. } | Self::PasteBefore { .. } => {
                // Intentional no-op: paste repeat is intercepted in repeat_last_change()
                // before execute() is called, because it needs Editor-level register access.
            }
            Self::OpenLine {
                above,
                inserted_text,
                shift_width,
                expand_tab,
            } => {
                let line_idx = buffer.cursor().line();
                let line_text = buffer.line(line_idx).unwrap_or_default();

                let mut indent: String = line_text
                    .chars()
                    .take_while(|c| c.is_whitespace() && *c != '\n')
                    .collect();

                if !*above {
                    // Match `o` behavior: add one extra indent level after opening delimiters.
                    let trimmed =
                        line_text.trim_end_matches(|c: char| c == '\n' || c.is_whitespace());
                    if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
                        if *expand_tab {
                            indent.push_str(&" ".repeat(*shift_width));
                        } else {
                            indent.push('\t');
                        }
                    }
                }

                if *above {
                    let text = format!("{}\n", indent);
                    buffer.insert_text_at(line_idx, 0, &text);
                    buffer
                        .cursor_mut()
                        .set_position(line_idx, indent.chars().count());
                } else {
                    let (insert_pos, text) = if line_text.ends_with('\n') {
                        ((line_idx + 1, 0), format!("{}\n", indent))
                    } else {
                        let line_len = line_text.chars().count();
                        ((line_idx, line_len), format!("\n{}\n", indent))
                    };
                    buffer.insert_text_at(insert_pos.0, insert_pos.1, &text);
                    buffer
                        .cursor_mut()
                        .set_position(line_idx + 1, indent.chars().count());
                }

                if inserted_text.is_empty() {
                    // Match insert-mode exit cleanup for `o/O<Esc>` on whitespace-only lines.
                    let current_line = buffer.cursor().line();
                    if let Some(line) = buffer.line(current_line) {
                        let line_wo_nl = line.trim_end_matches('\n');
                        if !line_wo_nl.is_empty() && line_wo_nl.chars().all(|c| c.is_whitespace()) {
                            let whitespace_len = line_wo_nl.chars().count();
                            buffer.delete_range(current_line, 0, current_line, whitespace_len);
                            buffer.cursor_mut().set_position(current_line, 0);
                        }
                    }
                    return;
                }

                let line = buffer.cursor().line();
                let col = buffer.cursor().col();
                buffer.insert_text_at(line, col, inserted_text);

                // Position cursor at end of inserted text - 1 (Vim Esc behavior)
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
                if final_col > 0 {
                    final_col -= 1;
                }
                buffer.cursor_mut().set_position(final_line, final_col);
            }
            Self::DeleteVisualChar {
                line_delta,
                offset_col,
            } => {
                let start_line = buffer.cursor().line();
                let start_col = buffer.cursor().col();
                let end_line = start_line + line_delta;
                let end_col = if *line_delta == 0 {
                    start_col + offset_col
                } else {
                    *offset_col
                };
                buffer.delete_range(start_line, start_col, end_line, end_col);
                buffer.cursor_mut().set_position(start_line, start_col);
            }
            Self::DeleteVisualLine { line_count } => {
                let start_line = buffer.cursor().line();
                let end_line_exclusive = start_line + line_count;
                buffer.delete_range(start_line, 0, end_line_exclusive, 0);
                let new_line = start_line.min(buffer.line_count().saturating_sub(1));
                buffer.cursor_mut().set_position(new_line, 0);
            }
            Self::DeleteVisualBlock { line_count, width } => {
                let start_line = buffer.cursor().line();
                let start_col = buffer.cursor().col();

                for i in 0..*line_count {
                    let line_idx = start_line + i;
                    if line_idx >= buffer.line_count() {
                        break;
                    }
                    if let Some(line_text) = buffer.line(line_idx) {
                        let line_len = line_text.trim_end_matches('\n').chars().count();
                        if start_col < line_len {
                            let end_col = (start_col + width).min(line_len);
                            buffer.delete_range(line_idx, start_col, line_idx, end_col);
                        }
                    }
                }

                let line_len = buffer
                    .line(start_line)
                    .map(|l| l.trim_end_matches('\n').chars().count())
                    .unwrap_or(0);
                let clamped_col = if line_len > 0 {
                    start_col.min(line_len - 1)
                } else {
                    0
                };
                buffer.cursor_mut().set_position(start_line, clamped_col);
            }
            Self::ChangeVisualBlock {
                line_count,
                width,
                inserted_text,
            } => {
                let start_line = buffer.cursor().line();
                let start_col = buffer.cursor().col();

                // Delete block at current cursor geometry.
                for i in 0..*line_count {
                    let line_idx = start_line + i;
                    if line_idx >= buffer.line_count() {
                        break;
                    }
                    if let Some(line_text) = buffer.line(line_idx) {
                        let line_len = line_text.trim_end_matches('\n').chars().count();
                        if start_col < line_len {
                            let end_col = (start_col + width).min(line_len);
                            buffer.delete_range(line_idx, start_col, line_idx, end_col);
                        }
                    }
                }

                // Reinsert captured text on each selected line.
                if !inserted_text.is_empty() {
                    let initial_line_count = buffer.line_count();
                    for i in 0..*line_count {
                        let line_idx = start_line + i;
                        if line_idx >= initial_line_count {
                            break;
                        }
                        if let Some(line_text) = buffer.line(line_idx) {
                            let line_len = line_text.trim_end_matches('\n').chars().count();
                            let insert_col = start_col.min(line_len);
                            buffer.insert_text_at(line_idx, insert_col, inserted_text);
                        }
                    }

                    let mut final_line = start_line;
                    let mut final_col = start_col;
                    for ch in inserted_text.chars() {
                        if ch == '\n' {
                            final_line += 1;
                            final_col = 0;
                        } else {
                            final_col += 1;
                        }
                    }
                    if final_col > 0 {
                        final_col -= 1;
                    }
                    buffer.cursor_mut().set_position(final_line, final_col);
                } else {
                    let line_len = buffer
                        .line(start_line)
                        .map(|l| l.trim_end_matches('\n').chars().count())
                        .unwrap_or(0);
                    let clamped_col = if line_len > 0 {
                        start_col.min(line_len - 1)
                    } else {
                        0
                    };
                    buffer.cursor_mut().set_position(start_line, clamped_col);
                }
            }
            Self::Change {
                delete,
                inserted_text,
                linewise,
            } => {
                // Inline changes usually insert at the original cursor column,
                // except text objects (ci", ciw, etc.) which insert at the
                // resolved object start after delete.
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
                } else if !matches!(delete.as_ref(), RepeatAction::DeleteTextObject { .. }) {
                    // For non-text-object changes (C, s, c$, etc.), preserve
                    // the original insert point even if delete clamped cursor.
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
