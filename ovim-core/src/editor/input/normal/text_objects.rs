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
use crate::KeyEvent;
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
    let count = editor.effective_count();
    editor.reset_input_state();
    editor.clear_count();

    let Some(object_type) = super::super::text_objects::from_key(
        editor,
        key_event.code,
        prefix == TextObjectPrefix::Inner,
    ) else {
        return Ok(true);
    };
    // Delete, yank and change act on the range as Vim classifies it (whole lines or not).
    let result =
        match operator {
            Operator::Delete | Operator::Yank | Operator::Change => object_type
                .resolve_for_operator(editor.buffer_mut(), count, operator == Operator::Change),
            _ => object_type
                .resolve_counted(editor.buffer_mut(), count)
                .map(|range| (range, false)),
        };
    if let Some((range, linewise)) = result {
        match operator {
            Operator::Delete => {
                apply_delete_operator(editor, range, object_type, count, linewise)?;
            }
            Operator::Yank => {
                apply_yank_operator(editor, range, linewise)?;
            }
            Operator::Change => {
                apply_change_operator(editor, range, object_type, count, linewise)?;
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
                let (first, last) = range.covered_lines(editor.buffer());
                let cursor_before = editor.cursor_position();
                super::operator_motion::apply_lines(editor, operator, first, last, cursor_before)?;
            }
        }
    }

    Ok(true)
}

fn apply_delete_operator(
    editor: &mut Editor,
    range: TextObjectRange,
    object_type: TextObjectType,
    count: usize,
    linewise: bool,
) -> Result<()> {
    let cursor_before = editor.cursor_position();
    let is_paragraph = matches!(object_type, TextObjectType::Paragraph { .. });

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
        // Linewise objects (dip/dap, di{ over whole lines) go to a Line register so a
        // subsequent `p` pastes them as whole new lines; a Character register would
        // splice them into the middle of the current line.
        let reg_type = if linewise {
            RegisterType::Line
        } else {
            RegisterType::Character
        };
        editor.delete_to_register_with_type(deleted, reg_type);
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
        editor.set_repeat_action(RepeatAction::DeleteTextObject { object_type, count });
    }
    helpers::clamp_cursor_to_buffer(editor);
    if linewise && !is_paragraph {
        // Like `dd`, whole-line deletes leave the cursor on the first non-blank.
        let first_non_blank = editor.buffer().first_non_blank_col(range.start_line);
        editor
            .buffer_mut()
            .set_cursor_char_col(range.start_line, first_non_blank);
    }

    Ok(())
}

fn apply_yank_operator(editor: &mut Editor, range: TextObjectRange, linewise: bool) -> Result<()> {
    let yanked = TextObjects::yank_range(editor.buffer(), range)?;
    let reg_type = if linewise {
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
    // The cursor goes to the start of what was yanked.
    editor
        .buffer_mut()
        .set_cursor_char_col(range.start_line, range.start_col);
    Ok(())
}

fn apply_change_operator(
    editor: &mut Editor,
    range: TextObjectRange,
    object_type: TextObjectType,
    count: usize,
    linewise: bool,
) -> Result<()> {
    // Lines are changed as lines: they become one empty line to type on.
    if linewise {
        let (first, last) = range.covered_lines(editor.buffer());
        return super::operators::change_lines(
            editor,
            first,
            last + 1,
            RepeatAction::DeleteTextObject { object_type, count },
        );
    }

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
        delete_action: RepeatAction::DeleteTextObject { object_type, count },
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
