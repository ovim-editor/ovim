//! Quickfix commands: `:make`, `:cope`, `:ccl`, `:cn`, `:cp`, `:cfir`,
//! `:cla`, `:cdo` and `:cfdo`.

use super::Ex;
use crate::command_result::{err, ok, CommandResult};
use crate::editor::{Editor, QuickfixEntry};
use crate::unicode::GraphemeCol;

/// Open the entry's file at its position.
pub fn jump_to_quickfix_entry(editor: &mut Editor, entry: &QuickfixEntry) -> CommandResult {
    if let Some(ref path) = entry.filename {
        // Load the file if needed
        let path_str = path.to_string_lossy().to_string();
        if let Err(e) = editor.load_file(&path_str) {
            return err(format!("Failed to load file: {}", e));
        }

        // Jump to line/column (convert from 1-indexed to 0-indexed)
        let line = entry.lnum.saturating_sub(1);
        let col = entry.col.saturating_sub(1);
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line, GraphemeCol(col));
        editor.buffer_mut().validate_cursor_position();

        ok(entry.display_text())
    } else {
        ok(entry.text.clone())
    }
}

/// Execute :make command — runs makeprg through the launch pipeline (so
/// `:RunStop` stops it and a second `:make` replaces it); its diagnostics
/// become the quickfix list when it finishes.
pub(super) fn make(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let args = ex.args;
    use crate::launch::plan::PlanKind;

    // Build the command: makeprg + args (default: "cargo build")
    let makeprg = editor.options.makeprg.clone();
    let cmd = if args.is_empty() {
        makeprg
    } else {
        format!("{} {}", makeprg, args)
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    editor.begin_request(crate::editor::LaunchRequest::shell(
        PlanKind::Task,
        "make",
        &cmd,
        cwd,
    ));
    ok(format!("Running: {}", cmd))
}

/// `:cope[n]`: the list, current entry marked.
pub(super) fn open(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    let list = editor.quickfix_list();
    if list.is_empty() {
        return ok("Quickfix list is empty");
    }
    let title = if list.title().is_empty() {
        "Quickfix List"
    } else {
        list.title()
    };
    let entries: Vec<String> = list
        .entries()
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let marker = if index == list.selected_index() {
                ">"
            } else {
                " "
            };
            format!("{marker} {}", entry.display_text())
        })
        .collect();
    ok(format!(
        "{title} ({} items)\n{}",
        list.len(),
        entries.join("\n")
    ))
}

/// `:ccl[ose]` clears the list.
pub(super) fn close(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.quickfix_list_mut().clear();
    ok("Quickfix list cleared")
}

fn go(editor: &mut Editor, step: fn(&mut crate::editor::QuickfixList)) -> CommandResult {
    if editor.quickfix_list().is_empty() {
        return err("Quickfix list is empty");
    }
    step(editor.quickfix_list_mut());
    match editor.quickfix_list().current_entry().cloned() {
        Some(entry) => jump_to_quickfix_entry(editor, &entry),
        None => err("No current entry"),
    }
}

pub(super) fn next(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    go(editor, |list| list.next())
}

pub(super) fn previous(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    go(editor, |list| list.previous())
}

pub(super) fn first(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    go(editor, |list| list.first())
}

pub(super) fn last(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    go(editor, |list| list.last())
}

/// `:cdo {cmd}` runs `cmd` at every quickfix entry, `:cfdo {cmd}` at the
/// first entry of every file. Stops at the first error like vim.
pub(super) fn quickfix_do(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let per_file = ex.command.names[0] == "cfdo";
    let name = if per_file { "cfdo" } else { "cdo" };
    if ex.args.is_empty() {
        return err("E471: Argument required");
    }
    let entries: Vec<(usize, QuickfixEntry)> = {
        let mut seen = std::collections::HashSet::new();
        editor
            .quickfix_list()
            .entries()
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, entry)| {
                entry
                    .filename
                    .as_ref()
                    .is_some_and(|file| !per_file || seen.insert(file.clone()))
            })
            .collect()
    };
    if entries.is_empty() {
        return err("E42: No Errors");
    }
    let total = entries.len();
    let mut done = 0;
    for (index, entry) in entries {
        editor.quickfix_list_mut().set_selected(index);
        if let CommandResult::Error(error) = jump_to_quickfix_entry(editor, &entry) {
            return err(format!("{name}: {}", error.error));
        }
        if let CommandResult::Error(error) = super::run_line(editor, ex.args) {
            return err(format!(
                "{name}: stopped at entry {} of {total}: {}",
                index + 1,
                error.error
            ));
        }
        done += 1;
    }
    ok(format!(
        "{name}: ran on {done} {}",
        if per_file { "file(s)" } else { "entr(ies)" }
    ))
}
