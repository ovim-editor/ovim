//! Lua scripting support for the editor

#[cfg(feature = "lua")]
use super::Editor;
#[cfg(feature = "lua")]
use crate::ai::ChatOpts;
#[cfg(feature = "lua")]
use crate::lua::editor_bridge::AiCommand;
#[cfg(feature = "lua")]
use crate::lua::LuaContext;
#[cfg(feature = "lua")]
use anyhow::Result;

#[cfg(feature = "lua")]
use super::InputHandler;

/// Transient error toast for a failed config or plugin load. Lua errors carry
/// a multi-line traceback; the toast shows the message line and points at the
/// log file, which has the full trace.
#[cfg(feature = "lua")]
fn lua_failure_toast(title: &str, error: &anyhow::Error) -> super::ToastRequest {
    let detail = error.to_string();
    let first_line = detail.lines().next().unwrap_or("unknown error");
    super::ToastRequest::new(
        super::ToastSource::System,
        super::ToastLevel::Error,
        format!(
            "{first_line}\nFull trace: {}",
            crate::log::log_file_path().display()
        ),
    )
    .with_title(title)
}

#[cfg(feature = "lua")]
impl Editor {
    /// Runs a command queued by the user's config or a plugin, surfacing
    /// failures instead of dropping them (OV-00197): a broken keymap or
    /// option in init.lua previously produced no feedback at all.
    fn run_config_command(&mut self, cmd: &str, source: &str) {
        if let Err(error) = InputHandler::execute_command_string(self, cmd) {
            crate::log_warn!("lua", "{} command '{}' failed: {}", source, cmd, error);
            let first_line = error.to_string();
            let first_line = first_line
                .lines()
                .next()
                .unwrap_or("unknown error")
                .to_string();
            self.push_toast(
                super::ToastRequest::new(
                    super::ToastSource::System,
                    super::ToastLevel::Warning,
                    format!("'{cmd}' failed: {first_line}"),
                )
                .with_title(format!("{source} command failed")),
            );
        }
    }

    /// Enables Lua scripting support
    pub fn enable_lua(&mut self) -> Result<()> {
        if self.lua_context.is_none() {
            let mut context = LuaContext::new()?;
            // Create EditorBridge for Lua-Editor communication
            let bridge = crate::lua::EditorBridge::new();
            // Sync initial state to bridge
            self.sync_lua_bridge(&bridge);
            // Set up vim API with bridge
            crate::lua::setup_vim_api(context.lua(), bridge.clone())?;
            crate::lua::language_api::setup_ovim_api(
                context.lua(),
                self.language_catalog.clone(),
                context.source_context(),
            )?;
            // Load built-in defaults (runs before user config)
            context.load_builtin()?;
            bridge.take_ai_chat_context_assignment();
            bridge.take_ai_profile_assignments();
            // Try to load user config
            match context.load_config() {
                Ok(true) => {
                    // Config loaded successfully - process any commands that were queued
                    let commands = bridge.drain_commands();
                    for cmd in commands {
                        self.run_config_command(&cmd, "init.lua");
                    }
                    self.sync_ai_config_from_bridge(&bridge);
                }
                Ok(false) => {
                    // No config file found - not an error
                }
                Err(e) => {
                    crate::log_error!("lua", "Error loading Lua config: {}", e);
                    self.push_toast(lua_failure_toast("init.lua failed to load", &e));
                }
            }
            // Load plugins from plugin directories (failures are logged inside)
            for (plugin_path, error) in context.load_plugins() {
                let plugin = plugin_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| plugin_path.display().to_string());
                self.push_toast(lua_failure_toast(
                    &format!("plugin '{plugin}' failed to load"),
                    &error,
                ));
            }
            // Process any commands from plugins
            let commands = bridge.drain_commands();
            for cmd in commands {
                self.run_config_command(&cmd, "plugin");
            }
            self.sync_ai_config_from_bridge(&bridge);
            self.lua_context = Some(context);
            self.editor_bridge = Some(bridge);
        }
        Ok(())
    }

    /// Reloads Lua configuration
    pub fn reload_lua_config(&mut self) -> Result<String> {
        let Some(ref mut context) = self.lua_context else {
            return Ok("Lua not enabled".to_string());
        };
        self.ai_state.chat_preference.reload();
        self.ai_state.chat_config_override = false;
        context.reload_config()?;

        // Process any commands that were queued during reload
        if let Some(ref bridge) = self.editor_bridge {
            let commands = bridge.drain_commands();
            for cmd in commands {
                InputHandler::execute_command_string(self, &cmd)?;
            }
        }
        // Sync AI config (profiles, contexts, default_profile) registered from Lua.
        // Without this, profiles defined in the reloaded config wouldn't take effect
        // until the next event loop tick called process_lua_commands().
        if let Some(ref bridge) = self.editor_bridge {
            let bridge = bridge.clone();
            self.sync_ai_config_from_bridge(&bridge);
        }
        // Re-resolve an idle chat after reload while retaining its draft/history.
        if !self.ai_chat_has_pending_work() {
            let profile = self.ai_chat_context_profile("chat");
            let model = self
                .ai_chat_remembered_selection()
                .and_then(|selection| selection.model.clone());
            if let Some(chat) = self
                .ai_state
                .chat
                .as_mut()
                .filter(|chat| chat.follow_chat_default)
            {
                chat.opts.profile = profile;
                chat.model_override = model;
            }
        }
        Ok("Configuration reloaded".to_string())
    }

    /// Syncs the current editor state to the Lua bridge
    fn sync_lua_bridge(&self, bridge: &crate::lua::EditorBridge) {
        // Update cursor position
        let cursor = self.buffer().cursor();
        bridge.update_cursor(cursor.line(), cursor.col().0);
        // Update buffer content
        bridge.update_buffer(self.buffer().rope().to_string());
        // Update mode
        bridge.update_mode(format!("{:?}", self.mode));
    }

    /// Sync editor state to Lua bridge and get pending commands
    pub fn get_lua_commands(&self) -> Vec<String> {
        if let Some(ref bridge) = self.editor_bridge {
            // Sync state before getting commands
            self.sync_lua_bridge(bridge);
            // Get and return pending commands
            bridge.drain_commands()
        } else {
            Vec::new()
        }
    }

    /// Update Lua bridge after editor state changes
    pub fn update_lua_state(&self) {
        if let Some(ref bridge) = self.editor_bridge {
            self.sync_lua_bridge(bridge);
        }
    }

    /// Process pending Lua commands and execute them
    pub fn process_lua_commands(&mut self) -> Result<()> {
        let commands = self.get_lua_commands();
        for cmd in commands {
            // Execute each command using InputHandler
            InputHandler::execute_command_string(self, &cmd)?;
        }
        // Resolve registrations before commands that may reference them.
        if let Some(ref bridge) = self.editor_bridge {
            let bridge = bridge.clone();
            self.sync_ai_config_from_bridge(&bridge);
        }
        self.process_ai_bridge_commands();
        Ok(())
    }

    /// Gets a reference to the Lua context
    pub fn lua_context(&self) -> Option<&LuaContext> {
        self.lua_context.as_ref()
    }

    /// Gets a mutable reference to the Lua context
    pub fn lua_context_mut(&mut self) -> Option<&mut LuaContext> {
        self.lua_context.as_mut()
    }

    /// Executes Lua code
    pub fn execute_lua(&mut self, code: &str) -> Result<String> {
        self.with_external_effects(|editor| editor.execute_lua_inner(code))
    }

    fn execute_lua_inner(&mut self, code: &str) -> Result<String> {
        if let Some(ref context) = self.lua_context {
            // Sync state to bridge before execution
            self.update_lua_state();
            // Execute Lua code
            let result = context.execute(code)?;
            Ok(crate::lua::lua_value_to_string(&result))
        } else {
            anyhow::bail!("Lua support not enabled")
        }
    }

    /// Executes a Lua file
    pub fn execute_lua_file(&mut self, path: &str) -> Result<()> {
        self.with_external_effects(|editor| editor.execute_lua_file_inner(path))
    }

    fn execute_lua_file_inner(&mut self, path: &str) -> Result<()> {
        if let Some(ref mut context) = self.lua_context {
            context.execute_file(path)?;
            Ok(())
        } else {
            anyhow::bail!("Lua support not enabled")
        }
    }

    // -----------------------------------------------------------------
    // AI bridge integration
    // -----------------------------------------------------------------

    /// Sync AI config (profiles, contexts, default_profile) from Lua bridge.
    fn sync_ai_config_from_bridge(&mut self, bridge: &crate::lua::EditorBridge) {
        if let Some((
            contexts,
            default_profile,
            profiles,
            api_key_registry,
            prompts,
            format_prompts,
            project_context,
            chat_context,
            agent_loop,
        )) = bridge.take_ai_config_if_dirty()
        {
            if bridge.take_ai_chat_context_assignment() {
                self.ai_state.chat_config_override = true;
            }
            let authored_profiles = bridge.take_ai_profile_assignments();
            if let Some(selection) = self.ai_state.chat_preference.selection.as_mut() {
                if authored_profiles.contains(&selection.profile) {
                    // An authored model beats the remembered model, without
                    // replacing the saved preference on disk.
                    selection.model = None;
                }
            }
            // Merge Lua profiles (Lua wins over TOML on conflict)
            for (name, lua_profile) in profiles {
                let profile = lua_profile.into_profile_config(name.clone());
                self.ai_state.config.profiles.insert(name, profile);
            }
            // Merge contexts
            for (ctx_name, profile_name) in contexts {
                self.ai_state.config.contexts.insert(ctx_name, profile_name);
            }
            // Merge API key registry
            for (key_name, key_config) in api_key_registry {
                self.ai_state
                    .config
                    .api_key_registry
                    .insert(key_name, key_config);
            }
            // Merge prompt templates
            for (prompt_name, template) in prompts {
                self.ai_state.config.prompts.insert(prompt_name, template);
            }
            // Merge format prompts
            for (format_name, prompt) in format_prompts {
                self.ai_state
                    .config
                    .format_prompts
                    .insert(format_name, prompt);
            }
            // Project context, chat context, agent loop configs
            self.ai_state.config.project_context = project_context;
            self.ai_state.config.chat_context = chat_context;
            // Store agent_loop as a global default; per-profile overrides
            // are already handled in LuaProfileConfig::into_profile_config().
            let _ = agent_loop; // reserved for future global agent_loop usage
                                // Default profile
            if let Some(dp) = default_profile {
                self.ai_state.config.default_profile = dp.clone();
                self.ai_state.active_profile = dp;
            }
            if let Err(error) = self
                .ai_state
                .subagents
                .reconfigure_if_idle(&self.ai_state.config)
            {
                crate::log_warn!(
                    "agent_runtime",
                    "could not refresh delegated-agent config: {error}"
                );
            }
        }
    }

    /// Process pending AI commands queued from Lua.
    fn process_ai_bridge_commands(&mut self) {
        let commands = if let Some(ref bridge) = self.editor_bridge {
            bridge.drain_ai_commands()
        } else {
            return;
        };

        for cmd in commands {
            match cmd {
                AiCommand::OpenChat {
                    name,
                    profile,
                    allow_edits,
                    system_prompt,
                    initial_message,
                } => {
                    if let Err(error) = self.open_ai_chat(ChatOpts {
                        name: name.unwrap_or_else(|| "chat".to_string()),
                        profile,
                        allow_edits: allow_edits.unwrap_or(true),
                        system_prompt,
                        initial_message,
                    }) {
                        self.set_status_message(error.to_string());
                    }
                }
                AiCommand::EditSelection { profile } => {
                    let _ = self.start_ai_chat_from_visual_with_profile(profile);
                }
            }
        }
    }
}

#[cfg(all(test, feature = "lua"))]
mod tests {
    use super::lua_failure_toast;
    use std::time::Duration;

    #[test]
    fn lua_failure_toasts_expire_after_the_default_error_lifetime() {
        let toast = lua_failure_toast("init.lua failed to load", &anyhow::anyhow!("boom"));

        assert!(!toast.sticky);
        assert_eq!(toast.ttl, Some(Duration::from_secs(8)));
    }
}

#[cfg(all(test, feature = "lua"))]
mod chat_preference_tests {
    use super::*;
    use crate::ai::chat_preference::{ChatPreference, ChatSelection};
    use crate::ai::AiProviderKind;
    use crate::lua::EditorBridge;

    fn configured_editor(source: &str) -> (Editor, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = Editor::default();
        editor.ai_state.config = crate::ai::AiConfig::default();
        editor.ai_state.chat_preference = ChatPreference::load(dir.path().join("preference.json"));
        editor
            .ai_state
            .chat_preference
            .remember(ChatSelection {
                profile: "claude_code".into(),
                provider: AiProviderKind::ClaudeCode,
                model: Some("opus[1m]".into()),
            })
            .unwrap();
        let context = LuaContext::new().unwrap();
        let bridge = EditorBridge::new();
        crate::lua::setup_vim_api(context.lua(), bridge.clone()).unwrap();
        context.load_builtin().unwrap();
        bridge.take_ai_chat_context_assignment();
        bridge.take_ai_profile_assignments();
        context.execute_void(source).unwrap();
        editor.sync_ai_config_from_bridge(&bridge);
        editor.lua_context = Some(context);
        editor.editor_bridge = Some(bridge);
        (editor, dir)
    }

    #[test]
    fn builtins_and_unrelated_init_settings_do_not_override_the_saved_choice() {
        let (mut editor, _dir) = configured_editor("vim.ai.contexts.query = 'codex_terra'");
        assert_eq!(editor.ai_chat_effective_profile(), "claude_code");
        editor.open_ai_chat(ChatOpts::default()).unwrap();
        assert_eq!(editor.ai_chat_selected_model(), "opus[1m]");
        assert_eq!(editor.ai_chat_resolved_profile().unwrap().model, "opus[1m]");
        assert_eq!(
            editor.ai_state.config.profiles["claude_code"].model,
            "default"
        );
        assert_eq!(
            editor.ai_chat_context_profile("query").as_deref(),
            Some("codex_terra")
        );
    }

    #[test]
    fn explicit_lua_settings_win_even_when_equal_to_builtins() {
        for source in [
            "vim.ai.contexts.chat = 'codex_sol'",
            "vim.ai.default_profile = 'codex_sol'",
            "vim.ai.setup({default_profile = 'codex_sol'})",
            "vim.ai.setup({contexts = {chat = {profile = 'codex_sol'}}})",
        ] {
            let (mut editor, dir) = configured_editor(source);
            assert_eq!(editor.ai_chat_effective_profile(), "codex_sol", "{source}");
            editor.open_ai_chat(ChatOpts::default()).unwrap();
            assert_eq!(editor.ai_chat_selected_model(), "gpt-5.6-sol", "{source}");
            // A live picker change works, but does not rewrite authored config.
            assert!(editor.ai_select_chat_model("claude_code", "sonnet"));
            editor.execute_lua(source).unwrap();
            editor.process_lua_commands().unwrap();
            assert_eq!(
                editor.ai_chat_context_profile("chat").as_deref(),
                Some("codex_sol")
            );
            assert_eq!(
                ChatPreference::load(dir.path().join("preference.json"))
                    .selection
                    .unwrap()
                    .model
                    .as_deref(),
                Some("sonnet")
            );
        }
    }

    #[test]
    fn lua_can_register_and_open_a_profile_together_and_reports_invalid_options() {
        let (mut editor, _dir) = configured_editor("");
        editor.execute_lua("vim.ai.profiles.register('custom', {provider='claude_code', model='sonnet'}); vim.ai.open_chat({profile='custom'})").unwrap();
        editor.process_lua_commands().unwrap();
        assert_eq!(editor.ai_chat_effective_profile(), "custom");
        assert_eq!(editor.ai_chat_selected_model(), "sonnet");
        editor
            .execute_lua("vim.ai.open_chat({profile='missing'})")
            .unwrap();
        editor.process_lua_commands().unwrap();
        assert!(editor
            .status_message()
            .contains("Unknown AI profile: missing"));
        assert_eq!(editor.ai_chat_effective_profile(), "custom");
    }

    #[test]
    fn authored_profile_model_beats_saved_model_without_changing_the_saved_file() {
        let (mut editor, dir) = configured_editor("vim.ai.setup({profiles = {claude_code = {provider = 'claude_code', model = 'sonnet'}}})");
        editor.open_ai_chat(ChatOpts::default()).unwrap();
        assert_eq!(editor.ai_chat_effective_profile(), "claude_code");
        assert_eq!(editor.ai_chat_selected_model(), "sonnet");
        assert_eq!(
            ChatPreference::load(dir.path().join("preference.json"))
                .selection
                .unwrap()
                .model
                .as_deref(),
            Some("opus[1m]")
        );
    }
}
