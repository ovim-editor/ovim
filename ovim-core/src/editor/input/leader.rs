//! Leader sequence handler for <Space>-prefixed commands.
//!
//! This module handles the Leader input state, processing commands like:
//! - `<Space>th` - Type hierarchy (LSP)
//! - `<Space>ca` - Code actions (LSP)
//! - `<Space>e` - Show diagnostic at cursor
//! - `<Space>o` - Document outline
//! - etc.

use anyhow::Result;
use crate::{KeyCode, KeyEvent};

use crate::editor::input_state::InputState;
use crate::editor::{Editor, Picker};
use crate::mode::Mode;

/// Handles input when in Leader state.
///
/// Called when the editor is in `InputState::Leader { keys }` state.
/// The `keys` vector contains any keys already pressed after the leader.
pub fn handle_leader_input(editor: &mut Editor, key: KeyEvent, keys: &[char]) -> Result<()> {
    // Handle Escape - cancel leader sequence
    if key.code == KeyCode::Esc {
        editor.reset_input_state();
        return Ok(());
    }

    let KeyCode::Char(c) = key.code else {
        // Non-character key - cancel
        editor.reset_input_state();
        return Ok(());
    };

    if keys.is_empty() {
        // First key after leader
        handle_first_leader_key(editor, c)
    } else {
        // Subsequent key in sequence
        handle_leader_sequence(editor, keys, c)
    }
}

/// Handles the first key after leader (<Space>).
fn handle_first_leader_key(editor: &mut Editor, key: char) -> Result<()> {
    match key {
        // Single-key commands
        'e' => {
            // <Space>e - Show diagnostic at cursor (like vim.diagnostic.open_float())
            editor.show_diagnostic_at_cursor();
            editor.reset_input_state();
        }
        'o' => {
            // <Space>o - Document outline (symbols)
            editor.request_document_symbols();
            editor.reset_input_state();
        }
        'S' => {
            // <Space>S - Workspace symbols
            editor.request_workspace_symbols();
            editor.reset_input_state();
        }
        'i' => {
            // <Space>i - Organize imports
            editor.request_organize_imports();
            editor.reset_input_state();
        }

        // Multi-key sequences - accumulate the key
        'l' => {
            // <Space>l... - LSP manager prefix
            editor.set_input_state(InputState::Leader { keys: vec!['l'] });
        }
        't' => {
            // <Space>t... - Type hierarchy prefix
            editor.set_input_state(InputState::Leader { keys: vec!['t'] });
        }
        'c' => {
            // <Space>c... - Code actions/call hierarchy prefix
            editor.set_input_state(InputState::Leader { keys: vec!['c'] });
        }
        's' => {
            // <Space>s... - Search prefix
            editor.set_input_state(InputState::Leader { keys: vec!['s'] });
        }

        // Unknown key - cancel sequence
        _ => {
            editor.reset_input_state();
        }
    }

    Ok(())
}

/// Handles subsequent keys in a leader sequence.
fn handle_leader_sequence(editor: &mut Editor, keys: &[char], next_key: char) -> Result<()> {
    match (keys, next_key) {
        // <Space>l... sequences
        (&['l'], 'm') => {
            // <Space>lm - LSP Manager panel
            editor.open_lsp_manager();
            editor.reset_input_state();
        }

        // <Space>t... sequences
        (&['t'], 'h') => {
            // <Space>th - Type hierarchy
            editor.request_type_hierarchy();
            editor.reset_input_state();
        }

        // <Space>c... sequences
        (&['c'], 'a') => {
            // <Space>ca - Code actions
            editor.request_code_actions();
            editor.reset_input_state();
        }
        (&['c'], 'i') => {
            // <Space>ci - Incoming calls (call hierarchy)
            editor.request_call_hierarchy_incoming();
            editor.reset_input_state();
        }
        (&['c'], 'o') => {
            // <Space>co - Outgoing calls (call hierarchy)
            editor.request_call_hierarchy_outgoing();
            editor.reset_input_state();
        }

        // <Space>s... sequences
        (&['s'], 'f') => {
            // <Space>sf - Find files
            let base_dir = editor.picker_base_dir();
            let picker = Picker::new_file_finder(base_dir);
            editor.set_picker(picker);
            editor.set_mode(Mode::Picker);
            editor.mark_picker_selection_changed();
            editor.reset_input_state();
        }
        (&['s'], 'g') => {
            // <Space>sg - Live grep
            let base_dir = editor.picker_base_dir();
            let picker = Picker::new_live_grep(base_dir);
            editor.set_picker(picker);
            editor.set_mode(Mode::Picker);
            editor.reset_input_state();
        }

        // Unknown sequence - cancel
        _ => {
            editor.reset_input_state();
        }
    }

    Ok(())
}
