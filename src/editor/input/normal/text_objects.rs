//! Text object handling in normal mode.
//!
//! Handles text objects after an operator with 'i' (inner) or 'a' (around) prefix:
//! diw, daw, di", da", di{, da{, dip, dap, dit, dat, dif, daf, dii, dai, etc.

use crate::editor::input::helpers;
use crate::editor::{
    Change, Editor, Operator, PendingSemanticChange, Range, RegisterType, TextObjectRange,
    TextObjectType, TextObjects,
};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

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
    let object_type: Option<TextObjectType> = match key_event.code {
        KeyCode::Char('w') => Some(TextObjectType::Word {
            inner: text_obj_type == 'i',
        }),
        KeyCode::Char('"') | KeyCode::Char('\'') | KeyCode::Char('`') => {
            let quote = match key_event.code {
                KeyCode::Char(c) => c,
                _ => unreachable!(),
            };
            Some(TextObjectType::Quote {
                char: quote,
                inner: text_obj_type == 'i',
            })
        }
        KeyCode::Char('(') | KeyCode::Char(')') | KeyCode::Char('b') => {
            Some(TextObjectType::Paired {
                open: '(',
                close: ')',
                inner: text_obj_type == 'i',
            })
        }
        KeyCode::Char('[') | KeyCode::Char(']') => Some(TextObjectType::Paired {
            open: '[',
            close: ']',
            inner: text_obj_type == 'i',
        }),
        KeyCode::Char('{') | KeyCode::Char('}') | KeyCode::Char('B') => {
            Some(TextObjectType::Paired {
                open: '{',
                close: '}',
                inner: text_obj_type == 'i',
            })
        }
        KeyCode::Char('<') | KeyCode::Char('>') => Some(TextObjectType::Paired {
            open: '<',
            close: '>',
            inner: text_obj_type == 'i',
        }),
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
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());

    let deleted = TextObjects::yank_range(editor.buffer(), range)?;

    let change_range = Range::new(
        (range.start_line, range.start_col),
        (range.end_line, range.end_col),
    );

    let change = if let Some(obj_type) = object_type {
        let cursor_after = (range.start_line, range.start_col);
        Change::delete_text_object(obj_type, cursor_before, cursor_after, deleted.clone(), change_range)
    } else {
        Change::delete(change_range, deleted.clone(), cursor_before)
    };

    change.apply(editor.buffer_mut());
    editor.delete_to_register(deleted);
    editor.add_change(change);
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

    editor.buffer_mut().delete_range(
        range.start_line,
        range.start_col,
        range.end_line,
        range.end_col,
    );
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(range.start_line, range.start_col);

    editor.delete_to_register(deleted.clone());

    if let Some(obj_type) = object_type {
        editor.set_pending_semantic_change(PendingSemanticChange {
            object_type: Some(obj_type),
            is_word_change: false,
            is_search_match_change: false,
            search_pattern: None,
            search_forward: None,
            old_text: deleted,
            old_range: change_range,
            cursor_before,
        });
    } else {
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

fn apply_case_operator(
    editor: &mut Editor,
    range: TextObjectRange,
    case_op: CaseOp,
) -> Result<()> {
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
        let deleted = editor.buffer_mut().delete_range(
            range.start_line,
            range.start_col,
            range.end_line,
            range.end_col,
        );
        let delete_range = Range::new(
            (range.start_line, range.start_col),
            (range.end_line, range.end_col),
        );
        let delete_change = Change::delete(delete_range, deleted, cursor_before);

        let insert_change = Change::insert((range.start_line, range.start_col), transformed, cursor_before);
        insert_change.apply(editor.buffer_mut());

        editor.add_change(delete_change);
        editor.add_change(insert_change);
    }

    Ok(())
}
