use crate::ai::{AiConfig, AiJobResult, ExtractionStrategy, PROFILE_LOCAL};
use crate::mode::Mode;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiJobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl std::fmt::Display for AiJobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        };
        write!(f, "{value}")
    }
}

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

#[derive(Debug, Clone)]
pub struct AiLogBlock {
    pub lock_id: u64,
    pub anchor_line: usize,
    pub status: AiJobStatus,
    pub provider_label: String,
    pub lines: Vec<String>,
    pub updated_at: Instant,
}

pub struct PendingAiJob {
    pub job_id: u64,
    pub lock_id: u64,
    pub selection: AiSelectionSnapshot,
    pub submitted_at: Instant,
    pub task: tokio::task::JoinHandle<anyhow::Result<AiJobResult>>,
    pub receiver: tokio::sync::oneshot::Receiver<anyhow::Result<AiJobResult>>,
}

pub struct AiState {
    pub config: AiConfig,
    pub prompt: AiPromptState,
    pub active_selection: Option<AiSelectionSnapshot>,
    pub pending_jobs: Vec<PendingAiJob>,
    pub logs: Vec<AiLogBlock>,
    pub active_profile: String,
    pub extraction: ExtractionStrategy,
    pub next_lock_id: u64,
    pub next_job_id: u64,
}

impl Default for AiState {
    fn default() -> Self {
        let config = AiConfig::load().unwrap_or_else(|_| AiConfig::default());
        let default_profile = if config.profiles.contains_key(&config.default_profile) {
            config.default_profile.clone()
        } else {
            PROFILE_LOCAL.to_string()
        };
        let extraction = config
            .resolve_profile(&default_profile)
            .map(|profile| profile.extraction)
            .unwrap_or_default();

        Self {
            config,
            prompt: AiPromptState::default(),
            active_selection: None,
            pending_jobs: Vec::new(),
            logs: Vec::new(),
            active_profile: default_profile,
            extraction,
            next_lock_id: 1,
            next_job_id: 1,
        }
    }
}

