//! Text object handling in normal mode.
//!
//! Handles text objects after an operator with 'i' (inner) or 'a' (around) prefix:
//! diw, daw, di", da", di{, da{, dip, dap, dit, dat, dif, daf, dii, dai, etc.

use crate::editor::input::helpers;
use crate::editor::{
    CursorPos, Editor, InputState, Operator, PendingChangeRepeat, RegisterType, TextObjectPrefix,
    TextObjectRange, TextObjectType, TextObjects,
};
use crate::mode::Mode;
use crate::repeat_action::{CaseTransform, RepeatAction};
use crate::{KeyCode, KeyEvent};
use anyhow::Result;

/// Try to handle a text object after operator + 'i' or 'a'.
///
/// Returns `Ok(true)` if the key was handled, `Ok(false)` otherwise.
pub fn try_handle(editor: &mut Editor, key_event: KeyEvent) -> Result<bool> {
    let InputState::TextObjectPending {
        operator: Some(operator),
        prefix,
    } = *editor.input_state()
    else {
        return Ok(false);
    };
    editor.reset_input_state();
    editor.clear_count();

    let Some(object_type) = super::super::text_objects::from_key(
        editor,
        key_event.code,
        prefix == TextObjectPrefix::Inner,
    ) else {
        return Ok(true);
    };
    let result = if operator == Operator::Change {
        object_type.resolve_for_change(editor.buffer())
    } else {
        object_type.resolve(editor.buffer())
    };
    if let Some(range) = result {
        match operator {
            Operator::Delete => {
                apply_delete_operator(editor, range, object_type)?;
            }
            Operator::Yank => {
                apply_yank_operator(editor, range, key_event.code)?;
            }
            Operator::Change => {
                apply_change_operator(editor, range, object_type)?;
            }
            Operator::Lowercase => {
                apply_case_operator(editor, range, object_type, CaseTransform::Lower)?;
            }
            Operator::Uppercase => {
                apply_case_operator(editor, range, object_type, CaseTransform::Upper)?;
            }
            Operator::ToggleCase => {
                apply_case_operator(editor, range, object_type, CaseTransform::Toggle)?;
            }
            Operator::Fold => {
                let start_line = range.start_line.min(range.end_line);
                let end_line = range.start_line.max(range.end_line);
                editor
                    .buffer_mut()
                    .fold_manager_mut()
                    .create_fold(start_line, end_line);
            }
            Operator::Indent | Operator::Dedent | Operator::AutoIndent => {
                // Shift the lines the object covers (`>ip`, `>i{`, `=ip`).
                let (first, last) = lines_covered(editor, range);
                let cursor_before = editor.cursor_position();
                super::operator_motion::apply_lines(editor, operator, first, last, cursor_before)?;
            }
        }
    }

    Ok(true)
}

/// The lines a text object's range touches: a start at the end of its line (the
/// text after `{`) belongs to the next line, and an exclusive end at column 0
/// does not reach into its line.
fn lines_covered(editor: &Editor, range: TextObjectRange) -> (usize, usize) {
    let buffer = editor.buffer();
    let mut first = range.start_line;
    if first < range.end_line && range.start_col.0 >= buffer.line_len(first) {
        first += 1;
    }
    let mut last = range.end_line;
    if last > first && range.end_col.0 == 0 {
        last -= 1;
    }
    (first, last)
}

fn apply_delete_operator(
    editor: &mut Editor,
    range: TextObjectRange,
    object_type: TextObjectType,
) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let deleted = TextObjects::yank_range(editor.buffer(), range)?;

    // Pattern B: record() + push_recorded_undo() + set_repeat_action()
    let ((), edits) = editor.buffer_mut().record(|buf| {
        buf.delete_range(
            range.start_line,
            range.start_col,
            range.end_line,
            range.end_col,
        );
        buf.set_cursor_char_col(range.start_line, range.start_col);
    });
    let cursor_after = editor.cursor_position();
    if !edits.is_empty() {
        // Paragraph text objects (dip/dap) are linewise — store the register as
        // Line so a subsequent `p` pastes it as whole new lines, mirroring the
        // yank path's `p`-key branch. Otherwise a Character register splices the
        // paragraph into the middle of the current line.
        let reg_type = if matches!(object_type, TextObjectType::Paragraph { .. }) {
            RegisterType::Line
        } else {
            RegisterType::Character
        };
        editor.delete_to_register_with_type(deleted, reg_type);
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
        editor.set_repeat_action(RepeatAction::DeleteTextObject { object_type });
    }
    helpers::clamp_cursor_to_buffer(editor);

    Ok(())
}

fn apply_yank_operator(
    editor: &mut Editor,
    range: TextObjectRange,
    key_code: KeyCode,
) -> Result<()> {
    let yanked = TextObjects::yank_range(editor.buffer(), range)?;
    let reg_type = if key_code == KeyCode::Char('p') {
        RegisterType::Line
    } else {
        RegisterType::Character
    };
    editor.yank_to_register_with_type(yanked, reg_type);
    if reg_type == RegisterType::Line {
        editor.set_yank_flash_lines(range.start_line, range.end_line);
    } else {
        // range cols are char-space (CharCol); flash range takes grapheme
        // cols — convert per line (OV-00299).
        let start_text = editor
            .buffer()
            .line_text(range.start_line)
            .unwrap_or_default()
            .to_string();
        let end_text = editor
            .buffer()
            .line_text(range.end_line)
            .unwrap_or_default()
            .to_string();
        let start_grapheme = crate::unicode::char_to_grapheme_col(&start_text, range.start_col);
        let end_grapheme = crate::unicode::char_to_grapheme_col(&end_text, range.end_col);
        editor.set_yank_flash_range(
            range.start_line,
            start_grapheme,
            range.end_line,
            end_grapheme,
        );
    }
    Ok(())
}

fn apply_change_operator(
    editor: &mut Editor,
    range: TextObjectRange,
    object_type: TextObjectType,
) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = CursorPos::new(cursor.line(), cursor.col());

    let deleted = TextObjects::yank_range(editor.buffer(), range)?;

    let ((), edits) = editor.buffer_mut().record(|buf| {
        buf.delete_range(
            range.start_line,
            range.start_col,
            range.end_line,
            range.end_col,
        );
        buf.set_cursor_char_col(range.start_line, range.start_col);
    });
    let cursor_after = editor.cursor_position();
    let delete_token = if edits.is_empty() {
        editor
            .buffer_mut()
            .change_manager_mut()
            .last_repeat_register = editor.pending_register();
        None
    } else {
        editor.delete_to_register(deleted);
        Some(editor.push_recorded_undo(edits, cursor_before, cursor_after))
    };
    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteTextObject { object_type },
        linewise: false,
        delete_token,
    });

    let new_cursor = editor.buffer().cursor();
    let new_cursor_pos = CursorPos::new(new_cursor.line(), new_cursor.col());
    editor.start_change_building(new_cursor_pos);
    editor.set_mode(Mode::Insert);

    Ok(())
}

fn apply_case_operator(
    editor: &mut Editor,
    range: TextObjectRange,
    object_type: TextObjectType,
    transform: CaseTransform,
) -> Result<()> {
    let text = TextObjects::yank_range(editor.buffer(), range)?;

    let transformed = transform.apply_to(&text);

    // The cursor lands on the start of the text object, changed or not, and the
    // operator is what `.` repeats either way.
    let cursor_before = editor.cursor_position();
    let ((), edits) = editor.buffer_mut().record(|buf| {
        if transformed != text {
            buf.delete_range(
                range.start_line,
                range.start_col,
                range.end_line,
                range.end_col,
            );
            buf.insert_text_at(range.start_line, range.start_col, &transformed);
        }
        buf.set_cursor_char_col(range.start_line, range.start_col);
    });
    if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
    }
    editor.set_repeat_action(RepeatAction::ChangeCaseTextObject {
        object_type,
        transform,
    });

    Ok(())
}
