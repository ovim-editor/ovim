use crate::agent_runtime::{BranchLocator, ConversationLocator};
use crate::ai::chat_types::ConversationTree;
use crate::buffer::BufferId;
use crate::run_log::{
    apply_recovery, BranchLifecycleEvent, ConversationKey, ConversationScope, EventEnvelope,
    EventKind, LeaseStatus, MessageRole, RecoveryPlanner, RepositoryRegistration,
    RepositorySnapshot, RunEventSink,
};
use anyhow::{anyhow, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::ai_state::DurableChatBinding;
use super::Editor;

impl Editor {
    /// Establish durable catalog identity and restore history before opening
    /// the UI. Buffer ids are deliberately absent from the runtime locator.
    pub(crate) fn prepare_durable_ai_chat(
        &mut self,
        buffer_id: BufferId,
        logical_name: &str,
    ) -> Result<()> {
        let ui_key = (buffer_id, logical_name.to_owned());
        let durable_name = if logical_name.trim().is_empty() {
            "chat"
        } else {
            logical_name
        };
        let Some(services) = self.ai_state.durable_runs.as_ref() else {
            return Ok(());
        };
        if let Some(existing) = self.ai_state.durable_chat_bindings.get(&ui_key) {
            services.catalog.renew_lease(
                &existing.binding.run_id,
                &services.owner,
                services.lease_duration,
            )?;
            return Ok(());
        }

        let start = self
            .get_buffer_by_id(buffer_id)
            .and_then(|buffer| buffer.file_path().map(PathBuf::from))
            .or_else(|| self.ai_project_start_path())
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| anyhow!("no repository start path is available"))?;
        // New buffers commonly point at a file that has not been written yet.
        // libgit2 cannot discover upward from a nonexistent path, so begin at
        // its nearest existing ancestor while retaining the original path for
        // conversation scoping below.
        let repository_start = start
            .ancestors()
            .find(|candidate| candidate.exists())
            .unwrap_or(start.as_path());
        let snapshot =
            RepositorySnapshot::capture(repository_start, crate::run_log::RepositoryId::new())
                .context("AI chat requires a containing Git repository")?;
        let worktree = snapshot
            .local_paths
            .workdir
            .clone()
            .ok_or_else(|| anyhow!("bare repositories cannot host an editor chat"))?;
        let common_git_dir = snapshot
            .local_paths
            .common_git_dir
            .clone()
            .unwrap_or_else(|| snapshot.local_paths.git_dir.clone());
        let repository = services
            .catalog
            .register_repository(RepositoryRegistration {
                common_git_dir,
                worktree_aliases: vec![worktree.clone()],
            })?;
        let binding = services.catalog.open_conversation(
            ConversationKey {
                repository_id: repository.repository_id,
                scope: conversation_scope(self.get_buffer_by_id(buffer_id), &worktree)?,
                logical_name: durable_name.to_owned(),
            },
            crate::run_log::BranchId::new(),
        )?;

        let stale = matches!(
            services.catalog.lease_status(&binding.run_id)?,
            LeaseStatus::Stale(_)
        );
        services.catalog.acquire_lease(
            binding.run_id.clone(),
            services.owner.clone(),
            services.lease_duration,
        )?;
        let locator = ConversationLocator(format!("conversation:{}", binding.conversation_id));
        let prepared = (|| -> Result<Vec<EventEnvelope>> {
            if stale {
                let sink: Arc<dyn RunEventSink> = services.store.clone();
                let plan = RecoveryPlanner::new(sink).plan(&binding.run_id)?;
                apply_recovery(&plan, services.store.as_ref(), true)?;
            }
            let events = services.store.events(&binding.run_id)?;
            self.ai_state.agent_runtime.restore_conversation(
                locator.clone(),
                binding.clone(),
                events.clone(),
            )?;
            Ok(events)
        })();
        let events = match prepared {
            Ok(events) => events,
            Err(error) => {
                let _ = services
                    .catalog
                    .release_lease(&binding.run_id, &services.owner);
                return Err(error);
            }
        };
        let (conversation, nodes) = project_visible_messages(&events, &binding.selected_branch_id);
        self.ai_state
            .conversations
            .insert(ui_key.clone(), conversation);
        self.ai_state
            .conversation_runtime_nodes
            .insert(ui_key.clone(), nodes);
        self.ai_state.durable_chat_bindings.insert(
            ui_key,
            DurableChatBinding {
                binding,
                locator,
                lease_renewed_at: std::time::Instant::now(),
            },
        );
        Ok(())
    }

    pub(crate) fn durable_ai_mutations_available(&self) -> bool {
        if self.ai_state.run_storage_warning.is_some() {
            return false;
        }
        #[cfg(test)]
        if self.ai_state.durable_runs.is_none() {
            // Legacy editor unit tests intentionally use the explicit
            // in-memory runtime and are not a production authorization path.
            return true;
        }
        self.ai_state.durable_runs.is_some()
            && self
                .ai_state
                .durable_chat_bindings
                .contains_key(&self.ai_chat_conversation_key())
    }

    pub(crate) fn heartbeat_ai_chat_lease(&mut self) -> Result<()> {
        let Some(services) = self.ai_state.durable_runs.as_ref() else {
            return Ok(());
        };
        let key = self.ai_chat_conversation_key();
        let binding = self
            .ai_state
            .durable_chat_bindings
            .get(&key)
            .ok_or_else(|| anyhow!("durable chat identity is unavailable"))?;
        if binding.lease_renewed_at.elapsed() < services.lease_duration / 3 {
            return Ok(());
        }
        let run_id = binding.binding.run_id.clone();
        services
            .catalog
            .renew_lease(&run_id, &services.owner, services.lease_duration)?;
        if let Some(binding) = self.ai_state.durable_chat_bindings.get_mut(&key) {
            binding.lease_renewed_at = std::time::Instant::now();
        }
        Ok(())
    }

    /// Forget the provider-native thread while retaining ovim's durable audit log.
    /// The next turn reconstructs context exclusively from the newly cleared UI tree.
    pub(crate) fn reset_durable_ai_chat_provider_session(&mut self) -> Result<()> {
        let key = self.ai_chat_conversation_key();
        let Some(services) = self.ai_state.durable_runs.as_ref() else {
            return Ok(());
        };
        let Some(binding) = self.ai_state.durable_chat_bindings.get(&key) else {
            return Ok(());
        };
        let Some((_, branch)) = self
            .ai_state
            .agent_runtime
            .selected_branch(&binding.locator)
        else {
            return Ok(());
        };
        crate::ai::DurableCodexSession::new(
            services.catalog.clone(),
            binding.binding.root_agent_id.clone(),
            branch.branch_id.clone(),
        )
        .invalidate()?;
        Ok(())
    }
}

fn conversation_scope(
    buffer: Option<&crate::buffer::Buffer>,
    worktree: &Path,
) -> Result<ConversationScope> {
    let Some(path) = buffer.and_then(|buffer| buffer.file_path()) else {
        return Ok(ConversationScope::NoFile);
    };
    let path = crate::ai::path_policy::canonicalize_or_normalize(Path::new(path));
    let root = crate::ai::path_policy::canonicalize_or_normalize(worktree);
    let relative = path
        .strip_prefix(&root)
        .with_context(|| format!("{} is outside {}", path.display(), root.display()))?;
    if relative.as_os_str().is_empty() {
        Ok(ConversationScope::NoFile)
    } else {
        Ok(ConversationScope::RepositoryPath(relative.to_owned()))
    }
}

fn project_visible_messages(
    events: &[EventEnvelope],
    selected_branch_id: &crate::run_log::BranchId,
) -> (
    ConversationTree,
    HashMap<crate::ai::chat_types::NodeId, super::ai_state::ChatRuntimeNodeRef>,
) {
    let by_id = events
        .iter()
        .map(|event| (event.event_id.clone(), event))
        .collect::<HashMap<_, _>>();
    let labels = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::BranchLifecycle(BranchLifecycleEvent {
                branch_id,
                label: Some(label),
                ..
            }) => Some((branch_id.clone(), BranchLocator(label.clone()))),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let mut causal = HashSet::new();
    let mut cursor = events
        .iter()
        .rev()
        .find(|event| event.branch_id.as_ref() == Some(selected_branch_id));
    while let Some(event) = cursor {
        if !causal.insert(event.event_id.clone()) {
            break;
        }
        cursor = event
            .caused_by
            .as_ref()
            .and_then(|cause| by_id.get(cause).copied());
    }

    let mut conversation = ConversationTree::new();
    let mut nodes = HashMap::new();
    for event in events
        .iter()
        .filter(|event| causal.contains(&event.event_id))
    {
        if matches!(
            &event.kind,
            EventKind::Unknown { name, .. }
                if name == crate::agent_runtime::CONVERSATION_CONTEXT_RESET_EVENT
        ) {
            conversation = ConversationTree::new();
            nodes.clear();
            continue;
        }
        let EventKind::Message(message) = &event.kind else {
            continue;
        };
        let node =
            match message.role {
                MessageRole::User => conversation.append_user_message(message.content.clone()),
                MessageRole::Agent => conversation
                    .append_assistant_message(message.content.clone(), "Agent".to_owned()),
                MessageRole::ReasoningSummary => conversation
                    .append_thinking_message(message.content.clone(), "Agent".to_owned()),
                MessageRole::System => continue,
            };
        if let Some(branch) = event
            .branch_id
            .as_ref()
            .and_then(|id| labels.get(id))
            .cloned()
        {
            nodes.insert(
                node,
                super::ai_state::ChatRuntimeNodeRef {
                    event_id: event.event_id.clone(),
                    branch,
                },
            );
        }
    }
    (conversation, nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::ChatOpts;
    use crate::editor::ai_state::AiState;
    use crate::run_log::RunStorageLayout;
    use std::fs;

    fn repository_file() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        git2::Repository::init(directory.path()).unwrap();
        let file = directory.path().join("src.rs");
        fs::write(&file, "fn main() {}\n").unwrap();
        (directory, file)
    }

    fn durable_editor(file: &Path, layout: RunStorageLayout) -> Editor {
        let mut editor = Editor::default();
        editor.open_file(file).unwrap();
        editor.ai_state = Box::new(AiState::with_run_storage_layout(layout).unwrap());
        editor
    }

    #[test]
    fn new_file_inside_repository_gets_durable_chat_identity() {
        let directory = tempfile::tempdir().unwrap();
        git2::Repository::init(directory.path()).unwrap();
        let storage = tempfile::tempdir().unwrap();
        let mut editor = Editor::default();
        editor.ai_state = Box::new(
            AiState::with_run_storage_layout(RunStorageLayout::new(storage.path().join("runs")))
                .unwrap(),
        );
        let new_file = directory.path().join("nested").join("README.md");
        editor.set_file_path(new_file.to_string_lossy().into_owned());

        let buffer_id = editor.buffer().id();
        editor.prepare_durable_ai_chat(buffer_id, "chat").unwrap();

        assert!(editor
            .ai_state
            .durable_chat_bindings
            .contains_key(&(buffer_id, "chat".to_string())));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reopening_same_repository_path_restores_messages_and_identity() {
        let (_repository, file) = repository_file();
        let storage = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        let first_binding = {
            let mut editor = durable_editor(&file, layout.clone());
            editor.open_ai_chat(ChatOpts::default()).unwrap();
            let turn = editor.begin_ai_runtime_turn("inspect this").unwrap();
            editor
                .ai_state
                .agent_runtime
                .append_agent_message(&turn, "It is small.")
                .unwrap();
            editor.ai_state.agent_runtime.complete_turn(&turn).unwrap();
            editor
                .ai_state
                .durable_chat_bindings
                .get(&editor.ai_chat_conversation_key())
                .unwrap()
                .binding
                .clone()
        };

        let mut reopened = durable_editor(&file, layout);
        reopened.open_ai_chat(ChatOpts::default()).unwrap();
        let restored = reopened
            .ai_state
            .durable_chat_bindings
            .get(&reopened.ai_chat_conversation_key())
            .unwrap();
        assert_eq!(
            restored.binding.conversation_id,
            first_binding.conversation_id
        );
        assert_eq!(restored.binding.run_id, first_binding.run_id);
        let messages = reopened.conversation().unwrap().messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "inspect this");
        assert_eq!(messages[1].content, "It is small.");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn context_reset_survives_durable_reopen() {
        let (_repository, file) = repository_file();
        let storage = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        {
            let mut editor = durable_editor(&file, layout.clone());
            editor.open_ai_chat(ChatOpts::default()).unwrap();
            let turn = editor.begin_ai_runtime_turn("old question").unwrap();
            editor
                .ai_state
                .agent_runtime
                .append_agent_message(&turn, "old answer")
                .unwrap();
            editor.ai_state.agent_runtime.complete_turn(&turn).unwrap();
            editor
                .conversation_mut()
                .unwrap()
                .append_user_message("old question".into());
            editor
                .conversation_mut()
                .unwrap()
                .append_assistant_message("old answer".into(), "Agent".into());
            let chat = editor.ai_state.chat.as_mut().unwrap();
            chat.input = "/clear".into();
            chat.input_cursor = chat.input.len();
            editor.submit_ai_chat_message().unwrap();
            assert!(editor.ai_chat_messages().is_empty());
        }

        let mut reopened = durable_editor(&file, layout);
        reopened.open_ai_chat(ChatOpts::default()).unwrap();
        assert!(reopened.ai_chat_messages().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn active_work_does_not_release_its_lease_on_drop() {
        let (_repository, file) = repository_file();
        let storage = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        let mut first = durable_editor(&file, layout.clone());
        let first_buffer = first.buffer().id();
        first.prepare_durable_ai_chat(first_buffer, "chat").unwrap();
        let turn = first.begin_ai_runtime_turn("unfinished").unwrap();
        let run_id = turn.run_id;
        drop(first);

        let catalog = crate::run_log::RunCatalog::open(&layout).unwrap();
        assert!(matches!(
            catalog.lease_status(&run_id).unwrap(),
            LeaseStatus::Active(_)
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_restore_releases_the_acquired_lease() {
        let (_repository, file) = repository_file();
        let storage = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        let mut editor = durable_editor(&file, layout.clone());
        let snapshot =
            RepositorySnapshot::capture(&file, crate::run_log::RepositoryId::new()).unwrap();
        let worktree = snapshot.local_paths.workdir.unwrap();
        let services = editor.ai_state.durable_runs.as_ref().unwrap();
        let repository = services
            .catalog
            .register_repository(RepositoryRegistration {
                common_git_dir: snapshot
                    .local_paths
                    .common_git_dir
                    .unwrap_or(snapshot.local_paths.git_dir),
                worktree_aliases: vec![worktree.clone()],
            })
            .unwrap();
        let binding = services
            .catalog
            .open_conversation(
                ConversationKey {
                    repository_id: repository.repository_id,
                    scope: conversation_scope(Some(editor.buffer()), &worktree).unwrap(),
                    logical_name: "chat".into(),
                },
                crate::run_log::BranchId::new(),
            )
            .unwrap();
        services
            .store
            .append(crate::run_log::NewRunEvent::new(
                binding.run_id.clone(),
                crate::run_log::EventActor::User,
                EventKind::Unknown {
                    name: "corrupt_test_history".into(),
                    payload: serde_json::json!({}),
                },
            ))
            .unwrap();
        let run_id = binding.run_id;

        let buffer_id = editor.buffer().id();
        assert!(editor.prepare_durable_ai_chat(buffer_id, "chat").is_err());
        let catalog = crate::run_log::RunCatalog::open(&layout).unwrap();
        assert_eq!(catalog.lease_status(&run_id).unwrap(), LeaseStatus::Missing);
    }
}
