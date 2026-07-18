//! Concurrent control plane for durable delegated-agent dispatches.

use super::{
    validated_terminal_handoff, AgentApprovalClient, AgentCancellationToken, AgentDispatchRecord,
    AgentDispatchScheduler, AgentLoopBudget, AgentLoopError, AgentLoopEventRecord,
    AgentLoopEventSink, AgentLoopInput, AgentLoopResult, AgentLoopRunner, AgentLoopRuntimeHooks,
    AgentMailbox, AgentProviderAdapter, AgentToolExecutor, AgentWorkspaceDescriptor, DispatchError,
    DispatchHandle, DispatchRequest, DispatchState, HandoffStatus, HandoffValidator, MailboxError,
    RootAgentBudget, ScopedToolView, SubagentModelCatalog,
};
use crate::run_log::{
    AgentId, EventEnvelope, EventKind, MailboxNotificationKind, RunEventSink, RunId,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, Notify, Semaphore};
use tokio::task::AbortHandle;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSupervisorConfig {
    pub max_concurrent: usize,
    pub max_queued: usize,
    pub max_children_per_parent: usize,
    pub max_total_per_run: usize,
    pub max_depth: usize,
    pub child_budget: AgentLoopBudget,
    pub root_max_provider_events: usize,
    pub root_max_tool_calls: usize,
}

impl Default for AgentSupervisorConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            max_queued: 8,
            max_children_per_parent: 4,
            max_total_per_run: 8,
            max_depth: 1,
            child_budget: AgentLoopBudget::default(),
            root_max_provider_events: 1024,
            root_max_tool_calls: 160,
        }
    }
}

pub struct AgentLoopDependencies {
    pub provider: Arc<dyn AgentProviderAdapter>,
    pub tool_view: ScopedToolView,
    pub tool_executor: Arc<dyn AgentToolExecutor>,
    pub approval_client: Arc<dyn AgentApprovalClient>,
    pub workspace: AgentWorkspaceDescriptor,
    pub envelope: super::DelegationEnvelope,
    pub budget: Option<AgentLoopBudget>,
}

pub trait AgentLoopInputFactory: Send + Sync {
    fn build(&self, dispatch: &AgentDispatchRecord) -> Result<AgentLoopDependencies, String>;
}

#[derive(Clone, Debug)]
pub struct LiveAgentHandle {
    pub dispatch: DispatchHandle,
    pub cancellation: AgentCancellationToken,
    pub abort_handle: AbortHandle,
}

#[derive(Clone)]
pub struct AgentSupervisor {
    inner: Arc<SupervisorInner>,
}

struct SupervisorInner {
    run_id: RunId,
    root_agent_id: AgentId,
    sink: Arc<dyn RunEventSink>,
    scheduler: Arc<Mutex<AgentDispatchScheduler>>,
    factory: Arc<dyn AgentLoopInputFactory>,
    config: AgentSupervisorConfig,
    semaphore: Arc<Semaphore>,
    root_budget: Arc<RootAgentBudget>,
    live: AsyncMutex<BTreeMap<AgentId, LiveAgentHandle>>,
    mailboxes: Mutex<HashMap<AgentId, AgentMailbox>>,
    drain_lock: AsyncMutex<()>,
    changed: Notify,
}

impl AgentSupervisor {
    pub fn new(
        run_id: RunId,
        root_agent_id: AgentId,
        sink: Arc<dyn RunEventSink>,
        model_catalog: Arc<SubagentModelCatalog>,
        factory: Arc<dyn AgentLoopInputFactory>,
        config: AgentSupervisorConfig,
    ) -> Result<Self, AgentSupervisorError> {
        validate_config(&config)?;
        let scheduler = AgentDispatchScheduler::new(run_id.clone(), sink.clone(), model_catalog);
        Ok(Self::from_scheduler(
            run_id,
            root_agent_id,
            sink,
            scheduler,
            factory,
            config,
        ))
    }

    /// Rehydrate durable ownership. Queued children are eligible to run;
    /// anything that may have crossed a provider/effect boundary is already
    /// terminated with a validated interrupted handoff by the scheduler.
    pub fn rehydrate(
        run_id: RunId,
        root_agent_id: AgentId,
        sink: Arc<dyn RunEventSink>,
        model_catalog: Arc<SubagentModelCatalog>,
        factory: Arc<dyn AgentLoopInputFactory>,
        config: AgentSupervisorConfig,
    ) -> Result<Self, AgentSupervisorError> {
        validate_config(&config)?;
        let scheduler =
            AgentDispatchScheduler::rehydrate(run_id.clone(), sink.clone(), model_catalog)?;
        let supervisor =
            Self::from_scheduler(run_id, root_agent_id, sink, scheduler, factory, config);
        supervisor.recover_mailbox_notifications()?;
        Ok(supervisor)
    }

    fn from_scheduler(
        run_id: RunId,
        root_agent_id: AgentId,
        sink: Arc<dyn RunEventSink>,
        scheduler: AgentDispatchScheduler,
        factory: Arc<dyn AgentLoopInputFactory>,
        config: AgentSupervisorConfig,
    ) -> Self {
        Self {
            inner: Arc::new(SupervisorInner {
                run_id,
                root_agent_id,
                sink,
                scheduler: Arc::new(Mutex::new(scheduler)),
                factory,
                semaphore: Arc::new(Semaphore::new(config.max_concurrent)),
                root_budget: Arc::new(RootAgentBudget::new(
                    config.root_max_provider_events,
                    config.root_max_tool_calls,
                )),
                config,
                live: AsyncMutex::new(BTreeMap::new()),
                mailboxes: Mutex::new(HashMap::new()),
                drain_lock: AsyncMutex::new(()),
                changed: Notify::new(),
            }),
        }
    }

    pub async fn dispatch(
        &self,
        request: DispatchRequest,
    ) -> Result<DispatchHandle, AgentSupervisorError> {
        let handle = {
            let mut scheduler = self.inner.scheduler.lock().map_err(|_| poisoned())?;
            let records = scheduler.dispatch_records();
            self.validate_dispatch_limits(&records, &request)?;
            scheduler.dispatch(request)?
        };
        self.drain_ready().await?;
        Ok(handle)
    }

    pub fn state(&self, agent_id: &AgentId) -> Result<Option<DispatchState>, AgentSupervisorError> {
        Ok(self
            .inner
            .scheduler
            .lock()
            .map_err(|_| poisoned())?
            .state(agent_id)
            .cloned())
    }

    pub fn dispatches(&self) -> Result<Vec<AgentDispatchRecord>, AgentSupervisorError> {
        Ok(self
            .inner
            .scheduler
            .lock()
            .map_err(|_| poisoned())?
            .dispatch_records())
    }

    pub async fn live_handles(&self) -> BTreeMap<AgentId, LiveAgentHandle> {
        self.inner.live.lock().await.clone()
    }

    pub fn mailbox(&self, recipient: AgentId) -> Result<AgentMailbox, AgentSupervisorError> {
        let mut mailboxes = self.inner.mailboxes.lock().map_err(|_| poisoned())?;
        if let Some(mailbox) = mailboxes.get(&recipient) {
            return Ok(mailbox.clone());
        }
        let mailbox = AgentMailbox::new(
            self.inner.run_id.clone(),
            recipient.clone(),
            self.inner.sink.clone(),
        )?;
        mailboxes.insert(recipient, mailbox.clone());
        Ok(mailbox)
    }

    pub async fn interrupt(
        &self,
        agent_id: &AgentId,
        reason: impl Into<String>,
    ) -> Result<Vec<AgentId>, AgentSupervisorError> {
        let reason = reason.into();
        let records = self.dispatches()?;
        if !records
            .iter()
            .any(|record| &record.handle.agent_id == agent_id)
        {
            return Err(AgentSupervisorError::UnknownAgent(agent_id.clone()));
        }
        let mut descendants = descendants_including(&records, agent_id);
        descendants.sort();
        descendants.reverse();
        let live = self.inner.live.lock().await;
        let mut queued = Vec::new();
        for current in &descendants {
            if let Some(handle) = live.get(current) {
                handle.cancellation.cancel(reason.clone());
            } else if records.iter().any(|record| {
                &record.handle.agent_id == current && record.state == DispatchState::Queued
            }) {
                queued.push(current.clone());
            }
        }
        drop(live);
        for current in queued {
            let terminal = {
                let mut scheduler = self.inner.scheduler.lock().map_err(|_| poisoned())?;
                let record = scheduler
                    .dispatch_record(&current)
                    .ok_or_else(|| AgentSupervisorError::UnknownAgent(current.clone()))?;
                let terminal = scheduler.finish_with_handoff(
                    &record.handle,
                    validated_terminal_handoff(
                        HandoffStatus::Interrupted,
                        format!("cancelled before provider start: {reason}"),
                    )?,
                )?;
                (record, terminal)
            };
            self.notify_terminal(&terminal.0, &terminal.1)?;
        }
        self.inner.changed.notify_waiters();
        Ok(descendants)
    }

    pub async fn wait_for_idle(&self, timeout: Duration) -> Result<bool, AgentSupervisorError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let all_terminal = self
                .dispatches()?
                .iter()
                .all(|record| record.state.is_terminal());
            if all_terminal && self.inner.live.lock().await.is_empty() {
                return Ok(true);
            }
            if tokio::time::timeout_at(deadline, self.inner.changed.notified())
                .await
                .is_err()
            {
                return Ok(false);
            }
        }
    }

    pub async fn start_recovered(&self) -> Result<(), AgentSupervisorError> {
        self.drain_ready().await
    }

    async fn drain_ready(&self) -> Result<(), AgentSupervisorError> {
        let _guard = self.inner.drain_lock.lock().await;
        loop {
            let permit = match self.inner.semaphore.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => break,
            };
            let record = {
                let mut scheduler = self.inner.scheduler.lock().map_err(|_| poisoned())?;
                let Some(record) = scheduler.queued_dispatches().into_iter().next() else {
                    drop(permit);
                    break;
                };
                scheduler.transition(
                    &record.handle,
                    DispatchState::Starting,
                    Some("supervisor allocated provider concurrency".into()),
                )?;
                record
            };
            let cancellation = AgentCancellationToken::new();
            let supervisor = self.clone();
            let task_record = record.clone();
            let task_cancellation = cancellation.clone();
            let (launch, launched) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let _ = launched.await;
                let _permit = permit;
                supervisor.run_one(task_record, task_cancellation).await;
            });
            self.inner.live.lock().await.insert(
                record.handle.agent_id.clone(),
                LiveAgentHandle {
                    dispatch: record.handle,
                    cancellation,
                    abort_handle: task.abort_handle(),
                },
            );
            let _ = launch.send(());
        }
        Ok(())
    }

    async fn run_one(&self, record: AgentDispatchRecord, cancellation: AgentCancellationToken) {
        let result = self.run_one_inner(&record, cancellation).await;
        if let Err(error) = result {
            let status = match &error {
                AgentSupervisorError::Loop(AgentLoopError::CancelledBeforeBinding(_)) => {
                    HandoffStatus::Interrupted
                }
                AgentSupervisorError::Loop(AgentLoopError::TimedOutBeforeBinding) => {
                    HandoffStatus::TimedOut
                }
                _ => HandoffStatus::Failed,
            };
            let terminal = self.inner.scheduler.lock().ok().and_then(|mut scheduler| {
                let state = scheduler.state(&record.handle.agent_id)?.clone();
                if state.is_terminal() {
                    return None;
                }
                scheduler
                    .terminate_conservatively(
                        &record.handle,
                        status,
                        &format!("child loop infrastructure failed: {error}"),
                    )
                    .ok()
            });
            if let Some(terminal) = terminal {
                let _ = self.notify_terminal(&record, &terminal);
            }
        }
        self.inner.live.lock().await.remove(&record.handle.agent_id);
        self.inner.changed.notify_waiters();
        tokio::spawn(self.clone().drain_ready_task());
    }

    fn drain_ready_task(self) -> super::AgentFuture<'static, ()> {
        Box::pin(async move {
            let _ = self.drain_ready().await;
        })
    }

    async fn run_one_inner(
        &self,
        record: &AgentDispatchRecord,
        cancellation: AgentCancellationToken,
    ) -> Result<(), AgentSupervisorError> {
        let dependencies = self
            .inner
            .factory
            .build(record)
            .map_err(AgentSupervisorError::InputFactory)?;
        if dependencies.workspace.assignment != record.handle.workspace {
            return Err(AgentSupervisorError::InputFactory(
                "factory workspace differs from durable dispatch".into(),
            ));
        }
        let bridge = Arc::new(SchedulerLoopBridge {
            scheduler: self.inner.scheduler.clone(),
        });
        let AgentLoopResult { handoff, .. } = AgentLoopRunner::run(AgentLoopInput {
            handle: record.handle.clone(),
            envelope: dependencies.envelope,
            route: record.resolved_route.clone(),
            provider: dependencies.provider,
            tool_view: dependencies.tool_view,
            tool_executor: dependencies.tool_executor,
            event_sink: bridge.clone(),
            runtime_hooks: bridge,
            approval_client: dependencies.approval_client,
            workspace: dependencies.workspace,
            cancellation,
            budget: dependencies
                .budget
                .unwrap_or_else(|| self.inner.config.child_budget.clone()),
            root_budget: self.inner.root_budget.clone(),
            handoff_validator: HandoffValidator::default(),
        })
        .await?;
        let terminal = self
            .inner
            .scheduler
            .lock()
            .map_err(|_| poisoned())?
            .finish_with_handoff(&record.handle, handoff)?;
        self.notify_terminal(record, &terminal)?;
        Ok(())
    }

    fn notify_terminal(
        &self,
        record: &AgentDispatchRecord,
        terminal: &super::DispatchTerminalRecord,
    ) -> Result<(), AgentSupervisorError> {
        let EventKind::AgentHandoff(handoff) = &terminal.handoff_event.kind else {
            return Err(AgentSupervisorError::InvalidHistory(
                "terminal record does not contain a handoff".into(),
            ));
        };
        let recipient = record
            .parent_agent_id
            .clone()
            .unwrap_or_else(|| self.inner.root_agent_id.clone());
        self.mailbox(recipient)?
            .notify(MailboxNotificationKind::Handoff {
                source_agent_id: record.handle.agent_id.clone(),
                terminal_event_id: terminal.terminal_event.event_id.clone(),
                handoff_event_id: terminal.handoff_event.event_id.clone(),
                handoff: Box::new(handoff.handoff.parent_projection()),
            })?;
        self.inner.changed.notify_waiters();
        Ok(())
    }

    fn recover_mailbox_notifications(&self) -> Result<(), AgentSupervisorError> {
        let events = self.inner.sink.events(&self.inner.run_id)?;
        let notified = events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::MailboxNotification(notification) => match &notification.notification {
                    MailboxNotificationKind::Handoff {
                        source_agent_id,
                        terminal_event_id,
                        handoff_event_id,
                        ..
                    } => Some((
                        source_agent_id.clone(),
                        terminal_event_id.clone(),
                        handoff_event_id.clone(),
                    )),
                    _ => None,
                },
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        for record in self.dispatches()? {
            if !record.state.is_terminal() {
                continue;
            }
            let terminal = events.iter().rev().find(|event| {
                event.agent_id.as_ref() == Some(&record.handle.agent_id)
                    && matches!(
                        event.kind,
                        EventKind::AgentLifecycle(ref lifecycle)
                            if matches!(
                                lifecycle.state,
                                crate::run_log::AgentLifecycleState::Completed
                                    | crate::run_log::AgentLifecycleState::Failed
                                    | crate::run_log::AgentLifecycleState::Interrupted
                            )
                    )
            });
            let Some(terminal) = terminal else { continue };
            let Some(handoff_id) = terminal.caused_by.as_ref() else {
                continue;
            };
            if notified.contains(&(
                record.handle.agent_id.clone(),
                terminal.event_id.clone(),
                handoff_id.clone(),
            )) {
                continue;
            }
            let Some(handoff_event) = events
                .iter()
                .find(|event| &event.event_id == handoff_id)
                .cloned()
            else {
                continue;
            };
            self.notify_terminal(
                &record,
                &super::DispatchTerminalRecord {
                    handoff_event,
                    terminal_event: terminal.clone(),
                },
            )?;
        }
        Ok(())
    }

    fn validate_dispatch_limits(
        &self,
        records: &[AgentDispatchRecord],
        request: &DispatchRequest,
    ) -> Result<(), AgentSupervisorError> {
        if records.len() >= self.inner.config.max_total_per_run {
            return Err(AgentSupervisorError::RunAgentLimit);
        }
        let queued = records
            .iter()
            .filter(|record| record.state == DispatchState::Queued)
            .count();
        if queued >= self.inner.config.max_queued {
            return Err(AgentSupervisorError::QueueLimit);
        }
        if let Some(parent) = &request.parent_agent_id {
            let children = records
                .iter()
                .filter(|record| record.parent_agent_id.as_ref() == Some(parent))
                .count();
            if children >= self.inner.config.max_children_per_parent {
                return Err(AgentSupervisorError::ParentChildLimit(parent.clone()));
            }
            let depth = agent_depth(records, parent)? + 1;
            if depth > self.inner.config.max_depth {
                return Err(AgentSupervisorError::DepthLimit { depth });
            }
        }
        Ok(())
    }
}

struct SchedulerLoopBridge {
    scheduler: Arc<Mutex<AgentDispatchScheduler>>,
}

impl AgentLoopEventSink for SchedulerLoopBridge {
    fn record(
        &self,
        event: AgentLoopEventRecord,
    ) -> super::AgentFuture<'_, Result<EventEnvelope, AgentLoopError>> {
        Box::pin(async move {
            self.scheduler
                .lock()
                .map_err(|_| AgentLoopError::EventSink("scheduler lock poisoned".into()))?
                .record_runtime_event(
                    &event.handle,
                    event.kind,
                    event.operation_id,
                    event.provider_call_id,
                )
                .map_err(|error| AgentLoopError::EventSink(error.to_string()))
        })
    }
}

impl AgentLoopRuntimeHooks for SchedulerLoopBridge {
    fn transition(
        &self,
        handle: &DispatchHandle,
        state: DispatchState,
        detail: Option<String>,
    ) -> super::AgentFuture<'_, Result<EventEnvelope, AgentLoopError>> {
        let handle = handle.clone();
        Box::pin(async move {
            self.scheduler
                .lock()
                .map_err(|_| AgentLoopError::RuntimeHook("scheduler lock poisoned".into()))?
                .transition(&handle, state, detail)
                .map_err(|error| AgentLoopError::RuntimeHook(error.to_string()))
        })
    }
}

fn validate_config(config: &AgentSupervisorConfig) -> Result<(), AgentSupervisorError> {
    if config.max_concurrent == 0
        || config.max_queued == 0
        || config.max_children_per_parent == 0
        || config.max_total_per_run == 0
        || config.root_max_provider_events == 0
        || config.root_max_tool_calls == 0
    {
        return Err(AgentSupervisorError::InvalidConfig(
            "supervisor limits must be positive".into(),
        ));
    }
    Ok(())
}

fn agent_depth(
    records: &[AgentDispatchRecord],
    agent_id: &AgentId,
) -> Result<usize, AgentSupervisorError> {
    let by_id = records
        .iter()
        .map(|record| (&record.handle.agent_id, record))
        .collect::<BTreeMap<_, _>>();
    let mut current = Some(agent_id);
    let mut seen = BTreeSet::new();
    let mut depth = 0;
    while let Some(agent_id) = current {
        if !seen.insert(agent_id.clone()) {
            return Err(AgentSupervisorError::InvalidHistory(
                "agent ancestry contains a cycle".into(),
            ));
        }
        let record = by_id
            .get(agent_id)
            .ok_or_else(|| AgentSupervisorError::UnknownAgent(agent_id.clone()))?;
        current = record.parent_agent_id.as_ref();
        if current.is_some() {
            depth += 1;
        }
    }
    Ok(depth)
}

fn descendants_including(records: &[AgentDispatchRecord], root: &AgentId) -> Vec<AgentId> {
    let mut result = vec![root.clone()];
    let mut cursor = 0;
    while cursor < result.len() {
        let parent = result[cursor].clone();
        result.extend(
            records
                .iter()
                .filter(|record| record.parent_agent_id.as_ref() == Some(&parent))
                .map(|record| record.handle.agent_id.clone()),
        );
        cursor += 1;
    }
    result
}

fn poisoned() -> AgentSupervisorError {
    AgentSupervisorError::Poisoned
}

#[derive(Debug)]
pub enum AgentSupervisorError {
    InvalidConfig(String),
    InvalidHistory(String),
    QueueLimit,
    RunAgentLimit,
    ParentChildLimit(AgentId),
    DepthLimit { depth: usize },
    UnknownAgent(AgentId),
    InputFactory(String),
    Dispatch(DispatchError),
    Loop(AgentLoopError),
    Mailbox(MailboxError),
    RunLog(crate::run_log::RunLogError),
    Poisoned,
}

impl From<DispatchError> for AgentSupervisorError {
    fn from(value: DispatchError) -> Self {
        Self::Dispatch(value)
    }
}

impl From<AgentLoopError> for AgentSupervisorError {
    fn from(value: AgentLoopError) -> Self {
        Self::Loop(value)
    }
}

impl From<MailboxError> for AgentSupervisorError {
    fn from(value: MailboxError) -> Self {
        Self::Mailbox(value)
    }
}

impl From<crate::run_log::RunLogError> for AgentSupervisorError {
    fn from(value: crate::run_log::RunLogError) -> Self {
        Self::RunLog(value)
    }
}

impl fmt::Display for AgentSupervisorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(detail) => write!(formatter, "invalid supervisor config: {detail}"),
            Self::InvalidHistory(detail) => {
                write!(formatter, "invalid supervisor history: {detail}")
            }
            Self::QueueLimit => formatter.write_str("agent ready queue is full"),
            Self::RunAgentLimit => formatter.write_str("root run agent budget is exhausted"),
            Self::ParentChildLimit(parent) => {
                write!(formatter, "agent {parent} child budget is exhausted")
            }
            Self::DepthLimit { depth } => write!(
                formatter,
                "agent depth {depth} exceeds the configured limit"
            ),
            Self::UnknownAgent(agent) => write!(formatter, "agent {agent} is unknown"),
            Self::InputFactory(detail) => {
                write!(formatter, "could not build child loop input: {detail}")
            }
            Self::Dispatch(error) => write!(formatter, "agent scheduler: {error}"),
            Self::Loop(error) => write!(formatter, "agent loop: {error}"),
            Self::Mailbox(error) => write!(formatter, "agent mailbox: {error}"),
            Self::RunLog(error) => write!(formatter, "run log: {error}"),
            Self::Poisoned => formatter.write_str("agent supervisor lock was poisoned"),
        }
    }
}

impl std::error::Error for AgentSupervisorError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::{
        catalog_model_id, fake_provider::FakeProviderAdapter, AgentKind, AgentKindName,
        DelegationEnvelope, DenyAllAgentApprovals, ReasoningEffort, RequestedModelRoute,
        WorkspaceAssignment, WorkspaceStrategy,
    };
    use crate::ai::AiConfig;
    use crate::run_log::{
        EventActor, EventKind, InMemoryRunEventSink, MessageEvent, MessageRole, NewRunEvent,
        RunEventSink, TurnId, WorkspaceId,
    };
    use std::sync::atomic::Ordering;

    struct UnusedExecutor;

    impl AgentToolExecutor for UnusedExecutor {
        fn execute(
            &self,
            _call: super::super::AgentToolCall,
        ) -> super::super::AgentFuture<
            '_,
            Result<super::super::AgentToolResult, super::super::AgentToolError>,
        > {
            Box::pin(async { Err(super::super::AgentToolError::new("unexpected tool request")) })
        }
    }

    struct FakeInputFactory {
        out_of_order: FakeProviderAdapter,
        delayed: FakeProviderAdapter,
        failure: FakeProviderAdapter,
        timeout: FakeProviderAdapter,
    }

    impl FakeInputFactory {
        fn new(tick: Duration) -> Self {
            Self {
                out_of_order: FakeProviderAdapter::new("out_of_order_completion")
                    .with_tick_duration(tick),
                delayed: FakeProviderAdapter::new("delayed_completion").with_tick_duration(tick),
                failure: FakeProviderAdapter::new("tool_failure").with_tick_duration(tick),
                timeout: FakeProviderAdapter::new("timeout").with_tick_duration(tick),
            }
        }
    }

    impl AgentLoopInputFactory for FakeInputFactory {
        fn build(&self, dispatch: &AgentDispatchRecord) -> Result<AgentLoopDependencies, String> {
            let provider: Arc<dyn AgentProviderAdapter> = if dispatch.objective.contains("fast") {
                Arc::new(self.out_of_order.clone().for_call("fast-second"))
            } else if dispatch.objective.contains("slow") {
                Arc::new(self.out_of_order.clone().for_call("slow-first"))
            } else if dispatch.objective.contains("fail") {
                Arc::new(self.failure.clone())
            } else if dispatch.objective.contains("timeout") {
                Arc::new(self.timeout.clone())
            } else {
                Arc::new(self.delayed.clone())
            };
            Ok(AgentLoopDependencies {
                provider,
                tool_view: ScopedToolView::default(),
                tool_executor: Arc::new(UnusedExecutor),
                approval_client: Arc::new(DenyAllAgentApprovals),
                workspace: AgentWorkspaceDescriptor {
                    assignment: dispatch.handle.workspace.clone(),
                    root: None,
                    read_only: true,
                },
                envelope: DelegationEnvelope::objective(dispatch.objective.clone()),
                budget: None,
            })
        }
    }

    fn catalog() -> Arc<SubagentModelCatalog> {
        Arc::new(SubagentModelCatalog::from_config(&AiConfig::default()).unwrap())
    }

    fn request(objective: &str) -> DispatchRequest {
        DispatchRequest {
            objective: objective.into(),
            role: AgentKind::built_in(AgentKindName::Explorer),
            requested_route: RequestedModelRoute::exact(
                catalog_model_id("local", "qwen2.5-coder:7b"),
                ReasoningEffort::none(),
            ),
            parent_agent_id: None,
            causing_turn_id: None,
            caused_by_event: None,
            workspace: WorkspaceAssignment {
                workspace_id: WorkspaceId::new(),
                strategy: WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None },
            },
        }
    }

    fn supervisor_with(
        sink: Arc<InMemoryRunEventSink>,
        factory: Arc<FakeInputFactory>,
        config: AgentSupervisorConfig,
    ) -> (AgentSupervisor, RunId, AgentId) {
        let run_id = RunId::new();
        let root = AgentId::new();
        let supervisor = AgentSupervisor::new(
            run_id.clone(),
            root.clone(),
            sink,
            catalog(),
            factory,
            config,
        )
        .unwrap();
        (supervisor, run_id, root)
    }

    #[tokio::test]
    async fn runs_three_children_concurrently_and_finishes_out_of_order() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let factory = Arc::new(FakeInputFactory::new(Duration::from_millis(15)));
        let maximum_active = factory.out_of_order.maximum_active_counter();
        let (supervisor, run_id, root) =
            supervisor_with(sink.clone(), factory, AgentSupervisorConfig::default());
        let slow = supervisor.dispatch(request("slow one")).await.unwrap();
        let fast = supervisor.dispatch(request("fast two")).await.unwrap();
        let slow_three = supervisor.dispatch(request("slow three")).await.unwrap();
        assert!(supervisor
            .wait_for_idle(Duration::from_secs(3))
            .await
            .unwrap());

        assert!(maximum_active.load(Ordering::Acquire) >= 3);
        for handle in [&slow, &fast, &slow_three] {
            assert_eq!(
                supervisor.state(&handle.agent_id).unwrap(),
                Some(DispatchState::Completed)
            );
        }
        let terminal_sequence = |agent_id: &AgentId| {
            sink.events(&run_id)
                .unwrap()
                .into_iter()
                .find(|event| {
                    event.agent_id.as_ref() == Some(agent_id)
                        && matches!(
                            event.kind,
                            EventKind::AgentLifecycle(ref lifecycle)
                                if lifecycle.state == crate::run_log::AgentLifecycleState::Completed
                        )
                })
                .unwrap()
                .sequence
        };
        assert!(terminal_sequence(&fast.agent_id) < terminal_sequence(&slow.agent_id));
        assert_eq!(
            supervisor.mailbox(root).unwrap().pending().unwrap().len(),
            3
        );
    }

    #[tokio::test]
    async fn failure_and_timeout_are_independent_and_durably_mailed() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let factory = Arc::new(FakeInputFactory::new(Duration::from_millis(2)));
        let (supervisor, _, root) =
            supervisor_with(sink, factory, AgentSupervisorConfig::default());
        let failed = supervisor
            .dispatch(request("fail independently"))
            .await
            .unwrap();
        let timed_out = supervisor
            .dispatch(request("timeout independently"))
            .await
            .unwrap();
        let completed = supervisor
            .dispatch(request("complete independently"))
            .await
            .unwrap();
        assert!(supervisor
            .wait_for_idle(Duration::from_secs(2))
            .await
            .unwrap());

        assert_eq!(
            supervisor.state(&failed.agent_id).unwrap(),
            Some(DispatchState::Failed)
        );
        assert_eq!(
            supervisor.state(&timed_out.agent_id).unwrap(),
            Some(DispatchState::Interrupted)
        );
        assert_eq!(
            supervisor.state(&completed.agent_id).unwrap(),
            Some(DispatchState::Completed)
        );
        let entries = supervisor.mailbox(root).unwrap().pending().unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|entry| matches!(
            &entry.notification,
            MailboxNotificationKind::Handoff { handoff, .. }
                if handoff.status == HandoffStatus::Failed
        )));
        assert!(entries.iter().any(|entry| matches!(
            &entry.notification,
            MailboxNotificationKind::Handoff { handoff, .. }
                if handoff.status == HandoffStatus::TimedOut
        )));
    }

    #[tokio::test]
    async fn malformed_handoff_never_creates_completed_lifecycle() {
        struct MalformedFactory(FakeProviderAdapter);
        impl AgentLoopInputFactory for MalformedFactory {
            fn build(
                &self,
                dispatch: &AgentDispatchRecord,
            ) -> Result<AgentLoopDependencies, String> {
                Ok(AgentLoopDependencies {
                    provider: Arc::new(self.0.clone()),
                    tool_view: ScopedToolView::default(),
                    tool_executor: Arc::new(UnusedExecutor),
                    approval_client: Arc::new(DenyAllAgentApprovals),
                    workspace: AgentWorkspaceDescriptor {
                        assignment: dispatch.handle.workspace.clone(),
                        root: None,
                        read_only: true,
                    },
                    envelope: DelegationEnvelope::objective(dispatch.objective.clone()),
                    budget: None,
                })
            }
        }
        let sink = Arc::new(InMemoryRunEventSink::new());
        let run_id = RunId::new();
        let root = AgentId::new();
        let supervisor = AgentSupervisor::new(
            run_id.clone(),
            root,
            sink.clone(),
            catalog(),
            Arc::new(MalformedFactory(FakeProviderAdapter::new(
                "malformed_handoff",
            ))),
            AgentSupervisorConfig::default(),
        )
        .unwrap();
        let child = supervisor.dispatch(request("malformed")).await.unwrap();
        assert!(supervisor
            .wait_for_idle(Duration::from_secs(1))
            .await
            .unwrap());
        assert_eq!(
            supervisor.state(&child.agent_id).unwrap(),
            Some(DispatchState::Failed)
        );
        assert!(!sink.events(&run_id).unwrap().iter().any(|event| matches!(
            event.kind,
            EventKind::AgentLifecycle(ref lifecycle)
                if lifecycle.agent_id == child.agent_id
                    && lifecycle.state == crate::run_log::AgentLifecycleState::Completed
        )));
    }

    #[tokio::test]
    async fn queue_and_concurrency_limits_fail_before_allocating_extra_agent() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let factory = Arc::new(FakeInputFactory::new(Duration::from_millis(30)));
        let maximum_active = factory.delayed.maximum_active_counter();
        let config = AgentSupervisorConfig {
            max_concurrent: 1,
            max_queued: 1,
            ..AgentSupervisorConfig::default()
        };
        let (supervisor, _, _) = supervisor_with(sink, factory, config);
        supervisor.dispatch(request("first")).await.unwrap();
        supervisor.dispatch(request("second")).await.unwrap();
        assert!(matches!(
            supervisor.dispatch(request("third")).await,
            Err(AgentSupervisorError::QueueLimit)
        ));
        assert_eq!(supervisor.dispatches().unwrap().len(), 2);
        assert!(supervisor
            .wait_for_idle(Duration::from_secs(2))
            .await
            .unwrap());
        assert_eq!(maximum_active.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn root_and_child_budgets_terminalize_with_validated_partial_handoffs() {
        let root_sink = Arc::new(InMemoryRunEventSink::new());
        let root_factory = Arc::new(FakeInputFactory::new(Duration::from_millis(2)));
        let root_config = AgentSupervisorConfig {
            max_concurrent: 1,
            root_max_provider_events: 2,
            ..AgentSupervisorConfig::default()
        };
        let (root_limited, _, root_recipient) =
            supervisor_with(root_sink, root_factory, root_config);
        let first = root_limited
            .dispatch(request("first budget"))
            .await
            .unwrap();
        let second = root_limited
            .dispatch(request("second budget"))
            .await
            .unwrap();
        assert!(root_limited
            .wait_for_idle(Duration::from_secs(1))
            .await
            .unwrap());
        assert_eq!(
            root_limited.state(&first.agent_id).unwrap(),
            Some(DispatchState::Completed)
        );
        assert_eq!(
            root_limited.state(&second.agent_id).unwrap(),
            Some(DispatchState::Interrupted)
        );
        assert_eq!(
            root_limited
                .mailbox(root_recipient)
                .unwrap()
                .pending()
                .unwrap()
                .len(),
            2
        );

        let child_sink = Arc::new(InMemoryRunEventSink::new());
        let child_factory = Arc::new(FakeInputFactory::new(Duration::from_millis(20)));
        let child_config = AgentSupervisorConfig {
            child_budget: AgentLoopBudget {
                timeout: Duration::from_millis(5),
                max_provider_events: 1,
                max_tool_calls: 1,
            },
            ..AgentSupervisorConfig::default()
        };
        let (child_limited, _, child_recipient) =
            supervisor_with(child_sink, child_factory, child_config);
        let child = child_limited
            .dispatch(request("child budget"))
            .await
            .unwrap();
        assert!(child_limited
            .wait_for_idle(Duration::from_secs(1))
            .await
            .unwrap());
        assert_eq!(
            child_limited.state(&child.agent_id).unwrap(),
            Some(DispatchState::Interrupted)
        );
        let entries = child_limited
            .mailbox(child_recipient)
            .unwrap()
            .pending()
            .unwrap();
        assert!(matches!(
            &entries[0].notification,
            MailboxNotificationKind::Handoff { handoff, .. }
                if matches!(handoff.status, HandoffStatus::Interrupted | HandoffStatus::TimedOut)
        ));
    }

    #[tokio::test]
    async fn parent_interrupt_cascades_to_live_descendant() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let factory = Arc::new(FakeInputFactory::new(Duration::from_millis(40)));
        let config = AgentSupervisorConfig {
            max_depth: 1,
            ..AgentSupervisorConfig::default()
        };
        let (supervisor, run_id, _) = supervisor_with(sink.clone(), factory, config);
        let mut parent_request = request("parent delayed");
        parent_request.role = AgentKind::built_in(AgentKindName::Planner);
        let parent = supervisor.dispatch(parent_request).await.unwrap();
        let turn_id = TurnId::new();
        let last_parent = sink
            .events(&run_id)
            .unwrap()
            .into_iter()
            .rev()
            .find(|event| event.agent_id.as_ref() == Some(&parent.agent_id))
            .unwrap();
        let cause = sink
            .append(NewRunEvent {
                run_id: run_id.clone(),
                caused_by: Some(last_parent.event_id),
                operation_id: None,
                provider_call_id: None,
                actor: EventActor::Agent(parent.agent_id.clone()),
                agent_id: Some(parent.agent_id.clone()),
                turn_id: Some(turn_id.clone()),
                workspace_id: Some(parent.workspace.workspace_id.clone()),
                branch_id: None,
                kind: EventKind::Message(MessageEvent {
                    role: MessageRole::Agent,
                    content: "dispatch child".into(),
                }),
            })
            .unwrap();
        let mut child_request = request("child delayed");
        child_request.parent_agent_id = Some(parent.agent_id.clone());
        child_request.causing_turn_id = Some(turn_id);
        child_request.caused_by_event = Some(cause.event_id);
        let child = supervisor.dispatch(child_request).await.unwrap();
        while supervisor.live_handles().await.len() < 2 {
            tokio::task::yield_now().await;
        }
        let interrupted = supervisor
            .interrupt(&parent.agent_id, "stop subtree")
            .await
            .unwrap();
        assert_eq!(interrupted.len(), 2);
        assert!(supervisor
            .wait_for_idle(Duration::from_secs(2))
            .await
            .unwrap());
        assert_eq!(
            supervisor.state(&parent.agent_id).unwrap(),
            Some(DispatchState::Interrupted)
        );
        assert_eq!(
            supervisor.state(&child.agent_id).unwrap(),
            Some(DispatchState::Interrupted)
        );
    }

    #[tokio::test]
    async fn recovery_interrupts_in_flight_and_does_not_duplicate_existing_mail() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let run_id = RunId::new();
        let root = AgentId::new();
        let mut scheduler = AgentDispatchScheduler::new(run_id.clone(), sink.clone(), catalog());
        let queued = scheduler.dispatch(request("queued recovery")).unwrap();
        let running = scheduler.dispatch(request("running recovery")).unwrap();
        scheduler
            .transition(&running, DispatchState::Starting, None)
            .unwrap();
        scheduler
            .transition(&running, DispatchState::Running, None)
            .unwrap();
        drop(scheduler);

        let factory = Arc::new(FakeInputFactory::new(Duration::from_millis(2)));
        let recovered = AgentSupervisor::rehydrate(
            run_id.clone(),
            root.clone(),
            sink.clone(),
            catalog(),
            factory.clone(),
            AgentSupervisorConfig::default(),
        )
        .unwrap();
        assert_eq!(
            recovered.state(&running.agent_id).unwrap(),
            Some(DispatchState::Interrupted)
        );
        assert_eq!(
            recovered
                .mailbox(root.clone())
                .unwrap()
                .pending()
                .unwrap()
                .len(),
            1
        );
        recovered.start_recovered().await.unwrap();
        assert!(recovered
            .wait_for_idle(Duration::from_secs(1))
            .await
            .unwrap());
        assert_eq!(
            recovered.state(&queued.agent_id).unwrap(),
            Some(DispatchState::Completed)
        );
        assert_eq!(
            recovered
                .mailbox(root.clone())
                .unwrap()
                .pending()
                .unwrap()
                .len(),
            2
        );

        drop(recovered);
        let reopened = AgentSupervisor::rehydrate(
            run_id,
            root.clone(),
            sink,
            catalog(),
            factory,
            AgentSupervisorConfig::default(),
        )
        .unwrap();
        assert_eq!(reopened.mailbox(root).unwrap().pending().unwrap().len(), 2);
    }
}
