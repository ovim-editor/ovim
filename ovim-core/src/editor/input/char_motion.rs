//! Character motion handler for f/t/F/T and other awaiting-char commands.
//!
//! This module handles the AwaitingChar input state, processing the target
//! character for find/till motions. It also handles operator combinations
//! like df, dt, cf, ct, plus r/m/'/` commands.

use crate::{KeyCode, KeyEvent};
use anyhow::Result;

use crate::editor::editing_state::PendingChangeRepeat;
use crate::editor::input_state::CharMotion;
use crate::editor::motions::Motions;
use crate::editor::operators::Operator;
use crate::editor::RegisterType;
use crate::editor::{Editor, FindDirection, FindType};
use crate::mode::Mode;
use crate::repeat_action::RepeatAction;

/// Handles the second key in a character motion sequence.
///
/// Called when the editor is in `InputState::AwaitingChar` state.
/// The `motion` indicates what type of motion (f/t/F/T),
/// and `operator` indicates if there's a pending operator (for df/dt/cf/ct).
pub fn handle_char_motion(
    editor: &mut Editor,
    key: KeyEvent,
    motion: CharMotion,
    operator: Option<Operator>,
) -> Result<()> {
    // Handle Escape - cancel the motion
    if key.code == KeyCode::Esc {
        editor.reset_input_state();
        editor.clear_pending_operator();
        editor.clear_count();
        return Ok(());
    }

    // We need a character to proceed
    let KeyCode::Char(target) = key.code else {
        // Non-character key - cancel
        editor.reset_input_state();
        return Ok(());
    };

    let count = editor.effective_count();

    match motion {
        CharMotion::Find => handle_find_forward(editor, target, count, operator),
        CharMotion::Till => handle_till_forward(editor, target, count, operator),
        CharMotion::FindBack => handle_find_backward(editor, target, count, operator),
        CharMotion::TillBack => handle_till_backward(editor, target, count, operator),
        CharMotion::Replace => handle_replace_char(editor, target, count)?,
        CharMotion::Mark => handle_set_mark(editor, target),
        CharMotion::JumpMarkLine => {
            if operator.is_some() {
                handle_mark_operator(editor, target, operator, true);
            } else {
                handle_jump_mark_line(editor, target);
            }
        }
        CharMotion::JumpMarkExact => {
            if operator.is_some() {
                handle_mark_operator(editor, target, operator, false);
            } else {
                handle_jump_mark_exact(editor, target);
            }
        }
    }

    // Clear state
    editor.reset_input_state();
    editor.clear_pending_operator();
    editor.clear_count();

    Ok(())
}

fn handle_replace_char(editor: &mut Editor, target: char, count: usize) -> Result<()> {
    if editor.mode().is_visual() {
        crate::editor::input::helpers::replace_visual_selection(editor, target)?;
        crate::editor::input::helpers::exit_visual_mode_to_normal(editor);
        return Ok(());
    }

    editor.record_operation(
        |buf| buf.replace_chars_at_cursor(target, count),
        Some(RepeatAction::ReplaceChar { ch: target, count }),
    );
    Ok(())
}

fn handle_set_mark(editor: &mut Editor, target: char) {
    if target.is_ascii_lowercase() || target.is_ascii_uppercase() {
        editor.set_mark(target);
    }
}

/// Handle mark motions with a pending operator (d'a, d`a, y'a, y`a, c'a, c`a).
///
/// `line_wise` = true for `'` (line-wise), false for `` ` `` (character-wise).
fn handle_mark_operator(
    editor: &mut Editor,
    target: char,
    operator: Option<Operator>,
    line_wise: bool,
) {
    let Some(op) = operator else {
        return;
    };

    // Get mark position without jumping
    let mark_pos = if target.is_ascii_lowercase() {
        editor
            .nav
            .marks
            .get_mark(target)
            .map(|m| (m.line, m.col))
    } else {
        // Only local marks supported with operators for now.
        // Global marks (A-Z) involve file switching.
        None
    };

    let Some((mark_line, mark_col)) = mark_pos else {
        return; // Mark not set
    };

    let cursor_line = editor.buffer().cursor().line();
    let cursor_col = editor.buffer().cursor().col();

    if line_wise {
        // Line-wise operation (d'a, y'a, c'a)
        let start_line = cursor_line.min(mark_line);
        let end_line = cursor_line.max(mark_line);
        let line_count = end_line - start_line + 1;

        match op {
            Operator::Delete => {
                // Move cursor to start_line so delete_lines operates on the right range
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(start_line, 0);
                let deleted = editor.record_operation(
                    |buf| buf.delete_lines(line_count),
                    None,
                );
                if !deleted.is_empty() {
                    editor.delete_to_register_with_type(deleted, RegisterType::Line);
                }
            }
            Operator::Yank => {
                let mut yanked = String::new();
                for line_idx in start_line..=end_line {
                    if let Some(line) = editor.buffer().line(line_idx) {
                        yanked.push_str(&line);
                    }
                }
                editor.yank_to_register_with_type(yanked, RegisterType::Line);
                editor.set_yank_flash_lines(start_line, end_line);
            }
            Operator::Change => {
                // Move cursor to start_line, then delete lines and enter insert
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(start_line, 0);

                let indent = editor
                    .buffer()
                    .line(start_line)
                    .map(|l| {
                        l.chars()
                            .take_while(|c| c.is_whitespace() && *c != '\n')
                            .collect::<String>()
                    })
                    .unwrap_or_default();

                let deleted = editor.record_operation(
                    |buf| {
                        let d = buf.delete_lines(line_count);
                        buf.insert_text_at(start_line, 0, &format!("{}\n", indent));
                        buf.cursor_mut().set_position(start_line, indent.len());
                        d
                    },
                    None,
                );
                if !deleted.is_empty() {
                    editor.delete_to_register_with_type(deleted, RegisterType::Line);
                }
                editor.start_change_building(editor.cursor_position());
                editor.set_mode(Mode::Insert);
            }
            _ => {}
        }
    } else {
        // Character-wise operation (d`a, y`a, c`a)
        let (start_line, start_col, end_line, end_col) =
            if cursor_line < mark_line || (cursor_line == mark_line && cursor_col <= mark_col) {
                (cursor_line, cursor_col, mark_line, mark_col)
            } else {
                (mark_line, mark_col, cursor_line, cursor_col)
            };

        // Vim treats backtick mark motions as exclusive for operators
        let end_col_exclusive = end_col;

        let cursor_before = (cursor_line, cursor_col);

        match op {
            Operator::Delete => {
                let (deleted, edits) = editor.buffer_mut().record(|buf| {
                    let d = buf.delete_range(start_line, start_col, end_line, end_col_exclusive);
                    buf.cursor_mut().set_position(start_line, start_col);
                    d
                });
                if !edits.is_empty() {
                    let cursor_after = editor.cursor_position();
                    editor.push_recorded_undo(edits, cursor_before, cursor_after);
                    if !deleted.is_empty() {
                        editor.delete_to_register_with_type(deleted, RegisterType::Character);
                    }
                }
            }
            Operator::Yank => {
                let start_char = editor.buffer().rope().line_to_char(start_line) + start_col;
                let end_char = editor.buffer().rope().line_to_char(end_line) + end_col_exclusive;
                if end_char > start_char {
                    let yanked = editor
                        .buffer()
                        .rope()
                        .slice(start_char..end_char)
                        .to_string();
                    editor.yank_to_register_with_type(yanked, RegisterType::Character);
                    editor.set_yank_flash_range(
                        start_line,
                        start_col,
                        end_line,
                        end_col_exclusive.saturating_sub(1),
                    );
                }
            }
            Operator::Change => {
                let (deleted, edits) = editor.buffer_mut().record(|buf| {
                    let d = buf.delete_range(start_line, start_col, end_line, end_col_exclusive);
                    buf.cursor_mut().set_position(start_line, start_col);
                    d
                });
                if !edits.is_empty() {
                    let cursor_after = editor.cursor_position();
                    let token = editor.push_recorded_undo_returning_token(
                        edits,
                        cursor_before,
                        cursor_after,
                    );
                    if !deleted.is_empty() {
                        editor.delete_to_register_with_type(deleted, RegisterType::Character);
                    }
                    editor.set_pending_change_repeat(PendingChangeRepeat {
                        delete_action: RepeatAction::DeleteCharMotion {
                            target: '\0',
                            forward: true,
                            till: false,
                            count: 1,
                        },
                        linewise: false,
                        delete_token: Some(token),
                    });
                    editor.start_change_building(editor.cursor_position());
                    editor.set_mode(Mode::Insert);
                }
            }
            _ => {}
        }
    }
}

fn handle_jump_mark_line(editor: &mut Editor, target: char) {
    if target == '\'' {
        editor.jump_back();
        return;
    }

    if !(target.is_ascii_lowercase() || target.is_ascii_uppercase() || matches!(target, '.' | '^'))
    {
        return;
    }

    editor.add_jump();
    let _ = editor.jump_to_mark_line(target);
}

fn handle_jump_mark_exact(editor: &mut Editor, target: char) {
    if target == '`' {
        editor.jump_back();
        return;
    }

    if !(target.is_ascii_lowercase()
        || target.is_ascii_uppercase()
        || matches!(target, '.' | '^' | '[' | ']'))
    {
        return;
    }

    editor.add_jump();
    let _ = editor.jump_to_mark(target);
}

/// Handles `f{char}` - find character forward
fn handle_find_forward(
    editor: &mut Editor,
    target: char,
    count: usize,
    operator: Option<Operator>,
) {
    let start_line = editor.buffer().cursor().line();
    let start_col = editor.buffer().cursor().col();
    let cursor_before = (start_line, start_col);

    let moved = Motions::find_char_forward(editor.buffer_mut(), target, count);

    if moved {
        // Store for ; and , repeat
        editor.set_last_find(target, FindType::Find, FindDirection::Forward);

        // Apply operator if pending
        if let Some(op) = operator {
            let end_line = editor.buffer().cursor().line();
            let end_col = editor.buffer().cursor().col();
            apply_operator_to_range(
                editor,
                op,
                cursor_before,
                target,
                true,
                false,
                count,
                start_line,
                start_col,
                end_line,
                end_col,
                true,
            );
        }
    }
}

/// Handles `t{char}` - till character forward
fn handle_till_forward(
    editor: &mut Editor,
    target: char,
    count: usize,
    operator: Option<Operator>,
) {
    let start_line = editor.buffer().cursor().line();
    let start_col = editor.buffer().cursor().col();
    let cursor_before = (start_line, start_col);

    let moved = Motions::till_char_forward(editor.buffer_mut(), target, count);

    if moved {
        // Store for ; and , repeat
        editor.set_last_find(target, FindType::Till, FindDirection::Forward);

        // Apply operator if pending
        if let Some(op) = operator {
            let end_line = editor.buffer().cursor().line();
            let end_col = editor.buffer().cursor().col();
            apply_operator_to_range(
                editor,
                op,
                cursor_before,
                target,
                true,
                true,
                count,
                start_line,
                start_col,
                end_line,
                end_col,
                true,
            );
        }
    }
}

/// Handles `F{char}` - find character backward
fn handle_find_backward(
    editor: &mut Editor,
    target: char,
    count: usize,
    operator: Option<Operator>,
) {
    let start_line = editor.buffer().cursor().line();
    let start_col = editor.buffer().cursor().col();
    let cursor_before = (start_line, start_col);

    let moved = Motions::find_char_backward(editor.buffer_mut(), target, count);

    if moved {
        editor.set_last_find(target, FindType::Find, FindDirection::Backward);

        if let Some(op) = operator {
            let end_line = editor.buffer().cursor().line();
            let end_col = editor.buffer().cursor().col();
            // For backward motions, the end position is before start
            apply_operator_to_range(
                editor,
                op,
                cursor_before,
                target,
                false,
                false,
                count,
                end_line,
                end_col,
                start_line,
                start_col,
                true,
            );
        }
    }
}

/// Handles `T{char}` - till character backward
fn handle_till_backward(
    editor: &mut Editor,
    target: char,
    count: usize,
    operator: Option<Operator>,
) {
    let start_line = editor.buffer().cursor().line();
    let start_col = editor.buffer().cursor().col();
    let cursor_before = (start_line, start_col);

    let moved = Motions::till_char_backward(editor.buffer_mut(), target, count);

    if moved {
        editor.set_last_find(target, FindType::Till, FindDirection::Backward);

        if let Some(op) = operator {
            let end_line = editor.buffer().cursor().line();
            let end_col = editor.buffer().cursor().col();
            apply_operator_to_range(
                editor,
                op,
                cursor_before,
                target,
                false,
                true,
                count,
                end_line,
                end_col,
                start_line,
                start_col,
                true,
            );
        }
    }
}

/// Applies an operator to a character range.
///
/// This handles df, dt, cf, ct, yf, yt, etc.
fn apply_operator_to_range(
    editor: &mut Editor,
    operator: Operator,
    cursor_before: (usize, usize),
    target: char,
    forward: bool,
    till: bool,
    count: usize,
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
    inclusive: bool,
) {
    // For inclusive motions (f), include the target character by making the end column exclusive.
    let mut end_col = if inclusive {
        end_col.saturating_add(1)
    } else {
        end_col
    };

    // Clamp end_col to the line length to avoid overflow/past-EOL issues.
    if let Some(line) = editor.buffer().line(end_line) {
        let line_len = line.trim_end_matches('\n').chars().count();
        end_col = end_col.min(line_len);
    }

    match operator {
        Operator::Delete => {
            let (deleted, edits) = editor.buffer_mut().record(|buf| {
                let d = buf.delete_range(start_line, start_col, end_line, end_col);
                buf.cursor_mut().set_position(start_line, start_col);
                d
            });
            let cursor_after = editor.cursor_position();

            if !edits.is_empty() {
                if !deleted.is_empty() {
                    editor.delete_to_register_with_type(deleted, RegisterType::Character);
                }
                // push_recorded_undo() calls mark_buffer_modified() internally
                editor.push_recorded_undo(edits, cursor_before, cursor_after);
                editor.set_repeat_action(RepeatAction::DeleteCharMotion {
                    target,
                    forward,
                    till,
                    count,
                });
            }
        }
        Operator::Change => {
            let (deleted, edits) = editor.buffer_mut().record(|buf| {
                let d = buf.delete_range(start_line, start_col, end_line, end_col);
                buf.cursor_mut().set_position(start_line, start_col);
                d
            });
            let delete_token = if !edits.is_empty() {
                let cursor_after = editor.cursor_position();
                let token =
                    editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after);
                if !deleted.is_empty() {
                    editor.delete_to_register_with_type(deleted, RegisterType::Character);
                }
                Some(token)
            } else {
                None
            };

            editor.set_pending_change_repeat(PendingChangeRepeat {
                delete_action: RepeatAction::DeleteCharMotion {
                    target,
                    forward,
                    till,
                    count,
                },
                linewise: false,
                delete_token,
            });
            editor.start_change_building(editor.cursor_position());
            editor.set_mode(Mode::Insert);
        }
        Operator::Yank => {
            let start_char = editor.buffer().rope().line_to_char(start_line) + start_col;
            let end_char = editor.buffer().rope().line_to_char(end_line) + end_col;
            if end_char > start_char {
                let yanked = editor
                    .buffer()
                    .rope()
                    .slice(start_char..end_char)
                    .to_string();
                editor.yank_to_register_with_type(yanked, RegisterType::Character);
                editor.set_yank_flash_range(
                    start_line,
                    start_col,
                    end_line,
                    end_col.saturating_sub(1),
                );
            }
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(start_line, start_col);
        }
        // Other operators (indent, etc.) typically don't apply to char motions
        _ => {}
    }
}
