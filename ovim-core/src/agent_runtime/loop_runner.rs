//! Provider-independent execution loop for one delegated agent.
//!
//! The runner knows nothing about editor presentation or root-chat state. Its
//! complete authority is injected: a delegation envelope, resolved route,
//! scoped tools, approval client, workspace descriptor, durable event sink,
//! lifecycle hooks, budgets, and cancellation.

use super::{
    AgentWorkspaceWarning, DispatchHandle, DispatchState, HandoffConfidence, HandoffStatus,
    HandoffValidationError, HandoffValidator, ResolvedModelRoute, StructuredHandoffV1,
    ValidatedHandoff, WorkspaceAssignment,
};
use crate::ai::tools::StrictJsonSchema;
use crate::run_log::{
    AgentProviderEvent as RecordedAgentProviderEvent, AgentProviderState, EventEnvelope, EventKind,
    OperationId, ToolIntentEvent, ToolOutcome, ToolResultEvent, ToolSideEffect, ToolStartedEvent,
    AGENT_PROVIDER_EVENT_VERSION,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;

pub type AgentFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelegationEnvelope {
    pub version: u32,
    pub objective: String,
    pub done_when: Vec<String>,
    pub non_goals: Vec<String>,
    pub relevant_paths: Vec<String>,
    pub parent_brief: Option<String>,
    pub workspace_warnings: Vec<AgentWorkspaceWarning>,
}

impl DelegationEnvelope {
    pub fn objective(objective: impl Into<String>) -> Self {
        Self {
            version: 1,
            objective: objective.into(),
            done_when: Vec::new(),
            non_goals: Vec::new(),
            relevant_paths: Vec::new(),
            parent_brief: None,
            workspace_warnings: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentWorkspaceDescriptor {
    pub assignment: WorkspaceAssignment,
    /// Present only when the selected workspace strategy has a materialized
    /// filesystem root. A virtual read-only snapshot may intentionally omit it.
    pub root: Option<PathBuf>,
    pub read_only: bool,
    pub warnings: Vec<AgentWorkspaceWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderBinding {
    pub provider: String,
    pub profile: String,
    pub model: String,
    pub reasoning_effort: String,
    pub session_id: String,
}

#[derive(Clone, Debug)]
pub struct AgentProviderStart {
    pub handle: DispatchHandle,
    pub envelope: DelegationEnvelope,
    pub route: ResolvedModelRoute,
    pub workspace: AgentWorkspaceDescriptor,
    /// The complete, already capability-scoped provider tool contract.
    /// Adapters must not consult the root tool registry or profile tool list.
    pub scoped_tools: Vec<ScopedTool>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentProviderEvent {
    CallStarted {
        provider_call_id: String,
    },
    ToolRequest {
        provider_call_id: Option<String>,
        tool_call_id: String,
        tool_name: String,
        arguments: Value,
    },
    ToolObservedStarted {
        provider_call_id: Option<String>,
        tool_call_id: String,
        tool_name: String,
    },
    ToolObservedFailed {
        provider_call_id: Option<String>,
        tool_call_id: String,
        tool_name: String,
        error: String,
    },
    Handoff {
        payload: Vec<u8>,
    },
    ProviderFailed {
        error: String,
    },
    Cancelled {
        reason: String,
    },
    TimedOut {
        detail: String,
    },
    Checkpoint {
        label: String,
    },
}

pub trait AgentProviderSession: Send {
    fn binding(&self) -> &ProviderBinding;

    fn next_event(&mut self) -> AgentFuture<'_, Result<AgentProviderEvent, AgentProviderError>>;

    fn submit_tool_result(
        &mut self,
        _tool_call_id: &str,
        _result: &AgentToolResult,
    ) -> AgentFuture<'_, Result<(), AgentProviderError>> {
        Box::pin(async { Ok(()) })
    }
}

pub trait AgentProviderAdapter: Send + Sync {
    fn start(
        &self,
        request: AgentProviderStart,
    ) -> AgentFuture<'_, Result<Box<dyn AgentProviderSession>, AgentProviderError>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentProviderError {
    pub detail: String,
}

impl AgentProviderError {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for AgentProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for AgentProviderError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedTool {
    pub name: String,
    pub description: String,
    pub input_schema: StrictJsonSchema,
    pub side_effect: ToolSideEffect,
    pub requires_approval: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScopedToolView {
    tools: BTreeMap<String, ScopedTool>,
}

impl ScopedToolView {
    pub fn new(tools: impl IntoIterator<Item = ScopedTool>) -> Result<Self, AgentLoopError> {
        let mut by_name = BTreeMap::new();
        for tool in tools {
            if tool.name.trim().is_empty() {
                return Err(AgentLoopError::InvalidInput("tool name is empty".into()));
            }
            if by_name.insert(tool.name.clone(), tool).is_some() {
                return Err(AgentLoopError::InvalidInput(
                    "tool view repeats a tool name".into(),
                ));
            }
        }
        Ok(Self { tools: by_name })
    }

    pub fn get(&self, name: &str) -> Option<&ScopedTool> {
        self.tools.get(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    pub fn tools(&self) -> Vec<ScopedTool> {
        self.tools.values().cloned().collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentToolCall {
    pub handle: DispatchHandle,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub workspace: AgentWorkspaceDescriptor,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentToolResult {
    pub outcome: ToolOutcome,
    pub summary: Option<String>,
    pub result: Option<Value>,
}

impl AgentToolResult {
    pub fn completed(result: Option<Value>) -> Self {
        Self {
            outcome: ToolOutcome::Completed,
            summary: None,
            result,
        }
    }

    pub fn failed(summary: impl Into<String>) -> Self {
        Self {
            outcome: ToolOutcome::Failed,
            summary: Some(summary.into()),
            result: None,
        }
    }
}

pub trait AgentToolExecutor: Send + Sync {
    fn execute(
        &self,
        call: AgentToolCall,
    ) -> AgentFuture<'_, Result<AgentToolResult, AgentToolError>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentToolError {
    pub detail: String,
}

impl AgentToolError {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for AgentToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for AgentToolError {}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentApprovalRequest {
    pub handle: DispatchHandle,
    pub tool_name: String,
    pub arguments: Value,
    pub workspace: AgentWorkspaceDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentApprovalDecision {
    Allowed,
    Denied { reason: String },
}

pub trait AgentApprovalClient: Send + Sync {
    fn request(
        &self,
        request: AgentApprovalRequest,
    ) -> AgentFuture<'_, Result<AgentApprovalDecision, String>>;
}

#[derive(Default)]
pub struct DenyAllAgentApprovals;

impl AgentApprovalClient for DenyAllAgentApprovals {
    fn request(
        &self,
        _request: AgentApprovalRequest,
    ) -> AgentFuture<'_, Result<AgentApprovalDecision, String>> {
        Box::pin(async {
            Ok(AgentApprovalDecision::Denied {
                reason: "no approval surface is attached to this child".into(),
            })
        })
    }
}

pub trait AgentLoopEventSink: Send + Sync {
    fn record(
        &self,
        event: AgentLoopEventRecord,
    ) -> AgentFuture<'_, Result<EventEnvelope, AgentLoopError>>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentLoopEventRecord {
    pub handle: DispatchHandle,
    pub kind: EventKind,
    pub operation_id: Option<OperationId>,
    pub provider_call_id: Option<String>,
}

pub trait AgentLoopRuntimeHooks: Send + Sync {
    fn transition(
        &self,
        handle: &DispatchHandle,
        state: DispatchState,
        detail: Option<String>,
    ) -> AgentFuture<'_, Result<EventEnvelope, AgentLoopError>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentLoopBudget {
    pub timeout: Duration,
    pub max_provider_events: usize,
    pub max_tool_calls: usize,
}

impl Default for AgentLoopBudget {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(600),
            max_provider_events: 256,
            max_tool_calls: 48,
        }
    }
}

/// Run-wide counters shared by otherwise independent child loops.
pub struct RootAgentBudget {
    max_provider_events: usize,
    max_tool_calls: usize,
    provider_events: AtomicUsize,
    tool_calls: AtomicUsize,
}

impl RootAgentBudget {
    pub fn new(max_provider_events: usize, max_tool_calls: usize) -> Self {
        Self {
            max_provider_events,
            max_tool_calls,
            provider_events: AtomicUsize::new(0),
            tool_calls: AtomicUsize::new(0),
        }
    }

    pub fn provider_events(&self) -> usize {
        self.provider_events.load(Ordering::Acquire)
    }

    pub fn tool_calls(&self) -> usize {
        self.tool_calls.load(Ordering::Acquire)
    }

    fn reserve_provider_event(&self) -> bool {
        reserve(&self.provider_events, self.max_provider_events)
    }

    fn reserve_tool_call(&self) -> bool {
        reserve(&self.tool_calls, self.max_tool_calls)
    }
}

fn reserve(counter: &AtomicUsize, maximum: usize) -> bool {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current < maximum).then_some(current + 1)
        })
        .is_ok()
}

#[derive(Clone, Debug)]
pub struct AgentCancellationToken {
    sender: Arc<watch::Sender<Option<String>>>,
}

impl Default for AgentCancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentCancellationToken {
    pub fn new() -> Self {
        let (sender, _) = watch::channel(None);
        Self {
            sender: Arc::new(sender),
        }
    }

    pub fn cancel(&self, reason: impl Into<String>) -> bool {
        let reason = reason.into();
        self.sender.send_if_modified(|current| {
            if current.is_some() {
                false
            } else {
                *current = Some(reason);
                true
            }
        })
    }

    pub fn reason(&self) -> Option<String> {
        self.sender.borrow().clone()
    }

    pub async fn cancelled(&self) -> String {
        let mut receiver = self.sender.subscribe();
        loop {
            if let Some(reason) = receiver.borrow().clone() {
                return reason;
            }
            if receiver.changed().await.is_err() {
                return "cancellation owner was dropped".into();
            }
        }
    }
}

pub struct AgentLoopInput {
    pub handle: DispatchHandle,
    pub envelope: DelegationEnvelope,
    pub route: ResolvedModelRoute,
    pub provider: Arc<dyn AgentProviderAdapter>,
    pub tool_view: ScopedToolView,
    pub tool_executor: Arc<dyn AgentToolExecutor>,
    pub event_sink: Arc<dyn AgentLoopEventSink>,
    pub runtime_hooks: Arc<dyn AgentLoopRuntimeHooks>,
    pub approval_client: Arc<dyn AgentApprovalClient>,
    pub workspace: AgentWorkspaceDescriptor,
    pub cancellation: AgentCancellationToken,
    pub budget: AgentLoopBudget,
    pub root_budget: Arc<RootAgentBudget>,
    pub handoff_validator: HandoffValidator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentLoopUsage {
    pub provider_events: usize,
    pub tool_calls: usize,
}

#[derive(Clone, Debug)]
pub struct AgentLoopResult {
    pub binding: ProviderBinding,
    pub handoff: ValidatedHandoff,
    pub usage: AgentLoopUsage,
    pub workspace_warnings: Vec<AgentWorkspaceWarning>,
}

pub struct AgentLoopRunner;

impl AgentLoopRunner {
    pub async fn run(input: AgentLoopInput) -> Result<AgentLoopResult, AgentLoopError> {
        validate_input(&input)?;
        let deadline = Instant::now() + input.budget.timeout;
        let request = AgentProviderStart {
            handle: input.handle.clone(),
            envelope: input.envelope.clone(),
            route: input.route.clone(),
            workspace: input.workspace.clone(),
            scoped_tools: input.tool_view.tools(),
        };
        let mut session = tokio::select! {
            reason = input.cancellation.cancelled() => {
                return Err(AgentLoopError::CancelledBeforeBinding(reason));
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Err(AgentLoopError::TimedOutBeforeBinding);
            }
            result = input.provider.start(request) => result.map_err(AgentLoopError::ProviderStart)?,
        };
        let binding = session.binding().clone();
        validate_binding(&binding, &input.route)?;
        record_provider_event(&input, AgentProviderState::Bound, &binding, None, None).await?;
        input
            .runtime_hooks
            .transition(
                &input.handle,
                DispatchState::Running,
                Some(format!(
                    "provider {} profile {} model {} effort {} bound",
                    binding.provider, binding.profile, binding.model, binding.reasoning_effort
                )),
            )
            .await?;

        let mut usage = AgentLoopUsage {
            provider_events: 0,
            tool_calls: 0,
        };
        let mut observed_tools = BTreeMap::new();
        let handoff = loop {
            let event = tokio::select! {
                reason = input.cancellation.cancelled() => {
                    close_observed_tools(&input, &mut observed_tools, ToolOutcome::Interrupted, &reason).await?;
                    break synthetic_handoff(HandoffStatus::Interrupted, format!("agent was cancelled: {reason}"))?;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    close_observed_tools(&input, &mut observed_tools, ToolOutcome::Interrupted, "agent loop timed out").await?;
                    break synthetic_handoff(HandoffStatus::TimedOut, "agent loop exceeded its elapsed-time budget")?;
                }
                event = session.next_event() => event,
            };
            if usage.provider_events >= input.budget.max_provider_events
                || !input.root_budget.reserve_provider_event()
            {
                close_observed_tools(
                    &input,
                    &mut observed_tools,
                    ToolOutcome::Interrupted,
                    "provider-event budget exhausted",
                )
                .await?;
                break synthetic_handoff(
                    HandoffStatus::Interrupted,
                    "provider-event budget exhausted before a handoff",
                )?;
            }
            usage.provider_events += 1;
            match event {
                Ok(AgentProviderEvent::CallStarted { provider_call_id }) => {
                    record_provider_event(
                        &input,
                        AgentProviderState::CallStarted,
                        &binding,
                        Some(provider_call_id),
                        None,
                    )
                    .await?;
                }
                Ok(AgentProviderEvent::ToolRequest {
                    provider_call_id,
                    tool_call_id,
                    tool_name,
                    arguments,
                }) => {
                    if usage.tool_calls >= input.budget.max_tool_calls
                        || !input.root_budget.reserve_tool_call()
                    {
                        let result = AgentToolResult::failed("tool-call budget exhausted");
                        session
                            .submit_tool_result(&tool_call_id, &result)
                            .await
                            .map_err(AgentLoopError::Provider)?;
                        continue;
                    }
                    usage.tool_calls += 1;
                    let result = execute_tool_request(
                        &input,
                        provider_call_id,
                        tool_call_id.clone(),
                        tool_name,
                        arguments,
                        deadline,
                    )
                    .await?;
                    session
                        .submit_tool_result(&tool_call_id, &result)
                        .await
                        .map_err(AgentLoopError::Provider)?;
                }
                Ok(AgentProviderEvent::ToolObservedStarted {
                    provider_call_id,
                    tool_call_id,
                    tool_name,
                }) => {
                    let operation_id = OperationId::new();
                    record_tool_intent(
                        &input,
                        operation_id.clone(),
                        provider_call_id.clone(),
                        &tool_name,
                        Value::Object(Default::default()),
                        ToolSideEffect::Unknown,
                    )
                    .await?;
                    input
                        .runtime_hooks
                        .transition(
                            &input.handle,
                            DispatchState::WaitingForTool,
                            Some(format!("provider observed tool {tool_name}")),
                        )
                        .await?;
                    record_tool_started(&input, operation_id.clone(), provider_call_id, tool_name)
                        .await?;
                    observed_tools.insert(tool_call_id, operation_id);
                }
                Ok(AgentProviderEvent::ToolObservedFailed {
                    provider_call_id,
                    tool_call_id,
                    tool_name: _,
                    error,
                }) => {
                    let Some(operation_id) = observed_tools.remove(&tool_call_id) else {
                        return Err(AgentLoopError::ProviderProtocol(format!(
                            "provider failed unknown tool call {tool_call_id}"
                        )));
                    };
                    record_tool_result(
                        &input,
                        operation_id,
                        provider_call_id,
                        AgentToolResult::failed(error),
                    )
                    .await?;
                    input
                        .runtime_hooks
                        .transition(&input.handle, DispatchState::Running, None)
                        .await?;
                }
                Ok(AgentProviderEvent::Handoff { payload }) => {
                    close_observed_tools(
                        &input,
                        &mut observed_tools,
                        ToolOutcome::Interrupted,
                        "provider returned a handoff with a tool still in flight",
                    )
                    .await?;
                    match input.handoff_validator.validate_json(&payload, None) {
                        Ok(handoff) => break handoff,
                        Err(error) => {
                            break synthetic_handoff(
                                HandoffStatus::Failed,
                                format!("provider returned an invalid structured handoff: {error}"),
                            )?;
                        }
                    }
                }
                Ok(AgentProviderEvent::ProviderFailed { error })
                | Err(AgentProviderError { detail: error }) => {
                    close_observed_tools(&input, &mut observed_tools, ToolOutcome::Failed, &error)
                        .await?;
                    break synthetic_handoff(
                        HandoffStatus::Failed,
                        format!("provider failed: {error}"),
                    )?;
                }
                Ok(AgentProviderEvent::Cancelled { reason }) => {
                    close_observed_tools(
                        &input,
                        &mut observed_tools,
                        ToolOutcome::Interrupted,
                        &reason,
                    )
                    .await?;
                    break synthetic_handoff(
                        HandoffStatus::Interrupted,
                        format!("provider cancelled the child: {reason}"),
                    )?;
                }
                Ok(AgentProviderEvent::TimedOut { detail }) => {
                    close_observed_tools(
                        &input,
                        &mut observed_tools,
                        ToolOutcome::Interrupted,
                        &detail,
                    )
                    .await?;
                    break synthetic_handoff(
                        HandoffStatus::TimedOut,
                        format!("provider timed out: {detail}"),
                    )?;
                }
                Ok(AgentProviderEvent::Checkpoint { label }) => {
                    record_provider_event(
                        &input,
                        AgentProviderState::Checkpoint,
                        &binding,
                        None,
                        Some(label),
                    )
                    .await?;
                }
            }
        };
        Ok(AgentLoopResult {
            binding,
            handoff,
            usage,
            workspace_warnings: input.workspace.warnings.clone(),
        })
    }
}

async fn execute_tool_request(
    input: &AgentLoopInput,
    provider_call_id: Option<String>,
    tool_call_id: String,
    tool_name: String,
    arguments: Value,
    deadline: Instant,
) -> Result<AgentToolResult, AgentLoopError> {
    let operation_id = OperationId::new();
    let descriptor = input.tool_view.get(&tool_name).cloned();
    let side_effect = descriptor
        .as_ref()
        .map_or(ToolSideEffect::Unknown, |tool| tool.side_effect.clone());
    record_tool_intent(
        input,
        operation_id.clone(),
        provider_call_id.clone(),
        &tool_name,
        arguments.clone(),
        side_effect,
    )
    .await?;
    input
        .runtime_hooks
        .transition(
            &input.handle,
            DispatchState::WaitingForTool,
            Some(format!("waiting for tool {tool_name}")),
        )
        .await?;

    let result = if let Some(descriptor) = descriptor {
        if let Err(error) = descriptor.input_schema.validate_instance(&arguments) {
            AgentToolResult::failed(format!("tool arguments failed schema validation: {error}"))
        } else {
            let decision = if descriptor.requires_approval {
                input
                    .runtime_hooks
                    .transition(
                        &input.handle,
                        DispatchState::WaitingForUser,
                        Some(format!("waiting for approval for {tool_name}")),
                    )
                    .await?;
                await_bounded(
                    &input.cancellation,
                    deadline,
                    input.approval_client.request(AgentApprovalRequest {
                        handle: input.handle.clone(),
                        tool_name: tool_name.clone(),
                        arguments: arguments.clone(),
                        workspace: input.workspace.clone(),
                    }),
                )
                .await
                .map_err(AgentLoopError::Approval)?
            } else {
                AgentApprovalDecision::Allowed
            };
            match decision {
                AgentApprovalDecision::Denied { reason } => AgentToolResult::failed(reason),
                AgentApprovalDecision::Allowed => {
                    input
                        .runtime_hooks
                        .transition(&input.handle, DispatchState::WaitingForTool, None)
                        .await?;
                    record_tool_started(
                        input,
                        operation_id.clone(),
                        provider_call_id.clone(),
                        tool_name.clone(),
                    )
                    .await?;
                    match await_bounded(
                        &input.cancellation,
                        deadline,
                        input.tool_executor.execute(AgentToolCall {
                            handle: input.handle.clone(),
                            tool_call_id,
                            tool_name,
                            arguments,
                            workspace: input.workspace.clone(),
                        }),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(error) => AgentToolResult::failed(error.detail),
                    }
                }
            }
        }
    } else {
        AgentToolResult::failed("tool is outside this agent's scoped tool view")
    };
    record_tool_result(input, operation_id, provider_call_id, result.clone()).await?;
    input
        .runtime_hooks
        .transition(&input.handle, DispatchState::Running, None)
        .await?;
    Ok(result)
}

async fn await_bounded<T, E>(
    cancellation: &AgentCancellationToken,
    deadline: Instant,
    future: AgentFuture<'_, Result<T, E>>,
) -> Result<T, AgentToolError>
where
    E: fmt::Display,
{
    tokio::select! {
        reason = cancellation.cancelled() => Err(AgentToolError::new(format!("cancelled: {reason}"))),
        _ = tokio::time::sleep_until(deadline) => Err(AgentToolError::new("agent loop timed out")),
        result = future => result.map_err(|error| AgentToolError::new(error.to_string())),
    }
}

async fn close_observed_tools(
    input: &AgentLoopInput,
    observed: &mut BTreeMap<String, OperationId>,
    outcome: ToolOutcome,
    detail: &str,
) -> Result<(), AgentLoopError> {
    let pending = std::mem::take(observed);
    for (_, operation_id) in pending {
        record_tool_result(
            input,
            operation_id,
            None,
            AgentToolResult {
                outcome: outcome.clone(),
                summary: Some(detail.into()),
                result: None,
            },
        )
        .await?;
    }
    Ok(())
}

async fn record_provider_event(
    input: &AgentLoopInput,
    state: AgentProviderState,
    binding: &ProviderBinding,
    provider_call_id: Option<String>,
    checkpoint: Option<String>,
) -> Result<EventEnvelope, AgentLoopError> {
    input
        .event_sink
        .record(AgentLoopEventRecord {
            handle: input.handle.clone(),
            kind: EventKind::AgentProvider(RecordedAgentProviderEvent {
                version: AGENT_PROVIDER_EVENT_VERSION,
                state,
                provider: binding.provider.clone(),
                profile: binding.profile.clone(),
                model: binding.model.clone(),
                reasoning_effort: binding.reasoning_effort.clone(),
                session_id: Some(binding.session_id.clone()),
                checkpoint,
            }),
            operation_id: None,
            provider_call_id,
        })
        .await
}

async fn record_tool_intent(
    input: &AgentLoopInput,
    operation_id: OperationId,
    provider_call_id: Option<String>,
    tool_name: &str,
    arguments: Value,
    side_effect: ToolSideEffect,
) -> Result<EventEnvelope, AgentLoopError> {
    input
        .event_sink
        .record(AgentLoopEventRecord {
            handle: input.handle.clone(),
            kind: EventKind::ToolIntent(ToolIntentEvent {
                tool_name: tool_name.into(),
                arguments,
                side_effect,
            }),
            operation_id: Some(operation_id),
            provider_call_id,
        })
        .await
}

async fn record_tool_started(
    input: &AgentLoopInput,
    operation_id: OperationId,
    provider_call_id: Option<String>,
    tool_name: String,
) -> Result<EventEnvelope, AgentLoopError> {
    input
        .event_sink
        .record(AgentLoopEventRecord {
            handle: input.handle.clone(),
            kind: EventKind::ToolStarted(ToolStartedEvent { tool_name }),
            operation_id: Some(operation_id),
            provider_call_id,
        })
        .await
}

async fn record_tool_result(
    input: &AgentLoopInput,
    operation_id: OperationId,
    provider_call_id: Option<String>,
    result: AgentToolResult,
) -> Result<EventEnvelope, AgentLoopError> {
    input
        .event_sink
        .record(AgentLoopEventRecord {
            handle: input.handle.clone(),
            kind: EventKind::ToolResult(ToolResultEvent {
                outcome: result.outcome,
                summary: result.summary,
                result: result.result,
            }),
            operation_id: Some(operation_id),
            provider_call_id,
        })
        .await
}

fn validate_input(input: &AgentLoopInput) -> Result<(), AgentLoopError> {
    if input.envelope.version != 1 {
        return Err(AgentLoopError::InvalidInput(format!(
            "unsupported delegation envelope version {}",
            input.envelope.version
        )));
    }
    if input.envelope.objective.trim().is_empty() {
        return Err(AgentLoopError::InvalidInput(
            "delegation objective is empty".into(),
        ));
    }
    if input.workspace.assignment != input.handle.workspace {
        return Err(AgentLoopError::InvalidInput(
            "workspace descriptor does not match dispatch handle".into(),
        ));
    }
    if input.envelope.workspace_warnings != input.workspace.warnings {
        return Err(AgentLoopError::InvalidInput(
            "delegation envelope and workspace descriptor warnings differ".into(),
        ));
    }
    if input.budget.timeout.is_zero()
        || input.budget.max_provider_events == 0
        || input.budget.max_tool_calls == 0
    {
        return Err(AgentLoopError::InvalidInput(
            "agent loop budgets must be positive".into(),
        ));
    }
    Ok(())
}

fn validate_binding(
    binding: &ProviderBinding,
    route: &ResolvedModelRoute,
) -> Result<(), AgentLoopError> {
    if binding.provider.trim().is_empty()
        || binding.profile.trim().is_empty()
        || binding.model.trim().is_empty()
        || binding.reasoning_effort.trim().is_empty()
        || binding.session_id.trim().is_empty()
    {
        return Err(AgentLoopError::InvalidProviderBinding(
            "provider binding contains an empty field".into(),
        ));
    }
    if binding.provider != route.provider
        || binding.profile != route.profile_name
        || binding.model != route.model
        || binding.reasoning_effort != route.reasoning_effort.as_str()
    {
        return Err(AgentLoopError::InvalidProviderBinding(
            "provider binding differs from the resolved dispatch route".into(),
        ));
    }
    Ok(())
}

fn synthetic_handoff(
    status: HandoffStatus,
    detail: impl Into<String>,
) -> Result<ValidatedHandoff, AgentLoopError> {
    let detail = detail.into();
    HandoffValidator::default()
        .validate(
            StructuredHandoffV1 {
                version: 1,
                status,
                summary: detail.clone(),
                evidence: Vec::new(),
                changed_files: Vec::new(),
                verification: Vec::new(),
                blockers: vec![detail],
                followups: Vec::new(),
                confidence: HandoffConfidence::Low,
            },
            Some(status),
        )
        .map_err(AgentLoopError::HandoffValidation)
}

#[derive(Debug)]
pub enum AgentLoopError {
    InvalidInput(String),
    CancelledBeforeBinding(String),
    TimedOutBeforeBinding,
    ProviderStart(AgentProviderError),
    Provider(AgentProviderError),
    ProviderProtocol(String),
    InvalidProviderBinding(String),
    Approval(AgentToolError),
    EventSink(String),
    RuntimeHook(String),
    HandoffValidation(HandoffValidationError),
}

impl fmt::Display for AgentLoopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(detail) => write!(formatter, "invalid child loop input: {detail}"),
            Self::CancelledBeforeBinding(reason) => {
                write!(
                    formatter,
                    "child cancelled before provider binding: {reason}"
                )
            }
            Self::TimedOutBeforeBinding => {
                formatter.write_str("child timed out before provider binding")
            }
            Self::ProviderStart(error) => {
                write!(formatter, "could not start child provider: {error}")
            }
            Self::Provider(error) => write!(formatter, "child provider failed: {error}"),
            Self::ProviderProtocol(detail) => {
                write!(formatter, "invalid provider event sequence: {detail}")
            }
            Self::InvalidProviderBinding(detail) => {
                write!(formatter, "invalid provider binding: {detail}")
            }
            Self::Approval(error) => write!(formatter, "child approval failed: {}", error.detail),
            Self::EventSink(detail) => write!(formatter, "could not record child event: {detail}"),
            Self::RuntimeHook(detail) => write!(formatter, "child runtime hook failed: {detail}"),
            Self::HandoffValidation(error) => {
                write!(formatter, "could not build terminal handoff: {error}")
            }
        }
    }
}

impl std::error::Error for AgentLoopError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::{
        fake_provider::FakeProviderAdapter, ModelRouteResolution, ReasoningEffort,
        WorkspaceStrategy,
    };
    use crate::run_log::{
        AgentId, AgentLifecycleEvent, AgentLifecycleState, EventActor, EventId,
        InMemoryRunEventSink, NewRunEvent, RunEventSink, RunId, WorkspaceId,
    };
    use std::sync::Mutex;

    struct UnusedToolExecutor;

    impl AgentToolExecutor for UnusedToolExecutor {
        fn execute(
            &self,
            _call: AgentToolCall,
        ) -> AgentFuture<'_, Result<AgentToolResult, AgentToolError>> {
            Box::pin(async { Err(AgentToolError::new("unexpected tool execution")) })
        }
    }

    struct CountingToolExecutor {
        calls: Arc<AtomicUsize>,
    }

    impl AgentToolExecutor for CountingToolExecutor {
        fn execute(
            &self,
            _call: AgentToolCall,
        ) -> AgentFuture<'_, Result<AgentToolResult, AgentToolError>> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Box::pin(async { Ok(AgentToolResult::completed(None)) })
        }
    }

    struct RecordingHarness {
        sink: Arc<InMemoryRunEventSink>,
        last_event: Mutex<Option<EventId>>,
        states: Mutex<Vec<DispatchState>>,
    }

    impl RecordingHarness {
        fn new() -> Self {
            Self {
                sink: Arc::new(InMemoryRunEventSink::new()),
                last_event: Mutex::new(None),
                states: Mutex::new(vec![DispatchState::Starting]),
            }
        }

        fn append(&self, record: AgentLoopEventRecord) -> Result<EventEnvelope, AgentLoopError> {
            let mut last = self.last_event.lock().unwrap();
            let event = self
                .sink
                .append(NewRunEvent {
                    run_id: record.handle.run_id,
                    caused_by: last.clone(),
                    operation_id: record.operation_id,
                    provider_call_id: record.provider_call_id,
                    actor: EventActor::Agent(record.handle.agent_id.clone()),
                    agent_id: Some(record.handle.agent_id),
                    turn_id: None,
                    workspace_id: Some(record.handle.workspace.workspace_id),
                    branch_id: None,
                    kind: record.kind,
                })
                .map_err(|error| AgentLoopError::EventSink(error.to_string()))?;
            *last = Some(event.event_id.clone());
            Ok(event)
        }
    }

    impl AgentLoopEventSink for RecordingHarness {
        fn record(
            &self,
            record: AgentLoopEventRecord,
        ) -> AgentFuture<'_, Result<EventEnvelope, AgentLoopError>> {
            Box::pin(async move { self.append(record) })
        }
    }

    impl AgentLoopRuntimeHooks for RecordingHarness {
        fn transition(
            &self,
            handle: &DispatchHandle,
            state: DispatchState,
            detail: Option<String>,
        ) -> AgentFuture<'_, Result<EventEnvelope, AgentLoopError>> {
            let handle = handle.clone();
            Box::pin(async move {
                self.states.lock().unwrap().push(state.clone());
                self.append(AgentLoopEventRecord {
                    handle: handle.clone(),
                    kind: EventKind::AgentLifecycle(AgentLifecycleEvent {
                        agent_id: handle.agent_id.clone(),
                        parent_agent_id: None,
                        state: match state {
                            DispatchState::Created => AgentLifecycleState::Created,
                            DispatchState::Queued => AgentLifecycleState::Queued,
                            DispatchState::Starting => AgentLifecycleState::Starting,
                            DispatchState::Running => AgentLifecycleState::Running,
                            DispatchState::WaitingForAgent => AgentLifecycleState::WaitingForAgent,
                            DispatchState::WaitingForTool => AgentLifecycleState::WaitingForTool,
                            DispatchState::WaitingForUser => AgentLifecycleState::WaitingForUser,
                            DispatchState::Completed => AgentLifecycleState::Completed,
                            DispatchState::Interrupted => AgentLifecycleState::Interrupted,
                            DispatchState::Failed => AgentLifecycleState::Failed,
                        },
                        kind: "test".into(),
                        objective: Some("test child".into()),
                        detail,
                        dispatch_spec: None,
                    }),
                    operation_id: None,
                    provider_call_id: None,
                })
            })
        }
    }

    fn route() -> ResolvedModelRoute {
        ResolvedModelRoute {
            catalog_generation: "test".into(),
            catalog_model_id: "test/model".into(),
            profile_name: "test".into(),
            provider: "fake".into(),
            model: "model".into(),
            reasoning_effort: ReasoningEffort::medium(),
            resolution: ModelRouteResolution::Exact,
            fallback_reason: None,
        }
    }

    fn input(
        scenario: &str,
        harness: Arc<RecordingHarness>,
        cancellation: AgentCancellationToken,
    ) -> AgentLoopInput {
        let assignment = WorkspaceAssignment {
            workspace_id: WorkspaceId::new(),
            strategy: WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
        };
        AgentLoopInput {
            handle: DispatchHandle {
                run_id: RunId::new(),
                agent_id: AgentId::new(),
                workspace: assignment.clone(),
            },
            envelope: DelegationEnvelope::objective("exercise provider-independent loop"),
            route: route(),
            provider: Arc::new(
                FakeProviderAdapter::new(scenario).with_tick_duration(Duration::from_millis(2)),
            ),
            tool_view: ScopedToolView::default(),
            tool_executor: Arc::new(UnusedToolExecutor),
            event_sink: harness.clone(),
            runtime_hooks: harness,
            approval_client: Arc::new(DenyAllAgentApprovals),
            workspace: AgentWorkspaceDescriptor {
                assignment,
                root: None,
                read_only: true,
                warnings: Vec::new(),
            },
            cancellation,
            budget: AgentLoopBudget {
                timeout: Duration::from_secs(1),
                ..AgentLoopBudget::default()
            },
            root_budget: Arc::new(RootAgentBudget::new(1024, 128)),
            handoff_validator: HandoffValidator::default(),
        }
    }

    #[tokio::test]
    async fn fake_provider_binding_and_validated_handoff_are_durable() {
        let harness = Arc::new(RecordingHarness::new());
        let result = AgentLoopRunner::run(input(
            "delayed_completion",
            harness.clone(),
            AgentCancellationToken::new(),
        ))
        .await
        .unwrap();

        assert_eq!(result.handoff.status(), HandoffStatus::Completed);
        assert_eq!(result.binding.model, "model");
        let events = harness.sink.events(&result_event_run(&harness)).unwrap();
        assert!(events.iter().any(|event| matches!(
            event.kind,
            EventKind::AgentProvider(RecordedAgentProviderEvent {
                state: AgentProviderState::Bound,
                ..
            })
        )));
        assert!(events.iter().any(|event| matches!(
            event.kind,
            EventKind::AgentProvider(RecordedAgentProviderEvent {
                state: AgentProviderState::CallStarted,
                ..
            })
        )));
        assert_eq!(
            harness.states.lock().unwrap().as_slice(),
            &[DispatchState::Starting, DispatchState::Running]
        );
    }

    #[tokio::test]
    async fn malformed_and_unknown_provider_calls_never_reach_the_scoped_executor() {
        let harness = Arc::new(RecordingHarness::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut input = input("delayed_completion", harness, AgentCancellationToken::new());
        input.tool_view = ScopedToolView::new([ScopedTool {
            name: "read_snapshot".into(),
            description: "Read one snapshot path.".into(),
            input_schema: crate::ai::tools::StrictJsonSchema::new(serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "path": { "type": "string", "minLength": 1 } },
                "required": ["path"]
            }))
            .unwrap(),
            side_effect: ToolSideEffect::Read,
            requires_approval: false,
        }])
        .unwrap();
        input.tool_executor = Arc::new(CountingToolExecutor {
            calls: calls.clone(),
        });
        let deadline = Instant::now() + Duration::from_secs(1);

        let malformed = execute_tool_request(
            &input,
            None,
            "malformed".into(),
            "read_snapshot".into(),
            serde_json::json!({}),
            deadline,
        )
        .await
        .unwrap();
        assert_eq!(malformed.outcome, ToolOutcome::Failed);
        assert!(malformed.summary.unwrap().contains("schema validation"));

        let unknown = execute_tool_request(
            &input,
            None,
            "unknown".into(),
            "bash".into(),
            serde_json::json!({"command": "pwd"}),
            deadline,
        )
        .await
        .unwrap();
        assert_eq!(unknown.outcome, ToolOutcome::Failed);
        assert!(unknown.summary.unwrap().contains("outside"));
        assert_eq!(calls.load(Ordering::Acquire), 0);
    }

    fn result_event_run(harness: &RecordingHarness) -> RunId {
        harness.sink.runs().unwrap().into_iter().next().unwrap()
    }

    #[tokio::test]
    async fn malformed_provider_handoff_becomes_validated_failure() {
        let harness = Arc::new(RecordingHarness::new());
        let result = AgentLoopRunner::run(input(
            "malformed_handoff",
            harness,
            AgentCancellationToken::new(),
        ))
        .await
        .unwrap();

        assert_eq!(result.handoff.status(), HandoffStatus::Failed);
        assert!(result.handoff.as_handoff().blockers[0].contains("invalid structured handoff"));
    }

    #[tokio::test]
    async fn tool_failure_trace_is_closed_before_independent_provider_failure() {
        let harness = Arc::new(RecordingHarness::new());
        let result = AgentLoopRunner::run(input(
            "tool_failure",
            harness.clone(),
            AgentCancellationToken::new(),
        ))
        .await
        .unwrap();
        assert_eq!(result.handoff.status(), HandoffStatus::Failed);
        let events = harness.sink.events(&result_event_run(&harness)).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ToolStarted(_)))
                .count(),
            1
        );
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::ToolResult(ToolResultEvent {
                outcome: ToolOutcome::Failed,
                ..
            })
        )));
    }

    #[tokio::test]
    async fn cancellation_returns_a_validated_interrupted_handoff() {
        let harness = Arc::new(RecordingHarness::new());
        let cancellation = AgentCancellationToken::new();
        let input = input("delayed_completion", harness, cancellation.clone());
        let task = tokio::spawn(async move { AgentLoopRunner::run(input).await.unwrap() });
        tokio::task::yield_now().await;
        assert!(cancellation.cancel("parent interrupted subtree"));
        let result = task.await.unwrap();

        assert_eq!(result.handoff.status(), HandoffStatus::Interrupted);
        assert!(result.handoff.as_handoff().blockers[0].contains("parent interrupted subtree"));
    }

    #[tokio::test]
    async fn provider_timeout_returns_a_validated_timed_out_handoff() {
        let harness = Arc::new(RecordingHarness::new());
        let result = AgentLoopRunner::run(input("timeout", harness, AgentCancellationToken::new()))
            .await
            .unwrap();
        assert_eq!(result.handoff.status(), HandoffStatus::TimedOut);
    }
}
