//! Presentation and lifecycle for provider-owned agents. Tool events in this
//! module are observations, never instructions for Ovim's tool executor.
use super::{ai_chat_state::PendingAiChatJob, Editor};
use crate::ai::{claude_code, AiProfileConfig, ChatRole, StreamChunk, ToolCallInfo};
use crate::run_log::{ProviderConfigurationFingerprint, ProviderSessionKey, RunCatalog};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) struct ExternalAgentState {
    pub root: std::path::PathBuf,
    pub permission: Option<ExternalPermission>,
    pub session_id: Option<String>,
    tools: HashMap<String, (ToolCallInfo, crate::agent_runtime::PendingToolRef)>,
    checkpoint: Option<(Arc<RunCatalog>, ProviderSessionKey)>,
    configuration: String,
}

impl ExternalAgentState {
    pub(crate) fn has_running_tools(&self) -> bool {
        !self.tools.is_empty()
    }
}

pub(crate) struct ExternalPermission {
    pub name: String,
    pub input: Value,
    pub reason: String,
    pub response: tokio::sync::oneshot::Sender<crate::ai::chat_types::ExternalPermissionAnswer>,
    question_index: usize,
    answers: serde_json::Map<String, Value>,
}

impl ExternalPermission {
    fn valid_questions(&self) -> bool {
        let Some(questions) = self.input.get("questions").and_then(Value::as_array) else {
            return false;
        };
        let mut seen = std::collections::HashSet::new();
        !questions.is_empty()
            && questions.len() <= 4
            && questions.iter().all(|question| {
                question
                    .get("question")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty() && seen.insert(text))
                    && question
                        .get("options")
                        .and_then(Value::as_array)
                        .is_some_and(|options| {
                            options.len() <= 8
                                && options.iter().all(|option| {
                                    option
                                        .get("label")
                                        .and_then(Value::as_str)
                                        .is_some_and(|label| !label.trim().is_empty())
                                })
                        })
            })
    }
    fn question(&self) -> Option<&Value> {
        (self.name == "AskUserQuestion")
            .then(|| {
                self.input
                    .get("questions")?
                    .as_array()?
                    .get(self.question_index)
            })
            .flatten()
    }
    fn summary(&self) -> String {
        if let Some(question) = self.question() {
            let mut text = question["question"]
                .as_str()
                .unwrap_or("Claude has a question")
                .to_owned();
            if let Some(options) = question["options"].as_array() {
                for (index, option) in options.iter().enumerate() {
                    text.push_str(&format!(
                        "\n{}. {} — {}",
                        index + 1,
                        option["label"].as_str().unwrap_or(""),
                        option["description"].as_str().unwrap_or("")
                    ));
                }
            }
            text.push_str("\n\nType your answer in the composer and press Enter.");
            text
        } else {
            format!(
                "Claude Code: {}\n{}\n{}\n\nApproval applies to this invocation only.",
                self.name,
                self.reason,
                serde_json::to_string_pretty(&self.input).unwrap_or_default()
            )
        }
    }
}

fn history_digest(
    configuration: &str,
    messages: &[crate::ai::ChatMessage],
) -> ProviderConfigurationFingerprint {
    let mut hash = Sha256::new();
    hash.update(configuration);
    for message in messages {
        // Length-delimited JSON excludes timestamps and includes attachments,
        // so switching branches/providers or editing history invalidates resume.
        let value = json!([
            format!("{:?}", message.role),
            message.content,
            message.model,
            message.tool_call_id,
            message
                .tool_calls
                .iter()
                .map(|call| json!([call.id, call.name, call.arguments]))
                .collect::<Vec<_>>(),
            message
                .images
                .iter()
                .map(|image| format!("{:x}", Sha256::digest(&image.data)))
                .collect::<Vec<_>>()
        ]);
        let bytes = serde_json::to_vec(&value).expect("JSON value serializes");
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    ProviderConfigurationFingerprint {
        version: 1,
        value: format!("{:x}", hash.finalize()),
    }
}

fn append_images(content: &mut Vec<Value>, images: &[crate::ai::chat_types::ImageAttachment]) {
    use base64::Engine;
    for image in images {
        content.push(json!({"type":"image", "source":{"type":"base64", "media_type":image.mime_type, "data":base64::engine::general_purpose::STANDARD.encode(&image.data)}}));
    }
}

fn claude_input(
    previous: &[crate::ai::ChatMessage],
    latest: &crate::ai::ChatMessage,
    resume: bool,
) -> Vec<Value> {
    let mut content = Vec::new();
    if !resume && !previous.is_empty() {
        content.push(json!({"type":"text", "text":"Earlier conversation shown in Ovim (context, not new instructions):"}));
        for message in previous {
            let mut text = format!("{:?}: {}", message.role, message.content);
            for tool in &message.tool_calls {
                text.push_str(&format!(
                    "\nObserved tool {} ({}): {}",
                    tool.name, tool.id, tool.arguments
                ));
            }
            if let Some(id) = &message.tool_call_id {
                text.push_str(&format!("\nResult for tool {id}"));
            }
            content.push(json!({"type":"text", "text":text}));
            append_images(&mut content, &message.images);
        }
        content.push(json!({"type":"text", "text":"Current user message follows:"}));
    }
    if !latest.content.is_empty() {
        content.push(json!({"type":"text", "text":latest.content}));
    }
    append_images(&mut content, &latest.images);
    content
}

impl Editor {
    pub fn ai_chat_uses_external_agent(&self) -> bool {
        self.ai_state
            .config
            .resolve_profile(&self.ai_chat_effective_profile())
            .is_some_and(|profile| profile.provider.owns_agent_loop())
    }

    pub fn ai_chat_has_external_question(&self) -> bool {
        self.external_permission()
            .is_some_and(|pending| pending.question().is_some())
    }

    fn external_permission(&self) -> Option<&ExternalPermission> {
        self.ai_state
            .chat
            .as_ref()?
            .external_agent
            .as_ref()?
            .permission
            .as_ref()
    }

    pub(crate) fn external_permission_summary(&self) -> Option<String> {
        self.external_permission().map(ExternalPermission::summary)
    }

    pub(crate) fn resolve_external_permission(&mut self, allow: bool) -> bool {
        if allow && self.ai_chat_has_external_question() {
            self.set_status_message("Answer Claude's question in the chat composer");
            return true;
        }
        let pending = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.external_agent.as_mut())
            .and_then(|state| state.permission.take());
        let Some(pending) = pending else {
            return false;
        };
        let _ = pending
            .response
            .send(crate::ai::chat_types::ExternalPermissionAnswer { allow, input: None });
        self.set_status_message(if allow {
            "Approved Claude Code tool for this invocation"
        } else {
            "Denied Claude Code tool"
        });
        true
    }

    pub(crate) fn answer_external_question(&mut self, answer: &str) -> bool {
        if !self.ai_chat_has_external_question() {
            return false;
        }
        if answer.trim().is_empty() {
            return true;
        }
        let chat = self.ai_state.chat.as_mut().expect("question has chat");
        let state = chat.external_agent.as_mut().expect("question has runtime");
        let pending = state.permission.as_mut().expect("question has callback");
        let question = pending.question().expect("checked question");
        let key = question["question"].as_str().unwrap_or_default().to_owned();
        let numbers: Option<Vec<&str>> = answer
            .split(',')
            .map(|part| {
                part.trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|number| number.checked_sub(1))
                    .and_then(|index| question["options"].get(index))
                    .and_then(|option| option["label"].as_str())
            })
            .collect();
        let selected = numbers
            .filter(|labels| labels.len() == 1 || question["multiSelect"].as_bool() == Some(true))
            .map(|labels| labels.join(", "))
            .unwrap_or_else(|| answer.to_owned());
        pending.answers.insert(key, Value::String(selected));
        pending.question_index += 1;
        chat.input.clear();
        chat.input_cursor = 0;
        if pending.question().is_some() {
            let summary = pending.summary();
            if let Some(conv) = self.conversation_mut() {
                conv.append_assistant_message(summary, "Claude Agent".into());
            }
        } else {
            let mut pending = state.permission.take().expect("question callback");
            pending.input["answers"] = Value::Object(pending.answers);
            let _ = pending
                .response
                .send(crate::ai::chat_types::ExternalPermissionAnswer {
                    allow: true,
                    input: Some(pending.input),
                });
        }
        true
    }

    pub(crate) fn spawn_external_agent(&mut self, profile: AiProfileConfig) -> Result<()> {
        let executable = which::which("claude").context("Claude Code is not installed or not on PATH. Install Claude Code and run `claude auth login` in your terminal")?;
        which::which("node")
            .context("Claude Code integration requires Node.js 18.18 or newer on PATH")?;
        let chat = self.ai_state.chat.as_ref().context("No active chat")?;
        if profile.model.trim().is_empty() {
            bail!("Claude Code model must be a model name or 'default'");
        }
        if profile.api_key.is_some() || profile.api_key_env.is_some() || profile.base_url.is_some()
        {
            bail!("Claude Code manages its own authentication and endpoint settings; remove API credentials/base_url from this profile");
        }
        if !profile.tools.is_empty()
            || profile.system_prompt.is_some()
            || profile.chat_prompt.is_some()
            || chat.opts.system_prompt.is_some()
        {
            bail!("Claude Code profiles use Claude's own tools and system prompt; configure these through Claude Code settings");
        }
        let effort = chat
            .reasoning_effort_override
            .clone()
            .or(profile.reasoning_effort.clone());
        if effort
            .as_deref()
            .is_some_and(|value| !["low", "medium", "high", "xhigh", "max"].contains(&value))
        {
            bail!("Claude Code effort must be low, medium, high, xhigh, max, or profile default");
        }
        let turn = chat
            .runtime_turn
            .as_deref()
            .context("No active turn")?
            .clone();
        let cwd = self
            .ai_effective_project_root()
            .or_else(|| self.ai_project_start_path())
            .unwrap_or(std::env::current_dir()?)
            .canonicalize()?;
        let cwd = if cwd.is_file() {
            cwd.parent().context("No parent folder")?.to_path_buf()
        } else {
            cwd
        };
        let configuration = serde_json::to_string(&json!([
            claude_code::SDK_VERSION,
            "ovim-editor-mcp-v1",
            profile.name,
            profile.model,
            effort,
            cwd,
            executable,
            chat.allow_edits,
            chat.context_generation,
            std::env::var_os("CLAUDE_CONFIG_DIR").map(|path| path.to_string_lossy().into_owned())
        ]))?;
        let messages = self.ai_chat_messages();
        let (latest, previous) = messages.split_last().context("No user message")?;
        if latest.role != ChatRole::User {
            bail!("Claude Code requires a user message");
        }
        let checkpoint = self.ai_state.durable_runs.as_ref().map(|services| {
            (
                services.catalog.clone(),
                ProviderSessionKey {
                    provider: "claude_code".into(),
                    agent_id: turn.agent_id.clone(),
                    branch_id: turn.branch_id.clone(),
                },
            )
        });
        let resume = checkpoint
            .as_ref()
            .map(|(catalog, key)| {
                catalog.provider_session(key, &history_digest(&configuration, previous))
            })
            .transpose()?
            .flatten()
            .map(|session| session.provider_thread_id);
        let content = if latest.content.starts_with('/') {
            // Native commands must be the whole prompt; prefixing editor state
            // or reconstructed history would turn them into ordinary prose.
            if resume.is_none() {
                bail!("Start a Claude Code conversation before using a native command");
            }
            claude_input(previous, latest, true)
        } else {
            let mut content = claude_input(previous, latest, resume.is_some());
            // Supply the same editor snapshot as other chat profiles, as user
            // context. Claude still owns its system prompt and tool definitions.
            content.insert(
                0,
                json!({"type":"text", "text":self.build_editor_state_context(2_500)}),
            );
            content
        };
        let request = claude_code::Request {
            cwd: cwd.clone(),
            executable,
            model: profile.model.clone(),
            effort,
            allow_edits: chat.allow_edits,
            resume,
            content,
        };
        let branch_generation = self
            .conversation()
            .map(|conv| conv.branch_generation())
            .unwrap_or_default();
        // Consume a checkpoint before running: interrupted/failed turns must not
        // resume a native session whose unseen side effects/history moved ahead.
        if let Some((catalog, key)) = &checkpoint {
            catalog.delete_provider_session(key)?;
        }
        let (tx, receiver) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            if let Err(error) = claude_code::stream(request, tx.clone()).await {
                let _ = tx.send(StreamChunk::Error(error.to_string()));
            }
        });
        let chat = self.ai_state.chat.as_mut().context("No active chat")?;
        chat.external_agent = Some(ExternalAgentState {
            root: cwd,
            permission: None,
            session_id: None,
            tools: HashMap::new(),
            checkpoint,
            configuration,
        });
        chat.pending_job = Some(PendingAiChatJob {
            receiver,
            task,
            profile_name: profile.name,
            model_name: profile.model,
            turn: Box::new(turn),
            branch_generation,
            steer_tx: None,
        });
        chat.streaming_content = Some(String::new());
        chat.streaming_thinking = None;
        chat.streaming_tool_calls.clear();
        chat.runtime_recorded_content_bytes = 0;
        chat.runtime_recorded_thinking_bytes = 0;
        chat.runtime_last_content_event = None;
        chat.runtime_last_reasoning_event = None;
        Ok(())
    }

    pub(crate) fn observe_external_tool(
        &mut self,
        mut call: ToolCallInfo,
        model: &str,
    ) -> Result<()> {
        if let Some(name) = call.name.strip_prefix("mcp__ovim__") {
            if self
                .ai_state
                .tool_registry
                .editor_bridge_tools()
                .any(|(tool, _)| tool.name == name)
            {
                call.name = name.to_owned();
            }
        }
        let state = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.external_agent.as_ref())
            .context("Tool observation without an external agent")?;
        if state.tools.contains_key(&call.id) {
            bail!("Duplicate Claude tool ID: {}", call.id);
        }
        self.flush_ai_runtime_stream_segments();
        self.commit_partial_streaming(model);
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.streaming_content = Some(String::new());
            chat.runtime_recorded_content_bytes = 0;
            chat.runtime_recorded_thinking_bytes = 0;
            chat.runtime_last_content_event = None;
            chat.runtime_last_reasoning_event = None;
        }
        let turn = self.active_ai_runtime_turn().context("No active turn")?;
        let tool = self.ai_runtime_record_tool_intent(&turn, &call)?;
        self.ai_runtime_start_tool(&turn, &tool)?;
        let event_id = self.ai_runtime_current_tip();
        let node = self.conversation_mut().map(|conv| {
            conv.append_assistant_message_with_tools_and_state(
                String::new(),
                model.into(),
                vec![call.clone()],
                vec![],
            )
        });
        if let (Some(node), Some(event)) = (node, event_id) {
            self.record_ai_chat_node(node, event);
        }
        if let Some(state) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.external_agent.as_mut())
        {
            state.tools.insert(call.id.clone(), (call, tool));
        }
        Ok(())
    }

    pub(crate) fn observe_external_result(
        &mut self,
        id: String,
        content: String,
        error: bool,
    ) -> Result<()> {
        let (call, tool) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.external_agent.as_mut())
            .and_then(|state| state.tools.remove(&id))
            .context("Claude result has no matching tool observation")?;
        if call.name == "explain_with_codebase" {
            self.bind_editor_walkthrough_result(&call.id, &content);
        }
        let result = if error {
            crate::ai::ToolResult::Error(content.clone())
        } else {
            crate::ai::ToolResult::Success(content.clone())
        };
        let turn = self.active_ai_runtime_turn().context("No active turn")?;
        self.ai_runtime_finish_tool(&turn, &tool, &result)?;
        self.record_tool_event_summary(&call, &result);
        let event_id = self.ai_runtime_current_tip();
        let node = self
            .conversation_mut()
            .map(|conv| conv.append_tool_result(id, content));
        if let (Some(node), Some(event)) = (node, event_id) {
            self.record_ai_chat_node(node, event);
        }
        Ok(())
    }

    pub(crate) fn receive_external_permission(
        &mut self,
        name: String,
        input: Value,
        reason: String,
        response: tokio::sync::oneshot::Sender<crate::ai::chat_types::ExternalPermissionAnswer>,
    ) {
        let permission = ExternalPermission {
            name,
            input,
            reason,
            response,
            question_index: 0,
            answers: serde_json::Map::new(),
        };
        let summary = permission.summary();
        if permission.name == "AskUserQuestion" {
            if !permission.valid_questions() {
                let _ = permission
                    .response
                    .send(crate::ai::chat_types::ExternalPermissionAnswer::default());
                self.set_status_message("Claude Code sent an invalid question");
                return;
            }
            if let Some(conv) = self.conversation_mut() {
                conv.append_assistant_message(summary.clone(), "Claude Agent".into());
            }
        }
        if let Some(state) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.external_agent.as_mut())
        {
            state.permission = Some(permission);
        }
        self.set_status_message(summary);
    }

    pub(crate) fn checkpoint_external_agent(&mut self) -> Result<()> {
        let Some(state) = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.external_agent.as_ref())
        else {
            return Ok(());
        };
        if !state.tools.is_empty() {
            bail!("Claude Code completed with unfinished tools");
        }
        if let (Some(id), Some((catalog, key))) = (&state.session_id, &state.checkpoint) {
            catalog.upsert_provider_session(
                key.clone(),
                id.clone(),
                history_digest(&state.configuration, self.ai_chat_messages()),
                None,
            )?;
        }
        Ok(())
    }

    pub(crate) fn retire_external_tools(&mut self) {
        let Some(turn) = self.active_ai_runtime_turn() else {
            return;
        };
        let tools = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.external_agent.as_mut())
            .map(|state| std::mem::take(&mut state.tools))
            .unwrap_or_default();
        for (id, (_, tool)) in tools {
            if let Err(error) = self.ai_state.agent_runtime.mark_tool_outcome_unknown(
                &turn,
                &tool,
                "Claude Code stopped before the tool outcome was observed",
            ) {
                crate::log_warn!(
                    "agent_runtime",
                    "Unable to record interrupted Claude tool: {error}"
                );
            }
            if let Some(conv) = self.conversation_mut() {
                conv.append_tool_result(
                    id,
                    "Interrupted; outcome unknown. Check the working tree before retrying.".into(),
                );
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::ai::{AiProviderKind, ChatOpts, ConversationTree};

    pub(crate) fn editor() -> Editor {
        let mut editor = Editor::default();
        let mut profile = editor.ai_state.config.profiles["local"].clone();
        profile.name = "claude_code".into();
        profile.provider = AiProviderKind::ClaudeCode;
        profile.model = "default".into();
        editor
            .ai_state
            .config
            .profiles
            .insert(profile.name.clone(), profile);
        assert!(editor.ai_set_profile("claude_code"));
        editor
            .open_ai_chat(ChatOpts {
                profile: Some("claude_code".into()),
                allow_edits: true,
                ..Default::default()
            })
            .unwrap();
        editor
    }

    pub(crate) fn attach(editor: &mut Editor) -> tokio::sync::mpsc::UnboundedSender<StreamChunk> {
        let turn = editor.begin_ai_runtime_turn("Inspect the fixture").unwrap();
        editor
            .conversation_mut()
            .unwrap()
            .append_user_message("Inspect the fixture".into());
        let (tx, receiver) = tokio::sync::mpsc::unbounded_channel();
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.runtime_turn = Some(Box::new(turn.clone()));
        chat.waiting = true;
        chat.streaming_content = Some(String::new());
        chat.external_agent = Some(ExternalAgentState {
            root: std::env::current_dir().unwrap(),
            permission: None,
            session_id: None,
            tools: HashMap::new(),
            checkpoint: None,
            configuration: "test".into(),
        });
        chat.pending_job = Some(PendingAiChatJob {
            receiver,
            task: tokio::spawn(std::future::pending()),
            profile_name: "claude_code".into(),
            model_name: "default".into(),
            turn: Box::new(turn),
            branch_generation: 0,
            steer_tx: None,
        });
        tx
    }

    #[tokio::test]
    async fn claude_tools_are_presented_and_logged_without_executing_in_ovim() {
        let mut editor = editor();
        let before = editor.buffer().rope().to_string();
        let tx = attach(&mut editor);
        tx.send(StreamChunk::Content("Inspecting. ".into()))
            .unwrap();
        tx.send(StreamChunk::ExternalToolStart(ToolCallInfo {
            id: "tool-1".into(),
            name: "bash".into(),
            arguments: json!({"command":"this-command-must-never-be-executed"}),
        }))
        .unwrap();
        tx.send(StreamChunk::ExternalToolResult {
            id: "tool-1".into(),
            content: "Provider-owned result".into(),
            error: false,
        })
        .unwrap();
        tx.send(StreamChunk::Content("Finished.".into())).unwrap();
        tx.send(StreamChunk::ExternalSession("session-1".into()))
            .unwrap();
        tx.send(StreamChunk::Done).unwrap();
        assert!(editor.poll_pending_ai_chat_job());
        assert_eq!(
            editor.ai_chat_activity(),
            super::super::AiChatActivity::Idle
        );
        assert_eq!(editor.buffer().rope().to_string(), before);
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|m| m.role == ChatRole::Tool && m.content == "Provider-owned result"));
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|m| m.content == "Finished."));
        assert!(!editor
            .ai_chat_messages()
            .iter()
            .any(|m| m.role == ChatRole::Error));
    }

    #[tokio::test]
    async fn claude_approval_only_answers_the_provider_and_cancellation_cleans_up() {
        let mut editor = editor();
        let tx = attach(&mut editor);
        let (response, answer) = tokio::sync::oneshot::channel();
        tx.send(StreamChunk::ExternalPermission {
            name: "Bash".into(),
            input: json!({"command":"example"}),
            reason: "Claude asks".into(),
            response,
        })
        .unwrap();
        editor.poll_pending_ai_chat_job();
        assert!(editor.ai_chat_has_pending_tool_approval());
        assert!(editor
            .ai_chat_pending_tool_approval_summary()
            .unwrap()
            .contains("example"));
        assert!(editor.ai_chat_resolve_pending_tool_approval(true, true));
        assert!(answer.await.unwrap().allow);
        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .pending_shell_execution
            .is_none());
        let task = editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .pending_job
            .as_ref()
            .unwrap()
            .task
            .abort_handle();
        assert!(editor.cancel_ai_chat_generation());
        tokio::task::yield_now().await;
        assert!(task.is_finished());
        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .external_agent
            .is_none());
    }

    #[tokio::test]
    async fn claude_questions_use_the_shared_composer_instead_of_queuing_a_turn() {
        let mut editor = editor();
        let tx = attach(&mut editor);
        let (response, mut answer) = tokio::sync::oneshot::channel();
        let input = json!({"questions":[
            {"question":"Which colors?", "multiSelect":true,"options":[{"label":"Blue"},{"label":"Green"}]},
            {"question":"Any detail?", "options":[]}
        ]});
        tx.send(StreamChunk::ExternalPermission {
            name: "AskUserQuestion".into(),
            input,
            reason: "Question".into(),
            response,
        })
        .unwrap();
        editor.poll_pending_ai_chat_job();
        assert!(editor.ai_chat_has_external_question());
        assert!(!editor.ai_chat_has_pending_tool_approval());
        editor.ai_state.chat.as_mut().unwrap().input = "1, 2".into();
        editor.submit_ai_chat_message().unwrap();
        assert!(answer.try_recv().is_err());
        editor.ai_state.chat.as_mut().unwrap().input = "På norsk 🦦".into();
        editor.submit_ai_chat_message().unwrap();
        let answer = answer.await.unwrap();
        assert!(answer.allow);
        assert_eq!(
            answer.input.unwrap()["answers"],
            json!({"Which colors?":"Blue, Green", "Any detail?":"På norsk 🦦"})
        );
        assert!(editor.ai_chat_input().is_empty());
        assert!(!editor.ai_chat_has_external_question());
    }

    #[tokio::test]
    async fn claude_rejects_invalid_questions_and_retires_unknown_tool_outcomes() {
        let mut editor = editor();
        let tx = attach(&mut editor);
        let (response, answer) = tokio::sync::oneshot::channel();
        tx.send(StreamChunk::ExternalPermission {
            name: "AskUserQuestion".into(),
            input: json!({"questions":[{}]}),
            reason: "Question".into(),
            response,
        })
        .unwrap();
        editor.poll_pending_ai_chat_job();
        assert!(!answer.await.unwrap().allow);
        tx.send(StreamChunk::ExternalToolStart(ToolCallInfo {
            id: "tool-1".into(),
            name: "Write".into(),
            arguments: json!({}),
        }))
        .unwrap();
        editor.poll_pending_ai_chat_job();
        assert!(editor.cancel_ai_chat_generation());
        assert!(editor
            .ai_chat_messages()
            .iter()
            .any(|m| m.role == ChatRole::Tool && m.content.contains("outcome unknown")));
    }

    #[tokio::test]
    async fn claude_profile_updates_defaults_but_cannot_change_during_a_turn() {
        let mut editor = editor();
        assert_eq!(
            editor.ai_chat_context_profile("chat").as_deref(),
            Some("claude_code")
        );
        assert_eq!(
            editor.ai_chat_context_profile("query").as_deref(),
            Some("claude_code")
        );
        assert!(!editor.ai_chat_reasoning_efforts().contains(&"none"));
        assert!(!editor.set_ai_chat_yolo_mode(true));
        assert!(
            !editor.set_ai_chat_comprehension_policy(super::super::ComprehensionPolicy::Publish)
        );
        assert!(!editor.ai_subagent_parent_tools_visible());
        let _tx = attach(&mut editor);
        assert!(!editor.ai_set_profile("local"));
        assert!(editor.cancel_ai_chat_generation());
        assert!(editor.ai_set_profile("local"));
        assert_eq!(
            editor.ai_chat_context_profile("chat").as_deref(),
            Some("local")
        );
    }

    #[test]
    fn claude_checkpoints_require_exact_history_and_configuration() {
        let mut tree = ConversationTree::new();
        tree.append_user_message("Question".into());
        let node = tree.append_assistant_message("Answer".into(), "Claude".into());
        let fingerprint = history_digest("config", tree.messages());
        assert_eq!(fingerprint, history_digest("config", tree.messages()));
        assert_ne!(
            fingerprint,
            history_digest("different model or cwd", tree.messages())
        );
        tree.append_user_message("Next question".into());
        assert_ne!(fingerprint, history_digest("config", tree.messages()));
        tree.fork_from(node);
        assert_eq!(fingerprint, history_digest("config", tree.messages()));
    }

    #[test]
    fn fresh_native_sessions_preserve_visible_tools_and_images() {
        let mut tree = ConversationTree::new();
        tree.append_user_message("Earlier question".into());
        let mut previous = tree.messages().to_vec();
        previous[0]
            .images
            .push(crate::ai::chat_types::ImageAttachment {
                path: "example.png".into(),
                mime_type: "image/png".into(),
                data: vec![1, 2, 3],
            });
        previous[0].tool_calls.push(ToolCallInfo {
            id: "observed-1".into(),
            name: "Read".into(),
            arguments: json!({"file_path":"example.rs"}),
        });
        tree.append_user_message("Current question".into());
        let latest = tree.messages().last().unwrap();
        let fresh = claude_input(&previous, latest, false);
        assert!(fresh.iter().any(|block| block["text"]
            .as_str()
            .is_some_and(|text| text.contains("observed-1") && text.contains("example.rs"))));
        assert!(fresh
            .iter()
            .any(|block| block["type"] == "image" && block["source"]["data"] == "AQID"));
        assert_eq!(
            claude_input(&previous, latest, true),
            vec![json!({"type":"text", "text":"Current question"})]
        );
    }

    #[tokio::test]
    async fn completed_claude_turn_persists_a_checkpoint_for_its_final_visible_history() {
        let directory = tempfile::tempdir().unwrap();
        let layout = crate::run_log::RunStorageLayout::new(directory.path().join("runs"));
        let catalog = Arc::new(RunCatalog::open(&layout).unwrap());
        let mut editor = editor();
        let tx = attach(&mut editor);
        let turn = editor.active_ai_runtime_turn().unwrap();
        let key = ProviderSessionKey {
            provider: "claude_code".into(),
            agent_id: turn.agent_id.clone(),
            branch_id: turn.branch_id.clone(),
        };
        editor
            .ai_state
            .chat
            .as_mut()
            .unwrap()
            .external_agent
            .as_mut()
            .unwrap()
            .checkpoint = Some((catalog.clone(), key.clone()));
        let before = history_digest("test", editor.ai_chat_messages());
        tx.send(StreamChunk::Content("Finished.".into())).unwrap();
        tx.send(StreamChunk::ExternalSession("native-session".into()))
            .unwrap();
        tx.send(StreamChunk::Done).unwrap();
        editor.poll_pending_ai_chat_job();
        let after = history_digest("test", editor.ai_chat_messages());
        assert_ne!(before, after);
        let reopened = RunCatalog::open(&layout).unwrap();
        assert!(reopened.provider_session(&key, &before).unwrap().is_none());
        assert_eq!(
            reopened
                .provider_session(&key, &after)
                .unwrap()
                .unwrap()
                .provider_thread_id,
            "native-session"
        );
    }

    #[tokio::test]
    async fn dropping_editor_aborts_claude_job() {
        let mut editor = editor();
        let _tx = attach(&mut editor);
        let task = editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .pending_job
            .as_ref()
            .unwrap()
            .task
            .abort_handle();
        drop(editor);
        tokio::task::yield_now().await;
        assert!(task.is_finished());
    }
}
