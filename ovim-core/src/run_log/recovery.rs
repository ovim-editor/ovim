use super::{
    AgentId, AgentLifecycleEvent, AgentLifecycleState, BranchId, EventActor, EventEnvelope,
    EventId, EventKind, NewRunEvent, OperationId, RunEventSink, RunId, RunLifecycleEvent,
    RunLifecycleState, RunLogError, ToolOutcome, ToolResultEvent, TurnId, TurnLifecycleEvent,
    TurnLifecycleState, WorkspaceId,
};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;

/// Storage-neutral fold over durable run history.
#[derive(Clone)]
pub struct RecoveryPlanner {
    sink: Arc<dyn RunEventSink>,
}

impl RecoveryPlanner {
    pub fn new(sink: Arc<dyn RunEventSink>) -> Self {
        Self { sink }
    }

    pub fn plan(&self, run_id: &RunId) -> Result<RecoveryPlan, RecoveryError> {
        let events = self.sink.events(run_id)?;
        RecoveryPlan::from_events(run_id.clone(), events)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryPlan {
    pub run_id: RunId,
    pub observed_last_sequence: Option<u64>,
    pub pending_tools: Vec<PendingToolRecovery>,
    pub active_turns: Vec<ActiveTurnRecovery>,
    pub active_agents: Vec<ActiveAgentRecovery>,
    /// Latest nonterminal run lifecycle, including `Recovered` (which query
    /// semantics consider active). `run_needs_recovery` distinguishes an
    /// already-recovered idle run from a crashed active run.
    pub active_run: Option<ActiveRunRecovery>,
    pub run_needs_recovery: bool,
    branch_tips: BTreeMap<Option<BranchId>, EventId>,
    last_event_id: Option<EventId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingToolRecovery {
    pub operation_id: OperationId,
    pub phase: PendingToolPhase,
    pub tool_name: String,
    pub provider_call_id: Option<String>,
    pub actor: EventActor,
    pub agent_id: Option<AgentId>,
    pub turn_id: Option<TurnId>,
    pub workspace_id: Option<WorkspaceId>,
    pub branch_id: Option<BranchId>,
    observed_sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingToolPhase {
    /// An intent is durable, but no effect-start boundary was crossed.
    IntentOnly,
    /// Execution crossed the effect-start boundary; its outcome is unknowable.
    Started,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveTurnRecovery {
    pub turn_id: TurnId,
    pub actor: EventActor,
    pub agent_id: Option<AgentId>,
    pub workspace_id: Option<WorkspaceId>,
    pub branch_id: Option<BranchId>,
    observed_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveAgentRecovery {
    pub agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub kind: String,
    pub objective: Option<String>,
    pub actor: EventActor,
    pub workspace_id: Option<WorkspaceId>,
    pub branch_id: Option<BranchId>,
    observed_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveRunRecovery {
    pub state: RunLifecycleState,
    pub objective: Option<String>,
    pub actor: EventActor,
    pub workspace_id: Option<WorkspaceId>,
    pub branch_id: Option<BranchId>,
    observed_sequence: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryReport {
    pub appended: Vec<EventEnvelope>,
}

impl RecoveryPlan {
    pub fn from_events(
        run_id: RunId,
        mut events: Vec<EventEnvelope>,
    ) -> Result<Self, RecoveryError> {
        events.sort_by_key(|event| event.sequence);
        if events.iter().any(|event| event.run_id != run_id) {
            return Err(RecoveryError::MixedRuns(run_id));
        }

        let observed_last_sequence = events.last().map(|event| event.sequence);
        let last_event_id = events.last().map(|event| event.event_id.clone());
        let mut branch_tips = BTreeMap::new();
        let mut tools: HashMap<OperationId, ToolFold> = HashMap::new();
        let mut turns: HashMap<TurnId, ActiveTurnRecovery> = HashMap::new();
        let mut agents: HashMap<AgentId, ActiveAgentRecovery> = HashMap::new();
        let mut active_run = None;

        for event in events {
            branch_tips.insert(event.branch_id.clone(), event.event_id.clone());
            match &event.kind {
                EventKind::ToolIntent(intent) => {
                    let operation_id = event
                        .operation_id
                        .clone()
                        .ok_or(RecoveryError::MissingOperationId(event.event_id.clone()))?;
                    if tools.contains_key(&operation_id) {
                        return Err(RecoveryError::DuplicateToolIntent(operation_id));
                    }
                    tools.insert(
                        operation_id,
                        ToolFold {
                            pending: pending_tool(&event, intent.tool_name.clone()),
                            terminal: false,
                        },
                    );
                }
                EventKind::ToolStarted(started) => {
                    let operation_id = event
                        .operation_id
                        .clone()
                        .ok_or(RecoveryError::MissingOperationId(event.event_id.clone()))?;
                    let tool = tools
                        .get_mut(&operation_id)
                        .ok_or_else(|| RecoveryError::StartedWithoutIntent(operation_id.clone()))?;
                    if !tool.terminal {
                        tool.pending.phase = PendingToolPhase::Started;
                        tool.pending.tool_name = started.tool_name.clone();
                        copy_tool_identity(&mut tool.pending, &event);
                    }
                }
                EventKind::ToolResult(_) => {
                    let operation_id = event
                        .operation_id
                        .clone()
                        .ok_or(RecoveryError::MissingOperationId(event.event_id.clone()))?;
                    let tool = tools
                        .get_mut(&operation_id)
                        .ok_or_else(|| RecoveryError::ResultWithoutIntent(operation_id.clone()))?;
                    tool.terminal = true;
                }
                EventKind::TurnLifecycle(lifecycle) => {
                    let turn_id = event
                        .turn_id
                        .clone()
                        .ok_or(RecoveryError::MissingTurnId(event.event_id.clone()))?;
                    if turn_is_active(&lifecycle.state) {
                        turns.insert(turn_id.clone(), active_turn(&event, turn_id));
                    } else {
                        turns.remove(&turn_id);
                    }
                }
                EventKind::AgentLifecycle(lifecycle) => {
                    if agent_is_active(&lifecycle.state) {
                        agents.insert(lifecycle.agent_id.clone(), active_agent(&event, lifecycle));
                    } else {
                        agents.remove(&lifecycle.agent_id);
                    }
                }
                EventKind::RunLifecycle(lifecycle) => {
                    if run_is_active(&lifecycle.state) {
                        active_run = Some(ActiveRunRecovery {
                            state: lifecycle.state.clone(),
                            objective: lifecycle.objective.clone(),
                            actor: event.actor.clone(),
                            workspace_id: event.workspace_id.clone(),
                            branch_id: event.branch_id.clone(),
                            observed_sequence: event.sequence,
                        });
                    } else {
                        active_run = None;
                    }
                }
                _ => {}
            }
        }

        let mut pending_tools: Vec<_> = tools
            .into_values()
            .filter(|tool| !tool.terminal)
            .map(|tool| tool.pending)
            .collect();
        pending_tools.sort_by_key(|tool| tool.observed_sequence);
        let mut active_turns: Vec<_> = turns.into_values().collect();
        active_turns.sort_by_key(|turn| turn.observed_sequence);
        let mut active_agents: Vec<_> = agents.into_values().collect();
        active_agents.sort_by_key(|agent| agent.observed_sequence);

        let non_run_work =
            !pending_tools.is_empty() || !active_turns.is_empty() || !active_agents.is_empty();
        let run_needs_recovery = active_run.as_ref().is_some_and(|run| {
            run.state != RunLifecycleState::Recovered
                || non_run_work
                    && observed_last_sequence
                        .is_some_and(|sequence| sequence > run.observed_sequence)
        });

        Ok(Self {
            run_id,
            observed_last_sequence,
            pending_tools,
            active_turns,
            active_agents,
            active_run,
            run_needs_recovery,
            branch_tips,
            last_event_id,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.pending_tools.is_empty()
            && self.active_turns.is_empty()
            && self.active_agents.is_empty()
            && !self.run_needs_recovery
    }
}

struct ToolFold {
    pending: PendingToolRecovery,
    terminal: bool,
}

/// Appends recovery facts; it never rewrites or deletes the crashed history.
/// `lease_authorized` must only be true after the caller has established that
/// no live owner can still append to this run.
pub fn apply_recovery(
    plan: &RecoveryPlan,
    sink: &dyn RunEventSink,
    lease_authorized: bool,
) -> Result<RecoveryReport, RecoveryError> {
    if !lease_authorized {
        return Err(RecoveryError::StaleOwnershipNotConfirmed);
    }
    let actual = sink.last_sequence(&plan.run_id)?;
    if actual != plan.observed_last_sequence {
        return Err(RecoveryError::PlanStale {
            expected: plan.observed_last_sequence,
            actual,
        });
    }
    if plan.is_empty() {
        return Ok(RecoveryReport { appended: vec![] });
    }

    let mut branch_tips = plan.branch_tips.clone();
    let mut appended = Vec::new();

    for tool in &plan.pending_tools {
        let outcome = match tool.phase {
            PendingToolPhase::IntentOnly => ToolOutcome::Interrupted,
            PendingToolPhase::Started => ToolOutcome::UnknownAfterCrash,
        };
        let summary = match tool.phase {
            PendingToolPhase::IntentOnly => format!(
                "{} was interrupted during crash recovery before its effect began",
                tool.tool_name
            ),
            PendingToolPhase::Started => format!(
                "{} crossed its effect-start boundary before the crash; outcome is unknown",
                tool.tool_name
            ),
        };
        let event = append_recovery_event(
            sink,
            &plan.run_id,
            &mut branch_tips,
            tool.branch_id.clone(),
            tool.actor.clone(),
            tool.agent_id.clone(),
            tool.turn_id.clone(),
            tool.workspace_id.clone(),
            Some(tool.operation_id.clone()),
            tool.provider_call_id.clone(),
            EventKind::ToolResult(ToolResultEvent {
                outcome,
                summary: Some(summary),
                result: None,
            }),
        )?;
        appended.push(event);
    }

    for turn in &plan.active_turns {
        let event = append_recovery_event(
            sink,
            &plan.run_id,
            &mut branch_tips,
            turn.branch_id.clone(),
            turn.actor.clone(),
            turn.agent_id.clone(),
            Some(turn.turn_id.clone()),
            turn.workspace_id.clone(),
            None,
            None,
            EventKind::TurnLifecycle(TurnLifecycleEvent {
                state: TurnLifecycleState::Interrupted,
                detail: Some("active turn interrupted during stale-run crash recovery".into()),
            }),
        )?;
        appended.push(event);
    }

    for agent in &plan.active_agents {
        let event = append_recovery_event(
            sink,
            &plan.run_id,
            &mut branch_tips,
            agent.branch_id.clone(),
            agent.actor.clone(),
            Some(agent.agent_id.clone()),
            None,
            agent.workspace_id.clone(),
            None,
            None,
            EventKind::AgentLifecycle(AgentLifecycleEvent {
                agent_id: agent.agent_id.clone(),
                parent_agent_id: agent.parent_agent_id.clone(),
                state: AgentLifecycleState::Interrupted,
                kind: agent.kind.clone(),
                objective: agent.objective.clone(),
                detail: Some("active agent interrupted during stale-run crash recovery".into()),
                dispatch_spec: None,
            }),
        )?;
        appended.push(event);
    }

    if plan.run_needs_recovery {
        let run = plan
            .active_run
            .as_ref()
            .expect("run_needs_recovery implies an active run");
        // A run marker has one parent, so preserve its own trajectory rather
        // than creating a cross-branch causal edge. Per-run sequence still
        // records that all branch-local terminalization happened first.
        let cause = branch_tips
            .get(&run.branch_id)
            .cloned()
            .or_else(|| plan.last_event_id.clone());
        let mut event = NewRunEvent::new(
            plan.run_id.clone(),
            run.actor.clone(),
            EventKind::RunLifecycle(RunLifecycleEvent {
                state: RunLifecycleState::Recovered,
                objective: run.objective.clone(),
                detail: Some(
                    "stale run recovered append-only; interrupted work was terminalized".into(),
                ),
                creation: None,
            }),
        );
        event.caused_by = cause;
        event.workspace_id = run.workspace_id.clone();
        event.branch_id = run.branch_id.clone();
        let envelope = sink.append(event)?;
        appended.push(envelope);
    }

    Ok(RecoveryReport { appended })
}

#[allow(clippy::too_many_arguments)]
fn append_recovery_event(
    sink: &dyn RunEventSink,
    run_id: &RunId,
    branch_tips: &mut BTreeMap<Option<BranchId>, EventId>,
    branch_id: Option<BranchId>,
    actor: EventActor,
    agent_id: Option<AgentId>,
    turn_id: Option<TurnId>,
    workspace_id: Option<WorkspaceId>,
    operation_id: Option<OperationId>,
    provider_call_id: Option<String>,
    kind: EventKind,
) -> Result<EventEnvelope, RunLogError> {
    let mut event = NewRunEvent::new(run_id.clone(), actor, kind);
    event.caused_by = branch_tips.get(&branch_id).cloned();
    event.agent_id = agent_id;
    event.turn_id = turn_id;
    event.workspace_id = workspace_id;
    event.branch_id = branch_id.clone();
    event.operation_id = operation_id;
    event.provider_call_id = provider_call_id;
    let envelope = sink.append(event)?;
    branch_tips.insert(branch_id, envelope.event_id.clone());
    Ok(envelope)
}

fn pending_tool(event: &EventEnvelope, tool_name: String) -> PendingToolRecovery {
    PendingToolRecovery {
        operation_id: event.operation_id.clone().unwrap(),
        phase: PendingToolPhase::IntentOnly,
        tool_name,
        provider_call_id: event.provider_call_id.clone(),
        actor: event.actor.clone(),
        agent_id: event.agent_id.clone(),
        turn_id: event.turn_id.clone(),
        workspace_id: event.workspace_id.clone(),
        branch_id: event.branch_id.clone(),
        observed_sequence: event.sequence,
    }
}

fn copy_tool_identity(tool: &mut PendingToolRecovery, event: &EventEnvelope) {
    tool.provider_call_id = event.provider_call_id.clone();
    tool.actor = event.actor.clone();
    tool.agent_id = event.agent_id.clone();
    tool.turn_id = event.turn_id.clone();
    tool.workspace_id = event.workspace_id.clone();
    tool.branch_id = event.branch_id.clone();
    tool.observed_sequence = event.sequence;
}

fn active_turn(event: &EventEnvelope, turn_id: TurnId) -> ActiveTurnRecovery {
    ActiveTurnRecovery {
        turn_id,
        actor: event.actor.clone(),
        agent_id: event.agent_id.clone(),
        workspace_id: event.workspace_id.clone(),
        branch_id: event.branch_id.clone(),
        observed_sequence: event.sequence,
    }
}

fn active_agent(event: &EventEnvelope, lifecycle: &AgentLifecycleEvent) -> ActiveAgentRecovery {
    ActiveAgentRecovery {
        agent_id: lifecycle.agent_id.clone(),
        parent_agent_id: lifecycle.parent_agent_id.clone(),
        kind: lifecycle.kind.clone(),
        objective: lifecycle.objective.clone(),
        actor: event.actor.clone(),
        workspace_id: event.workspace_id.clone(),
        branch_id: event.branch_id.clone(),
        observed_sequence: event.sequence,
    }
}

fn turn_is_active(state: &TurnLifecycleState) -> bool {
    matches!(
        state,
        TurnLifecycleState::Started | TurnLifecycleState::Steered
    )
}

fn agent_is_active(state: &AgentLifecycleState) -> bool {
    matches!(
        state,
        AgentLifecycleState::Created
            | AgentLifecycleState::Dispatched
            | AgentLifecycleState::Started
            | AgentLifecycleState::Queued
            | AgentLifecycleState::Starting
            | AgentLifecycleState::Running
            | AgentLifecycleState::WaitingForAgent
            | AgentLifecycleState::WaitingForTool
            | AgentLifecycleState::WaitingForUser
            | AgentLifecycleState::Waiting
            | AgentLifecycleState::Blocked
    )
}

fn run_is_active(state: &RunLifecycleState) -> bool {
    matches!(
        state,
        RunLifecycleState::Created
            | RunLifecycleState::ObjectiveUpdated
            | RunLifecycleState::Recovered
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryError {
    Store(RunLogError),
    StaleOwnershipNotConfirmed,
    PlanStale {
        expected: Option<u64>,
        actual: Option<u64>,
    },
    MixedRuns(RunId),
    MissingOperationId(EventId),
    DuplicateToolIntent(OperationId),
    StartedWithoutIntent(OperationId),
    ResultWithoutIntent(OperationId),
    MissingTurnId(EventId),
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(formatter),
            Self::StaleOwnershipNotConfirmed => {
                formatter.write_str("run recovery requires caller-confirmed stale ownership")
            }
            Self::PlanStale { expected, actual } => write!(
                formatter,
                "recovery plan is stale: expected last sequence {expected:?}, found {actual:?}"
            ),
            Self::MixedRuns(run_id) => {
                write!(formatter, "recovery input contains events outside {run_id}")
            }
            Self::MissingOperationId(event_id) => write!(
                formatter,
                "tool event {event_id} has no operation identifier"
            ),
            Self::DuplicateToolIntent(operation_id) => write!(
                formatter,
                "operation {operation_id} has more than one intent"
            ),
            Self::StartedWithoutIntent(operation_id) => write!(
                formatter,
                "operation {operation_id} started without a durable intent"
            ),
            Self::ResultWithoutIntent(operation_id) => write!(
                formatter,
                "operation {operation_id} ended without a durable intent"
            ),
            Self::MissingTurnId(event_id) => write!(
                formatter,
                "turn lifecycle event {event_id} has no turn identifier"
            ),
        }
    }
}

impl std::error::Error for RecoveryError {}

impl From<RunLogError> for RecoveryError {
    fn from(error: RunLogError) -> Self {
        Self::Store(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::{AgentRuntime, AgentSpec, BranchLocator};
    use crate::run_log::{InMemoryRunEventSink, ToolSideEffect};
    use serde_json::json;

    fn crashed_tool(started: bool) -> (Arc<InMemoryRunEventSink>, RunId, OperationId) {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let mut runtime = AgentRuntime::with_sink(sink.clone());
        let turn = runtime
            .begin_turn(
                "chat",
                BranchLocator("main".into()),
                "do it",
                AgentSpec::chat(),
            )
            .unwrap();
        let tool = runtime
            .record_tool_intent(
                &turn,
                "shell",
                json!({"command": "cargo test"}),
                ToolSideEffect::Mutation,
                Some("provider-call".into()),
            )
            .unwrap();
        if started {
            runtime.start_tool(&turn, &tool).unwrap();
        }
        (sink, turn.run_id, tool.operation_id)
    }

    #[test]
    fn crash_before_start_is_interrupted_without_claiming_an_effect() {
        let (sink, run_id, operation_id) = crashed_tool(false);
        let plan = RecoveryPlanner::new(sink.clone()).plan(&run_id).unwrap();
        assert_eq!(plan.pending_tools[0].phase, PendingToolPhase::IntentOnly);

        apply_recovery(&plan, sink.as_ref(), true).unwrap();
        let result = sink
            .events(&run_id)
            .unwrap()
            .into_iter()
            .find(|event| {
                event.operation_id.as_ref() == Some(&operation_id)
                    && matches!(event.kind, EventKind::ToolResult(_))
            })
            .unwrap();
        assert!(matches!(
            result.kind,
            EventKind::ToolResult(ToolResultEvent {
                outcome: ToolOutcome::Interrupted,
                ..
            })
        ));
    }

    #[test]
    fn crash_after_start_records_unknown_outcome() {
        let (sink, run_id, _) = crashed_tool(true);
        let plan = RecoveryPlanner::new(sink.clone()).plan(&run_id).unwrap();
        assert_eq!(plan.pending_tools[0].phase, PendingToolPhase::Started);
        let report = apply_recovery(&plan, sink.as_ref(), true).unwrap();
        assert!(report.appended.iter().any(|event| matches!(
            event.kind,
            EventKind::ToolResult(ToolResultEvent {
                outcome: ToolOutcome::UnknownAfterCrash,
                ..
            })
        )));
    }

    #[test]
    fn completed_run_is_untouched() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let run_id = RunId::new();
        let created = sink
            .append(NewRunEvent::new(
                run_id.clone(),
                EventActor::User,
                EventKind::RunLifecycle(RunLifecycleEvent {
                    state: RunLifecycleState::Created,
                    objective: None,
                    detail: None,
                    creation: None,
                }),
            ))
            .unwrap();
        sink.append(
            NewRunEvent::new(
                run_id.clone(),
                EventActor::User,
                EventKind::RunLifecycle(RunLifecycleEvent {
                    state: RunLifecycleState::Completed,
                    objective: None,
                    detail: None,
                    creation: None,
                }),
            )
            .caused_by(created.event_id),
        )
        .unwrap();
        let plan = RecoveryPlanner::new(sink.clone()).plan(&run_id).unwrap();
        assert!(plan.is_empty());
        assert!(apply_recovery(&plan, sink.as_ref(), true)
            .unwrap()
            .appended
            .is_empty());
        assert_eq!(sink.events(&run_id).unwrap().len(), 2);
    }

    #[test]
    fn multiple_operations_are_terminalized_before_lifecycles() {
        let (sink, run_id, _) = crashed_tool(false);
        // Add a second independent crashed operation directly while retaining
        // the first operation's durable identities.
        let events = sink.events(&run_id).unwrap();
        let template = events
            .iter()
            .find(|event| matches!(event.kind, EventKind::ToolIntent(_)))
            .unwrap();
        let second = OperationId::new();
        let mut event = NewRunEvent::new(
            run_id.clone(),
            template.actor.clone(),
            EventKind::ToolIntent(super::super::ToolIntentEvent {
                tool_name: "write".into(),
                arguments: json!({}),
                side_effect: ToolSideEffect::Mutation,
            }),
        );
        event.caused_by = events.last().map(|event| event.event_id.clone());
        event.operation_id = Some(second);
        event.agent_id = template.agent_id.clone();
        event.turn_id = template.turn_id.clone();
        event.workspace_id = template.workspace_id.clone();
        event.branch_id = template.branch_id.clone();
        sink.append(event).unwrap();

        let plan = RecoveryPlanner::new(sink.clone()).plan(&run_id).unwrap();
        assert_eq!(plan.pending_tools.len(), 2);
        let report = apply_recovery(&plan, sink.as_ref(), true).unwrap();
        let first_lifecycle = report
            .appended
            .iter()
            .position(|event| !matches!(event.kind, EventKind::ToolResult(_)))
            .unwrap();
        assert_eq!(first_lifecycle, 2);
    }

    #[test]
    fn applying_without_stale_lease_authorization_writes_nothing() {
        let (sink, run_id, _) = crashed_tool(false);
        let plan = RecoveryPlanner::new(sink.clone()).plan(&run_id).unwrap();
        let before = sink.events(&run_id).unwrap().len();
        assert_eq!(
            apply_recovery(&plan, sink.as_ref(), false).unwrap_err(),
            RecoveryError::StaleOwnershipNotConfirmed
        );
        assert_eq!(sink.events(&run_id).unwrap().len(), before);
    }

    #[test]
    fn recovery_is_idempotent_when_replanned() {
        let (sink, run_id, _) = crashed_tool(true);
        let first = RecoveryPlanner::new(sink.clone()).plan(&run_id).unwrap();
        apply_recovery(&first, sink.as_ref(), true).unwrap();
        let second = RecoveryPlanner::new(sink.clone()).plan(&run_id).unwrap();
        assert!(second.is_empty());
        assert!(apply_recovery(&second, sink.as_ref(), true)
            .unwrap()
            .appended
            .is_empty());
    }
}
