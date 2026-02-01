//! Lua scripting support for the editor

#[cfg(feature = "lua")]
use super::Editor;
#[cfg(feature = "lua")]
use crate::lua::LuaContext;
#[cfg(feature = "lua")]
use anyhow::Result;

#[cfg(feature = "lua")]
use super::InputHandler;

#[cfg(feature = "lua")]
impl Editor {
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
            // Try to load config
            match context.load_config() {
                Ok(true) => {
                    // Config loaded successfully - process any commands that were queued
                    let commands = bridge.drain_commands();
                    for cmd in commands {
                        let _ = InputHandler::execute_command_string(self, &cmd);
                    }
                }
                Ok(false) => {
                    // No config file found - not an error
                }
                Err(e) => {
                    crate::log_error!("lua", "Error loading Lua config: {}", e);
                }
            }
            // Load plugins from plugin directories
            if let Err(e) = context.load_plugins() {
                crate::log_error!("lua", "Error loading Lua plugins: {}", e);
            }
            // Process any commands from plugins
            let commands = bridge.drain_commands();
            for cmd in commands {
                let _ = InputHandler::execute_command_string(self, &cmd);
            }
            self.lua_context = Some(context);
            self.editor_bridge = Some(bridge);
        }
        Ok(())
    }

    /// Reloads Lua configuration
    pub fn reload_lua_config(&mut self) -> Result<String> {
        if let Some(ref mut context) = self.lua_context {
            context.reload_config()?;
            // Process any commands that were queued
            if let Some(ref bridge) = self.editor_bridge {
                let commands = bridge.drain_commands();
                for cmd in commands {
                    InputHandler::execute_command_string(self, &cmd)?;
                }
            }
            Ok("Configuration reloaded".to_string())
        } else {
            Ok("Lua not enabled".to_string())
        }
    }

    /// Syncs the current editor state to the Lua bridge
    fn sync_lua_bridge(&self, bridge: &crate::lua::EditorBridge) {
        // Update cursor position
        let cursor = self.buffer().cursor();
        bridge.update_cursor(cursor.line(), cursor.col());
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
        if let Some(ref mut context) = self.lua_context {
            context.execute_file(path)?;
            Ok(())
        } else {
            anyhow::bail!("Lua support not enabled")
        }
    }
}
