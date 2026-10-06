//! Key handling while the run console has focus (`<Space>rf`).
//!
//! `j`/`k` move the highlighted line, `Ctrl-d`/`Ctrl-u` page, `g`/`G` jump to
//! the top/bottom (`G` resumes following live output), `Enter` opens the
//! source location on the line (stack frames, compiler errors), `[`/`]`
//! switch between runs, `r` reruns, `s` stops, `x` clears, `i` types a line for
//! the program's stdin (`D` ends the input), `+`/`-` resize the panel,
//! `q`/`Esc` hides the console and returns to the buffer.

use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

use crate::editor::Editor;
use crate::launch::LaunchMode;
use crate::mode::Mode;

pub fn handle_run_console_mode(editor: &mut Editor, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(Modifiers::CONTROL);
    if matches!(key.code, KeyCode::F(_)) && super::debug_keys::try_handle(editor, key)? {
        editor.mark_dirty();
        return Ok(());
    }
    let half = (editor.run_console().view_height / 2).max(1) as isize;
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            editor.close_run_console();
        }
        // Taller / shorter console.
        KeyCode::Char('+') | KeyCode::Char('=') => {
            let _ = editor.resize_panel("console", "+3");
        }
        KeyCode::Char('-') => {
            let _ = editor.resize_panel("console", "-3");
        }
        KeyCode::Char('j') | KeyCode::Down => editor.run_console_mut().move_cursor(1),
        KeyCode::Char('k') | KeyCode::Up => editor.run_console_mut().move_cursor(-1),
        KeyCode::Char('d') if ctrl => editor.run_console_mut().move_cursor(half),
        KeyCode::Char('u') if ctrl => editor.run_console_mut().move_cursor(-half),
        KeyCode::Char('f') if ctrl => editor.run_console_mut().move_cursor(half * 2),
        KeyCode::Char('b') if ctrl => editor.run_console_mut().move_cursor(-half * 2),
        KeyCode::PageDown => editor.run_console_mut().move_cursor(half * 2),
        KeyCode::PageUp => editor.run_console_mut().move_cursor(-half * 2),
        KeyCode::Char('g') | KeyCode::Home => {
            let console = editor.run_console_mut();
            console.set_cursor(0);
            console.scroll_to_top();
        }
        KeyCode::Char('G') | KeyCode::End => {
            let console = editor.run_console_mut();
            let last = console
                .viewed()
                .map(|r| r.lines.len().saturating_sub(1))
                .unwrap_or(0);
            console.set_cursor(last);
            console.scroll_to_bottom();
        }
        KeyCode::Enter => {
            let line = editor.run_console().cursor;
            editor.run_console_jump(line);
        }
        KeyCode::Char('[') | KeyCode::Char('H') => editor.run_console_mut().view_previous(),
        KeyCode::Char(']') | KeyCode::Char('L') => editor.run_console_mut().view_next(),
        KeyCode::Char('r') => editor.launch_last(),
        KeyCode::Char('s') => {
            editor.launch_stop();
        }
        KeyCode::Char('c') if ctrl => {
            editor.launch_stop();
        }
        KeyCode::Char('x') => editor.clear_run_console(),
        KeyCode::Char('i') => {
            // Type a line for the program's stdin (`:RunInput`).
            editor.set_mode(Mode::Command);
            editor.set_command_line("RunInput ");
        }
        KeyCode::Char('D') => editor.run_eof(),
        KeyCode::Char('R') => editor.launch_at_cursor(LaunchMode::Run),
        _ => {}
    }
    editor.mark_dirty();
    Ok(())
}
