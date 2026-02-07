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
                let end = (start + line_count).min(buffer.line_count());
                let indent_str = " ".repeat(*tab_width);
                for line_idx in start..end {
                    buffer.insert_text_at(line_idx, 0, &indent_str);
                }
                // Position cursor at first indented line, col = tab_width
                if start < end {
                    buffer.cursor_mut().set_position(start, *tab_width);
                }
            }
            Self::DedentLines {
                line_count,
                tab_width,
            } => {
                let start = buffer.cursor().line();
                let end = (start + line_count).min(buffer.line_count());
                for line_idx in start..end {
                    if let Some(line) = buffer.line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let chars: Vec<char> = line_text.chars().collect();
                        let mut remove = 0;
                        for &ch in chars.iter().take(*tab_width) {
                            if ch == ' ' {
                                remove += 1;
                            } else if ch == '\t' {
                                remove += 1;
                                break;
                            } else {
                                break;
                            }
                        }
                        if remove > 0 {
                            buffer.delete_range(line_idx, 0, line_idx, remove);
                        }
                    }
                }
                // Clamp cursor: line may have shortened
                let cur_line = buffer.cursor().line();
                let cur_col = buffer.cursor().col();
                if let Some(line) = buffer.line(cur_line) {
                    let line_len = line.trim_end_matches('\n').chars().count();
                    if cur_col > 0 && cur_col >= line_len {
                        buffer
                            .cursor_mut()
                            .set_col(if line_len > 0 { line_len - 1 } else { 0 });
                    }
                }
            }
            Self::ToggleCase { count } => {
                for _ in 0..*count {
                    let line_idx = buffer.cursor().line();
                    let col = buffer.cursor().col();
                    let Some(line) = buffer.line(line_idx) else {
                        return;
                    };
                    let line_text = line.trim_end_matches('\n');
                    let chars: Vec<char> = line_text.chars().collect();
                    if col >= chars.len() {
                        return;
                    }

                    let ch = chars[col];
                    let toggled = if ch.is_lowercase() {
                        ch.to_uppercase().to_string()
                    } else {
                        ch.to_lowercase().to_string()
                    };
                    buffer.delete_range(line_idx, col, line_idx, col + 1);
                    buffer.insert_text_at(line_idx, col, &toggled);
                    // Re-read line length: toggling may change char count (e.g. ß → SS)
                    let new_col = col + toggled.chars().count();
                    let new_line_len = buffer
                        .line(line_idx)
                        .map(|l| l.trim_end_matches('\n').chars().count())
                        .unwrap_or(0);
                    if new_col < new_line_len {
                        buffer.cursor_mut().set_col(new_col);
                    } else {
                        break; // At end of line — stop, don't re-toggle same char
                    }
                }
            }
        }
    }
}
