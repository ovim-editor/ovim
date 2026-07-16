use crate::ai::chat_types::{ChatOpts, ConversationTree};
use crate::buffer::BufferId;
use crate::mode::Mode;
use anyhow::Result;

use super::ai_chat_state::AiChatState;
use super::Editor;

impl Editor {
    // -----------------------------------------------------------------
    // Open / Close
    // -----------------------------------------------------------------

    /// Open or resume an AI chat panel.
    pub fn open_ai_chat(&mut self, opts: ChatOpts) -> Result<()> {
        if self
            .ai_state
            .chat
            .as_ref()
            .is_some_and(|chat| chat.opts.name == opts.name)
        {
            let mode_before = self.mode();
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.mode_before_chat = mode_before;
            }
            self.set_mode(Mode::AiChat);
            self.maybe_prompt_exa_on_chat_open();
            return Ok(());
        }

        // Switching to another named conversation replaces the live panel.
        // Its projected message history remains stored under its own key.
        if self.ai_state.chat.is_some() {
            self.discard_active_ai_chat("chat replaced");
        }
        let buffer_id = self.buffer().id();
        let mode_before = self.mode();

        if let Err(error) = self.prepare_durable_ai_chat(buffer_id, &opts.name) {
            self.set_lsp_status(format!(
                "Durable AI history unavailable; agent edits are disabled: {error}"
            ));
        }

        // Ensure conversation exists
        let key = (buffer_id, opts.name.clone());
        self.ai_state.conversations.entry(key.clone()).or_default();

        // Send initial message if provided and conversation is empty
        let initial = opts.initial_message.clone();
        let buffer_clean = !self.buffer().is_modified();
        let branch_generation = self
            .ai_state
            .conversations
            .get(&key)
            .map(ConversationTree::branch_generation)
            .unwrap_or_default();
        let mut chat = AiChatState::new(opts, buffer_id, mode_before);
        let runtime_locator = self
            .ai_state
            .durable_chat_bindings
            .get(&key)
            .map(|binding| binding.locator.clone())
            .unwrap_or_else(|| {
                crate::agent_runtime::ConversationLocator(format!(
                    "buffer:{buffer_id}:conversation:{}",
                    chat.opts.name
                ))
            });
        chat.runtime_branch = self
            .ai_state
            .agent_runtime
            .selected_branch(&runtime_locator)
            .map(|(locator, _)| locator.clone())
            .unwrap_or_else(|| {
                crate::agent_runtime::BranchLocator(format!("branch-{branch_generation}"))
            });
        chat.buffer_was_clean_at_chat_start = buffer_clean;
        self.ai_state.chat = Some(chat);
        self.set_mode(Mode::AiChat);
        self.maybe_prompt_no_repo_session_folder_access_on_chat_open();
        self.maybe_prompt_exa_on_chat_open();

        if let Some(msg) = initial {
            if let Some(conv) = self.conversation() {
                if conv.is_empty() && !msg.is_empty() {
                    // Will be handled as if user typed and submitted
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        chat.input = msg;
                        chat.input_cursor = chat.input.len();
                    }
                }
            }
        }

        Ok(())
    }

    /// Hide the AI chat panel without clearing or interrupting it.
    pub fn close_ai_chat(&mut self) {
        self.render_cache.ai_chat_text_selection = None;
        self.render_cache.ai_chat_text_selecting = false;
        let mode_before = self
            .ai_state
            .chat
            .as_ref()
            .map(|chat| chat.mode_before_chat);
        if let Some(mode) = mode_before {
            self.set_mode(mode);
        }
    }

    /// Stop the active AI round without hiding the panel or discarding chat state.
    pub fn cancel_ai_chat_generation(&mut self) -> bool {
        if !self.ai_chat_has_pending_work() {
            return false;
        }

        let model_name = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.pending_job.as_ref())
            .map(|job| job.model_name.clone())
            .unwrap_or_default();
        let had_agent_work = self.ai_state.chat.as_ref().is_some_and(|chat| {
            chat.waiting
                || chat.pending_job.is_some()
                || chat.pending_tool_approval.is_some()
                || chat.pending_auto_mode_classification.is_some()
                || chat.pending_shell_execution.is_some()
                || chat.pending_web_execution.is_some()
                || chat.pending_code_explanation.is_some()
        });

        self.flush_ai_runtime_stream_segments();
        self.commit_partial_streaming(&model_name);

        let (
            pending_job,
            pending_approval,
            pending_classification,
            pending_shell,
            pending_web,
            pending_explanation,
        ) = {
            let chat = self.ai_state.chat.as_mut().expect("pending chat exists");
            (
                chat.pending_job.take(),
                chat.pending_tool_approval.take(),
                chat.pending_auto_mode_classification.take(),
                chat.pending_shell_execution.take(),
                chat.pending_web_execution.take(),
                chat.pending_code_explanation.take(),
            )
        };

        if let Some(job) = pending_job {
            job.task.abort();
        }
        if let Some(pending) = pending_approval {
            if let (Some(turn), Some(tool)) =
                (pending.dynamic_turn.as_ref(), pending.runtime_tool.as_ref())
            {
                if let Err(error) =
                    self.ai_state
                        .agent_runtime
                        .fail_tool(turn, tool, "cancelled by user")
                {
                    crate::log_warn!("agent_runtime", "failed to cancel pending tool: {error}");
                }
            } else if let (Some(turn), Some(tool)) =
                (self.active_ai_runtime_turn(), pending.runtime_tool.as_ref())
            {
                if let Err(error) =
                    self.ai_state
                        .agent_runtime
                        .fail_tool(&turn, tool, "cancelled by user")
                {
                    crate::log_warn!("agent_runtime", "failed to cancel pending tool: {error}");
                }
            }
            if let Some(response) = pending.dynamic_response {
                let _ = response.send(Err("cancelled by user".into()));
            }
        }
        if let Some(pending) = pending_classification {
            if let Err(error) = self.ai_state.agent_runtime.fail_tool(
                &pending.runtime_turn,
                &pending.runtime_tool,
                "cancelled by user",
            ) {
                crate::log_warn!("agent_runtime", "failed to cancel classified tool: {error}");
            }
            let _ = pending
                .dynamic_response
                .send(Err("cancelled by user".into()));
        }
        if let Some(pending) = pending_shell {
            pending.task.abort();
            if let Err(error) = self.ai_state.agent_runtime.mark_tool_outcome_unknown(
                &pending.runtime_turn,
                &pending.runtime_tool,
                "cancelled by user before the shell result was observed",
            ) {
                crate::log_warn!("agent_runtime", "failed to cancel shell tool: {error}");
            }
            let _ = pending
                .dynamic_response
                .send(Err("cancelled by user".into()));
        }
        if let Some(pending) = pending_web {
            pending.task.abort();
            if let (Some(turn), Some(tool)) =
                (pending.runtime_turn.as_ref(), pending.runtime_tool.as_ref())
            {
                if let Err(error) =
                    self.ai_state
                        .agent_runtime
                        .fail_tool(turn, tool, "cancelled by user")
                {
                    crate::log_warn!("agent_runtime", "failed to cancel web tool: {error}");
                }
            }
        }
        if let Some(pending) = pending_explanation {
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.active_buffer_id = pending.original_active_buffer_id;
            }
            self.ai_state.active_selection = None;
            match pending.continuation {
                super::ai_chat_state::CodeExplanationContinuation::Dynamic {
                    runtime_tool,
                    runtime_turn,
                    response,
                } => {
                    if let Err(error) = self.ai_state.agent_runtime.fail_tool(
                        &runtime_turn,
                        &runtime_tool,
                        "cancelled by user",
                    ) {
                        crate::log_warn!(
                            "agent_runtime",
                            "failed to cancel code walkthrough: {error}"
                        );
                    }
                    let _ = response.send(Err("cancelled by user".into()));
                }
                super::ai_chat_state::CodeExplanationContinuation::Batch {
                    runtime_tool,
                    runtime_turn,
                    ..
                } => {
                    if let (Some(turn), Some(tool)) = (runtime_turn.as_ref(), runtime_tool.as_ref())
                    {
                        if let Err(error) =
                            self.ai_state
                                .agent_runtime
                                .fail_tool(turn, tool, "cancelled by user")
                        {
                            crate::log_warn!(
                                "agent_runtime",
                                "failed to cancel code walkthrough: {error}"
                            );
                        }
                    }
                }
                super::ai_chat_state::CodeExplanationContinuation::Replay => {}
            }
        }

        self.ai_runtime_interrupt_turn("cancelled by user");
        self.clear_streaming_state();
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.pending_no_repo_folder_approval = None;
        }
        if had_agent_work {
            if let Some(conv) = self.conversation_mut() {
                conv.append_error("Generation stopped by user.".to_string());
            }
            self.set_lsp_status("AI generation stopped".to_string());
        } else {
            self.set_lsp_status("AI folder access prompt cancelled".to_string());
        }
        true
    }

    /// Permanently discard the live panel state when replacing conversations.
    fn discard_active_ai_chat(&mut self, reason: &str) {
        if let Some(job) = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.pending_job.as_ref())
        {
            job.task.abort();
        }
        if let Some(web) = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.pending_web_execution.as_ref())
        {
            web.task.abort();
        }
        self.ai_runtime_interrupt_turn(reason);
        if let Some(mut chat) = self.ai_state.chat.take() {
            chat.pending_job.take();
        }
    }

    // -----------------------------------------------------------------
    // Context profile
    // -----------------------------------------------------------------

    pub fn ai_chat_context_profile(&self, context: &str) -> Option<String> {
        // Look up in contexts table first
        if let Some(profile) = self.ai_state.config.contexts.get(context) {
            if self.ai_state.config.profiles.contains_key(profile) {
                return Some(profile.clone());
            }
        }
        // Fallback to active profile
        Some(self.ai_state.active_profile.clone())
    }

    /// Effective profile currently used by the active chat session.
    pub fn ai_chat_effective_profile(&self) -> String {
        self.ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.opts.profile.clone())
            .unwrap_or_else(|| self.ai_state.active_profile.clone())
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    pub(crate) fn ai_chat_conversation_key(&self) -> (BufferId, String) {
        if let Some(chat) = &self.ai_state.chat {
            (chat.origin_buffer_id, chat.opts.name.clone())
        } else {
            (self.buffer().id(), "chat".to_string())
        }
    }

    /// Shorthand for getting the current conversation (read-only).
    pub fn conversation(&self) -> Option<&ConversationTree> {
        let key = self.ai_chat_conversation_key();
        self.ai_state.conversations.get(&key)
    }

    /// Shorthand for getting the current conversation (mutable).
    pub(crate) fn conversation_mut(&mut self) -> Option<&mut ConversationTree> {
        let key = self.ai_chat_conversation_key();
        self.ai_state.conversations.get_mut(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::{ChatOpts, ChatRole, NodeId, StreamChunk, ToolCallInfo};
    use crate::buffer::Buffer;
    use crate::run_log::{EventKind, TurnLifecycleState};

    fn open_test_chat(editor: &mut Editor) {
        editor
            .open_ai_chat(ChatOpts {
                name: "chat".to_string(),
                allow_edits: true,
                ..Default::default()
            })
            .expect("open chat");
    }

    fn attach_pending_runtime_job(
        editor: &mut Editor,
        turn: crate::agent_runtime::PendingTurnRef,
        branch_generation: u64,
    ) -> tokio::task::AbortHandle {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async { std::future::pending::<()>().await });
        let abort_handle = task.abort_handle();
        let chat = editor.ai_state.chat.as_mut().expect("chat");
        chat.runtime_turn = Some(Box::new(turn.clone()));
        chat.pending_job = Some(super::super::ai_chat_state::PendingAiChatJob {
            receiver: rx,
            task,
            profile_name: "test".into(),
            model_name: "test".into(),
            turn: Box::new(turn),
            branch_generation,
            steer_tx: None,
        });
        chat.waiting = true;
        abort_handle
    }

    #[tokio::test(flavor = "current_thread")]
    async fn closing_chat_keeps_provider_and_live_state_running() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("inspect").unwrap();
        let run_id = turn.run_id.clone();
        let abort_handle = attach_pending_runtime_job(&mut editor, turn, 0);
        editor.ai_state.chat.as_mut().unwrap().input = "follow up".into();

        editor.close_ai_chat();
        tokio::task::yield_now().await;

        assert!(!abort_handle.is_finished());
        assert_ne!(editor.mode(), Mode::AiChat);
        let chat = editor.ai_state.chat.as_ref().expect("hidden chat retained");
        assert_eq!(chat.input, "follow up");
        assert!(chat.pending_job.is_some());
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(!matches!(
            &events.last().unwrap().kind,
            EventKind::TurnLifecycle(event) if event.state == TurnLifecycleState::Interrupted
        ));

        editor.discard_active_ai_chat("test cleanup");
    }

    #[test]
    fn activity_tracks_runtime_ownership_even_without_waiting_flag() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        assert_eq!(
            editor.ai_chat_activity(),
            crate::editor::AiChatActivity::Idle
        );

        let turn = editor.begin_ai_runtime_turn("inspect").unwrap();
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.runtime_turn = Some(Box::new(turn));
        chat.waiting = false;

        assert_eq!(
            editor.ai_chat_activity(),
            crate::editor::AiChatActivity::Inference
        );
        assert!(editor.ai_chat_has_pending_work());
        editor.ai_runtime_interrupt_turn("test cleanup");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelling_generation_stops_provider_but_preserves_chat() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("inspect").unwrap();
        let run_id = turn.run_id.clone();
        let abort_handle = attach_pending_runtime_job(&mut editor, turn, 0);
        {
            let chat = editor.ai_state.chat.as_mut().unwrap();
            chat.input = "keep this draft".into();
            chat.streaming_content = Some("Partial answer".into());
        }

        assert!(editor.cancel_ai_chat_generation());
        tokio::task::yield_now().await;

        assert!(abort_handle.is_finished());
        assert_eq!(editor.mode(), Mode::AiChat);
        assert_eq!(editor.ai_chat_input(), "keep this draft");
        assert!(!editor.ai_chat_waiting());
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|message| message.content == "Partial answer"));
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|message| message.content == "Generation stopped by user."));
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(matches!(
            &events.last().unwrap().kind,
            EventKind::TurnLifecycle(event)
                if event.state == TurnLifecycleState::Interrupted
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submitting_new_turn_clears_stale_chat_notice() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        editor.set_lsp_status("Queued message moved back to the composer".into());
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.input = "new request".into();
        chat.input_cursor = chat.input.len();

        editor.submit_ai_chat_message().unwrap();

        assert_eq!(editor.lsp_status(), "");
        editor.cancel_ai_chat_generation();
    }

    #[test]
    fn effective_profile_controls_optional_tool_call_limit() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        assert_eq!(editor.ai_chat_tool_call_limit(), None);

        editor
            .ai_state
            .config
            .profiles
            .get_mut(crate::ai::PROFILE_LOCAL)
            .unwrap()
            .agent_loop
            .max_tool_calls = Some(75);

        // ChatOpts has no explicit profile, so this also proves the active
        // profile is used rather than a hard-coded fallback.
        assert_eq!(editor.ai_chat_tool_call_limit(), Some(75));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reopening_chat_resumes_live_state_and_preserves_underlying_mode() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("inspect").unwrap();
        let abort_handle = attach_pending_runtime_job(&mut editor, turn, 0);
        editor.ai_state.chat.as_mut().unwrap().input = "still here".into();

        editor.close_ai_chat();
        assert_ne!(editor.mode(), Mode::AiChat);

        open_test_chat(&mut editor);
        tokio::task::yield_now().await;

        assert!(!abort_handle.is_finished());
        assert_eq!(editor.mode(), Mode::AiChat);
        assert_eq!(editor.ai_chat_input(), "still here");
        editor.close_ai_chat();
        assert_ne!(editor.mode(), Mode::AiChat);

        editor.discard_active_ai_chat("test cleanup");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_provider_branch_is_aborted_before_output_is_applied() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("inspect").unwrap();
        let run_id = turn.run_id.clone();
        let abort_handle = attach_pending_runtime_job(&mut editor, turn, 0);
        {
            let conv = editor.conversation_mut().unwrap();
            conv.append_user_message("root".into());
            let root = conv.active_leaf_id().unwrap();
            conv.append_assistant_message("old branch".into(), "test".into());
            conv.fork_from(root);
        }

        assert!(editor.poll_pending_ai_chat_job());
        tokio::task::yield_now().await;

        assert!(abort_handle.is_finished());
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(matches!(
            &events.last().unwrap().kind,
            EventKind::TurnLifecycle(event)
                if event.state == TurnLifecycleState::Interrupted
        ));
        assert!(editor
            .conversation()
            .unwrap()
            .messages()
            .iter()
            .any(|message| message.content.contains("Discarded stale response")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn dynamic_tool_events_are_terminal_before_codex_receives_result() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("check diagnostics").unwrap();
        let run_id = turn.run_id.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async { std::future::pending::<()>().await });
        let abort_handle = task.abort_handle();
        editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(turn.clone()));
        editor.ai_state.chat.as_mut().unwrap().streaming_content = Some(String::new());
        editor.ai_state.chat.as_mut().unwrap().pending_job =
            Some(super::super::ai_chat_state::PendingAiChatJob {
                receiver: rx,
                task,
                profile_name: "test".into(),
                model_name: "test".into(),
                turn: Box::new(turn),
                branch_generation: 0,
                steer_tx: None,
            });

        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        tx.send(StreamChunk::Content("Before tool. ".into()))
            .unwrap();
        tx.send(StreamChunk::DynamicToolRequest {
            call: ToolCallInfo {
                id: "provider-call-1".into(),
                name: "read_diagnostics".into(),
                arguments: serde_json::json!({}),
            },
            response: result_tx,
        })
        .unwrap();

        assert!(editor.poll_pending_ai_chat_job());
        let events_before_provider_result = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(matches!(
            &events_before_provider_result.last().unwrap().kind,
            EventKind::ToolResult(_)
        ));
        let pre_tool_message = events_before_provider_result
            .iter()
            .position(|event| {
                matches!(
                    &event.kind,
                    EventKind::Message(crate::run_log::MessageEvent {
                        role: crate::run_log::MessageRole::Agent,
                        content,
                    }) if content == "Before tool. "
                )
            })
            .unwrap();
        let tool_intent = events_before_provider_result
            .iter()
            .position(|event| matches!(event.kind, EventKind::ToolIntent(_)))
            .unwrap();
        assert!(pre_tool_message < tool_intent);
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|message| message.role == ChatRole::Tool
                && message.tool_call_id.as_deref() == Some("provider-call-1")));
        let _provider_result = result_rx.await.unwrap();

        tx.send(StreamChunk::Content("After tool.".into())).unwrap();
        tx.send(StreamChunk::Done).unwrap();
        assert!(editor.poll_pending_ai_chat_job());
        abort_handle.abort();

        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(matches!(
            &events.last().unwrap().kind,
            EventKind::TurnLifecycle(event)
                if event.state == TurnLifecycleState::Completed
        ));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ToolResult(_)))
                .count(),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn dynamic_code_walkthrough_blocks_provider_until_user_finishes() {
        let repo = tempfile::tempdir().unwrap();
        git2::Repository::init(repo.path()).unwrap();
        let file = repo.path().join("main.rs");
        std::fs::write(&file, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();

        let mut editor = Editor::default();
        editor.open_file(&file).unwrap();
        open_test_chat(&mut editor);
        let profile = editor.ai_state.active_profile.clone();
        editor
            .ai_state
            .config
            .profiles
            .get_mut(&profile)
            .unwrap()
            .scope
            .files = crate::ai::FileScope::Project;
        editor.set_viewport_height(24);

        let turn = editor
            .begin_ai_runtime_turn("explain the entry point")
            .unwrap();
        let run_id = turn.run_id.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async { std::future::pending::<()>().await });
        let abort_handle = task.abort_handle();
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.runtime_turn = Some(Box::new(turn.clone()));
        chat.pending_job = Some(super::super::ai_chat_state::PendingAiChatJob {
            receiver: rx,
            task,
            profile_name: "test".into(),
            model_name: "test".into(),
            turn: Box::new(turn),
            branch_generation: 0,
            steer_tx: None,
        });

        let (response_tx, mut response_rx) = tokio::sync::oneshot::channel();
        tx.send(StreamChunk::DynamicToolRequest {
            call: ToolCallInfo {
                id: "walkthrough-call".into(),
                name: "explain_with_codebase".into(),
                arguments: serde_json::json!({
                    "steps": [{
                        "path": "main.rs",
                        "start_line": 1,
                        "end_line": 3,
                        "comment": "This is the executable entry point."
                    }]
                }),
            },
            response: response_tx,
        })
        .unwrap();

        assert!(editor.poll_pending_ai_chat_job());
        assert_eq!(
            editor.ai_chat_activity(),
            crate::editor::AiChatActivity::WaitingCodeExplanation
        );
        assert!(editor.ai_chat_has_pending_code_explanation());
        assert!(matches!(
            response_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(matches!(
            events.last().unwrap().kind,
            EventKind::ToolStarted(_)
        ));

        assert!(editor.finish_code_explanation(false));
        let provider_result = response_rx.await.unwrap().unwrap();
        assert!(provider_result.contains("completed the code walkthrough"));
        assert!(!editor.ai_chat_has_pending_code_explanation());
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(matches!(
            events.last().unwrap().kind,
            EventKind::ToolResult(_)
        ));

        abort_handle.abort();
        editor.discard_active_ai_chat("test cleanup");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn auto_mode_unauthorized_deploy_is_sent_to_terra_before_user_escalation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let file = dir.path().join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let mut editor = Editor::default();
        editor.open_file(&file).unwrap();
        open_test_chat(&mut editor);
        editor.ai_state.config.tool_approval_mode = crate::ai::ToolApprovalMode::Auto;
        let turn = editor.begin_ai_runtime_turn("inspect the project").unwrap();
        let run_id = turn.run_id.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async { std::future::pending::<()>().await });
        let abort_handle = task.abort_handle();
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.runtime_turn = Some(Box::new(turn.clone()));
        chat.streaming_content = Some(String::new());
        chat.pending_job = Some(super::super::ai_chat_state::PendingAiChatJob {
            receiver: rx,
            task,
            profile_name: "test".into(),
            model_name: "test".into(),
            turn: Box::new(turn),
            branch_generation: 0,
            steer_tx: None,
        });

        let (result_tx, mut result_rx) = tokio::sync::oneshot::channel();
        tx.send(StreamChunk::DynamicToolRequest {
            call: ToolCallInfo {
                id: "deploy-call".into(),
                name: "bash".into(),
                arguments: serde_json::json!({"command": "./deploy production"}),
            },
            response: result_tx,
        })
        .unwrap();

        assert!(editor.poll_pending_ai_chat_job());
        assert!(!editor.ai_chat_has_pending_tool_approval());
        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .pending_auto_mode_classification
            .is_some());
        assert!(matches!(
            result_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ToolIntent(_))));
        assert!(!events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ToolStarted(_))));

        // The classifier task may still be connecting to app-server. Dropping
        // its receiver is sufficient here; this test covers routing, while
        // verdict handling is exercised by the focused classifier tests.
        editor
            .ai_state
            .chat
            .as_mut()
            .unwrap()
            .pending_auto_mode_classification = None;
        abort_handle.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn dynamic_path_tool_uses_the_same_paused_approval_flow() {
        let repo = tempfile::tempdir().unwrap();
        git2::Repository::init(repo.path()).unwrap();
        let file = repo.path().join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let external = tempfile::tempdir().unwrap();
        let external_file = external.path().join("outside.txt");
        std::fs::write(&external_file, "outside\n").unwrap();

        let mut editor = Editor::default();
        editor.open_file(&file).unwrap();
        open_test_chat(&mut editor);
        let turn = editor
            .begin_ai_runtime_turn("inspect outside file")
            .unwrap();
        let run_id = turn.run_id.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async { std::future::pending::<()>().await });
        let abort_handle = task.abort_handle();
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.runtime_turn = Some(Box::new(turn.clone()));
        chat.pending_job = Some(super::super::ai_chat_state::PendingAiChatJob {
            receiver: rx,
            task,
            profile_name: "test".into(),
            model_name: "test".into(),
            turn: Box::new(turn),
            branch_generation: 0,
            steer_tx: None,
        });

        let (response_tx, mut response_rx) = tokio::sync::oneshot::channel();
        tx.send(StreamChunk::DynamicToolRequest {
            call: ToolCallInfo {
                id: "outside-read".into(),
                name: "read_file_at_path".into(),
                arguments: serde_json::json!({"path": external_file}),
            },
            response: response_tx,
        })
        .unwrap();

        assert!(editor.poll_pending_ai_chat_job());
        assert_eq!(
            editor.ai_chat_activity(),
            crate::editor::AiChatActivity::WaitingToolApproval
        );
        assert!(matches!(
            response_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        assert!(editor.ai_chat_resolve_pending_tool_approval(false, false));
        assert!(response_rx.await.unwrap().is_err());

        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ToolStarted(_)))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ToolResult(_)))
                .count(),
            1
        );
        abort_handle.abort();
        editor.discard_active_ai_chat("test cleanup");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn completed_agent_items_become_separate_chat_messages() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("work in stages").unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async { std::future::pending::<()>().await });
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.runtime_turn = Some(Box::new(turn.clone()));
        chat.streaming_content = Some(String::new());
        chat.waiting = true;
        chat.pending_job = Some(super::super::ai_chat_state::PendingAiChatJob {
            receiver: rx,
            task,
            profile_name: "test".into(),
            model_name: "test".into(),
            turn: Box::new(turn),
            branch_generation: 0,
            steer_tx: None,
        });

        tx.send(StreamChunk::Thinking("Inspecting first.".into()))
            .unwrap();
        tx.send(StreamChunk::Content("I found the cause.".into()))
            .unwrap();
        tx.send(StreamChunk::AgentMessageComplete).unwrap();
        tx.send(StreamChunk::Content("The fix is verified.".into()))
            .unwrap();
        tx.send(StreamChunk::AgentMessageComplete).unwrap();
        tx.send(StreamChunk::Done).unwrap();

        assert!(editor.poll_pending_ai_chat_job());
        let messages = editor.conversation().unwrap().messages();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, ChatRole::Thinking);
        assert_eq!(messages[0].content, "Inspecting first.");
        assert_eq!(messages[1].role, ChatRole::Assistant);
        assert_eq!(messages[1].content, "I found the cause.");
        assert_eq!(messages[2].role, ChatRole::Assistant);
        assert_eq!(messages[2].content, "The fix is verified.");
        assert!(!editor.ai_chat_waiting());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_provider_job_is_detached_before_async_tool_work() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("search the web").unwrap();
        attach_pending_runtime_job(&mut editor, turn, 0);

        editor.finish_provider_stream_before_tools();

        assert!(editor.ai_state.chat.as_ref().unwrap().pending_job.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn lost_durable_lease_blocks_approved_shell_effect() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git2::Repository::init(&repo).unwrap();
        let file = repo.join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let runs = crate::run_log::RunStorageLayout::new(dir.path().join("runs"));
        let mut editor = Editor::default();
        editor.ai_state =
            Box::new(super::super::ai_state::AiState::with_run_storage_layout(runs).unwrap());
        editor.open_file(&file).unwrap();
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("create the marker").unwrap();
        let call = ToolCallInfo {
            id: "write-marker".into(),
            name: "bash".into(),
            arguments: serde_json::json!({"command": "touch effect-marker"}),
        };
        let tool = editor.ai_runtime_record_tool_intent(&turn, &call).unwrap();
        let key = editor.ai_chat_conversation_key();
        let binding = editor.ai_state.durable_chat_bindings.get(&key).unwrap();
        let run_id = binding.binding.run_id.clone();
        let services = editor.ai_state.durable_runs.as_ref().unwrap();
        let owner = services.owner.clone();
        services.catalog.release_lease(&run_id, &owner).unwrap();
        editor
            .ai_state
            .durable_chat_bindings
            .get_mut(&key)
            .unwrap()
            .lease_renewed_at = std::time::Instant::now() - std::time::Duration::from_secs(60);

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        editor.execute_dynamic_tool_after_policy(turn, tool, call, response_tx, None, false);

        assert!(response_rx.await.unwrap().is_err());
        assert!(!repo.join("effect-marker").exists());
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        assert!(!events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ToolStarted(_))));
        assert!(events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ToolResult(_))));
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::TurnLifecycle(lifecycle)
                if lifecycle.state == TurnLifecycleState::Failed
        )));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authorized_shell_runs_in_background_while_editor_poll_stays_responsive() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let file = dir.path().join("main.rs");
        std::fs::write(&file, "fn main() {}\n// second line\n").unwrap();
        let runs = tempfile::tempdir().unwrap();
        let mut editor = Editor::default();
        editor.ai_state = Box::new(
            super::super::ai_state::AiState::with_run_storage_layout(
                crate::run_log::RunStorageLayout::new(runs.path()),
            )
            .unwrap(),
        );
        editor.open_file(&file).unwrap();
        editor.set_mode(crate::mode::Mode::Normal);
        open_test_chat(&mut editor);
        let turn = editor.begin_ai_runtime_turn("run the gated check").unwrap();
        let run_id = turn.run_id.clone();
        let call = ToolCallInfo {
            id: "gated-shell".into(),
            name: "bash".into(),
            arguments: serde_json::json!({
                "command": "while [ ! -f release-gate ]; do sleep 0.01; done; touch agent-marker; echo completed"
            }),
        };
        let tool = editor.ai_runtime_record_tool_intent(&turn, &call).unwrap();
        let (response_tx, mut response_rx) = tokio::sync::oneshot::channel();

        let started = std::time::Instant::now();
        editor.execute_dynamic_tool_after_policy(turn, tool, call, response_tx, None, false);
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .pending_shell_execution
            .is_some());
        assert_eq!(
            editor.ai_chat_activity(),
            super::super::AiChatActivity::RunningShell
        );

        // A live tool belongs to the chat, not to the chat panel. Hiding the
        // panel must return input ownership to the editor while the tool keeps
        // running in the background.
        crate::editor::InputHandler::handle_key_event(
            &mut editor,
            crate::KeyEvent::new(crate::KeyCode::Esc, crate::Modifiers::NONE),
        )
        .unwrap();
        assert_eq!(editor.mode(), crate::mode::Mode::Normal);
        crate::editor::InputHandler::handle_key_event(
            &mut editor,
            crate::KeyEvent::new(crate::KeyCode::Char('j'), crate::Modifiers::NONE),
        )
        .unwrap();
        assert_eq!(editor.cursor_position().line, 1);
        assert_eq!(
            editor.ai_chat_activity(),
            super::super::AiChatActivity::RunningShell
        );
        assert!(!editor.poll_pending_ai_chat_job());
        assert!(matches!(
            response_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));

        std::fs::write(dir.path().join("release-gate"), "go").unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if editor.poll_pending_ai_chat_job() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "shell did not finish"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let result = response_rx.await.unwrap().unwrap();
        assert!(result.contains("completed"), "{result}");
        assert!(editor.lsp_status().is_empty());
        let events = editor.ai_state.agent_runtime.events(&run_id).unwrap();
        let start_index = events
            .iter()
            .position(|event| matches!(event.kind, EventKind::ToolStarted(_)))
            .unwrap();
        let result_index = events
            .iter()
            .position(|event| matches!(event.kind, EventKind::ToolResult(_)))
            .unwrap();
        let mutation_index = events
            .iter()
            .position(|event| {
                matches!(
                    &event.kind,
                    EventKind::FileMutation(mutation) if mutation.path == "agent-marker"
                )
            })
            .unwrap();
        assert!(start_index < mutation_index && mutation_index < result_index);
        assert_eq!(
            events[mutation_index].operation_id,
            events[start_index].operation_id
        );
    }

    fn attach_finished_classifier(
        editor: &mut Editor,
        result: Result<crate::ai::auto_mode::ClassifierVerdict, String>,
    ) -> tokio::sync::oneshot::Receiver<Result<String, String>> {
        let turn = editor.begin_ai_runtime_turn("review command").unwrap();
        let call = ToolCallInfo {
            id: "classifier-call".into(),
            name: "bash".into(),
            arguments: serde_json::json!({"command": "custom-tool"}),
        };
        let tool = editor.ai_runtime_record_tool_intent(&turn, &call).unwrap();
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let (classification_tx, classification_rx) = tokio::sync::oneshot::channel();
        classification_tx.send(result).unwrap();
        editor
            .ai_state
            .chat
            .as_mut()
            .unwrap()
            .pending_auto_mode_classification =
            Some(super::super::ai_chat_state::PendingAutoModeClassification {
                tool_call: call,
                runtime_tool: tool,
                runtime_turn: turn,
                dynamic_response: response_tx,
                receiver: classification_rx,
            });
        response_rx
    }

    #[tokio::test]
    async fn classifier_failure_escalates_to_paused_user_approval() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let mut response = attach_finished_classifier(&mut editor, Err("protocol failed".into()));

        assert_eq!(editor.ai_chat_attention_generation(), 0);
        assert!(editor.poll_pending_auto_mode_classification());
        assert!(editor.ai_chat_has_pending_tool_approval());
        assert_eq!(editor.ai_chat_attention_generation(), 1);
        assert!(matches!(
            response.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        assert!(editor.set_ai_chat_yolo_mode(true));
        assert!(!editor.ai_chat_has_pending_tool_approval());
    }

    #[tokio::test]
    async fn enabling_yolo_releases_pending_terra_review_without_prompt() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let _response = attach_finished_classifier(&mut editor, Err("still reviewing".into()));

        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .pending_auto_mode_classification
            .is_some());
        assert!(editor.set_ai_chat_yolo_mode(true));

        let chat = editor.ai_state.chat.as_ref().unwrap();
        assert!(chat.pending_auto_mode_classification.is_none());
        assert!(chat.pending_tool_approval.is_none());
        assert!(editor.ai_chat_yolo_mode());
    }

    #[tokio::test]
    async fn classifier_ask_escalates_to_paused_user_approval() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let verdict = crate::ai::auto_mode::ClassifierVerdict::parse_strict(
            r#"{"policy_version":"ovim.auto-mode.v2","decision":"ask","scope":{"project_root":"/repo"},"reason":"the user did not authorize credential access","confidence":0.96,"expiry":{"kind":"after_command"}}"#,
        )
        .unwrap();
        let mut response = attach_finished_classifier(&mut editor, Ok(verdict));

        assert!(editor.poll_pending_auto_mode_classification());
        assert!(editor.ai_chat_has_pending_tool_approval());
        assert!(editor
            .lsp_status()
            .contains("the user did not authorize credential access"));
        assert!(matches!(
            response.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn shell_approval_summary_contains_full_command_and_terra_reason() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        editor.ai_state.chat.as_mut().unwrap().pending_tool_approval =
            Some(super::super::ai_chat_state::PendingToolApproval {
                tool_call: ToolCallInfo {
                    id: "approval-summary".into(),
                    name: "bash".into(),
                    arguments: serde_json::json!({
                        "command": "git diff --check && cargo test\nprintf 'complete\\n'"
                    }),
                },
                reason: "the requested write is not clearly authorized".into(),
                runtime_tool: None,
                runtime_tool_started: false,
                remaining_tool_calls: Vec::new(),
                model_name: "test".into(),
                requested_path: std::path::PathBuf::from("/repo"),
                approval_root: std::path::PathBuf::from("/repo"),
                dynamic_response: None,
                dynamic_turn: None,
            });

        let summary = editor.ai_chat_pending_tool_approval_summary().unwrap();
        assert!(summary.contains("git diff --check && cargo test"));
        assert!(summary.contains("printf 'complete\\n'"));
        assert!(summary.contains("Terra: the requested write is not clearly authorized"));
        assert!(summary.contains("Working directory: /repo"));
    }

    #[tokio::test]
    async fn classifier_deny_returns_terminal_error_without_execution() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let verdict = crate::ai::auto_mode::ClassifierVerdict::parse_strict(
            r#"{"policy_version":"ovim.auto-mode.v2","decision":"deny","scope":{"project_root":"/repo"},"reason":"conflicts with objective","confidence":0.99,"expiry":{"kind":"after_command"}}"#,
        )
        .unwrap();
        let response = attach_finished_classifier(&mut editor, Ok(verdict));

        assert!(editor.poll_pending_auto_mode_classification());
        assert!(!editor.ai_chat_has_pending_tool_approval());
        assert!(response.await.unwrap().is_err());
    }

    fn append_recorded_test_turn(
        editor: &mut Editor,
        user: &str,
        assistant: &str,
    ) -> (NodeId, NodeId, crate::agent_runtime::PendingTurnRef) {
        let turn = editor.begin_ai_runtime_turn(user).unwrap();
        let user_event = turn.initiating_event.caused_by.clone().unwrap();
        editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(turn.clone()));
        let user_node = editor
            .conversation_mut()
            .unwrap()
            .append_user_message(user.into());
        editor.record_ai_chat_node(user_node, user_event);
        let assistant_event = editor.ai_runtime_append_agent_message(assistant).unwrap();
        let assistant_node = editor
            .conversation_mut()
            .unwrap()
            .append_assistant_message(assistant.into(), "test".into());
        editor.record_ai_chat_node(assistant_node, assistant_event);
        editor.ai_runtime_complete_turn();
        (user_node, assistant_node, turn)
    }

    #[test]
    fn shell_authorization_projection_is_chronological_durable_and_bounded() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        for index in 0..10 {
            append_recorded_test_turn(&mut editor, &format!("instruction {index}"), "ack");
        }
        let context = editor.shell_authorization_context(std::path::Path::new("/repo"));
        assert_eq!(context.explicit_user_instructions.len(), 8);
        assert_eq!(
            context.explicit_user_instructions[0].instruction,
            "instruction 2"
        );
        assert_eq!(
            context.explicit_user_instructions[7].instruction,
            "instruction 9"
        );
        assert!(context
            .explicit_user_instructions
            .iter()
            .all(|authorization| authorization.source_id.starts_with("evt_")));
        assert_eq!(context.authorized_objectives[0].objective, "instruction 9");
        assert_eq!(
            context.authorized_objectives[0].source_id,
            context.explicit_user_instructions[7].source_id
        );
    }

    #[test]
    fn ui_fork_gets_distinct_runtime_branch_and_switch_back_resumes_main() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let (_, first_reply, first_turn) = append_recorded_test_turn(&mut editor, "one", "a1");
        let (_, main_leaf, _) = append_recorded_test_turn(&mut editor, "two", "a2");

        assert!(editor.fork_ai_chat_runtime_from(first_reply));
        let fork_turn = editor.begin_ai_runtime_turn("forked").unwrap();
        assert_ne!(fork_turn.branch_id, first_turn.branch_id);
        let fork_user_event_id = fork_turn.initiating_event.caused_by.clone().unwrap();
        let events = editor
            .ai_state
            .agent_runtime
            .events(&fork_turn.run_id)
            .unwrap();
        let fork_user_event = events
            .iter()
            .find(|event| event.event_id == fork_user_event_id)
            .unwrap();
        let selected_event = events
            .iter()
            .find(|event| Some(&event.event_id) == fork_user_event.caused_by.as_ref())
            .unwrap();
        let durable_fork_event = events
            .iter()
            .find(|event| Some(&event.event_id) == selected_event.caused_by.as_ref())
            .unwrap();
        assert!(matches!(
            durable_fork_event.kind,
            EventKind::BranchLifecycle(_)
        ));
        assert_eq!(
            durable_fork_event.caused_by,
            editor
                .ai_state
                .conversation_runtime_nodes
                .get(&editor.ai_chat_conversation_key())
                .unwrap()
                .get(&first_reply)
                .map(|node| node.event_id.clone())
        );
        editor.ai_state.chat.as_mut().unwrap().runtime_turn = Some(Box::new(fork_turn));
        editor.ai_runtime_complete_turn();

        assert!(editor.switch_ai_chat_runtime_branch(main_leaf));
        let resumed = editor.begin_ai_runtime_turn("back").unwrap();
        assert_eq!(resumed.branch_id, first_turn.branch_id);
    }

    #[test]
    fn sibling_fork_messages_switch_between_both_durable_branches() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let (_, first_reply, _) = append_recorded_test_turn(&mut editor, "one", "a1");
        let (main_user, _, _) = append_recorded_test_turn(&mut editor, "main", "a2");

        assert!(editor.fork_ai_chat_runtime_from(first_reply));
        let (fork_user, _, _) = append_recorded_test_turn(&mut editor, "fork", "b2");
        assert_eq!(
            editor.conversation().unwrap().sibling_navigation(fork_user),
            Some((1, 2, main_user, main_user))
        );

        {
            let chat = editor.ai_state.chat.as_mut().unwrap();
            chat.viewport.follow_latest = false;
            chat.viewport.row_scroll_from_bottom = 12;
            chat.history.selected_node_id = Some(fork_user);
        }

        assert!(editor.switch_ai_chat_runtime_branch(main_user));
        let chat = editor.ai_state.chat.as_ref().unwrap();
        assert!(chat.viewport.follow_latest);
        assert_eq!(chat.viewport.row_scroll_from_bottom, 0);
        assert!(chat.history.selected_node_id.is_none());
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|message| message.content == "main"));
        assert!(!editor
            .ai_chat_messages()
            .iter()
            .any(|message| message.content == "fork"));

        assert!(editor.switch_ai_chat_runtime_branch(fork_user));
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|message| message.content == "fork"));
    }

    #[test]
    fn history_selection_tracks_node_identity_across_appends() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.append_user_message("u1".to_string());
            conv.append_assistant_message("a1".to_string(), "m".to_string());
            conv.append_user_message("u2".to_string());
        }

        editor.ai_chat_reset_history_cursor();
        editor.ai_chat_history_cursor_move_older(1); // select a1

        let idx_before = editor
            .ai_chat_history_selected_index()
            .expect("selected index");
        assert_eq!(editor.ai_chat_messages()[idx_before].content, "a1");

        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.append_assistant_message("a2".to_string(), "m".to_string());
        }

        let idx_after = editor
            .ai_chat_history_selected_index()
            .expect("selected index");
        assert_eq!(editor.ai_chat_messages()[idx_after].content, "a1");
    }

    #[test]
    fn history_cursor_visibility_scrolls_viewport_when_selection_offscreen() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.append_user_message("u1".to_string());
            conv.append_assistant_message("a1".to_string(), "m".to_string());
            conv.append_user_message("u2".to_string());
            conv.append_assistant_message("a2".to_string(), "m".to_string());
        }

        editor.render_cache.ai_chat_last_total_rows = 8;
        editor.render_cache.ai_chat_last_visible_start_row = 6;
        editor.render_cache.ai_chat_last_visible_end_row = 8;
        editor.render_cache.ai_chat_last_message_row_spans = vec![(0, 2), (2, 4), (4, 6), (6, 8)];

        editor.ai_chat_reset_history_cursor(); // latest (a2)
        editor.ai_chat_history_cursor_move_older(2); // target a1, above visible region

        let chat = editor.ai_state.chat.as_ref().expect("chat");
        assert!(!chat.viewport.follow_latest);
        assert_eq!(chat.viewport.pinned_base_total_rows, Some(8));
        assert!(chat.viewport.row_scroll_from_bottom > 0);
    }

    #[test]
    fn history_selection_falls_back_to_latest_when_node_leaves_branch() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        let root_id;
        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.append_user_message("u1".to_string());
            root_id = conv.node_ids_for_active_branch()[0];
            conv.append_assistant_message("a1".to_string(), "m".to_string());
            conv.append_user_message("u2".to_string());
        }

        editor.ai_chat_reset_history_cursor();
        editor.ai_chat_history_cursor_move_older(1); // select a1 on original branch

        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.fork_from(root_id);
            conv.append_assistant_message("alt".to_string(), "m".to_string());
        }

        let idx = editor
            .ai_chat_history_selected_index()
            .expect("selected index");
        assert_eq!(editor.ai_chat_messages()[idx].content, "alt");
    }

    #[test]
    fn chat_view_mode_toggles_between_docked_and_review() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        assert!(!editor.ai_chat_review_mode());
        editor.ai_chat_enter_review_mode();
        assert!(editor.ai_chat_review_mode());
        editor.ai_chat_exit_review_mode();
        assert!(!editor.ai_chat_review_mode());
    }

    #[test]
    fn accept_review_clears_markers_and_returns_to_docked_chat() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);
        let buffer_id = editor.buffer().id();

        editor.ai_chat_enter_review_mode();
        {
            let chat = editor.ai_state.chat.as_mut().expect("chat");
            chat.agent_edits.record_edit(buffer_id, 0, 0);
            assert_eq!(chat.agent_edits.total_edit_count(), 1);
        }

        editor.ai_chat_accept_review();

        assert!(!editor.ai_chat_review_mode());
        let edits = editor
            .ai_state
            .chat
            .as_ref()
            .expect("chat")
            .agent_edits
            .total_edit_count();
        assert_eq!(edits, 0);
    }

    #[test]
    fn effective_message_scroll_is_clamped_to_viewport_window() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.viewport.follow_latest = false;
            chat.viewport.row_scroll_from_bottom = 10_000;
            chat.viewport.pinned_base_total_rows = Some(50);
        }

        // With 50 rows and a viewport of 12, max safe scroll is 38.
        let effective = editor.ai_chat_effective_message_scroll(50, 12);
        assert_eq!(effective, 38);
    }

    #[test]
    fn pinned_message_scroll_tracks_both_stream_growth_and_shrinkage() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.viewport.follow_latest = false;
            chat.viewport.row_scroll_from_bottom = 20;
            chat.viewport.pinned_base_total_rows = Some(100);
        }

        assert_eq!(editor.ai_chat_effective_message_scroll(115, 20), 35);
        assert_eq!(editor.ai_chat_effective_message_scroll(90, 20), 10);
    }

    #[test]
    fn scrolling_down_during_streaming_does_not_jump_to_latest() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        editor.render_cache.ai_chat_last_total_rows = 120;
        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.viewport.follow_latest = false;
            chat.viewport.row_scroll_from_bottom = 3;
            chat.viewport.pinned_base_total_rows = Some(100);
        }

        assert!(!editor.ai_chat_scroll_viewport_down(3));
        let chat = editor.ai_state.chat.as_ref().unwrap();
        assert!(!chat.viewport.follow_latest);
        assert_eq!(chat.viewport.row_scroll_from_bottom, 20);
        assert_eq!(chat.viewport.pinned_base_total_rows, Some(120));
        assert_eq!(editor.ai_chat_effective_message_scroll(120, 20), 20);

        assert!(editor.ai_chat_scroll_viewport_down(20));
        let chat = editor.ai_state.chat.as_ref().unwrap();
        assert!(chat.viewport.follow_latest);
        assert_eq!(chat.viewport.row_scroll_from_bottom, 0);
    }

    #[test]
    fn scrolling_up_during_streaming_rebases_the_pinned_offset() {
        let mut editor = Editor::default();
        open_test_chat(&mut editor);

        editor.render_cache.ai_chat_last_total_rows = 120;
        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.viewport.follow_latest = false;
            chat.viewport.row_scroll_from_bottom = 3;
            chat.viewport.pinned_base_total_rows = Some(100);
        }

        editor.ai_chat_scroll_viewport_up(3);
        let chat = editor.ai_state.chat.as_ref().unwrap();
        assert_eq!(chat.viewport.row_scroll_from_bottom, 26);
        assert_eq!(chat.viewport.pinned_base_total_rows, Some(120));
        assert_eq!(editor.ai_chat_effective_message_scroll(120, 20), 26);
    }

    #[test]
    fn conversation_history_survives_buffer_index_shift() {
        let mut editor = Editor::default();

        // Seed two buffers so deleting one will shift indices.
        editor.add_buffer(Buffer::new_from_str("second\n"));
        open_test_chat(&mut editor);

        {
            let conv = editor.conversation_mut().expect("conversation");
            conv.append_user_message("hello".to_string());
        }

        // Delete the first buffer so the chat buffer index changes.
        editor.switch_to_buffer(0);
        let should_quit = editor.delete_current_buffer();
        assert!(!should_quit);

        // Conversation should still resolve through stable BufferId keying.
        let messages = editor.ai_chat_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello");
    }
}
