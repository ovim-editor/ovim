//! Editor-owned assembly for the read-only delegated-agent preview.
//!
//! Root chat remains in its existing orchestration path. This service owns a
//! separate per-run supervisor and immutable snapshot stack, while sharing the
//! exact durable run sink and configured provider profiles.

use crate::agent_runtime::{
    AgentCapability, AgentKindName, AgentLoopBudget, AgentRoleTemplate, AgentSupervisor,
    AgentSupervisorConfig, AgentWorkspaceManager, CompletionContract, DelegatedAgentKind,
    DelegationContextMode, DelegationEnvelope, DelegationExpectedOutput, DelegationIdentity,
    DispatchRequest, ModelFallbackPolicy, ProfileAgentProvider, ReasoningEffort,
    RequestedModelRoute, SnapshotAgentLoopInputFactory, SubagentModelCatalog, WorkspaceAssignment,
    WorkspacePolicy, WorkspaceStrategy,
};
use crate::ai::chat_types::ToolCallInfo;
use crate::ai::tools::subagents::{
    is_parent_control_tool, INTERRUPT_AGENT_TOOL, LIST_AGENTS_TOOL, SPAWN_AGENT_TOOL,
    WAIT_AGENT_TOOL,
};
use crate::ai::tools::ToolDefinition;
use crate::ai::tools::ToolResult;
use crate::ai::{AiConfig, AiSubagentConfig};
use crate::run_log::{
    AgentId, ArtifactStore, BaseManifest, BaseManifestId, EventId, EventKind, LocalRunStore,
    ManifestId, RepositoryId, RunEventSink, RunId, TurnId, WorkspaceId,
};
use serde::Deserialize;
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

pub(crate) struct AiSubagentService {
    startup_policy: AiSubagentConfig,
    config_fingerprint: String,
    catalog: Result<Arc<SubagentModelCatalog>, String>,
    provider: Arc<ProfileAgentProvider>,
    runs: Mutex<HashMap<RunId, Arc<AiSubagentRun>>>,
}

pub(crate) struct AiSubagentRun {
    pub root_agent_id: AgentId,
    pub repository_id: RepositoryId,
    pub supervisor: AgentSupervisor,
    pub artifact_store: ArtifactStore,
    pub workspace_manager: AgentWorkspaceManager,
    pub input_factory: Arc<SnapshotAgentLoopInputFactory>,
    manifest_registry: BaseManifestRegistry,
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
    ) -> Result<Arc<AiSubagentRun>, String> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| "subagent run registry is poisoned".to_string())?;
        if let Some(run) = runs.get(&run_id) {
            if run.root_agent_id != root_agent_id || run.repository_id != repository_id {
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
        if has_prior_child_history {
            return Err(
                "this resumed run contains delegated-agent history; the read-only preview preserves it but conservatively refuses to reconstruct child provider sessions after restart"
                    .into(),
            );
        }

        store
            .layout()
            .ensure_run_directory(&run_id)
            .map_err(|error| error.to_string())?;
        let artifact_store = ArtifactStore::open(store.layout().artifact_directory(&run_id))
            .map_err(|error| error.to_string())?;
        let workspace_manager = AgentWorkspaceManager::new(artifact_store.clone());
        let input_factory = Arc::new(SnapshotAgentLoopInputFactory::new(
            workspace_manager.clone(),
            self.provider.clone(),
        ));
        let sink: Arc<dyn RunEventSink> = store.clone();
        let supervisor = AgentSupervisor::new(
            run_id.clone(),
            root_agent_id.clone(),
            sink,
            self.catalog()?,
            input_factory.clone(),
            supervisor_config(&self.startup_policy),
        )
        .map_err(|error| error.to_string())?;
        let run = Arc::new(AiSubagentRun {
            root_agent_id,
            repository_id,
            supervisor,
            artifact_store,
            workspace_manager,
            input_factory,
            manifest_registry: BaseManifestRegistry::new(
                store.layout().run_directory(&run_id).join("manifests"),
            ),
        });
        runs.insert(run_id, run.clone());
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

    #[cfg(test)]
    fn manifest(&self, manifest_id: &ManifestId) -> Result<BaseManifest, String> {
        self.manifest_registry
            .load(manifest_id)
            .map_err(|error| error.to_string())
    }
}

impl Editor {
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
        if !matches!(call.name.as_str(), WAIT_AGENT_TOOL | INTERRUPT_AGENT_TOOL) {
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
            super::ai_chat_state::SubagentControlContinuation,
        ),
    > {
        if self.ai_state.chat.is_none() {
            return Err((
                ToolResult::Error("no active chat session".into()),
                continuation,
            ));
        }
        let prepared = match self.prepare_ai_subagent_async_control(&call) {
            Ok(prepared) => prepared,
            Err(error) => return Err((ToolResult::Error(error), continuation)),
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
        run.input_factory.register_prepared(
            manifest_id.clone(),
            envelope,
            Some(AgentLoopBudget {
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
            }),
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

    fn list_ai_subagents(&self) -> Result<serde_json::Value, String> {
        let root = self.active_subagent_root()?;
        let run = self.ai_state.subagents.run(
            root.store,
            root.run_id,
            root.root_agent_id.clone(),
            root.repository_id,
        )?;
        let pending = run
            .supervisor
            .mailbox(root.root_agent_id)
            .map_err(|error| error.to_string())?
            .pending()
            .map_err(|error| error.to_string())?;
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
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "agents": records,
            "pending_attention": pending.len(),
        }))
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

#[cfg(test)]
mod tests {
    use super::*;
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
        editor.ai_state.subagents = Box::new(AiSubagentService::new(&editor.ai_state.config));
        editor.open_ai_chat(ChatOpts::default()).unwrap();
        (repository, storage, editor)
    }

    fn attach_root_turn(editor: &mut Editor) {
        let turn = editor.begin_ai_runtime_turn("inspect in parallel").unwrap();
        editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(turn));
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
            )
            .unwrap();
        let reopened = service
            .run(store.clone(), run_id.clone(), root, repository.clone())
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
            .run(store, run_id, AgentId::new(), RepositoryId::new())
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
}
