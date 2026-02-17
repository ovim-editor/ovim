pub mod engine;
pub mod loader;
pub mod schema;
pub mod spec;
pub mod template;

use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Instant;

pub use engine::{execute_workflow, execute_workflow_with_progress};
pub use loader::{default_workflow_dir, load_workflows, load_workflows_from_dir};
pub use spec::{
    WorkflowNestedStepSpec, WorkflowOutputFormat, WorkflowOutputSpec, WorkflowSpec,
    WorkflowStepSpec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStepProgressKind {
    Started,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowProgressEvent {
    pub step_id: String,
    pub kind: WorkflowStepProgressKind,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowRunStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct WorkflowRunResult {
    pub outputs: BTreeMap<String, Value>,
    pub log_lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowRunRecord {
    pub id: u64,
    pub workflow_name: String,
    pub status: WorkflowRunStatus,
    pub started_at: Instant,
    pub finished_at: Option<Instant>,
    pub current_step: Option<String>,
    pub message: String,
    pub outputs: Option<BTreeMap<String, Value>>,
}

pub struct PendingWorkflowRun {
    pub run_id: u64,
    pub receiver: tokio::sync::oneshot::Receiver<anyhow::Result<WorkflowRunResult>>,
    pub progress_receiver: tokio::sync::mpsc::UnboundedReceiver<WorkflowProgressEvent>,
    pub task: tokio::task::JoinHandle<()>,
}
