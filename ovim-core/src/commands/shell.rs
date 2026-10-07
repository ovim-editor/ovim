//! Shell commands: `:!`, `:{range}!` filters, `:r !`, `:w !` and
//! `:terminal` / `:shell`.
//!
//! `:!cmd` and `:terminal` need the user's terminal, so the core only queues
//! them ([`crate::tick::TerminalRequest`]) and the frontend decides: the TUI
//! runs them, the GUI and a headless session decline, and the API entry
//! point ([`super::execute_command_api`]) runs `:!cmd` with captured output.
//! Filters, `:r !` and `:w !` exchange text with the buffer and run here.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use super::edit::{insert_lines_below, lines_text, unmodifiable};
use super::range::{self, LineRange};
use super::Ex;
use crate::command_result::{err, ok, ok_silent, CommandResult};
use crate::editor::shell_expansion::expand_shell_command;
use crate::editor::{Editor, PendingShellCommand, PendingTerminalSession};
use crate::unicode::{CharCol, GraphemeCol};

fn shell() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    }
}

/// Expand `%` and `#` like vim.
fn expand(editor: &Editor, command: &str) -> String {
    let current_file = editor.buffer().file_path().unwrap_or("").to_string();
    let alternate_file = editor.registers().get(Some('#'));
    expand_shell_command(command, &current_file, &alternate_file)
}

/// Run `command` with `input` on stdin, capturing its output.
fn run_piped(editor: &mut Editor, command: &str, input: Option<&str>) -> std::io::Result<Output> {
    let (shell, flag) = shell();
    editor.with_external_effects(|_| {
        let mut child = Command::new(shell)
            .arg(flag)
            .arg(command)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        // Feed stdin from its own thread: a filter such as `cat` fills its
        // stdout pipe while we are still writing, so writing everything
        // before reading anything deadlocks on large input.
        std::thread::scope(|scope| {
            let writer = input.zip(stdin).map(|(input, mut stdin)| {
                scope.spawn(move || {
                    // A command that exits without reading its input
                    // (`:w !true`) closes the pipe; vim does not treat that
                    // as a failure.
                    match stdin.write_all(input.as_bytes()) {
                        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
                        other => other,
                    }
                })
            });
            let output = child.wait_with_output();
            if let Some(writer) = writer {
                if let Ok(Err(error)) = writer.join() {
                    return Err(error);
                }
            }
            output
        })
    })
}

fn failed(output: &Output) -> CommandResult {
    let stderr = String::from_utf8_lossy(&output.stderr);
    err(format!("Command failed: {}", stderr.trim()))
}

/// `:!{cmd}` queues `cmd` for the frontend (`:!` alone repeats the last
/// one); `:{range}!{filter}` pipes the lines through `filter`.
pub(super) fn bang(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.explicit_range {
        return filter(editor, ex.range.expect("range given"), ex.args);
    }
    let command = if ex.args.is_empty() {
        match editor.build.last_shell_command.clone() {
            Some(last) => last,
            None => return err("No previous shell command"),
        }
    } else {
        expand(editor, ex.args)
    };
    editor.build.last_shell_command = Some(command.clone());
    editor.build.pending_shell_command = Some(PendingShellCommand { command });
    ok_silent()
}

/// `:{range}!{filter}` replaces the lines with the filter's output.
fn filter(editor: &mut Editor, range: LineRange, command: &str) -> CommandResult {
    if command.is_empty() {
        return ok_silent();
    }
    if !editor.buffer().is_modifiable() {
        return unmodifiable();
    }
    let command = expand(editor, command);
    let (start, end) = range.indexes();
    let input = lines_text(editor, start, end);
    let output = match run_piped(editor, &command, Some(&input)) {
        Ok(output) => output,
        Err(error) => return err(format!("Failed to run command: {error}")),
    };
    if !output.status.success() {
        return failed(&output);
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    editor.record_operation(
        |buffer| {
            buffer.delete_range(start, CharCol::ZERO, end + 1, CharCol::ZERO);
            if !text.is_empty() {
                buffer.insert_text_at(start, CharCol::ZERO, &text);
            }
            buffer.cursor_mut().set_position(start, GraphemeCol::ZERO);
        },
        None,
    );
    ok(format!("{} lines filtered", text.lines().count()))
}

/// `:[line]r[ead] !{cmd}` / `:[line]r[ead] [file]`: insert below the line
/// (0 = above the first line).
pub(super) fn read(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if !editor.buffer().is_modifiable() {
        return unmodifiable();
    }
    let below = ex.range.expect("line-range command").end;
    let from_command = ex.args.starts_with('!');
    let (text, message) = if let Some(command) = ex.args.strip_prefix('!') {
        let command = expand(editor, command.trim());
        let output = match run_piped(editor, &command, None) {
            Ok(output) => output,
            Err(error) => return err(format!("Failed to run command: {error}")),
        };
        if !output.status.success() {
            return failed(&output);
        }
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        if text.is_empty() {
            return ok("Command produced no output");
        }
        let lines = text.lines().count();
        (text, format!("{lines} line{} inserted", plural(lines)))
    } else {
        let target = if ex.args.is_empty() {
            match editor.buffer().file_path() {
                Some(path) => path.to_string(),
                None => return err("E32: No file name"),
            }
        } else {
            ex.args.to_string()
        };
        match std::fs::read_to_string(&target) {
            Ok(text) => {
                let message = format!("Read {} lines from {target}", text.lines().count());
                (text, message)
            }
            Err(_) => return err(format!("E484: Can't open file {target}")),
        }
    };
    let text = if text.ends_with('\n') {
        text
    } else {
        format!("{text}\n")
    };
    // vim (nvim --clean): the cursor lands on the first line read from a
    // file, but on the last line of a command's output, at its first
    // non-blank character.
    let cursor_line = if from_command {
        below + text.lines().count().max(1) - 1
    } else {
        below
    };
    editor.record_operation(
        |buffer| {
            insert_lines_below(buffer, below, &text);
            buffer
                .cursor_mut()
                .set_position(cursor_line, GraphemeCol::ZERO);
            if from_command {
                crate::editor::Motions::first_non_blank(buffer);
            }
        },
        None,
    );
    ok(message)
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// `:[range]w[rite] !{cmd}`: pipe the lines (default: all) to `cmd`.
pub(super) fn write_to_command(
    editor: &mut Editor,
    range: LineRange,
    command: &str,
) -> CommandResult {
    let command = expand(editor, command);
    let (start, end) = range.indexes();
    let end = end.min(range::last_line(editor).saturating_sub(1));
    let content = lines_text(editor, start, end);
    let output = match run_piped(editor, &command, Some(&content)) {
        Ok(output) => output,
        Err(error) => return err(format!("Failed to run command: {error}")),
    };
    if !output.status.success() {
        return failed(&output);
    }
    let lines = content.lines().count();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout.trim();
    if stdout.is_empty() {
        ok(format!("{lines} line{} written", plural(lines)))
    } else if stdout.len() > 100 {
        ok(format!(
            "{lines} lines written: {}...",
            crate::unicode::truncate_bytes(stdout, 100)
        ))
    } else {
        ok(format!("{lines} lines written: {stdout}"))
    }
}

/// `:ter[minal] [cmd]` / `:sh[ell] [cmd]`: an interactive session in the
/// frontend's terminal.
pub(super) fn terminal(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let command = (!ex.args.is_empty()).then(|| ex.args.to_string());
    editor.build.pending_terminal_session = Some(PendingTerminalSession { command });
    ok_silent()
}

/// Run a queued `:!cmd` with captured output, for callers without a
/// terminal (the API entry point).
pub(super) fn run_captured(editor: &mut Editor, command: &str) -> CommandResult {
    let output = match run_piped(editor, command, None) {
        Ok(output) => output,
        Err(error) => return err(format!("Failed to execute command: {error}")),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = stdout.into_owned();
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&stderr);
    }
    let result = result.trim_end().to_string();
    if output.status.success() {
        if result.is_empty() {
            ok("Command executed successfully")
        } else {
            ok(result)
        }
    } else {
        let code = output
            .status
            .code()
            .map(|code| format!(" (exit code {code})"))
            .unwrap_or_default();
        if result.is_empty() {
            err(format!("Command failed{code}"))
        } else {
            err(format!("{result}\n\nCommand failed{code}"))
        }
    }
}
