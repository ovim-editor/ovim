//! `vim.ai.*` Lua API namespace.

use super::editor_bridge::{AiCommand, EditorBridge, LuaProfileConfig};
use mlua::{Lua, Result, Table, Value};

/// Build the `vim.ai` table and all sub-namespaces.
pub fn setup_ai_api(lua: &Lua, bridge: EditorBridge) -> Result<Table<'_>> {
    let ai = lua.create_table()?;

    // vim.ai.setup(opts)
    {
        let b = bridge.clone();
        let setup = lua.create_function(move |_lua, opts: Table| {
            // default_profile
            if let Ok(dp) = opts.get::<_, String>("default_profile") {
                b.set_ai_default_profile(dp);
            }

            // contexts — string shorthand or table form
            if let Ok(contexts) = opts.get::<_, Table>("contexts") {
                for pair in contexts.pairs::<String, Value>() {
                    let (key, val) = pair?;
                    let profile_name = match val {
                        Value::String(s) => s.to_str()?.to_string(),
                        Value::Table(t) => t.get::<_, String>("profile").map_err(|_| {
                            mlua::Error::external("contexts entry table must have 'profile' field")
                        })?,
                        _ => {
                            return Err(mlua::Error::external(format!(
                                "contexts.{}: expected string or table",
                                key
                            )));
                        }
                    };
                    b.set_ai_context(key, profile_name);
                }
            }

            // profiles
            if let Ok(profiles) = opts.get::<_, Table>("profiles") {
                for pair in profiles.pairs::<String, Table>() {
                    let (name, tbl) = pair?;
                    let config = parse_lua_profile(&tbl)?;
                    b.register_ai_profile(name, config);
                }
            }

            Ok(())
        })?;
        ai.set("setup", setup)?;
    }

    // vim.ai.contexts — metatable with __index/__newindex
    {
        let contexts = lua.create_table()?;
        let meta = lua.create_table()?;

        let b = bridge.clone();
        let index = lua.create_function(move |lua, (_, key): (Value, String)| {
            match b.get_ai_context(&key) {
                Some(v) => Ok(Value::String(lua.create_string(&v)?)),
                None => Ok(Value::Nil),
            }
        })?;
        meta.set("__index", index)?;

        let b = bridge.clone();
        let newindex =
            lua.create_function(move |_lua, (_, key, value): (Value, String, String)| {
                b.set_ai_context(key, value);
                Ok(())
            })?;
        meta.set("__newindex", newindex)?;

        contexts.set_metatable(Some(meta));
        ai.set("contexts", contexts)?;
    }

    // vim.ai.default_profile — via metatable on ai table itself (handled at the end).

    // vim.ai.open_chat(opts)
    {
        let b = bridge.clone();
        let open_chat = lua.create_function(move |_lua, opts: Option<Table>| {
            let (name, profile, allow_edits, system_prompt, initial_message) =
                if let Some(ref opts) = opts {
                    (
                        opts.get::<_, String>("name").ok(),
                        opts.get::<_, String>("profile").ok(),
                        opts.get::<_, bool>("allow_edits").ok(),
                        opts.get::<_, String>("system_prompt").ok(),
                        opts.get::<_, String>("initial_message").ok(),
                    )
                } else {
                    (None, None, None, None, None)
                };

            b.queue_ai_command(AiCommand::OpenChat {
                name,
                profile,
                allow_edits,
                system_prompt,
                initial_message,
            });
            Ok(())
        })?;
        ai.set("open_chat", open_chat)?;
    }

    // vim.ai.edit_selection(opts)
    {
        let b = bridge.clone();
        let edit_selection = lua.create_function(move |_lua, opts: Option<Table>| {
            let profile = opts.and_then(|t| t.get::<_, String>("profile").ok());
            b.queue_ai_command(AiCommand::EditSelection { profile });
            Ok(())
        })?;
        ai.set("edit_selection", edit_selection)?;
    }

    // vim.ai.profiles.register(name, opts)
    {
        let profiles_table = lua.create_table()?;

        let b = bridge.clone();
        let register = lua.create_function(move |_lua, (name, opts): (String, Table)| {
            let config = parse_lua_profile(&opts)?;
            b.register_ai_profile(name, config);
            Ok(())
        })?;
        profiles_table.set("register", register)?;

        ai.set("profiles", profiles_table)?;
    }

    // Stubs: vim.ai.models.register, vim.ai.tools.register, vim.ai.edit_formats.register
    {
        let models = lua.create_table()?;
        let models_register = lua.create_function(|_lua, (_name, _opts): (String, Table)| {
            // M4/M5 stub — register data, no behavior yet
            Ok(())
        })?;
        models.set("register", models_register)?;
        ai.set("models", models)?;

        let tools = lua.create_table()?;
        let tools_register = lua.create_function(|_lua, (_name, _opts): (String, Table)| Ok(()))?;
        tools.set("register", tools_register)?;
        ai.set("tools", tools)?;

        let edit_formats = lua.create_table()?;
        let edit_formats_register =
            lua.create_function(|_lua, (_name, _opts): (String, Table)| Ok(()))?;
        edit_formats.set("register", edit_formats_register)?;
        ai.set("edit_formats", edit_formats)?;
    }

    // Metatable for vim.ai itself — handles default_profile as a virtual field.
    {
        let meta = lua.create_table()?;

        let b = bridge.clone();
        let index = lua.create_function(move |lua, (_, key): (Table, String)| {
            if key == "default_profile" {
                match b.get_ai_default_profile() {
                    Some(v) => Ok(Value::String(lua.create_string(&v)?)),
                    None => Ok(Value::Nil),
                }
            } else {
                Ok(Value::Nil)
            }
        })?;
        meta.set("__index", index)?;

        let b = bridge.clone();
        let newindex =
            lua.create_function(move |_lua, (tbl, key, value): (Table, String, Value)| {
                if key == "default_profile" {
                    let s = match value {
                        Value::String(s) => s.to_str()?.to_string(),
                        _ => {
                            return Err(mlua::Error::external(
                                "vim.ai.default_profile must be a string",
                            ));
                        }
                    };
                    b.set_ai_default_profile(s);
                    Ok(())
                } else {
                    // Allow setting other fields normally via rawset
                    tbl.raw_set(key, value)?;
                    Ok(())
                }
            })?;
        meta.set("__newindex", newindex)?;

        ai.set_metatable(Some(meta));
    }

    Ok(ai)
}

/// Parse a Lua table into a LuaProfileConfig.
fn parse_lua_profile(tbl: &Table) -> Result<LuaProfileConfig> {
    let model: String = tbl
        .get::<_, String>("model")
        .map_err(|_| mlua::Error::external("profile must have a 'model' field"))?;

    Ok(LuaProfileConfig {
        model,
        provider: tbl.get::<_, String>("provider").ok(),
        base_url: tbl.get::<_, String>("base_url").ok(),
        api_key_env: tbl.get::<_, String>("api_key_env").ok(),
        temperature: tbl.get::<_, f32>("temperature").ok(),
        max_tokens: tbl.get::<_, u32>("max_tokens").ok(),
        system_prompt: tbl.get::<_, String>("system_prompt").ok(),
        tools: parse_string_list(tbl, "tools"),
        scope: tbl.get::<_, String>("scope").ok(),
        scope_shell: tbl.get::<_, bool>("scope_shell").unwrap_or(false),
        scope_network: tbl.get::<_, bool>("scope_network").unwrap_or(false),
        edit_mode: tbl.get::<_, String>("edit_mode").ok(),
        edit_format: tbl.get::<_, String>("edit_format").ok(),
    })
}

/// Extract a string list from a Lua table field (returns empty vec on missing/invalid).
fn parse_string_list(tbl: &Table, key: &str) -> Vec<String> {
    tbl.get::<_, Table>(key)
        .ok()
        .map(|t| {
            t.sequence_values::<String>()
                .filter_map(|r| r.ok())
                .collect()
        })
        .unwrap_or_default()
}
