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
            if editor.search.search_buffer.is_empty() {
                // Backspace on empty search buffer exits search mode (like Neovim)
                editor.restore_search_start_position();
                editor.clear_search_buffer();

                if let Some(vss) = editor.take_visual_search_state() {
                    editor.set_visual_start(vss.anchor.0, vss.anchor.1);
                    editor.set_mode(vss.mode);
                } else if editor.visual_start().is_some() {
                    editor.set_mode(Mode::Visual);
                } else {
                    editor.set_mode(Mode::Normal);
                }
            } else {
                editor.backspace_search_buffer();
                editor.execute_search();
            }
        }
        KeyCode::Enter => {
            // Execute the search and accept it
            editor.execute_search();

            // Check if we're extending a visual selection
            if let Some(visual_search_state) = editor.take_visual_search_state() {
                // Restore visual mode and extend selection to cursor position
                // Set the visual anchor to the original position
                editor.set_visual_start(visual_search_state.anchor.0, visual_search_state.anchor.1);

                // Move cursor to match position (already done by execute_search)
                // The selection now spans from anchor to cursor

                // Restore the original visual mode
                editor.set_mode(visual_search_state.mode);
            } else if editor.visual_start().is_some() {
                // Legacy path: visual mode was active but no search state saved
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

            // Check if we had a visual search state (restore visual mode)
            if let Some(visual_search_state) = editor.take_visual_search_state() {
                // Restore visual mode with original anchor
                editor.set_visual_start(visual_search_state.anchor.0, visual_search_state.anchor.1);
                editor.set_mode(visual_search_state.mode);
            } else if editor.visual_start().is_some() {
                // Legacy path: visual mode was active but no search state saved
                editor.set_mode(Mode::Visual);
            } else {
                editor.set_mode(Mode::Normal);
            }
        }
        _ => {}
    }
    Ok(())
}
