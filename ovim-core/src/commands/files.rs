//! Files: writing (`:w`, `:wq`, `:x`, `:wa`, `:up`, `:sav`), editing and
//! reloading (`:e`, `:checkt`, `:rec`) and the working directory.

use super::Ex;
use crate::command_result::{err, ok, ok_silent, CommandResult};
use crate::editor::Editor;

/// Expands `~` to the home directory.
pub(crate) fn expand_tilde(path: &str) -> Result<std::path::PathBuf, String> {
    if path == "~" || path.starts_with("~/") {
        let home = dirs::home_dir().ok_or("Could not determine home directory")?;
        return Ok(if path == "~" {
            home
        } else {
            home.join(&path[2..])
        });
    }
    Ok(std::path::PathBuf::from(path))
}

/// Options for [`save_buffer`].
pub(super) struct SaveOpts<'a> {
    /// Path to save to (None = the buffer's own file).
    pub path: Option<&'a str>,
    /// Skip the read-only and changed-on-disk checks.
    pub force: bool,
}

fn same_file(a: &str, b: &str) -> bool {
    let (a, b) = (std::path::Path::new(a), std::path::Path::new(b));
    a == b
        || match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
}

fn written(path: &str, editor: &Editor) -> String {
    format!(
        "\"{}\" {}L, {}C written",
        path,
        editor.buffer().line_count(),
        editor.buffer().rope().len_chars()
    )
}

/// Save the buffer to its file, or make `opts.path` its file.
pub(super) fn save_buffer(editor: &mut Editor, opts: SaveOpts<'_>) -> CommandResult {
    if !opts.force && editor.buffer().is_read_only() {
        return err("E45: 'readonly' option is set (add ! to override)");
    }

    let resolved = match opts.path {
        Some(raw) => match expand_tilde(raw) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => return err(format!("Failed to expand path '{}': {}", raw, e)),
        },
        None => match editor.buffer().file_path().map(|s| s.to_string()) {
            Some(p) => p,
            None => return err("No file name"),
        },
    };

    let old_path = editor.buffer().file_path().map(|s| s.to_string());

    // Do not silently overwrite changes made by another process. Save-as to a
    // different file remains valid, and the bang variants are the explicit
    // escape hatch when the user intentionally wants the in-memory copy to win.
    let targets_current_file = old_path
        .as_deref()
        .is_some_and(|current| same_file(current, &resolved));
    if !opts.force && targets_current_file && editor.buffer().file_mtime().is_some() {
        match editor.buffer().check_external_modification() {
            Ok(true) => return err("E211: File changed since editing started (add ! to override)"),
            Ok(false) => {}
            Err(error) => return err(format!("Failed to check file before saving: {error}")),
        }
    }

    match editor.buffer_mut().save_as(&resolved) {
        Ok(_) => {
            let new_path = editor.buffer().file_path().map(|s| s.to_string());
            editor.handle_file_path_transition_after_save(old_path, new_path);
            // Git refresh runs on a background thread to avoid blocking the UI.
            editor.spawn_git_refresh(&resolved, editor.options.blame);
            if opts.force {
                editor.buffer_mut().set_read_only(false);
            }
            editor.mark_saved();
            editor.mark_buffer_saved();

            let saved_path = editor
                .buffer()
                .file_path()
                .map(|p| p.to_string())
                .unwrap_or(resolved);
            ok(written(&saved_path, editor))
        }
        Err(e) => err(format!("Failed to save: {}", e)),
    }
}

/// `:[range]w[rite][!] [file]` and `:[range]w[rite] !{cmd}`.
///
/// Like vim, `:w {file}` on a named buffer writes a copy and keeps editing
/// the buffer's own file (`:saveas` renames); an unnamed buffer takes the
/// name. An existing other file needs `!` (E13).
pub(super) fn write(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if let Some(command) = ex.args.strip_prefix('!') {
        let range = ex.range.expect("whole-buffer default");
        return super::shell::write_to_command(editor, range, command.trim());
    }
    if ex.explicit_range {
        return err("E140: Use ! to write partial buffer");
    }
    let current = editor.buffer().file_path().map(str::to_string);
    let target = match (ex.args, &current) {
        ("", _) => None,
        (file, Some(current)) => {
            let path = match expand_tilde(file) {
                Ok(path) => path,
                Err(error) => return err(error),
            };
            let path = path.to_string_lossy().to_string();
            (!same_file(current, &path)).then_some(path)
        }
        // An unnamed buffer takes the name.
        (file, None) => {
            return save_buffer(
                editor,
                SaveOpts {
                    path: Some(file),
                    force: ex.bang,
                },
            )
        }
    };
    let Some(target) = target else {
        return save_buffer(
            editor,
            SaveOpts {
                path: None,
                force: ex.bang,
            },
        );
    };
    if !ex.bang && std::path::Path::new(&target).exists() {
        return err("E13: File exists (add ! to override)");
    }
    match editor.buffer().write_copy(&target) {
        Ok(()) => ok(written(&target, editor)),
        Err(error) => err(format!("Failed to save: {error}")),
    }
}

/// `:wq[!] [file]`, and `:x[it]` / `:exi[t]` which only write a modified
/// buffer; both then quit like `:q`.
fn write_then_quit(editor: &mut Editor, ex: &Ex, only_if_modified: bool) -> CommandResult {
    let needs_write = !only_if_modified || !ex.args.is_empty() || editor.is_modified();
    if needs_write {
        let saved = save_buffer(
            editor,
            SaveOpts {
                path: (!ex.args.is_empty()).then_some(ex.args),
                force: ex.bang,
            },
        );
        if let CommandResult::Error(_) = saved {
            return saved;
        }
    }
    match super::windows::close_or_quit(editor, ex.bang) {
        CommandResult::Success(_) if editor.should_quit() && needs_write => {
            ok("Saved and quitting")
        }
        other => other,
    }
}

pub(super) fn write_quit(editor: &mut Editor, ex: &Ex) -> CommandResult {
    write_then_quit(editor, ex, false)
}

pub(super) fn xit(editor: &mut Editor, ex: &Ex) -> CommandResult {
    write_then_quit(editor, ex, true)
}

/// `:wa[ll][!]`: write every modified buffer. vim (nvim --clean): a
/// modified unnamed buffer reports "E141: No file name for buffer N"
/// without stopping the other writes.
pub(super) fn write_all(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let (written, errors) = editor.write_all_modified_buffers(ex.bang);
    if !errors.is_empty() {
        return err(errors.join("; "));
    }
    if written == 0 {
        return ok_silent();
    }
    ok(format!(
        "{} buffer{} written",
        written,
        if written == 1 { "" } else { "s" }
    ))
}

/// `:wqa[ll]` / `:xa[ll]`: write all, then quit.
pub(super) fn write_all_quit(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let written = write_all(editor, ex);
    if let CommandResult::Error(_) = written {
        return written;
    }
    editor.quit();
    ok("Quitting all")
}

/// `:up[date]`: write only when the buffer has unsaved changes.
pub(super) fn update(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if !editor.current_buffer_needs_write() {
        return ok_silent();
    }
    save_buffer(
        editor,
        SaveOpts {
            path: (!ex.args.is_empty()).then_some(ex.args),
            force: ex.bang,
        },
    )
}

/// `:sav[eas][!] {file}`: write to `file` and make it the buffer's file.
pub(super) fn save_as(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.args.is_empty() {
        return err("E471: Argument required");
    }
    let path = match expand_tilde(ex.args) {
        Ok(path) => path,
        Err(error) => return err(error),
    };
    if !ex.bang && path.exists() {
        return err("E13: File exists (add ! to override)");
    }
    save_buffer(
        editor,
        SaveOpts {
            path: Some(ex.args),
            force: ex.bang,
        },
    )
}

/// `:e[dit][!]` reloads the buffer's file; `:e[dit][!] {file}` opens one
/// (a directory opens the file explorer).
pub(super) fn edit(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.args.is_empty() {
        reload_buffer(editor, ex.bang)
    } else {
        edit_file(editor, ex.args, ex.bang)
    }
}

fn reload_buffer(editor: &mut Editor, force: bool) -> CommandResult {
    if !force && editor.is_modified() {
        return err("No write since last change (add ! to override)");
    }
    let Some(path) = editor.buffer().file_path().map(str::to_string) else {
        return err(if force {
            "No file to reload"
        } else {
            "No file name"
        });
    };
    match editor.buffer_mut().reload_from_disk() {
        Ok(_) => {
            editor.mark_saved();
            editor.mark_buffer_modified_force_send();
            let line_count = editor.buffer().line_count();
            ok(format!("\"{}\" {}L reloaded", path, line_count))
        }
        Err(e) => err(format!("Failed to reload: {}", e)),
    }
}

fn edit_file(editor: &mut Editor, raw_filename: &str, force: bool) -> CommandResult {
    if !force && editor.is_modified() {
        return err("No write since last change (add ! to override)");
    }
    open_file(editor, raw_filename)
}

/// Open a file in the current window. The buffer it was showing stays loaded,
/// so callers that keep it visible elsewhere (`:sp file`) skip the
/// unsaved-changes check of [`edit_file`].
pub(super) fn open_file(editor: &mut Editor, raw_filename: &str) -> CommandResult {
    let filename = match expand_tilde(raw_filename) {
        Ok(path) => path.to_string_lossy().to_string(),
        Err(e) => return err(format!("Failed to expand path '{}': {}", raw_filename, e)),
    };
    let path = std::path::Path::new(&filename);
    if path.is_dir() {
        return match editor.open_directory(path) {
            Ok(()) => ok(format!("Exploring: {}", path.display())),
            Err(error) => err(format!("Failed to open directory: {error}")),
        };
    }
    match super::windows::open_or_create(editor, &filename) {
        Ok(false) => {
            let name = editor
                .buffer()
                .file_path()
                .map(str::to_string)
                .unwrap_or_else(|| "[No Name]".to_string());
            ok(format!("Editing: {}", name))
        }
        // vim: a file that does not exist yet opens as a new buffer.
        Ok(true) => ok(format!("\"{filename}\" [New]")),
        Err(e) => err(format!("Failed to load file: {}", e)),
    }
}

/// `:checkt[ime]`: reload the file if another process changed it.
pub(super) fn checktime(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    match editor.buffer().check_external_modification() {
        Ok(true) => match editor.buffer_mut().reload_if_changed_sync() {
            Ok(true) => {
                editor.mark_buffer_modified_force_send();
                ok("File reloaded from disk (external changes detected)")
            }
            Ok(false) => ok("No external changes detected"),
            Err(e) => err(format!("Failed to reload: {}", e)),
        },
        Ok(false) => ok("No external changes detected"),
        Err(e) => err(format!("Failed to check file: {}", e)),
    }
}

/// `:rec[over]`: restore the buffer from its swap file.
pub(super) fn recover(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    if !editor.buffer().has_swap_file() {
        return err("No swap file exists for this buffer");
    }
    match editor.buffer_mut().recover_from_swap_file() {
        Ok(true) => ok("Buffer recovered from swap file"),
        Ok(false) => err("Failed to recover: swap file is empty or missing"),
        Err(e) => err(format!("Failed to recover: {}", e)),
    }
}

/// `:f[ile]`: name, modified flag and position, like CTRL-G.
pub(super) fn file_info(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    let name = editor
        .buffer()
        .file_path()
        .map(|path| format!("\"{path}\""))
        .unwrap_or_else(|| "\"[No Name]\"".to_string());
    let modified = if editor.is_modified() {
        " [Modified]"
    } else {
        ""
    };
    let line = editor.buffer().cursor().line() + 1;
    let total = editor.buffer().line_count();
    let percent = (line * 100).checked_div(total).unwrap_or(0);
    ok(format!(
        "{name}{modified} line {line} of {total} --{percent}%--"
    ))
}

pub(super) fn pwd(_editor: &mut Editor, _ex: &Ex) -> CommandResult {
    ok(std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "(unknown)".to_string()))
}

/// `:cd [dir]` / `:lc[d] [dir]` (no argument: the home directory).
pub(super) fn cd(_editor: &mut Editor, ex: &Ex) -> CommandResult {
    let target = if ex.args.is_empty() {
        match dirs::home_dir() {
            Some(home) => home,
            None => return err("Could not determine home directory"),
        }
    } else {
        match expand_tilde(ex.args) {
            Ok(path) => path,
            Err(error) => return err(error),
        }
    };
    match std::env::set_current_dir(&target) {
        Ok(()) => ok(target.display().to_string()),
        Err(e) if ex.args.is_empty() => err(format!("Failed to cd: {e}")),
        Err(e) => err(format!(
            "E344: Can't find directory \"{}\" ({})",
            ex.args, e
        )),
    }
}
