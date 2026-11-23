//! Run, test and console commands with arguments; the argument-free
//! `:Run`, `:Debug`, `:Test*`, `:CodeLens*` and `:Run*` console commands are
//! one-line table entries.

use super::Ex;
use crate::command_result::{err, ok, ok_silent, CommandResult};
use crate::editor::Editor;

/// `:RunJump {N}`: open the source location on console line N (0-based).
pub(super) fn run_jump(editor: &mut Editor, ex: &Ex) -> CommandResult {
    match ex.args.parse::<usize>() {
        Ok(index) => {
            editor.run_console_jump(index);
            ok_silent()
        }
        Err(_) => err("Usage: RunJump <line index>"),
    }
}

/// `:RunInput [text]`: send a line to the running program's stdin.
pub(super) fn run_input(editor: &mut Editor, ex: &Ex) -> CommandResult {
    editor.run_input(ex.args);
    ok_silent()
}

/// `:PanelSize {panel} {size}`.
pub(super) fn panel_size(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let mut parts = ex.args.split_whitespace();
    let panel = parts.next().unwrap_or("");
    match editor.resize_panel(panel, parts.next().unwrap_or("")) {
        Ok(message) => ok(message),
        Err(message) => err(message),
    }
}

/// `:TestPanel` toggles the test panel.
pub(super) fn test_panel(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.toggle_test_panel();
    ok(if editor.is_test_panel_open() {
        "Test panel opened"
    } else {
        "Test panel closed"
    })
}

/// `:TestOutput` / `:MakeOutput`: raw output of the last `:make` or test run.
pub(super) fn test_output(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    if editor.last_make_output().is_none() {
        return err("No make/test output available");
    }
    editor.open_test_output_buffer();
    ok("Make/test output")
}
