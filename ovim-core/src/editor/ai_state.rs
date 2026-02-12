use crate::ai::chat_types::ConversationTree;
use crate::ai::{AiConfig, AiJobResult, ExtractionStrategy, PROFILE_LOCAL};
use crate::mode::Mode;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct AiPromptState {
    pub input: String,
    pub cursor: usize,
}

impl Default for AiPromptState {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AiSelectionSnapshot {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub anchor_line: usize,
    pub selected_text: String,
    pub mode_before_prompt: Mode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiRegionStatus {
    Running,
    Generated,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct AiEditRegion {
    pub id: u64,
    pub start_char: usize,
    pub end_char: usize,
    pub status: AiRegionStatus,
    pub prompt: String,
    pub original_text: String,
    pub generated_text: String,
    pub profile_name: String,
    pub provider_label: String,
    pub extraction: ExtractionStrategy,
    pub reasoning_lines: Vec<String>,
    pub raw_output: Option<String>,
    pub created_at: Instant,
    pub updated_at: Instant,
}

pub struct PendingAiJob {
    pub job_id: u64,
    pub lock_id: u64,
    pub selection: AiSelectionSnapshot,
    pub submitted_at: Instant,
    pub task: tokio::task::JoinHandle<anyhow::Result<AiJobResult>>,
    pub receiver: tokio::sync::oneshot::Receiver<anyhow::Result<AiJobResult>>,
    pub completed_result: Option<anyhow::Result<AiJobResult>>,
}

pub struct AiState {
    pub config: AiConfig,
    pub prompt: AiPromptState,
    pub active_selection: Option<AiSelectionSnapshot>,
    pub pending_jobs: Vec<PendingAiJob>,
    pub regions: Vec<AiEditRegion>,
    pub selected_region_id: Option<u64>,
    pub selection_hold_until_exit: bool,
    pub active_profile: String,
    pub extraction: ExtractionStrategy,
    pub next_lock_id: u64,
    pub next_job_id: u64,
    pub last_observed_buffer_version: usize,
    /// Active chat session state (None when chat is closed).
    pub chat: Option<super::ai_chat_state::AiChatState>,
    /// Persistent conversations keyed by (buffer_id, conversation_name).
    pub conversations: HashMap<(usize, String), ConversationTree>,
}

impl Default for AiState {
    fn default() -> Self {
        let mut config = AiConfig::load().unwrap_or_else(|_| AiConfig::default());
        let default_profile = if config.profiles.contains_key(&config.default_profile) {
            config.default_profile.clone()
        } else {
            PROFILE_LOCAL.to_string()
        };
        let extraction = config
            .resolve_profile(&default_profile)
            .map(|profile| profile.extraction)
            .unwrap_or_default();

        // Initialize default contexts if empty
        if config.contexts.is_empty() {
            for ctx in &["selection", "chat", "query"] {
                config
                    .contexts
                    .insert(ctx.to_string(), default_profile.clone());
            }
        }

        Self {
            config,
            prompt: AiPromptState::default(),
            active_selection: None,
            pending_jobs: Vec::new(),
            regions: Vec::new(),
            selected_region_id: None,
            selection_hold_until_exit: false,
            active_profile: default_profile,
            extraction,
            next_lock_id: 1,
            next_job_id: 1,
            last_observed_buffer_version: 0,
            chat: None,
            conversations: HashMap::new(),
        }
    }
}
