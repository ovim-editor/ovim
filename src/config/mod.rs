#[cfg(feature = "lua")]
use crate::lua::LuaContext;
use anyhow::Result;
use std::path::PathBuf;

/// Configuration manager for ovim
pub struct Config {
    /// Lua context for executing configuration
    #[cfg(feature = "lua")]
    lua_context: LuaContext,
    /// Runtime paths for plugins and scripts
    runtime_paths: Vec<PathBuf>,
}

impl Config {
    /// Creates a new configuration manager
    pub fn new() -> Result<Self> {
        #[cfg(feature = "lua")]
        let lua_context = LuaContext::new()?;
        let runtime_paths = Self::get_runtime_paths();

        Ok(Self {
            #[cfg(feature = "lua")]
            lua_context,
            runtime_paths,
        })
    }

    /// Loads configuration from standard locations
    #[cfg(feature = "lua")]
    pub fn load(&mut self) -> Result<bool> {
        self.lua_context.load_config()
    }

    /// Reloads configuration
    #[cfg(feature = "lua")]
    pub fn reload(&mut self) -> Result<()> {
        self.lua_context.reload_config()
    }

    /// Gets a reference to the Lua context
    #[cfg(feature = "lua")]
    pub fn lua_context(&self) -> &LuaContext {
        &self.lua_context
    }

    /// Gets a mutable reference to the Lua context
    #[cfg(feature = "lua")]
    pub fn lua_context_mut(&mut self) -> &mut LuaContext {
        &mut self.lua_context
    }

    /// Gets the runtime paths for plugins
    pub fn runtime_paths(&self) -> &[PathBuf] {
        &self.runtime_paths
    }

    /// Gets the list of runtime paths where plugins are searched
    fn get_runtime_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // $OVIM_CONFIG/plugins
        if let Ok(ovim_config) = std::env::var("OVIM_CONFIG") {
            let mut path = PathBuf::from(ovim_config);
            path.push("plugins");
            paths.push(path);
        }

        // $XDG_CONFIG_HOME/ovim/plugins
        if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
            let mut path = PathBuf::from(xdg_config);
            path.push("ovim");
            path.push("plugins");
            paths.push(path);
        }

        // ~/.config/ovim/plugins
        if let Some(home) = std::env::var_os("HOME") {
            let mut path = PathBuf::from(&home);
            path.push(".config");
            path.push("ovim");
            path.push("plugins");
            paths.push(path.clone());

            // ~/.ovim/plugins
            let mut alt_path = PathBuf::from(&home);
            alt_path.push(".ovim");
            alt_path.push("plugins");
            paths.push(alt_path);
        }

        paths
    }

    /// Discovers and loads plugins from runtime paths
    #[cfg(feature = "lua")]
    pub fn load_plugins(&mut self) -> Result<()> {
        for runtime_path in &self.runtime_paths {
            if !runtime_path.exists() {
                continue;
            }

            // Iterate through directories in runtime path
            if let Ok(entries) = std::fs::read_dir(runtime_path) {
                for entry in entries.flatten() {
                    if let Ok(file_type) = entry.file_type() {
                        if file_type.is_dir() {
                            // Try to load init.lua from plugin directory
                            let mut init_path = entry.path();
                            init_path.push("init.lua");

                            if init_path.exists() {
                                // Load the plugin
                                if let Err(e) = self.lua_context.execute_file(&init_path) {
                                    eprintln!("Failed to load plugin {:?}: {}", entry.path(), e);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new().expect("Failed to create Config")
    }
}
