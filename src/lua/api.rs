use anyhow::Result;
use mlua::{Lua, Table};
use crate::lua::EditorBridge;

/// Sets up the vim global table with API namespaces
pub fn setup_vim_api(lua: &Lua, bridge: EditorBridge) -> Result<()> {
    let vim = lua.create_table()?;

    // Create vim.api namespace
    let api = create_api_table(lua, bridge.clone())?;
    vim.set("api", api)?;

    // Create vim.fn namespace
    let fn_table = create_fn_table(lua, bridge.clone())?;
    vim.set("fn", fn_table)?;

    // Create vim.cmd function
    let cmd = create_cmd_function(lua, bridge)?;
    vim.set("cmd", cmd)?;

    // Create vim.g (global variables) namespace
    let g = lua.create_table()?;
    vim.set("g", g)?;

    // Create vim.opt (options) namespace
    let opt = lua.create_table()?;
    vim.set("opt", opt)?;

    // Set vim as a global
    lua.globals().set("vim", vim)?;

    Ok(())
}

/// Creates the vim.api table with editor API functions
fn create_api_table(lua: &Lua, bridge: EditorBridge) -> Result<Table> {
    let api = lua.create_table()?;

    // vim.api.nvim_command(cmd)
    let bridge_clone = bridge.clone();
    let nvim_command = lua.create_function(move |_lua, cmd: String| {
        bridge_clone.execute_command(cmd).map_err(mlua::Error::external)?;
        Ok(())
    })?;
    api.set("nvim_command", nvim_command)?;

    // vim.api.nvim_exec(src, output)
    let bridge_clone = bridge.clone();
    let nvim_exec = lua.create_function(move |_lua, (src, output): (String, bool)| {
        // Execute each line as a command
        for line in src.lines() {
            if !line.trim().is_empty() {
                bridge_clone.execute_command(line.to_string()).map_err(mlua::Error::external)?;
            }
        }
        if output {
            Ok(Some("".to_string()))
        } else {
            Ok(None)
        }
    })?;
    api.set("nvim_exec", nvim_exec)?;

    // vim.api.nvim_get_current_line()
    let bridge_clone = bridge.clone();
    let nvim_get_current_line = lua.create_function(move |_lua, ()| {
        if let Some((line, _)) = bridge_clone.get_cursor() {
            Ok(bridge_clone.get_line(line).unwrap_or_default())
        } else {
            Ok("".to_string())
        }
    })?;
    api.set("nvim_get_current_line", nvim_get_current_line)?;

    Ok(api)
}

/// Creates the vim.fn table with vim functions
fn create_fn_table(lua: &Lua, bridge: EditorBridge) -> Result<Table> {
    let fn_table = lua.create_table()?;

    // vim.fn.line('.')
    let bridge_clone = bridge.clone();
    let line = lua.create_function(move |_lua, expr: String| {
        if expr == "." {
            // Current line (1-indexed)
            if let Some((line, _)) = bridge_clone.get_cursor() {
                Ok(line + 1)
            } else {
                Ok(1)
            }
        } else if expr == "$" {
            // Last line
            Ok(bridge_clone.get_line_count())
        } else {
            // Default to 1
            Ok(1)
        }
    })?;
    fn_table.set("line", line)?;

    // vim.fn.col('.')
    let bridge_clone = bridge.clone();
    let col = lua.create_function(move |_lua, expr: String| {
        if expr == "." {
            // Current column (1-indexed)
            if let Some((_, col)) = bridge_clone.get_cursor() {
                Ok(col + 1)
            } else {
                Ok(1)
            }
        } else {
            Ok(1)
        }
    })?;
    fn_table.set("col", col)?;

    Ok(fn_table)
}

/// Creates the vim.cmd function for executing ex commands
fn create_cmd_function(lua: &Lua, bridge: EditorBridge) -> Result<mlua::Function> {
    let cmd = lua.create_function(move |_lua, command: String| {
        bridge.execute_command(command).map_err(mlua::Error::external)?;
        Ok(())
    })?;
    Ok(cmd)
}
