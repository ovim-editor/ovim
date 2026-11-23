//! Options, mappings, listings and configuration: `:set`, `:colo`, the
//! `:map` family, `:noh`, `:reg`, `:marks`, `:help`, `:blame`, `:so`,
//! `:lua`, `:luaf` and `:reload`.

use super::set::handle_set_command;
use super::Ex;
use crate::command_result::{err, ok, ok_silent, CommandResult};
use crate::editor::{Editor, KeyMapManager, MapMode};

/// `:se[t] {option} ...`: each option in turn (vim), except a `path=`
/// value, which takes the rest of the line. Bare `:set` lists options.
pub(super) fn set(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.args.contains(" path=") || !ex.args.contains(char::is_whitespace) {
        return handle_set_command(editor, ex.args);
    }
    let mut last = ok_silent();
    for option in ex.args.split_whitespace() {
        match handle_set_command(editor, option) {
            error @ CommandResult::Error(_) => return error,
            CommandResult::Success(success) if success.message.is_some() => {
                last = CommandResult::Success(success)
            }
            CommandResult::Success(_) => {}
        }
    }
    last
}

/// `:unset pullbase [path=...]`: forget a pull base.
pub(super) fn unset(editor: &mut Editor, ex: &Ex) -> CommandResult {
    match ex.args.split_once(' ') {
        Some(("pullbase", path)) if path.trim_start().starts_with("path=") => {
            handle_set_command(editor, &format!("pullbase= {}", path.trim_start()))
        }
        _ if ex.args == "pullbase" => handle_set_command(editor, "pullbase="),
        _ => err("Usage: :unset pullbase [path=DIR]"),
    }
}

/// `:colo[rscheme] [name]`: show or switch the color scheme.
pub(super) fn colorscheme(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.args.is_empty() {
        return ok(format!(
            "Current: {}\nAvailable: {}",
            editor.current_color_scheme_name(),
            editor.list_color_schemes().join(", ")
        ));
    }
    match editor.set_color_scheme(ex.args) {
        Ok(_) => ok(format!("Color scheme set to '{}'", ex.args)),
        Err(e) => err(format!(
            "{}. Available schemes: {}",
            e,
            editor.list_color_schemes().join(", ")
        )),
    }
}

/// Key notation with `<leader>` expanded.
fn map_keys(editor: &Editor, keys: &str) -> String {
    let leader = editor.leader_key().to_string();
    KeyMapManager::parse_key_notation(
        &keys
            .replace("<leader>", &leader)
            .replace("<Leader>", &leader),
    )
}

/// `:map {lhs} {rhs}` and friends; with only `{lhs}` show that mapping,
/// without arguments list the mode's mappings.
pub(super) fn map(editor: &mut Editor, ex: &Ex, mode: MapMode, noremap: bool) -> CommandResult {
    let mut parts = ex.args.splitn(2, char::is_whitespace);
    let lhs = parts.next().unwrap_or("");
    let rhs = parts.next().map(str::trim_start).unwrap_or("");
    if lhs.is_empty() {
        let mappings = editor.keymaps().list_mappings(Some(mode));
        if mappings.is_empty() {
            return ok("No mappings");
        }
        let lines: Vec<String> = mappings
            .into_iter()
            .map(|(mode, mapping)| {
                format!(
                    "{}{}  {}  {}",
                    mode.display_char(),
                    if mapping.noremap { '*' } else { ' ' },
                    mapping.lhs,
                    mapping.rhs
                )
            })
            .collect();
        return ok(format!("--- Mappings ---\n{}", lines.join("\n")));
    }
    let lhs = map_keys(editor, lhs);
    if rhs.is_empty() {
        return match editor.keymaps().get_mapping(mode, &lhs) {
            Some(mapping) => ok(format!(
                "{}  {}  {}",
                mode.display_char(),
                mapping.lhs,
                mapping.rhs
            )),
            None => ok("No mapping found"),
        };
    }
    let rhs = map_keys(editor, rhs);
    editor.keymaps_mut().add_mapping(mode, lhs, rhs, noremap);
    ok_silent()
}

pub(super) fn unmap(editor: &mut Editor, ex: &Ex, mode: MapMode) -> CommandResult {
    let Some(lhs) = ex.args.split_whitespace().next() else {
        return err("E474: Invalid argument");
    };
    let lhs = map_keys(editor, lhs);
    if editor.keymaps_mut().remove_mapping(mode, &lhs) {
        ok_silent()
    } else {
        err("E31: No such mapping")
    }
}

pub(super) fn mapclear(editor: &mut Editor, mode: MapMode) -> CommandResult {
    editor.keymaps_mut().clear_mappings(mode);
    ok_silent()
}

pub(super) fn nohlsearch(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.clear_search_highlight();
    ok("Search highlighting cleared")
}

/// `:reg[isters] [names]` / `:di[splay] [names]`: list the registers, or
/// only the named ones (vim).
pub(super) fn registers(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let wanted: Vec<char> = ex.args.chars().filter(|c| !c.is_whitespace()).collect();
    let lines: Vec<String> = editor
        .registers()
        .list_registers()
        .into_iter()
        .filter(|(name, _)| {
            wanted.is_empty() || name.chars().nth(1).is_some_and(|c| wanted.contains(&c))
        })
        .map(|(name, content)| format!("{name:<4} {content}"))
        .collect();
    if lines.is_empty() {
        return ok("No registers in use");
    }
    ok(format!("--- Registers ---\n{}", lines.join("\n")))
}

/// `:marks [names]`: list the marks, or only the named ones (vim).
pub(super) fn marks(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let wanted: Vec<char> = ex.args.chars().filter(|c| !c.is_whitespace()).collect();
    let lines: Vec<String> = editor
        .list_marks()
        .into_iter()
        .filter(|(name, ..)| wanted.is_empty() || wanted.contains(name))
        .map(|(name, line, col, file)| match file {
            Some(file) => format!(" '{name}  {:>5}  {col:>3}  {file}", line + 1),
            None => format!(" '{name}  {:>5}  {col:>3}", line + 1),
        })
        .collect();
    if lines.is_empty() {
        return ok("No marks set");
    }
    ok(format!("mark  line   col  file\n{}", lines.join("\n")))
}

/// `:h[elp] keybindings` points at the compatibility guide; ovim has no
/// other help pages.
pub(super) fn help(_editor: &mut Editor, ex: &Ex) -> CommandResult {
    match ex.args {
        "keybindings" | "keys" => {
            ok("Keybinding compatibility guide: architecture/knowledge/keybinding-compat.md")
        }
        "" => err("E149: Sorry, no help for ovim; try :help keybindings"),
        topic => err(format!("E149: Sorry, no help for {topic}")),
    }
}

/// `:blame` toggles inline git blame.
pub(super) fn blame(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.options.blame = !editor.options.blame;
    if editor.options.blame {
        editor.buffer_mut().load_git_blame();
        ok("blame on")
    } else {
        editor.buffer_mut().clear_git_blame();
        ok("blame off")
    }
}

#[cfg(not(feature = "lua"))]
fn no_lua() -> CommandResult {
    err("Lua support not compiled in")
}

/// `:lua {code}`.
pub(super) fn lua(_editor: &mut Editor, _ex: &Ex) -> CommandResult {
    #[cfg(feature = "lua")]
    {
        match _editor.execute_lua(_ex.args) {
            Ok(result) => ok(result),
            Err(e) => err(format!("Lua error: {}", e)),
        }
    }
    #[cfg(not(feature = "lua"))]
    no_lua()
}

/// `:luaf[ile] {file}`.
pub(super) fn luafile(_editor: &mut Editor, ex: &Ex) -> CommandResult {
    let _path = ex.args;
    #[cfg(feature = "lua")]
    {
        match _editor.execute_lua_file(_path) {
            Ok(_) => ok(format!("Executed {}", _path)),
            Err(e) => err(format!("Lua error: {}", e)),
        }
    }
    #[cfg(not(feature = "lua"))]
    no_lua()
}

/// `:reload` / `:ConfigReload`: re-run the Lua config.
pub(super) fn reload(_editor: &mut Editor, _ex: &Ex) -> CommandResult {
    #[cfg(feature = "lua")]
    {
        match _editor.reload_lua_config() {
            Ok(msg) => ok(msg),
            Err(e) => err(format!("Failed to reload config: {}", e)),
        }
    }
    #[cfg(not(feature = "lua"))]
    no_lua()
}

/// `:so[urce] {file}`: run a Lua file and the commands it queues,
/// reporting failed commands instead of dropping them (OV-00197).
pub(super) fn source(_editor: &mut Editor, ex: &Ex) -> CommandResult {
    let _path = std::path::Path::new(ex.args);
    #[cfg(feature = "lua")]
    {
        let editor = _editor;
        let Some(context) = editor.lua_context_mut() else {
            return err("Lua not enabled");
        };
        if let Err(e) = context.execute_file(_path) {
            return err(format!("Failed to source {}: {}", ex.args, e));
        }
        let mut failed = Vec::new();
        for command in editor.get_lua_commands() {
            if let CommandResult::Error(error) = super::run_line(editor, &command) {
                crate::log_warn!(
                    "lua",
                    "sourced command '{}' failed: {}",
                    command,
                    error.error
                );
                failed.push(command);
            }
        }
        if failed.is_empty() {
            ok(format!("Sourced: {}", _path.display()))
        } else {
            ok(format!(
                "Sourced: {} ({} command(s) failed: {}; see log)",
                _path.display(),
                failed.len(),
                failed.join(", ")
            ))
        }
    }
    #[cfg(not(feature = "lua"))]
    no_lua()
}
