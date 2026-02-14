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
            .filter(|t| caps.contains(&t.required_scope) && caps.allows_side_effect(t.side_effect))
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
                // Must fit within capabilities (scope + side effect)
                if !caps.contains(&t.required_scope) || !caps.allows_side_effect(t.side_effect) {
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
            allow_mutations: true,
        };
        let tools = reg.tools_for_scope(&caps);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(!names.contains(&"search_project")); // Needs Project scope
        assert!(!names.contains(&"run_shell")); // External side effect blocked (shell=false)
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
            allow_mutations: true,
        };

        // Profile with explicit tool list
        let mut profile = AiProfileConfig {
            name: "test".to_string(),
            provider: crate::ai::types::AiProviderKind::Ollama,
            model: "test".to_string(),
            base_url: None,
            api_key: None,
            api_key_env: None,
            temperature: None,
            max_tokens: None,
            system_prompt: None,
            edit_format: crate::ai::types::EditFormat::Json,
            chat_edit_format: None,
            context: crate::ai::types::ContextGatheringPolicy::default(),
            agent_loop: crate::ai::types::AgentLoopConfig::default(),
            tools: vec!["read_file".to_string()],
            scope: crate::ai::types::ProfileScope::default(),
            edit_prompt: None,
            chat_prompt: None,
            chat_edit_prompt: None,
            reasoning_effort: None,
            verbosity: None,
            syntax_check: None,
            retry: crate::ai::types::RetryPolicy::default(),
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
        assert!(reg.get("search_project").is_some());
        assert!(reg.get("list_files").is_some());
        assert!(reg.get("edit_range").is_some());
        assert!(reg.get("insert_lines").is_some());
        assert!(reg.get("delete_lines").is_some());
    }

    #[test]
    fn mutation_tools_excluded_when_allow_edits_false() {
        let reg = ToolRegistry::new();
        let caps = Capabilities {
            file_scope: FileScope::Project,
            shell: true,
            network: true,
            allow_mutations: false,
        };
        let tools = reg.tools_for_scope(&caps);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        // Read tools present
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"search_project"));
        assert!(names.contains(&"list_files"));
        // Mutation tools excluded
        assert!(!names.contains(&"edit_range"));
        assert!(!names.contains(&"insert_lines"));
        assert!(!names.contains(&"delete_lines"));
    }

    #[test]
    fn mutation_tools_included_when_allow_edits_true() {
        let reg = ToolRegistry::new();
        let caps = Capabilities {
            file_scope: FileScope::Project,
            shell: true,
            network: true,
            allow_mutations: true,
        };
        let tools = reg.tools_for_scope(&caps);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"edit_range"));
        assert!(names.contains(&"insert_lines"));
        assert!(names.contains(&"delete_lines"));
    }
}
