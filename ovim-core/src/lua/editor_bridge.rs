use crate::ai::{
    default_api_key_env, infer_provider, parse_provider_str, AiProfileConfig, ContextPolicy,
    EditMode, ExtractionStrategy, FileScope, ProfileScope,
};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A thread-safe bridge between Lua and the Editor
/// Allows Lua code to safely interact with editor state
#[derive(Clone)]
pub struct EditorBridge {
    inner: Arc<Mutex<EditorBridgeInner>>,
}

struct EditorBridgeInner {
    /// Commands to execute on the editor
    pending_commands: Vec<String>,
    /// Current cursor position (line, column)
    cursor_pos: Option<(usize, usize)>,
    /// Current buffer content (cached)
    buffer_content: Option<String>,
    /// Current mode
    mode: Option<String>,
    /// Global variables (vim.g namespace)
    global_vars: HashMap<String, GlobalValue>,
    /// AI context mappings (e.g. "chat" -> "opus")
    ai_contexts: HashMap<String, String>,
    /// AI default profile override
    ai_default_profile: Option<String>,
    /// AI profiles registered from Lua
    ai_profiles: HashMap<String, LuaProfileConfig>,
    /// Pending AI commands from Lua
    ai_pending_commands: Vec<AiCommand>,
    /// Whether AI config has been modified since last sync
    ai_dirty: bool,
}

/// AI profile configuration from Lua (before conversion to AiProfileConfig).
#[derive(Debug, Clone)]
pub struct LuaProfileConfig {
    pub model: String,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub system_prompt: Option<String>,
    pub tools: Vec<String>,
    pub scope: Option<String>,
    pub scope_shell: bool,
    pub scope_network: bool,
    pub edit_mode: Option<String>,
    pub edit_format: Option<String>,
}

impl LuaProfileConfig {
    /// Convert to the engine's AiProfileConfig.
    pub fn into_profile_config(self, name: String) -> AiProfileConfig {
        let provider = self
            .provider
            .as_deref()
            .and_then(parse_provider_str)
            .unwrap_or_else(|| infer_provider(&self.model));

        let scope = ProfileScope {
            files: match self.scope.as_deref() {
                Some("project") => FileScope::Project,
                _ => FileScope::File,
            },
            shell: self.scope_shell,
            network: self.scope_network,
        };

        AiProfileConfig {
            name,
            provider,
            model: self.model,
            base_url: self.base_url,
            api_key_env: self.api_key_env.or_else(|| default_api_key_env(provider)),
            temperature: self.temperature,
            max_tokens: self.max_tokens,
            system_prompt: self.system_prompt,
            extraction: ExtractionStrategy::Raw,
            context_policy: ContextPolicy::default(),
            tools: self.tools,
            scope,
            edit_mode: match self.edit_mode.as_deref() {
                Some("tools") => EditMode::Tools,
                _ => EditMode::Format,
            },
            edit_format: self.edit_format.unwrap_or_else(|| "codeblock".to_string()),
        }
    }
}

/// Snapshot of AI config from Lua bridge: (contexts, default_profile, profiles).
pub type AiConfigSnapshot = (
    HashMap<String, String>,
    Option<String>,
    HashMap<String, LuaProfileConfig>,
);

/// Commands queued by Lua for the editor to process.
#[derive(Debug, Clone)]
pub enum AiCommand {
    OpenChat {
        name: Option<String>,
        profile: Option<String>,
        allow_edits: Option<bool>,
        system_prompt: Option<String>,
        initial_message: Option<String>,
    },
    EditSelection {
        profile: Option<String>,
    },
}

/// Value types that can be stored in vim.g
#[derive(Clone, Debug)]
pub enum GlobalValue {
    String(String),
    Number(f64),
    Integer(i64),
    Boolean(bool),
    Nil,
}

impl EditorBridge {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(EditorBridgeInner {
                pending_commands: Vec::new(),
                cursor_pos: None,
                buffer_content: None,
                mode: None,
                global_vars: HashMap::new(),
                ai_contexts: HashMap::new(),
                ai_default_profile: None,
                ai_profiles: HashMap::new(),
                ai_pending_commands: Vec::new(),
                ai_dirty: false,
            })),
        }
    }

    /// Set a global variable (vim.g.name = value)
    pub fn set_global(&self, name: String, value: GlobalValue) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.global_vars.insert(name, value);
    }

    /// Get a global variable (vim.g.name)
    pub fn get_global(&self, name: &str) -> Option<GlobalValue> {
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.global_vars.get(name).cloned()
    }

    /// Queue a command to be executed on the editor
    pub fn execute_command(&self, command: String) -> Result<()> {
        // Handle mutex poisoning gracefully by recovering the guard
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.pending_commands.push(command);
        Ok(())
    }

    /// Update the cached cursor position
    pub fn update_cursor(&self, line: usize, column: usize) {
        // Handle mutex poisoning gracefully by recovering the guard
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.cursor_pos = Some((line, column));
    }

    /// Get the current cursor position
    pub fn get_cursor(&self) -> Option<(usize, usize)> {
        // Handle mutex poisoning gracefully by recovering the guard
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.cursor_pos
    }

    /// Update the cached buffer content
    pub fn update_buffer(&self, content: String) {
        // Handle mutex poisoning gracefully by recovering the guard
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.buffer_content = Some(content);
    }

    /// Get the current buffer content
    pub fn get_buffer(&self) -> Option<String> {
        // Handle mutex poisoning gracefully by recovering the guard
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.buffer_content.clone()
    }

    /// Update the cached mode
    pub fn update_mode(&self, mode: String) {
        // Handle mutex poisoning gracefully by recovering the guard
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.mode = Some(mode);
    }

    /// Get the current mode
    pub fn get_mode(&self) -> Option<String> {
        // Handle mutex poisoning gracefully by recovering the guard
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.mode.clone()
    }

    /// Get all pending commands and clear the queue
    pub fn drain_commands(&self) -> Vec<String> {
        // Handle mutex poisoning gracefully by recovering the guard
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.pending_commands.drain(..).collect()
    }

    /// Get a specific line from the buffer
    pub fn get_line(&self, line: usize) -> Option<String> {
        // Handle mutex poisoning gracefully by recovering the guard
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(ref content) = inner.buffer_content {
            content.lines().nth(line).map(|s| s.to_string())
        } else {
            None
        }
    }

    // -----------------------------------------------------------------
    // AI config bridge
    // -----------------------------------------------------------------

    pub fn set_ai_context(&self, name: String, profile: String) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.ai_contexts.insert(name, profile);
        inner.ai_dirty = true;
    }

    pub fn get_ai_context(&self, name: &str) -> Option<String> {
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.ai_contexts.get(name).cloned()
    }

    pub fn set_ai_default_profile(&self, name: String) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.ai_default_profile = Some(name);
        inner.ai_dirty = true;
    }

    pub fn get_ai_default_profile(&self) -> Option<String> {
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.ai_default_profile.clone()
    }

    pub fn register_ai_profile(&self, name: String, config: LuaProfileConfig) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.ai_profiles.insert(name, config);
        inner.ai_dirty = true;
    }

    pub fn queue_ai_command(&self, cmd: AiCommand) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.ai_pending_commands.push(cmd);
    }

    pub fn drain_ai_commands(&self) -> Vec<AiCommand> {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.ai_pending_commands.drain(..).collect()
    }

    /// Returns AI config if it was modified since last call.
    /// Clears the dirty flag.
    pub fn take_ai_config_if_dirty(&self) -> Option<AiConfigSnapshot> {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !inner.ai_dirty {
            return None;
        }
        inner.ai_dirty = false;
        Some((
            inner.ai_contexts.clone(),
            inner.ai_default_profile.clone(),
            inner.ai_profiles.clone(),
        ))
    }

    /// Get the number of lines in the buffer
    pub fn get_line_count(&self) -> usize {
        // Handle mutex poisoning gracefully by recovering the guard
        let inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(ref content) = inner.buffer_content {
            content.lines().count()
        } else {
            0
        }
    }
}

impl Default for EditorBridge {
    fn default() -> Self {
        Self::new()
    }
}
