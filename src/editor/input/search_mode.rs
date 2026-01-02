//! Search mode input handling
//!
//! Handles incremental search input, Enter to confirm, Esc to cancel.

use crate::editor::Editor;
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

/// Handles input in Search mode
pub fn handle_search_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Char(ch) => {
            // Add character to search buffer
            editor.append_to_search_buffer(ch);
            // Incremental search: update highlighting immediately
            editor.execute_search();
        }
        KeyCode::Backspace => {
            // Remove last character from search buffer
            editor.backspace_search_buffer();
            // Incremental search: update highlighting after backspace
            editor.execute_search();
        }
        KeyCode::Enter => {
            // Execute the search and accept it
            editor.execute_search();
            // Return to visual mode if visual_start is set, otherwise normal mode
            if editor.visual_start().is_some() {
                editor.set_mode(Mode::Visual);
            } else {
                editor.set_mode(Mode::Normal);
            }
        }
        KeyCode::Esc => {
            // Cancel search mode
            // BUG FIX: Restore cursor to position before search started
            editor.restore_search_start_position();
            editor.clear_search_buffer();
            // Return to visual mode if visual_start is set, otherwise normal mode
            if editor.visual_start().is_some() {
                editor.set_mode(Mode::Visual);
            } else {
                editor.set_mode(Mode::Normal);
            }
        }
        _ => {}
    }
    Ok(())
}
