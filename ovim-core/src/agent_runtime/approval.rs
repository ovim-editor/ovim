//! Durable, child-attributed approval routing with monotonic policy ceilings.
//!
//! The broker is deliberately independent of foreground chat approval state.
//! Each wait is keyed by run, child, and tool operation, projected from the
//! append-only log, and resumed through a private channel owned by that exact
//! request.

use super::{
    AgentApprovalClient, AgentApprovalDecision, AgentApprovalRequest, AgentCapability, AgentFuture,
    AgentWorkspaceDescriptor, DispatchHandle, WorkspaceStrategy,
};
use crate::run_log::{
    AgentApprovalDecisionSnapshot, AgentApprovalRequestedEvent,
    AgentApprovalResolutionSourceSnapshot, AgentApprovalResolvedEvent,
    AgentApprovalWorkspaceSnapshot, AgentCapabilitySnapshot, AgentId,
    AgentWorkspaceStrategySnapshot, EventActor, EventEnvelope, EventId, EventKind, NewRunEvent,
    OperationId, RunEventSink, RunId, RunLogError, ToolSideEffect, AGENT_APPROVAL_EVENT_VERSION,
};
use chrono::Utc;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tokio::sync::watch;

pub type AgentCapabilitySet = BTreeSet<AgentCapability>;

/// Every source of authority participates in the same typed intersection.
/// Adding a permissive profile or workspace can therefore never compensate
/// for a capability absent from a parent, root, project, role, or phase gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCapabilityCeiling {
    pub role_maximum: AgentCapabilitySet,
    pub parent_effective: AgentCapabilitySet,
    pub root_authorization: AgentCapabilitySet,
    pub project_policy: AgentCapabilitySet,
    pub profile_allowlist: AgentCapabilitySet,
    pub workspace_policy: AgentCapabilitySet,
    pub phase_feature_gates: AgentCapabilitySet,
}

impl AgentCapabilityCeiling {
    pub fn uniform(capabilities: AgentCapabilitySet) -> Self {
        Self {
            role_maximum: capabilities.clone(),
            parent_effective: capabilities.clone(),
            root_authorization: capabilities.clone(),
            project_policy: capabilities.clone(),
            profile_allowlist: capabilities.clone(),
            workspace_policy: capabilities.clone(),
            phase_feature_gates: capabilities,
        }
    }

    pub fn effective(&self) -> AgentCapabilitySet {
        let mut effective = self.role_maximum.clone();
        for limit in [
            &self.parent_effective,
            &self.root_authorization,
            &self.project_policy,
            &self.profile_allowlist,
            &self.workspace_policy,
            &self.phase_feature_gates,
        ] {
            effective.retain(|capability| limit.contains(capability));
        }
        effective
    }

    pub fn allows(&self, capability: &AgentCapability) -> bool {
        self.effective().contains(capability)
    }

    pub fn is_monotonic(&self) -> bool {
        let effective = self.effective();
        [
            &self.role_maximum,
            &self.parent_effective,
            &self.root_authorization,
            &self.project_policy,
            &self.profile_allowlist,
            &self.workspace_policy,
            &self.phase_feature_gates,
        ]
        .into_iter()
        .all(|limit| effective.is_subset(limit))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentApprovalKey {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentApprovalContext {
    pub handle: DispatchHandle,
    pub task_name: String,
    /// Root-to-parent chain; the child itself is carried by `handle`.
    pub ancestry: Vec<AgentId>,
    pub role: String,
    pub model: String,
    pub reasoning_effort: String,
    pub workspace: AgentWorkspaceDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAgentApproval {
    pub key: AgentApprovalKey,
    pub request_event_id: EventId,
    pub sequence: u64,
    pub request: AgentApprovalRequestedEvent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedAgentApproval {
    pub pending: PendingAgentApproval,
    pub resolution_event_id: EventId,
    pub sequence: u64,
    pub resolution: AgentApprovalResolvedEvent,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentApprovalProjection {
    pending: BTreeMap<AgentApprovalKey, PendingAgentApproval>,
    resolved: BTreeMap<AgentApprovalKey, ResolvedAgentApproval>,
}

impl AgentApprovalProjection {
    pub fn rehydrate(run_id: &RunId, events: &[EventEnvelope]) -> Result<Self, AgentApprovalError> {
        let mut projection = Self::default();
        let mut events = events.iter().collect::<Vec<_>>();
        events.sort_by_key(|event| event.sequence);
        for event in events {
            if &event.run_id != run_id {
                continue;
            }
            match &event.kind {
                EventKind::AgentApprovalRequested(request) => {
                    validate_requested_envelope(event, request)?;
                    let key = approval_key(event)?;
                    if projection.pending.contains_key(&key)
                        || projection.resolved.contains_key(&key)
                    {
                        return Err(AgentApprovalError::InvalidHistory(format!(
                            "approval operation {} was requested more than once",
                            key.operation_id
                        )));
                    }
                    projection.pending.insert(
                        key.clone(),
                        PendingAgentApproval {
                            key,
                            request_event_id: event.event_id.clone(),
                            sequence: event.sequence,
                            request: request.clone(),
                        },
                    );
                }
                EventKind::AgentApprovalResolved(resolution) => {
                    let key = approval_key(event)?;
                    let Some(pending) = projection.pending.remove(&key) else {
                        return Err(AgentApprovalError::InvalidHistory(format!(
                            "approval resolution {} has no pending request",
                            event.event_id
                        )));
                    };
                    if resolution.request_event_id != pending.request_event_id
                        || event.caused_by.as_ref() != Some(&pending.request_event_id)
                    {
                        return Err(AgentApprovalError::InvalidHistory(format!(
                            "approval resolution {} targets a stale request",
                            event.event_id
                        )));
                    }
                    projection.resolved.insert(
                        key,
                        ResolvedAgentApproval {
                            pending,
                            resolution_event_id: event.event_id.clone(),
                            sequence: event.sequence,
                            resolution: resolution.clone(),
                        },
                    );
                }
                _ => {}
            }
        }
        Ok(projection)
    }

    pub fn pending(&self) -> Vec<PendingAgentApproval> {
        let mut pending = self.pending.values().cloned().collect::<Vec<_>>();
        pending.sort_by_key(|entry| entry.sequence);
        pending
    }

    pub fn resolved(&self) -> Vec<ResolvedAgentApproval> {
        let mut resolved = self.resolved.values().cloned().collect::<Vec<_>>();
        resolved.sort_by_key(|entry| entry.sequence);
        resolved
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentApprovalResponseDecision {
    Allow,
    Deny { reason: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentApprovalResponse {
    pub key: AgentApprovalKey,
    pub request_event_id: EventId,
    pub decision: AgentApprovalResponseDecision,
}

#[derive(Clone)]
pub struct AgentApprovalBroker {
    inner: Arc<BrokerInner>,
}

struct BrokerInner {
    run_id: RunId,
    sink: Arc<dyn RunEventSink>,
    entries: Mutex<BTreeMap<AgentApprovalKey, ApprovalEntry>>,
    attention_generation: AtomicU64,
}

enum ApprovalEntry {
    Pending {
        pending: PendingAgentApproval,
        ceiling: Option<AgentCapabilityCeiling>,
        sender: Option<watch::Sender<Option<AgentApprovalDecision>>>,
    },
    Resolved(ResolvedAgentApproval),
}

impl AgentApprovalBroker {
    /// Opens one run-scoped broker and fails closed any request left pending by
    /// a prior process. Queued/running provider sessions are not resumed here.
    pub fn new(run_id: RunId, sink: Arc<dyn RunEventSink>) -> Result<Self, AgentApprovalError> {
        let events = sink.events(&run_id)?;
        let projection = AgentApprovalProjection::rehydrate(&run_id, &events)?;
        let mut entries = BTreeMap::new();
        for resolved in projection.resolved() {
            entries.insert(
                resolved.pending.key.clone(),
                ApprovalEntry::Resolved(resolved),
            );
        }
        for pending in projection.pending() {
            entries.insert(
                pending.key.clone(),
                ApprovalEntry::Pending {
                    pending,
                    ceiling: None,
                    sender: None,
                },
            );
        }
        let broker = Self {
            inner: Arc::new(BrokerInner {
                run_id,
                sink,
                entries: Mutex::new(entries),
                attention_generation: AtomicU64::new(0),
            }),
        };
        let abandoned = broker.pending()?;
        for pending in abandoned {
            broker.resolve_system(
                &pending.key,
                &pending.request_event_id,
                AgentApprovalResolutionSourceSnapshot::Restart,
                "Ovim restarted while this child approval was pending; the request was denied conservatively",
            )?;
            broker.bump_attention();
        }
        Ok(broker)
    }

    pub fn scoped_client(
        &self,
        context: AgentApprovalContext,
        ceiling: AgentCapabilityCeiling,
    ) -> Result<Arc<dyn AgentApprovalClient>, AgentApprovalError> {
        if context.handle.run_id != self.inner.run_id {
            return Err(AgentApprovalError::WrongRun {
                expected: self.inner.run_id.clone(),
                actual: context.handle.run_id,
            });
        }
        if context.workspace.assignment != context.handle.workspace {
            return Err(AgentApprovalError::InvalidContext(
                "approval workspace differs from dispatch handle".into(),
            ));
        }
        if !ceiling.is_monotonic() {
            return Err(AgentApprovalError::InvalidContext(
                "approval capability ceiling is not monotonic".into(),
            ));
        }
        Ok(Arc::new(ScopedAgentApprovalClient {
            broker: self.clone(),
            context,
            ceiling,
        }))
    }

    pub fn pending(&self) -> Result<Vec<PendingAgentApproval>, AgentApprovalError> {
        let entries = self.inner.entries.lock().map_err(|_| poisoned())?;
        let mut pending = entries
            .values()
            .filter_map(|entry| match entry {
                ApprovalEntry::Pending { pending, .. } => Some(pending.clone()),
                ApprovalEntry::Resolved(_) => None,
            })
            .collect::<Vec<_>>();
        pending.sort_by_key(|entry| entry.sequence);
        Ok(pending)
    }

    pub fn resolved(&self) -> Result<Vec<ResolvedAgentApproval>, AgentApprovalError> {
        let entries = self.inner.entries.lock().map_err(|_| poisoned())?;
        let mut resolved = entries
            .values()
            .filter_map(|entry| match entry {
                ApprovalEntry::Resolved(resolved) => Some(resolved.clone()),
                ApprovalEntry::Pending { .. } => None,
            })
            .collect::<Vec<_>>();
        resolved.sort_by_key(|entry| entry.sequence);
        Ok(resolved)
    }

    pub fn attention_generation(&self) -> u64 {
        self.inner.attention_generation.load(Ordering::Acquire)
    }

    /// Resolve exactly one child wait. Repeating the same choice is safe;
    /// changing a terminal choice or targeting a different child/request is
    /// rejected without waking any other child.
    pub fn respond(
        &self,
        response: AgentApprovalResponse,
    ) -> Result<AgentApprovalDecision, AgentApprovalError> {
        let mut entries = self.inner.entries.lock().map_err(|_| poisoned())?;
        let Some(entry) = entries.get(&response.key) else {
            let wrong_agent = entries.keys().any(|key| {
                key.run_id == response.key.run_id
                    && key.operation_id == response.key.operation_id
                    && key.agent_id != response.key.agent_id
            });
            return Err(if wrong_agent {
                AgentApprovalError::WrongAgent(response.key.agent_id)
            } else {
                AgentApprovalError::UnknownRequest(response.key)
            });
        };
        match entry {
            ApprovalEntry::Resolved(resolved) => {
                if resolved.pending.request_event_id != response.request_event_id {
                    return Err(AgentApprovalError::StaleResponse);
                }
                let existing = decision_from_resolution(&resolved.resolution);
                if same_response_choice(&existing, &response.decision) {
                    return Ok(existing);
                }
                return Err(AgentApprovalError::ConflictingDecision);
            }
            ApprovalEntry::Pending {
                pending, ceiling, ..
            } => {
                if pending.request_event_id != response.request_event_id {
                    return Err(AgentApprovalError::StaleResponse);
                }
                if matches!(response.decision, AgentApprovalResponseDecision::Allow) {
                    let required = capability_from_snapshot(&pending.request.required_capability);
                    if !ceiling
                        .as_ref()
                        .is_some_and(|ceiling| ceiling.allows(&required))
                    {
                        return Err(AgentApprovalError::CapabilityCeilingDenied(required));
                    }
                }
            }
        }
        let (decision, reason) = match response.decision {
            AgentApprovalResponseDecision::Allow => (
                AgentApprovalDecision::Allowed,
                "user allowed this scoped child operation".into(),
            ),
            AgentApprovalResponseDecision::Deny { reason } => {
                let reason = reason
                    .filter(|reason| !reason.trim().is_empty())
                    .unwrap_or_else(|| "user denied this child operation".into());
                (
                    AgentApprovalDecision::Denied {
                        reason: reason.clone(),
                    },
                    reason,
                )
            }
        };
        self.resolve_locked(
            &mut entries,
            &response.key,
            &response.request_event_id,
            decision.clone(),
            AgentApprovalResolutionSourceSnapshot::User,
            "user",
            reason,
        )?;
        Ok(decision)
    }

    fn request(
        &self,
        context: AgentApprovalContext,
        ceiling: AgentCapabilityCeiling,
        request: AgentApprovalRequest,
    ) -> AgentFuture<'static, Result<AgentApprovalDecision, AgentApprovalError>> {
        let broker = self.clone();
        Box::pin(async move {
            let wait = broker.begin_request(context, ceiling, request)?;
            let RequestWait::Pending {
                key,
                request_event_id,
                mut receiver,
                deadline,
            } = wait
            else {
                return Ok(wait.immediate_decision());
            };
            let mut guard = PendingWaitGuard {
                broker: broker.clone(),
                key: key.clone(),
                request_event_id: request_event_id.clone(),
                armed: true,
            };
            let result = tokio::select! {
                changed = receiver.changed() => {
                    match changed {
                        Ok(()) => receiver.borrow().clone().ok_or_else(|| {
                            AgentApprovalError::InvalidHistory(
                                "approval channel woke without a decision".into()
                            )
                        }),
                        Err(_) => Err(AgentApprovalError::InvalidHistory(
                            "approval decision channel closed unexpectedly".into()
                        )),
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    broker.resolve_system(
                        &key,
                        &request_event_id,
                        AgentApprovalResolutionSourceSnapshot::Timeout,
                        "child approval deadline elapsed; the operation was denied",
                    )
                }
            };
            guard.armed = false;
            result
        })
    }

    fn begin_request(
        &self,
        context: AgentApprovalContext,
        ceiling: AgentCapabilityCeiling,
        request: AgentApprovalRequest,
    ) -> Result<RequestWait, AgentApprovalError> {
        validate_request_context(&self.inner.run_id, &context, &request)?;
        let key = AgentApprovalKey {
            run_id: request.handle.run_id.clone(),
            agent_id: request.handle.agent_id.clone(),
            operation_id: request.operation_id.clone(),
        };
        let now = Utc::now();
        let remaining = request
            .deadline
            .saturating_duration_since(tokio::time::Instant::now());
        let deadline_at =
            now + chrono::Duration::from_std(remaining).unwrap_or(chrono::Duration::MAX);
        let requested = AgentApprovalRequestedEvent {
            version: AGENT_APPROVAL_EVENT_VERSION,
            task_name: context.task_name,
            ancestry: context.ancestry,
            role: context.role,
            model: context.model,
            reasoning_effort: context.reasoning_effort,
            tool_name: request.tool_name,
            normalized_effect: request.normalized_effect.clone(),
            required_capability: capability_snapshot(&request.required_capability),
            effective_capabilities: ceiling
                .effective()
                .iter()
                .map(capability_snapshot)
                .collect(),
            workspace: workspace_snapshot(&context.workspace),
            reason: request.reason,
            created_at: now.to_rfc3339(),
            deadline_at: deadline_at.to_rfc3339(),
        };
        let mut entries = self.inner.entries.lock().map_err(|_| poisoned())?;
        if let Some(existing) = entries.get(&key) {
            return duplicate_request_wait(existing, &requested, request.deadline);
        }
        let envelope = self.inner.sink.append(NewRunEvent {
            run_id: key.run_id.clone(),
            caused_by: Some(request.tool_intent_event_id),
            operation_id: Some(key.operation_id.clone()),
            provider_call_id: request.provider_call_id,
            actor: EventActor::System("agent_approval_broker".into()),
            agent_id: Some(key.agent_id.clone()),
            turn_id: request.turn_id,
            workspace_id: Some(context.workspace.assignment.workspace_id.clone()),
            branch_id: None,
            kind: EventKind::AgentApprovalRequested(requested.clone()),
        })?;
        let pending = PendingAgentApproval {
            key: key.clone(),
            request_event_id: envelope.event_id,
            sequence: envelope.sequence,
            request: requested,
        };
        if let Some(reason) = policy_denial(
            &context.workspace,
            &ceiling,
            &request.required_capability,
            &request.normalized_effect,
        ) {
            entries.insert(
                key.clone(),
                ApprovalEntry::Pending {
                    pending: pending.clone(),
                    ceiling: Some(ceiling),
                    sender: None,
                },
            );
            let decision = AgentApprovalDecision::Denied {
                reason: reason.clone(),
            };
            self.resolve_locked(
                &mut entries,
                &key,
                &pending.request_event_id,
                decision.clone(),
                AgentApprovalResolutionSourceSnapshot::Policy,
                "policy",
                reason,
            )?;
            return Ok(RequestWait::Immediate(decision));
        }
        if remaining.is_zero() {
            entries.insert(
                key.clone(),
                ApprovalEntry::Pending {
                    pending: pending.clone(),
                    ceiling: Some(ceiling),
                    sender: None,
                },
            );
            let decision = self.resolve_locked(
                &mut entries,
                &key,
                &pending.request_event_id,
                AgentApprovalDecision::Denied {
                    reason: "child approval deadline elapsed; the operation was denied".into(),
                },
                AgentApprovalResolutionSourceSnapshot::Timeout,
                "agent_approval_broker",
                "child approval deadline elapsed; the operation was denied".into(),
            )?;
            return Ok(RequestWait::Immediate(decision));
        }
        let (sender, receiver) = watch::channel(None);
        entries.insert(
            key.clone(),
            ApprovalEntry::Pending {
                pending: pending.clone(),
                ceiling: Some(ceiling),
                sender: Some(sender),
            },
        );
        self.bump_attention();
        Ok(RequestWait::Pending {
            key,
            request_event_id: pending.request_event_id,
            receiver,
            deadline: request.deadline,
        })
    }

    fn resolve_system(
        &self,
        key: &AgentApprovalKey,
        request_event_id: &EventId,
        source: AgentApprovalResolutionSourceSnapshot,
        reason: &str,
    ) -> Result<AgentApprovalDecision, AgentApprovalError> {
        let mut entries = self.inner.entries.lock().map_err(|_| poisoned())?;
        if let Some(ApprovalEntry::Resolved(resolved)) = entries.get(key) {
            if &resolved.pending.request_event_id != request_event_id {
                return Err(AgentApprovalError::StaleResponse);
            }
            return Ok(decision_from_resolution(&resolved.resolution));
        }
        self.resolve_locked(
            &mut entries,
            key,
            request_event_id,
            AgentApprovalDecision::Denied {
                reason: reason.into(),
            },
            source,
            "agent_approval_broker",
            reason.into(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_locked(
        &self,
        entries: &mut BTreeMap<AgentApprovalKey, ApprovalEntry>,
        key: &AgentApprovalKey,
        request_event_id: &EventId,
        decision: AgentApprovalDecision,
        source: AgentApprovalResolutionSourceSnapshot,
        decided_by: &str,
        reason: String,
    ) -> Result<AgentApprovalDecision, AgentApprovalError> {
        let Some(ApprovalEntry::Pending {
            pending, sender, ..
        }) = entries.get(key)
        else {
            return Err(AgentApprovalError::UnknownRequest(key.clone()));
        };
        if &pending.request_event_id != request_event_id {
            return Err(AgentApprovalError::StaleResponse);
        }
        let pending = pending.clone();
        let sender = sender.clone();
        let resolution = AgentApprovalResolvedEvent {
            version: AGENT_APPROVAL_EVENT_VERSION,
            request_event_id: request_event_id.clone(),
            decision: match decision {
                AgentApprovalDecision::Allowed => AgentApprovalDecisionSnapshot::Allowed,
                AgentApprovalDecision::Denied { .. } => AgentApprovalDecisionSnapshot::Denied,
            },
            source,
            decided_by: decided_by.into(),
            reason,
            resolved_at: Utc::now().to_rfc3339(),
        };
        let envelope = self.inner.sink.append(NewRunEvent {
            run_id: key.run_id.clone(),
            caused_by: Some(request_event_id.clone()),
            operation_id: Some(key.operation_id.clone()),
            provider_call_id: None,
            actor: if source == AgentApprovalResolutionSourceSnapshot::User {
                EventActor::User
            } else {
                EventActor::System("agent_approval_broker".into())
            },
            agent_id: Some(key.agent_id.clone()),
            turn_id: None,
            workspace_id: Some(pending.request.workspace.workspace_id.clone()),
            branch_id: None,
            kind: EventKind::AgentApprovalResolved(resolution.clone()),
        })?;
        let resolved = ResolvedAgentApproval {
            pending,
            resolution_event_id: envelope.event_id,
            sequence: envelope.sequence,
            resolution,
        };
        entries.insert(key.clone(), ApprovalEntry::Resolved(resolved));
        if let Some(sender) = sender {
            sender.send_replace(Some(decision.clone()));
        }
        Ok(decision)
    }

    fn bump_attention(&self) {
        self.inner
            .attention_generation
            .fetch_add(1, Ordering::AcqRel);
    }
}

struct ScopedAgentApprovalClient {
    broker: AgentApprovalBroker,
    context: AgentApprovalContext,
    ceiling: AgentCapabilityCeiling,
}

impl AgentApprovalClient for ScopedAgentApprovalClient {
    fn request(
        &self,
        request: AgentApprovalRequest,
    ) -> AgentFuture<'_, Result<AgentApprovalDecision, String>> {
        let future = self
            .broker
            .request(self.context.clone(), self.ceiling.clone(), request);
        Box::pin(async move { future.await.map_err(|error| error.to_string()) })
    }
}

enum RequestWait {
    Immediate(AgentApprovalDecision),
    Pending {
        key: AgentApprovalKey,
        request_event_id: EventId,
        receiver: watch::Receiver<Option<AgentApprovalDecision>>,
        deadline: tokio::time::Instant,
    },
}

impl RequestWait {
    fn immediate_decision(self) -> AgentApprovalDecision {
        match self {
            Self::Immediate(decision) => decision,
            Self::Pending { .. } => unreachable!("pending request was destructured above"),
        }
    }
}

struct PendingWaitGuard {
    broker: AgentApprovalBroker,
    key: AgentApprovalKey,
    request_event_id: EventId,
    armed: bool,
}

impl Drop for PendingWaitGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.broker.resolve_system(
                &self.key,
                &self.request_event_id,
                AgentApprovalResolutionSourceSnapshot::Cancellation,
                "child approval wait was interrupted; the operation was denied",
            );
        }
    }
}

fn validate_request_context(
    run_id: &RunId,
    context: &AgentApprovalContext,
    request: &AgentApprovalRequest,
) -> Result<(), AgentApprovalError> {
    if &request.handle.run_id != run_id || request.handle.run_id != context.handle.run_id {
        return Err(AgentApprovalError::WrongRun {
            expected: run_id.clone(),
            actual: request.handle.run_id.clone(),
        });
    }
    if request.handle != context.handle {
        return Err(AgentApprovalError::WrongAgent(
            request.handle.agent_id.clone(),
        ));
    }
    if request.workspace != context.workspace
        || request.workspace.assignment != request.handle.workspace
    {
        return Err(AgentApprovalError::InvalidContext(
            "approval request changed its assigned workspace".into(),
        ));
    }
    if request.tool_name.trim().is_empty() || request.reason.trim().is_empty() {
        return Err(AgentApprovalError::InvalidContext(
            "approval tool and reason must be non-empty".into(),
        ));
    }
    Ok(())
}

fn policy_denial(
    workspace: &AgentWorkspaceDescriptor,
    ceiling: &AgentCapabilityCeiling,
    required: &AgentCapability,
    effect: &ToolSideEffect,
) -> Option<String> {
    if matches!(effect, ToolSideEffect::Unknown) {
        return Some("tool effect is ambiguous and cannot be escalated to user approval".into());
    }
    if workspace.read_only && !matches!(effect, ToolSideEffect::Read | ToolSideEffect::Navigation) {
        return Some("read-only child policy denies mutation or external effects".into());
    }
    if workspace.read_only
        && matches!(
            required,
            AgentCapability::Shell
                | AgentCapability::WorkspaceWrite
                | AgentCapability::ExternalEffects
        )
    {
        return Some("read-only workspace policy excludes the requested capability".into());
    }
    if !ceiling.allows(required) {
        return Some(format!(
            "requested capability {required:?} is outside the child policy ceiling"
        ));
    }
    None
}

fn duplicate_request_wait(
    existing: &ApprovalEntry,
    requested: &AgentApprovalRequestedEvent,
    deadline: tokio::time::Instant,
) -> Result<RequestWait, AgentApprovalError> {
    match existing {
        ApprovalEntry::Pending {
            pending, sender, ..
        } if &pending.request == requested => {
            let Some(sender) = sender else {
                return Err(AgentApprovalError::InvalidHistory(
                    "replayed pending approval cannot be awaited".into(),
                ));
            };
            Ok(RequestWait::Pending {
                key: pending.key.clone(),
                request_event_id: pending.request_event_id.clone(),
                receiver: sender.subscribe(),
                deadline,
            })
        }
        ApprovalEntry::Resolved(resolved) if &resolved.pending.request == requested => Ok(
            RequestWait::Immediate(decision_from_resolution(&resolved.resolution)),
        ),
        _ => Err(AgentApprovalError::OperationCollision),
    }
}

fn approval_key(event: &EventEnvelope) -> Result<AgentApprovalKey, AgentApprovalError> {
    Ok(AgentApprovalKey {
        run_id: event.run_id.clone(),
        agent_id: event.agent_id.clone().ok_or_else(|| {
            AgentApprovalError::InvalidHistory(format!(
                "approval event {} has no child identity",
                event.event_id
            ))
        })?,
        operation_id: event.operation_id.clone().ok_or_else(|| {
            AgentApprovalError::InvalidHistory(format!(
                "approval event {} has no operation identity",
                event.event_id
            ))
        })?,
    })
}

fn validate_requested_envelope(
    event: &EventEnvelope,
    request: &AgentApprovalRequestedEvent,
) -> Result<(), AgentApprovalError> {
    if request.version != AGENT_APPROVAL_EVENT_VERSION
        || event.workspace_id.as_ref() != Some(&request.workspace.workspace_id)
        || event.agent_id.is_none()
        || event.operation_id.is_none()
    {
        return Err(AgentApprovalError::InvalidHistory(format!(
            "approval request {} has inconsistent attribution",
            event.event_id
        )));
    }
    Ok(())
}

fn workspace_snapshot(workspace: &AgentWorkspaceDescriptor) -> AgentApprovalWorkspaceSnapshot {
    AgentApprovalWorkspaceSnapshot {
        workspace_id: workspace.assignment.workspace_id.clone(),
        strategy: match &workspace.assignment.strategy {
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
        root: workspace
            .root
            .as_ref()
            .map(|root| root.to_string_lossy().into_owned()),
        read_only: workspace.read_only,
    }
}

fn capability_snapshot(capability: &AgentCapability) -> AgentCapabilitySnapshot {
    match capability {
        AgentCapability::Read => AgentCapabilitySnapshot::Read,
        AgentCapability::Navigate => AgentCapabilitySnapshot::Navigate,
        AgentCapability::SafeShell => AgentCapabilitySnapshot::SafeShell,
        AgentCapability::Shell => AgentCapabilitySnapshot::Shell,
        AgentCapability::WorkspaceWrite => AgentCapabilitySnapshot::WorkspaceWrite,
        AgentCapability::ExternalEffects => AgentCapabilitySnapshot::ExternalEffects,
        AgentCapability::DispatchAgents => AgentCapabilitySnapshot::DispatchAgents,
    }
}

fn capability_from_snapshot(capability: &AgentCapabilitySnapshot) -> AgentCapability {
    match capability {
        AgentCapabilitySnapshot::Read => AgentCapability::Read,
        AgentCapabilitySnapshot::Navigate => AgentCapability::Navigate,
        AgentCapabilitySnapshot::SafeShell => AgentCapability::SafeShell,
        AgentCapabilitySnapshot::Shell => AgentCapability::Shell,
        AgentCapabilitySnapshot::WorkspaceWrite => AgentCapability::WorkspaceWrite,
        AgentCapabilitySnapshot::ExternalEffects => AgentCapability::ExternalEffects,
        AgentCapabilitySnapshot::DispatchAgents => AgentCapability::DispatchAgents,
    }
}

fn decision_from_resolution(resolution: &AgentApprovalResolvedEvent) -> AgentApprovalDecision {
    match resolution.decision {
        AgentApprovalDecisionSnapshot::Allowed => AgentApprovalDecision::Allowed,
        AgentApprovalDecisionSnapshot::Denied => AgentApprovalDecision::Denied {
            reason: resolution.reason.clone(),
        },
    }
}

fn same_response_choice(
    existing: &AgentApprovalDecision,
    response: &AgentApprovalResponseDecision,
) -> bool {
    matches!(
        (existing, response),
        (
            AgentApprovalDecision::Allowed,
            AgentApprovalResponseDecision::Allow
        ) | (
            AgentApprovalDecision::Denied { .. },
            AgentApprovalResponseDecision::Deny { .. }
        )
    )
}

fn poisoned() -> AgentApprovalError {
    AgentApprovalError::Poisoned
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentApprovalError {
    WrongRun { expected: RunId, actual: RunId },
    WrongAgent(AgentId),
    UnknownRequest(AgentApprovalKey),
    StaleResponse,
    ConflictingDecision,
    CapabilityCeilingDenied(AgentCapability),
    OperationCollision,
    InvalidContext(String),
    InvalidHistory(String),
    RunLog(String),
    Poisoned,
}

impl fmt::Display for AgentApprovalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRun { expected, actual } => {
                write!(
                    formatter,
                    "approval belongs to run {actual}, expected {expected}"
                )
            }
            Self::WrongAgent(agent_id) => {
                write!(
                    formatter,
                    "approval response targets the wrong child {agent_id}"
                )
            }
            Self::UnknownRequest(key) => write!(
                formatter,
                "approval request for child {} operation {} is unknown",
                key.agent_id, key.operation_id
            ),
            Self::StaleResponse => formatter.write_str("approval response targets a stale request"),
            Self::ConflictingDecision => {
                formatter.write_str("approval request already has a different decision")
            }
            Self::CapabilityCeilingDenied(capability) => write!(
                formatter,
                "approval cannot widen the child capability ceiling to {capability:?}"
            ),
            Self::OperationCollision => formatter.write_str(
                "approval operation identity was reused with different request metadata",
            ),
            Self::InvalidContext(detail) => write!(formatter, "invalid approval context: {detail}"),
            Self::InvalidHistory(detail) => write!(formatter, "invalid approval history: {detail}"),
            Self::RunLog(detail) => write!(formatter, "approval event log failed: {detail}"),
            Self::Poisoned => formatter.write_str("approval broker lock was poisoned"),
        }
    }
}

impl std::error::Error for AgentApprovalError {}

impl From<RunLogError> for AgentApprovalError {
    fn from(error: RunLogError) -> Self {
        Self::RunLog(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::{
        InMemoryRunEventSink, ToolIntentEvent, WorkspaceId, EVENT_PAYLOAD_VERSION,
        EVENT_SCHEMA_VERSION,
    };
    use serde_json::json;
    use std::path::PathBuf;
    use std::time::Duration;

    fn ids(suffix: &str) -> (RunId, AgentId, WorkspaceId, OperationId) {
        (
            RunId::parse(format!("run_{suffix}")).unwrap(),
            AgentId::parse(format!("agt_{suffix}")).unwrap(),
            WorkspaceId::parse(format!("wsp_{suffix}")).unwrap(),
            OperationId::parse(format!("op_{suffix}")).unwrap(),
        )
    }

    fn workspace(workspace_id: WorkspaceId, read_only: bool) -> AgentWorkspaceDescriptor {
        AgentWorkspaceDescriptor {
            assignment: super::super::WorkspaceAssignment {
                workspace_id,
                strategy: if read_only {
                    WorkspaceStrategy::ReadOnlySnapshot { manifest_id: None }
                } else {
                    WorkspaceStrategy::SharedWorkspace
                },
            },
            root: (!read_only).then(|| PathBuf::from("/tmp/ovim-approval-test")),
            read_only,
            warnings: Vec::new(),
        }
    }

    fn context(
        run_id: RunId,
        agent_id: AgentId,
        workspace: AgentWorkspaceDescriptor,
        parent: &str,
    ) -> AgentApprovalContext {
        AgentApprovalContext {
            handle: DispatchHandle {
                run_id,
                agent_id,
                workspace: workspace.assignment.clone(),
            },
            task_name: "inspect_child_effect".into(),
            ancestry: vec![AgentId::parse(format!("agt_{parent}")).unwrap()],
            role: "implementer".into(),
            model: "gpt-5.3-codex".into(),
            reasoning_effort: "high".into(),
            workspace,
        }
    }

    fn append_intent(
        sink: &Arc<InMemoryRunEventSink>,
        context: &AgentApprovalContext,
        operation_id: &OperationId,
        effect: ToolSideEffect,
    ) -> EventId {
        sink.append(NewRunEvent {
            run_id: context.handle.run_id.clone(),
            caused_by: None,
            operation_id: Some(operation_id.clone()),
            provider_call_id: Some("provider-call".into()),
            actor: EventActor::Agent(context.handle.agent_id.clone()),
            agent_id: Some(context.handle.agent_id.clone()),
            turn_id: None,
            workspace_id: Some(context.workspace.assignment.workspace_id.clone()),
            branch_id: None,
            kind: EventKind::ToolIntent(ToolIntentEvent {
                tool_name: "write_file".into(),
                arguments: json!({ "path": "src/lib.rs" }),
                side_effect: effect,
            }),
        })
        .unwrap()
        .event_id
    }

    fn request(
        context: &AgentApprovalContext,
        operation_id: OperationId,
        intent: EventId,
        effect: ToolSideEffect,
        capability: AgentCapability,
        timeout: Duration,
    ) -> AgentApprovalRequest {
        AgentApprovalRequest {
            handle: context.handle.clone(),
            operation_id,
            tool_intent_event_id: intent,
            provider_call_id: Some("provider-call".into()),
            turn_id: None,
            tool_name: "write_file".into(),
            normalized_effect: effect,
            required_capability: capability,
            workspace: context.workspace.clone(),
            reason: "write policy requires user confirmation".into(),
            deadline: tokio::time::Instant::now() + timeout,
        }
    }

    fn all_capabilities() -> AgentCapabilitySet {
        BTreeSet::from([
            AgentCapability::Read,
            AgentCapability::Navigate,
            AgentCapability::SafeShell,
            AgentCapability::Shell,
            AgentCapability::WorkspaceWrite,
            AgentCapability::ExternalEffects,
            AgentCapability::DispatchAgents,
        ])
    }

    async fn start_request(
        client: Arc<dyn AgentApprovalClient>,
        request: AgentApprovalRequest,
    ) -> tokio::task::JoinHandle<Result<AgentApprovalDecision, String>> {
        let task = tokio::spawn(async move { client.request(request).await });
        tokio::task::yield_now().await;
        task
    }

    #[tokio::test]
    async fn simultaneous_children_queue_without_overwriting_and_route_exactly() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let (run_id, first_agent, first_workspace, first_operation) = ids("simultaneous");
        let second_agent = AgentId::parse("agt_simultaneous_second").unwrap();
        let second_workspace = WorkspaceId::parse("wsp_simultaneous_second").unwrap();
        let second_operation = OperationId::parse("op_simultaneous_second").unwrap();
        let broker = AgentApprovalBroker::new(run_id.clone(), sink.clone()).unwrap();
        let first_context = context(
            run_id.clone(),
            first_agent,
            workspace(first_workspace, false),
            "root",
        );
        let second_context = context(
            run_id,
            second_agent,
            workspace(second_workspace, false),
            "root",
        );
        let ceiling = AgentCapabilityCeiling::uniform(all_capabilities());
        let first_client = broker
            .scoped_client(first_context.clone(), ceiling.clone())
            .unwrap();
        let second_client = broker
            .scoped_client(second_context.clone(), ceiling)
            .unwrap();
        let first_intent = append_intent(
            &sink,
            &first_context,
            &first_operation,
            ToolSideEffect::Mutation,
        );
        let second_intent = append_intent(
            &sink,
            &second_context,
            &second_operation,
            ToolSideEffect::Mutation,
        );
        let first_task = start_request(
            first_client,
            request(
                &first_context,
                first_operation,
                first_intent,
                ToolSideEffect::Mutation,
                AgentCapability::WorkspaceWrite,
                Duration::from_secs(5),
            ),
        )
        .await;
        let second_task = start_request(
            second_client,
            request(
                &second_context,
                second_operation,
                second_intent,
                ToolSideEffect::Mutation,
                AgentCapability::WorkspaceWrite,
                Duration::from_secs(5),
            ),
        )
        .await;

        let pending = broker.pending().unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(broker.attention_generation(), 2);
        assert_eq!(pending[0].request.ancestry[0].as_str(), "agt_root");
        assert_eq!(pending[0].request.role, "implementer");
        assert_eq!(pending[0].request.model, "gpt-5.3-codex");
        assert_eq!(pending[0].request.reasoning_effort, "high");
        assert_eq!(pending[0].request.tool_name, "write_file");
        assert_eq!(
            pending[0].request.normalized_effect,
            ToolSideEffect::Mutation
        );
        assert!(!pending[0].request.created_at.is_empty());
        assert!(!pending[0].request.deadline_at.is_empty());

        let first = pending
            .iter()
            .find(|entry| entry.key.agent_id == first_context.handle.agent_id)
            .unwrap();
        broker
            .respond(AgentApprovalResponse {
                key: first.key.clone(),
                request_event_id: first.request_event_id.clone(),
                decision: AgentApprovalResponseDecision::Allow,
            })
            .unwrap();
        assert_eq!(
            first_task.await.unwrap().unwrap(),
            AgentApprovalDecision::Allowed
        );
        assert!(!second_task.is_finished());

        let second = broker.pending().unwrap().remove(0);
        broker
            .respond(AgentApprovalResponse {
                key: second.key,
                request_event_id: second.request_event_id,
                decision: AgentApprovalResponseDecision::Deny {
                    reason: Some("keep this child read-only".into()),
                },
            })
            .unwrap();
        assert_eq!(
            second_task.await.unwrap().unwrap(),
            AgentApprovalDecision::Denied {
                reason: "keep this child read-only".into()
            }
        );
        assert!(broker.pending().unwrap().is_empty());
    }

    #[tokio::test]
    async fn decisions_are_idempotent_but_wrong_agent_stale_and_conflicting_responses_fail() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let (run_id, agent_id, workspace_id, operation_id) = ids("responses");
        let broker = AgentApprovalBroker::new(run_id.clone(), sink.clone()).unwrap();
        let context = context(run_id, agent_id, workspace(workspace_id, false), "root");
        let intent = append_intent(&sink, &context, &operation_id, ToolSideEffect::Mutation);
        let client = broker
            .scoped_client(
                context.clone(),
                AgentCapabilityCeiling::uniform(all_capabilities()),
            )
            .unwrap();
        let task = start_request(
            client,
            request(
                &context,
                operation_id.clone(),
                intent,
                ToolSideEffect::Mutation,
                AgentCapability::WorkspaceWrite,
                Duration::from_secs(5),
            ),
        )
        .await;
        let pending = broker.pending().unwrap().remove(0);
        let response = AgentApprovalResponse {
            key: pending.key.clone(),
            request_event_id: pending.request_event_id.clone(),
            decision: AgentApprovalResponseDecision::Allow,
        };
        assert_eq!(
            broker.respond(response.clone()).unwrap(),
            AgentApprovalDecision::Allowed
        );
        assert_eq!(
            broker.respond(response).unwrap(),
            AgentApprovalDecision::Allowed
        );
        assert_eq!(task.await.unwrap().unwrap(), AgentApprovalDecision::Allowed);
        assert_eq!(broker.resolved().unwrap().len(), 1);

        let stale = broker.respond(AgentApprovalResponse {
            key: pending.key.clone(),
            request_event_id: EventId::new(),
            decision: AgentApprovalResponseDecision::Allow,
        });
        assert_eq!(stale.unwrap_err(), AgentApprovalError::StaleResponse);
        let conflicting = broker.respond(AgentApprovalResponse {
            key: pending.key.clone(),
            request_event_id: pending.request_event_id.clone(),
            decision: AgentApprovalResponseDecision::Deny { reason: None },
        });
        assert_eq!(
            conflicting.unwrap_err(),
            AgentApprovalError::ConflictingDecision
        );
        let wrong_agent = broker.respond(AgentApprovalResponse {
            key: AgentApprovalKey {
                agent_id: AgentId::parse("agt_intruder").unwrap(),
                ..pending.key
            },
            request_event_id: pending.request_event_id,
            decision: AgentApprovalResponseDecision::Allow,
        });
        assert!(matches!(
            wrong_agent,
            Err(AgentApprovalError::WrongAgent(_))
        ));

        let deny_operation = OperationId::parse("op_responses_deny").unwrap();
        let deny_intent = append_intent(&sink, &context, &deny_operation, ToolSideEffect::Mutation);
        let deny_task = start_request(
            broker
                .scoped_client(
                    context.clone(),
                    AgentCapabilityCeiling::uniform(all_capabilities()),
                )
                .unwrap(),
            request(
                &context,
                deny_operation,
                deny_intent,
                ToolSideEffect::Mutation,
                AgentCapability::WorkspaceWrite,
                Duration::from_secs(5),
            ),
        )
        .await;
        let pending = broker.pending().unwrap().remove(0);
        let deny = AgentApprovalResponse {
            key: pending.key,
            request_event_id: pending.request_event_id,
            decision: AgentApprovalResponseDecision::Deny {
                reason: Some("denied once".into()),
            },
        };
        let expected = AgentApprovalDecision::Denied {
            reason: "denied once".into(),
        };
        assert_eq!(broker.respond(deny.clone()).unwrap(), expected);
        assert_eq!(broker.respond(deny).unwrap(), expected);
        assert_eq!(deny_task.await.unwrap().unwrap(), expected);
        assert_eq!(broker.resolved().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn timeout_and_cancel_fail_closed_and_leave_no_pending_request() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let (run_id, agent_id, workspace_id, operation_id) = ids("timeout");
        let broker = AgentApprovalBroker::new(run_id.clone(), sink.clone()).unwrap();
        let context = context(run_id, agent_id, workspace(workspace_id, false), "root");
        let ceiling = AgentCapabilityCeiling::uniform(all_capabilities());
        let intent = append_intent(&sink, &context, &operation_id, ToolSideEffect::Mutation);
        let client = broker
            .scoped_client(context.clone(), ceiling.clone())
            .unwrap();
        let timed_out = client
            .request(request(
                &context,
                operation_id,
                intent,
                ToolSideEffect::Mutation,
                AgentCapability::WorkspaceWrite,
                Duration::from_millis(10),
            ))
            .await
            .unwrap();
        assert!(matches!(timed_out, AgentApprovalDecision::Denied { .. }));
        assert!(broker.pending().unwrap().is_empty());
        assert_eq!(
            broker.resolved().unwrap()[0].resolution.source,
            AgentApprovalResolutionSourceSnapshot::Timeout
        );

        let cancel_operation = OperationId::parse("op_cancel").unwrap();
        let cancel_intent =
            append_intent(&sink, &context, &cancel_operation, ToolSideEffect::Mutation);
        let cancel_task = start_request(
            broker.scoped_client(context.clone(), ceiling).unwrap(),
            request(
                &context,
                cancel_operation,
                cancel_intent,
                ToolSideEffect::Mutation,
                AgentCapability::WorkspaceWrite,
                Duration::from_secs(5),
            ),
        )
        .await;
        assert_eq!(broker.pending().unwrap().len(), 1);
        cancel_task.abort();
        assert!(cancel_task.await.unwrap_err().is_cancelled());
        tokio::task::yield_now().await;
        assert!(broker.pending().unwrap().is_empty());
        assert_eq!(
            broker.resolved().unwrap()[1].resolution.source,
            AgentApprovalResolutionSourceSnapshot::Cancellation
        );
    }

    #[test]
    fn restart_replay_denies_abandoned_request_and_marks_attention() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let (run_id, agent_id, workspace_id, operation_id) = ids("restart");
        let workspace = workspace(workspace_id.clone(), false);
        let context = context(run_id.clone(), agent_id.clone(), workspace.clone(), "root");
        let intent = append_intent(&sink, &context, &operation_id, ToolSideEffect::Mutation);
        let request_event = sink
            .append(NewRunEvent {
                run_id: run_id.clone(),
                caused_by: Some(intent),
                operation_id: Some(operation_id),
                provider_call_id: None,
                actor: EventActor::System("crashed_broker".into()),
                agent_id: Some(agent_id),
                turn_id: None,
                workspace_id: Some(workspace_id.clone()),
                branch_id: None,
                kind: EventKind::AgentApprovalRequested(AgentApprovalRequestedEvent {
                    version: AGENT_APPROVAL_EVENT_VERSION,
                    task_name: "abandoned".into(),
                    ancestry: context.ancestry,
                    role: "implementer".into(),
                    model: "gpt-5.3-codex".into(),
                    reasoning_effort: "high".into(),
                    tool_name: "write_file".into(),
                    normalized_effect: ToolSideEffect::Mutation,
                    required_capability: AgentCapabilitySnapshot::WorkspaceWrite,
                    effective_capabilities: vec![AgentCapabilitySnapshot::WorkspaceWrite],
                    workspace: workspace_snapshot(&workspace),
                    reason: "needs approval".into(),
                    created_at: Utc::now().to_rfc3339(),
                    deadline_at: (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
                }),
            })
            .unwrap();

        let broker = AgentApprovalBroker::new(run_id.clone(), sink.clone()).unwrap();
        assert!(broker.pending().unwrap().is_empty());
        assert_eq!(broker.attention_generation(), 1);
        let resolved = broker.resolved().unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].pending.request_event_id, request_event.event_id);
        assert_eq!(
            resolved[0].resolution.source,
            AgentApprovalResolutionSourceSnapshot::Restart
        );
        let replay =
            AgentApprovalProjection::rehydrate(&run_id, &sink.events(&run_id).unwrap()).unwrap();
        assert!(replay.pending().is_empty());
        assert_eq!(replay.resolved().len(), 1);
    }

    #[tokio::test]
    async fn ceiling_and_read_only_policy_deny_without_prompting() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let (run_id, agent_id, workspace_id, operation_id) = ids("policy");
        let broker = AgentApprovalBroker::new(run_id.clone(), sink.clone()).unwrap();
        let context = context(run_id, agent_id, workspace(workspace_id, true), "root");
        let ceiling = AgentCapabilityCeiling::uniform(all_capabilities());
        let intent = append_intent(&sink, &context, &operation_id, ToolSideEffect::Unknown);
        let denied = broker
            .scoped_client(context.clone(), ceiling)
            .unwrap()
            .request(request(
                &context,
                operation_id,
                intent,
                ToolSideEffect::Unknown,
                AgentCapability::Read,
                Duration::from_secs(5),
            ))
            .await
            .unwrap();
        assert!(matches!(denied, AgentApprovalDecision::Denied { .. }));
        assert!(broker.pending().unwrap().is_empty());
        assert_eq!(broker.attention_generation(), 0);
        assert_eq!(
            broker.resolved().unwrap()[0].resolution.source,
            AgentApprovalResolutionSourceSnapshot::Policy
        );

        let write_context = AgentApprovalContext {
            workspace: workspace(context.workspace.assignment.workspace_id.clone(), false),
            handle: DispatchHandle {
                workspace: super::super::WorkspaceAssignment {
                    workspace_id: context.workspace.assignment.workspace_id.clone(),
                    strategy: WorkspaceStrategy::SharedWorkspace,
                },
                ..context.handle.clone()
            },
            ..context
        };
        let operation = OperationId::parse("op_ceiling").unwrap();
        let intent = append_intent(&sink, &write_context, &operation, ToolSideEffect::Mutation);
        let denied = broker
            .scoped_client(
                write_context.clone(),
                AgentCapabilityCeiling::uniform(BTreeSet::from([AgentCapability::Read])),
            )
            .unwrap()
            .request(request(
                &write_context,
                operation,
                intent,
                ToolSideEffect::Mutation,
                AgentCapability::WorkspaceWrite,
                Duration::from_secs(5),
            ))
            .await
            .unwrap();
        assert!(matches!(denied, AgentApprovalDecision::Denied { .. }));
        assert_eq!(broker.attention_generation(), 0);
    }

    #[test]
    fn capability_intersection_is_monotonic_across_all_policy_layers() {
        let capabilities = all_capabilities().into_iter().collect::<Vec<_>>();
        let all = capabilities.iter().cloned().collect::<BTreeSet<_>>();
        let mut state = 0x5eed_u64;
        for _ in 0..512 {
            let mut next_set = || {
                let mut set = BTreeSet::new();
                for capability in &capabilities {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1);
                    if state & (1 << 63) != 0 {
                        set.insert(capability.clone());
                    }
                }
                set
            };
            let ceiling = AgentCapabilityCeiling {
                role_maximum: next_set(),
                parent_effective: next_set(),
                root_authorization: next_set(),
                project_policy: next_set(),
                profile_allowlist: next_set(),
                workspace_policy: next_set(),
                phase_feature_gates: next_set(),
            };
            assert!(ceiling.is_monotonic());
            let effective = ceiling.effective();
            assert!(effective.is_subset(&all));
            assert!(effective.is_subset(&ceiling.parent_effective));
            assert!(effective.is_subset(&ceiling.root_authorization));
            assert!(effective.is_subset(&ceiling.project_policy));
            assert!(effective.is_subset(&ceiling.profile_allowlist));
            assert!(effective.is_subset(&ceiling.workspace_policy));
        }
    }

    #[tokio::test]
    async fn every_pending_approval_is_durable_and_attention_visible() {
        let sink = Arc::new(InMemoryRunEventSink::new());
        let (run_id, agent_id, workspace_id, operation_id) = ids("visible");
        let broker = AgentApprovalBroker::new(run_id.clone(), sink.clone()).unwrap();
        let context = context(
            run_id.clone(),
            agent_id,
            workspace(workspace_id, false),
            "root",
        );
        let intent = append_intent(&sink, &context, &operation_id, ToolSideEffect::Mutation);
        let task = start_request(
            broker
                .scoped_client(
                    context.clone(),
                    AgentCapabilityCeiling::uniform(all_capabilities()),
                )
                .unwrap(),
            request(
                &context,
                operation_id,
                intent,
                ToolSideEffect::Mutation,
                AgentCapability::WorkspaceWrite,
                Duration::from_secs(5),
            ),
        )
        .await;
        let pending = broker.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert!(broker.attention_generation() > 0);
        let events = sink.events(&run_id).unwrap();
        assert!(events.iter().any(|event| {
            event.event_id == pending[0].request_event_id
                && matches!(event.kind, EventKind::AgentApprovalRequested(_))
                && event.schema_version == EVENT_SCHEMA_VERSION
                && event.payload_version == EVENT_PAYLOAD_VERSION
        }));
        task.abort();
        let _ = task.await;
    }
}
