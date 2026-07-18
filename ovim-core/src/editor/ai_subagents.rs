//! Editor-owned assembly for the read-only delegated-agent preview.
//!
//! Root chat remains in its existing orchestration path. This service owns a
//! separate per-run supervisor and immutable snapshot stack, while sharing the
//! exact durable run sink and configured provider profiles.

use crate::agent_runtime::{
    AgentApprovalBroker, AgentApprovalKey, AgentApprovalResponse, AgentApprovalResponseDecision,
    AgentCapability, AgentControlPlaneSnapshot, AgentKindName, AgentLoopBudget,
    AgentLoopInputFactory, AgentProviderAdapter, AgentRoleTemplate, AgentSupervisor,
    AgentSupervisorConfig, AgentWorkspaceManager, CapturedSnapshotDiagnostics,
    CapturedSnapshotSymbolIndex, CompletionContract, DelegatedAgentKind, DelegationContextMode,
    DelegationEnvelope, DelegationExpectedOutput, DelegationIdentity, DispatchRequest,
    FollowupAgentRequest, ModelFallbackPolicy, ProfileAgentProvider, ReasoningEffort,
    RequestedModelRoute, SendAgentMessageRequest, SnapshotAgentLoopInputFactory,
    SnapshotDiagnostic, SnapshotSymbol, SubagentModelCatalog, WorkspaceAssignment, WorkspacePolicy,
    WorkspaceStrategy,
};
use crate::ai::chat_types::ToolCallInfo;
use crate::ai::tools::subagents::{
    is_parent_control_tool, FOLLOWUP_AGENT_TOOL, INTERRUPT_AGENT_TOOL, LIST_AGENTS_TOOL,
    SEND_MESSAGE_TOOL, SPAWN_AGENT_TOOL, WAIT_AGENT_TOOL,
};
use crate::ai::tools::ToolDefinition;
use crate::ai::tools::ToolResult;
use crate::ai::{AiConfig, AiSubagentConfig};
use crate::run_log::{
    AgentId, ArtifactStore, BaseManifest, BaseManifestId, EventId, EventKind, LocalRunStore,
    ManifestId, OperationId, RepoPath, RepositoryId, RunEventSink, RunId, TurnId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::Editor;

#[derive(Clone)]
struct ActiveSubagentRoot {
    store: Arc<LocalRunStore>,
    run_id: RunId,
    root_agent_id: AgentId,
    repository_id: RepositoryId,
    turn_id: TurnId,
    caused_by_event: EventId,
    repository_root: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnAgentArguments {
    task_name: String,
    objective: String,
    agent_kind: String,
    model: String,
    reasoning_effort: String,
    context_mode: String,
    expected_output: String,
    relevant_paths: Vec<String>,
    done_when: Vec<String>,
    non_goals: Vec<String>,
    timeout_seconds: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InterruptAgentArguments {
    agent_id: String,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendMessageArguments {
    agent_id: String,
    message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowupAgentArguments {
    agent_id: String,
    objective: String,
}

pub(crate) struct AiSubagentService {
    startup_policy: AiSubagentConfig,
    config_fingerprint: String,
    catalog: Result<Arc<SubagentModelCatalog>, String>,
    provider: Arc<dyn AgentProviderAdapter>,
    runs: Mutex<HashMap<RunId, Arc<AiSubagentRun>>>,
}

pub(crate) struct AiSubagentRun {
    pub run_id: RunId,
    pub store: Arc<LocalRunStore>,
    pub root_agent_id: AgentId,
    pub repository_id: RepositoryId,
    pub supervisor: AgentSupervisor,
    pub approval_broker: Arc<AgentApprovalBroker>,
    pub artifact_store: ArtifactStore,
    pub workspace_manager: AgentWorkspaceManager,
    pub input_factory: Arc<SnapshotAgentLoopInputFactory>,
    manifest_registry: BaseManifestRegistry,
    delegation_registry: PreparedDelegationRegistry,
    repository_root: PathBuf,
}

const PREPARED_DELEGATION_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DurablePreparedDelegation {
    version: u32,
    manifest_id: ManifestId,
    envelope: DelegationEnvelope,
    timeout_millis: u64,
    max_provider_events: usize,
    max_tool_calls: usize,
    symbols: Vec<SnapshotSymbol>,
    diagnostics: Vec<SnapshotDiagnostic>,
}

impl DurablePreparedDelegation {
    fn budget(&self) -> Result<AgentLoopBudget, String> {
        if self.version != PREPARED_DELEGATION_VERSION
            || self.timeout_millis == 0
            || self.max_provider_events == 0
            || self.max_tool_calls == 0
        {
            return Err("prepared delegation has an unsupported version or empty budget".into());
        }
        Ok(AgentLoopBudget {
            timeout: std::time::Duration::from_millis(self.timeout_millis),
            max_provider_events: self.max_provider_events,
            max_tool_calls: self.max_tool_calls,
        })
    }
}

impl AiSubagentService {
    pub fn new(config: &AiConfig) -> Self {
        Self {
            startup_policy: config.subagents.clone(),
            config_fingerprint: subagent_config_fingerprint(config),
            catalog: SubagentModelCatalog::from_config(config)
                .map(Arc::new)
                .map_err(|error| error.to_string()),
            provider: Arc::new(ProfileAgentProvider::new(config)),
            runs: Mutex::new(HashMap::new()),
        }
    }

    pub fn policy(&self) -> &AiSubagentConfig {
        &self.startup_policy
    }

    pub fn catalog(&self) -> Result<Arc<SubagentModelCatalog>, String> {
        self.catalog.clone()
    }

    pub fn parent_tools(&self) -> Result<Vec<ToolDefinition>, String> {
        if !self.startup_policy.enabled {
            return Ok(Vec::new());
        }
        crate::ai::tools::subagents::parent_control_tools(
            self.catalog()?.as_ref(),
            &self.startup_policy,
        )
        .map_err(|error| error.to_string())
    }

    pub fn parent_capabilities(&self) -> BTreeSet<AgentCapability> {
        if self.startup_policy.enabled {
            BTreeSet::from([AgentCapability::DispatchAgents])
        } else {
            BTreeSet::new()
        }
    }

    pub fn attention_generation(&self) -> u64 {
        self.runs
            .lock()
            .map(|runs| {
                runs.values().fold(0_u64, |generation, run| {
                    generation.saturating_add(run.approval_broker.attention_generation())
                })
            })
            .unwrap_or_default()
    }

    fn registered_run(&self, run_id: &RunId) -> Result<Arc<AiSubagentRun>, String> {
        self.runs
            .lock()
            .map_err(|_| "subagent run registry is poisoned".to_string())?
            .get(run_id)
            .cloned()
            .ok_or_else(|| format!("unknown or inactive delegated-agent run {run_id}"))
    }

    /// Running supervisors keep the exact startup policy/profile snapshot.
    /// A mutable Lua/config reload must rebuild AiState rather than silently
    /// changing routing or authority beneath queued children.
    pub fn ensure_compatible(&self, current: &AiConfig) -> Result<(), String> {
        if !self.startup_policy.enabled {
            return Err("the subagent preview is disabled".into());
        }
        if current.subagents != self.startup_policy
            || subagent_config_fingerprint(current) != self.config_fingerprint
        {
            return Err(
                "AI subagent configuration changed after editor startup; restart Ovim before dispatching"
                    .into(),
            );
        }
        let catalog = self.catalog()?;
        if catalog.entries().next().is_none() {
            return Err("the subagent preview has no allowed model routes".into());
        }
        Ok(())
    }

    pub fn run(
        &self,
        store: Arc<LocalRunStore>,
        run_id: RunId,
        root_agent_id: AgentId,
        repository_id: RepositoryId,
        repository_root: PathBuf,
    ) -> Result<Arc<AiSubagentRun>, String> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| "subagent run registry is poisoned".to_string())?;
        if let Some(run) = runs.get(&run_id) {
            if run.root_agent_id != root_agent_id
                || run.repository_id != repository_id
                || run.repository_root != repository_root
            {
                return Err("subagent run identity changed after initialization".into());
            }
            return Ok(run.clone());
        }

        let has_prior_child_history = store
            .events(&run_id)
            .map_err(|error| error.to_string())?
            .iter()
            .any(|event| {
                matches!(
                    &event.kind,
                    EventKind::AgentLifecycle(lifecycle) if lifecycle.dispatch_spec.is_some()
                )
            });
        store
            .layout()
            .ensure_run_directory(&run_id)
            .map_err(|error| error.to_string())?;
        let artifact_store = ArtifactStore::open(store.layout().artifact_directory(&run_id))
            .map_err(|error| error.to_string())?;
        let workspace_manager = AgentWorkspaceManager::new(artifact_store.clone());
        let sink: Arc<dyn RunEventSink> = store.clone();
        let approval_broker = Arc::new(
            AgentApprovalBroker::new(run_id.clone(), sink.clone())
                .map_err(|error| error.to_string())?,
        );
        let input_factory = Arc::new(SnapshotAgentLoopInputFactory::with_approval_broker(
            workspace_manager.clone(),
            self.provider.clone(),
            approval_broker.clone(),
        ));
        let supervisor = if has_prior_child_history {
            AgentSupervisor::rehydrate(
                run_id.clone(),
                root_agent_id.clone(),
                sink,
                self.catalog()?,
                input_factory.clone(),
                supervisor_config(&self.startup_policy),
            )
        } else {
            AgentSupervisor::new(
                run_id.clone(),
                root_agent_id.clone(),
                sink,
                self.catalog()?,
                input_factory.clone(),
                supervisor_config(&self.startup_policy),
            )
        }
        .map_err(|error| error.to_string())?;
        let run_directory = store.layout().run_directory(&run_id);
        let run = Arc::new(AiSubagentRun {
            run_id: run_id.clone(),
            store,
            root_agent_id,
            repository_id,
            supervisor,
            approval_broker,
            artifact_store,
            workspace_manager,
            input_factory,
            manifest_registry: BaseManifestRegistry::new(run_directory.join("manifests")),
            delegation_registry: PreparedDelegationRegistry::new(run_directory.join("delegations")),
            repository_root,
        });
        if has_prior_child_history {
            run.restore_prepared_dispatches();
        }
        runs.insert(run_id, run.clone());
        if has_prior_child_history
            && run
                .supervisor
                .dispatches()
                .map_err(|error| error.to_string())?
                .iter()
                .any(|record| record.state == crate::agent_runtime::DispatchState::Queued)
        {
            let supervisor = run.supervisor.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    if let Err(error) = supervisor.start_recovered().await {
                        crate::log_warn!(
                            "agent_runtime",
                            "could not restart durable queued children: {error}"
                        );
                    }
                });
            } else {
                crate::log_warn!(
                    "agent_runtime",
                    "durable queued children require an async runtime before restart"
                );
            }
        }
        Ok(run)
    }
}

impl AiSubagentRun {
    /// Publish manifest metadata before making the projection available to a
    /// queue runner. The content blobs were already fsynced by ArtifactStore.
    pub fn register_manifest(
        &self,
        manifest_id: ManifestId,
        manifest: BaseManifest,
        repository_start: &Path,
    ) -> Result<(), String> {
        if manifest.repository.repository_id != self.repository_id {
            return Err("captured manifest belongs to another repository".into());
        }
        self.manifest_registry
            .register(&manifest_id, &manifest)
            .map_err(|error| error.to_string())?;
        self.workspace_manager
            .register_read_only(manifest_id, manifest, repository_start)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn register_prepared_delegation(
        &self,
        manifest_id: ManifestId,
        envelope: DelegationEnvelope,
        budget: AgentLoopBudget,
        symbols: Vec<SnapshotSymbol>,
        diagnostics: Vec<SnapshotDiagnostic>,
    ) -> Result<(), String> {
        let timeout_millis = u64::try_from(budget.timeout.as_millis())
            .map_err(|_| "delegated-agent timeout does not fit durable storage".to_string())?;
        let prepared = DurablePreparedDelegation {
            version: PREPARED_DELEGATION_VERSION,
            manifest_id: manifest_id.clone(),
            envelope: envelope.clone(),
            timeout_millis,
            max_provider_events: budget.max_provider_events,
            max_tool_calls: budget.max_tool_calls,
            symbols: symbols.clone(),
            diagnostics: diagnostics.clone(),
        };
        // Publish the complete restart contract before a scheduler identity can
        // be allocated. A crash may leave an unreferenced prepared record, but
        // can never leave a queued child whose context existed only in memory.
        self.delegation_registry
            .register(&manifest_id, &prepared)
            .map_err(|error| error.to_string())?;
        self.workspace_manager
            .register_snapshot_adapters(
                &manifest_id,
                Arc::new(CapturedSnapshotSymbolIndex::new(
                    manifest_id.clone(),
                    symbols,
                )),
                Arc::new(CapturedSnapshotDiagnostics::new(
                    manifest_id.clone(),
                    diagnostics,
                )),
            )
            .map_err(|error| error.to_string())?;
        self.input_factory
            .register_prepared(manifest_id, envelope, Some(budget))
    }

    fn restore_prepared_dispatches(&self) {
        let records = match self.supervisor.dispatches() {
            Ok(records) => records,
            Err(error) => {
                crate::log_warn!(
                    "agent_runtime",
                    "could not inspect recovered child dispatches: {error}"
                );
                return;
            }
        };
        for record in records {
            let WorkspaceStrategy::ReadOnlySnapshot {
                manifest_id: Some(manifest_id),
            } = &record.handle.workspace.strategy
            else {
                continue;
            };
            if let Err(error) = self.restore_prepared_dispatch(&record, manifest_id) {
                // A queued child with missing/corrupt context will be started
                // only far enough for the input factory to fail closed and
                // record a validated terminal handoff. Terminal history still
                // remains inspectable even when an old preview predates this
                // registry.
                crate::log_warn!(
                    "agent_runtime",
                    "could not restore delegation context for {}: {error}",
                    record.handle.agent_id
                );
            }
        }
    }

    fn restore_prepared_dispatch(
        &self,
        record: &crate::agent_runtime::AgentDispatchRecord,
        manifest_id: &ManifestId,
    ) -> Result<(), String> {
        let manifest = self
            .manifest_registry
            .load(manifest_id)
            .map_err(|error| error.to_string())?;
        if manifest.repository.repository_id != self.repository_id {
            return Err("captured manifest belongs to another repository".into());
        }
        let prepared = self
            .delegation_registry
            .load(manifest_id)
            .map_err(|error| error.to_string())?;
        if prepared.manifest_id != *manifest_id
            || prepared.envelope.task_name != record.task_name
            || (record.turn_generation == 0 && prepared.envelope.objective != record.objective)
        {
            return Err("prepared delegation disagrees with durable dispatch identity".into());
        }
        let identity = prepared
            .envelope
            .identity
            .as_deref()
            .ok_or_else(|| "prepared delegation has no durable identity".to_string())?;
        if identity.run_id != record.handle.run_id
            || identity.workspace_id != record.handle.workspace.workspace_id
            || identity.manifest_id != *manifest_id
            || record.parent_agent_id.as_ref() != Some(&identity.parent_agent_id)
        {
            return Err("prepared delegation changed run, parent, workspace, or manifest".into());
        }
        let budget = prepared.budget()?;
        self.workspace_manager
            .register_read_only(
                manifest_id.clone(),
                manifest,
                self.repository_root.as_path(),
            )
            .map_err(|error| error.to_string())?;
        self.workspace_manager
            .register_snapshot_adapters(
                manifest_id,
                Arc::new(CapturedSnapshotSymbolIndex::new(
                    manifest_id.clone(),
                    prepared.symbols,
                )),
                Arc::new(CapturedSnapshotDiagnostics::new(
                    manifest_id.clone(),
                    prepared.diagnostics,
                )),
            )
            .map_err(|error| error.to_string())?;
        self.input_factory
            .register_prepared(manifest_id.clone(), prepared.envelope, Some(budget))
    }

    #[cfg(test)]
    fn manifest(&self, manifest_id: &ManifestId) -> Result<BaseManifest, String> {
        self.manifest_registry
            .load(manifest_id)
            .map_err(|error| error.to_string())
    }
}

impl Editor {
    /// Build one restart-safe, transport-neutral snapshot from the existing
    /// editor-owned supervisor and durable projections.
    pub fn ai_agent_snapshot(&self, run_id: &RunId) -> Result<AgentControlPlaneSnapshot, String> {
        let run = self.ai_state.subagents.registered_run(run_id)?;
        let events = run
            .store
            .events(run_id)
            .map_err(|error| error.to_string())?;
        let pending_notifications = run
            .supervisor
            .mailbox(run.root_agent_id.clone())
            .map_err(|error| error.to_string())?
            .pending()
            .map_err(|error| error.to_string())?
            .len();
        crate::agent_runtime::build_agent_snapshot(
            run_id.clone(),
            run.root_agent_id.clone(),
            run.supervisor
                .dispatches()
                .map_err(|error| error.to_string())?,
            &events,
            run.supervisor
                .messages()
                .map_err(|error| error.to_string())?,
            run.approval_broker
                .pending()
                .map_err(|error| error.to_string())?,
            run.approval_broker
                .resolved()
                .map_err(|error| error.to_string())?,
            pending_notifications,
        )
    }

    pub fn ai_agent_events(
        &self,
        run_id: &RunId,
        agent_id: &AgentId,
        after_sequence: u64,
        limit: usize,
    ) -> Result<Vec<crate::run_log::EventEnvelope>, String> {
        if !(1..=1_000).contains(&limit) {
            return Err("agent event limit must be between 1 and 1000".into());
        }
        let run = self.ai_state.subagents.registered_run(run_id)?;
        ensure_run_agent(&run, agent_id, None)?;
        Ok(run
            .store
            .events(run_id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|event| {
                event.sequence > after_sequence && event.agent_id.as_ref() == Some(agent_id)
            })
            .take(limit)
            .collect())
    }

    pub fn prepare_ai_agent_wait(
        &self,
        run_id: &RunId,
        agent_id: &AgentId,
        turn_generation: u32,
        timeout: std::time::Duration,
    ) -> Result<PreparedHeadlessAgentControl, String> {
        if timeout.is_zero() || timeout > std::time::Duration::from_secs(60) {
            return Err("agent wait timeout must be between 1ms and 60s".into());
        }
        let run = self.ai_state.subagents.registered_run(run_id)?;
        ensure_run_agent(&run, agent_id, Some(turn_generation))?;
        Ok(PreparedHeadlessAgentControl::Wait {
            mailbox: run
                .supervisor
                .mailbox(run.root_agent_id.clone())
                .map_err(|error| error.to_string())?,
            agent_id: agent_id.clone(),
            timeout,
        })
    }

    pub fn prepare_ai_agent_interrupt(
        &self,
        run_id: &RunId,
        agent_id: &AgentId,
        turn_generation: u32,
        reason: String,
    ) -> Result<PreparedHeadlessAgentControl, String> {
        let run = self.ai_state.subagents.registered_run(run_id)?;
        ensure_run_agent(&run, agent_id, Some(turn_generation))?;
        Ok(PreparedHeadlessAgentControl::Interrupt {
            supervisor: run.supervisor.clone(),
            agent_id: agent_id.clone(),
            reason,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ai_agent_send_message(
        &self,
        run_id: &RunId,
        agent_id: &AgentId,
        turn_generation: u32,
        parent_agent_id: AgentId,
        causing_turn_id: TurnId,
        caused_by_event: EventId,
        content: String,
    ) -> Result<serde_json::Value, String> {
        let run = self.ai_state.subagents.registered_run(run_id)?;
        ensure_run_agent(&run, agent_id, Some(turn_generation))?;
        if parent_agent_id != run.root_agent_id {
            return Err("message parent does not match the run root agent".into());
        }
        let message = run
            .supervisor
            .send_message(SendAgentMessageRequest {
                sender_agent_id: parent_agent_id,
                recipient_agent_id: agent_id.clone(),
                causing_turn_id,
                caused_by_event,
                content,
            })
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "outcome": "queued",
            "run_id": run_id,
            "agent_id": agent_id,
            "message_event_id": message.message_event_id,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_ai_agent_followup(
        &self,
        run_id: &RunId,
        agent_id: &AgentId,
        turn_generation: u32,
        parent_agent_id: AgentId,
        causing_turn_id: TurnId,
        caused_by_event: EventId,
        objective: String,
    ) -> Result<PreparedHeadlessAgentControl, String> {
        let run = self.ai_state.subagents.registered_run(run_id)?;
        let record = ensure_run_agent(&run, agent_id, Some(turn_generation))?;
        if parent_agent_id != run.root_agent_id {
            return Err("follow-up parent does not match the run root agent".into());
        }
        let dependencies = run
            .input_factory
            .build(&record)
            .map_err(|error| error.to_string())?;
        let budget = dependencies
            .budget
            .unwrap_or_else(|| supervisor_config(self.ai_state.subagents.policy()).child_budget);
        Ok(PreparedHeadlessAgentControl::Followup {
            supervisor: run.supervisor.clone(),
            request: FollowupAgentRequest {
                agent_id: agent_id.clone(),
                parent_agent_id,
                causing_turn_id,
                caused_by_event,
                objective,
                capabilities: None,
                budget,
                retained_session_requested: false,
            },
        })
    }

    pub fn ai_agent_respond_approval(
        &self,
        run_id: &RunId,
        agent_id: &AgentId,
        turn_generation: u32,
        operation_id: OperationId,
        request_event_id: EventId,
        allow: bool,
        reason: Option<String>,
    ) -> Result<serde_json::Value, String> {
        let run = self.ai_state.subagents.registered_run(run_id)?;
        ensure_run_agent(&run, agent_id, Some(turn_generation))?;
        let decision = if allow {
            AgentApprovalResponseDecision::Allow
        } else {
            AgentApprovalResponseDecision::Deny { reason }
        };
        run.approval_broker
            .respond(AgentApprovalResponse {
                key: AgentApprovalKey {
                    run_id: run_id.clone(),
                    agent_id: agent_id.clone(),
                    operation_id: operation_id.clone(),
                },
                request_event_id: request_event_id.clone(),
                decision,
            })
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "outcome": if allow { "allowed" } else { "denied" },
            "run_id": run_id,
            "agent_id": agent_id,
            "operation_id": operation_id,
            "request_event_id": request_event_id,
        }))
    }

    /// Parent controls are a root-runtime capability, not ordinary profile
    /// tools. Merely registering a profile or opening a chat never exposes
    /// them; the durable root binding must be active and exact.
    pub(crate) fn ai_subagent_parent_tools_visible(&self) -> bool {
        self.ai_state
            .subagents
            .parent_capabilities()
            .contains(&AgentCapability::DispatchAgents)
            && self.active_subagent_root().is_ok()
    }

    pub(crate) fn ai_subagent_parent_tools(&self) -> Vec<ToolDefinition> {
        if !self.ai_subagent_parent_tools_visible() {
            return Vec::new();
        }
        self.ai_state.subagents.parent_tools().unwrap_or_default()
    }

    pub(crate) fn is_ai_subagent_control_tool(&self, name: &str) -> bool {
        is_parent_control_tool(name)
    }

    pub(crate) fn execute_ai_subagent_control_tool(&mut self, call: &ToolCallInfo) -> ToolResult {
        if !is_parent_control_tool(&call.name) {
            return ToolResult::Error("unknown delegated-agent control".into());
        }
        let definition = match self
            .ai_subagent_parent_tools()
            .into_iter()
            .find(|definition| definition.name == call.name)
        {
            Some(definition) => definition,
            None => {
                return ToolResult::Error(
                    "delegated-agent controls require an enabled durable root turn with DispatchAgents authority"
                        .into(),
                )
            }
        };
        if let Some(schema) = definition.custom_input_schema.as_ref()
            && let Err(error) = schema.validate_instance(&call.arguments)
        {
            return ToolResult::Error(format!("invalid {} arguments: {error}", call.name));
        }

        let result = match call.name.as_str() {
            SPAWN_AGENT_TOOL => self.spawn_ai_subagent(&call.arguments),
            LIST_AGENTS_TOOL => self.list_ai_subagents(),
            SEND_MESSAGE_TOOL => self.send_ai_subagent_message(&call.arguments),
            FOLLOWUP_AGENT_TOOL => Err(
                "followup_agent is asynchronous and must be dispatched through the editor turn loop"
                    .into(),
            ),
            WAIT_AGENT_TOOL => Err(
                "wait_agent is asynchronous and must be dispatched through the editor turn loop"
                    .into(),
            ),
            INTERRUPT_AGENT_TOOL => Err(
                "interrupt_agent is asynchronous and must be dispatched through the editor turn loop"
                    .into(),
            ),
            _ => unreachable!("parent-control name was checked above"),
        };
        match result {
            Ok(value) => ToolResult::Success(value.to_string()),
            Err(error) => ToolResult::Error(error),
        }
    }

    pub(crate) fn prepare_ai_subagent_async_control(
        &self,
        call: &ToolCallInfo,
    ) -> Result<PreparedAsyncSubagentControl, String> {
        if !matches!(
            call.name.as_str(),
            WAIT_AGENT_TOOL | INTERRUPT_AGENT_TOOL | FOLLOWUP_AGENT_TOOL
        ) {
            return Err("delegated-agent control does not run asynchronously".into());
        }
        let definition = self
            .ai_subagent_parent_tools()
            .into_iter()
            .find(|definition| definition.name == call.name)
            .ok_or_else(|| {
                "delegated-agent controls require an enabled durable root turn with DispatchAgents authority"
                    .to_string()
            })?;
        if let Some(schema) = definition.custom_input_schema.as_ref() {
            schema
                .validate_instance(&call.arguments)
                .map_err(|error| format!("invalid {} arguments: {error}", call.name))?;
        }
        let root = self.active_subagent_root()?;
        let run = self.ai_state.subagents.run(
            root.store,
            root.run_id,
            root.root_agent_id.clone(),
            root.repository_id,
            root.repository_root.clone(),
        )?;
        match call.name.as_str() {
            WAIT_AGENT_TOOL => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Arguments {
                    timeout_seconds: u64,
                }
                let args: Arguments = serde_json::from_value(call.arguments.clone())
                    .map_err(|error| error.to_string())?;
                Ok(PreparedAsyncSubagentControl::Wait {
                    mailbox: run
                        .supervisor
                        .mailbox(root.root_agent_id)
                        .map_err(|e| e.to_string())?,
                    timeout: std::time::Duration::from_secs(args.timeout_seconds),
                })
            }
            INTERRUPT_AGENT_TOOL => {
                let args: InterruptAgentArguments = serde_json::from_value(call.arguments.clone())
                    .map_err(|error| error.to_string())?;
                let agent_id = AgentId::parse(args.agent_id).map_err(|error| error.to_string())?;
                Ok(PreparedAsyncSubagentControl::Interrupt {
                    supervisor: run.supervisor.clone(),
                    agent_id,
                    reason: args.reason,
                })
            }
            FOLLOWUP_AGENT_TOOL => {
                let args: FollowupAgentArguments = serde_json::from_value(call.arguments.clone())
                    .map_err(|error| error.to_string())?;
                let agent_id = AgentId::parse(args.agent_id).map_err(|error| error.to_string())?;
                let record = run
                    .supervisor
                    .dispatches()
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .find(|record| record.handle.agent_id == agent_id)
                    .ok_or_else(|| format!("unknown delegated agent {agent_id}"))?;
                let dependencies = run
                    .input_factory
                    .build(&record)
                    .map_err(|error| error.to_string())?;
                let budget = dependencies.budget.unwrap_or_else(|| {
                    supervisor_config(self.ai_state.subagents.policy()).child_budget
                });
                Ok(PreparedAsyncSubagentControl::Followup {
                    supervisor: run.supervisor.clone(),
                    request: FollowupAgentRequest {
                        agent_id,
                        parent_agent_id: root.root_agent_id,
                        causing_turn_id: root.turn_id,
                        caused_by_event: root.caused_by_event,
                        objective: args.objective,
                        capabilities: None,
                        budget,
                        retained_session_requested: false,
                    },
                })
            }
            _ => unreachable!(),
        }
    }

    pub(crate) fn begin_pending_ai_subagent_control(
        &mut self,
        call: ToolCallInfo,
        continuation: super::ai_chat_state::SubagentControlContinuation,
    ) -> Result<
        (),
        (
            ToolResult,
            Box<super::ai_chat_state::SubagentControlContinuation>,
        ),
    > {
        if self.ai_state.chat.is_none() {
            return Err((
                ToolResult::Error("no active chat session".into()),
                Box::new(continuation),
            ));
        }
        let prepared = match self.prepare_ai_subagent_async_control(&call) {
            Ok(prepared) => prepared,
            Err(error) => return Err((ToolResult::Error(error), Box::new(continuation))),
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = sender.send(prepared.execute().await);
        });
        let chat = self
            .ai_state
            .chat
            .as_mut()
            .expect("active chat checked above");
        chat.pending_subagent_control = Some(super::ai_chat_state::PendingSubagentControl {
            tool_call: call,
            continuation,
            receiver,
            task,
        });
        chat.waiting = true;
        self.set_lsp_status("Waiting for delegated-agent activity".into());
        Ok(())
    }

    pub(crate) fn poll_pending_ai_subagent_control(&mut self) -> bool {
        let user_steering = self.ai_state.chat.as_ref().is_some_and(|chat| {
            !chat.queued_inputs.is_empty()
                && chat
                    .pending_subagent_control
                    .as_ref()
                    .is_some_and(|pending| pending.tool_call.name == WAIT_AGENT_TOOL)
        });
        let received = if user_steering {
            Some(ToolResult::Success(
                json!({ "outcome": "user_steering", "updates": [] }).to_string(),
            ))
        } else {
            let Some(pending) = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|chat| chat.pending_subagent_control.as_mut())
            else {
                return false;
            };
            match pending.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return false,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Some(ToolResult::Error(
                    "delegated-agent control stopped without returning a result".into(),
                )),
            }
        };
        let pending = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.pending_subagent_control.take())
            .expect("pending delegated-agent control exists");
        if user_steering {
            pending.task.abort();
        }
        let mut result = received.expect("completed control produced a result");
        if pending.tool_call.name == WAIT_AGENT_TOOL
            && !user_steering
            && let ToolResult::Success(payload) = &result
            && let Err(error) = self.consume_ai_subagent_updates(payload)
        {
            result = ToolResult::Error(error);
        }
        match pending.continuation {
            super::ai_chat_state::SubagentControlContinuation::Dynamic {
                runtime_tool,
                runtime_turn,
                response,
            } => {
                self.finish_dynamic_tool(
                    &runtime_turn,
                    &runtime_tool,
                    &pending.tool_call,
                    response,
                    result,
                );
                self.set_lsp_status(String::new());
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.waiting = true;
                }
                true
            }
            super::ai_chat_state::SubagentControlContinuation::Batch {
                runtime_tool,
                runtime_turn,
                remaining_tool_calls,
                model_name,
            } => {
                if let (Some(turn), Some(tool)) = (runtime_turn.as_ref(), runtime_tool.as_ref())
                    && let Err(error) = self.ai_runtime_finish_tool(turn, tool, &result)
                {
                    self.ai_runtime_fail_turn(format!(
                        "failed to record delegated-agent tool result: {error}"
                    ));
                    self.clear_streaming_state();
                    return true;
                }
                self.record_tool_event_summary(&pending.tool_call, &result);
                let result_content =
                    self.format_tool_result_with_target(&pending.tool_call, &result);
                if let Some(conversation) = self.conversation_mut() {
                    conversation.append_tool_result(pending.tool_call.id, result_content);
                }
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.tool_call_count = chat.tool_call_count.saturating_add(1);
                }
                self.set_lsp_status(String::new());
                self.execute_tool_call_batch(remaining_tool_calls, model_name)
            }
        }
    }

    fn consume_ai_subagent_updates(&self, payload: &str) -> Result<(), String> {
        let value: serde_json::Value = serde_json::from_str(payload)
            .map_err(|error| format!("invalid delegated-agent wait result: {error}"))?;
        let Some(updates) = value.get("updates").and_then(serde_json::Value::as_array) else {
            return Ok(());
        };
        if updates.is_empty() {
            return Ok(());
        }
        let root = self.active_subagent_root()?;
        let run = self.ai_state.subagents.run(
            root.store,
            root.run_id,
            root.root_agent_id.clone(),
            root.repository_id,
            root.repository_root,
        )?;
        let mailbox = run
            .supervisor
            .mailbox(root.root_agent_id)
            .map_err(|error| error.to_string())?;
        for update in updates {
            let id = update
                .get("notification_event_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "delegated-agent update has no event identity".to_string())?;
            mailbox
                .consume(&EventId::parse(id).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn active_subagent_root(&self) -> Result<ActiveSubagentRoot, String> {
        self.ai_state
            .subagents
            .ensure_compatible(&self.ai_state.config)?;
        let turn = self
            .active_ai_runtime_turn()
            .ok_or_else(|| "there is no active root agent turn".to_string())?;
        let services = self
            .ai_state
            .durable_runs
            .as_ref()
            .ok_or_else(|| "durable run storage is unavailable".to_string())?;
        let binding = self
            .ai_state
            .durable_chat_bindings
            .get(&self.ai_chat_conversation_key())
            .ok_or_else(|| "the active chat has no durable root binding".to_string())?;
        if binding.binding.run_id != turn.run_id
            || binding.binding.root_agent_id != turn.agent_id
            || binding.binding.workspace_id != turn.workspace_id
        {
            return Err("active root turn does not match its durable binding".into());
        }
        let caused_by_event = self
            .ai_state
            .agent_runtime
            .selected_branch_tip(&binding.locator)
            .cloned()
            .ok_or_else(|| "active root branch has no durable causal tip".to_string())?;
        let repository_root = self
            .ai_repo_root()
            .ok_or_else(|| "delegated agents require an active Git repository".to_string())?;
        Ok(ActiveSubagentRoot {
            store: services.store.clone(),
            run_id: turn.run_id,
            root_agent_id: turn.agent_id,
            repository_id: binding.binding.key.repository_id.clone(),
            turn_id: turn.turn_id,
            caused_by_event,
            repository_root,
        })
    }

    fn spawn_ai_subagent(
        &mut self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let args: SpawnAgentArguments =
            serde_json::from_value(arguments.clone()).map_err(|error| error.to_string())?;
        let root = self.active_subagent_root()?;
        let effort = ReasoningEffort::new(args.reasoning_effort.clone())
            .map_err(|error| error.to_string())?;
        let requested_route = RequestedModelRoute {
            catalog_model_id: args.model,
            reasoning_effort: effort,
            fallback_policy: ModelFallbackPolicy::FailClosed,
        };
        // Resolve before allocating any child, workspace, manifest, or
        // lifecycle identity. Invalid model/effort pairs leave no durable
        // dispatch residue.
        let catalog = self.ai_state.subagents.catalog()?;
        let resolved = catalog
            .resolve(&requested_route, true)
            .map_err(|error| error.to_string())?;

        let run = self.ai_state.subagents.run(
            root.store,
            root.run_id.clone(),
            root.root_agent_id.clone(),
            root.repository_id.clone(),
            root.repository_root.clone(),
        )?;
        let manifest_id = ManifestId::new();
        let base_manifest_id = BaseManifestId::new();
        let workspace_id = WorkspaceId::new();
        let capture = self
            .capture_ai_base_manifest(
                &root.repository_root,
                root.repository_id,
                base_manifest_id,
                chrono::Utc::now().to_rfc3339(),
                &run.artifact_store,
            )
            .map_err(|error| error.to_string())?;
        let symbols = self.capture_snapshot_symbols(&root.repository_root);
        let diagnostics = self.capture_snapshot_diagnostics();
        run.register_manifest(manifest_id.clone(), capture.manifest, &root.repository_root)?;

        let agent_kind = match args.agent_kind.as_str() {
            "explorer" => DelegatedAgentKind::Explorer,
            "reviewer" => DelegatedAgentKind::Reviewer,
            _ => return Err("only explorer and reviewer delegated roles are supported".into()),
        };
        let expected_output = match args.expected_output.as_str() {
            "analysis" => DelegationExpectedOutput::Analysis,
            "review_report" => DelegationExpectedOutput::ReviewReport,
            "verification" => DelegationExpectedOutput::Verification,
            _ => return Err("unsupported delegated output contract".into()),
        };
        if args.context_mode != "brief" {
            return Err("only brief delegated context is supported".into());
        }
        let role_name = match agent_kind {
            DelegatedAgentKind::Explorer => AgentKindName::Explorer,
            DelegatedAgentKind::Reviewer => AgentKindName::Reviewer,
        };
        let role = AgentRoleTemplate {
            name: role_name,
            instructions: "Use only the immutable captured snapshot. Report bounded evidence and never mutate source or perform external effects.".into(),
            capabilities: BTreeSet::from([AgentCapability::Read]),
            workspace_policy: WorkspacePolicy::ReadOnlyProjection,
            completion_contract: CompletionContract::ReviewReport,
        };
        let envelope = DelegationEnvelope {
            version: 1,
            task_name: args.task_name.clone(),
            objective: args.objective.clone(),
            agent_kind,
            context_mode: DelegationContextMode::Brief,
            expected_output,
            done_when: args.done_when,
            non_goals: args.non_goals,
            relevant_paths: args.relevant_paths,
            parent_brief: None,
            identity: Some(Box::new(DelegationIdentity {
                run_id: root.run_id.clone(),
                parent_agent_id: root.root_agent_id.clone(),
                causing_turn_id: root.turn_id.clone(),
                causing_event_id: root.caused_by_event.clone(),
                workspace_id: workspace_id.clone(),
                manifest_id: manifest_id.clone(),
            })),
            effective_capabilities: vec!["read".into()],
            timeout_seconds: args.timeout_seconds,
            workspace_warnings: capture
                .adapter_issues
                .into_iter()
                .map(|issue| crate::agent_runtime::AgentWorkspaceWarning {
                    kind: crate::agent_runtime::AgentWorkspaceWarningKind::CaptureIssue,
                    path: issue.path,
                    artifact_id: None,
                    detail: issue.detail,
                })
                .collect(),
        };
        run.register_prepared_delegation(
            manifest_id.clone(),
            envelope,
            AgentLoopBudget {
                timeout: std::time::Duration::from_secs(args.timeout_seconds),
                max_provider_events: self
                    .ai_state
                    .subagents
                    .policy()
                    .budgets
                    .max_provider_events_per_agent,
                max_tool_calls: self
                    .ai_state
                    .subagents
                    .policy()
                    .budgets
                    .max_tool_calls_per_agent,
            },
            symbols,
            diagnostics,
        )?;
        let handle = run
            .supervisor
            .dispatch_nonblocking(DispatchRequest {
                task_name: args.task_name.clone(),
                objective: args.objective,
                role,
                requested_route,
                parent_agent_id: Some(root.root_agent_id),
                causing_turn_id: Some(root.turn_id),
                caused_by_event: Some(root.caused_by_event),
                workspace: WorkspaceAssignment {
                    workspace_id,
                    strategy: WorkspaceStrategy::ReadOnlySnapshot {
                        manifest_id: Some(manifest_id.clone()),
                    },
                },
            })
            .map_err(|error| error.to_string())?;
        let state = run
            .supervisor
            .state(&handle.agent_id)
            .map_err(|error| error.to_string())?
            .map(|state| format!("{state:?}").to_lowercase())
            .unwrap_or_else(|| "created".into());
        Ok(json!({
            "task_name": args.task_name,
            "run_id": handle.run_id,
            "agent_id": handle.agent_id,
            "workspace_id": handle.workspace.workspace_id,
            "manifest_id": manifest_id,
            "model": resolved.catalog_model_id,
            "profile": resolved.profile_name,
            "provider": resolved.provider,
            "reasoning_effort": resolved.reasoning_effort,
            "state": state
        }))
    }

    fn send_ai_subagent_message(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let args: SendMessageArguments =
            serde_json::from_value(arguments.clone()).map_err(|error| error.to_string())?;
        let root = self.active_subagent_root()?;
        let run = self.ai_state.subagents.run(
            root.store,
            root.run_id,
            root.root_agent_id.clone(),
            root.repository_id,
            root.repository_root,
        )?;
        let recipient_agent_id =
            AgentId::parse(args.agent_id).map_err(|error| error.to_string())?;
        let message = run
            .supervisor
            .send_message(SendAgentMessageRequest {
                sender_agent_id: root.root_agent_id,
                recipient_agent_id,
                causing_turn_id: root.turn_id,
                caused_by_event: root.caused_by_event,
                content: args.message,
            })
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "outcome": "queued",
            "message_event_id": message.message_event_id,
            "agent_id": message.recipient_agent_id,
            "state": "queued"
        }))
    }

    fn list_ai_subagents(&self) -> Result<serde_json::Value, String> {
        let root = self.active_subagent_root()?;
        let run = self.ai_state.subagents.run(
            root.store,
            root.run_id,
            root.root_agent_id.clone(),
            root.repository_id,
            root.repository_root,
        )?;
        let pending = run
            .supervisor
            .mailbox(root.root_agent_id)
            .map_err(|error| error.to_string())?
            .pending()
            .map_err(|error| error.to_string())?;
        let pending_approvals = run
            .approval_broker
            .pending()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|approval| {
                json!({
                    "request_event_id": approval.request_event_id,
                    "agent_id": approval.key.agent_id,
                    "operation_id": approval.key.operation_id,
                    "task_name": approval.request.task_name,
                    "ancestry": approval.request.ancestry,
                    "role": approval.request.role,
                    "model": approval.request.model,
                    "reasoning_effort": approval.request.reasoning_effort,
                    "tool": approval.request.tool_name,
                    "effect": approval.request.normalized_effect,
                    "workspace": approval.request.workspace,
                    "reason": approval.request.reason,
                    "created_at": approval.request.created_at,
                    "deadline_at": approval.request.deadline_at,
                })
            })
            .collect::<Vec<_>>();
        let messages = run
            .supervisor
            .messages()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|message| {
                let (state, detail) = match message.state {
                    crate::agent_runtime::AgentMessageState::Queued => ("queued", None),
                    crate::agent_runtime::AgentMessageState::Delivering { .. } => {
                        ("delivering", None)
                    }
                    crate::agent_runtime::AgentMessageState::Delivered { .. } => {
                        ("delivered", None)
                    }
                    crate::agent_runtime::AgentMessageState::Rejected { detail, .. } => {
                        ("rejected", Some(detail))
                    }
                };
                json!({
                    "message_event_id": message.message_event_id,
                    "sender_agent_id": message.sender_agent_id,
                    "recipient_agent_id": message.recipient_agent_id,
                    "state": state,
                    "detail": detail,
                    "consumed": message.consumption_event_id.is_some(),
                })
            })
            .collect::<Vec<_>>();
        let records = run
            .supervisor
            .dispatches()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|record| {
                json!({
                    "task_name": record.task_name,
                    "agent_id": record.handle.agent_id,
                    "parent_agent_id": record.parent_agent_id,
                    "workspace_id": record.handle.workspace.workspace_id,
                    "workspace": format!("{:?}", record.handle.workspace.strategy),
                    "agent_kind": record.role.name.to_string(),
                    "model": record.resolved_route.catalog_model_id,
                    "profile": record.resolved_route.profile_name,
                    "provider": record.resolved_route.provider,
                    "reasoning_effort": record.resolved_route.reasoning_effort,
                    "state": format!("{:?}", record.state).to_lowercase(),
                    "queue_sequence": record.queue_sequence,
                    "turn_generation": record.turn_generation,
                    "followup_turn_id": record.followup.as_ref().map(|turn| &turn.followup_turn_id),
                })
            })
            .collect::<Vec<_>>();
        let pending_attention = pending.len() + pending_approvals.len();
        Ok(json!({
            "agents": records,
            "pending_approvals": pending_approvals,
            "messages": messages,
            "pending_attention": pending_attention,
        }))
    }

    fn capture_snapshot_symbols(&self, repository_root: &Path) -> Vec<SnapshotSymbol> {
        let Some(path) = self.buffer().file_path().map(Path::new) else {
            return Vec::new();
        };
        let Ok(relative) = path.strip_prefix(repository_root) else {
            return Vec::new();
        };
        let Ok(path) = RepoPath::parse(relative.to_string_lossy().replace('\\', "/")) else {
            return Vec::new();
        };
        fn flatten(
            output: &mut Vec<SnapshotSymbol>,
            path: &RepoPath,
            symbols: &[lsp_types::DocumentSymbol],
        ) {
            for symbol in symbols {
                output.push(SnapshotSymbol {
                    name: symbol.name.clone(),
                    kind: format!("{:?}", symbol.kind).to_lowercase(),
                    path: path.clone(),
                    line: symbol.range.start.line as usize + 1,
                    column: symbol.range.start.character as usize + 1,
                });
                if let Some(children) = symbol.children.as_deref() {
                    flatten(output, path, children);
                }
            }
        }
        let mut captured = Vec::new();
        flatten(
            &mut captured,
            &path,
            &self.lsp.state.available_document_symbols,
        );
        captured
    }

    fn capture_snapshot_diagnostics(&self) -> Vec<SnapshotDiagnostic> {
        self.get_project_diagnostics_for_chat()
            .into_iter()
            .filter(|file| {
                file.buffer_revision
                    .is_some_and(|revision| file.lsp_versions.contains(&(revision as i32)))
            })
            .filter_map(|file| {
                let path = RepoPath::parse(file.path).ok()?;
                Some(
                    file.diagnostics
                        .into_iter()
                        .map(move |diagnostic| SnapshotDiagnostic {
                            path: path.clone(),
                            line: diagnostic.line as usize + 1,
                            column: diagnostic.start_character as usize + 1,
                            severity: diagnostic.severity.unwrap_or_else(|| "unknown".into()),
                            message: diagnostic.message,
                        }),
                )
            })
            .flatten()
            .collect()
    }
}

fn ensure_run_agent(
    run: &AiSubagentRun,
    agent_id: &AgentId,
    turn_generation: Option<u32>,
) -> Result<crate::agent_runtime::AgentDispatchRecord, String> {
    let record = run
        .supervisor
        .dispatches()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|record| &record.handle.agent_id == agent_id)
        .ok_or_else(|| format!("agent {agent_id} does not belong to run {}", run.run_id))?;
    if turn_generation.is_some_and(|generation| generation != record.turn_generation) {
        return Err(format!(
            "stale agent generation: expected {}, current is {}",
            turn_generation.unwrap_or_default(),
            record.turn_generation
        ));
    }
    Ok(record)
}

pub enum PreparedHeadlessAgentControl {
    Wait {
        mailbox: crate::agent_runtime::AgentMailbox,
        agent_id: AgentId,
        timeout: std::time::Duration,
    },
    Interrupt {
        supervisor: AgentSupervisor,
        agent_id: AgentId,
        reason: String,
    },
    Followup {
        supervisor: AgentSupervisor,
        request: FollowupAgentRequest,
    },
}

impl PreparedHeadlessAgentControl {
    pub async fn execute(self) -> Result<serde_json::Value, String> {
        match self {
            Self::Wait {
                mailbox,
                agent_id,
                timeout,
            } => match mailbox.wait_for_agent(&agent_id, timeout).await {
                Ok(crate::agent_runtime::MailboxWaitOutcome::TimedOut) => {
                    Ok(json!({ "outcome": "timed_out", "agent_id": agent_id, "updates": [] }))
                }
                Ok(crate::agent_runtime::MailboxWaitOutcome::Updates(entries)) => Ok(json!({
                    "outcome": "updates",
                    "agent_id": agent_id,
                    "updates": entries.into_iter().map(|entry| json!({
                        "notification_event_id": entry.notification_event_id,
                        "sequence": entry.sequence,
                        "recorded_at": entry.recorded_at,
                        "notification": entry.notification,
                    })).collect::<Vec<_>>()
                })),
                Err(error) => Err(error.to_string()),
            },
            Self::Interrupt {
                supervisor,
                agent_id,
                reason,
            } => supervisor
                .interrupt(&agent_id, reason)
                .await
                .map(|agents| json!({ "outcome": "interrupted", "agent_ids": agents }))
                .map_err(|error| error.to_string()),
            Self::Followup {
                supervisor,
                request,
            } => supervisor
                .followup_agent(request)
                .await
                .map(|followup| {
                    json!({
                        "outcome": "queued",
                        "agent_id": followup.handle.agent_id,
                        "followup_turn_id": followup.followup_turn_id,
                        "turn_generation": followup.turn_generation,
                    })
                })
                .map_err(|error| error.to_string()),
        }
    }
}

pub(crate) enum PreparedAsyncSubagentControl {
    Wait {
        mailbox: crate::agent_runtime::AgentMailbox,
        timeout: std::time::Duration,
    },
    Interrupt {
        supervisor: AgentSupervisor,
        agent_id: AgentId,
        reason: String,
    },
    Followup {
        supervisor: AgentSupervisor,
        request: FollowupAgentRequest,
    },
}

impl PreparedAsyncSubagentControl {
    pub(crate) async fn execute(self) -> ToolResult {
        let result: Result<serde_json::Value, String> = match self {
            Self::Wait { mailbox, timeout } => match mailbox.wait(timeout).await {
                Ok(crate::agent_runtime::MailboxWaitOutcome::TimedOut) => {
                    Ok(json!({ "outcome": "timed_out", "updates": [] }))
                }
                Ok(crate::agent_runtime::MailboxWaitOutcome::Updates(entries)) => {
                    let updates = entries
                        .iter()
                        .map(|entry| {
                            json!({
                                "notification_event_id": entry.notification_event_id,
                                "sequence": entry.sequence,
                                "recorded_at": entry.recorded_at,
                                "notification": entry.notification,
                            })
                        })
                        .collect::<Vec<_>>();
                    Ok(json!({ "outcome": "updates", "updates": updates }))
                }
                Err(error) => Err(error.to_string()),
            },
            Self::Interrupt {
                supervisor,
                agent_id,
                reason,
            } => supervisor
                .interrupt(&agent_id, reason)
                .await
                .map(|agents| json!({ "outcome": "interrupted", "agent_ids": agents }))
                .map_err(|error| error.to_string()),
            Self::Followup {
                supervisor,
                request,
            } => supervisor
                .followup_agent(request)
                .await
                .map(|followup| {
                    json!({
                        "outcome": "queued",
                        "agent_id": followup.handle.agent_id,
                        "followup_turn_id": followup.followup_turn_id,
                        "turn_generation": followup.turn_generation,
                        "state": "queued",
                    })
                })
                .map_err(|error| error.to_string()),
        };
        match result {
            Ok(value) => ToolResult::Success(value.to_string()),
            Err(error) => ToolResult::Error(error),
        }
    }
}

fn supervisor_config(policy: &AiSubagentConfig) -> AgentSupervisorConfig {
    AgentSupervisorConfig {
        max_concurrent: policy.max_concurrent,
        max_queued: policy.max_queued,
        max_children_per_parent: policy.max_children_per_parent,
        max_total_per_run: policy.max_total_per_run,
        max_depth: policy.max_depth,
        child_budget: crate::agent_runtime::AgentLoopBudget {
            timeout: std::time::Duration::from_secs(policy.default_timeout_seconds),
            max_provider_events: policy.budgets.max_provider_events_per_agent,
            max_tool_calls: policy.budgets.max_tool_calls_per_agent,
        },
        root_max_provider_events: policy.budgets.max_total_provider_events,
        root_max_tool_calls: policy.budgets.max_total_tool_calls,
    }
}

fn subagent_config_fingerprint(config: &AiConfig) -> String {
    let mut digest = Sha256::new();
    digest.update(b"ovim-editor-subagents-v1\0");
    let mut profiles = config.profiles.values().collect::<Vec<_>>();
    profiles.sort_by(|left, right| left.name.cmp(&right.name));
    for profile in profiles {
        for value in [
            profile.name.as_str(),
            &profile.provider.to_string(),
            profile.model.as_str(),
            profile.base_url.as_deref().unwrap_or_default(),
            profile.reasoning_effort.as_deref().unwrap_or_default(),
        ] {
            digest.update(value.as_bytes());
            digest.update(b"\0");
        }
        for tool in &profile.tools {
            digest.update(tool.as_bytes());
            digest.update(b"\0");
        }
        digest.update([
            profile.scope.files as u8,
            u8::from(profile.scope.shell),
            u8::from(profile.scope.network),
        ]);
        // Detect credential-source changes without retaining them in service
        // diagnostics or exposing them to child context.
        for secret_source in [profile.api_key.as_deref(), profile.api_key_env.as_deref()] {
            if let Some(secret_source) = secret_source {
                digest.update(Sha256::digest(secret_source.as_bytes()));
            }
            digest.update(b"\0");
        }
    }
    format!("sha256:{:x}", digest.finalize())
}

#[derive(Clone)]
struct BaseManifestRegistry {
    root: PathBuf,
}

impl BaseManifestRegistry {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn register(
        &self,
        manifest_id: &ManifestId,
        manifest: &BaseManifest,
    ) -> Result<(), ManifestRegistryError> {
        ensure_private_directory(&self.root)?;
        let destination = self.path(manifest_id);
        if destination.exists() {
            let existing = self.load(manifest_id)?;
            return if &existing == manifest {
                Ok(())
            } else {
                Err(ManifestRegistryError::Conflict(manifest_id.clone()))
            };
        }
        let bytes = serde_json::to_vec(manifest)
            .map_err(|error| ManifestRegistryError::Serialization(error.to_string()))?;
        let temporary = self.root.join(format!(
            ".{}.{}.tmp",
            encoded_manifest_component(manifest_id),
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(ManifestRegistryError::Io)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &destination)?;
            let directory = OpenOptions::new().read(true).open(&self.root)?;
            directory.sync_all()?;
            Ok::<_, std::io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(ManifestRegistryError::Io)
    }

    fn load(&self, manifest_id: &ManifestId) -> Result<BaseManifest, ManifestRegistryError> {
        let bytes = fs::read(self.path(manifest_id)).map_err(ManifestRegistryError::Io)?;
        serde_json::from_slice(&bytes)
            .map_err(|error| ManifestRegistryError::Serialization(error.to_string()))
    }

    fn path(&self, manifest_id: &ManifestId) -> PathBuf {
        self.root
            .join(format!("{}.json", encoded_manifest_component(manifest_id)))
    }
}

#[derive(Clone)]
struct PreparedDelegationRegistry {
    root: PathBuf,
}

impl PreparedDelegationRegistry {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn register(
        &self,
        manifest_id: &ManifestId,
        prepared: &DurablePreparedDelegation,
    ) -> Result<(), DelegationRegistryError> {
        ensure_private_directory(&self.root).map_err(DelegationRegistryError::Manifest)?;
        let destination = self.path(manifest_id);
        if destination.exists() {
            let existing = self.load(manifest_id)?;
            return if &existing == prepared {
                Ok(())
            } else {
                Err(DelegationRegistryError::Conflict(manifest_id.clone()))
            };
        }
        let bytes = serde_json::to_vec(prepared)
            .map_err(|error| DelegationRegistryError::Serialization(error.to_string()))?;
        let temporary = self.root.join(format!(
            ".{}.{}.tmp",
            encoded_manifest_component(manifest_id),
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(DelegationRegistryError::Io)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &destination)?;
            let directory = OpenOptions::new().read(true).open(&self.root)?;
            directory.sync_all()?;
            Ok::<_, std::io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(DelegationRegistryError::Io)
    }

    fn load(
        &self,
        manifest_id: &ManifestId,
    ) -> Result<DurablePreparedDelegation, DelegationRegistryError> {
        let bytes = fs::read(self.path(manifest_id)).map_err(DelegationRegistryError::Io)?;
        serde_json::from_slice(&bytes)
            .map_err(|error| DelegationRegistryError::Serialization(error.to_string()))
    }

    fn path(&self, manifest_id: &ManifestId) -> PathBuf {
        self.root
            .join(format!("{}.json", encoded_manifest_component(manifest_id)))
    }
}

fn encoded_manifest_component(manifest_id: &ManifestId) -> String {
    let mut encoded = String::with_capacity(manifest_id.as_str().len() * 2);
    for byte in manifest_id.as_str().as_bytes() {
        use std::fmt::Write;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn ensure_private_directory(path: &Path) -> Result<(), ManifestRegistryError> {
    fs::create_dir_all(path).map_err(ManifestRegistryError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(ManifestRegistryError::Io)?;
    }
    Ok(())
}

#[derive(Debug)]
enum ManifestRegistryError {
    Io(std::io::Error),
    Serialization(String),
    Conflict(ManifestId),
}

impl std::fmt::Display for ManifestRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "manifest registry I/O: {error}"),
            Self::Serialization(error) => write!(formatter, "manifest registry JSON: {error}"),
            Self::Conflict(id) => write!(
                formatter,
                "manifest {id} was already registered differently"
            ),
        }
    }
}

impl std::error::Error for ManifestRegistryError {}

#[derive(Debug)]
enum DelegationRegistryError {
    Io(std::io::Error),
    Serialization(String),
    Conflict(ManifestId),
    Manifest(ManifestRegistryError),
}

impl std::fmt::Display for DelegationRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "delegation registry I/O: {error}"),
            Self::Serialization(error) => write!(formatter, "delegation registry JSON: {error}"),
            Self::Conflict(id) => write!(
                formatter,
                "prepared delegation for manifest {id} was already registered differently"
            ),
            Self::Manifest(error) => write!(formatter, "delegation registry directory: {error}"),
        }
    }
}

impl std::error::Error for DelegationRegistryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::fake_provider::FakeProviderAdapter;
    use crate::agent_runtime::{AgentDispatchScheduler, AgentLoopInputFactory};
    use crate::ai::chat_types::{ChatOpts, ToolCallInfo};
    use crate::ai::{AiProviderKind, PROFILE_LOCAL};
    use crate::editor::ai_state::AiState;
    use crate::run_log::{BaseManifestId, ManifestConfidence, RepositoryBase};
    use std::fs;

    fn manifest(repository_id: RepositoryId) -> BaseManifest {
        BaseManifest {
            base_manifest_id: BaseManifestId::new(),
            captured_at: "2026-07-18T10:00:00Z".into(),
            repository: RepositoryBase {
                repository_id,
                head_commit: None,
                index_tree: None,
            },
            files: Vec::new(),
            unsaved_buffers: Vec::new(),
            artifacts: Vec::new(),
            captured_bytes: 0,
            confidence: ManifestConfidence::Complete,
            issues: Vec::new(),
        }
    }

    fn enabled_editor() -> (tempfile::TempDir, tempfile::TempDir, Editor) {
        let repository = tempfile::tempdir().unwrap();
        git2::Repository::init(repository.path()).unwrap();
        let file = repository.path().join("lib.rs");
        fs::write(&file, "pub fn answer() -> u32 { 42 }\n").unwrap();
        let storage = tempfile::tempdir().unwrap();
        let layout = crate::run_log::RunStorageLayout::new(storage.path().join("runs"));
        let mut editor = Editor::default();
        editor.open_file(&file).unwrap();
        editor.ai_state = Box::new(AiState::with_run_storage_layout(layout).unwrap());
        editor.ai_state.config.subagents.enabled = true;
        let profile = editor
            .ai_state
            .config
            .profiles
            .get_mut(PROFILE_LOCAL)
            .unwrap();
        profile.provider = AiProviderKind::OpenAi;
        profile.model = "test-model".into();
        profile.reasoning_effort = Some("high".into());
        let mut service = AiSubagentService::new(&editor.ai_state.config);
        service.provider = Arc::new(FakeProviderAdapter::new("delayed_completion"));
        *editor.ai_state.subagents = service;
        editor.open_ai_chat(ChatOpts::default()).unwrap();
        (repository, storage, editor)
    }

    fn attach_root_turn(editor: &mut Editor) {
        let turn = editor.begin_ai_runtime_turn("inspect in parallel").unwrap();
        editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(turn));
    }

    fn spawn_call(task_name: &str) -> ToolCallInfo {
        ToolCallInfo {
            id: format!("tool-{task_name}"),
            name: SPAWN_AGENT_TOOL.into(),
            arguments: json!({
                "task_name": task_name,
                "objective": "Inspect the captured snapshot and cite evidence",
                "agent_kind": "explorer",
                "model": crate::agent_runtime::catalog_model_id(PROFILE_LOCAL, "test-model"),
                "reasoning_effort": "high",
                "context_mode": "brief",
                "expected_output": "analysis",
                "relevant_paths": ["lib.rs"],
                "done_when": ["Evidence is cited"],
                "non_goals": ["Do not edit"],
                "timeout_seconds": 60
            }),
        }
    }

    #[test]
    fn manifest_registry_is_durable_idempotent_and_conflict_safe() {
        let directory = tempfile::tempdir().unwrap();
        let registry = BaseManifestRegistry::new(directory.path().join("manifests"));
        let id = ManifestId::new();
        let repository = RepositoryId::new();
        let first = manifest(repository.clone());
        registry.register(&id, &first).unwrap();
        registry.register(&id, &first).unwrap();
        assert_eq!(registry.load(&id).unwrap(), first);

        let mut changed = manifest(repository);
        changed.captured_at = "later".into();
        assert!(matches!(
            registry.register(&id, &changed),
            Err(ManifestRegistryError::Conflict(conflict)) if conflict == id
        ));
    }

    #[test]
    fn config_changes_require_service_rebuild() {
        let mut config = AiConfig::default();
        config.subagents.enabled = true;
        let service = AiSubagentService::new(&config);
        assert!(service.ensure_compatible(&config).is_ok());

        config.subagents.max_concurrent = 2;
        assert!(service.ensure_compatible(&config).is_err());
    }

    #[test]
    fn service_owns_one_snapshot_stack_per_durable_root_run() {
        let directory = tempfile::tempdir().unwrap();
        git2::Repository::init(directory.path()).unwrap();
        let layout = crate::run_log::RunStorageLayout::new(directory.path().join("runs"));
        let store = Arc::new(LocalRunStore::new(layout));
        let mut config = AiConfig::default();
        config.subagents.enabled = true;
        let service = AiSubagentService::new(&config);
        let run_id = RunId::new();
        let root = AgentId::new();
        let repository = RepositoryId::new();
        let first = service
            .run(
                store.clone(),
                run_id.clone(),
                root.clone(),
                repository.clone(),
                directory.path().to_path_buf(),
            )
            .unwrap();
        let reopened = service
            .run(
                store.clone(),
                run_id.clone(),
                root,
                repository.clone(),
                directory.path().to_path_buf(),
            )
            .unwrap();
        assert!(Arc::ptr_eq(&first, &reopened));

        let manifest_id = ManifestId::new();
        first
            .register_manifest(manifest_id.clone(), manifest(repository), directory.path())
            .unwrap();
        assert_eq!(
            first.manifest(&manifest_id).unwrap().captured_at,
            "2026-07-18T10:00:00Z"
        );
        assert!(first
            .workspace_manager
            .read_only(&manifest_id)
            .unwrap()
            .is_some());

        assert!(service
            .run(
                store,
                run_id,
                AgentId::new(),
                RepositoryId::new(),
                directory.path().to_path_buf(),
            )
            .is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn parent_tools_require_feature_dispatch_capability_and_active_durable_root() {
        let (_repository, _storage, mut editor) = enabled_editor();
        assert!(!editor.ai_subagent_parent_tools_visible());
        assert!(editor.ai_subagent_parent_tools().is_empty());

        attach_root_turn(&mut editor);
        assert!(editor.ai_subagent_parent_tools_visible());
        assert!(editor
            .ai_state
            .subagents
            .parent_capabilities()
            .contains(&AgentCapability::DispatchAgents));
        let names = editor
            .ai_subagent_parent_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            names,
            BTreeSet::from([
                SPAWN_AGENT_TOOL.into(),
                LIST_AGENTS_TOOL.into(),
                WAIT_AGENT_TOOL.into(),
                INTERRUPT_AGENT_TOOL.into(),
                SEND_MESSAGE_TOOL.into(),
                FOLLOWUP_AGENT_TOOL.into(),
            ])
        );

        editor.ai_state.config.subagents.enabled = false;
        assert!(!editor.ai_subagent_parent_tools_visible());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_model_effort_pair_allocates_no_child_or_manifest() {
        let (_repository, _storage, mut editor) = enabled_editor();
        attach_root_turn(&mut editor);
        let call = ToolCallInfo {
            id: "tool-invalid-route".into(),
            name: SPAWN_AGENT_TOOL.into(),
            arguments: json!({
                "task_name": "inspect_store",
                "objective": "Inspect the store",
                "agent_kind": "explorer",
                "model": crate::agent_runtime::catalog_model_id(PROFILE_LOCAL, "test-model"),
                "reasoning_effort": "low",
                "context_mode": "brief",
                "expected_output": "analysis",
                "relevant_paths": ["ovim-core/src"],
                "done_when": ["Evidence is cited"],
                "non_goals": ["Do not edit"],
                "timeout_seconds": 60
            }),
        };
        assert!(matches!(
            editor.execute_ai_subagent_control_tool(&call),
            ToolResult::Error(_)
        ));
        assert!(editor.ai_state.subagents.runs.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wait_control_times_out_without_blocking_the_editor_task() {
        let (_repository, _storage, mut editor) = enabled_editor();
        attach_root_turn(&mut editor);
        let call = ToolCallInfo {
            id: "tool-wait".into(),
            name: WAIT_AGENT_TOOL.into(),
            arguments: json!({ "timeout_seconds": 1 }),
        };
        let prepared = editor.prepare_ai_subagent_async_control(&call).unwrap();
        let result = prepared.execute().await;
        let ToolResult::Success(payload) = result else {
            panic!("wait should complete successfully")
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&payload).unwrap()["outcome"],
            "timed_out"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn spawn_returns_immediately_list_projects_route_and_wait_delivers_handoff() {
        let (_repository, _storage, mut editor) = enabled_editor();
        attach_root_turn(&mut editor);
        let ToolResult::Success(spawned) =
            editor.execute_ai_subagent_control_tool(&spawn_call("inspect_snapshot"))
        else {
            panic!("spawn should return a durable handle")
        };
        let spawned: serde_json::Value = serde_json::from_str(&spawned).unwrap();
        assert_eq!(spawned["task_name"], "inspect_snapshot");
        assert_eq!(spawned["reasoning_effort"], "high");
        assert!(spawned["agent_id"].as_str().unwrap().starts_with("agt_"));

        let ToolResult::Success(listed) = editor.execute_ai_subagent_control_tool(&ToolCallInfo {
            id: "tool-list".into(),
            name: LIST_AGENTS_TOOL.into(),
            arguments: json!({}),
        }) else {
            panic!("list should project the child")
        };
        let listed: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed["agents"][0]["task_name"], "inspect_snapshot");
        assert_eq!(listed["agents"][0]["reasoning_effort"], "high");
        assert_eq!(listed["pending_approvals"], json!([]));
        let run = editor
            .ai_state
            .subagents
            .runs
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone();
        let dispatch = run.supervisor.dispatches().unwrap().remove(0);
        let child_tools = run
            .input_factory
            .build(&dispatch)
            .unwrap()
            .tool_view
            .names();
        for required in [
            crate::agent_runtime::SNAPSHOT_SEARCH_SYMBOLS_TOOL,
            crate::agent_runtime::SNAPSHOT_READ_DIAGNOSTICS_TOOL,
        ] {
            assert!(child_tools.iter().any(|name| name == required));
        }
        for forbidden in ["bash", "web_search", "spawn_agent", "write_file_at_path"] {
            assert!(!child_tools.iter().any(|name| name == forbidden));
        }

        let wait = ToolCallInfo {
            id: "tool-wait-handoff".into(),
            name: WAIT_AGENT_TOOL.into(),
            arguments: json!({ "timeout_seconds": 2 }),
        };
        let result = editor
            .prepare_ai_subagent_async_control(&wait)
            .unwrap()
            .execute()
            .await;
        let ToolResult::Success(payload) = result else {
            panic!("wait should receive the child handoff")
        };
        let payload_json: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload_json["outcome"], "updates");
        assert_eq!(
            payload_json["updates"][0]["notification"]["type"],
            "handoff"
        );
        assert!(run.approval_broker.pending().unwrap().is_empty());
        assert!(run.approval_broker.resolved().unwrap().is_empty());
        let snapshot = editor.ai_agent_snapshot(&run.run_id).unwrap();
        assert_eq!(
            snapshot.schema_version,
            crate::agent_runtime::AGENT_CONTROL_SNAPSHOT_VERSION
        );
        assert_eq!(snapshot.agents.len(), 1);
        assert_eq!(snapshot.agents[0].task_name, "inspect_snapshot");
        assert_eq!(snapshot.agents[0].ancestry, [run.root_agent_id.clone()]);
        let crate::run_log::AgentReported::Reported(usage) = &snapshot.agents[0].usage else {
            panic!("completed child should have durable usage")
        };
        assert_eq!(usage.provider_calls, 1);
        assert_eq!(
            usage.input_tokens,
            crate::run_log::AgentReported::NotReported
        );
        assert!(snapshot.agents[0].handoff.is_some());
        editor.consume_ai_subagent_updates(&payload).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn interrupt_targets_only_a_known_child_hierarchy() {
        let (_repository, _storage, mut editor) = enabled_editor();
        let mut service = AiSubagentService::new(&editor.ai_state.config);
        service.provider = Arc::new(
            FakeProviderAdapter::new("delayed_completion")
                .with_tick_duration(std::time::Duration::from_millis(100)),
        );
        *editor.ai_state.subagents = service;
        attach_root_turn(&mut editor);
        let ToolResult::Success(spawned) =
            editor.execute_ai_subagent_control_tool(&spawn_call("interrupt_me"))
        else {
            panic!("spawn should return a durable handle")
        };
        let spawned: serde_json::Value = serde_json::from_str(&spawned).unwrap();
        let agent_id = spawned["agent_id"].as_str().unwrap();
        let call = ToolCallInfo {
            id: "tool-interrupt".into(),
            name: INTERRUPT_AGENT_TOOL.into(),
            arguments: json!({ "agent_id": agent_id, "reason": "parent changed direction" }),
        };
        let result = editor
            .prepare_ai_subagent_async_control(&call)
            .unwrap()
            .execute()
            .await;
        let ToolResult::Success(payload) = result else {
            panic!("interrupt should succeed")
        };
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["outcome"], "interrupted");
        assert_eq!(payload["agent_ids"][0], agent_id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn send_message_queues_to_live_child_and_projects_delivery() {
        let (_repository, _storage, mut editor) = enabled_editor();
        let mut service = AiSubagentService::new(&editor.ai_state.config);
        service.provider = Arc::new(
            FakeProviderAdapter::new("restart")
                .with_tick_duration(std::time::Duration::from_millis(100)),
        );
        *editor.ai_state.subagents = service;
        attach_root_turn(&mut editor);
        let ToolResult::Success(spawned) =
            editor.execute_ai_subagent_control_tool(&spawn_call("message_me"))
        else {
            panic!("spawn should return a durable handle")
        };
        let spawned: serde_json::Value = serde_json::from_str(&spawned).unwrap();
        let agent_id = spawned["agent_id"].as_str().unwrap().to_string();
        let run = editor
            .ai_state
            .subagents
            .runs
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone();
        let agent = AgentId::parse(agent_id.clone()).unwrap();
        loop {
            if matches!(
                run.supervisor.state(&agent).unwrap(),
                Some(
                    crate::agent_runtime::DispatchState::Starting
                        | crate::agent_runtime::DispatchState::Running
                )
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
        let ToolResult::Success(queued) = editor.execute_ai_subagent_control_tool(&ToolCallInfo {
            id: "tool-message".into(),
            name: SEND_MESSAGE_TOOL.into(),
            arguments: json!({
                "agent_id": agent_id,
                "message": "Also inspect replay idempotence."
            }),
        }) else {
            panic!("message should be queued")
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&queued).unwrap()["outcome"],
            "queued"
        );
        assert!(run
            .supervisor
            .wait_for_idle(std::time::Duration::from_secs(2))
            .await
            .unwrap());
        let ToolResult::Success(listed) = editor.execute_ai_subagent_control_tool(&ToolCallInfo {
            id: "tool-list-messages".into(),
            name: LIST_AGENTS_TOOL.into(),
            arguments: json!({}),
        }) else {
            panic!("list should include message delivery")
        };
        let listed: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed["messages"][0]["state"], "delivered");
        assert_eq!(listed["messages"][0]["consumed"], true);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn followup_reopens_same_child_with_fresh_turn_and_projects_generation() {
        let (_repository, _storage, mut editor) = enabled_editor();
        attach_root_turn(&mut editor);
        let ToolResult::Success(spawned) =
            editor.execute_ai_subagent_control_tool(&spawn_call("follow_me"))
        else {
            panic!("spawn should return a durable handle")
        };
        let spawned: serde_json::Value = serde_json::from_str(&spawned).unwrap();
        let agent_id = spawned["agent_id"].as_str().unwrap().to_string();
        let run = editor
            .ai_state
            .subagents
            .runs
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone();
        assert!(run
            .supervisor
            .wait_for_idle(std::time::Duration::from_secs(2))
            .await
            .unwrap());

        let followup = ToolCallInfo {
            id: "tool-followup".into(),
            name: FOLLOWUP_AGENT_TOOL.into(),
            arguments: json!({
                "agent_id": agent_id,
                "objective": "Now verify the replay boundary."
            }),
        };
        let result = editor
            .prepare_ai_subagent_async_control(&followup)
            .unwrap()
            .execute()
            .await;
        let ToolResult::Success(payload) = result else {
            panic!("follow-up should reopen the idle child")
        };
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["agent_id"], spawned["agent_id"]);
        assert_eq!(payload["turn_generation"], 1);
        assert!(payload["followup_turn_id"]
            .as_str()
            .unwrap()
            .starts_with("trn_"));
        assert!(run
            .supervisor
            .wait_for_idle(std::time::Duration::from_secs(2))
            .await
            .unwrap());

        let ToolResult::Success(listed) = editor.execute_ai_subagent_control_tool(&ToolCallInfo {
            id: "tool-list-followup".into(),
            name: LIST_AGENTS_TOOL.into(),
            arguments: json!({}),
        }) else {
            panic!("list should project the follow-up generation")
        };
        let listed: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed["agents"][0]["agent_id"], spawned["agent_id"]);
        assert_eq!(listed["agents"][0]["turn_generation"], 1);
        assert_eq!(
            listed["agents"][0]["followup_turn_id"],
            payload["followup_turn_id"]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn followup_rejects_a_child_that_is_still_live() {
        let (_repository, _storage, mut editor) = enabled_editor();
        let mut service = AiSubagentService::new(&editor.ai_state.config);
        service.provider = Arc::new(
            FakeProviderAdapter::new("delayed_completion")
                .with_tick_duration(std::time::Duration::from_millis(100)),
        );
        *editor.ai_state.subagents = service;
        attach_root_turn(&mut editor);
        let ToolResult::Success(spawned) =
            editor.execute_ai_subagent_control_tool(&spawn_call("still_live"))
        else {
            panic!("spawn should return a durable handle")
        };
        let spawned: serde_json::Value = serde_json::from_str(&spawned).unwrap();
        let agent_id = AgentId::parse(spawned["agent_id"].as_str().unwrap()).unwrap();
        let run = editor
            .ai_state
            .subagents
            .runs
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone();
        loop {
            if matches!(
                run.supervisor.state(&agent_id).unwrap(),
                Some(
                    crate::agent_runtime::DispatchState::Starting
                        | crate::agent_runtime::DispatchState::Running
                )
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
        let followup = ToolCallInfo {
            id: "tool-followup-live".into(),
            name: FOLLOWUP_AGENT_TOOL.into(),
            arguments: json!({
                "agent_id": agent_id,
                "objective": "This must not overlap the current turn."
            }),
        };
        let result = editor
            .prepare_ai_subagent_async_control(&followup)
            .unwrap()
            .execute()
            .await;
        assert!(matches!(result, ToolResult::Error(_)));
        run.supervisor
            .interrupt(&agent_id, "test cleanup")
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resumed_editor_run_reconstructs_and_retries_a_durable_queued_child() {
        let (repository, _storage, mut editor) = enabled_editor();
        attach_root_turn(&mut editor);
        let root = editor.active_subagent_root().unwrap();
        let config = editor.ai_state.config.clone();
        let original = editor
            .ai_state
            .subagents
            .run(
                root.store.clone(),
                root.run_id.clone(),
                root.root_agent_id.clone(),
                root.repository_id.clone(),
                root.repository_root.clone(),
            )
            .unwrap();
        let manifest_id = ManifestId::new();
        let workspace_id = WorkspaceId::new();
        original
            .register_manifest(
                manifest_id.clone(),
                manifest(root.repository_id.clone()),
                repository.path(),
            )
            .unwrap();
        let budget = AgentLoopBudget {
            timeout: std::time::Duration::from_secs(2),
            max_provider_events: 32,
            max_tool_calls: 8,
        };
        original
            .register_prepared_delegation(
                manifest_id.clone(),
                DelegationEnvelope {
                    version: 1,
                    task_name: "queued_restart".into(),
                    objective: "Inspect the durable queued snapshot".into(),
                    agent_kind: DelegatedAgentKind::Explorer,
                    context_mode: DelegationContextMode::Brief,
                    expected_output: DelegationExpectedOutput::Analysis,
                    done_when: vec!["A validated handoff is recorded".into()],
                    non_goals: vec!["Do not mutate".into()],
                    relevant_paths: vec!["lib.rs".into()],
                    parent_brief: None,
                    identity: Some(Box::new(DelegationIdentity {
                        run_id: root.run_id.clone(),
                        parent_agent_id: root.root_agent_id.clone(),
                        causing_turn_id: root.turn_id.clone(),
                        causing_event_id: root.caused_by_event.clone(),
                        workspace_id: workspace_id.clone(),
                        manifest_id: manifest_id.clone(),
                    })),
                    effective_capabilities: vec!["read".into()],
                    timeout_seconds: 2,
                    workspace_warnings: Vec::new(),
                },
                budget,
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
        let sink: Arc<dyn RunEventSink> = root.store.clone();
        let mut scheduler = AgentDispatchScheduler::new(
            root.run_id.clone(),
            sink,
            editor.ai_state.subagents.catalog().unwrap(),
        );
        scheduler.set_external_parent(root.root_agent_id.clone());
        let handle = scheduler
            .dispatch(DispatchRequest {
                task_name: "queued_restart".into(),
                objective: "Inspect the durable queued snapshot".into(),
                role: AgentRoleTemplate {
                    name: AgentKindName::Explorer,
                    instructions: "Read only".into(),
                    capabilities: BTreeSet::from([AgentCapability::Read]),
                    workspace_policy: WorkspacePolicy::ReadOnlyProjection,
                    completion_contract: CompletionContract::ReviewReport,
                },
                requested_route: RequestedModelRoute::exact(
                    crate::agent_runtime::catalog_model_id(PROFILE_LOCAL, "test-model"),
                    ReasoningEffort::high(),
                ),
                parent_agent_id: Some(root.root_agent_id.clone()),
                causing_turn_id: Some(root.turn_id.clone()),
                caused_by_event: Some(root.caused_by_event.clone()),
                workspace: WorkspaceAssignment {
                    workspace_id,
                    strategy: WorkspaceStrategy::ReadOnlySnapshot {
                        manifest_id: Some(manifest_id.clone()),
                    },
                },
            })
            .unwrap();
        drop(scheduler);
        drop(original);
        drop(editor);

        let mut recovered_service = AiSubagentService::new(&config);
        recovered_service.provider = Arc::new(FakeProviderAdapter::new("delayed_completion"));
        let recovered = recovered_service
            .run(
                root.store,
                root.run_id,
                root.root_agent_id.clone(),
                root.repository_id,
                root.repository_root,
            )
            .unwrap();
        assert!(recovered
            .supervisor
            .wait_for_idle(std::time::Duration::from_secs(2))
            .await
            .unwrap());
        assert_eq!(
            recovered.supervisor.state(&handle.agent_id).unwrap(),
            Some(crate::agent_runtime::DispatchState::Completed)
        );
        let record = recovered.supervisor.dispatches().unwrap().remove(0);
        assert_eq!(record.handle.agent_id, handle.agent_id);
        assert_eq!(record.handle.workspace, handle.workspace);
        assert!(recovered.input_factory.build(&record).is_ok());
        assert_eq!(
            recovered
                .supervisor
                .mailbox(root.root_agent_id)
                .unwrap()
                .pending()
                .unwrap()
                .len(),
            1
        );
        let events = recovered.store.events(&recovered.run_id).unwrap();
        let restarted_snapshot = crate::agent_runtime::build_agent_snapshot(
            recovered.run_id.clone(),
            recovered.root_agent_id.clone(),
            recovered.supervisor.dispatches().unwrap(),
            &events,
            recovered.supervisor.messages().unwrap(),
            recovered.approval_broker.pending().unwrap(),
            recovered.approval_broker.resolved().unwrap(),
            1,
        )
        .unwrap();
        assert_eq!(restarted_snapshot.agents[0].lifecycle, "completed");
        assert!(matches!(
            restarted_snapshot.agents[0].usage,
            crate::run_log::AgentReported::Reported(_)
        ));
        assert!(restarted_snapshot.agents[0].handoff.is_some());
    }
}
