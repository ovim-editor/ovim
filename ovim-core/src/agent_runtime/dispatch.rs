//! Provider-independent multi-agent dispatch and scheduling policy.
//!
//! This module allocates ovim agent identities and records scheduling state. It
//! deliberately does not start provider sessions or create Git worktrees.

use crate::run_log::{
    AgentCapabilitySnapshot, AgentCompletionContractSnapshot, AgentDispatchSpecSnapshot, AgentId,
    AgentLifecycleEvent, AgentLifecycleState, AgentModelEffortSnapshot, AgentModelProfileSnapshot,
    AgentWorkspacePolicySnapshot, AgentWorkspaceStrategySnapshot, EventActor, EventEnvelope,
    EventId, EventKind, ManifestId, NewRunEvent, RunEventSink, RunId, RunLogError, TurnId,
    WorkspaceId,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentCapability {
    Read,
    Navigate,
    SafeShell,
    Shell,
    WorkspaceWrite,
    ExternalEffects,
    DispatchAgents,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentKindName {
    Implementer,
    Explorer,
    Verifier,
    Reviewer,
    Safety,
    Planner,
    Custom(String),
}

impl fmt::Display for AgentKindName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Implementer => formatter.write_str("implementer"),
            Self::Explorer => formatter.write_str("explorer"),
            Self::Verifier => formatter.write_str("verifier"),
            Self::Reviewer => formatter.write_str("reviewer"),
            Self::Safety => formatter.write_str("safety"),
            Self::Planner => formatter.write_str("planner"),
            Self::Custom(name) => write!(formatter, "custom:{name}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelEffort {
    Low,
    Medium,
    High,
}

/// Logical model requirements. Provider session/thread identifiers never
/// appear here and never participate in agent identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentModelProfile {
    pub model: String,
    pub effort: ModelEffort,
    pub fallback_model: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspacePolicy {
    SharedWorkspace,
    IsolatedWorktree,
    ReadOnlyProjection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionContract {
    StructuredHandoff,
    ReviewReport,
    SafetyVerdict,
    Plan,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentKind {
    pub name: AgentKindName,
    pub model: AgentModelProfile,
    pub instructions: String,
    pub capabilities: BTreeSet<AgentCapability>,
    pub workspace_policy: WorkspacePolicy,
    pub completion_contract: CompletionContract,
}

impl AgentKind {
    pub fn built_in(name: AgentKindName) -> Self {
        let capabilities = |values: &[AgentCapability]| values.iter().cloned().collect();
        match name {
            AgentKindName::Implementer => Self {
                name,
                model: model("gpt-5.6-sol", ModelEffort::Medium),
                instructions: "Implement the delegated objective and verify recorded changes."
                    .into(),
                capabilities: capabilities(&[
                    AgentCapability::Read,
                    AgentCapability::Navigate,
                    AgentCapability::Shell,
                    AgentCapability::WorkspaceWrite,
                ]),
                workspace_policy: WorkspacePolicy::IsolatedWorktree,
                completion_contract: CompletionContract::StructuredHandoff,
            },
            AgentKindName::Explorer => Self {
                name,
                model: model("gpt-5.6-terra", ModelEffort::Low),
                instructions: "Explore the delegated question without mutating source.".into(),
                capabilities: capabilities(&[
                    AgentCapability::Read,
                    AgentCapability::Navigate,
                    AgentCapability::SafeShell,
                ]),
                workspace_policy: WorkspacePolicy::ReadOnlyProjection,
                completion_contract: CompletionContract::ReviewReport,
            },
            AgentKindName::Verifier => Self {
                name,
                model: model("gpt-5.6-terra", ModelEffort::Low),
                instructions: "Run verification and report evidence and failures.".into(),
                capabilities: capabilities(&[
                    AgentCapability::Read,
                    AgentCapability::Navigate,
                    AgentCapability::Shell,
                ]),
                workspace_policy: WorkspacePolicy::IsolatedWorktree,
                completion_contract: CompletionContract::StructuredHandoff,
            },
            AgentKindName::Reviewer => Self {
                name,
                model: model("gpt-5.6-terra", ModelEffort::Medium),
                instructions: "Review the selected source state without changing it.".into(),
                capabilities: capabilities(&[AgentCapability::Read, AgentCapability::Navigate]),
                workspace_policy: WorkspacePolicy::ReadOnlyProjection,
                completion_contract: CompletionContract::ReviewReport,
            },
            AgentKindName::Safety => Self {
                name,
                model: AgentModelProfile {
                    model: "gpt-5.6-luna".into(),
                    effort: ModelEffort::Low,
                    fallback_model: Some("gpt-5.6-terra".into()),
                },
                instructions: "Classify the proposed action against explicit user authorization."
                    .into(),
                capabilities: capabilities(&[AgentCapability::Read]),
                workspace_policy: WorkspacePolicy::ReadOnlyProjection,
                completion_contract: CompletionContract::SafetyVerdict,
            },
            AgentKindName::Planner => Self {
                name,
                model: model("gpt-5.6-sol", ModelEffort::Medium),
                instructions: "Decompose objectives and dispatch appropriately scoped agents."
                    .into(),
                capabilities: capabilities(&[
                    AgentCapability::Read,
                    AgentCapability::Navigate,
                    AgentCapability::DispatchAgents,
                ]),
                workspace_policy: WorkspacePolicy::ReadOnlyProjection,
                completion_contract: CompletionContract::Plan,
            },
            AgentKindName::Custom(_) => Self {
                name,
                model: model("gpt-5.6-terra", ModelEffort::Medium),
                instructions: String::new(),
                capabilities: BTreeSet::new(),
                workspace_policy: WorkspacePolicy::ReadOnlyProjection,
                completion_contract: CompletionContract::Custom("unspecified".into()),
            },
        }
    }

    pub fn can_write(&self) -> bool {
        self.capabilities.contains(&AgentCapability::WorkspaceWrite)
            // Arbitrary shell can mutate files even when the agent's primary
            // role is verification rather than implementation.
            || self.capabilities.contains(&AgentCapability::Shell)
    }
}

fn model(model: &str, effort: ModelEffort) -> AgentModelProfile {
    AgentModelProfile {
        model: model.into(),
        effort,
        fallback_model: None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceStrategy {
    SharedWorkspace,
    IsolatedWorktree {
        base_manifest_id: Option<ManifestId>,
    },
    ReadOnlySnapshot {
        manifest_id: Option<ManifestId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceAssignment {
    pub workspace_id: WorkspaceId,
    pub strategy: WorkspaceStrategy,
}

#[derive(Clone, Debug)]
pub struct DispatchRequest {
    pub objective: String,
    pub kind: AgentKind,
    pub parent_agent_id: Option<AgentId>,
    pub causing_turn_id: Option<TurnId>,
    pub caused_by_event: Option<EventId>,
    pub workspace: WorkspaceAssignment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchHandle {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub workspace: WorkspaceAssignment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DispatchState {
    Created,
    Queued,
    Starting,
    Running,
    WaitingForAgent,
    WaitingForTool,
    WaitingForUser,
    Completed,
    Interrupted,
    Failed,
}

impl DispatchState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Interrupted | Self::Failed)
    }
}

#[derive(Clone, Debug)]
struct ScheduledAgent {
    handle: DispatchHandle,
    kind: AgentKind,
    objective: String,
    parent_agent_id: Option<AgentId>,
    causing_turn_id: Option<TurnId>,
    state: DispatchState,
    last_event_id: EventId,
}

pub struct AgentDispatchScheduler {
    run_id: RunId,
    sink: Arc<dyn RunEventSink>,
    agents: HashMap<AgentId, ScheduledAgent>,
    shared_writer: HashMap<WorkspaceId, AgentId>,
}

impl AgentDispatchScheduler {
    pub fn new(run_id: RunId, sink: Arc<dyn RunEventSink>) -> Self {
        Self {
            run_id,
            sink,
            agents: HashMap::new(),
            shared_writer: HashMap::new(),
        }
    }

    pub fn dispatch(&mut self, request: DispatchRequest) -> Result<DispatchHandle, DispatchError> {
        self.validate_request(&request)?;
        let agent_id = AgentId::new();
        if request.kind.can_write()
            && matches!(
                request.workspace.strategy,
                WorkspaceStrategy::SharedWorkspace
            )
        {
            if let Some(holder) = self.shared_writer.get(&request.workspace.workspace_id) {
                return Err(DispatchError::SharedWorkspaceWriterHeld {
                    workspace_id: request.workspace.workspace_id.clone(),
                    holder: holder.clone(),
                });
            }
        }
        let handle = DispatchHandle {
            run_id: self.run_id.clone(),
            agent_id: agent_id.clone(),
            workspace: request.workspace.clone(),
        };
        let created_event = self
            .sink
            .append(NewRunEvent {
                run_id: self.run_id.clone(),
                caused_by: request.caused_by_event.clone(),
                operation_id: None,
                provider_call_id: None,
                actor: request
                    .parent_agent_id
                    .clone()
                    .map(EventActor::Agent)
                    .unwrap_or_else(|| EventActor::System("agent_scheduler".into())),
                agent_id: Some(agent_id.clone()),
                turn_id: request.causing_turn_id.clone(),
                workspace_id: Some(request.workspace.workspace_id.clone()),
                branch_id: None,
                kind: lifecycle_event(
                    &agent_id,
                    request.parent_agent_id.clone(),
                    &request.kind,
                    Some(request.objective.clone()),
                    Some(dispatch_spec_snapshot(
                        &request.kind,
                        &request.workspace.strategy,
                    )),
                    DispatchState::Created,
                    None,
                ),
            })
            .map_err(DispatchError::RunLog)?;
        let queued_event = self
            .sink
            .append(NewRunEvent {
                run_id: self.run_id.clone(),
                caused_by: Some(created_event.event_id.clone()),
                operation_id: None,
                provider_call_id: None,
                actor: EventActor::System("agent_scheduler".into()),
                agent_id: Some(agent_id.clone()),
                turn_id: request.causing_turn_id.clone(),
                workspace_id: Some(request.workspace.workspace_id.clone()),
                branch_id: None,
                kind: lifecycle_event(
                    &agent_id,
                    request.parent_agent_id.clone(),
                    &request.kind,
                    Some(request.objective.clone()),
                    None,
                    DispatchState::Queued,
                    None,
                ),
            })
            .map_err(|source| DispatchError::QueueAfterCreationFailed {
                agent_id: agent_id.clone(),
                created_event_id: created_event.event_id,
                source,
            })?;
        if request.kind.can_write()
            && matches!(
                request.workspace.strategy,
                WorkspaceStrategy::SharedWorkspace
            )
        {
            self.shared_writer
                .insert(request.workspace.workspace_id.clone(), agent_id.clone());
        }
        self.agents.insert(
            agent_id,
            ScheduledAgent {
                handle: handle.clone(),
                kind: request.kind,
                objective: request.objective,
                parent_agent_id: request.parent_agent_id,
                causing_turn_id: request.causing_turn_id,
                state: DispatchState::Queued,
                last_event_id: queued_event.event_id,
            },
        );
        Ok(handle)
    }

    pub fn transition(
        &mut self,
        handle: &DispatchHandle,
        next: DispatchState,
        detail: Option<String>,
    ) -> Result<EventEnvelope, DispatchError> {
        let agent = self
            .agents
            .get(&handle.agent_id)
            .ok_or_else(|| DispatchError::UnknownAgent(handle.agent_id.clone()))?;
        if agent.handle.run_id != handle.run_id || agent.handle.workspace != handle.workspace {
            return Err(DispatchError::HandleMismatch(handle.agent_id.clone()));
        }
        if !valid_transition(&agent.state, &next) {
            return Err(DispatchError::InvalidTransition {
                agent_id: handle.agent_id.clone(),
                from: agent.state.clone(),
                to: next,
            });
        }
        let event = self
            .sink
            .append(NewRunEvent {
                run_id: self.run_id.clone(),
                caused_by: Some(agent.last_event_id.clone()),
                operation_id: None,
                provider_call_id: None,
                actor: EventActor::System("agent_scheduler".into()),
                agent_id: Some(handle.agent_id.clone()),
                turn_id: agent.causing_turn_id.clone(),
                workspace_id: Some(handle.workspace.workspace_id.clone()),
                branch_id: None,
                kind: lifecycle_event(
                    &handle.agent_id,
                    agent.parent_agent_id.clone(),
                    &agent.kind,
                    Some(agent.objective.clone()),
                    None,
                    next.clone(),
                    detail,
                ),
            })
            .map_err(DispatchError::RunLog)?;
        let agent = self
            .agents
            .get_mut(&handle.agent_id)
            .expect("checked above");
        agent.state = next.clone();
        agent.last_event_id = event.event_id.clone();
        if next.is_terminal() {
            if self.shared_writer.get(&handle.workspace.workspace_id) == Some(&handle.agent_id) {
                self.shared_writer.remove(&handle.workspace.workspace_id);
            }
        }
        Ok(event)
    }

    pub fn state(&self, agent_id: &AgentId) -> Option<&DispatchState> {
        self.agents.get(agent_id).map(|agent| &agent.state)
    }

    fn validate_request(&self, request: &DispatchRequest) -> Result<(), DispatchError> {
        if request.objective.trim().is_empty() {
            return Err(DispatchError::EmptyObjective);
        }
        match (&request.parent_agent_id, &request.causing_turn_id) {
            (Some(parent), Some(turn)) if !self.agents.contains_key(parent) => {
                let _ = turn;
                return Err(DispatchError::UnknownParent(parent.clone()));
            }
            (Some(parent), Some(turn)) => {
                let cause_id = request
                    .caused_by_event
                    .as_ref()
                    .ok_or(DispatchError::ChildMissingCausingEvent)?;
                let cause = self
                    .sink
                    .event(&self.run_id, cause_id)
                    .map_err(DispatchError::RunLog)?
                    .ok_or_else(|| DispatchError::CausingEventNotFound(cause_id.clone()))?;
                if cause.run_id != self.run_id {
                    return Err(DispatchError::CausingEventWrongRun(cause_id.clone()));
                }
                if cause.agent_id.as_ref() != Some(parent) {
                    return Err(DispatchError::CausingEventAgentMismatch {
                        event_id: cause_id.clone(),
                        expected: parent.clone(),
                        actual: cause.agent_id,
                    });
                }
                if cause.turn_id.as_ref() != Some(turn) {
                    return Err(DispatchError::CausingEventTurnMismatch {
                        event_id: cause_id.clone(),
                        expected: turn.clone(),
                        actual: cause.turn_id,
                    });
                }
            }
            (Some(_), None) => return Err(DispatchError::ChildMissingCausingTurn),
            (None, Some(_)) => return Err(DispatchError::TurnWithoutParent),
            _ => {}
        }
        if request.kind.can_write()
            && matches!(
                request.workspace.strategy,
                WorkspaceStrategy::ReadOnlySnapshot { .. }
            )
        {
            return Err(DispatchError::WriteCapabilityInReadOnlyWorkspace);
        }
        Ok(())
    }
}

fn valid_transition(from: &DispatchState, to: &DispatchState) -> bool {
    use DispatchState::*;
    matches!(
        (from, to),
        (Created, Queued | Interrupted | Failed)
            | (Queued, Starting | Interrupted | Failed)
            | (Starting, Running | Interrupted | Failed)
            | (
                Running,
                WaitingForAgent
                    | WaitingForTool
                    | WaitingForUser
                    | Completed
                    | Interrupted
                    | Failed
            )
            | (
                WaitingForAgent | WaitingForTool | WaitingForUser,
                Running | Completed | Interrupted | Failed
            )
    )
}

fn lifecycle_event(
    agent_id: &AgentId,
    parent_agent_id: Option<AgentId>,
    kind: &AgentKind,
    objective: Option<String>,
    dispatch_spec: Option<AgentDispatchSpecSnapshot>,
    state: DispatchState,
    detail: Option<String>,
) -> EventKind {
    EventKind::AgentLifecycle(AgentLifecycleEvent {
        agent_id: agent_id.clone(),
        parent_agent_id,
        state: lifecycle_state(state),
        kind: kind.name.to_string(),
        objective,
        detail,
        dispatch_spec,
    })
}

fn dispatch_spec_snapshot(
    kind: &AgentKind,
    assigned_workspace: &WorkspaceStrategy,
) -> AgentDispatchSpecSnapshot {
    AgentDispatchSpecSnapshot {
        version: 1,
        model: AgentModelProfileSnapshot {
            model: kind.model.model.clone(),
            effort: match kind.model.effort {
                ModelEffort::Low => AgentModelEffortSnapshot::Low,
                ModelEffort::Medium => AgentModelEffortSnapshot::Medium,
                ModelEffort::High => AgentModelEffortSnapshot::High,
            },
            fallback_model: kind.model.fallback_model.clone(),
        },
        instructions: kind.instructions.clone(),
        capabilities: kind
            .capabilities
            .iter()
            .map(|capability| match capability {
                AgentCapability::Read => AgentCapabilitySnapshot::Read,
                AgentCapability::Navigate => AgentCapabilitySnapshot::Navigate,
                AgentCapability::SafeShell => AgentCapabilitySnapshot::SafeShell,
                AgentCapability::Shell => AgentCapabilitySnapshot::Shell,
                AgentCapability::WorkspaceWrite => AgentCapabilitySnapshot::WorkspaceWrite,
                AgentCapability::ExternalEffects => AgentCapabilitySnapshot::ExternalEffects,
                AgentCapability::DispatchAgents => AgentCapabilitySnapshot::DispatchAgents,
            })
            .collect(),
        kind_workspace_policy: match kind.workspace_policy {
            WorkspacePolicy::SharedWorkspace => AgentWorkspacePolicySnapshot::SharedWorkspace,
            WorkspacePolicy::IsolatedWorktree => AgentWorkspacePolicySnapshot::IsolatedWorktree,
            WorkspacePolicy::ReadOnlyProjection => AgentWorkspacePolicySnapshot::ReadOnlyProjection,
        },
        assigned_workspace: match assigned_workspace {
            WorkspaceStrategy::SharedWorkspace => AgentWorkspaceStrategySnapshot::SharedWorkspace,
            WorkspaceStrategy::IsolatedWorktree { base_manifest_id } => {
                AgentWorkspaceStrategySnapshot::IsolatedWorktree {
                    base_manifest_id: base_manifest_id.clone(),
                }
            }
            WorkspaceStrategy::ReadOnlySnapshot { manifest_id } => {
                AgentWorkspaceStrategySnapshot::ReadOnlySnapshot {
                    manifest_id: manifest_id.clone(),
                }
            }
        },
        completion_contract: match &kind.completion_contract {
            CompletionContract::StructuredHandoff => {
                AgentCompletionContractSnapshot::StructuredHandoff
            }
            CompletionContract::ReviewReport => AgentCompletionContractSnapshot::ReviewReport,
            CompletionContract::SafetyVerdict => AgentCompletionContractSnapshot::SafetyVerdict,
            CompletionContract::Plan => AgentCompletionContractSnapshot::Plan,
            CompletionContract::Custom(value) => {
                AgentCompletionContractSnapshot::Custom(value.clone())
            }
        },
    }
}

fn lifecycle_state(state: DispatchState) -> AgentLifecycleState {
    match state {
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
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    EmptyObjective,
    UnknownParent(AgentId),
    ChildMissingCausingTurn,
    ChildMissingCausingEvent,
    TurnWithoutParent,
    CausingEventNotFound(EventId),
    CausingEventWrongRun(EventId),
    CausingEventAgentMismatch {
        event_id: EventId,
        expected: AgentId,
        actual: Option<AgentId>,
    },
    CausingEventTurnMismatch {
        event_id: EventId,
        expected: TurnId,
        actual: Option<TurnId>,
    },
    WriteCapabilityInReadOnlyWorkspace,
    SharedWorkspaceWriterHeld {
        workspace_id: WorkspaceId,
        holder: AgentId,
    },
    UnknownAgent(AgentId),
    HandleMismatch(AgentId),
    InvalidTransition {
        agent_id: AgentId,
        from: DispatchState,
        to: DispatchState,
    },
    RunLog(RunLogError),
    QueueAfterCreationFailed {
        agent_id: AgentId,
        created_event_id: EventId,
        source: RunLogError,
    },
}

impl fmt::Display for DispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyObjective => formatter.write_str("agent objective is empty"),
            Self::UnknownParent(id) => write!(formatter, "parent agent {id} is unknown"),
            Self::ChildMissingCausingTurn => {
                formatter.write_str("child dispatch has no causing turn")
            }
            Self::ChildMissingCausingEvent => {
                formatter.write_str("child dispatch has no causing event")
            }
            Self::TurnWithoutParent => {
                formatter.write_str("dispatch has a causing turn but no parent agent")
            }
            Self::CausingEventNotFound(id) => {
                write!(formatter, "causing event {id} is not present in this run")
            }
            Self::CausingEventWrongRun(id) => {
                write!(formatter, "causing event {id} belongs to another run")
            }
            Self::CausingEventAgentMismatch {
                event_id,
                expected,
                actual,
            } => write!(
                formatter,
                "causing event {event_id} belongs to agent {actual:?}, expected {expected}"
            ),
            Self::CausingEventTurnMismatch {
                event_id,
                expected,
                actual,
            } => write!(
                formatter,
                "causing event {event_id} belongs to turn {actual:?}, expected {expected}"
            ),
            Self::WriteCapabilityInReadOnlyWorkspace => formatter
                .write_str("write-capable agent cannot use a read-only workspace projection"),
            Self::SharedWorkspaceWriterHeld {
                workspace_id,
                holder,
            } => write!(
                formatter,
                "shared workspace {workspace_id} writer lease is held by agent {holder}"
            ),
            Self::UnknownAgent(id) => write!(formatter, "agent {id} is unknown"),
            Self::HandleMismatch(id) => write!(
                formatter,
                "dispatch handle for agent {id} does not match scheduler state"
            ),
            Self::InvalidTransition { agent_id, from, to } => write!(
                formatter,
                "agent {agent_id} cannot transition from {from:?} to {to:?}"
            ),
            Self::RunLog(error) => write!(formatter, "could not record agent lifecycle: {error}"),
            Self::QueueAfterCreationFailed {
                agent_id,
                created_event_id,
                source,
            } => write!(
                formatter,
                "agent {agent_id} was durably created by {created_event_id} but could not be queued: {source}"
            ),
        }
    }
}

impl std::error::Error for DispatchError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedAgent {
    pub agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub causing_turn_id: Option<TurnId>,
    pub kind: String,
    pub objective: Option<String>,
    pub workspace_id: Option<WorkspaceId>,
    pub dispatch_spec: Option<AgentDispatchSpecSnapshot>,
    pub state: DispatchState,
}

/// UI-facing selection is deliberately separate from scheduler ownership and
/// writer leases. Switching it cannot pause, resume, or cancel an agent.
#[derive(Clone, Debug, Default)]
pub struct AgentFollowProjection {
    agents: BTreeMap<AgentId, ProjectedAgent>,
    children: BTreeMap<AgentId, Vec<AgentId>>,
    selected: Option<AgentId>,
    followed: Option<AgentId>,
}

impl AgentFollowProjection {
    pub fn rehydrate(events: &[EventEnvelope]) -> Result<Self, ProjectionError> {
        let mut ordered: Vec<_> = events.iter().collect();
        ordered.sort_by_key(|event| event.sequence);
        let mut projection = Self::default();
        for event in ordered {
            let EventKind::AgentLifecycle(lifecycle) = &event.kind else {
                continue;
            };
            if event.agent_id.as_ref() != Some(&lifecycle.agent_id) {
                return Err(ProjectionError::EnvelopeAgentMismatch(
                    lifecycle.agent_id.clone(),
                ));
            }
            let state = projected_state(&lifecycle.state);
            match projection.agents.get_mut(&lifecycle.agent_id) {
                None => {
                    if !matches!(state, DispatchState::Created | DispatchState::Queued) {
                        return Err(ProjectionError::LifecycleBeforeDispatch(
                            lifecycle.agent_id.clone(),
                        ));
                    }
                    if let Some(parent) = &lifecycle.parent_agent_id {
                        if !projection.agents.contains_key(parent) {
                            return Err(ProjectionError::UnknownParent(parent.clone()));
                        }
                        projection
                            .children
                            .entry(parent.clone())
                            .or_default()
                            .push(lifecycle.agent_id.clone());
                    }
                    projection.agents.insert(
                        lifecycle.agent_id.clone(),
                        ProjectedAgent {
                            agent_id: lifecycle.agent_id.clone(),
                            parent_agent_id: lifecycle.parent_agent_id.clone(),
                            causing_turn_id: event.turn_id.clone(),
                            kind: lifecycle.kind.clone(),
                            objective: lifecycle.objective.clone(),
                            workspace_id: event.workspace_id.clone(),
                            dispatch_spec: lifecycle.dispatch_spec.clone(),
                            state,
                        },
                    );
                    if projection.selected.is_none() {
                        projection.selected = Some(lifecycle.agent_id.clone());
                    }
                }
                Some(agent) => {
                    if agent.parent_agent_id != lifecycle.parent_agent_id {
                        return Err(ProjectionError::ParentChanged(agent.agent_id.clone()));
                    }
                    if lifecycle.dispatch_spec.is_some() {
                        return Err(ProjectionError::DispatchSpecRepeated(
                            agent.agent_id.clone(),
                        ));
                    }
                    let legacy_start = agent.state == DispatchState::Queued
                        && lifecycle.state == AgentLifecycleState::Started;
                    if !legacy_start && !valid_transition(&agent.state, &state) {
                        return Err(ProjectionError::InvalidLifecycle {
                            agent_id: agent.agent_id.clone(),
                            from: agent.state.clone(),
                            to: state,
                        });
                    }
                    agent.state = state;
                }
            }
        }
        projection.followed = projection.selected.clone();
        Ok(projection)
    }

    pub fn agents(&self) -> &BTreeMap<AgentId, ProjectedAgent> {
        &self.agents
    }

    pub fn children(&self, agent_id: &AgentId) -> &[AgentId] {
        self.children
            .get(agent_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn selected(&self) -> Option<&AgentId> {
        self.selected.as_ref()
    }

    pub fn followed(&self) -> Option<&AgentId> {
        self.followed.as_ref()
    }

    pub fn select(&mut self, agent_id: AgentId) -> Result<(), ProjectionError> {
        if !self.agents.contains_key(&agent_id) {
            return Err(ProjectionError::UnknownAgent(agent_id));
        }
        self.selected = Some(agent_id);
        Ok(())
    }

    pub fn follow(&mut self, agent_id: AgentId) -> Result<(), ProjectionError> {
        if !self.agents.contains_key(&agent_id) {
            return Err(ProjectionError::UnknownAgent(agent_id));
        }
        self.followed = Some(agent_id);
        Ok(())
    }
}

fn projected_state(state: &AgentLifecycleState) -> DispatchState {
    match state {
        AgentLifecycleState::Created => DispatchState::Created,
        AgentLifecycleState::Dispatched | AgentLifecycleState::Queued => DispatchState::Queued,
        AgentLifecycleState::Starting => DispatchState::Starting,
        AgentLifecycleState::Started | AgentLifecycleState::Running => DispatchState::Running,
        AgentLifecycleState::Waiting | AgentLifecycleState::WaitingForAgent => {
            DispatchState::WaitingForAgent
        }
        AgentLifecycleState::WaitingForTool => DispatchState::WaitingForTool,
        AgentLifecycleState::Blocked | AgentLifecycleState::WaitingForUser => {
            DispatchState::WaitingForUser
        }
        AgentLifecycleState::Completed => DispatchState::Completed,
        AgentLifecycleState::Interrupted => DispatchState::Interrupted,
        AgentLifecycleState::Failed => DispatchState::Failed,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionError {
    EnvelopeAgentMismatch(AgentId),
    LifecycleBeforeDispatch(AgentId),
    UnknownParent(AgentId),
    ParentChanged(AgentId),
    DispatchSpecRepeated(AgentId),
    InvalidLifecycle {
        agent_id: AgentId,
        from: DispatchState,
        to: DispatchState,
    },
    UnknownAgent(AgentId),
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ProjectionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::{InMemoryRunEventSink, MessageEvent, MessageRole};

    fn scheduler() -> (AgentDispatchScheduler, Arc<InMemoryRunEventSink>) {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let scheduler =
            AgentDispatchScheduler::new(RunId::parse("run_dispatch_test").unwrap(), sink.clone());
        (scheduler, sink)
    }

    fn request(
        kind: AgentKind,
        workspace_id: WorkspaceId,
        strategy: WorkspaceStrategy,
    ) -> DispatchRequest {
        DispatchRequest {
            objective: "bounded delegated objective".into(),
            kind,
            parent_agent_id: None,
            causing_turn_id: None,
            caused_by_event: None,
            workspace: WorkspaceAssignment {
                workspace_id,
                strategy,
            },
        }
    }

    fn record_parent_turn(
        sink: &InMemoryRunEventSink,
        parent: &DispatchHandle,
        turn_id: TurnId,
    ) -> EventEnvelope {
        let prior = sink
            .events(&parent.run_id)
            .unwrap()
            .into_iter()
            .rev()
            .find(|event| event.agent_id.as_ref() == Some(&parent.agent_id))
            .unwrap();
        sink.append(NewRunEvent {
            run_id: parent.run_id.clone(),
            caused_by: Some(prior.event_id),
            operation_id: None,
            provider_call_id: None,
            actor: EventActor::Agent(parent.agent_id.clone()),
            agent_id: Some(parent.agent_id.clone()),
            turn_id: Some(turn_id),
            workspace_id: Some(parent.workspace.workspace_id.clone()),
            branch_id: None,
            kind: EventKind::Message(MessageEvent {
                role: MessageRole::Agent,
                content: "dispatch child".into(),
            }),
        })
        .unwrap()
    }

    #[test]
    fn child_has_ovim_identity_and_durable_parent_turn_causality() {
        let (mut scheduler, sink) = scheduler();
        let workspace = WorkspaceId::parse("wsp_parent_child").unwrap();
        let parent = scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Planner),
                workspace.clone(),
                WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
            ))
            .unwrap();
        let turn = TurnId::parse("trn_causing_dispatch").unwrap();
        let cause = record_parent_turn(&sink, &parent, turn.clone());
        let child = scheduler
            .dispatch(DispatchRequest {
                objective: "inspect the storage layer".into(),
                kind: AgentKind::built_in(AgentKindName::Explorer),
                parent_agent_id: Some(parent.agent_id.clone()),
                causing_turn_id: Some(turn.clone()),
                caused_by_event: Some(cause.event_id.clone()),
                workspace: WorkspaceAssignment {
                    workspace_id: workspace,
                    strategy: WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
                },
            })
            .unwrap();

        assert_ne!(parent.agent_id, child.agent_id);
        let events = sink.events(&parent.run_id).unwrap();
        let child_event = events
            .iter()
            .find(|event| event.agent_id.as_ref() == Some(&child.agent_id))
            .unwrap();
        let EventKind::AgentLifecycle(lifecycle) = &child_event.kind else {
            panic!("child dispatch was not a lifecycle event")
        };
        assert_eq!(lifecycle.parent_agent_id.as_ref(), Some(&parent.agent_id));
        assert_eq!(child_event.turn_id.as_ref(), Some(&turn));
        assert_eq!(child_event.actor, EventActor::Agent(parent.agent_id));
        assert_eq!(child_event.caused_by.as_ref(), Some(&cause.event_id));
        assert!(child_event.provider_call_id.is_none());
    }

    #[test]
    fn concurrent_read_agents_share_a_snapshot() {
        let (mut scheduler, _) = scheduler();
        let workspace = WorkspaceId::parse("wsp_shared_readers").unwrap();
        let first = scheduler.dispatch(request(
            AgentKind::built_in(AgentKindName::Explorer),
            workspace.clone(),
            WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
        ));
        let second = scheduler.dispatch(request(
            AgentKind::built_in(AgentKindName::Reviewer),
            workspace,
            WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
        ));
        assert!(first.is_ok());
        assert!(second.is_ok());
    }

    #[test]
    fn child_dispatch_requires_a_matching_parent_turn_event() {
        let (mut scheduler, sink) = scheduler();
        let workspace = WorkspaceId::parse("wsp_causal_rejections").unwrap();
        let parent = scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Planner),
                workspace.clone(),
                WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
            ))
            .unwrap();
        let turn = TurnId::parse("trn_expected_cause").unwrap();
        let child_request = |causing_turn_id, caused_by_event| DispatchRequest {
            objective: "causally delegated work".into(),
            kind: AgentKind::built_in(AgentKindName::Explorer),
            parent_agent_id: Some(parent.agent_id.clone()),
            causing_turn_id: Some(causing_turn_id),
            caused_by_event,
            workspace: WorkspaceAssignment {
                workspace_id: workspace.clone(),
                strategy: WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
            },
        };
        assert_eq!(
            scheduler
                .dispatch(child_request(turn.clone(), None))
                .unwrap_err(),
            DispatchError::ChildMissingCausingEvent
        );

        let cause = record_parent_turn(&sink, &parent, turn.clone());
        assert!(matches!(
            scheduler
                .dispatch(child_request(
                    TurnId::parse("trn_wrong_cause").unwrap(),
                    Some(cause.event_id.clone()),
                ))
                .unwrap_err(),
            DispatchError::CausingEventTurnMismatch { event_id, .. }
                if event_id == cause.event_id
        ));

        let other_parent = scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Planner),
                workspace.clone(),
                WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
            ))
            .unwrap();
        let other_cause = record_parent_turn(&sink, &other_parent, turn.clone());
        assert!(matches!(
            scheduler
                .dispatch(child_request(turn, Some(other_cause.event_id.clone())))
                .unwrap_err(),
            DispatchError::CausingEventAgentMismatch { event_id, .. }
                if event_id == other_cause.event_id
        ));
    }

    #[test]
    fn dispatch_records_created_then_queued_and_rejects_invalid_lifecycle_edges() {
        let (mut scheduler, sink) = scheduler();
        let handle = scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Explorer),
                WorkspaceId::parse("wsp_lifecycle").unwrap(),
                WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
            ))
            .unwrap();
        let events = sink.events(&handle.run_id).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].kind,
            EventKind::AgentLifecycle(AgentLifecycleEvent {
                state: AgentLifecycleState::Created,
                ..
            })
        ));
        assert!(matches!(
            events[1].kind,
            EventKind::AgentLifecycle(AgentLifecycleEvent {
                state: AgentLifecycleState::Queued,
                ..
            })
        ));
        assert_eq!(events[1].caused_by.as_ref(), Some(&events[0].event_id));

        assert!(matches!(
            scheduler.transition(&handle, DispatchState::Completed, None),
            Err(DispatchError::InvalidTransition {
                from: DispatchState::Queued,
                to: DispatchState::Completed,
                ..
            })
        ));
        assert_eq!(
            scheduler.state(&handle.agent_id),
            Some(&DispatchState::Queued)
        );
    }

    #[test]
    fn resolved_custom_dispatch_spec_round_trips_and_rehydrates() {
        let (mut scheduler, sink) = scheduler();
        let kind = AgentKind {
            name: AgentKindName::Custom("migration-auditor".into()),
            model: AgentModelProfile {
                model: "custom-model-v2".into(),
                effort: ModelEffort::High,
                fallback_model: Some("fallback-model".into()),
            },
            instructions: "Audit migrations using the pinned policy revision.".into(),
            capabilities: BTreeSet::from([
                AgentCapability::Read,
                AgentCapability::Navigate,
                AgentCapability::SafeShell,
            ]),
            workspace_policy: WorkspacePolicy::ReadOnlyProjection,
            completion_contract: CompletionContract::Custom("migration-audit-v3".into()),
        };
        let handle = scheduler
            .dispatch(request(
                kind,
                WorkspaceId::parse("wsp_custom_snapshot").unwrap(),
                WorkspaceStrategy::ReadOnlySnapshot {
                    manifest_id: Some(ManifestId::parse("mft_custom_snapshot").unwrap()),
                },
            ))
            .unwrap();
        let events = sink.events(&handle.run_id).unwrap();
        let EventKind::AgentLifecycle(created) = &events[0].kind else {
            panic!("expected created lifecycle")
        };
        let snapshot = created.dispatch_spec.as_ref().unwrap();
        assert_eq!(snapshot.version, 1);
        assert_eq!(snapshot.model.model, "custom-model-v2");
        assert_eq!(snapshot.model.effort, AgentModelEffortSnapshot::High);
        assert_eq!(
            snapshot.completion_contract,
            AgentCompletionContractSnapshot::Custom("migration-audit-v3".into())
        );
        assert!(matches!(
            snapshot.assigned_workspace,
            AgentWorkspaceStrategySnapshot::ReadOnlySnapshot { .. }
        ));
        assert!(matches!(
            events[1].kind,
            EventKind::AgentLifecycle(AgentLifecycleEvent {
                dispatch_spec: None,
                ..
            })
        ));

        let wire = serde_json::to_string(&events[0].kind).unwrap();
        let restored: EventKind = serde_json::from_str(&wire).unwrap();
        assert_eq!(restored, events[0].kind);
        let projection = AgentFollowProjection::rehydrate(&events).unwrap();
        assert_eq!(
            projection.agents()[&handle.agent_id].dispatch_spec.as_ref(),
            Some(snapshot)
        );
    }

    #[test]
    fn shared_workspace_allows_only_one_writer_until_terminal() {
        let (mut scheduler, _) = scheduler();
        let workspace = WorkspaceId::parse("wsp_shared_writer").unwrap();
        let first = scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Implementer),
                workspace.clone(),
                WorkspaceStrategy::SharedWorkspace,
            ))
            .unwrap();
        let rejected = scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Implementer),
                workspace.clone(),
                WorkspaceStrategy::SharedWorkspace,
            ))
            .unwrap_err();
        assert!(matches!(
            rejected,
            DispatchError::SharedWorkspaceWriterHeld {
                workspace_id,
                holder
            } if workspace_id == workspace && holder == first.agent_id
        ));

        scheduler
            .transition(&first, DispatchState::Starting, None)
            .unwrap();
        scheduler
            .transition(&first, DispatchState::Running, None)
            .unwrap();
        scheduler
            .transition(&first, DispatchState::Completed, None)
            .unwrap();
        assert!(scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Implementer),
                workspace,
                WorkspaceStrategy::SharedWorkspace,
            ))
            .is_ok());
    }

    #[test]
    fn isolated_worktree_writers_may_coexist() {
        let (mut scheduler, _) = scheduler();
        let base = WorkspaceId::parse("wsp_isolated_base").unwrap();
        let first = scheduler.dispatch(request(
            AgentKind::built_in(AgentKindName::Implementer),
            base.clone(),
            WorkspaceStrategy::IsolatedWorktree {
                base_manifest_id: None,
            },
        ));
        let second = scheduler.dispatch(request(
            AgentKind::built_in(AgentKindName::Implementer),
            base,
            WorkspaceStrategy::IsolatedWorktree {
                base_manifest_id: None,
            },
        ));
        assert!(first.is_ok());
        assert!(second.is_ok());
    }

    #[test]
    fn arbitrary_shell_is_scheduled_as_write_capable() {
        let (mut scheduler, _) = scheduler();
        let rejection = scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Verifier),
                WorkspaceId::parse("wsp_read_only_verifier").unwrap(),
                WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
            ))
            .unwrap_err();
        assert_eq!(rejection, DispatchError::WriteCapabilityInReadOnlyWorkspace);
    }

    #[test]
    fn follow_and_selection_switch_without_changing_scheduler_state() {
        let (mut scheduler, sink) = scheduler();
        let workspace = WorkspaceId::parse("wsp_projection_switch").unwrap();
        let first = scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Explorer),
                workspace.clone(),
                WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
            ))
            .unwrap();
        let second = scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Reviewer),
                workspace,
                WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
            ))
            .unwrap();
        let events = sink.events(&first.run_id).unwrap();
        let mut projection = AgentFollowProjection::rehydrate(&events).unwrap();
        projection.select(second.agent_id.clone()).unwrap();
        projection.follow(first.agent_id.clone()).unwrap();

        assert_eq!(projection.selected(), Some(&second.agent_id));
        assert_eq!(projection.followed(), Some(&first.agent_id));
        assert_eq!(
            scheduler.state(&first.agent_id),
            Some(&DispatchState::Queued)
        );
        assert_eq!(
            scheduler.state(&second.agent_id),
            Some(&DispatchState::Queued)
        );
    }

    #[test]
    fn projection_rehydrates_tree_and_lifecycle_from_events() {
        let (mut scheduler, sink) = scheduler();
        let workspace = WorkspaceId::parse("wsp_projection_rehydrate").unwrap();
        let parent = scheduler
            .dispatch(request(
                AgentKind::built_in(AgentKindName::Planner),
                workspace.clone(),
                WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
            ))
            .unwrap();
        let child_turn = TurnId::parse("trn_projection_child").unwrap();
        let child_cause = record_parent_turn(&sink, &parent, child_turn.clone());
        let child = scheduler
            .dispatch(DispatchRequest {
                objective: "review the plan".into(),
                kind: AgentKind::built_in(AgentKindName::Reviewer),
                parent_agent_id: Some(parent.agent_id.clone()),
                causing_turn_id: Some(child_turn),
                caused_by_event: Some(child_cause.event_id),
                workspace: WorkspaceAssignment {
                    workspace_id: workspace,
                    strategy: WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
                },
            })
            .unwrap();
        scheduler
            .transition(&child, DispatchState::Starting, None)
            .unwrap();
        scheduler
            .transition(&child, DispatchState::Running, None)
            .unwrap();
        scheduler
            .transition(&child, DispatchState::WaitingForAgent, None)
            .unwrap();

        let events = sink.events(&parent.run_id).unwrap();
        let projection = AgentFollowProjection::rehydrate(&events).unwrap();
        assert_eq!(
            projection.children(&parent.agent_id),
            &[child.agent_id.clone()]
        );
        assert_eq!(
            projection.agents()[&child.agent_id].state,
            DispatchState::WaitingForAgent
        );
        assert_eq!(
            projection.agents()[&child.agent_id]
                .causing_turn_id
                .as_ref()
                .map(TurnId::as_str),
            Some("trn_projection_child")
        );
    }
}
