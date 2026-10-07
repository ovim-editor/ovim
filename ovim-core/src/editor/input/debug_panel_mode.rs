//! Key handling while the debug panel has focus (`<Space>df`).
//!
//! `j`/`k` move over the rows (call stack, variables, watches, breakpoints,
//! exception filters), `Ctrl-d`/`Ctrl-u` page, `g`/`G` top/bottom.
//! `Enter`/`l`/`Space` acts on the row: select a frame, expand or collapse a
//! variable, jump to a breakpoint, toggle an exception filter. `h` collapses
//! or goes to the parent. `d`/`x` deletes a breakpoint or watch, `e`/`t`
//! enables/disables a breakpoint. `a` adds a watch, `E` toggles "break on
//! exceptions", `<`/`>` resize the panel, `c`/`n`/`i`/`o` continue and step,
//! `q`/`Esc` returns to the buffer.

use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

use crate::dap::PendingDebugAction;
use crate::editor::Editor;
use crate::mode::Mode;

pub fn handle_debug_panel_mode(editor: &mut Editor, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(Modifiers::CONTROL);
    if matches!(key.code, KeyCode::F(_)) && super::debug_keys::try_handle(editor, key)? {
        editor.mark_dirty();
        return Ok(());
    }
    let half = (editor.debug_state().panel.view_height.get() / 2).max(1) as isize;
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => editor.set_mode(Mode::Normal),
        KeyCode::Char('j') | KeyCode::Down => editor.debug_panel_move(1),
        KeyCode::Char('k') | KeyCode::Up => editor.debug_panel_move(-1),
        KeyCode::Char('d') if ctrl => editor.debug_panel_move(half),
        KeyCode::Char('u') if ctrl => editor.debug_panel_move(-half),
        KeyCode::PageDown => editor.debug_panel_move(half * 2),
        KeyCode::PageUp => editor.debug_panel_move(-half * 2),
        KeyCode::Char('g') | KeyCode::Home => editor.debug_panel_edge(false),
        KeyCode::Char('G') | KeyCode::End => editor.debug_panel_edge(true),
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right | KeyCode::Char(' ') => {
            editor.debug_panel_activate()
        }
        KeyCode::Char('h') | KeyCode::Left => editor.debug_panel_collapse(),
        KeyCode::Char('d') | KeyCode::Char('x') | KeyCode::Delete => editor.debug_panel_delete(),
        KeyCode::Char('e') | KeyCode::Char('t') => editor.debug_panel_toggle_enabled(),
        KeyCode::Char('a') => {
            editor.set_mode(Mode::Command);
            editor.set_command_line("DebugWatch ");
        }
        KeyCode::Char('E') => {
            if let Err(message) = editor.toggle_exception_filter("") {
                editor.set_status_message(message);
            }
        }
        KeyCode::Char('<') | KeyCode::Char('-') => editor.debug_panel_resize(-4),
        KeyCode::Char('>') | KeyCode::Char('+') | KeyCode::Char('=') => {
            editor.debug_panel_resize(4)
        }
        KeyCode::Char('c') if !ctrl => step(editor, PendingDebugAction::Continue),
        KeyCode::Char('n') => step(editor, PendingDebugAction::StepOver),
        KeyCode::Char('i') => step(editor, PendingDebugAction::StepIn),
        KeyCode::Char('o') => step(editor, PendingDebugAction::StepOut),
        KeyCode::Char('s') => {
            editor.launch_stop();
        }
        _ => {}
    }
    editor.mark_dirty();
    Ok(())
}

fn step(editor: &mut Editor, action: PendingDebugAction) {
    if editor.is_debug_active() {
        editor.dap_manager_mut().queue(action);
    } else {
        editor.set_status_message("No debug session");
    }
}
