use crate::ai::types::{
    AgentMode, AiProviderKind, CapabilityTier, ContextPolicy, EditMode, ExtractionStrategy,
    ProfileScope, PROFILE_LOCAL,
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
    pub api_key_env: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub system_prompt: Option<String>,
    pub extraction: ExtractionStrategy,
    pub context_policy: ContextPolicy,
    pub tools: Vec<String>,
    pub scope: ProfileScope,
    pub edit_mode: EditMode,
    pub edit_format: String,
}

#[derive(Debug, Clone)]
pub struct AiConfig {
    pub default_profile: String,
    pub profiles: HashMap<String, AiProfileConfig>,
    /// Maps context names ("chat", "selection", "query") to profile names.
    pub contexts: HashMap<String, String>,
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
    api_key_env: Option<String>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    system_prompt: Option<String>,
    extraction: Option<ExtractionStrategy>,
    capability_tier: Option<CapabilityTier>,
    agent_mode: Option<AgentMode>,
    context_budget_tokens: Option<usize>,
    max_tool_calls: Option<u16>,
    max_iterations: Option<u8>,
    retrieval_k: Option<u16>,
    callgraph_hops: Option<u8>,
    enable_pruning: Option<bool>,
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
                api_key_env: None,
                temperature: Some(0.2),
                max_tokens: Some(2048),
                system_prompt: Some(default_system_prompt().to_string()),
                extraction: ExtractionStrategy::Json,
                context_policy: ContextPolicy::for_tier(CapabilityTier::Small),
                tools: vec![],
                scope: ProfileScope::default(),
                edit_mode: EditMode::Format,
                edit_format: "codeblock".to_string(),
            },
        );

        Self {
            default_profile: PROFILE_LOCAL.to_string(),
            profiles,
            contexts: HashMap::new(),
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
            let mut context_policy =
                ContextPolicy::for_tier(profile.capability_tier.unwrap_or_default());
            if let Some(mode) = profile.agent_mode {
                context_policy.mode = mode;
            }
            if let Some(value) = profile.context_budget_tokens {
                context_policy.context_budget_tokens = value;
            }
            if let Some(value) = profile.max_tool_calls {
                context_policy.max_tool_calls = value;
            }
            if let Some(value) = profile.max_iterations {
                context_policy.max_iterations = value;
            }
            if let Some(value) = profile.retrieval_k {
                context_policy.retrieval_k = value;
            }
            if let Some(value) = profile.callgraph_hops {
                context_policy.callgraph_hops = value;
            }
            if let Some(value) = profile.enable_pruning {
                context_policy.enable_pruning = value;
            }

            cfg.profiles.insert(
                name.clone(),
                AiProfileConfig {
                    name,
                    provider: profile.provider,
                    model: profile.model,
                    base_url: profile.base_url,
                    api_key_env: profile.api_key_env,
                    temperature: profile.temperature,
                    max_tokens: profile.max_tokens,
                    system_prompt: profile.system_prompt,
                    extraction: profile.extraction.unwrap_or(ExtractionStrategy::Json),
                    context_policy,
                    tools: vec![],
                    scope: ProfileScope::default(),
                    edit_mode: EditMode::Format,
                    edit_format: "codeblock".to_string(),
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
    "You are an editing agent. Return JSON: {\"replacement\": string, \"top_insertions\": string[], \"log\": string[]}. Only include valid JSON."
}
