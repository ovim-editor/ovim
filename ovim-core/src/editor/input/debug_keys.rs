//! Run / debug function keys (F5, Shift-F5, Ctrl-F5, F9, Shift-F9, F10, F11).
//!
//! Shared by every mode that can have focus while a session is live (normal,
//! the debug panel, the run console) so the keys do not depend on where the
//! keyboard focus happens to be.

use crate::editor::Editor;
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, Modifiers};

/// Handles a run/debug function key. Returns false for any other key.
pub fn try_handle(editor: &mut Editor, key_event: KeyEvent) -> anyhow::Result<bool> {
    Ok(handle(editor, key_event))
}

fn handle(editor: &mut Editor, key_event: KeyEvent) -> bool {
    match key_event.code {
        // Shift+F5 - stop the run / debug session
        KeyCode::F(5) if key_event.modifiers.contains(Modifiers::SHIFT) => {
            editor.launch_stop();
            true
        }
        // Ctrl+F5 - run (no debugger) whatever is at the cursor
        KeyCode::F(5) if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.launch_at_cursor(crate::launch::LaunchMode::Run);
            true
        }
        // F5 - continue (if a session is stopped/running) or debug whatever is at the cursor
        KeyCode::F(5) => {
            if editor.is_debug_active() {
                editor
                    .dap_manager_mut()
                    .queue(crate::dap::PendingDebugAction::Continue);
            } else {
                editor.launch_at_cursor(crate::launch::LaunchMode::Debug);
            }
            true
        }
        // F9 - toggle breakpoint at cursor line
        KeyCode::F(9) if !key_event.modifiers.contains(Modifiers::SHIFT) => {
            editor.toggle_breakpoint();
            true
        }
        // Shift+F9 - toggle conditional breakpoint (prompts for condition)
        KeyCode::F(9) if key_event.modifiers.contains(Modifiers::SHIFT) => {
            // Enter command mode with ":DebugCondition " pre-filled.
            editor.clear_command_line();
            editor.insert_into_command_line("DebugCondition ");
            editor.set_mode(Mode::Command);
            true
        }
        // F10 - step over
        KeyCode::F(10) => {
            if editor.is_debug_active() {
                editor
                    .dap_manager_mut()
                    .queue(crate::dap::PendingDebugAction::StepOver);
            }
            true
        }
        // F11 - step in (without Shift) / step out (with Shift)
        KeyCode::F(11) => {
            if editor.is_debug_active() {
                let action = if key_event.modifiers.contains(Modifiers::SHIFT) {
                    crate::dap::PendingDebugAction::StepOut
                } else {
                    crate::dap::PendingDebugAction::StepIn
                };
                editor.dap_manager_mut().queue(action);
            }
            true
        }
        _ => false,
    }
}
