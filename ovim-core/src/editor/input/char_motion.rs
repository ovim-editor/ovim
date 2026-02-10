//! Character motion handler for f, t, F, T motions.
//!
//! This module handles the AwaitingChar input state, processing the target
//! character for find/till motions. It also handles operator combinations
//! like df, dt, cf, ct.

use crate::{KeyCode, KeyEvent};
use anyhow::Result;

use crate::editor::input_state::CharMotion;
use crate::editor::motions::Motions;
use crate::editor::operators::Operator;
use crate::editor::RegisterType;
use crate::editor::{Change, Editor, FindDirection, FindType, Range};
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
        // Mark and replace operations - delegate to legacy handler for now
        CharMotion::Replace
        | CharMotion::Mark
        | CharMotion::JumpMarkLine
        | CharMotion::JumpMarkExact => {
            // These will be handled by the legacy pending_command system for now
            // TODO: Migrate these to new state machine
        }
    }

    // Clear state
    editor.reset_input_state();
    editor.clear_pending_operator();
    editor.clear_count();

    Ok(())
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
            let deleted = editor
                .buffer_mut()
                .delete_range(start_line, start_col, end_line, end_col);

            if !deleted.is_empty() {
                let range = Range::new((start_line, start_col), (end_line, end_col));
                let change = Change::delete(range, deleted.clone(), cursor_before);
                editor.delete_to_register_with_type(deleted, RegisterType::Character);
                editor.add_change(change);
                editor.mark_buffer_modified();
            }

            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(start_line, start_col);

            // Group subsequent insert-mode edits into a composite change.
            editor.start_change_building(cursor_before);
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
