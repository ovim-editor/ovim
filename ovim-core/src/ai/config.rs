use crate::ai::types::{
    AgentLoopConfig, AiProviderKind, ApiKeyConfig, ContextGatheringPolicy, DiagnosticScope,
    EditFormat, FileScope, ProfileScope, RetryPolicy, ToolApprovalMode, PROFILE_LOCAL,
};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// A selectable profile/model pair shared by both chat interfaces.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AiChatModelOption {
    pub id: String,
    pub label: String,
    pub provider: String,
    pub model: String,
}

/// A provider-defined permission mode exposed by the shared chat controls.
/// The ID is the exact value passed to the provider runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct AiPermissionModeOption {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    /// A mode that disables the provider's approval prompts. Pickers never
    /// offer it, it is enabled only by an explicit, repeated command, and it
    /// applies to the current chat without being remembered.
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiPermissionModes {
    pub default: &'static str,
    pub options: &'static [AiPermissionModeOption],
}

#[derive(Debug, Clone)]
pub struct AiProfileConfig {
    pub name: String,
    pub provider: AiProviderKind,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub system_prompt: Option<String>,
    pub edit_format: EditFormat,
    pub chat_edit_format: Option<EditFormat>,
    pub context: ContextGatheringPolicy,
    pub agent_loop: AgentLoopConfig,
    pub tools: Vec<String>,
    pub scope: ProfileScope,
    pub edit_prompt: Option<String>,
    pub chat_prompt: Option<String>,
    pub chat_edit_prompt: Option<String>,
    pub reasoning_effort: Option<String>,
    pub verbosity: Option<String>,
    pub syntax_check: Option<bool>,
    pub retry: RetryPolicy,
}

impl AiProfileConfig {
    fn permission_capability(&self) -> Option<&'static AiPermissionModes> {
        match self.provider {
            AiProviderKind::ClaudeCode => Some(&super::claude_code::PERMISSION_CAPABILITY),
            _ => None,
        }
    }

    pub fn permission_modes(&self) -> &'static [AiPermissionModeOption] {
        self.permission_capability()
            .map(|capability| capability.options)
            .unwrap_or_default()
    }

    pub fn default_permission_mode(&self) -> Option<&'static str> {
        self.permission_capability()
            .map(|capability| capability.default)
    }

    pub fn validate_permission_mode(&self, mode: &str) -> Result<()> {
        if self
            .permission_modes()
            .iter()
            .any(|option| option.id == mode)
        {
            return Ok(());
        }
        anyhow::bail!(
            "Permission mode {mode:?} is not supported by {}",
            self.provider
        )
    }

    /// Shared boundary for interactive and restored chat model selections.
    /// Most providers select models through profiles; Claude also accepts aliases.
    pub(crate) fn validate_chat_model(&self, model: &str) -> Result<()> {
        if model.is_empty()
            || model.len() > 512
            || model.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            anyhow::bail!(
                "Model must be a nonempty alias or model ID without whitespace (at most 512 bytes)"
            );
        }
        if self.provider != AiProviderKind::ClaudeCode && self.model != model {
            anyhow::bail!("Configure this provider's model in its AI profile");
        }
        Ok(())
    }

    /// One precedence rule for runtime requests and both interfaces. Models
    /// without effort support must not inherit a stale override or profile value.
    pub fn resolve_reasoning_effort<'a>(&'a self, selection: Option<&'a str>) -> Option<&'a str> {
        if self.provider == AiProviderKind::ClaudeCode
            && !super::claude_code::supports_effort(&self.model)
        {
            return None;
        }
        selection.or(self.reasoning_effort.as_deref()).or_else(|| {
            (self.provider == AiProviderKind::ClaudeCode)
                .then(|| super::claude_code::default_effort(&self.model))
                .flatten()
        })
    }

    /// Presentation is separate from the stable configuration key.
    pub fn display_name(&self) -> &str {
        if self.provider == AiProviderKind::ClaudeCode && self.name == "claude_code" {
            "Claude Agent"
        } else {
            &self.name
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectContextConfig {
    /// File names to search for (e.g. ".ovim.md", "AGENTS.md", "CLAUDE.md").
    pub files: Vec<String>,
    /// Walk from repo root down to file's directory, collecting all matches.
    pub hierarchical: bool,
    /// Budget in characters (rough proxy for tokens).
    pub budget: usize,
    /// Master switch.
    pub enabled: bool,
}

impl Default for ProjectContextConfig {
    fn default() -> Self {
        Self {
            files: vec![
                ".ovim.md".to_string(),
                "AGENTS.md".to_string(),
                "CLAUDE.md".to_string(),
            ],
            hierarchical: true,
            budget: 2000,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatContextConfig {
    /// Number of recent turns whose tool results are kept verbatim.
    pub observation_window: usize,
    /// Template for masked tool results. `{turn}` is replaced with the turn number.
    pub mask_template: String,
    /// Max context tokens (deferred — not enforced yet).
    pub max_context_tokens: usize,
}

impl Default for ChatContextConfig {
    fn default() -> Self {
        Self {
            observation_window: 10,
            mask_template: "[output from turn {turn}]".to_string(),
            max_context_tokens: 100_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AiConfig {
    pub default_profile: String,
    pub profiles: HashMap<String, AiProfileConfig>,
    /// Maps context names (such as "chat" and "query") to profile names.
    pub contexts: HashMap<String, String>,
    /// Named API key sources (env var or file).
    pub api_key_registry: HashMap<String, ApiKeyConfig>,
    /// Global prompt templates (e.g. "edit", "chat") with `{{variable}}` interpolation.
    pub prompts: HashMap<String, String>,
    /// Format-specific system prompts registered via `vim.ai.formats.register()`.
    pub format_prompts: HashMap<String, String>,
    /// Project context file loading configuration.
    pub project_context: ProjectContextConfig,
    /// Chat context management (observation masking).
    pub chat_context: ChatContextConfig,
    /// Tool-approval behavior for AI chat tool calls.
    pub tool_approval_mode: ToolApprovalMode,
}

#[derive(Debug, Deserialize)]
struct AiTomlConfig {
    default_profile: Option<String>,
    tool_approval_mode: Option<String>,
    #[serde(default)]
    profiles: HashMap<String, AiTomlProfile>,
    /// Parsed only to produce a clear migration error. Delegated-agent policy
    /// is owned by Ovim's harness, not legacy user configuration.
    #[serde(rename = "subagents")]
    obsolete_subagents: Option<toml::Value>,
}

#[derive(Debug, Deserialize)]
struct AiTomlProfile {
    provider: AiProviderKind,
    model: String,
    base_url: Option<String>,
    api_key: Option<String>,
    api_key_env: Option<String>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    system_prompt: Option<String>,
    edit_format: Option<String>,
    chat_edit_format: Option<String>,
    surrounding_lines: Option<u16>,
    symbols: Option<u16>,
    diagnostics: Option<String>,
    related_slices: Option<bool>,
    context_budget: Option<usize>,
    max_tool_calls: Option<u64>,
    tools: Option<Vec<String>>,
    scope: Option<String>,
    scope_shell: Option<bool>,
    scope_network: Option<bool>,
    reasoning_effort: Option<String>,
    verbosity: Option<String>,
    syntax_check: Option<bool>,
    retry_max: Option<u8>,
    retry_fallback: Option<String>,
    edit_prompt: Option<String>,
    chat_prompt: Option<String>,
    chat_edit_prompt: Option<String>,
}

impl Default for AiConfig {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert(
            PROFILE_LOCAL.to_string(),
            AiProfileConfig {
                name: PROFILE_LOCAL.to_string(),
                provider: AiProviderKind::Ollama,
                model: "qwen2.5-coder:7b".to_string(),
                base_url: Some("http://127.0.0.1:11434".to_string()),
                api_key: None,
                api_key_env: None,
                temperature: Some(0.2),
                max_tokens: Some(2048),
                system_prompt: Some(default_system_prompt().to_string()),
                edit_format: EditFormat::Json,
                chat_edit_format: None,
                context: ContextGatheringPolicy {
                    budget: 2_500,
                    related_slices: false,
                    symbols: 6,
                    ..ContextGatheringPolicy::default()
                },
                agent_loop: AgentLoopConfig::default(),
                tools: vec![],
                scope: ProfileScope::default(),
                edit_prompt: None,
                chat_prompt: None,
                chat_edit_prompt: None,
                reasoning_effort: None,
                verbosity: None,
                syntax_check: None,
                retry: RetryPolicy::default(),
            },
        );

        Self {
            default_profile: PROFILE_LOCAL.to_string(),
            profiles,
            contexts: HashMap::new(),
            api_key_registry: HashMap::new(),
            prompts: HashMap::new(),
            format_prompts: HashMap::new(),
            project_context: ProjectContextConfig::default(),
            chat_context: ChatContextConfig::default(),
            tool_approval_mode: ToolApprovalMode::default(),
        }
    }
}

impl AiConfig {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read AI config: {}", path.display()))?;
        let parsed: AiTomlConfig = toml::from_str(&content)
            .with_context(|| format!("failed to parse AI config: {}", path.display()))?;
        reject_obsolete_subagent_config(&parsed)?;

        let mut cfg = Self::default();
        for (name, profile) in parsed.profiles {
            let edit_format =
                parse_edit_format_str(profile.edit_format.as_deref().unwrap_or("json"));
            let chat_edit_format = profile
                .chat_edit_format
                .as_deref()
                .map(parse_edit_format_str);

            let diagnostics = match profile.diagnostics.as_deref() {
                Some("file") => DiagnosticScope::File,
                _ => DiagnosticScope::Overlapping,
            };

            let context = ContextGatheringPolicy {
                surrounding_lines: profile.surrounding_lines.unwrap_or(6),
                symbols: profile.symbols.unwrap_or(12),
                diagnostics,
                related_slices: profile.related_slices.unwrap_or(true),
                budget: profile.context_budget.unwrap_or(8_000),
            };

            let agent_loop = AgentLoopConfig {
                // Treat the legacy value 0 as unlimited as well as omission.
                max_tool_calls: profile.max_tool_calls.filter(|limit| *limit > 0),
            };

            let retry = RetryPolicy {
                max: profile.retry_max.unwrap_or(0),
                fallback: profile.retry_fallback,
            };
            let file_scope = profile
                .scope
                .as_deref()
                .and_then(parse_file_scope_str)
                .unwrap_or(FileScope::File);
            let scope = ProfileScope {
                files: file_scope,
                shell: profile.scope_shell.unwrap_or(false),
                network: profile.scope_network.unwrap_or(false),
            };
            let tools = profile.tools.unwrap_or_default();

            // Eagerly resolve api_key_env: if the user didn't set one in TOML,
            // fill in the provider default (e.g. "ANTHROPIC_API_KEY").
            let api_key_env = profile
                .api_key_env
                .or_else(|| default_api_key_env(profile.provider));

            cfg.profiles.insert(
                name.clone(),
                AiProfileConfig {
                    name,
                    provider: profile.provider,
                    model: profile.model,
                    base_url: profile.base_url,
                    api_key: profile.api_key,
                    api_key_env,
                    temperature: profile.temperature,
                    max_tokens: profile.max_tokens,
                    system_prompt: profile.system_prompt,
                    edit_format,
                    chat_edit_format,
                    context,
                    agent_loop,
                    tools,
                    scope,
                    edit_prompt: profile.edit_prompt,
                    chat_prompt: profile.chat_prompt,
                    chat_edit_prompt: profile.chat_edit_prompt,
                    reasoning_effort: profile.reasoning_effort,
                    verbosity: profile.verbosity,
                    syntax_check: profile.syntax_check,
                    retry,
                },
            );
        }

        if let Some(default_profile) = parsed.default_profile {
            cfg.default_profile = default_profile;
        }
        if let Some(mode) = parsed.tool_approval_mode.as_deref() {
            cfg.tool_approval_mode = parse_tool_approval_mode(mode);
        }
        if !cfg.profiles.contains_key(&cfg.default_profile) {
            cfg.default_profile = PROFILE_LOCAL.to_string();
        }

        Ok(cfg)
    }

    pub fn resolve_profile(&self, name: &str) -> Option<&AiProfileConfig> {
        self.profiles.get(name)
    }

    /// Names of the environment variables a model-run shell must not inherit:
    /// the provider credentials ovim and the provider CLIs it drives read, plus
    /// every profile's configured `api_key_env`.
    pub fn shell_scrubbed_env_names(&self) -> Vec<String> {
        let mut names: Vec<String> = PROVIDER_CREDENTIAL_ENV_VARS
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        for profile in self.profiles.values() {
            if let Some(name) = profile
                .api_key_env
                .as_deref()
                .filter(|name| !name.is_empty())
            {
                if !names.iter().any(|known| known == name) {
                    names.push(name.to_string());
                }
            }
        }
        names
    }
}

/// Provider credential environment variables. A prompt-injected shell program
/// (`env`, `curl -d "$OPENAI_API_KEY" ...`) could otherwise exfiltrate them, so
/// the agent's shell never inherits them.
pub const PROVIDER_CREDENTIAL_ENV_VARS: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OVIM_OPENAI_API_KEY",
    "OVIM_ANTHROPIC_API_KEY",
    "EXA_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CODEX_API_KEY",
];

fn reject_obsolete_subagent_config(parsed: &AiTomlConfig) -> Result<()> {
    if parsed.obsolete_subagents.is_some() {
        anyhow::bail!(
            "[subagents] is no longer configurable in ai.toml; remove the section because Ovim's safe delegated-agent harness is enabled automatically"
        );
    }
    Ok(())
}

/// Parse a string into an EditFormat enum.
pub fn parse_edit_format_str(s: &str) -> EditFormat {
    match s {
        "codeblock" => EditFormat::Codeblock,
        "json" => EditFormat::Json,
        "raw" => EditFormat::Raw,
        "apply_patch" => EditFormat::ApplyPatch,
        "str_replace" => EditFormat::StrReplace,
        other => {
            if let Some(name) = other.strip_prefix("lua:") {
                EditFormat::Lua(name.to_string())
            } else {
                EditFormat::Lua(other.to_string())
            }
        }
    }
}

fn parse_file_scope_str(s: &str) -> Option<FileScope> {
    match s.to_ascii_lowercase().as_str() {
        "selection" => Some(FileScope::Selection),
        "file" => Some(FileScope::File),
        "project" => Some(FileScope::Project),
        "any" => Some(FileScope::Any),
        _ => None,
    }
}

fn parse_tool_approval_mode(s: &str) -> ToolApprovalMode {
    match s.to_ascii_lowercase().as_str() {
        "auto" => ToolApprovalMode::Auto,
        "always_prompt" => ToolApprovalMode::AlwaysPrompt,
        _ => ToolApprovalMode::SensitivePrompt,
    }
}

/// Infer provider from model name prefix.
pub fn infer_provider(model: &str) -> AiProviderKind {
    infer_provider_with_openai_key(model, false)
}

/// Like [`infer_provider`], for a profile that names no provider. Codex-family
/// model names mean the Codex device login unless the profile has an OpenAI API
/// key (configured on the profile or present in the environment): configs
/// written before the Codex route existed relied on that key, and silently
/// switching them to a different login would be worse than keeping the API.
pub fn infer_provider_with_openai_key(model: &str, openai_key_configured: bool) -> AiProviderKind {
    let m = model.to_lowercase();
    if matches!(
        m.as_str(),
        "gpt-6-astra" | "gpt-6-sol" | "gpt-6.1-sol" | "gpt-6-luna"
    ) || m.starts_with("gpt-5.6-")
        || m.contains("codex")
    {
        if openai_key_configured {
            AiProviderKind::OpenAi
        } else {
            AiProviderKind::Codex
        }
    } else if m.starts_with("claude") {
        AiProviderKind::Anthropic
    } else if m.starts_with("gpt-")
        || m.starts_with("o1-")
        || m.starts_with("o3-")
        || m.starts_with("o4-")
    {
        AiProviderKind::OpenAi
    } else {
        AiProviderKind::Ollama
    }
}

/// Default API key environment variable for a given provider.
pub fn default_api_key_env(provider: AiProviderKind) -> Option<String> {
    match provider {
        AiProviderKind::ClaudeCode | AiProviderKind::Codex | AiProviderKind::CodexAppServer => None,
        AiProviderKind::OpenAi => Some("OPENAI_API_KEY".to_string()),
        AiProviderKind::Anthropic => Some("ANTHROPIC_API_KEY".to_string()),
        AiProviderKind::Ollama => None,
    }
}

/// Parse a provider string (e.g. from Lua) into AiProviderKind.
pub fn parse_provider_str(s: &str) -> Option<AiProviderKind> {
    match s.to_lowercase().as_str() {
        "claude_code" => Some(AiProviderKind::ClaudeCode),
        "codex" => Some(AiProviderKind::Codex),
        "codex_app_server" => Some(AiProviderKind::CodexAppServer),
        "openai" => Some(AiProviderKind::OpenAi),
        "anthropic" => Some(AiProviderKind::Anthropic),
        "ollama" => Some(AiProviderKind::Ollama),
        _ => None,
    }
}

fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("ovim").join("ai.toml")
}

fn default_system_prompt() -> &'static str {
    "You are an editing agent. Return JSON: {\"replacement\": string, \"new_import_statements\": string[], \"log\": string[]}. Only include valid JSON."
}

/// Returns a system prompt appropriate for the given edit format.
/// Used as a fallback when a profile has no explicit system prompt.
pub fn system_prompt_for_edit_format(format: &EditFormat) -> &'static str {
    match format {
        EditFormat::Json => {
            "You are a code editing assistant. Return your response as JSON with the schema: {\"replacement\": string, \"new_import_statements\": string[], \"log\": string[]}. Only output valid JSON, no explanation."
        }
        EditFormat::Codeblock => {
            "You are a code editing assistant. Return ONLY the replacement code inside a single fenced code block (```). Do not include any explanation outside the code block."
        }
        EditFormat::Raw => {
            "You are a code editing assistant. Return ONLY the replacement code with no explanation, no markdown, no code fences."
        }
        EditFormat::ApplyPatch | EditFormat::StrReplace | EditFormat::Lua(_) => {
            "You are a code editing assistant. Return ONLY the replacement code with no explanation, no markdown, no code fences."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_the_default_tool_approval_mode() {
        assert_eq!(ToolApprovalMode::default(), ToolApprovalMode::Auto);
        assert_eq!(
            AiConfig::default().tool_approval_mode,
            ToolApprovalMode::Auto
        );
    }

    #[test]
    fn agent_turns_are_unlimited_by_default() {
        assert_eq!(AgentLoopConfig::default().max_tool_calls, None);
        assert_eq!(
            AiConfig::default()
                .resolve_profile(PROFILE_LOCAL)
                .unwrap()
                .agent_loop
                .max_tool_calls,
            None
        );
    }

    #[test]
    fn profile_scope_default_is_file() {
        assert_eq!(ProfileScope::default().files, FileScope::File);
    }

    #[test]
    fn parse_file_scope_values() {
        assert_eq!(
            parse_file_scope_str("selection"),
            Some(FileScope::Selection)
        );
        assert_eq!(parse_file_scope_str("file"), Some(FileScope::File));
        assert_eq!(parse_file_scope_str("project"), Some(FileScope::Project));
        assert_eq!(parse_file_scope_str("any"), Some(FileScope::Any));
        assert_eq!(parse_file_scope_str("unknown"), None);
    }

    #[test]
    fn parse_tool_approval_mode_values() {
        assert_eq!(parse_tool_approval_mode("auto"), ToolApprovalMode::Auto);
        assert_eq!(
            parse_tool_approval_mode("always_prompt"),
            ToolApprovalMode::AlwaysPrompt
        );
        assert_eq!(
            parse_tool_approval_mode("sensitive_prompt"),
            ToolApprovalMode::SensitivePrompt
        );
        // Unknown values fail closed to sensitive_prompt.
        assert_eq!(
            parse_tool_approval_mode("something_else"),
            ToolApprovalMode::SensitivePrompt
        );
    }

    #[test]
    fn legacy_toml_cannot_configure_delegated_agent_policy() {
        let parsed: AiTomlConfig = toml::from_str("[subagents]\nenabled = false\n").unwrap();
        assert!(reject_obsolete_subagent_config(&parsed).is_err());
    }

    #[test]
    fn codex_provider_is_parseable_and_needs_no_api_key() {
        assert_eq!(parse_provider_str("codex"), Some(AiProviderKind::Codex));
        assert_eq!(
            parse_provider_str("codex_app_server"),
            Some(AiProviderKind::CodexAppServer)
        );
        assert_eq!(infer_provider("gpt-6-astra"), AiProviderKind::Codex);
        assert_eq!(infer_provider("GPT-6-ASTRA"), AiProviderKind::Codex);
        assert_eq!(infer_provider("gpt-6-sol"), AiProviderKind::Codex);
        assert_eq!(infer_provider("gpt-6.1-sol"), AiProviderKind::Codex);
        assert_eq!(infer_provider("gpt-6-luna"), AiProviderKind::Codex);
        assert_eq!(infer_provider("gpt-5.6-sol"), AiProviderKind::Codex);
        assert_eq!(infer_provider("gpt-5.6-terra"), AiProviderKind::Codex);
        assert_eq!(default_api_key_env(AiProviderKind::Codex), None);
    }

    #[test]
    fn an_openai_key_keeps_older_configs_on_the_api_instead_of_codex_login() {
        for model in ["gpt-6-sol", "gpt-6-luna", "gpt-6.1-sol", "gpt-5.6-terra"] {
            assert_eq!(
                infer_provider_with_openai_key(model, false),
                AiProviderKind::Codex,
                "{model}"
            );
            assert_eq!(
                infer_provider_with_openai_key(model, true),
                AiProviderKind::OpenAi,
                "{model}"
            );
        }
        // The key only matters where the answer would otherwise be Codex.
        assert_eq!(
            infer_provider_with_openai_key("claude-sonnet-5-5", true),
            AiProviderKind::Anthropic
        );
        assert_eq!(
            infer_provider_with_openai_key("qwen2.5-coder:7b", true),
            AiProviderKind::Ollama
        );
        assert_eq!(
            infer_provider_with_openai_key("gpt-4o", false),
            AiProviderKind::OpenAi
        );
        assert_eq!(infer_provider("gpt-6-sol"), AiProviderKind::Codex);
    }
}
