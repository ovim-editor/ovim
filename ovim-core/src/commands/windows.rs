//! Quitting, windows, tab pages and the buffer list.

use super::files::{expand_tilde, open_file};
use super::Ex;
use crate::command_result::{err, ok, ok_silent, CommandResult};
use crate::editor::Editor;

const E37: &str = "E37: No write since last change (add ! to override)";

/// Close the current window, else the current tab page, else quit — vim's
/// `:q`. Quitting the last window discards every buffer, so any modified
/// buffer blocks it without `!` (OV-00331).
pub(super) fn close_or_quit(editor: &mut Editor, force: bool) -> CommandResult {
    if editor.window_count() > 1 && editor.close_current_window() {
        return ok_silent();
    }
    if !editor.tab_page_manager().is_single_tab() {
        editor.close_current_tab();
        return ok(format!(
            "Tab closed. Now on tab {}",
            editor.current_tab_index() + 1
        ));
    }
    if !force && editor.any_buffer_modified() {
        return err(E37);
    }
    editor.quit();
    ok(if force {
        "Quitting (forced)"
    } else {
        "Quitting"
    })
}

pub(super) fn quit(editor: &mut Editor, ex: &Ex) -> CommandResult {
    close_or_quit(editor, ex.bang)
}

/// `:qa[ll]` / `:quita[ll]`: any modified buffer blocks it without `!`.
pub(super) fn quit_all(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if !ex.bang && editor.any_buffer_modified() {
        return err(E37);
    }
    editor.quit();
    ok(if ex.bang {
        "Quitting all (forced)"
    } else {
        "Quitting all"
    })
}

/// `:cq[uit] [N]`: quit with exit code N (default 1).
pub(super) fn cquit(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let code = if ex.args.is_empty() { "1" } else { ex.args };
    match code.parse::<i32>() {
        Ok(code) => {
            editor.quit_with_code(code);
            ok(format!("Quitting with error code {code}"))
        }
        Err(_) => err(format!("Invalid exit code: {code}")),
    }
}

/// `:clo[se]`: close the current window, never the last one.
pub(super) fn close(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    if editor.window_count() > 1 && editor.close_current_window() {
        ok_silent()
    } else {
        err("E444: Cannot close last window")
    }
}

pub(super) fn only(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    if editor.window_count() == 1 {
        ok("Already only one window")
    } else {
        editor.close_other_windows();
        ok("All other windows closed")
    }
}

/// `:sp[lit] [file]` / `:vs[plit] [file]`.
fn split(editor: &mut Editor, ex: &Ex, vertical: bool) -> CommandResult {
    if vertical {
        editor.split_window_vertical();
    } else {
        editor.split_window_horizontal();
    }
    if !ex.args.is_empty() {
        // The other window keeps showing the current buffer, so unsaved
        // changes there are no obstacle (vim: `:sp file` always works).
        let opened = open_file(editor, ex.args);
        if let CommandResult::Error(_) = opened {
            editor.close_current_window();
        }
        return opened;
    }
    ok(format!(
        "Split {} ({} windows)",
        if vertical {
            "vertically"
        } else {
            "horizontally"
        },
        editor.window_count()
    ))
}

pub(super) fn split_horizontal(editor: &mut Editor, ex: &Ex) -> CommandResult {
    split(editor, ex, false)
}

pub(super) fn split_vertical(editor: &mut Editor, ex: &Ex) -> CommandResult {
    split(editor, ex, true)
}

/// Load `filename` into the current window, or start a new buffer with that
/// name when the file does not exist yet (vim: `"name" [New]`). Returns
/// whether the buffer is new.
pub(super) fn open_or_create(editor: &mut Editor, filename: &str) -> anyhow::Result<bool> {
    match editor.load_file(filename) {
        Ok(()) => Ok(false),
        Err(_) if !std::path::Path::new(filename).exists() => {
            let absolute = std::path::absolute(filename)
                .unwrap_or_else(|_| std::path::PathBuf::from(filename));
            editor.add_buffer(crate::buffer::Buffer::new());
            editor.set_file_path(absolute.to_string_lossy().to_string());
            editor.mark_dirty();
            // The tab title derives from the buffer's file path.
            editor.sync_current_tab_buffer();
            Ok(true)
        }
        Err(error) => Err(error),
    }
}

fn tab_number(editor: &Editor) -> usize {
    editor.current_tab_index() + 1
}

/// `:tabnew [file]` / `:tabe[dit] [file]`: a missing file becomes a new
/// buffer with that name.
pub(super) fn tab_new(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.args.is_empty() {
        editor.new_tab();
        return ok(format!("Created tab {}", tab_number(editor)));
    }
    let filename = match expand_tilde(ex.args) {
        Ok(path) => path.to_string_lossy().to_string(),
        Err(e) => return err(format!("Failed to expand path '{}': {}", ex.args, e)),
    };
    editor.new_tab();
    match open_or_create(editor, &filename) {
        Ok(false) => ok(format!("Opened {} in tab {}", filename, tab_number(editor))),
        Ok(true) => ok(format!(
            "Created new file {} in tab {}",
            filename,
            tab_number(editor)
        )),
        Err(e) => err(format!("Failed to load file: {}", e)),
    }
}

pub(super) fn tab_next(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.next_tab();
    ok(format!("Tab {}", tab_number(editor)))
}

pub(super) fn tab_previous(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.previous_tab();
    ok(format!("Tab {}", tab_number(editor)))
}

pub(super) fn tab_first(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.first_tab();
    ok("Tab 1")
}

pub(super) fn tab_last(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.last_tab();
    ok(format!("Tab {}", tab_number(editor)))
}

pub(super) fn tab_close(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    if editor.tab_page_manager().is_single_tab() {
        return err("Cannot close last tab");
    }
    editor.close_current_tab();
    ok(format!("Tab closed. Now on tab {}", tab_number(editor)))
}

pub(super) fn tab_only(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    if editor.tab_page_manager().is_single_tab() {
        return ok("Already only one tab");
    }
    let closed = editor.tab_count() - 1;
    editor.close_other_tabs();
    ok(format!("Closed {closed} tabs"))
}

pub(super) fn tabs(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    let current = editor.current_tab_index();
    let list: Vec<String> = (0..editor.tab_page_manager().tabs().len())
        .map(|index| {
            let marker = if index == current { ">" } else { " " };
            format!("{marker} {} {}", index + 1, editor.get_tab_title(index))
        })
        .collect();
    ok(list.join("\n"))
}

fn buffer_name(editor: &Editor) -> String {
    editor
        .buffer()
        .file_path()
        .and_then(|path| std::path::Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("[No Name]")
        .to_string()
}

fn buffer_status(editor: &Editor) -> CommandResult {
    ok(format!(
        "Buffer {} of {}: {}",
        editor.current_buffer_index() + 1,
        editor.buffer_count(),
        buffer_name(editor)
    ))
}

/// `:ls` / `:buffers` / `:files`.
pub(super) fn list_buffers(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    let list: Vec<String> = editor
        .buffer_names()
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let current = if index == editor.current_buffer_index() {
                "%"
            } else {
                " "
            };
            let modified = if editor
                .buffer_at(index)
                .is_some_and(|buffer| !buffer.change_manager().is_at_save_point())
            {
                "+"
            } else {
                " "
            };
            format!("{current} {modified}  {name}")
        })
        .collect();
    ok(list.join("\n"))
}

pub(super) fn next_buffer(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.next_buffer();
    buffer_status(editor)
}

pub(super) fn previous_buffer(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.prev_buffer();
    buffer_status(editor)
}

/// `:b[uffer] {N|name}`: switch by number or by a unique part of the name.
pub(super) fn buffer(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let target = ex.args;
    if target.is_empty() {
        return ok_silent();
    }
    let index = match target.parse::<usize>() {
        Ok(number) if number >= 1 && number <= editor.buffer_count() => number - 1,
        Ok(number) => return err(format!("E86: Buffer {number} does not exist")),
        Err(_) => {
            let names = editor.buffer_names();
            let matching: Vec<usize> = (0..names.len())
                .filter(|&index| names[index].contains(target))
                .collect();
            match matching.as_slice() {
                [index] => *index,
                [] => return err(format!("E94: No matching buffer for {target}")),
                _ => return err(format!("E93: More than one match for {target}")),
            }
        }
    };
    editor.switch_to_buffer(index);
    ok_silent()
}

/// `:bd[elete][!]` deletes the current buffer; deleting the last one quits.
pub(super) fn delete_buffer(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if !ex.bang && editor.is_modified() {
        return err(E37);
    }
    if editor.delete_current_buffer() {
        editor.quit();
        ok("Last buffer deleted, quitting")
    } else {
        ok(format!(
            "Buffer deleted. Now showing: {}",
            buffer_name(editor)
        ))
    }
}
