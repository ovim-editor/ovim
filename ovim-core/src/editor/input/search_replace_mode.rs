//! Key handling for the "Replace in files" review panel.

use crate::editor::search_replace::{ReviewRow, SearchReplaceField};
use crate::editor::{Editor, PickerAction};
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

pub fn handle_search_replace_mode(editor: &mut Editor, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(Modifiers::CONTROL);
    let alt = key.modifiers.contains(Modifiers::ALT);
    let Some(focus) = editor.search_replace_panel().map(|panel| panel.focus) else {
        editor.close_search_replace();
        return Ok(());
    };

    // Keys that work from every field.
    match key.code {
        KeyCode::Esc => {
            editor.close_search_replace();
            return Ok(());
        }
        KeyCode::Char('c') if ctrl => {
            editor.close_search_replace();
            return Ok(());
        }
        KeyCode::Tab => {
            with_panel(editor, |panel| panel.next_field());
            return Ok(());
        }
        KeyCode::BackTab => {
            with_panel(editor, |panel| panel.previous_field());
            return Ok(());
        }
        KeyCode::Char('r') if ctrl => return apply(editor),
        KeyCode::Enter if alt => return apply(editor),
        KeyCode::Char('t') if ctrl => {
            with_panel(editor, |panel| panel.toggle_selected());
            return Ok(());
        }
        KeyCode::Char('a') if ctrl => {
            with_panel(editor, |panel| panel.toggle_all());
            return Ok(());
        }
        KeyCode::Char('c') if alt => {
            with_panel(editor, |panel| {
                panel.case_sensitive = !panel.case_sensitive;
                panel.mark_dirty_now();
            });
            return Ok(());
        }
        KeyCode::Char('w') if alt => {
            with_panel(editor, |panel| {
                panel.whole_word = !panel.whole_word;
                panel.mark_dirty_now();
            });
            return Ok(());
        }
        KeyCode::Char('r') if alt => {
            with_panel(editor, |panel| {
                panel.regex = !panel.regex;
                panel.mark_dirty_now();
            });
            return Ok(());
        }
        KeyCode::Down => {
            with_panel(editor, |panel| panel.move_selection(1));
            return Ok(());
        }
        KeyCode::Up => {
            with_panel(editor, |panel| panel.move_selection(-1));
            return Ok(());
        }
        KeyCode::Char('n') if ctrl => {
            with_panel(editor, |panel| panel.move_selection(1));
            return Ok(());
        }
        KeyCode::Char('p') if ctrl => {
            with_panel(editor, |panel| panel.move_selection(-1));
            return Ok(());
        }
        KeyCode::PageDown => {
            with_panel(editor, |panel| panel.move_selection(10));
            return Ok(());
        }
        KeyCode::PageUp => {
            with_panel(editor, |panel| panel.move_selection(-10));
            return Ok(());
        }
        _ => {}
    }

    if focus == SearchReplaceField::Results {
        match key.code {
            KeyCode::Char('j') => with_panel(editor, |panel| panel.move_selection(1)),
            KeyCode::Char('k') => with_panel(editor, |panel| panel.move_selection(-1)),
            KeyCode::Char('g') => with_panel(editor, |panel| panel.selected = 0),
            KeyCode::Char('G') => with_panel(editor, |panel| {
                panel.selected = panel.row_count().saturating_sub(1)
            }),
            KeyCode::Char(' ') | KeyCode::Char('x') => {
                with_panel(editor, |panel| panel.toggle_selected())
            }
            KeyCode::Char('a') => with_panel(editor, |panel| panel.toggle_all()),
            KeyCode::Char('A') => return apply(editor),
            KeyCode::Char('/') | KeyCode::Char('i') => {
                with_panel(editor, |panel| panel.focus = SearchReplaceField::Find)
            }
            KeyCode::Enter => return jump_to_selection(editor),
            _ => {}
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Enter => {
            with_panel(editor, |panel| {
                if panel.is_pending() && focus != SearchReplaceField::Replace {
                    panel.mark_dirty_now();
                }
                panel.focus = SearchReplaceField::Results;
            });
        }
        KeyCode::Backspace => edit_input(editor, focus, |input| input.backspace()),
        KeyCode::Delete => edit_input(editor, focus, |input| input.delete()),
        KeyCode::Left => edit_input(editor, focus, |input| {
            input.move_left();
            false
        }),
        KeyCode::Right => edit_input(editor, focus, |input| {
            input.move_right();
            false
        }),
        KeyCode::Home => edit_input(editor, focus, |input| {
            input.move_home();
            false
        }),
        KeyCode::End => edit_input(editor, focus, |input| {
            input.move_end();
            false
        }),
        KeyCode::Char(c) if !ctrl && !alt => edit_input(editor, focus, |input| input.insert(c)),
        _ => {}
    }
    Ok(())
}

fn with_panel(
    editor: &mut Editor,
    f: impl FnOnce(&mut crate::editor::search_replace::SearchReplacePanel),
) {
    if let Some(panel) = editor.search_replace_panel_mut() {
        f(panel);
    }
    editor.mark_dirty();
}

fn edit_input(
    editor: &mut Editor,
    focus: SearchReplaceField,
    f: impl FnOnce(&mut crate::editor::SingleLineInput) -> bool,
) {
    if let Some(panel) = editor.search_replace_panel_mut() {
        let changed = panel.active_input_mut().map(f).unwrap_or(false);
        // The replacement text only changes the preview, not the matches.
        if changed && focus != SearchReplaceField::Replace {
            panel.mark_dirty();
        }
    }
    editor.mark_dirty();
}

fn apply(editor: &mut Editor) -> Result<()> {
    match editor.apply_search_replace() {
        Ok(report) => {
            editor.discard_search_replace();
            editor.set_status_message(report.summary());
        }
        Err(message) => editor.set_status_message(message),
    }
    Ok(())
}

fn jump_to_selection(editor: &mut Editor) -> Result<()> {
    let target = editor
        .search_replace_panel()
        .and_then(|panel| match panel.selected_row()? {
            ReviewRow::File(file) => {
                let file = &panel.results[file];
                file.matches
                    .first()
                    .map(|m| (file.path.clone(), m.found.line, m.found.start_col))
            }
            ReviewRow::Match(file, m) => {
                let file = &panel.results[file];
                Some((
                    file.path.clone(),
                    file.matches[m].found.line,
                    file.matches[m].found.start_col,
                ))
            }
        });
    if let Some((path, line, col)) = target {
        editor.close_search_replace();
        editor.execute_picker_action(PickerAction::OpenFile {
            path: path.to_string_lossy().to_string(),
            line,
            col,
        })?;
    }
    Ok(())
}
