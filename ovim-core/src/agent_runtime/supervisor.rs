//! Concurrent control plane for durable delegated-agent dispatches.

use super::{
    validated_terminal_handoff, AgentApprovalClient, AgentCancellationToken, AgentDispatchRecord,
    AgentDispatchScheduler, AgentLoopBudget, AgentLoopError, AgentLoopEventRecord,
    AgentLoopEventSink, AgentLoopInput, AgentLoopResult, AgentLoopRunner, AgentLoopRuntimeHooks,
    AgentMailbox, AgentMessageClaimOutcome, AgentMessageError, AgentMessageQueue,
    AgentMessageRecord, AgentProviderAdapter, AgentProviderError, AgentProviderEvent,
    AgentProviderFollowup, AgentProviderSession, AgentProviderStart, AgentToolExecutor,
    AgentToolResult, AgentWorkspaceDescriptor, DispatchError, DispatchHandle, DispatchRequest,
    DispatchState, FollowupAgentHandle, FollowupAgentRequest, HandoffStatus, HandoffValidator,
    MailboxError, RootAgentBudget, ScopedToolView, SubagentModelCatalog,
};
use crate::run_log::{
    AgentId, EventEnvelope, EventId, EventKind, MailboxNotificationKind, RunEventSink, RunId,
    TurnId,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendAgentMessageRequest {
    pub sender_agent_id: AgentId,
    pub recipient_agent_id: AgentId,
    pub causing_turn_id: TurnId,
    pub caused_by_event: EventId,
    pub content: String,
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
    idle_sessions: Mutex<HashMap<AgentId, Box<dyn AgentProviderSession>>>,
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
        let mut scheduler =
            AgentDispatchScheduler::new(run_id.clone(), sink.clone(), model_catalog);
        scheduler.set_external_parent(root_agent_id.clone());
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
        let mut scheduler =
            AgentDispatchScheduler::rehydrate(run_id.clone(), sink.clone(), model_catalog)?;
        scheduler.set_external_parent(root_agent_id.clone());
        let supervisor =
            Self::from_scheduler(run_id, root_agent_id, sink, scheduler, factory, config);
        supervisor.recover_agent_messages()?;
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
                idle_sessions: Mutex::new(HashMap::new()),
                drain_lock: AsyncMutex::new(()),
                changed: Notify::new(),
            }),
        }
    }

    pub async fn dispatch(
        &self,
        request: DispatchRequest,
    ) -> Result<DispatchHandle, AgentSupervisorError> {
        let handle = self.allocate(request)?;
        self.drain_ready().await?;
        Ok(handle)
    }

    /// Durably allocate and queue a child, then drain in an independent task.
    /// This is the parent-tool boundary: the caller receives the stable child
    /// identity without waiting for provider startup or completion.
    pub fn dispatch_nonblocking(
        &self,
        request: DispatchRequest,
    ) -> Result<DispatchHandle, AgentSupervisorError> {
        let handle = self.allocate(request)?;
        let supervisor = self.clone();
        tokio::spawn(async move {
            if let Err(error) = supervisor.drain_ready().await {
                crate::log_warn!("agent_runtime", "could not drain child queue: {error}");
            }
        });
        Ok(handle)
    }

    fn allocate(&self, request: DispatchRequest) -> Result<DispatchHandle, AgentSupervisorError> {
        let mut scheduler = self.inner.scheduler.lock().map_err(|_| poisoned())?;
        let records = scheduler.dispatch_records();
        self.validate_dispatch_limits(&records, &request)?;
        scheduler.dispatch(request).map_err(Into::into)
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

    pub fn message_queue(
        &self,
        recipient: AgentId,
    ) -> Result<AgentMessageQueue, AgentSupervisorError> {
        Ok(AgentMessageQueue::new(
            self.inner.run_id.clone(),
            recipient.clone(),
            self.inner.sink.clone(),
            self.mailbox(recipient)?,
        )?)
    }

    pub fn messages(&self) -> Result<Vec<AgentMessageRecord>, AgentSupervisorError> {
        let projection =
            super::AgentMessageProjection::rehydrate(&self.inner.sink.events(&self.inner.run_id)?)
                .map_err(AgentMessageError::Projection)?;
        Ok(projection.messages().cloned().collect())
    }

    /// Queue one bounded parent-authored message for a currently active child.
    /// The provider wrapper accepts it only between provider/tool operations;
    /// terminal and merely queued children reject steering explicitly.
    pub fn send_message(
        &self,
        request: SendAgentMessageRequest,
    ) -> Result<AgentMessageRecord, AgentSupervisorError> {
        let record = self
            .inner
            .scheduler
            .lock()
            .map_err(|_| poisoned())?
            .dispatch_record(&request.recipient_agent_id)
            .ok_or_else(|| {
                AgentSupervisorError::UnknownAgent(request.recipient_agent_id.clone())
            })?;
        let expected_parent = record
            .parent_agent_id
            .as_ref()
            .unwrap_or(&self.inner.root_agent_id);
        if expected_parent != &request.sender_agent_id {
            return Err(AgentSupervisorError::MessageSenderMismatch {
                recipient: request.recipient_agent_id,
                sender: request.sender_agent_id,
            });
        }
        if !matches!(
            record.state,
            DispatchState::Starting
                | DispatchState::Running
                | DispatchState::WaitingForAgent
                | DispatchState::WaitingForTool
                | DispatchState::WaitingForUser
        ) {
            return Err(AgentSupervisorError::MessageTargetNotLive {
                agent_id: request.recipient_agent_id,
                state: record.state,
            });
        }
        let queued = self.message_queue(record.handle.agent_id)?.queue(
            request.sender_agent_id,
            request.causing_turn_id,
            request.caused_by_event,
            request.content,
        )?;
        self.inner.changed.notify_waiters();
        Ok(queued)
    }

    pub async fn followup_agent(
        &self,
        mut request: FollowupAgentRequest,
    ) -> Result<FollowupAgentHandle, AgentSupervisorError> {
        if self.inner.live.lock().await.contains_key(&request.agent_id) {
            return Err(AgentSupervisorError::FollowupTargetBusy(request.agent_id));
        }
        let record = self
            .inner
            .scheduler
            .lock()
            .map_err(|_| poisoned())?
            .dispatch_record(&request.agent_id)
            .ok_or_else(|| AgentSupervisorError::UnknownAgent(request.agent_id.clone()))?;
        let dependencies = self
            .inner
            .factory
            .build(&record)
            .map_err(AgentSupervisorError::InputFactory)?;
        let inherited_budget = dependencies
            .budget
            .unwrap_or_else(|| self.inner.config.child_budget.clone());
        if request.budget.timeout > inherited_budget.timeout
            || request.budget.max_provider_events > inherited_budget.max_provider_events
            || request.budget.max_tool_calls > inherited_budget.max_tool_calls
        {
            return Err(AgentSupervisorError::FollowupBudgetWidening);
        }
        let retained = self
            .inner
            .idle_sessions
            .lock()
            .map_err(|_| poisoned())?
            .get(&request.agent_id)
            .is_some_and(|session| session.can_followup());
        request.retained_session_requested = retained;
        let followup = self
            .inner
            .scheduler
            .lock()
            .map_err(|_| poisoned())?
            .begin_followup(request)?;
        self.drain_ready().await?;
        Ok(followup)
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
        // Classify and terminate under `live` held for the whole pass, matching
        // drain_ready's live -> scheduler order. Because promotion holds `live`
        // across Queued -> Starting, any descendant absent from the live map
        // here is provably not mid-start: if it is still non-terminal it is safe
        // to terminate before it ever reaches the provider. Terminal records are
        // collected and notified after the locks are released.
        let mut terminals = Vec::new();
        {
            let live = self.inner.live.lock().await;
            let mut scheduler = self.inner.scheduler.lock().map_err(|_| poisoned())?;
            for current in &descendants {
                if let Some(handle) = live.get(current) {
                    handle.cancellation.cancel(reason.clone());
                    continue;
                }
                let Some(state) = scheduler.state(current) else {
                    continue;
                };
                if state.is_terminal() {
                    continue;
                }
                let record = scheduler
                    .dispatch_record(current)
                    .ok_or_else(|| AgentSupervisorError::UnknownAgent(current.clone()))?;
                let terminal = scheduler.finish_with_handoff(
                    &record.handle,
                    validated_terminal_handoff(
                        HandoffStatus::Interrupted,
                        format!("cancelled before provider start: {reason}"),
                    )?,
                )?;
                terminals.push((record, terminal));
            }
        }
        for (record, terminal) in terminals {
            self.notify_terminal(&record, &terminal)?;
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
            // Hold `live` across the entire Queued -> Starting promotion and the
            // live-map insertion. A child is therefore present in `live` for the
            // whole time it is not Queued, closing the window where interrupt()
            // could observe a Starting child that is absent from the map (and so
            // neither cancel nor terminate it while still reporting it stopped).
            // Lock order is live -> scheduler everywhere; the gate below still
            // prevents provider execution until after publication.
            let mut live = self.inner.live.lock().await;
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
            live.insert(
                record.handle.agent_id.clone(),
                LiveAgentHandle {
                    dispatch: record.handle,
                    cancellation,
                    abort_handle: task.abort_handle(),
                },
            );
            // Publish before releasing the gate so the spawned task cannot reach
            // run_one (and its own `live` access) until the handle is in place.
            drop(live);
            let _ = launch.send(());
        }
        Ok(())
    }

    async fn run_one(&self, record: AgentDispatchRecord, cancellation: AgentCancellationToken) {
        let result = self.run_one_inner(&record, cancellation).await;
        if let Err(error) = result {
            if let Ok(mut sessions) = self.inner.idle_sessions.lock() {
                sessions.remove(&record.handle.agent_id);
            }
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
        let mut dependencies = self
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
        let message_queue = self.message_queue(record.handle.agent_id.clone())?;
        let provider: Arc<dyn AgentProviderAdapter> = Arc::new(MessageDeliveringProvider {
            inner: dependencies.provider,
            queue: message_queue.clone(),
        });
        let retained_session = if record.followup.is_some() {
            self.inner
                .idle_sessions
                .lock()
                .map_err(|_| poisoned())?
                .remove(&record.handle.agent_id)
        } else {
            None
        };
        if let Some(followup) = &record.followup {
            dependencies.envelope.objective = followup.objective.clone();
            dependencies.envelope.parent_brief = Some(format!(
                "Previous validated handoff: {}",
                followup.prior_handoff_summary
            ));
            if let Some(identity) = dependencies.envelope.identity.as_mut() {
                identity.causing_turn_id = followup.parent_turn_id.clone();
                identity.causing_event_id = followup.parent_event_id.clone();
            }
        }
        let budget = record
            .followup
            .as_ref()
            .map(|followup| followup.budget.clone())
            .or(dependencies.budget)
            .unwrap_or_else(|| self.inner.config.child_budget.clone());
        let input = AgentLoopInput {
            handle: record.handle.clone(),
            envelope: dependencies.envelope,
            route: record.resolved_route.clone(),
            provider,
            tool_view: dependencies.tool_view,
            tool_executor: dependencies.tool_executor,
            event_sink: bridge.clone(),
            runtime_hooks: bridge,
            approval_client: dependencies.approval_client,
            workspace: dependencies.workspace,
            cancellation,
            budget,
            root_budget: self.inner.root_budget.clone(),
            handoff_validator: HandoffValidator::default(),
            turn_generation: record.turn_generation,
        };
        let AgentLoopResult {
            handoff,
            workspace_warnings,
            retained_session: next_idle_session,
            ..
        } = if let (Some(session), Some(followup)) = (retained_session, record.followup.as_ref()) {
            AgentLoopRunner::run_followup(
                input,
                session,
                AgentProviderFollowup {
                    handle: record.handle.clone(),
                    followup_turn_id: followup.followup_turn_id.clone(),
                    turn_generation: followup.turn_generation,
                    objective: followup.objective.clone(),
                },
            )
            .await?
        } else {
            AgentLoopRunner::run(input).await?
        };
        for message in message_queue.unsettled()? {
            message_queue.reject_queued(
                &message.message_event_id,
                "child reached a terminal provider boundary before message delivery",
            )?;
        }
        let terminal = self
            .inner
            .scheduler
            .lock()
            .map_err(|_| poisoned())?
            .finish_with_handoff_and_warnings(&record.handle, handoff, workspace_warnings)?;
        if let Some(session) = next_idle_session {
            self.inner
                .idle_sessions
                .lock()
                .map_err(|_| poisoned())?
                .insert(record.handle.agent_id.clone(), session);
        }
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
                handoff: Box::new(
                    handoff
                        .handoff
                        .parent_projection_with_workspace_warnings(&handoff.workspace_warnings),
                ),
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
        let records = self
            .dispatches()?
            .into_iter()
            .map(|record| (record.handle.agent_id.clone(), record))
            .collect::<BTreeMap<_, _>>();
        // One stable AgentId may have several terminal generations. Recover
        // every missing handoff notification, not only the latest terminal
        // event, while the mailbox projection prevents duplicate consumption.
        for terminal in events.iter().filter(|event| {
            matches!(
                event.kind,
                EventKind::AgentLifecycle(ref lifecycle)
                    if matches!(
                        lifecycle.state,
                        crate::run_log::AgentLifecycleState::Completed
                            | crate::run_log::AgentLifecycleState::Failed
                            | crate::run_log::AgentLifecycleState::Interrupted
                    )
            )
        }) {
            let Some(record) = terminal
                .agent_id
                .as_ref()
                .and_then(|agent_id| records.get(agent_id))
            else {
                continue;
            };
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
                record,
                &super::DispatchTerminalRecord {
                    handoff_event,
                    terminal_event: terminal.clone(),
                },
            )?;
        }
        Ok(())
    }

    fn recover_agent_messages(&self) -> Result<(), AgentSupervisorError> {
        for record in self.dispatches()? {
            self.message_queue(record.handle.agent_id)?
                .reconcile_after_restart()?;
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
            let depth = if parent == &self.inner.root_agent_id {
                1
            } else {
                agent_depth(records, parent)? + 1
            };
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

#[derive(Clone)]
struct MessageDeliveringProvider {
    inner: Arc<dyn AgentProviderAdapter>,
    queue: AgentMessageQueue,
}

impl AgentProviderAdapter for MessageDeliveringProvider {
    fn start(
        &self,
        request: AgentProviderStart,
    ) -> super::AgentFuture<'_, Result<Box<dyn AgentProviderSession>, AgentProviderError>> {
        Box::pin(async move {
            let session = self.inner.start(request).await?;
            Ok(Box::new(MessageDeliveringSession {
                inner: session,
                queue: self.queue.clone(),
            }) as Box<dyn AgentProviderSession>)
        })
    }
}

struct MessageDeliveringSession {
    inner: Box<dyn AgentProviderSession>,
    queue: AgentMessageQueue,
}

impl MessageDeliveringSession {
    async fn deliver_pending(&mut self) -> Result<(), AgentProviderError> {
        loop {
            let Some(message) = self
                .queue
                .queued()
                .map_err(|error| AgentProviderError::new(error.to_string()))?
                .into_iter()
                .next()
            else {
                return Ok(());
            };
            let session_id = self.inner.binding().session_id.clone();
            match self
                .queue
                .claim(&message.message_event_id, &session_id)
                .map_err(|error| AgentProviderError::new(error.to_string()))?
            {
                AgentMessageClaimOutcome::Claimed {
                    delivery_event_id, ..
                } => {
                    let result = self
                        .inner
                        .deliver_message(&message.message_event_id, &message.content)
                        .await
                        .map_err(|error| error.to_string());
                    self.queue
                        .finish_delivery(&message.message_event_id, &delivery_event_id, result)
                        .map_err(|error| AgentProviderError::new(error.to_string()))?;
                }
                AgentMessageClaimOutcome::AlreadyClaimed(_) => {
                    self.queue
                        .reject_queued(
                            &message.message_event_id,
                            "message delivery was already attempted at an ambiguous boundary",
                        )
                        .map_err(|error| AgentProviderError::new(error.to_string()))?;
                }
                AgentMessageClaimOutcome::Terminal(_) => {
                    self.queue
                        .reject_queued(
                            &message.message_event_id,
                            "message delivery was already terminal",
                        )
                        .map_err(|error| AgentProviderError::new(error.to_string()))?;
                }
            }
        }
    }
}

impl AgentProviderSession for MessageDeliveringSession {
    fn binding(&self) -> &super::ProviderBinding {
        self.inner.binding()
    }

    fn next_event(
        &mut self,
    ) -> super::AgentFuture<'_, Result<AgentProviderEvent, AgentProviderError>> {
        Box::pin(async move {
            self.deliver_pending().await?;
            self.inner.next_event().await
        })
    }

    fn submit_tool_result(
        &mut self,
        tool_call_id: &str,
        result: &AgentToolResult,
    ) -> super::AgentFuture<'_, Result<(), AgentProviderError>> {
        let tool_call_id = tool_call_id.to_string();
        let result = result.clone();
        Box::pin(async move {
            self.inner
                .submit_tool_result(&tool_call_id, &result)
                .await?;
            self.deliver_pending().await
        })
    }

    fn deliver_message(
        &mut self,
        message_event_id: &EventId,
        content: &str,
    ) -> super::AgentFuture<'_, Result<(), AgentProviderError>> {
        self.inner.deliver_message(message_event_id, content)
    }

    fn can_followup(&self) -> bool {
        self.inner.can_followup()
    }

    fn start_followup(
        &mut self,
        followup: &super::AgentProviderFollowup,
    ) -> super::AgentFuture<'_, Result<(), AgentProviderError>> {
        self.inner.start_followup(followup)
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
    DepthLimit {
        depth: usize,
    },
    UnknownAgent(AgentId),
    MessageSenderMismatch {
        recipient: AgentId,
        sender: AgentId,
    },
    MessageTargetNotLive {
        agent_id: AgentId,
        state: DispatchState,
    },
    FollowupTargetBusy(AgentId),
    FollowupBudgetWidening,
    InputFactory(String),
    Dispatch(DispatchError),
    Loop(AgentLoopError),
    Mailbox(MailboxError),
    Message(AgentMessageError),
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

impl From<AgentMessageError> for AgentSupervisorError {
    fn from(value: AgentMessageError) -> Self {
        Self::Message(value)
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
            Self::MessageSenderMismatch { recipient, sender } => write!(
                formatter,
                "agent {sender} is not the parent authorized to message child {recipient}"
            ),
            Self::MessageTargetNotLive { agent_id, state } => write!(
                formatter,
                "agent {agent_id} cannot receive a message while in state {state:?}"
            ),
            Self::FollowupTargetBusy(agent_id) => {
                write!(
                    formatter,
                    "agent {agent_id} has not reached an idle boundary"
                )
            }
            Self::FollowupBudgetWidening => formatter
                .write_str("follow-up budget cannot exceed the child turn's inherited budget"),
            Self::InputFactory(detail) => {
                write!(formatter, "could not build child loop input: {detail}")
            }
            Self::Dispatch(error) => write!(formatter, "agent scheduler: {error}"),
            Self::Loop(error) => write!(formatter, "agent loop: {error}"),
            Self::Mailbox(error) => write!(formatter, "agent mailbox: {error}"),
            Self::Message(error) => write!(formatter, "agent message: {error}"),
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
                    warnings: Vec::new(),
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
            task_name: format!("task_{}", objective.replace(' ', "_")),
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

    fn parent_cause(sink: &dyn RunEventSink, run_id: &RunId, root: &AgentId) -> (TurnId, EventId) {
        let turn_id = TurnId::new();
        let event = sink
            .append(NewRunEvent {
                run_id: run_id.clone(),
                caused_by: None,
                operation_id: None,
                provider_call_id: None,
                actor: EventActor::Agent(root.clone()),
                agent_id: Some(root.clone()),
                turn_id: Some(turn_id.clone()),
                workspace_id: None,
                branch_id: None,
                kind: EventKind::Message(MessageEvent {
                    role: MessageRole::Agent,
                    content: "message child".into(),
                }),
            })
            .unwrap();
        (turn_id, event.event_id)
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
    async fn parent_message_delivers_once_at_a_provider_boundary_and_completed_target_rejects() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let factory = Arc::new(FakeInputFactory::new(Duration::from_millis(30)));
        let (supervisor, run_id, root) =
            supervisor_with(sink.clone(), factory, AgentSupervisorConfig::default());
        let (turn_id, caused_by_event) = parent_cause(&*sink, &run_id, &root);
        let child = supervisor
            .dispatch_nonblocking(request("message boundary"))
            .unwrap();
        loop {
            if matches!(
                supervisor.state(&child.agent_id).unwrap(),
                Some(DispatchState::Starting | DispatchState::Running)
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
        let message = supervisor
            .send_message(SendAgentMessageRequest {
                sender_agent_id: root.clone(),
                recipient_agent_id: child.agent_id.clone(),
                causing_turn_id: turn_id.clone(),
                caused_by_event: caused_by_event.clone(),
                content: "Also inspect the restart edge.".into(),
            })
            .unwrap();
        assert!(supervisor
            .wait_for_idle(Duration::from_secs(2))
            .await
            .unwrap());
        let projection = supervisor
            .message_queue(child.agent_id.clone())
            .unwrap()
            .projection()
            .unwrap();
        assert!(matches!(
            projection.message(&message.message_event_id).unwrap().state,
            super::super::AgentMessageState::Delivered { .. }
        ));
        assert!(projection
            .message(&message.message_event_id)
            .unwrap()
            .consumption_event_id
            .is_some());
        assert!(matches!(
            supervisor.send_message(SendAgentMessageRequest {
                sender_agent_id: root,
                recipient_agent_id: child.agent_id,
                causing_turn_id: turn_id,
                caused_by_event,
                content: "too late".into(),
            }),
            Err(AgentSupervisorError::MessageTargetNotLive {
                state: DispatchState::Completed,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn followup_reuses_proven_idle_session_on_same_agent_with_fresh_budget() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let factory = Arc::new(FakeInputFactory::new(Duration::from_millis(2)));
        let (supervisor, run_id, root) =
            supervisor_with(sink.clone(), factory, AgentSupervisorConfig::default());
        let child = supervisor
            .dispatch(request("follow up once"))
            .await
            .unwrap();
        assert!(supervisor
            .wait_for_idle(Duration::from_secs(1))
            .await
            .unwrap());
        assert_eq!(
            supervisor.state(&child.agent_id).unwrap(),
            Some(DispatchState::Completed)
        );
        let (turn_id, caused_by_event) = parent_cause(&*sink, &run_id, &root);
        let followup = supervisor
            .followup_agent(FollowupAgentRequest {
                agent_id: child.agent_id.clone(),
                parent_agent_id: root.clone(),
                causing_turn_id: turn_id,
                caused_by_event,
                objective: "Now inspect the restart edge.".into(),
                capabilities: Some(BTreeSet::from([super::super::AgentCapability::Read])),
                budget: AgentLoopBudget {
                    timeout: Duration::from_secs(30),
                    max_provider_events: 32,
                    max_tool_calls: 8,
                },
                retained_session_requested: false,
            })
            .await
            .unwrap();
        assert_eq!(followup.handle.agent_id, child.agent_id);
        assert_eq!(followup.turn_generation, 1);
        assert!(supervisor
            .wait_for_idle(Duration::from_secs(1))
            .await
            .unwrap());
        let record = supervisor
            .dispatches()
            .unwrap()
            .into_iter()
            .find(|record| record.handle.agent_id == child.agent_id)
            .unwrap();
        assert_eq!(record.state, DispatchState::Completed);
        assert_eq!(record.turn_generation, 1);
        assert_eq!(record.handle.workspace, child.workspace);
        assert_eq!(record.followup.unwrap().budget.max_tool_calls, 8);
        let followup_event = sink
            .events(&run_id)
            .unwrap()
            .into_iter()
            .find_map(|event| match event.kind {
                EventKind::AgentFollowup(followup) => Some(followup),
                _ => None,
            })
            .unwrap();
        assert!(followup_event.retained_session_requested);
        assert_eq!(
            supervisor.mailbox(root).unwrap().pending().unwrap().len(),
            2
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
                        warnings: Vec::new(),
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
    async fn parent_interrupt_terminalizes_a_not_yet_live_descendant() {
        // Saturate the single provider slot with the parent so its child stays
        // queued (not live), then interrupt the subtree. interrupt() must
        // terminalize the not-yet-live child as interrupted under the same
        // live->scheduler lock ordering as promotion, so that when the parent's
        // permit frees moments later, drain_ready does not promote and run it.
        // This guards the interrupt classification rewrite; the exact
        // Starting-but-not-live window needs a drain-boundary hook to hit
        // deterministically and is covered by reasoning, not this test.
        let sink = Arc::new(InMemoryRunEventSink::new());
        // A long per-tick duration pins the parent live for the whole test; the
        // loop runner cancels it via select!, so interrupt still lands promptly.
        let factory = Arc::new(FakeInputFactory::new(Duration::from_secs(10)));
        let config = AgentSupervisorConfig {
            max_depth: 1,
            max_concurrent: 1,
            ..AgentSupervisorConfig::default()
        };
        let (supervisor, run_id, _) = supervisor_with(sink.clone(), factory, config);
        let mut parent_request = request("parent delayed");
        parent_request.role = AgentKind::built_in(AgentKindName::Planner);
        let parent = supervisor.dispatch(parent_request).await.unwrap();
        while supervisor.live_handles().await.is_empty() {
            tokio::task::yield_now().await;
        }
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

        // The single provider slot is held by the parent, so the child is queued.
        assert_eq!(supervisor.live_handles().await.len(), 1);
        assert_eq!(
            supervisor.state(&child.agent_id).unwrap(),
            Some(DispatchState::Queued)
        );

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
        // The child ends interrupted rather than being promoted-and-run once the
        // parent's permit freed, proving interrupt terminalized it in place.
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
