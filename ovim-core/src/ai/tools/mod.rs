pub mod builtins;
pub mod schema;

use std::collections::HashMap;

use crate::ai::config::AiProfileConfig;
use crate::ai::scope::{Capabilities, RequiredScope};

/// How a tool affects state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideEffect {
    Read,
    Mutation,
    External,
}

/// Parameter type for JSON schema generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamType {
    String,
    Integer,
    Boolean,
    FilePath,
    LineNumber,
    LineRange,
}

/// A single tool parameter.
#[derive(Debug, Clone)]
pub struct ToolParam {
    pub name: String,
    pub param_type: ParamType,
    pub required: bool,
    pub description: String,
}

/// Definition of a tool that can be invoked by the AI.
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub required_scope: RequiredScope,
    pub side_effect: SideEffect,
    pub parameters: Vec<ToolParam>,
}

/// A pending tool invocation with validated scope.
#[derive(Debug, Clone)]
pub struct ToolInvocation {
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Result of executing a tool.
#[derive(Debug, Clone)]
pub enum ToolResult {
    Success(String),
    Error(String),
}

/// Registry of all available tools.
pub struct ToolRegistry {
    tools: HashMap<String, ToolDefinition>,
}

impl ToolRegistry {
    /// Create a new registry with all built-in tools pre-registered.
    pub fn new() -> Self {
        let mut reg = Self {
            tools: HashMap::new(),
        };
        builtins::register_builtins(&mut reg);
        reg
    }

    /// Register a tool definition.
    pub fn register(&mut self, tool: ToolDefinition) {
        self.tools.insert(tool.name.clone(), tool);
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&ToolDefinition> {
        self.tools.get(name)
    }

    /// Return all tools whose required scope fits within the given capabilities.
    pub fn tools_for_scope(&self, caps: &Capabilities) -> Vec<&ToolDefinition> {
        self.tools
            .values()
            .filter(|t| caps.contains(&t.required_scope))
            .collect()
    }

    /// Filter tools by a profile's tool list AND scope capabilities.
    /// If the profile's tool list is empty, all scope-matching tools are returned.
    pub fn tools_for_profile(
        &self,
        profile: &AiProfileConfig,
        caps: &Capabilities,
    ) -> Vec<&ToolDefinition> {
        self.tools
            .values()
            .filter(|t| {
                // Must fit within capabilities
                if !caps.contains(&t.required_scope) {
                    return false;
                }
                // If profile has an explicit tool list, tool must be in it
                if !profile.tools.is_empty() && !profile.tools.contains(&t.name) {
                    return false;
                }
                true
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::FileScope;

    fn make_tool(name: &str, file_scope: FileScope, side_effect: SideEffect) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("Test tool {name}"),
            required_scope: RequiredScope {
                file_scope,
                shell: false,
                network: false,
            },
            side_effect,
            parameters: vec![],
        }
    }

    #[test]
    fn registry_register_and_get() {
        let mut reg = ToolRegistry {
            tools: HashMap::new(),
        };
        reg.register(make_tool("read_file", FileScope::File, SideEffect::Read));
        assert!(reg.get("read_file").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn tools_for_scope_filters_correctly() {
        let mut reg = ToolRegistry {
            tools: HashMap::new(),
        };
        reg.register(make_tool("read_file", FileScope::File, SideEffect::Read));
        reg.register(make_tool(
            "search_project",
            FileScope::Project,
            SideEffect::Read,
        ));
        reg.register(make_tool(
            "run_shell",
            FileScope::File,
            SideEffect::External,
        ));

        // File scope, no shell
        let caps = Capabilities {
            file_scope: FileScope::File,
            shell: false,
            network: false,
        };
        let tools = reg.tools_for_scope(&caps);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(!names.contains(&"search_project")); // Needs Project scope
        assert!(names.contains(&"run_shell")); // File scope is enough, shell not required by required_scope
    }

    #[test]
    fn tools_for_profile_respects_tool_list() {
        let mut reg = ToolRegistry {
            tools: HashMap::new(),
        };
        reg.register(make_tool("read_file", FileScope::File, SideEffect::Read));
        reg.register(make_tool(
            "read_selection",
            FileScope::File,
            SideEffect::Read,
        ));

        let caps = Capabilities {
            file_scope: FileScope::File,
            shell: false,
            network: false,
        };

        // Profile with explicit tool list
        let mut profile = AiProfileConfig {
            name: "test".to_string(),
            provider: crate::ai::types::AiProviderKind::Ollama,
            model: "test".to_string(),
            base_url: None,
            api_key_env: None,
            temperature: None,
            max_tokens: None,
            system_prompt: None,
            extraction: crate::ai::types::ExtractionStrategy::Json,
            context_policy: crate::ai::types::ContextPolicy::default(),
            tools: vec!["read_file".to_string()],
            scope: crate::ai::types::ProfileScope::default(),
            edit_mode: crate::ai::types::EditMode::Format,
            edit_format: "codeblock".to_string(),
        };
        let tools = reg.tools_for_profile(&profile, &caps);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "read_file");

        // Empty tool list = all matching tools
        profile.tools.clear();
        let tools = reg.tools_for_profile(&profile, &caps);
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn default_registry_has_builtins() {
        let reg = ToolRegistry::new();
        assert!(reg.get("read_file").is_some());
        assert!(reg.get("read_selection").is_some());
        assert!(reg.get("read_diagnostics").is_some());
    }
}
