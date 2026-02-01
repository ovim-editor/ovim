//! Substitute confirm mode input handling
//!
//! Handles interactive substitute confirmation for :s/pattern/replacement/c
//! Keys: y (yes), n (no/skip), a (all), q (quit), l (last - substitute and quit)

use crate::editor::Editor;
use anyhow::Result;
use crate::{KeyCode, KeyEvent};

/// Handles input in SubstituteConfirm mode
/// Keys: y (yes), n (no/skip), a (all), q (quit), l (last - substitute and quit)
pub fn handle_substitute_confirm_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // Confirm this substitution
            editor.confirm_substitute();
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            // Skip this match
            editor.skip_substitute();
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            // Substitute all remaining matches
            editor.confirm_all_substitutes();
        }
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            // Quit without substituting remaining matches
            editor.end_substitute_confirm();
        }
        KeyCode::Char('l') | KeyCode::Char('L') => {
            // Substitute this match and quit
            editor.confirm_substitute_and_quit();
        }
        _ => {
            // Show prompt in status
            editor.set_lsp_status("replace with ... (y/n/a/q/l)".to_string());
        }
    }
    Ok(())
}
