//! Normal mode input handling.
//!
//! This module dispatches normal mode key events to specialized handlers.
//! Each handler returns `Result<bool>`:
//! - `Ok(true)` - Key was handled, stop dispatching
//! - `Ok(false)` - Key was not handled, try next handler
//! - `Err(_)` - Error occurred

mod editing_commands;
mod mode_transitions;
mod motions_input;
mod operators;
mod pending_commands;
mod text_objects;

use crate::editor::Editor;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

/// Handle a key event in normal mode.
///
/// This dispatcher tries each handler in priority order until one handles the key.
pub fn handle_normal_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    if editor.handle_pseudocode_key(key_event) {
        return Ok(());
    }
    // Output panels can be passive rather than focused, so Escape must hide them
    // even when a pending operator or multi-key command consumes the key.
    if key_event.code == KeyCode::Esc {
        editor.close_test_panel();
        editor.close_run_console();
    }

    // 0. Buffer-local keys of the branch diff review (Enter / q / r / s). Only
    //    when nothing is pending so `]c`, counts and searches still work.
    if try_handle_diff_review_key(editor, key_event) {
        return Ok(());
    }

    // 1. Try pending operators (dd, dw, yy, etc.)
    if operators::try_handle(editor, key_event)? {
        return Ok(());
    }

    // 2. Try text objects after operator (diw, ci", etc.)
    if text_objects::try_handle(editor, key_event)? {
        return Ok(());
    }

    // 3. Try pending commands (g*, z*, m*, etc.)
    if pending_commands::try_handle(editor, key_event)? {
        return Ok(());
    }

    // 4. Try mode transitions (i, a, v, :, etc.)
    if mode_transitions::try_handle(editor, key_event)? {
        return Ok(());
    }

    // 5. Try editing commands (x, D, p, etc.)
    if editing_commands::try_handle(editor, key_event)? {
        return Ok(());
    }

    // 6. Try motions (h, j, k, l, w, b, etc.)
    if motions_input::try_handle(editor, key_event)? {
        return Ok(());
    }

    // 7. Set up operators and pending commands for multi-key sequences
    if setup_pending_state(editor, key_event)? {
        return Ok(());
    }

    // Clear count on unrecognized key
    editor.clear_count();
    Ok(())
}

/// Keys that only apply while the branch diff review buffer is current.
fn try_handle_diff_review_key(editor: &mut Editor, key_event: KeyEvent) -> bool {
    use crate::editor::InputState;

    if !editor.is_diff_review_buffer()
        || !matches!(editor.input_state(), InputState::Normal)
        || editor.count().is_some()
    {
        return false;
    }
    match key_event.code {
        KeyCode::Char('x')
            if !key_event
                .modifiers
                .intersects(Modifiers::CONTROL | Modifiers::ALT) =>
        {
            editor.toggle_diff_review_check_at_cursor()
        }
        KeyCode::Char('X')
            if !key_event
                .modifiers
                .intersects(Modifiers::CONTROL | Modifiers::ALT) =>
        {
            editor.toggle_diff_review_show_checked()
        }
        KeyCode::Enter => editor.diff_review_open_at_cursor(),
        KeyCode::Char('q') => editor.close_diff_review(),
        KeyCode::Char('r') => editor.refresh_diff_review(),
        KeyCode::Char('s') => editor.toggle_diff_review_layout(),
        KeyCode::Char('K')
            if !key_event
                .modifiers
                .intersects(Modifiers::CONTROL | Modifiers::ALT) =>
        {
            editor.expand_diff_context_at_cursor(true)
        }
        KeyCode::Char('J')
            if !key_event
                .modifiers
                .intersects(Modifiers::CONTROL | Modifiers::ALT) =>
        {
            editor.expand_diff_context_at_cursor(false)
        }
        KeyCode::Char('w')
            if key_event.modifiers.is_empty()
                && editor
                    .diff_review()
                    .is_some_and(|state| state.custom().is_some()) =>
        {
            editor.toggle_diff_review_equal_changes();
        }
        KeyCode::Char('a')
            if key_event.modifiers.is_empty()
                && editor
                    .diff_review()
                    .and_then(|state| state.custom())
                    .is_some_and(|review| {
                        review
                            .sections
                            .iter()
                            .any(|section| section.message.is_some())
                    }) =>
        {
            editor.toggle_diff_review_notes();
        }
        KeyCode::Char('o') => {
            if let Err(error) = editor.toggle_diff_review_overlay() {
                editor.set_status_message(format!("Diff overlay: {error:#}"));
            }
        }
        KeyCode::Char('O') => {
            if let Err(error) = editor.open_saved_diff_overlay() {
                editor.set_status_message(format!("Diff overlay: {error:#}"));
            }
        }
        _ => return false,
    }
    true
}

/// Set up pending operators or commands for multi-key sequences.
fn setup_pending_state(editor: &mut Editor, key_event: KeyEvent) -> Result<bool> {
    use crate::editor::{CharMotion, InputState, Operator};

    let KeyCode::Char(key) = key_event.code else {
        return Ok(false);
    };
    let operator = |operator| InputState::OperatorPending { operator };
    let awaiting = |motion| InputState::AwaitingChar {
        motion,
        operator: None,
    };
    let state = match key {
        'd' => operator(Operator::Delete),
        'y' => operator(Operator::Yank),
        'c' => operator(Operator::Change),
        '>' => operator(Operator::Indent),
        '<' => operator(Operator::Dedent),
        '=' => operator(Operator::AutoIndent),
        'g' => InputState::GPrefix { operator: None },
        'z' => InputState::ZPrefix,
        'Z' => InputState::QuitPrefix,
        '[' | ']' => InputState::BracketPrefix { bracket: key },
        '"' => InputState::RegisterPending,
        'q' if editor.is_recording_macro() => {
            editor.stop_macro_recording();
            return Ok(true);
        }
        'q' => InputState::MacroPrefix { is_recording: true },
        '@' => InputState::MacroPrefix {
            is_recording: false,
        },
        'm' => awaiting(CharMotion::Mark),
        '\'' => awaiting(CharMotion::JumpMarkLine),
        '`' => awaiting(CharMotion::JumpMarkExact),
        'r' => awaiting(CharMotion::Replace),
        'f' => awaiting(CharMotion::Find),
        'F' => awaiting(CharMotion::FindBack),
        't' => awaiting(CharMotion::Till),
        'T' => awaiting(CharMotion::TillBack),
        // Leader key (configurable via vim.g.mapleader, default: space)
        c if c == editor.leader_key() => InputState::Leader { keys: vec![] },
        _ => return Ok(false),
    };
    editor.set_input_state(state);
    Ok(true)
}
