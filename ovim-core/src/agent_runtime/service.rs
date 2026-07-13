use crate::run_log::{
    AgentId, AgentLifecycleEvent, AgentLifecycleState, BranchId, BranchLifecycleEvent,
    BranchLifecycleState, ConversationId, EventActor, EventEnvelope, EventId, EventKind,
    InMemoryRunEventSink, MessageEvent, MessageRole, NewRunEvent, OperationId, RunCreationMetadata,
    RunEventSink, RunId, RunLifecycleEvent, RunLifecycleState, RunLogError, ToolIntentEvent,
    ToolOutcome, ToolResultEvent, ToolSideEffect, ToolStartedEvent, TurnId, TurnLifecycleEvent,
    TurnLifecycleState, WorkspaceId,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConversationLocator(pub String);

impl From<&str> for ConversationLocator {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<String> for ConversationLocator {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BranchLocator(pub String);

impl From<&str> for BranchLocator {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<String> for BranchLocator {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSpec {
    /// Provider-independent role, such as `chat`, `reviewer`, or `implementer`.
    pub kind: String,
    pub objective: Option<String>,
}

impl AgentSpec {
    pub fn chat() -> Self {
        Self {
            kind: "chat".into(),
            objective: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationRef {
    pub conversation_id: ConversationId,
    pub run_id: RunId,
    pub root_agent_id: AgentId,
    pub workspace_id: WorkspaceId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchRef {
    pub branch_id: BranchId,
    pub parent_branch_id: Option<BranchId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingTurnRef {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub workspace_id: WorkspaceId,
    pub conversation_id: ConversationId,
    pub branch_id: BranchId,
    pub turn_id: TurnId,
    /// The `turn.started` envelope; its cause is the initiating user message.
    pub initiating_event: EventEnvelope,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingToolRef {
    pub operation_id: OperationId,
    pub provider_call_id: Option<String>,
    pub tool_name: String,
    pub intent_event: EventEnvelope,
    turn_id: TurnId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolState {
    Intended,
    Started,
    Terminal,
}

struct ToolRecord {
    turn_id: TurnId,
    state: ToolState,
    tool_name: String,
    provider_call_id: Option<String>,
}

struct BranchState {
    reference: BranchRef,
    /// Causality is branch-local: switching branches must resume that branch's
    /// trajectory rather than linking to whichever branch ran most recently.
    last_event: EventId,
}

struct ConversationState {
    reference: ConversationRef,
    branches: HashMap<BranchLocator, BranchState>,
    selected_branch: BranchLocator,
    active_turn: Option<TurnId>,
    terminal_turns: HashSet<TurnId>,
    tools: HashMap<OperationId, ToolRecord>,
}

#[derive(Debug)]
pub enum AgentRuntimeError {
    RunLog(RunLogError),
    UnknownConversation,
    UnknownBranch,
    BranchAlreadyExists,
    TurnAlreadyActive,
    TurnAlreadyTerminal,
    EmptyUserMessage,
    NoActiveTurn,
    WrongActiveBranch,
    WrongActiveTurn,
    UnknownOperation,
    WrongToolTurn,
    InvalidToolState,
    OutstandingToolOperations,
}

impl fmt::Display for AgentRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunLog(error) => write!(f, "run log: {error}"),
            Self::UnknownConversation => f.write_str("unknown conversation"),
            Self::UnknownBranch => f.write_str("unknown branch"),
            Self::BranchAlreadyExists => f.write_str("branch already exists"),
            Self::TurnAlreadyActive => f.write_str("a turn is already active"),
            Self::TurnAlreadyTerminal => f.write_str("turn already has a terminal event"),
            Self::EmptyUserMessage => f.write_str("user message is empty"),
            Self::NoActiveTurn => f.write_str("no turn is active"),
            Self::WrongActiveBranch => f.write_str("turn does not belong to the active branch"),
            Self::WrongActiveTurn => f.write_str("turn is not the active turn"),
            Self::UnknownOperation => f.write_str("unknown tool operation"),
            Self::WrongToolTurn => f.write_str("tool operation belongs to another turn"),
            Self::InvalidToolState => f.write_str("tool operation is in the wrong state"),
            Self::OutstandingToolOperations => f.write_str("turn has nonterminal tool operations"),
        }
    }
}

impl std::error::Error for AgentRuntimeError {}

impl From<RunLogError> for AgentRuntimeError {
    fn from(value: RunLogError) -> Self {
        Self::RunLog(value)
    }
}

pub struct AgentRuntime {
    sink: Arc<dyn RunEventSink>,
    workspace_id: WorkspaceId,
    run_creation: RunCreationMetadata,
    conversations: HashMap<ConversationLocator, ConversationState>,
}

impl Default for AgentRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRuntime {
    pub fn new() -> Self {
        Self::with_sink(Arc::new(InMemoryRunEventSink::new()))
    }

    pub fn with_sink(sink: Arc<dyn RunEventSink>) -> Self {
        Self::with_sink_and_run_metadata(sink, RunCreationMetadata::default())
    }

    pub fn with_sink_and_run_metadata(
        sink: Arc<dyn RunEventSink>,
        run_creation: RunCreationMetadata,
    ) -> Self {
        Self {
            sink,
            workspace_id: WorkspaceId::new(),
            run_creation,
            conversations: HashMap::new(),
        }
    }

    pub fn begin_turn(
        &mut self,
        locator: impl Into<ConversationLocator>,
        branch: impl Into<BranchLocator>,
        user_message: impl Into<String>,
        spec: AgentSpec,
    ) -> Result<PendingTurnRef, AgentRuntimeError> {
        let user_message = user_message.into();
        if user_message.trim().is_empty() {
            return Err(AgentRuntimeError::EmptyUserMessage);
        }
        let locator = locator.into();
        let branch = branch.into();
        if !self.conversations.contains_key(&locator) {
            self.create_conversation(locator.clone(), branch.clone(), &spec)?;
        }
        let state = self.conversations.get_mut(&locator).unwrap();
        if state.active_turn.is_some() {
            return Err(AgentRuntimeError::TurnAlreadyActive);
        }
        if state.selected_branch != branch {
            return Err(AgentRuntimeError::WrongActiveBranch);
        }
        let (branch_id, branch_tip) = state
            .branches
            .get(&branch)
            .map(|branch| {
                (
                    branch.reference.branch_id.clone(),
                    branch.last_event.clone(),
                )
            })
            .ok_or(AgentRuntimeError::UnknownBranch)?;
        let turn_id = TurnId::new();
        let message = append_for(
            &self.sink,
            &state.reference,
            Some(turn_id.clone()),
            Some(branch_tip),
            EventActor::User,
            EventKind::Message(MessageEvent {
                role: MessageRole::User,
                content: user_message,
            }),
            None,
            None,
            Some(branch_id.clone()),
        )?;
        let started = append_for(
            &self.sink,
            &state.reference,
            Some(turn_id.clone()),
            Some(message.event_id),
            EventActor::Agent(state.reference.root_agent_id.clone()),
            EventKind::TurnLifecycle(TurnLifecycleEvent {
                state: TurnLifecycleState::Started,
                detail: None,
            }),
            None,
            None,
            Some(branch_id.clone()),
        )?;
        state.branches.get_mut(&branch).unwrap().last_event = started.event_id.clone();
        state.active_turn = Some(turn_id.clone());
        Ok(PendingTurnRef {
            run_id: state.reference.run_id.clone(),
            agent_id: state.reference.root_agent_id.clone(),
            workspace_id: state.reference.workspace_id.clone(),
            conversation_id: state.reference.conversation_id.clone(),
            branch_id,
            turn_id,
            initiating_event: started,
        })
    }

    fn create_conversation(
        &mut self,
        locator: ConversationLocator,
        branch: BranchLocator,
        spec: &AgentSpec,
    ) -> Result<(), AgentRuntimeError> {
        let reference = ConversationRef {
            conversation_id: ConversationId::new(),
            run_id: RunId::new(),
            root_agent_id: AgentId::new(),
            workspace_id: self.workspace_id.clone(),
        };
        let initial_branch = BranchRef {
            branch_id: BranchId::new(),
            parent_branch_id: None,
        };
        let created = append_for(
            &self.sink,
            &reference,
            None,
            None,
            EventActor::User,
            EventKind::RunLifecycle(RunLifecycleEvent {
                state: RunLifecycleState::Created,
                objective: spec.objective.clone(),
                detail: None,
                creation: Some(RunCreationMetadata {
                    initial_branch_id: Some(initial_branch.branch_id.clone()),
                    ..self.run_creation.clone()
                }),
            }),
            None,
            None,
            Some(initial_branch.branch_id.clone()),
        )?;
        let dispatched = append_for(
            &self.sink,
            &reference,
            None,
            Some(created.event_id),
            EventActor::System("agent_runtime".into()),
            agent_event(&reference, spec, AgentLifecycleState::Dispatched),
            None,
            None,
            Some(initial_branch.branch_id.clone()),
        )?;
        let started = append_for(
            &self.sink,
            &reference,
            None,
            Some(dispatched.event_id),
            EventActor::Agent(reference.root_agent_id.clone()),
            agent_event(&reference, spec, AgentLifecycleState::Started),
            None,
            None,
            Some(initial_branch.branch_id.clone()),
        )?;
        let branch_created = append_for(
            &self.sink,
            &reference,
            None,
            Some(started.event_id),
            EventActor::System("agent_runtime".into()),
            EventKind::BranchLifecycle(BranchLifecycleEvent {
                state: BranchLifecycleState::Created,
                branch_id: initial_branch.branch_id.clone(),
                parent_branch_id: None,
                forked_at: None,
                label: Some(branch.0.clone()),
            }),
            None,
            None,
            Some(initial_branch.branch_id.clone()),
        )?;
        let mut branches = HashMap::new();
        branches.insert(
            branch.clone(),
            BranchState {
                reference: initial_branch,
                last_event: branch_created.event_id,
            },
        );
        self.conversations.insert(
            locator,
            ConversationState {
                reference,
                branches,
                selected_branch: branch,
                active_turn: None,
                terminal_turns: HashSet::new(),
                tools: HashMap::new(),
            },
        );
        Ok(())
    }

    pub fn fork_branch(
        &mut self,
        locator: &ConversationLocator,
        source: &BranchLocator,
        target: BranchLocator,
    ) -> Result<BranchRef, AgentRuntimeError> {
        let source_tip = self
            .conversations
            .get(locator)
            .ok_or(AgentRuntimeError::UnknownConversation)?
            .branches
            .get(source)
            .ok_or(AgentRuntimeError::UnknownBranch)?
            .last_event
            .clone();
        self.fork_branch_at(locator, source, target, source_tip)
    }

    /// Fork a trajectory from an earlier recorded event rather than the
    /// source branch's current tip. Conversation UIs use this when editing a
    /// previous user message.
    pub fn fork_branch_at(
        &mut self,
        locator: &ConversationLocator,
        source: &BranchLocator,
        target: BranchLocator,
        causal_event: EventId,
    ) -> Result<BranchRef, AgentRuntimeError> {
        let state = self
            .conversations
            .get_mut(locator)
            .ok_or(AgentRuntimeError::UnknownConversation)?;
        if state.active_turn.is_some() {
            return Err(AgentRuntimeError::TurnAlreadyActive);
        }
        if state.branches.contains_key(&target) {
            return Err(AgentRuntimeError::BranchAlreadyExists);
        }
        let source = state
            .branches
            .get(source)
            .ok_or(AgentRuntimeError::UnknownBranch)?;
        let parent_branch_id = source.reference.branch_id.clone();
        let branch = BranchRef {
            branch_id: BranchId::new(),
            parent_branch_id: Some(parent_branch_id.clone()),
        };
        let forked = append_for(
            &self.sink,
            &state.reference,
            None,
            Some(causal_event.clone()),
            EventActor::System("agent_runtime".into()),
            EventKind::BranchLifecycle(BranchLifecycleEvent {
                state: BranchLifecycleState::Forked,
                branch_id: branch.branch_id.clone(),
                parent_branch_id: Some(parent_branch_id),
                forked_at: Some(causal_event),
                label: Some(target.0.clone()),
            }),
            None,
            None,
            Some(branch.branch_id.clone()),
        )?;
        state.branches.insert(
            target,
            BranchState {
                reference: branch.clone(),
                last_event: forked.event_id,
            },
        );
        Ok(branch)
    }

    pub fn select_branch(
        &mut self,
        locator: &ConversationLocator,
        branch: &BranchLocator,
    ) -> Result<BranchRef, AgentRuntimeError> {
        let state = self
            .conversations
            .get_mut(locator)
            .ok_or(AgentRuntimeError::UnknownConversation)?;
        if state.active_turn.is_some() {
            return Err(AgentRuntimeError::TurnAlreadyActive);
        }
        let selected = state
            .branches
            .get(branch)
            .map(|branch| branch.reference.clone())
            .ok_or(AgentRuntimeError::UnknownBranch)?;
        let previous = state.branches[branch].last_event.clone();
        let selected_event = append_for(
            &self.sink,
            &state.reference,
            None,
            Some(previous),
            EventActor::System("agent_runtime".into()),
            EventKind::BranchLifecycle(BranchLifecycleEvent {
                state: BranchLifecycleState::Selected,
                branch_id: selected.branch_id.clone(),
                parent_branch_id: selected.parent_branch_id.clone(),
                forked_at: None,
                label: Some(branch.0.clone()),
            }),
            None,
            None,
            Some(selected.branch_id.clone()),
        )?;
        state.branches.get_mut(branch).unwrap().last_event = selected_event.event_id;
        state.selected_branch = branch.clone();
        Ok(selected)
    }

    pub fn conversation(&self, locator: &ConversationLocator) -> Option<&ConversationRef> {
        self.conversations
            .get(locator)
            .map(|state| &state.reference)
    }

    /// Transient editor locator for the currently selected durable branch.
    pub fn selected_branch(
        &self,
        locator: &ConversationLocator,
    ) -> Option<(&BranchLocator, &BranchRef)> {
        let state = self.conversations.get(locator)?;
        let branch = state.branches.get(&state.selected_branch)?;
        Some((&state.selected_branch, &branch.reference))
    }

    pub fn selected_branch_tip(&self, locator: &ConversationLocator) -> Option<&EventId> {
        let state = self.conversations.get(locator)?;
        state
            .branches
            .get(&state.selected_branch)
            .map(|branch| &branch.last_event)
    }

    pub fn append_reasoning_summary(
        &mut self,
        turn: &PendingTurnRef,
        content: impl Into<String>,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        self.append_message(turn, MessageRole::ReasoningSummary, content.into())
    }

    pub fn append_agent_message(
        &mut self,
        turn: &PendingTurnRef,
        content: impl Into<String>,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        self.append_message(turn, MessageRole::Agent, content.into())
    }

    fn append_message(
        &mut self,
        turn: &PendingTurnRef,
        role: MessageRole,
        content: String,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        let (sink, state) = self.active_state(turn)?;
        let previous = state.branches[&state.selected_branch].last_event.clone();
        let event = append_for(
            sink,
            &state.reference,
            Some(turn.turn_id.clone()),
            Some(previous),
            EventActor::Agent(turn.agent_id.clone()),
            EventKind::Message(MessageEvent { role, content }),
            None,
            None,
            Some(turn.branch_id.clone()),
        )?;
        state
            .branches
            .get_mut(&state.selected_branch)
            .unwrap()
            .last_event = event.event_id.clone();
        Ok(event)
    }

    pub fn record_tool_intent(
        &mut self,
        turn: &PendingTurnRef,
        tool_name: impl Into<String>,
        arguments: Value,
        side_effect: ToolSideEffect,
        provider_call_id: Option<String>,
    ) -> Result<PendingToolRef, AgentRuntimeError> {
        let operation_id = OperationId::new();
        let tool_name = tool_name.into();
        let (sink, state) = self.active_state(turn)?;
        let previous = state.branches[&state.selected_branch].last_event.clone();
        let event = append_for(
            sink,
            &state.reference,
            Some(turn.turn_id.clone()),
            Some(previous),
            EventActor::Agent(turn.agent_id.clone()),
            EventKind::ToolIntent(ToolIntentEvent {
                tool_name: tool_name.clone(),
                arguments,
                side_effect,
            }),
            Some(operation_id.clone()),
            provider_call_id.clone(),
            Some(turn.branch_id.clone()),
        )?;
        state
            .branches
            .get_mut(&state.selected_branch)
            .unwrap()
            .last_event = event.event_id.clone();
        state.tools.insert(
            operation_id.clone(),
            ToolRecord {
                turn_id: turn.turn_id.clone(),
                state: ToolState::Intended,
                tool_name: tool_name.clone(),
                provider_call_id: provider_call_id.clone(),
            },
        );
        Ok(PendingToolRef {
            operation_id,
            provider_call_id,
            tool_name,
            intent_event: event,
            turn_id: turn.turn_id.clone(),
        })
    }

    pub fn start_tool(
        &mut self,
        turn: &PendingTurnRef,
        tool: &PendingToolRef,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        self.tool_transition(
            turn,
            tool,
            &[ToolState::Intended],
            ToolState::Started,
            |name| EventKind::ToolStarted(ToolStartedEvent { tool_name: name }),
        )
    }

    pub fn complete_tool(
        &mut self,
        turn: &PendingTurnRef,
        tool: &PendingToolRef,
        summary: Option<String>,
        result: Option<Value>,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        self.tool_transition(
            turn,
            tool,
            &[ToolState::Started],
            ToolState::Terminal,
            |_| {
                EventKind::ToolResult(ToolResultEvent {
                    outcome: ToolOutcome::Completed,
                    summary,
                    result,
                })
            },
        )
    }

    pub fn fail_tool(
        &mut self,
        turn: &PendingTurnRef,
        tool: &PendingToolRef,
        summary: impl Into<String>,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        let summary = summary.into();
        self.tool_transition(
            turn,
            tool,
            &[ToolState::Intended, ToolState::Started],
            ToolState::Terminal,
            |_| {
                EventKind::ToolResult(ToolResultEvent {
                    outcome: ToolOutcome::Failed,
                    summary: Some(summary),
                    result: None,
                })
            },
        )
    }

    fn tool_transition(
        &mut self,
        turn: &PendingTurnRef,
        tool: &PendingToolRef,
        expected: &[ToolState],
        next: ToolState,
        kind: impl FnOnce(String) -> EventKind,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        if tool.turn_id != turn.turn_id {
            return Err(AgentRuntimeError::WrongToolTurn);
        }
        let (sink, state) = self.active_state(turn)?;
        let record = state
            .tools
            .get(&tool.operation_id)
            .ok_or(AgentRuntimeError::UnknownOperation)?;
        if record.turn_id != turn.turn_id {
            return Err(AgentRuntimeError::WrongToolTurn);
        }
        if !expected.contains(&record.state) {
            return Err(AgentRuntimeError::InvalidToolState);
        }
        let previous = state.branches[&state.selected_branch].last_event.clone();
        let event = append_for(
            sink,
            &state.reference,
            Some(turn.turn_id.clone()),
            Some(previous),
            EventActor::Agent(turn.agent_id.clone()),
            kind(tool.tool_name.clone()),
            Some(tool.operation_id.clone()),
            tool.provider_call_id.clone(),
            Some(turn.branch_id.clone()),
        )?;
        state
            .branches
            .get_mut(&state.selected_branch)
            .unwrap()
            .last_event = event.event_id.clone();
        state.tools.get_mut(&tool.operation_id).unwrap().state = next;
        Ok(event)
    }

    pub fn complete_turn(
        &mut self,
        turn: &PendingTurnRef,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        {
            let (_, state) = self.active_state(turn)?;
            if state
                .tools
                .values()
                .any(|tool| tool.turn_id == turn.turn_id && tool.state != ToolState::Terminal)
            {
                return Err(AgentRuntimeError::OutstandingToolOperations);
            }
        }
        self.finish_turn(turn, TurnLifecycleState::Completed, None)
    }

    pub fn fail_turn(
        &mut self,
        turn: &PendingTurnRef,
        detail: impl Into<String>,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        self.finish_turn(turn, TurnLifecycleState::Failed, Some(detail.into()))
    }

    pub fn interrupt_turn(
        &mut self,
        turn: &PendingTurnRef,
        detail: Option<String>,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        self.finish_turn(turn, TurnLifecycleState::Interrupted, detail)
    }

    fn finish_turn(
        &mut self,
        turn: &PendingTurnRef,
        terminal: TurnLifecycleState,
        detail: Option<String>,
    ) -> Result<EventEnvelope, AgentRuntimeError> {
        let (sink, state) = self.active_state(turn)?;
        if terminal != TurnLifecycleState::Completed {
            let mut outstanding = state
                .tools
                .iter()
                .filter(|(_, tool)| {
                    tool.turn_id == turn.turn_id && tool.state != ToolState::Terminal
                })
                .map(|(operation_id, tool)| {
                    (
                        operation_id.clone(),
                        tool.tool_name.clone(),
                        tool.provider_call_id.clone(),
                    )
                })
                .collect::<Vec<_>>();
            outstanding.sort_by(|left, right| left.0.cmp(&right.0));
            for (operation_id, tool_name, provider_call_id) in outstanding {
                let previous = state.branches[&state.selected_branch].last_event.clone();
                let interrupted = append_for(
                    sink,
                    &state.reference,
                    Some(turn.turn_id.clone()),
                    Some(previous),
                    EventActor::Agent(turn.agent_id.clone()),
                    EventKind::ToolResult(ToolResultEvent {
                        outcome: ToolOutcome::Interrupted,
                        summary: Some(format!("{tool_name} interrupted with its turn")),
                        result: None,
                    }),
                    Some(operation_id.clone()),
                    provider_call_id,
                    Some(turn.branch_id.clone()),
                )?;
                state
                    .branches
                    .get_mut(&state.selected_branch)
                    .unwrap()
                    .last_event = interrupted.event_id;
                state.tools.get_mut(&operation_id).unwrap().state = ToolState::Terminal;
            }
        }
        let previous = state.branches[&state.selected_branch].last_event.clone();
        let event = append_for(
            sink,
            &state.reference,
            Some(turn.turn_id.clone()),
            Some(previous),
            EventActor::Agent(turn.agent_id.clone()),
            EventKind::TurnLifecycle(TurnLifecycleEvent {
                state: terminal,
                detail,
            }),
            None,
            None,
            Some(turn.branch_id.clone()),
        )?;
        state
            .branches
            .get_mut(&state.selected_branch)
            .unwrap()
            .last_event = event.event_id.clone();
        state.active_turn = None;
        state.terminal_turns.insert(turn.turn_id.clone());
        Ok(event)
    }

    fn active_state(
        &mut self,
        turn: &PendingTurnRef,
    ) -> Result<(&Arc<dyn RunEventSink>, &mut ConversationState), AgentRuntimeError> {
        let state = self
            .conversations
            .values_mut()
            .find(|state| state.reference.conversation_id == turn.conversation_id)
            .ok_or(AgentRuntimeError::UnknownConversation)?;
        if state.reference.run_id != turn.run_id || state.reference.root_agent_id != turn.agent_id {
            return Err(AgentRuntimeError::WrongActiveTurn);
        }
        let selected = state.branches.get(&state.selected_branch).unwrap();
        if selected.reference.branch_id != turn.branch_id {
            return Err(AgentRuntimeError::WrongActiveBranch);
        }
        match &state.active_turn {
            Some(active) if active == &turn.turn_id => Ok((&self.sink, state)),
            Some(_) => Err(AgentRuntimeError::WrongActiveTurn),
            None if state.terminal_turns.contains(&turn.turn_id) => {
                Err(AgentRuntimeError::TurnAlreadyTerminal)
            }
            None => Err(AgentRuntimeError::NoActiveTurn),
        }
    }

    pub fn events(&self, run_id: &RunId) -> Result<Vec<EventEnvelope>, AgentRuntimeError> {
        Ok(self.sink.events(run_id)?)
    }

    pub fn conversation_events(
        &self,
        locator: &ConversationLocator,
    ) -> Result<Vec<EventEnvelope>, AgentRuntimeError> {
        let state = self
            .conversations
            .get(locator)
            .ok_or(AgentRuntimeError::UnknownConversation)?;
        self.events(&state.reference.run_id)
    }
}

fn agent_event(
    reference: &ConversationRef,
    spec: &AgentSpec,
    state: AgentLifecycleState,
) -> EventKind {
    EventKind::AgentLifecycle(AgentLifecycleEvent {
        agent_id: reference.root_agent_id.clone(),
        parent_agent_id: None,
        state,
        kind: spec.kind.clone(),
        objective: spec.objective.clone(),
        detail: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn append_for(
    sink: &Arc<dyn RunEventSink>,
    reference: &ConversationRef,
    turn_id: Option<TurnId>,
    caused_by: Option<EventId>,
    actor: EventActor,
    kind: EventKind,
    operation_id: Option<OperationId>,
    provider_call_id: Option<String>,
    branch_id: Option<BranchId>,
) -> Result<EventEnvelope, RunLogError> {
    sink.append(NewRunEvent {
        run_id: reference.run_id.clone(),
        caused_by,
        operation_id,
        provider_call_id,
        actor,
        agent_id: Some(reference.root_agent_id.clone()),
        turn_id,
        workspace_id: Some(reference.workspace_id.clone()),
        branch_id,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::{BaseManifestId, RepositoryId};
    use serde_json::json;

    fn labels(events: &[EventEnvelope]) -> Vec<String> {
        events
            .iter()
            .map(|event| match &event.kind {
                EventKind::RunLifecycle(value) => format!("run.{:?}", value.state),
                EventKind::BranchLifecycle(value) => format!("branch.{:?}", value.state),
                EventKind::AgentLifecycle(value) => format!("agent.{:?}", value.state),
                EventKind::TurnLifecycle(value) => format!("turn.{:?}", value.state),
                EventKind::Message(value) => format!("message.{:?}", value.role),
                EventKind::ToolIntent(_) => "tool.intent".into(),
                EventKind::ToolStarted(_) => "tool.started".into(),
                EventKind::ToolResult(value) => format!("tool.{:?}", value.outcome),
                _ => "other".into(),
            })
            .collect()
    }

    #[test]
    fn golden_turn_and_tool_sequence() {
        let mut runtime = AgentRuntime::new();
        let turn = runtime
            .begin_turn("buffer:1", "main", "inspect it", AgentSpec::chat())
            .unwrap();
        runtime
            .append_reasoning_summary(&turn, "I will inspect.")
            .unwrap();
        let tool = runtime
            .record_tool_intent(
                &turn,
                "read_file",
                json!({"path": "src/lib.rs"}),
                ToolSideEffect::Read,
                Some("provider-call-7".into()),
            )
            .unwrap();
        runtime.start_tool(&turn, &tool).unwrap();
        runtime
            .complete_tool(&turn, &tool, Some("read".into()), None)
            .unwrap();
        runtime.append_agent_message(&turn, "Found it.").unwrap();
        runtime.complete_turn(&turn).unwrap();

        assert_eq!(
            labels(&runtime.events(&turn.run_id).unwrap()),
            [
                "run.Created",
                "agent.Dispatched",
                "agent.Started",
                "branch.Created",
                "message.User",
                "turn.Started",
                "message.ReasoningSummary",
                "tool.intent",
                "tool.started",
                "tool.Completed",
                "message.Agent",
                "turn.Completed"
            ]
        );
        let events = runtime.events(&turn.run_id).unwrap();
        assert!(events
            .windows(2)
            .all(|pair| pair[1].caused_by == Some(pair[0].event_id.clone())));
        assert_eq!(events[7].operation_id, events[8].operation_id);
        assert_eq!(events[8].operation_id, events[9].operation_id);
        assert_eq!(
            events[7].provider_call_id.as_deref(),
            Some("provider-call-7")
        );
        assert!(events
            .iter()
            .all(|event| event.branch_id.as_ref() == Some(&turn.branch_id)));
    }

    #[test]
    fn locator_reuses_run_agent_and_branches_keep_identity() {
        let mut runtime = AgentRuntime::new();
        let locator = ConversationLocator("chat".into());
        let main = BranchLocator("main".into());
        let first = runtime
            .begin_turn(
                locator.0.as_str(),
                main.0.as_str(),
                "one",
                AgentSpec::chat(),
            )
            .unwrap();
        runtime.complete_turn(&first).unwrap();
        let fork = runtime
            .fork_branch(&locator, &main, BranchLocator("fork".into()))
            .unwrap();
        runtime
            .select_branch(&locator, &BranchLocator("fork".into()))
            .unwrap();
        let second = runtime
            .begin_turn("chat", "fork", "two", AgentSpec::chat())
            .unwrap();
        assert_eq!(first.run_id, second.run_id);
        assert_eq!(first.agent_id, second.agent_id);
        assert_eq!(second.branch_id, fork.branch_id);
        assert_ne!(first.branch_id, second.branch_id);
    }

    #[test]
    fn run_creation_records_repository_anchors_and_initial_branch() {
        let repository_id = RepositoryId::parse("repo_ovim").unwrap();
        let base_manifest_id = BaseManifestId::parse("bsm_initial").unwrap();
        let mut runtime = AgentRuntime::with_sink_and_run_metadata(
            Arc::new(InMemoryRunEventSink::new()),
            RunCreationMetadata {
                repository_id: Some(repository_id.clone()),
                base_commit: Some("0123456789abcdef".into()),
                base_manifest_id: Some(base_manifest_id.clone()),
                initial_branch_id: None,
            },
        );
        let turn = runtime
            .begin_turn("chat", "main", "inspect", AgentSpec::chat())
            .unwrap();
        let events = runtime.events(&turn.run_id).unwrap();
        let EventKind::RunLifecycle(created) = &events[0].kind else {
            panic!("first event must create the run")
        };
        let metadata = created.creation.as_ref().unwrap();
        assert_eq!(metadata.repository_id.as_ref(), Some(&repository_id));
        assert_eq!(metadata.base_commit.as_deref(), Some("0123456789abcdef"));
        assert_eq!(metadata.base_manifest_id.as_ref(), Some(&base_manifest_id));
        assert_eq!(metadata.initial_branch_id.as_ref(), Some(&turn.branch_id));
    }

    #[test]
    fn fork_event_durably_identifies_parent_and_exact_fork_point() {
        let mut runtime = AgentRuntime::new();
        let locator = ConversationLocator("chat".into());
        let main = BranchLocator("main".into());
        let first = runtime
            .begin_turn("chat", "main", "one", AgentSpec::chat())
            .unwrap();
        runtime.complete_turn(&first).unwrap();
        let fork_point = first.initiating_event.event_id.clone();
        let fork = runtime
            .fork_branch_at(
                &locator,
                &main,
                BranchLocator("alternative".into()),
                fork_point.clone(),
            )
            .unwrap();

        let events = runtime.events(&first.run_id).unwrap();
        let forked = events
            .iter()
            .find(|event| event.branch_id.as_ref() == Some(&fork.branch_id))
            .unwrap();
        assert_eq!(forked.caused_by.as_ref(), Some(&fork_point));
        let EventKind::BranchLifecycle(lifecycle) = &forked.kind else {
            panic!("first event on the fork must describe the fork")
        };
        assert_eq!(lifecycle.state, BranchLifecycleState::Forked);
        assert_eq!(lifecycle.parent_branch_id.as_ref(), Some(&first.branch_id));
        assert_eq!(lifecycle.forked_at.as_ref(), Some(&fork_point));
    }

    #[test]
    fn switching_back_resumes_that_branches_causal_tip() {
        let mut runtime = AgentRuntime::new();
        let locator = ConversationLocator("chat".into());
        let main = BranchLocator("main".into());
        let first = runtime
            .begin_turn("chat", "main", "one", AgentSpec::chat())
            .unwrap();
        let main_tip = runtime.complete_turn(&first).unwrap();
        runtime
            .fork_branch(&locator, &main, BranchLocator("fork".into()))
            .unwrap();
        runtime
            .select_branch(&locator, &BranchLocator("fork".into()))
            .unwrap();
        let fork_turn = runtime
            .begin_turn("chat", "fork", "on fork", AgentSpec::chat())
            .unwrap();
        let fork_tip = runtime.complete_turn(&fork_turn).unwrap();
        runtime.select_branch(&locator, &main).unwrap();
        let resumed = runtime
            .begin_turn("chat", "main", "back on main", AgentSpec::chat())
            .unwrap();

        let events = runtime.events(&resumed.run_id).unwrap();
        let selected_main = events
            .iter()
            .find(|event| {
                event.branch_id.as_ref() == Some(&first.branch_id)
                    && matches!(
                        &event.kind,
                        EventKind::BranchLifecycle(BranchLifecycleEvent {
                            state: BranchLifecycleState::Selected,
                            ..
                        })
                    )
            })
            .unwrap();
        assert_eq!(selected_main.caused_by, Some(main_tip.event_id));
        let selected_main_id = selected_main.event_id.clone();
        let resumed_message = events
            .into_iter()
            .find(|event| {
                event.turn_id.as_ref() == Some(&resumed.turn_id)
                    && matches!(
                        &event.kind,
                        EventKind::Message(MessageEvent {
                            role: MessageRole::User,
                            ..
                        })
                    )
            })
            .unwrap();
        assert_eq!(resumed_message.caused_by, Some(selected_main_id));
        assert_ne!(resumed_message.caused_by, Some(fork_tip.event_id));
    }

    #[test]
    fn denied_tool_can_fail_before_it_starts() {
        let mut runtime = AgentRuntime::new();
        let turn = runtime
            .begin_turn("chat", "main", "deploy", AgentSpec::chat())
            .unwrap();
        let tool = runtime
            .record_tool_intent(
                &turn,
                "deploy",
                json!({}),
                ToolSideEffect::External,
                Some("call-1".into()),
            )
            .unwrap();
        let failed = runtime.fail_tool(&turn, &tool, "denied").unwrap();
        assert!(matches!(
            failed.kind,
            EventKind::ToolResult(ToolResultEvent {
                outcome: ToolOutcome::Failed,
                ..
            })
        ));
        assert!(matches!(
            runtime.start_tool(&turn, &tool),
            Err(AgentRuntimeError::InvalidToolState)
        ));
    }

    #[test]
    fn completed_turn_rejects_outstanding_tool_operations() {
        let mut runtime = AgentRuntime::new();
        let turn = runtime
            .begin_turn("chat", "main", "inspect", AgentSpec::chat())
            .unwrap();
        let tool = runtime
            .record_tool_intent(
                &turn,
                "read_file",
                json!({"path": "src/lib.rs"}),
                ToolSideEffect::Read,
                Some("call-2".into()),
            )
            .unwrap();

        assert!(matches!(
            runtime.complete_turn(&turn),
            Err(AgentRuntimeError::OutstandingToolOperations)
        ));
        runtime.start_tool(&turn, &tool).unwrap();
        assert!(matches!(
            runtime.complete_turn(&turn),
            Err(AgentRuntimeError::OutstandingToolOperations)
        ));
        runtime.complete_tool(&turn, &tool, None, None).unwrap();
        runtime.complete_turn(&turn).unwrap();
    }

    #[test]
    fn interrupted_turn_terminalizes_outstanding_tool_operations_first() {
        let mut runtime = AgentRuntime::new();
        let turn = runtime
            .begin_turn("chat", "main", "inspect", AgentSpec::chat())
            .unwrap();
        let tool = runtime
            .record_tool_intent(
                &turn,
                "read_file",
                json!({"path": "src/lib.rs"}),
                ToolSideEffect::Read,
                Some("call-1".into()),
            )
            .unwrap();
        runtime.start_tool(&turn, &tool).unwrap();

        runtime
            .interrupt_turn(&turn, Some("chat closed".into()))
            .unwrap();

        let events = runtime.events(&turn.run_id).unwrap();
        assert!(matches!(
            &events[events.len() - 2].kind,
            EventKind::ToolResult(ToolResultEvent {
                outcome: ToolOutcome::Interrupted,
                ..
            })
        ));
        assert!(matches!(
            &events.last().unwrap().kind,
            EventKind::TurnLifecycle(TurnLifecycleEvent {
                state: TurnLifecycleState::Interrupted,
                ..
            })
        ));
    }

    #[test]
    fn rejects_duplicate_terminal_event() {
        let mut runtime = AgentRuntime::new();
        let turn = runtime
            .begin_turn("chat", "main", "go", AgentSpec::chat())
            .unwrap();
        runtime.complete_turn(&turn).unwrap();
        assert!(matches!(
            runtime.fail_turn(&turn, "late"),
            Err(AgentRuntimeError::TurnAlreadyTerminal)
        ));
        assert_eq!(
            labels(&runtime.events(&turn.run_id).unwrap())
                .iter()
                .filter(
                    |label| label.starts_with("turn.Completed") || label.starts_with("turn.Failed")
                )
                .count(),
            1
        );
    }
}
