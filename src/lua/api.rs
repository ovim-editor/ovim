use crate::lua::editor_bridge::GlobalValue;
use crate::lua::EditorBridge;
use anyhow::Result;
use mlua::{Lua, Table, Value};

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
    let cmd = create_cmd_function(lua, bridge.clone())?;
    vim.set("cmd", cmd)?;

    // Create vim.g (global variables) namespace with metatable
    let g = create_g_table(lua, bridge.clone())?;
    vim.set("g", g)?;

    // Create vim.opt (options) namespace with metatable
    let opt = create_opt_table(lua, bridge.clone())?;
    vim.set("opt", opt)?;

    // Set vim as a global
    lua.globals().set("vim", vim)?;

    Ok(())
}

/// Creates the vim.api table with editor API functions
fn create_api_table(lua: &Lua, bridge: EditorBridge) -> Result<Table<'_>> {
    let api = lua.create_table()?;

    // vim.api.nvim_command(cmd)
    let bridge_clone = bridge.clone();
    let nvim_command = lua.create_function(move |_lua, cmd: String| {
        bridge_clone
            .execute_command(cmd)
            .map_err(mlua::Error::external)?;
        Ok(())
    })?;
    api.set("nvim_command", nvim_command)?;

    // vim.api.nvim_exec(src, output)
    let bridge_clone = bridge.clone();
    let nvim_exec = lua.create_function(move |_lua, (src, output): (String, bool)| {
        // Execute each line as a command
        for line in src.lines() {
            if !line.trim().is_empty() {
                bridge_clone
                    .execute_command(line.to_string())
                    .map_err(mlua::Error::external)?;
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
fn create_fn_table(lua: &Lua, bridge: EditorBridge) -> Result<Table<'_>> {
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
fn create_cmd_function(lua: &Lua, bridge: EditorBridge) -> Result<mlua::Function<'_>> {
    let cmd = lua.create_function(move |_lua, command: String| {
        bridge
            .execute_command(command)
            .map_err(mlua::Error::external)?;
        Ok(())
    })?;
    Ok(cmd)
}

/// Creates the vim.g table with metatable for global variables
fn create_g_table(lua: &Lua, bridge: EditorBridge) -> Result<Table<'_>> {
    let g = lua.create_table()?;
    let metatable = lua.create_table()?;

    // __newindex: called when setting vim.g.name = value
    let bridge_clone = bridge.clone();
    let newindex = lua.create_function(move |_lua, (_, key, value): (Value, String, Value)| {
        let global_value = match value {
            Value::String(s) => GlobalValue::String(s.to_str().unwrap_or("").to_string()),
            Value::Integer(n) => GlobalValue::Integer(n),
            Value::Number(n) => GlobalValue::Number(n),
            Value::Boolean(b) => GlobalValue::Boolean(b),
            Value::Nil => GlobalValue::Nil,
            _ => {
                return Err(mlua::Error::external(format!(
                    "vim.g.{}: unsupported value type",
                    key
                )));
            }
        };
        bridge_clone.set_global(key, global_value);
        Ok(())
    })?;
    metatable.set("__newindex", newindex)?;

    // __index: called when getting vim.g.name
    let bridge_clone = bridge.clone();
    let index = lua.create_function(move |lua, (_, key): (Value, String)| {
        match bridge_clone.get_global(&key) {
            Some(GlobalValue::String(s)) => Ok(Value::String(lua.create_string(&s)?)),
            Some(GlobalValue::Integer(n)) => Ok(Value::Integer(n)),
            Some(GlobalValue::Number(n)) => Ok(Value::Number(n)),
            Some(GlobalValue::Boolean(b)) => Ok(Value::Boolean(b)),
            Some(GlobalValue::Nil) | None => Ok(Value::Nil),
        }
    })?;
    metatable.set("__index", index)?;

    g.set_metatable(Some(metatable));
    Ok(g)
}

/// Creates the vim.opt table with metatable for setting options
fn create_opt_table(lua: &Lua, bridge: EditorBridge) -> Result<Table<'_>> {
    let opt = lua.create_table()?;
    let metatable = lua.create_table()?;

    // __newindex: called when setting opt.name = value
    let bridge_clone = bridge.clone();
    let newindex = lua.create_function(
        move |_lua, (_, key, value): (mlua::Value, String, mlua::Value)| {
            let cmd: String = match key.as_str() {
                "number" | "nu" => match value {
                    mlua::Value::Boolean(true) => "set number".to_string(),
                    mlua::Value::Boolean(false) => "set nonumber".to_string(),
                    _ => return Err(mlua::Error::external("number must be boolean")),
                },
                "relativenumber" | "rnu" => match value {
                    mlua::Value::Boolean(true) => "set relativenumber".to_string(),
                    mlua::Value::Boolean(false) => "set norelativenumber".to_string(),
                    _ => return Err(mlua::Error::external("relativenumber must be boolean")),
                },
                "expandtab" | "et" => match value {
                    mlua::Value::Boolean(true) => "set expandtab".to_string(),
                    mlua::Value::Boolean(false) => "set noexpandtab".to_string(),
                    _ => return Err(mlua::Error::external("expandtab must be boolean")),
                },
                "tabstop" | "ts" => match value {
                    mlua::Value::Integer(n) => format!("set tabstop={}", n),
                    mlua::Value::Number(n) => format!("set tabstop={}", n as i64),
                    _ => return Err(mlua::Error::external("tabstop must be number")),
                },
                "shiftwidth" | "sw" => match value {
                    mlua::Value::Integer(n) => format!("set shiftwidth={}", n),
                    mlua::Value::Number(n) => format!("set shiftwidth={}", n as i64),
                    _ => return Err(mlua::Error::external("shiftwidth must be number")),
                },
                "scroll" => match value {
                    mlua::Value::Integer(n) => format!("set scroll={}", n),
                    mlua::Value::Number(n) => format!("set scroll={}", n as i64),
                    _ => return Err(mlua::Error::external("scroll must be number")),
                },
                "textwidth" | "tw" => match value {
                    mlua::Value::Integer(n) => format!("set textwidth={}", n),
                    mlua::Value::Number(n) => format!("set textwidth={}", n as i64),
                    _ => return Err(mlua::Error::external("textwidth must be number")),
                },
                _ => {
                    return Err(mlua::Error::external(format!("Unknown option: {}", key)));
                }
            };

            bridge_clone
                .execute_command(cmd)
                .map_err(mlua::Error::external)?;
            Ok(())
        },
    )?;
    metatable.set("__newindex", newindex)?;

    opt.set_metatable(Some(metatable));
    Ok(opt)
}
