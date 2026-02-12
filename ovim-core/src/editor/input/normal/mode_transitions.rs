//! Mode transition commands in normal mode.
//!
//! Commands that switch from normal mode to other modes:
//! i, a, I, A, o, O, v, V, Ctrl-V, R, :, /, ?

use crate::editor::input::helpers;
use crate::editor::{Editor, InsertEntryMode, Motions};
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

/// Try to handle a mode transition command.
///
/// Returns `Ok(true)` if the key was handled, `Ok(false)` otherwise.
pub fn try_handle(editor: &mut Editor, key_event: KeyEvent) -> Result<bool> {
    match key_event.code {
        // Escape - clear pending state, or dismiss diagnostic badge on double-Escape
        KeyCode::Esc => {
            let (errors, warnings, _, _) = editor.cached_diagnostic_count();
            let badge_visible =
                !editor.diagnostic_badge_dismissed() && (errors > 0 || warnings > 0);

            if badge_visible {
                if let Some(last) = editor.last_escape_time() {
                    if last.elapsed() < std::time::Duration::from_millis(300) {
                        editor.dismiss_diagnostic_badge();
                        editor.clear_last_escape_time();
                        return Ok(true);
                    }
                }
            }

            // Single Escape: record time, clear pending state
            editor.set_last_escape_time(std::time::Instant::now());
            editor.clear_count();
            editor.clear_pending_operator();
            editor.clear_pending_command();
            editor.reset_input_state();
            Ok(true)
        }
        // i - insert before cursor
        KeyCode::Char('i') if !key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.clear_count();
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(cursor_before);
            editor.set_mode(Mode::Insert);
            Ok(true)
        }
        // a - insert after cursor
        KeyCode::Char('a') if !key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.clear_count();
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(cursor_before);
            editor.set_change_entry_mode(InsertEntryMode::Append);
            editor.set_mode(Mode::Insert);
            // Move cursor right (insert after)
            let cursor = editor.buffer_mut().cursor_mut();
            cursor.move_right(1);
            Ok(true)
        }
        // I - insert at first non-blank
        KeyCode::Char('I') => {
            editor.clear_count();
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(cursor_before);
            editor.set_change_entry_mode(InsertEntryMode::FirstNonBlank);
            editor.set_mode(Mode::Insert);
            // Move to first non-blank character
            Motions::first_non_blank(editor.buffer_mut());
            Ok(true)
        }
        // A - insert at end of line
        KeyCode::Char('A') => {
            editor.clear_count();
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(cursor_before);
            editor.set_change_entry_mode(InsertEntryMode::EndOfLine);
            editor.set_mode(Mode::Insert);
            // Move to end of line
            let line_idx = editor.buffer().cursor().line();
            if let Some(line) = editor.buffer().line(line_idx) {
                let line_len = line.trim_end_matches('\n').chars().count();
                editor.buffer_mut().cursor_mut().set_col(line_len);
            }
            Ok(true)
        }
        // o - open line below
        KeyCode::Char('o') if !key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.clear_count();
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(cursor_before);
            editor.set_change_entry_mode(InsertEntryMode::OpenBelow);
            if helpers::insert_line_below(editor)? {
                editor.set_mode(Mode::Insert);
            } else {
                // Abort empty builder when insertion was blocked/no-op.
                editor.finalize_change_building();
            }
            Ok(true)
        }
        // O - open line above
        KeyCode::Char('O') => {
            editor.clear_count();
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(cursor_before);
            editor.set_change_entry_mode(InsertEntryMode::OpenAbove);
            if helpers::insert_line_above(editor)? {
                editor.set_mode(Mode::Insert);
            } else {
                // Abort empty builder when insertion was blocked/no-op.
                editor.finalize_change_building();
            }
            Ok(true)
        }
        // v - visual mode
        KeyCode::Char('v') if !key_event.modifiers.contains(Modifiers::CONTROL) => {
            let cursor = editor.buffer().cursor();
            editor.set_visual_start(cursor.line(), cursor.col());
            editor.set_mode(Mode::Visual);
            Ok(true)
        }
        // V - visual line mode
        KeyCode::Char('V') => {
            let cursor = editor.buffer().cursor();
            editor.set_visual_start(cursor.line(), 0);
            editor.set_mode(Mode::VisualLine);
            Ok(true)
        }
        // Ctrl-V - visual block mode
        KeyCode::Char('v') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            let cursor = editor.buffer().cursor();
            editor.set_visual_start(cursor.line(), cursor.col());
            editor.set_mode(Mode::VisualBlock);
            Ok(true)
        }
        // R - replace mode
        KeyCode::Char('R') => {
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.editing.replace_mode_state = Some(crate::editor::ReplaceModeState {
                start_position: cursor_before,
                replacements: String::new(),
                old_text: String::new(),
            });
            editor.set_mode(Mode::Replace);
            Ok(true)
        }
        // : - command mode
        KeyCode::Char(':') => {
            editor.clear_command_line();
            editor.set_mode(Mode::Command);
            Ok(true)
        }
        // / - search forward
        KeyCode::Char('/') => {
            editor.clear_search_buffer();
            editor.set_search_forward(true);
            editor.save_search_start_position();
            editor.set_mode(Mode::Search);
            Ok(true)
        }
        // ? - search backward
        KeyCode::Char('?') => {
            editor.clear_search_buffer();
            editor.set_search_forward(false);
            editor.save_search_start_position();
            editor.set_mode(Mode::Search);
            Ok(true)
        }
        // - - toggle file tree (oil.nvim style)
        KeyCode::Char('-') => {
            editor.toggle_file_tree();
            Ok(true)
        }
        _ => Ok(false),
    }
}
