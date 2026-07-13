use crate::run_log::{
    AgentId, AgentLifecycleEvent, AgentLifecycleState, BranchId, BranchLifecycleEvent,
    BranchLifecycleState, ConversationBinding, ConversationId, EventActor, EventEnvelope, EventId,
    EventKind, InMemoryRunEventSink, MessageEvent, MessageRole, NewRunEvent, OperationId,
    RepositoryId, RunCreationMetadata, RunEventSink, RunId, RunLifecycleEvent, RunLifecycleState,
    RunLogError, ToolIntentEvent, ToolOutcome, ToolResultEvent, ToolSideEffect, ToolStartedEvent,
    TurnId, TurnLifecycleEvent, TurnLifecycleState, WorkspaceId,
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
    /// A catalog binding may exist before its first event. The first turn uses
    /// these durable identities to create, rather than replace, the run.
    uninitialized_branch_id: Option<BranchId>,
    uninitialized_repository_id: Option<RepositoryId>,
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
    ConversationAlreadyExists,
    BindingMismatch(String),
    InvalidHistory(String),
    UnrecoveredWork(String),
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
            Self::ConversationAlreadyExists => f.write_str("conversation locator is already bound"),
            Self::BindingMismatch(detail) => {
                write!(f, "conversation binding does not match history: {detail}")
            }
            Self::InvalidHistory(detail) => write!(f, "invalid durable history: {detail}"),
            Self::UnrecoveredWork(detail) => {
                write!(f, "durable history contains unrecovered work: {detail}")
            }
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

    /// Rebuilds provider-independent continuation state from a durable binding
    /// and its normalized event history. The backing sink must contain the
    /// same history so the next append can safely reference its causal tip.
    pub fn restore_conversation(
        &mut self,
        locator: impl Into<ConversationLocator>,
        binding: ConversationBinding,
        events: Vec<EventEnvelope>,
    ) -> Result<ConversationRef, AgentRuntimeError> {
        let locator = locator.into();
        if self.conversations.contains_key(&locator) {
            return Err(AgentRuntimeError::ConversationAlreadyExists);
        }
        if self.conversations.values().any(|state| {
            state.reference.conversation_id == binding.conversation_id
                || state.reference.run_id == binding.run_id
        }) {
            return Err(AgentRuntimeError::BindingMismatch(
                "conversation or run is already restored under another locator".into(),
            ));
        }
        let persisted = self.sink.events(&binding.run_id)?;
        if persisted != events {
            return Err(AgentRuntimeError::BindingMismatch(
                "supplied events do not match the backing event sink".into(),
            ));
        }
        let reference = ConversationRef {
            conversation_id: binding.conversation_id.clone(),
            run_id: binding.run_id.clone(),
            root_agent_id: binding.root_agent_id.clone(),
            workspace_id: binding.workspace_id.clone(),
        };
        if events.is_empty() {
            self.conversations.insert(
                locator,
                ConversationState {
                    reference: reference.clone(),
                    branches: HashMap::new(),
                    selected_branch: BranchLocator(String::new()),
                    active_turn: None,
                    terminal_turns: HashSet::new(),
                    tools: HashMap::new(),
                    uninitialized_branch_id: Some(binding.selected_branch_id),
                    uninitialized_repository_id: Some(binding.key.repository_id),
                },
            );
            return Ok(reference);
        }
        let mut branches: HashMap<BranchLocator, BranchState> = HashMap::new();
        let mut branch_locators: HashMap<BranchId, BranchLocator> = HashMap::new();
        let mut selected_branch_id: Option<BranchId> = None;
        let mut initial_branch_id: Option<BranchId> = None;
        let mut seen_events = HashSet::new();
        let mut started_turns = HashSet::new();
        let mut terminal_turns = HashSet::new();
        let mut tools: HashMap<OperationId, ToolRecord> = HashMap::new();

        for (index, event) in events.iter().enumerate() {
            let expected_sequence = index as u64 + 1;
            if event.run_id != binding.run_id || event.sequence != expected_sequence {
                return Err(AgentRuntimeError::BindingMismatch(format!(
                    "event {} has the wrong run or sequence",
                    event.event_id
                )));
            }
            if event
                .agent_id
                .as_ref()
                .is_some_and(|id| id != &binding.root_agent_id)
            {
                return Err(AgentRuntimeError::BindingMismatch(format!(
                    "event {} belongs to another agent",
                    event.event_id
                )));
            }
            if event
                .workspace_id
                .as_ref()
                .is_some_and(|id| id != &binding.workspace_id)
            {
                return Err(AgentRuntimeError::BindingMismatch(format!(
                    "event {} belongs to another workspace",
                    event.event_id
                )));
            }
            if let Some(cause) = &event.caused_by {
                if !seen_events.contains(cause) {
                    return Err(AgentRuntimeError::InvalidHistory(format!(
                        "event {} references a missing or later cause {cause}",
                        event.event_id
                    )));
                }
            }
            // Once a branch is declared, every event on it must extend that
            // branch's own tip. Creation has no existing tip and Forked is the
            // deliberate exception: it is caused by an exact event in the
            // parent trajectory.
            if let Some(branch_id) = &event.branch_id {
                if let Some(branch_locator) = branch_locators.get(branch_id) {
                    let is_fork = matches!(
                        &event.kind,
                        EventKind::BranchLifecycle(BranchLifecycleEvent {
                            state: BranchLifecycleState::Forked,
                            ..
                        })
                    );
                    if !is_fork {
                        let expected = &branches[branch_locator].last_event;
                        if event.caused_by.as_ref() != Some(expected) {
                            return Err(AgentRuntimeError::InvalidHistory(format!(
                                "event {} does not extend branch {} tip {}",
                                event.event_id, branch_id, expected
                            )));
                        }
                    }
                }
            }

            match &event.kind {
                EventKind::RunLifecycle(lifecycle)
                    if lifecycle.state == RunLifecycleState::Created =>
                {
                    let metadata = lifecycle.creation.as_ref().ok_or_else(|| {
                        AgentRuntimeError::BindingMismatch(
                            "run creation has no durable metadata".into(),
                        )
                    })?;
                    if metadata.repository_id.as_ref() != Some(&binding.key.repository_id) {
                        return Err(AgentRuntimeError::BindingMismatch(
                            "run creation repository differs from the catalog binding".into(),
                        ));
                    }
                    initial_branch_id = metadata.initial_branch_id.clone();
                }
                EventKind::AgentLifecycle(lifecycle)
                    if lifecycle.agent_id != binding.root_agent_id =>
                {
                    return Err(AgentRuntimeError::BindingMismatch(format!(
                        "agent lifecycle {} names another agent",
                        event.event_id
                    )));
                }
                EventKind::BranchLifecycle(lifecycle) => {
                    if event.branch_id.as_ref() != Some(&lifecycle.branch_id) {
                        return Err(AgentRuntimeError::InvalidHistory(format!(
                            "branch lifecycle {} disagrees with its envelope",
                            event.event_id
                        )));
                    }
                    let label = lifecycle.label.as_ref().ok_or_else(|| {
                        AgentRuntimeError::InvalidHistory(format!(
                            "branch lifecycle {} has no durable locator",
                            event.event_id
                        ))
                    })?;
                    let locator = BranchLocator(label.clone());
                    match lifecycle.state {
                        BranchLifecycleState::Created => {
                            if lifecycle.parent_branch_id.is_some()
                                || lifecycle.forked_at.is_some()
                                || branch_locators.contains_key(&lifecycle.branch_id)
                                || branches.contains_key(&locator)
                            {
                                return Err(AgentRuntimeError::InvalidHistory(format!(
                                    "invalid branch creation {}",
                                    event.event_id
                                )));
                            }
                            branches.insert(
                                locator.clone(),
                                BranchState {
                                    reference: BranchRef {
                                        branch_id: lifecycle.branch_id.clone(),
                                        parent_branch_id: None,
                                    },
                                    last_event: event.event_id.clone(),
                                },
                            );
                            branch_locators.insert(lifecycle.branch_id.clone(), locator);
                            selected_branch_id.get_or_insert(lifecycle.branch_id.clone());
                        }
                        BranchLifecycleState::Forked => {
                            let parent = lifecycle.parent_branch_id.as_ref().ok_or_else(|| {
                                AgentRuntimeError::InvalidHistory(
                                    "fork has no parent branch".into(),
                                )
                            })?;
                            let forked_at = lifecycle.forked_at.as_ref().ok_or_else(|| {
                                AgentRuntimeError::InvalidHistory("fork has no fork event".into())
                            })?;
                            if !branch_locators.contains_key(parent)
                                || !seen_events.contains(forked_at)
                                || event.caused_by.as_ref() != Some(forked_at)
                                || branch_locators.contains_key(&lifecycle.branch_id)
                                || branches.contains_key(&locator)
                            {
                                return Err(AgentRuntimeError::InvalidHistory(format!(
                                    "invalid branch fork {}",
                                    event.event_id
                                )));
                            }
                            branches.insert(
                                locator.clone(),
                                BranchState {
                                    reference: BranchRef {
                                        branch_id: lifecycle.branch_id.clone(),
                                        parent_branch_id: Some(parent.clone()),
                                    },
                                    last_event: event.event_id.clone(),
                                },
                            );
                            branch_locators.insert(lifecycle.branch_id.clone(), locator);
                        }
                        BranchLifecycleState::Selected => {
                            let known =
                                branch_locators.get(&lifecycle.branch_id).ok_or_else(|| {
                                    AgentRuntimeError::InvalidHistory(
                                        "selected branch was never declared".into(),
                                    )
                                })?;
                            if known != &locator {
                                return Err(AgentRuntimeError::InvalidHistory(
                                    "selected branch locator changed".into(),
                                ));
                            }
                            selected_branch_id = Some(lifecycle.branch_id.clone());
                        }
                    }
                }
                EventKind::TurnLifecycle(lifecycle) => {
                    let turn_id = event.turn_id.clone().ok_or_else(|| {
                        AgentRuntimeError::InvalidHistory("turn lifecycle has no turn ID".into())
                    })?;
                    match lifecycle.state {
                        TurnLifecycleState::Started => {
                            if !started_turns.insert(turn_id.clone())
                                || terminal_turns.contains(&turn_id)
                            {
                                return Err(AgentRuntimeError::InvalidHistory(
                                    "turn was started more than once".into(),
                                ));
                            }
                        }
                        TurnLifecycleState::Completed
                        | TurnLifecycleState::Interrupted
                        | TurnLifecycleState::Failed => {
                            if !started_turns.contains(&turn_id)
                                || !terminal_turns.insert(turn_id.clone())
                            {
                                return Err(AgentRuntimeError::InvalidHistory(
                                    "turn terminal record has no unique start".into(),
                                ));
                            }
                            if tools.values().any(|tool| {
                                tool.turn_id == turn_id && tool.state != ToolState::Terminal
                            }) {
                                return Err(AgentRuntimeError::InvalidHistory(
                                    "turn became terminal before its tool operations".into(),
                                ));
                            }
                        }
                        TurnLifecycleState::Steered => {
                            if !started_turns.contains(&turn_id)
                                || terminal_turns.contains(&turn_id)
                            {
                                return Err(AgentRuntimeError::InvalidHistory(
                                    "turn was steered while it was not active".into(),
                                ));
                            }
                        }
                    }
                }
                EventKind::ToolIntent(intent) => {
                    let operation_id = event.operation_id.clone().ok_or_else(|| {
                        AgentRuntimeError::InvalidHistory("tool intent has no operation ID".into())
                    })?;
                    let turn_id = event.turn_id.clone().ok_or_else(|| {
                        AgentRuntimeError::InvalidHistory("tool intent has no turn ID".into())
                    })?;
                    if !started_turns.contains(&turn_id) || terminal_turns.contains(&turn_id) {
                        return Err(AgentRuntimeError::InvalidHistory(
                            "tool intent is outside an active turn".into(),
                        ));
                    }
                    if tools
                        .insert(
                            operation_id,
                            ToolRecord {
                                turn_id,
                                state: ToolState::Intended,
                                tool_name: intent.tool_name.clone(),
                                provider_call_id: event.provider_call_id.clone(),
                            },
                        )
                        .is_some()
                    {
                        return Err(AgentRuntimeError::InvalidHistory(
                            "operation has more than one intent".into(),
                        ));
                    }
                }
                EventKind::ToolStarted(_) => {
                    restored_tool_transition(
                        event,
                        &mut tools,
                        ToolState::Intended,
                        ToolState::Started,
                    )?;
                }
                EventKind::ToolResult(_) => {
                    let operation_id = event.operation_id.as_ref().ok_or_else(|| {
                        AgentRuntimeError::InvalidHistory("tool result has no operation ID".into())
                    })?;
                    let record = tools.get_mut(operation_id).ok_or_else(|| {
                        AgentRuntimeError::InvalidHistory("tool result has no intent".into())
                    })?;
                    if record.state == ToolState::Terminal
                        || event.turn_id.as_ref() != Some(&record.turn_id)
                    {
                        return Err(AgentRuntimeError::InvalidHistory(
                            "tool result is duplicated or belongs to another turn".into(),
                        ));
                    }
                    record.state = ToolState::Terminal;
                }
                _ => {}
            }

            if let Some(branch_id) = &event.branch_id {
                if let Some(locator) = branch_locators.get(branch_id) {
                    branches.get_mut(locator).unwrap().last_event = event.event_id.clone();
                } else if !branches.is_empty()
                    || initial_branch_id.as_ref().is_some_and(|id| id != branch_id)
                {
                    return Err(AgentRuntimeError::InvalidHistory(format!(
                        "event {} references an undeclared branch",
                        event.event_id
                    )));
                }
            } else if !branches.is_empty() {
                return Err(AgentRuntimeError::InvalidHistory(format!(
                    "event {} after branch creation has no branch ID",
                    event.event_id
                )));
            }
            seen_events.insert(event.event_id.clone());
        }

        let unfinished_turns: Vec<_> = started_turns.difference(&terminal_turns).collect();
        let unfinished_tools: Vec<_> = tools
            .iter()
            .filter(|(_, tool)| tool.state != ToolState::Terminal)
            .collect();
        if !unfinished_turns.is_empty() || !unfinished_tools.is_empty() {
            return Err(AgentRuntimeError::UnrecoveredWork(format!(
                "{} turn(s) and {} tool operation(s) are nonterminal",
                unfinished_turns.len(),
                unfinished_tools.len()
            )));
        }
        if branches.is_empty() {
            return Err(AgentRuntimeError::InvalidHistory(
                "history predates durable branch declarations and cannot be continued safely"
                    .into(),
            ));
        }
        if initial_branch_id
            .as_ref()
            .is_some_and(|id| !branch_locators.contains_key(id))
        {
            return Err(AgentRuntimeError::InvalidHistory(
                "run creation names an undeclared initial branch".into(),
            ));
        }
        let selected_id = selected_branch_id.ok_or_else(|| {
            AgentRuntimeError::InvalidHistory("history has no selected branch".into())
        })?;
        if selected_id != binding.selected_branch_id {
            return Err(AgentRuntimeError::BindingMismatch(
                "selected branch does not match the latest branch lifecycle".into(),
            ));
        }
        let selected_branch = branch_locators.get(&selected_id).cloned().ok_or_else(|| {
            AgentRuntimeError::InvalidHistory("selected branch is undeclared".into())
        })?;

        self.conversations.insert(
            locator,
            ConversationState {
                reference: reference.clone(),
                branches,
                selected_branch,
                active_turn: None,
                terminal_turns,
                tools,
                uninitialized_branch_id: None,
                uninitialized_repository_id: None,
            },
        );
        Ok(reference)
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
        } else if self
            .conversations
            .get(&locator)
            .is_some_and(|state| state.uninitialized_branch_id.is_some())
        {
            self.initialize_bound_conversation(&locator, branch.clone(), &spec)?;
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
                uninitialized_branch_id: None,
                uninitialized_repository_id: None,
            },
        );
        Ok(())
    }

    fn initialize_bound_conversation(
        &mut self,
        locator: &ConversationLocator,
        branch: BranchLocator,
        spec: &AgentSpec,
    ) -> Result<(), AgentRuntimeError> {
        let state = self
            .conversations
            .get_mut(locator)
            .ok_or(AgentRuntimeError::UnknownConversation)?;
        let initial_branch_id = state.uninitialized_branch_id.clone().ok_or_else(|| {
            AgentRuntimeError::InvalidHistory("binding is already initialized".into())
        })?;
        let repository_id = state.uninitialized_repository_id.clone();
        let reference = state.reference.clone();
        let mut creation = self.run_creation.clone();
        creation.repository_id = repository_id;
        creation.initial_branch_id = Some(initial_branch_id.clone());
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
                creation: Some(creation),
            }),
            None,
            None,
            Some(initial_branch_id.clone()),
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
            Some(initial_branch_id.clone()),
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
            Some(initial_branch_id.clone()),
        )?;
        let branch_created = append_for(
            &self.sink,
            &reference,
            None,
            Some(started.event_id),
            EventActor::System("agent_runtime".into()),
            EventKind::BranchLifecycle(BranchLifecycleEvent {
                state: BranchLifecycleState::Created,
                branch_id: initial_branch_id.clone(),
                parent_branch_id: None,
                forked_at: None,
                label: Some(branch.0.clone()),
            }),
            None,
            None,
            Some(initial_branch_id.clone()),
        )?;
        state.branches.insert(
            branch.clone(),
            BranchState {
                reference: BranchRef {
                    branch_id: initial_branch_id,
                    parent_branch_id: None,
                },
                last_event: branch_created.event_id,
            },
        );
        state.selected_branch = branch;
        state.uninitialized_branch_id = None;
        state.uninitialized_repository_id = None;
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

    /// Whether a conversation still owns work whose terminal outcome has not
    /// been recorded. Lease owners use this to avoid advertising a crashed
    /// turn as a clean shutdown.
    pub fn has_active_work(&self, locator: &ConversationLocator) -> bool {
        self.conversations.get(locator).is_some_and(|state| {
            state.active_turn.is_some()
                || state
                    .tools
                    .values()
                    .any(|tool| tool.state != ToolState::Terminal)
        })
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

fn restored_tool_transition(
    event: &EventEnvelope,
    tools: &mut HashMap<OperationId, ToolRecord>,
    expected: ToolState,
    next: ToolState,
) -> Result<(), AgentRuntimeError> {
    let operation_id = event.operation_id.as_ref().ok_or_else(|| {
        AgentRuntimeError::InvalidHistory("tool transition has no operation ID".into())
    })?;
    let record = tools
        .get_mut(operation_id)
        .ok_or_else(|| AgentRuntimeError::InvalidHistory("tool transition has no intent".into()))?;
    if record.state != expected || event.turn_id.as_ref() != Some(&record.turn_id) {
        return Err(AgentRuntimeError::InvalidHistory(
            "tool transition is out of order or belongs to another turn".into(),
        ));
    }
    record.state = next;
    Ok(())
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
    use crate::run_log::{BaseManifestId, ConversationKey, ConversationScope, RepositoryId};
    use serde_json::json;

    struct SnapshotSink {
        events: Vec<EventEnvelope>,
    }

    impl RunEventSink for SnapshotSink {
        fn append(&self, _event: NewRunEvent) -> Result<EventEnvelope, RunLogError> {
            Err(RunLogError::Storage {
                operation: "append to read-only test snapshot".into(),
                detail: "unsupported".into(),
            })
        }

        fn event(
            &self,
            run_id: &RunId,
            event_id: &EventId,
        ) -> Result<Option<EventEnvelope>, RunLogError> {
            Ok(self
                .events
                .iter()
                .find(|event| &event.run_id == run_id && &event.event_id == event_id)
                .cloned())
        }

        fn events(&self, run_id: &RunId) -> Result<Vec<EventEnvelope>, RunLogError> {
            Ok(self
                .events
                .iter()
                .filter(|event| &event.run_id == run_id)
                .cloned()
                .collect())
        }

        fn last_sequence(&self, run_id: &RunId) -> Result<Option<u64>, RunLogError> {
            Ok(self
                .events
                .iter()
                .filter(|event| &event.run_id == run_id)
                .map(|event| event.sequence)
                .max())
        }

        fn runs(&self) -> Result<Vec<RunId>, RunLogError> {
            let mut runs: Vec<_> = self
                .events
                .iter()
                .map(|event| event.run_id.clone())
                .collect();
            runs.sort();
            runs.dedup();
            Ok(runs)
        }
    }

    fn snapshot_sink(events: Vec<EventEnvelope>) -> Arc<dyn RunEventSink> {
        Arc::new(SnapshotSink { events })
    }

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

    fn binding_for(runtime: &AgentRuntime, locator: &ConversationLocator) -> ConversationBinding {
        let conversation = runtime.conversation(locator).unwrap();
        let (_, branch) = runtime.selected_branch(locator).unwrap();
        ConversationBinding {
            key: ConversationKey {
                repository_id: RepositoryId::parse("repo_restore_tests").unwrap(),
                scope: ConversationScope::NoFile,
                logical_name: locator.0.clone(),
            },
            conversation_id: conversation.conversation_id.clone(),
            run_id: conversation.run_id.clone(),
            root_agent_id: conversation.root_agent_id.clone(),
            workspace_id: conversation.workspace_id.clone(),
            selected_branch_id: branch.branch_id.clone(),
        }
    }

    fn runtime_with_catalog_repository(sink: Arc<dyn RunEventSink>) -> AgentRuntime {
        AgentRuntime::with_sink_and_run_metadata(
            sink,
            RunCreationMetadata {
                repository_id: Some(RepositoryId::parse("repo_restore_tests").unwrap()),
                ..RunCreationMetadata::default()
            },
        )
    }

    #[test]
    fn restores_multiple_conversations_with_distinct_workspace_ids() {
        let sink: Arc<dyn RunEventSink> = Arc::new(InMemoryRunEventSink::new());
        let mut runtime = AgentRuntime::with_sink(sink);
        let repository_id = RepositoryId::parse("repo_multi_workspace_restore").unwrap();
        let binding = |name: &str| ConversationBinding {
            key: ConversationKey {
                repository_id: repository_id.clone(),
                scope: ConversationScope::NoFile,
                logical_name: name.into(),
            },
            conversation_id: ConversationId::new(),
            run_id: RunId::new(),
            root_agent_id: AgentId::new(),
            workspace_id: WorkspaceId::new(),
            selected_branch_id: BranchId::new(),
        };
        let first = binding("first");
        let second = binding("second");
        assert_ne!(first.workspace_id, second.workspace_id);

        let first_ref = runtime
            .restore_conversation("first", first.clone(), Vec::new())
            .unwrap();
        let second_ref = runtime
            .restore_conversation("second", second.clone(), Vec::new())
            .unwrap();

        assert_eq!(first_ref.workspace_id, first.workspace_id);
        assert_eq!(second_ref.workspace_id, second.workspace_id);
    }

    #[test]
    fn restores_branches_and_continues_each_causal_tip_with_stable_ids() {
        let sink: Arc<dyn RunEventSink> = Arc::new(InMemoryRunEventSink::new());
        let locator = ConversationLocator("durable-chat".into());
        let main = BranchLocator("main".into());
        let fork_locator = BranchLocator("fork".into());
        let mut first_runtime = runtime_with_catalog_repository(sink.clone());
        let first = first_runtime
            .begin_turn(locator.0.clone(), main.0.clone(), "one", AgentSpec::chat())
            .unwrap();
        first_runtime.complete_turn(&first).unwrap();
        let fork = first_runtime
            .fork_branch(&locator, &main, fork_locator.clone())
            .unwrap();
        first_runtime
            .select_branch(&locator, &fork_locator)
            .unwrap();
        let fork_turn = first_runtime
            .begin_turn(
                locator.0.clone(),
                fork_locator.0.clone(),
                "fork",
                AgentSpec::chat(),
            )
            .unwrap();
        first_runtime.complete_turn(&fork_turn).unwrap();
        first_runtime.select_branch(&locator, &main).unwrap();
        let main_tip = first_runtime.selected_branch_tip(&locator).unwrap().clone();
        let binding = binding_for(&first_runtime, &locator);
        let encoded = serde_json::to_string(&first_runtime.events(&first.run_id).unwrap()).unwrap();
        let events: Vec<EventEnvelope> = serde_json::from_str(&encoded).unwrap();

        let mut restored = AgentRuntime::with_sink(sink.clone());
        let reference = restored
            .restore_conversation(locator.clone(), binding.clone(), events)
            .unwrap();
        assert_eq!(reference.run_id, binding.run_id);
        assert_eq!(reference.root_agent_id, binding.root_agent_id);
        assert_eq!(reference.workspace_id, binding.workspace_id);
        let resumed_main = restored
            .begin_turn(
                locator.0.clone(),
                main.0.clone(),
                "main again",
                AgentSpec::chat(),
            )
            .unwrap();
        assert_eq!(resumed_main.run_id, first.run_id);
        assert_eq!(resumed_main.agent_id, first.agent_id);
        assert_eq!(resumed_main.branch_id, first.branch_id);
        let main_user = sink
            .event(
                &resumed_main.run_id,
                resumed_main.initiating_event.caused_by.as_ref().unwrap(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(main_user.caused_by, Some(main_tip));
        restored.complete_turn(&resumed_main).unwrap();

        let fork_tip = restored
            .events(&binding.run_id)
            .unwrap()
            .into_iter()
            .rev()
            .find(|event| event.branch_id.as_ref() == Some(&fork.branch_id))
            .unwrap()
            .event_id;
        restored.select_branch(&locator, &fork_locator).unwrap();
        let selected_fork_tip = restored.selected_branch_tip(&locator).unwrap().clone();
        let selected = sink
            .event(&binding.run_id, &selected_fork_tip)
            .unwrap()
            .unwrap();
        assert_eq!(selected.caused_by, Some(fork_tip));
        let resumed_fork = restored
            .begin_turn(locator.0, fork_locator.0, "fork again", AgentSpec::chat())
            .unwrap();
        assert_eq!(resumed_fork.branch_id, fork.branch_id);
        let fork_user = sink
            .event(
                &binding.run_id,
                resumed_fork.initiating_event.caused_by.as_ref().unwrap(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(fork_user.caused_by, Some(selected_fork_tip));
    }

    #[test]
    fn restore_rejects_active_turn_and_pending_tool_histories() {
        let sink: Arc<dyn RunEventSink> = Arc::new(InMemoryRunEventSink::new());
        let locator = ConversationLocator("active".into());
        let mut runtime = runtime_with_catalog_repository(sink.clone());
        let turn = runtime
            .begin_turn(locator.0.clone(), "main", "work", AgentSpec::chat())
            .unwrap();
        let binding = binding_for(&runtime, &locator);
        let events = runtime.events(&turn.run_id).unwrap();
        assert!(matches!(
            AgentRuntime::with_sink(sink.clone()).restore_conversation(
                "restored-active",
                binding.clone(),
                events
            ),
            Err(AgentRuntimeError::UnrecoveredWork(_))
        ));

        runtime
            .record_tool_intent(&turn, "shell", json!({}), ToolSideEffect::Mutation, None)
            .unwrap();
        assert!(matches!(
            AgentRuntime::with_sink(sink).restore_conversation(
                "restored-tool",
                binding,
                runtime.events(&turn.run_id).unwrap()
            ),
            Err(AgentRuntimeError::UnrecoveredWork(_))
        ));
    }

    #[test]
    fn restore_rejects_binding_identity_mismatch() {
        let sink: Arc<dyn RunEventSink> = Arc::new(InMemoryRunEventSink::new());
        let locator = ConversationLocator("mismatch".into());
        let mut runtime = runtime_with_catalog_repository(sink.clone());
        let turn = runtime
            .begin_turn(locator.0.clone(), "main", "done", AgentSpec::chat())
            .unwrap();
        runtime.complete_turn(&turn).unwrap();
        let mut binding = binding_for(&runtime, &locator);
        binding.root_agent_id = AgentId::new();
        assert!(matches!(
            AgentRuntime::with_sink(sink).restore_conversation(
                "bad",
                binding,
                runtime.events(&turn.run_id).unwrap()
            ),
            Err(AgentRuntimeError::BindingMismatch(_))
        ));
    }

    #[test]
    fn empty_catalog_binding_initializes_its_exact_identities_on_first_turn() {
        let sink: Arc<dyn RunEventSink> = Arc::new(InMemoryRunEventSink::new());
        let repository_id = RepositoryId::parse("repo_bound_new").unwrap();
        let binding = ConversationBinding {
            key: ConversationKey {
                repository_id: repository_id.clone(),
                scope: ConversationScope::NoFile,
                logical_name: "new".into(),
            },
            conversation_id: ConversationId::new(),
            run_id: RunId::new(),
            root_agent_id: AgentId::new(),
            workspace_id: WorkspaceId::new(),
            selected_branch_id: BranchId::new(),
        };
        let mut runtime = AgentRuntime::with_sink(sink);
        runtime
            .restore_conversation("new", binding.clone(), Vec::new())
            .unwrap();
        let turn = runtime
            .begin_turn("new", "main", "first", AgentSpec::chat())
            .unwrap();
        assert_eq!(turn.run_id, binding.run_id);
        assert_eq!(turn.agent_id, binding.root_agent_id);
        assert_eq!(turn.workspace_id, binding.workspace_id);
        assert_eq!(turn.conversation_id, binding.conversation_id);
        assert_eq!(turn.branch_id, binding.selected_branch_id);
        let events = runtime.events(&binding.run_id).unwrap();
        let EventKind::RunLifecycle(created) = &events[0].kind else {
            panic!("bound run must begin with run creation")
        };
        let metadata = created.creation.as_ref().unwrap();
        assert_eq!(metadata.repository_id.as_ref(), Some(&repository_id));
        assert_eq!(
            metadata.initial_branch_id.as_ref(),
            Some(&binding.selected_branch_id)
        );
    }

    #[test]
    fn restore_rejects_cross_branch_causal_edges() {
        let source: Arc<dyn RunEventSink> = Arc::new(InMemoryRunEventSink::new());
        let locator = ConversationLocator("cross-branch".into());
        let main = BranchLocator("main".into());
        let fork_locator = BranchLocator("fork".into());
        let mut runtime = runtime_with_catalog_repository(source);
        let main_turn = runtime
            .begin_turn(locator.0.clone(), main.0.clone(), "main", AgentSpec::chat())
            .unwrap();
        let main_tip = runtime.complete_turn(&main_turn).unwrap().event_id;
        let fork = runtime
            .fork_branch(&locator, &main, fork_locator.clone())
            .unwrap();
        runtime.select_branch(&locator, &fork_locator).unwrap();
        let fork_turn = runtime
            .begin_turn(locator.0.clone(), fork_locator.0, "fork", AgentSpec::chat())
            .unwrap();
        runtime.complete_turn(&fork_turn).unwrap();
        let binding = binding_for(&runtime, &locator);
        let mut events = runtime.events(&binding.run_id).unwrap();
        let fork_user = events
            .iter_mut()
            .find(|event| {
                event.branch_id.as_ref() == Some(&fork.branch_id)
                    && event.turn_id.as_ref() == Some(&fork_turn.turn_id)
                    && matches!(
                        event.kind,
                        EventKind::Message(MessageEvent {
                            role: MessageRole::User,
                            ..
                        })
                    )
            })
            .unwrap();
        fork_user.caused_by = Some(main_tip);
        let sink = snapshot_sink(events.clone());
        assert!(matches!(
            AgentRuntime::with_sink(sink).restore_conversation("bad", binding, events),
            Err(AgentRuntimeError::InvalidHistory(_))
        ));
    }

    #[test]
    fn restore_rejects_repository_binding_mismatch() {
        let sink: Arc<dyn RunEventSink> = Arc::new(InMemoryRunEventSink::new());
        let recorded_repository = RepositoryId::parse("repo_recorded").unwrap();
        let locator = ConversationLocator("repository-mismatch".into());
        let mut runtime = AgentRuntime::with_sink_and_run_metadata(
            sink.clone(),
            RunCreationMetadata {
                repository_id: Some(recorded_repository),
                ..RunCreationMetadata::default()
            },
        );
        let turn = runtime
            .begin_turn(locator.0.clone(), "main", "done", AgentSpec::chat())
            .unwrap();
        runtime.complete_turn(&turn).unwrap();
        let binding = binding_for(&runtime, &locator);
        let mut events = runtime.events(&turn.run_id).unwrap();
        assert!(matches!(
            AgentRuntime::with_sink(sink).restore_conversation(
                "bad",
                binding.clone(),
                events.clone()
            ),
            Err(AgentRuntimeError::BindingMismatch(_))
        ));
        let EventKind::RunLifecycle(created) = &mut events[0].kind else {
            panic!("first event must create the run")
        };
        created.creation.as_mut().unwrap().repository_id = None;
        let missing_sink = snapshot_sink(events.clone());
        assert!(matches!(
            AgentRuntime::with_sink(missing_sink).restore_conversation("missing", binding, events),
            Err(AgentRuntimeError::BindingMismatch(_))
        ));
    }

    #[test]
    fn restore_rejects_steering_before_turn_start() {
        let source: Arc<dyn RunEventSink> = Arc::new(InMemoryRunEventSink::new());
        let locator = ConversationLocator("bad-steer".into());
        let mut runtime = runtime_with_catalog_repository(source);
        let turn = runtime
            .begin_turn(locator.0.clone(), "main", "done", AgentSpec::chat())
            .unwrap();
        runtime.complete_turn(&turn).unwrap();
        let binding = binding_for(&runtime, &locator);
        let mut events = runtime.events(&turn.run_id).unwrap();
        let user_message = events
            .iter_mut()
            .find(|event| {
                event.turn_id.as_ref() == Some(&turn.turn_id)
                    && matches!(
                        event.kind,
                        EventKind::Message(MessageEvent {
                            role: MessageRole::User,
                            ..
                        })
                    )
            })
            .unwrap();
        user_message.kind = EventKind::TurnLifecycle(TurnLifecycleEvent {
            state: TurnLifecycleState::Steered,
            detail: None,
        });
        let sink = snapshot_sink(events.clone());
        assert!(matches!(
            AgentRuntime::with_sink(sink).restore_conversation("bad", binding, events),
            Err(AgentRuntimeError::InvalidHistory(_))
        ));
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
