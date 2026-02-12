use crate::ai::types::{AiProviderKind, ExtractionStrategy, PROFILE_LOCAL};
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
}

#[derive(Debug, Clone)]
pub struct AiConfig {
    pub default_profile: String,
    pub profiles: HashMap<String, AiProfileConfig>,
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
            },
        );

        Self {
            default_profile: PROFILE_LOCAL.to_string(),
            profiles,
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

fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("ovim").join("ai.toml")
}

fn default_system_prompt() -> &'static str {
    "You are an editing agent. Return JSON: {\"replacement\": string, \"top_insertions\": string[], \"log\": string[]}. Only include valid JSON."
}

