//! Buffer management commands (:bn, :bp, :bd, :ls, etc.)

use crate::command_result::{err, ok, CommandResult};
use crate::editor::Editor;

/// Try to handle a buffer management command.
/// Returns `Some(result)` if the command was handled, `None` otherwise.
pub fn try_handle(editor: &mut Editor, command: &str) -> Option<CommandResult> {
    match command {
        "ls" | "buffers" | "files" => Some(list_buffers(editor)),
        "bnext" | "bn" => Some(next_buffer(editor)),
        "bprev" | "bp" | "bprevious" => Some(prev_buffer(editor)),
        "bd" | "bdelete" => Some(delete_buffer(editor, false)),
        "bd!" | "bdelete!" => Some(delete_buffer(editor, true)),
        _ => None,
    }
}

fn buffer_name(editor: &Editor) -> String {
    editor
        .buffer()
        .file_path()
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("[No Name]")
                .to_string()
        })
        .unwrap_or_else(|| "[No Name]".to_string())
}

fn list_buffers(editor: &Editor) -> CommandResult {
    let buf_list: Vec<String> = editor
        .buffer_names()
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let marker = if i == editor.current_buffer_index() {
                "%"
            } else {
                " "
            };
            let modified = if editor
                .buffer_at(i)
                .is_some_and(|b| !b.change_manager().is_at_save_point())
            {
                "+"
            } else {
                " "
            };
            format!("{} {}  {}", marker, modified, name)
        })
        .collect();
    ok(buf_list.join("\n"))
}

fn next_buffer(editor: &mut Editor) -> CommandResult {
    editor.next_buffer();
    ok(format!(
        "Buffer {} of {}: {}",
        editor.current_buffer_index() + 1,
        editor.buffer_count(),
        buffer_name(editor)
    ))
}

fn prev_buffer(editor: &mut Editor) -> CommandResult {
    editor.prev_buffer();
    ok(format!(
        "Buffer {} of {}: {}",
        editor.current_buffer_index() + 1,
        editor.buffer_count(),
        buffer_name(editor)
    ))
}

fn delete_buffer(editor: &mut Editor, force: bool) -> CommandResult {
    if !force && editor.is_modified() {
        return err("No write since last change (add ! to override)");
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
