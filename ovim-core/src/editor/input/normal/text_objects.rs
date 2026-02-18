//! Text object handling in normal mode.
//!
//! Handles text objects after an operator with 'i' (inner) or 'a' (around) prefix:
//! diw, daw, di", da", di{, da{, dip, dap, dit, dat, dif, daf, dii, dai, etc.

use crate::editor::input::helpers;
use crate::editor::{
    Change, Editor, Operator, PendingChangeRepeat, Range, RegisterType, TextObjectRange,
    TextObjectType, TextObjects,
};
use crate::mode::Mode;
use crate::repeat_action::RepeatAction;
use crate::{KeyCode, KeyEvent};
use anyhow::Result;

/// Try to handle a text object after operator + 'i' or 'a'.
///
/// Returns `Ok(true)` if the key was handled, `Ok(false)` otherwise.
pub fn try_handle(editor: &mut Editor, key_event: KeyEvent) -> Result<bool> {
    let text_obj_type = match editor.pending_command() {
        Some('i') | Some('a') => editor.pending_command().unwrap(),
        _ => return Ok(false),
    };

    let operator = match editor.pending_operator() {
        Some(op) => op,
        None => return Ok(false),
    };

    editor.clear_pending_command();
    editor.clear_pending_operator();
    editor.clear_count();

    let result = match key_event.code {
        KeyCode::Char('w') => {
            if text_obj_type == 'i' {
                TextObjects::inner_word(editor.buffer())
            } else {
                TextObjects::around_word(editor.buffer())
            }
        }
        KeyCode::Char('W') => {
            if text_obj_type == 'i' {
                TextObjects::inner_big_word(editor.buffer())
            } else {
                TextObjects::around_big_word(editor.buffer())
            }
        }
        KeyCode::Char('p') => {
            if text_obj_type == 'i' {
                TextObjects::inner_paragraph(editor.buffer())
            } else {
                TextObjects::around_paragraph(editor.buffer())
            }
        }
        KeyCode::Char('s') => {
            if text_obj_type == 'i' {
                TextObjects::inner_sentence(editor.buffer())
            } else {
                TextObjects::around_sentence(editor.buffer())
            }
        }
        KeyCode::Char('"') | KeyCode::Char('\'') | KeyCode::Char('`') => {
            let quote = match key_event.code {
                KeyCode::Char(c) => c,
                _ => unreachable!(),
            };
            TextObjects::quoted_string(editor.buffer(), quote, text_obj_type == 'a')
        }
        KeyCode::Char('(') | KeyCode::Char(')') | KeyCode::Char('b') => {
            TextObjects::paired_delimiters(editor.buffer(), '(', ')', text_obj_type == 'a')
        }
        KeyCode::Char('[') | KeyCode::Char(']') => {
            TextObjects::paired_delimiters(editor.buffer(), '[', ']', text_obj_type == 'a')
        }
        KeyCode::Char('{') | KeyCode::Char('}') | KeyCode::Char('B') => {
            TextObjects::paired_delimiters(editor.buffer(), '{', '}', text_obj_type == 'a')
        }
        KeyCode::Char('<') | KeyCode::Char('>') => {
            TextObjects::paired_delimiters(editor.buffer(), '<', '>', text_obj_type == 'a')
        }
        KeyCode::Char('t') => TextObjects::tag(editor.buffer(), text_obj_type == 'a'),
        KeyCode::Char('i') => {
            let tab_width = editor.options.tab_width;
            if text_obj_type == 'i' {
                TextObjects::inner_indent(editor.buffer(), tab_width)
            } else {
                TextObjects::around_indent(editor.buffer(), tab_width)
            }
        }
        KeyCode::Char('f') => {
            if text_obj_type == 'i' {
                TextObjects::inner_function(editor.buffer())
            } else {
                TextObjects::around_function(editor.buffer())
            }
        }
        _ => {
            // Unknown text object
            return Ok(true);
        }
    };

    // Determine the TextObjectType for semantic repeat
    let inner = text_obj_type == 'i';
    let object_type: Option<TextObjectType> = match key_event.code {
        KeyCode::Char('w') => Some(TextObjectType::Word { inner, big: false }),
        KeyCode::Char('W') => Some(TextObjectType::Word { inner, big: true }),
        KeyCode::Char('"') | KeyCode::Char('\'') | KeyCode::Char('`') => {
            let quote = match key_event.code {
                KeyCode::Char(c) => c,
                _ => unreachable!(),
            };
            Some(TextObjectType::Quote { char: quote, inner })
        }
        KeyCode::Char('(') | KeyCode::Char(')') | KeyCode::Char('b') => {
            Some(TextObjectType::Paired {
                open: '(',
                close: ')',
                inner,
            })
        }
        KeyCode::Char('[') | KeyCode::Char(']') => Some(TextObjectType::Paired {
            open: '[',
            close: ']',
            inner,
        }),
        KeyCode::Char('{') | KeyCode::Char('}') | KeyCode::Char('B') => {
            Some(TextObjectType::Paired {
                open: '{',
                close: '}',
                inner,
            })
        }
        KeyCode::Char('<') | KeyCode::Char('>') => Some(TextObjectType::Paired {
            open: '<',
            close: '>',
            inner,
        }),
        KeyCode::Char('p') => Some(TextObjectType::Paragraph { inner }),
        KeyCode::Char('s') => Some(TextObjectType::Sentence { inner }),
        KeyCode::Char('t') => Some(TextObjectType::Tag { inner }),
        KeyCode::Char('i') => Some(TextObjectType::Indent {
            inner,
            tab_width: editor.options.tab_width,
        }),
        KeyCode::Char('f') => Some(TextObjectType::Function { inner }),
        _ => None,
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
                apply_case_operator(editor, range, CaseOp::Lower)?;
            }
            Operator::Uppercase => {
                apply_case_operator(editor, range, CaseOp::Upper)?;
            }
            Operator::ToggleCase => {
                apply_case_operator(editor, range, CaseOp::Toggle)?;
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
                // Don't make sense with text objects
            }
        }
    }

    Ok(true)
}

fn apply_delete_operator(
    editor: &mut Editor,
    range: TextObjectRange,
    object_type: Option<TextObjectType>,
) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let deleted = TextObjects::yank_range(editor.buffer(), range)?;

    if let Some(obj_type) = object_type {
        // Pattern B: record() + push_recorded_undo() + set_repeat_action()
        let ((), edits) = editor.buffer_mut().record(|buf| {
            buf.delete_range(
                range.start_line,
                range.start_col,
                range.end_line,
                range.end_col,
            );
            buf.cursor_mut()
                .set_position(range.start_line, range.start_col);
        });
        let cursor_after = editor.cursor_position();
        if !edits.is_empty() {
            editor.delete_to_register(deleted);
            editor.push_recorded_undo(edits, cursor_before, cursor_after);
            editor.set_repeat_action(RepeatAction::DeleteTextObject {
                object_type: obj_type,
            });
        }
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
        editor.set_yank_flash_range(
            range.start_line,
            range.start_col,
            range.end_line,
            range.end_col,
        );
    }
    Ok(())
}

fn apply_change_operator(
    editor: &mut Editor,
    range: TextObjectRange,
    object_type: Option<TextObjectType>,
) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());

    let deleted = TextObjects::yank_range(editor.buffer(), range)?;

    let change_range = Range::new(
        (range.start_line, range.start_col),
        (range.end_line, range.end_col),
    );

    if let Some(obj_type) = object_type {
        let ((), edits) = editor.buffer_mut().record(|buf| {
            buf.delete_range(
                range.start_line,
                range.start_col,
                range.end_line,
                range.end_col,
            );
            buf.cursor_mut()
                .set_position(range.start_line, range.start_col);
        });
        if edits.is_empty() {
            return Ok(());
        }
        editor.delete_to_register(deleted);
        let cursor_after = editor.cursor_position();
        let delete_token =
            editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after);
        editor.set_pending_change_repeat(PendingChangeRepeat {
            delete_action: RepeatAction::DeleteTextObject {
                object_type: obj_type,
            },
            linewise: false,
            delete_token: Some(delete_token),
        });
    } else {
        let version_before = editor.buffer().version();
        editor.buffer_mut().delete_range(
            range.start_line,
            range.start_col,
            range.end_line,
            range.end_col,
        );
        if editor.buffer().version() == version_before {
            return Ok(());
        }
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(range.start_line, range.start_col);
        editor.delete_to_register(deleted.clone());
        let change = Change::delete(change_range, deleted, cursor_before);
        editor.add_change(change);
    }

    let new_cursor = editor.buffer().cursor();
    let new_cursor_pos = (new_cursor.line(), new_cursor.col());
    editor.start_change_building(new_cursor_pos);
    editor.set_mode(Mode::Insert);

    Ok(())
}

enum CaseOp {
    Lower,
    Upper,
    Toggle,
}

fn apply_case_operator(editor: &mut Editor, range: TextObjectRange, case_op: CaseOp) -> Result<()> {
    let cursor_before = (
        editor.buffer().cursor().line(),
        editor.buffer().cursor().col(),
    );

    let text = TextObjects::yank_range(editor.buffer(), range)?;

    let transformed = match case_op {
        CaseOp::Lower => text.to_lowercase(),
        CaseOp::Upper => text.to_uppercase(),
        CaseOp::Toggle => text
            .chars()
            .map(|ch| {
                if ch.is_lowercase() {
                    ch.to_uppercase().to_string()
                } else {
                    ch.to_lowercase().to_string()
                }
            })
            .collect(),
    };

    if transformed != text {
        let version_before = editor.buffer().version();
        let deleted = editor.buffer_mut().delete_range(
            range.start_line,
            range.start_col,
            range.end_line,
            range.end_col,
        );
        if editor.buffer().version() == version_before {
            return Ok(());
        }

        let delete_range = Range::new(
            (range.start_line, range.start_col),
            (range.end_line, range.end_col),
        );
        let delete_change = Change::delete(delete_range, deleted, cursor_before);

        let insert_change = Change::insert(
            (range.start_line, range.start_col),
            transformed,
            cursor_before,
        );
        let version_before_insert = editor.buffer().version();
        insert_change.apply(editor.buffer_mut());

        let cursor_after = (
            editor.buffer().cursor().line(),
            editor.buffer().cursor().col(),
        );
        let mut changes = vec![delete_change];
        if editor.buffer().version() != version_before_insert {
            changes.push(insert_change);
        }
        let composite = Change::composite(changes, cursor_before, cursor_after);
        editor.add_change(composite);
    }

    Ok(())
}
