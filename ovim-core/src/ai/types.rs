use serde::{Deserialize, Serialize};

pub const PROFILE_LOCAL: &str = "local";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileScope {
    /// Only the current selection.
    Selection,
    /// Current file only.
    #[default]
    File,
    /// All files in the project.
    Project,
    /// Unrestricted file access.
    Any,
}

impl FileScope {
    fn ordinal(self) -> u8 {
        match self {
            Self::Selection => 0,
            Self::File => 1,
            Self::Project => 2,
            Self::Any => 3,
        }
    }
}

impl PartialOrd for FileScope {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FileScope {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ordinal().cmp(&other.ordinal())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileScope {
    pub files: FileScope,
    pub shell: bool,
    pub network: bool,
}

impl Default for ProfileScope {
    fn default() -> Self {
        Self {
            files: FileScope::File,
            shell: false,
            network: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditMode {
    #[default]
    Format,
    Tools,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityTier {
    Small,
    Mid,
    Frontier,
}

impl Default for CapabilityTier {
    fn default() -> Self {
        Self::Mid
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    FastPath,
    Hybrid,
    ReactOnly,
}

impl Default for AgentMode {
    fn default() -> Self {
        Self::Hybrid
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContextPolicy {
    pub tier: CapabilityTier,
    pub mode: AgentMode,
    pub context_budget_tokens: usize,
    pub max_tool_calls: u16,
    pub max_iterations: u8,
    pub retrieval_k: u16,
    pub callgraph_hops: u8,
    pub enable_pruning: bool,
}

impl ContextPolicy {
    pub fn for_tier(tier: CapabilityTier) -> Self {
        match tier {
            CapabilityTier::Small => Self {
                tier,
                mode: AgentMode::FastPath,
                context_budget_tokens: 2_500,
                max_tool_calls: 10,
                max_iterations: 2,
                retrieval_k: 6,
                callgraph_hops: 1,
                enable_pruning: true,
            },
            CapabilityTier::Mid => Self {
                tier,
                mode: AgentMode::Hybrid,
                context_budget_tokens: 8_000,
                max_tool_calls: 20,
                max_iterations: 4,
                retrieval_k: 12,
                callgraph_hops: 2,
                enable_pruning: true,
            },
            CapabilityTier::Frontier => Self {
                tier,
                mode: AgentMode::Hybrid,
                context_budget_tokens: 24_000,
                max_tool_calls: 36,
                max_iterations: 6,
                retrieval_k: 20,
                callgraph_hops: 3,
                enable_pruning: true,
            },
        }
    }
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self::for_tier(CapabilityTier::default())
    }
}

#[derive(Debug, Clone)]
pub struct CodeSlice {
    pub label: String,
    pub path: Option<String>,
    pub language: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct SymbolFact {
    pub name: String,
    pub kind: String,
    pub line: u32,
    pub character: u32,
    pub path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiagnosticFact {
    pub message: String,
    pub severity: Option<String>,
    pub line: u32,
    pub start_character: u32,
    pub end_character: u32,
}

#[derive(Debug, Clone, Default)]
pub struct AiContextPack {
    pub selection: String,
    pub surrounding: Vec<CodeSlice>,
    pub symbol_facts: Vec<SymbolFact>,
    pub diagnostics: Vec<DiagnosticFact>,
    pub related_slices: Vec<CodeSlice>,
}

#[derive(Debug, Clone)]
pub struct AiRequest {
    pub prompt: String,
    pub selected_text: String,
    pub language_id: Option<String>,
    pub file_path: Option<String>,
    pub extraction: ExtractionStrategy,
    pub context_pack: Option<AiContextPack>,
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
    /// Whether edits overlapping this range should be blocked.
    /// Running AI jobs use `true`; generated-range tracking uses `false`.
    pub blocks_edits: bool,
}
