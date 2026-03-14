//! Buffer management commands (:bn, :bp, :bd, :ls, etc.)

use crate::command_result::{CommandResult, ErrorResponse, SuccessResponse};
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

fn buffer_display_name(editor: &Editor) -> String {
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
            let modified = if i < editor.buffer_count()
                && !editor.buffers[i].change_manager().is_at_save_point()
            {
                "+"
            } else {
                " "
            };
            format!("{} {}  {}", marker, modified, name)
        })
        .collect();
    CommandResult::Success(SuccessResponse {
        success: true,
        message: Some(buf_list.join("\n")),
        line_count: None,
    })
}

fn next_buffer(editor: &mut Editor) -> CommandResult {
    editor.next_buffer();
    let name = buffer_display_name(editor);
    let idx = editor.current_buffer_index() + 1;
    let total = editor.buffer_count();
    CommandResult::Success(SuccessResponse {
        success: true,
        message: Some(format!("Buffer {} of {}: {}", idx, total, name)),
        line_count: None,
    })
}

fn prev_buffer(editor: &mut Editor) -> CommandResult {
    editor.prev_buffer();
    let name = buffer_display_name(editor);
    let idx = editor.current_buffer_index() + 1;
    let total = editor.buffer_count();
    CommandResult::Success(SuccessResponse {
        success: true,
        message: Some(format!("Buffer {} of {}: {}", idx, total, name)),
        line_count: None,
    })
}

fn delete_buffer(editor: &mut Editor, force: bool) -> CommandResult {
    if !force && editor.is_modified() {
        return CommandResult::Error(ErrorResponse {
            error: "No write since last change (add ! to override)".to_string(),
        });
    }

    let should_quit = editor.delete_current_buffer();
    if should_quit {
        editor.quit();
        CommandResult::Success(SuccessResponse {
            success: true,
            message: Some("Last buffer deleted, quitting".to_string()),
            line_count: None,
        })
    } else {
        let name = buffer_display_name(editor);
        CommandResult::Success(SuccessResponse {
            success: true,
            message: Some(format!("Buffer deleted. Now showing: {}", name)),
            line_count: None,
        })
    }
}
