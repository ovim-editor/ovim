use crate::ai::types::{
    AgentLoopConfig, AiProviderKind, ApiKeyConfig, ContextGatheringPolicy, DiagnosticScope,
    EditFormat, ProfileScope, RetryPolicy, PROFILE_LOCAL,
};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

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

#[derive(Debug, Clone)]
pub struct AiConfig {
    pub default_profile: String,
    pub profiles: HashMap<String, AiProfileConfig>,
    /// Maps context names ("chat", "selection", "query") to profile names.
    pub contexts: HashMap<String, String>,
    /// Named API key sources (env var or file).
    pub api_key_registry: HashMap<String, ApiKeyConfig>,
    /// Global prompt templates (e.g. "edit", "chat") with `{{variable}}` interpolation.
    pub prompts: HashMap<String, String>,
    /// Format-specific system prompts registered via `vim.ai.formats.register()`.
    pub format_prompts: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct AiTomlConfig {
    default_profile: Option<String>,
    #[serde(default)]
    profiles: HashMap<String, AiTomlProfile>,
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
    max_tool_calls: Option<u16>,
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
                max_tool_calls: profile.max_tool_calls.unwrap_or(50),
            };

            let retry = RetryPolicy {
                max: profile.retry_max.unwrap_or(0),
                fallback: profile.retry_fallback,
            };

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
                    tools: vec![],
                    scope: ProfileScope::default(),
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

        if !cfg.profiles.contains_key(&cfg.default_profile) {
            cfg.default_profile = PROFILE_LOCAL.to_string();
        }

        Ok(cfg)
    }

    pub fn resolve_profile(&self, name: &str) -> Option<&AiProfileConfig> {
        self.profiles.get(name)
    }
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

/// Infer provider from model name prefix.
pub fn infer_provider(model: &str) -> AiProviderKind {
    let m = model.to_lowercase();
    if m.starts_with("claude") {
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
        AiProviderKind::OpenAi => Some("OPENAI_API_KEY".to_string()),
        AiProviderKind::Anthropic => Some("ANTHROPIC_API_KEY".to_string()),
        AiProviderKind::Ollama => None,
    }
}

/// Parse a provider string (e.g. from Lua) into AiProviderKind.
pub fn parse_provider_str(s: &str) -> Option<AiProviderKind> {
    match s.to_lowercase().as_str() {
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
