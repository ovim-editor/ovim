//! Picker mode input handling
//!
//! Handles file finder, grep, code actions, and LSP location pickers.
//! Supports navigation (j/k, Ctrl-N/P, arrows), query editing, and selection.

use crate::editor::Editor;
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handles input in Picker mode
pub fn handle_picker_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // Ctrl-C - cancel picker (same as Escape)
    if key_event.code == KeyCode::Char('c') && key_event.modifiers.contains(KeyModifiers::CONTROL) {
        editor.close_picker();
        editor.set_mode(Mode::Normal);
        return Ok(());
    }

    match key_event.code {
        // Escape - cancel picker
        KeyCode::Esc => {
            editor.close_picker();
            editor.set_mode(Mode::Normal);
        }
        // Tab / BackTab - toggle between query and file filter fields
        KeyCode::Tab | KeyCode::BackTab => {
            if let Some(picker) = editor.picker_mut() {
                picker.toggle_field();
            }
        }
        // Alt+Right / Alt+Left (macOS) or Ctrl+Right / Ctrl+Left (other) - toggle field
        KeyCode::Right
            if (cfg!(target_os = "macos") && key_event.modifiers.contains(KeyModifiers::ALT))
                || (!cfg!(target_os = "macos")
                    && key_event.modifiers.contains(KeyModifiers::CONTROL)) =>
        {
            if let Some(picker) = editor.picker_mut() {
                picker.toggle_field();
            }
        }
        KeyCode::Left
            if (cfg!(target_os = "macos") && key_event.modifiers.contains(KeyModifiers::ALT))
                || (!cfg!(target_os = "macos")
                    && key_event.modifiers.contains(KeyModifiers::CONTROL)) =>
        {
            if let Some(picker) = editor.picker_mut() {
                picker.toggle_field();
            }
        }
        // Enter - select current item
        KeyCode::Enter => {
            let action = editor.picker().and_then(|p| p.selected_action());
            editor.close_picker();
            editor.set_mode(Mode::Normal);
            if let Some(action) = action {
                editor.execute_picker_action(action)?;
            }
        }
        // Backspace - remove character before cursor
        KeyCode::Backspace => {
            if let Some(picker) = editor.picker_mut() {
                picker.backspace_query();
            }
            // Mark query changed for debouncing
            editor.mark_picker_query_changed();
        }
        // Delete - remove character at cursor
        KeyCode::Delete => {
            if let Some(picker) = editor.picker_mut() {
                picker.delete_char();
            }
            // Mark query changed for debouncing
            editor.mark_picker_query_changed();
        }
        // Left arrow - move cursor left in query
        KeyCode::Left => {
            if let Some(picker) = editor.picker_mut() {
                picker.move_cursor_left();
            }
        }
        // Right arrow - move cursor right in query
        KeyCode::Right => {
            if let Some(picker) = editor.picker_mut() {
                picker.move_cursor_right();
            }
        }
        // Home - move cursor to beginning of query
        KeyCode::Home => {
            if let Some(picker) = editor.picker_mut() {
                picker.move_cursor_home();
            }
        }
        // End - move cursor to end of query
        KeyCode::End => {
            if let Some(picker) = editor.picker_mut() {
                picker.move_cursor_end();
            }
        }
        // Ctrl-N - move down in results
        KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut moved = false;
            if let Some(picker) = editor.picker_mut() {
                picker.move_down();
                moved = true;
            }
            if moved {
                editor.mark_picker_selection_changed();
            }
        }
        // Ctrl-P - move up in results
        KeyCode::Char('p') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            let mut moved = false;
            if let Some(picker) = editor.picker_mut() {
                picker.move_up();
                moved = true;
            }
            if moved {
                editor.mark_picker_selection_changed();
            }
        }
        // Ctrl-D - move down half page in results
        KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            let half_page = editor.half_page_scroll();
            let mut moved = false;
            if let Some(picker) = editor.picker_mut() {
                picker.move_down_n(half_page);
                moved = true;
            }
            if moved {
                editor.mark_picker_selection_changed();
            }
        }
        // Ctrl-U - move up half page in results
        KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            let half_page = editor.half_page_scroll();
            let mut moved = false;
            if let Some(picker) = editor.picker_mut() {
                picker.move_up_n(half_page);
                moved = true;
            }
            if moved {
                editor.mark_picker_selection_changed();
            }
        }
        // Down arrow - move down in results
        KeyCode::Down => {
            let mut moved = false;
            if let Some(picker) = editor.picker_mut() {
                picker.move_down();
                moved = true;
            }
            if moved {
                editor.mark_picker_selection_changed();
            }
        }
        // Up arrow - move up in results
        KeyCode::Up => {
            let mut moved = false;
            if let Some(picker) = editor.picker_mut() {
                picker.move_up();
                moved = true;
            }
            if moved {
                editor.mark_picker_selection_changed();
            }
        }
        // Alt+F / Alt+B (Ghostty sends Alt+Right/Left as ESC+'f'/'b') - toggle field
        KeyCode::Char('f') | KeyCode::Char('b')
            if cfg!(target_os = "macos") && key_event.modifiers.contains(KeyModifiers::ALT) =>
        {
            if let Some(picker) = editor.picker_mut() {
                picker.toggle_field();
            }
        }
        // Any other character - insert at cursor
        KeyCode::Char(ch) => {
            if let Some(picker) = editor.picker_mut() {
                picker.insert_char(ch);
            }
            // Mark query changed for debouncing
            editor.mark_picker_query_changed();
        }
        _ => {}
    }

    Ok(())
}
