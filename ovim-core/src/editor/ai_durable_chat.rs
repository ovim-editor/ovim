use crate::agent_runtime::{BranchLocator, ConversationLocator};
use crate::ai::chat_types::ConversationTree;
use crate::buffer::BufferId;
use crate::run_log::{
    BranchLifecycleEvent, ConversationKey, ConversationScope, EventEnvelope, EventKind,
    MessageRole, RepositoryRegistration, RepositorySnapshot, RunEventSink,
};
use anyhow::{anyhow, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::ai_state::DurableChatBinding;
use super::Editor;

impl Editor {
    /// Controls whether opening a chat may restore persisted conversation
    /// history. The default is false; the binary enables it only for
    /// `ovim --resume`.
    pub fn set_ai_conversation_resume_enabled(&mut self, enabled: bool) {
        self.ai_state.resume_durable_conversations = enabled;
    }

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
        if self.ai_state.durable_chat_bindings.contains_key(&ui_key) {
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
        let conversation_key = ConversationKey {
            repository_id: repository.repository_id,
            scope: conversation_scope(self.get_buffer_by_id(buffer_id), &worktree)?,
            logical_name: durable_name.to_owned(),
        };
        let resume = self.ai_state.resume_durable_conversations;
        let mut binding = if resume {
            services
                .catalog
                .open_conversation(conversation_key, crate::run_log::BranchId::new())?
        } else {
            services
                .catalog
                .start_conversation(conversation_key, crate::run_log::BranchId::new())?
        };
        if resume && services.store.events(&binding.run_id)?.is_empty() {
            binding = services
                .catalog
                .start_conversation(binding.key.clone(), crate::run_log::BranchId::new())?;
        }
        let locator = ConversationLocator(format!("conversation:{}", binding.conversation_id));
        let prepared = (|| -> Result<Vec<EventEnvelope>> {
            let events = services.store.events(&binding.run_id)?;
            self.ai_state.agent_runtime.restore_conversation(
                locator.clone(),
                binding.clone(),
                events.clone(),
            )?;
            Ok(events)
        })();
        let (mut binding, locator, mut events, fallback_status) = match prepared {
            Ok(events) => (binding, locator, events, None),
            Err(error)
                if error
                    .downcast_ref::<crate::agent_runtime::AgentRuntimeError>()
                    .is_some_and(|error| {
                        matches!(
                            error,
                            crate::agent_runtime::AgentRuntimeError::InvalidHistory(_)
                                | crate::agent_runtime::AgentRuntimeError::UnrecoveredWork(_)
                        )
                    }) =>
            {
                // Never mutate a run that may still have a live writer. Keep
                // it intact for diagnosis and continue on an independent run.
                let may_still_be_active = error
                    .downcast_ref::<crate::agent_runtime::AgentRuntimeError>()
                    .is_some_and(|error| {
                        matches!(
                            error,
                            crate::agent_runtime::AgentRuntimeError::UnrecoveredWork(_)
                        )
                    });
                let fresh = services
                    .catalog
                    .start_conversation(binding.key.clone(), crate::run_log::BranchId::new())?;
                let fresh_locator =
                    ConversationLocator(format!("conversation:{}", fresh.conversation_id));
                self.ai_state.agent_runtime.restore_conversation(
                    fresh_locator.clone(),
                    fresh.clone(),
                    Vec::new(),
                )?;
                let status = if may_still_be_active {
                    "Previous AI run may still be active; started an independent chat"
                } else {
                    "Previous AI chat history was invalid; started a fresh chat"
                };
                (fresh, fresh_locator, Vec::new(), Some(status))
            }
            Err(error) => return Err(error),
        };

        if resume && !events.is_empty() {
            let source = self
                .ai_state
                .agent_runtime
                .selected_branch(&locator)
                .map(|(locator, _)| locator.clone())
                .ok_or_else(|| anyhow!("resumed conversation has no selected branch"))?;
            let source_tip = self
                .ai_state
                .agent_runtime
                .selected_branch_tip(&locator)
                .cloned()
                .ok_or_else(|| anyhow!("resumed conversation has no branch tip"))?;
            let target = BranchLocator(format!("resume-{}", crate::run_log::BranchId::new()));
            self.ai_state.agent_runtime.fork_branch_at(
                &locator,
                &source,
                target.clone(),
                source_tip,
            )?;
            self.ai_state
                .agent_runtime
                .select_branch(&locator, &target)?;
            let branch_id = self
                .ai_state
                .agent_runtime
                .selected_branch(&locator)
                .map(|(_, branch)| branch.branch_id.clone())
                .ok_or_else(|| anyhow!("resumed branch was not selected"))?;
            let catalog_updated = services
                .catalog
                .update_selected_branch(&binding, branch_id.clone())?;
            binding.selected_branch_id = branch_id;
            if !catalog_updated {
                crate::log_debug!(
                    "agent_runtime",
                    "conversation catalog advanced in another process; continuing independent run {}",
                    binding.run_id
                );
            }
            events = services.store.events(&binding.run_id)?;
        }
        let (conversation, nodes) = project_visible_messages(&events, &binding.selected_branch_id);
        self.ai_state
            .conversations
            .insert(ui_key.clone(), conversation);
        self.ai_state
            .conversation_runtime_nodes
            .insert(ui_key.clone(), nodes);
        self.ai_state
            .durable_chat_bindings
            .insert(ui_key, DurableChatBinding { binding, locator });
        if let Some(status) = fallback_status {
            self.set_lsp_status(status.into());
        } else if !resume {
            self.set_lsp_status(
                "Started a fresh AI conversation; launch ovim with --resume to restore history"
                    .into(),
            );
        }
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
        editor.set_ai_conversation_resume_enabled(true);
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
    async fn reopening_same_repository_path_restores_messages_on_a_new_branch() {
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
        assert_ne!(
            restored.binding.selected_branch_id,
            first_binding.selected_branch_id
        );
        let messages = reopened.conversation().unwrap().messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "inspect this");
        assert_eq!(messages[1].content, "It is small.");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn multiple_resumed_editors_continue_on_independent_branches() {
        let (_repository, file) = repository_file();
        let storage = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        {
            let mut original = durable_editor(&file, layout.clone());
            original.open_ai_chat(ChatOpts::default()).unwrap();
            let turn = original.begin_ai_runtime_turn("shared history").unwrap();
            original
                .ai_state
                .agent_runtime
                .append_agent_message(&turn, "base answer")
                .unwrap();
            original
                .ai_state
                .agent_runtime
                .complete_turn(&turn)
                .unwrap();
        }

        let mut first = durable_editor(&file, layout.clone());
        first.open_ai_chat(ChatOpts::default()).unwrap();
        let first_binding = first
            .ai_state
            .durable_chat_bindings
            .get(&first.ai_chat_conversation_key())
            .unwrap()
            .binding
            .clone();

        let mut second = durable_editor(&file, layout);
        second.open_ai_chat(ChatOpts::default()).unwrap();
        let second_binding = second
            .ai_state
            .durable_chat_bindings
            .get(&second.ai_chat_conversation_key())
            .unwrap()
            .binding
            .clone();

        assert_eq!(first_binding.run_id, second_binding.run_id);
        assert_ne!(
            first_binding.selected_branch_id,
            second_binding.selected_branch_id
        );

        let first_turn = first.begin_ai_runtime_turn("first continuation").unwrap();
        first
            .ai_state
            .agent_runtime
            .complete_turn(&first_turn)
            .unwrap();
        let second_turn = second.begin_ai_runtime_turn("second continuation").unwrap();
        second
            .ai_state
            .agent_runtime
            .complete_turn(&second_turn)
            .unwrap();
        assert_ne!(first_turn.branch_id, second_turn.branch_id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reopening_without_resume_starts_fresh_and_preserves_old_run() {
        let (_repository, file) = repository_file();
        let storage = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        let old_run = {
            let mut editor = durable_editor(&file, layout.clone());
            editor.open_ai_chat(ChatOpts::default()).unwrap();
            let turn = editor.begin_ai_runtime_turn("expensive history").unwrap();
            editor
                .ai_state
                .agent_runtime
                .append_agent_message(&turn, "large answer")
                .unwrap();
            editor.ai_state.agent_runtime.complete_turn(&turn).unwrap();
            turn.run_id
        };

        let mut reopened = durable_editor(&file, layout.clone());
        reopened.set_ai_conversation_resume_enabled(false);
        reopened.open_ai_chat(ChatOpts::default()).unwrap();
        let fresh_run = reopened
            .ai_state
            .durable_chat_bindings
            .get(&reopened.ai_chat_conversation_key())
            .unwrap()
            .binding
            .run_id
            .clone();

        assert_ne!(fresh_run, old_run);
        assert!(reopened.ai_chat_messages().is_empty());
        assert!(!reopened
            .ai_state
            .durable_runs
            .as_ref()
            .unwrap()
            .store
            .events(&old_run)
            .unwrap()
            .is_empty());
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
    async fn opening_same_chat_elsewhere_does_not_interrupt_active_run() {
        let (_repository, file) = repository_file();
        let storage = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        let mut first = durable_editor(&file, layout.clone());
        first.set_ai_conversation_resume_enabled(false);
        first.open_ai_chat(ChatOpts::default()).unwrap();
        let turn = first.begin_ai_runtime_turn("unfinished").unwrap();
        let first_run = turn.run_id.clone();

        let mut second = durable_editor(&file, layout.clone());
        second.set_ai_conversation_resume_enabled(false);
        second.open_ai_chat(ChatOpts::default()).unwrap();
        let second_run = second
            .ai_state
            .durable_chat_bindings
            .get(&second.ai_chat_conversation_key())
            .unwrap()
            .binding
            .run_id
            .clone();

        assert_ne!(first_run, second_run);
        first
            .ai_state
            .agent_runtime
            .append_agent_message(&turn, "still running")
            .unwrap();
        first.ai_state.agent_runtime.complete_turn(&turn).unwrap();
        let first_events = first
            .ai_state
            .durable_runs
            .as_ref()
            .unwrap()
            .store
            .events(&first_run)
            .unwrap();
        assert!(first_events.iter().any(|event| matches!(
            &event.kind,
            EventKind::Message(message) if message.content == "still running"
        )));

        let catalog = crate::run_log::RunCatalog::open(&layout).unwrap();
        let discovered = catalog
            .open_conversation(
                second
                    .ai_state
                    .durable_chat_bindings
                    .get(&second.ai_chat_conversation_key())
                    .unwrap()
                    .binding
                    .key
                    .clone(),
                crate::run_log::BranchId::new(),
            )
            .unwrap();
        assert_eq!(discovered.run_id, second_run);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resume_does_not_recover_or_mutate_a_possibly_active_run() {
        let (_repository, file) = repository_file();
        let storage = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        let mut first = durable_editor(&file, layout.clone());
        first.set_ai_conversation_resume_enabled(false);
        first.open_ai_chat(ChatOpts::default()).unwrap();
        let turn = first.begin_ai_runtime_turn("still working").unwrap();
        let first_run = turn.run_id.clone();
        let event_count_before = first
            .ai_state
            .durable_runs
            .as_ref()
            .unwrap()
            .store
            .events(&first_run)
            .unwrap()
            .len();

        let mut second = durable_editor(&file, layout);
        second.open_ai_chat(ChatOpts::default()).unwrap();
        let second_run = second
            .ai_state
            .durable_chat_bindings
            .get(&second.ai_chat_conversation_key())
            .unwrap()
            .binding
            .run_id
            .clone();
        assert_ne!(first_run, second_run);
        assert!(second
            .lsp_status()
            .contains("may still be active; started an independent chat"));
        assert_eq!(
            first
                .ai_state
                .durable_runs
                .as_ref()
                .unwrap()
                .store
                .events(&first_run)
                .unwrap()
                .len(),
            event_count_before
        );

        first
            .ai_state
            .agent_runtime
            .append_agent_message(&turn, "finished safely")
            .unwrap();
        first.ai_state.agent_runtime.complete_turn(&turn).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_history_starts_a_fresh_chat_and_preserves_the_broken_run() {
        let (_repository, file) = repository_file();
        let storage = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(storage.path().join("runs"));
        let mut editor = durable_editor(&file, layout.clone());
        let snapshot =
            RepositorySnapshot::capture(&file, crate::run_log::RepositoryId::new()).unwrap();
        let worktree = snapshot.local_paths.workdir.unwrap();
        let services = editor.ai_state.durable_runs.as_ref().unwrap();
        let store = services.store.clone();
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
        let broken_run_id = binding.run_id;

        let buffer_id = editor.buffer().id();
        editor.prepare_durable_ai_chat(buffer_id, "chat").unwrap();
        let fresh_binding = editor
            .ai_state
            .durable_chat_bindings
            .get(&(buffer_id, "chat".to_string()))
            .unwrap()
            .binding
            .clone();
        assert_ne!(fresh_binding.run_id, broken_run_id);
        assert_eq!(store.events(&broken_run_id).unwrap().len(), 1);
        assert!(store.events(&fresh_binding.run_id).unwrap().is_empty());
        assert!(editor.durable_ai_mutations_available());

        let catalog = crate::run_log::RunCatalog::open(&layout).unwrap();
        assert_eq!(
            catalog
                .open_conversation(fresh_binding.key.clone(), crate::run_log::BranchId::new())
                .unwrap(),
            fresh_binding
        );

        let turn = editor.begin_ai_runtime_turn("start fresh").unwrap();
        assert_eq!(turn.run_id, fresh_binding.run_id);
    }
}
