use serde::{Deserialize, Serialize};

pub const PROFILE_LOCAL: &str = "local";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderKind {
    OpenAi,
    Anthropic,
    Ollama,
}

impl std::fmt::Display for AiProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStrategy {
    Json,
    Codeblock,
    Raw,
}

impl std::fmt::Display for ExtractionStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Json => "json",
            Self::Codeblock => "codeblock",
            Self::Raw => "raw",
        };
        write!(f, "{value}")
    }
}

impl Default for ExtractionStrategy {
    fn default() -> Self {
        Self::Json
    }
}

#[derive(Debug, Clone)]
pub struct AiRequest {
    pub prompt: String,
    pub selected_text: String,
    pub language_id: Option<String>,
    pub file_path: Option<String>,
    pub extraction: ExtractionStrategy,
}

#[derive(Debug, Clone)]
pub struct AiJobResult {
    pub replacement: String,
    pub top_insertions: Vec<String>,
    pub log_lines: Vec<String>,
    pub raw_output: String,
    pub provider: AiProviderKind,
    pub profile_name: String,
    pub model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferLock {
    pub id: u64,
    pub start_char: usize,
    pub end_char: usize,
}

