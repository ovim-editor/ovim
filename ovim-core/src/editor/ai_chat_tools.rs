use crate::ai::chat_types::{ChatMessage, StreamChunk, ToolCallInfo, ToolSummaryKind};
use crate::ai::scope::{Capabilities, ScopeContext};
use crate::ai::stream_ai_chat;
use crate::ai::tools::builtins::{self, ProjectDiagnosticFile, ToolExecutionContext};
use crate::ai::tools::schema;
use crate::ai::tools::{SideEffect, ToolResult};
use anyhow::Result;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::ai_chat_state::{PendingAiChatJob, PendingToolApproval, ToolEventSummary};
use super::Editor;

#[derive(Debug, Clone)]
struct ToolApprovalRequest {
    requested_path: PathBuf,
    approval_root: PathBuf,
    message: String,
}

enum ToolDispatchOutcome {
    Completed(ToolResult),
    ApprovalRequired(ToolApprovalRequest),
}

enum ToolPathResolution {
    Allowed {
        absolute_path: PathBuf,
        boundary_root: PathBuf,
    },
    NeedsApproval(ToolApprovalRequest),
}

impl Editor {
    // -----------------------------------------------------------------
    // Tool execution helpers
    // -----------------------------------------------------------------

    /// Build capabilities for the current chat session.
    pub(crate) fn build_chat_capabilities(&self) -> Capabilities {
        let profile_scope = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| c.opts.profile.as_ref())
            .and_then(|p| self.ai_state.config.resolve_profile(p))
            .map(|p| &p.scope)
            .cloned()
            .unwrap_or_default();

        let allow_edits = self
            .ai_state
            .chat
            .as_ref()
            .map(|c| c.allow_edits)
            .unwrap_or(false);

        // Base capabilities from profile scope
        let mut caps = Capabilities {
            file_scope: profile_scope.files,
            shell: profile_scope.shell,
            network: profile_scope.network,
            allow_mutations: allow_edits,
        };

        // Without an active file target, keep this chat session file-scoped so
        // project-level tools do not behave inconsistently.
        if !self.active_chat_target_has_file_path()
            && caps.file_scope >= crate::ai::FileScope::Project
        {
            caps.file_scope = crate::ai::FileScope::File;
        }

        // Without an approved project boundary, force file-scoped access for
        // project tools to prevent broad accidental traversal from process CWD.
        if self.ai_effective_project_root().is_none()
            && caps.file_scope >= crate::ai::FileScope::Project
        {
            caps.file_scope = crate::ai::FileScope::File;
        }

        // If edits not allowed, disable shell but keep file_scope at profile level
        // so read-only project tools (search_project, list_files, read_file_at_path)
        // remain available.
        if !allow_edits {
            caps.shell = false;
        }

        caps
    }

    /// Build tool JSON schemas for the current chat session's provider.
    pub(crate) fn build_tool_schemas_for_chat(
        &self,
        profile: &crate::ai::AiProfileConfig,
    ) -> Vec<serde_json::Value> {
        let caps = self.build_chat_capabilities();
        let tools = self
            .ai_state
            .tool_registry
            .tools_for_profile(profile, &caps);
        if tools.is_empty() {
            return vec![];
        }

        match profile.provider {
            crate::ai::AiProviderKind::OpenAi | crate::ai::AiProviderKind::Ollama => {
                schema::tools_to_openai_schema(&tools)
            }
            crate::ai::AiProviderKind::Anthropic => schema::tools_to_anthropic_schema(&tools),
        }
    }

    /// Snapshot current editor state into a ToolExecutionContext.
    pub(crate) fn build_tool_execution_context(&self) -> ToolExecutionContext {
        let target_index = self.active_chat_target_buffer_index();
        let buf = &self.buffers[target_index];
        let buffer_content = buf.rope().to_string();
        let file_path = buf.file_path().map(|p| p.to_string());
        let cursor = {
            let c = buf.cursor();
            (c.line(), c.col())
        };

        // Try to get selection from visual mode or last selection
        let selection = self
            .ai_state
            .active_selection
            .as_ref()
            .map(|s| (s.start_line, s.start_col, s.end_line, s.end_col));

        // Get diagnostics for active target buffer
        let diagnostics = self.get_diagnostics_for_buffer_index(target_index);
        let project_diagnostics = self.get_project_diagnostics_for_chat();

        let current_file = buf
            .file_path()
            .map(PathBuf::from)
            .map(|p| self.absolutize_path(&p));
        let project_root = self.ai_effective_project_root();

        // Snapshot all open buffers so read_file_at_path can read
        // in-memory content instead of potentially stale disk files.
        let mut open_buffers = std::collections::HashMap::new();
        for b in &self.buffers {
            if let Some(p) = b.file_path() {
                let path = std::path::Path::new(p);
                let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
                open_buffers.insert(key, b.rope().to_string());
            }
        }

        ToolExecutionContext {
            buffer_content,
            file_path,
            cursor,
            selection,
            diagnostics,
            project_diagnostics,
            scope_context: ScopeContext {
                current_file,
                project_root,
            },
            capabilities: self.build_chat_capabilities(),
            open_buffers,
        }
    }

    fn active_chat_target_buffer_index(&self) -> usize {
        let current = self.current_buffer_index;
        self.ai_state
            .chat
            .as_ref()
            .map(|chat| chat.active_buffer_id)
            .and_then(|buffer_id| self.find_buffer_index_by_id(buffer_id))
            .unwrap_or(current)
    }

    fn active_chat_target_buffer_index_strict(&self) -> std::result::Result<usize, String> {
        let Some(chat) = self.ai_state.chat.as_ref() else {
            return Ok(self.current_buffer_index);
        };
        self.find_buffer_index_by_id(chat.active_buffer_id).ok_or_else(|| {
            format!(
                "Active chat target is no longer available (buffer id {}). Re-open the target file with open_file before continuing.",
                chat.active_buffer_id
            )
        })
    }

    fn active_chat_target_has_file_path(&self) -> bool {
        let Ok(target_index) = self.active_chat_target_buffer_index_strict() else {
            return false;
        };
        self.buffers
            .get(target_index)
            .and_then(|b| b.file_path())
            .is_some()
    }

    fn no_file_open_guidance(&self) -> String {
        "No file open. Open or select a file first, then retry. Tip: use open_file(path, create=true) if you know the target path.".to_string()
    }

    /// Execute a single read tool call, checking scope before dispatch.
    pub(crate) fn execute_tool_call(
        &self,
        tool_call: &ToolCallInfo,
        ctx: &ToolExecutionContext,
    ) -> ToolResult {
        let Some(tool_def) = self.ai_state.tool_registry.get(&tool_call.name) else {
            return ToolResult::Error(format!("unknown tool: {}", tool_call.name));
        };

        // Check that capabilities satisfy the tool's requirements
        if !ctx.capabilities.contains(&tool_def.required_scope) {
            return ToolResult::Error(format!(
                "tool '{}' requires scope not granted by current context",
                tool_call.name
            ));
        }

        builtins::execute_builtin(&tool_call.name, &tool_call.arguments, ctx)
    }

    /// Dispatch a single tool call by side effect. Read tools get a snapshot,
    /// mutation tools get `&mut self`.
    ///
    /// `approved_once_root` temporarily allows one outside-project access for the call.
    fn dispatch_tool_call_with_approval(
        &mut self,
        tc: &ToolCallInfo,
        approved_once_root: Option<&PathBuf>,
    ) -> ToolDispatchOutcome {
        if let Err(err) = self.active_chat_target_buffer_index_strict() {
            return ToolDispatchOutcome::Completed(ToolResult::Error(err));
        }

        if !self.active_chat_target_has_file_path() && tc.name != "open_file" {
            return ToolDispatchOutcome::Completed(ToolResult::Error(self.no_file_open_guidance()));
        }

        if tc.name == "read_file_at_path" {
            return self.execute_read_file_at_path_tool(tc, approved_once_root);
        }
        if tc.name == "list_files" {
            return self.execute_list_files_tool(tc, approved_once_root);
        }
        if tc.name == "open_file" {
            return self.execute_open_file_tool(tc, approved_once_root);
        }
        if matches!(
            tc.name.as_str(),
            "edit_range"
                | "insert_lines"
                | "delete_lines"
                | "write_file_at_path"
                | "create_file"
                | "snapshot_file"
                | "restore_file"
        ) {
            return self.execute_path_scoped_mutation_tool(tc, approved_once_root);
        }

        let result = match self
            .ai_state
            .tool_registry
            .get(&tc.name)
            .map(|t| t.side_effect)
        {
            Some(SideEffect::Read) => match tc.name.as_str() {
                "document_symbols" | "hover" | "goto_definition" => {
                    self.execute_lsp_tool(&tc.name, &tc.arguments)
                }
                _ => {
                    let ctx = self.build_tool_execution_context();
                    self.execute_tool_call(tc, &ctx)
                }
            },
            Some(SideEffect::Navigation) => self.execute_navigation_tool(&tc.name, &tc.arguments),
            Some(SideEffect::Mutation) => self.execute_mutation_tool(&tc.name, &tc.arguments),
            Some(SideEffect::External) => {
                ToolResult::Error("external tools not yet supported".into())
            }
            None => ToolResult::Error(format!("unknown tool: {}", tc.name)),
        };
        ToolDispatchOutcome::Completed(result)
    }

    /// Execute tool calls from a completed stream response, record results,
    /// and continue the conversation. Returns true to signal state changed.
    pub(crate) fn process_tool_calls(
        &mut self,
        tool_calls: Vec<ToolCallInfo>,
        content: String,
        model_name: &str,
    ) -> bool {
        let used = self
            .ai_state
            .chat
            .as_ref()
            .map(|c| c.tool_call_count)
            .unwrap_or(0);
        let max_tool_calls = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| c.opts.profile.as_ref())
            .and_then(|p| self.ai_state.config.resolve_profile(p))
            .map(|p| p.agent_loop.max_tool_calls)
            .unwrap_or(50);

        if used >= max_tool_calls {
            // Hit limit — commit what we have and stop
            if !content.is_empty() {
                if let Some(conv) = self.conversation_mut() {
                    conv.append_assistant_message(content, model_name.to_string());
                }
            }
            if let Some(conv) = self.conversation_mut() {
                conv.append_error("Tool call iteration limit reached.".to_string());
            }
            self.clear_streaming_state();
            return true;
        }

        // Set up undo group for this tool call batch
        if let Some(chat) = self.ai_state.chat.as_mut() {
            if chat.current_undo_group.is_none() {
                let gid = chat.next_undo_group_id;
                chat.next_undo_group_id += 1;
                chat.current_undo_group = Some(gid);
            }
        }

        // 1. Commit content + tool_calls as assistant message
        if let Some(conv) = self.conversation_mut() {
            conv.append_assistant_message_with_tools(
                content,
                model_name.to_string(),
                tool_calls.clone(),
            );
        }

        // 2. Execute tools. May pause for user approval.
        self.execute_tool_call_batch(tool_calls, model_name.to_string())
    }

    /// Resolve a paused outside-project tool request.
    pub fn ai_chat_resolve_pending_tool_approval(&mut self, allow: bool, remember: bool) -> bool {
        let pending = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|c| c.pending_tool_approval.take());

        let Some(pending) = pending else {
            return false;
        };

        if !allow {
            self.record_tool_event_summary(
                &pending.tool_call,
                &ToolResult::Error(format!(
                    "user denied outside-project access for '{}'",
                    pending.requested_path.display()
                )),
            );
            let result_content = self.format_tool_result_with_target(
                &pending.tool_call,
                &ToolResult::Error(format!(
                    "user denied outside-project access for '{}'",
                    pending.requested_path.display()
                )),
            );
            if let Some(conv) = self.conversation_mut() {
                conv.append_tool_result(pending.tool_call.id.clone(), result_content);
            }
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.tool_call_count = chat.tool_call_count.saturating_add(1);
                chat.waiting = true;
            }
            self.set_lsp_status("Denied outside-project tool access".to_string());
            return self.execute_tool_call_batch(pending.remaining_tool_calls, pending.model_name);
        }

        if remember {
            if let Some(chat) = self.ai_state.chat.as_mut() {
                let root = normalize_path(&pending.approval_root);
                if !chat
                    .approved_external_roots
                    .iter()
                    .any(|p| normalize_path(p) == root)
                {
                    chat.approved_external_roots.push(root);
                }
            }
        }

        let outcome =
            self.dispatch_tool_call_with_approval(&pending.tool_call, Some(&pending.approval_root));
        match outcome {
            ToolDispatchOutcome::Completed(result) => {
                self.record_tool_event_summary(&pending.tool_call, &result);
                let result_content =
                    self.format_tool_result_with_target(&pending.tool_call, &result);
                if let Some(conv) = self.conversation_mut() {
                    conv.append_tool_result(pending.tool_call.id.clone(), result_content);
                }
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.tool_call_count = chat.tool_call_count.saturating_add(1);
                    chat.waiting = true;
                }
                self.set_lsp_status(format!(
                    "Approved outside-project access: {}",
                    pending.requested_path.display()
                ));
                self.execute_tool_call_batch(pending.remaining_tool_calls, pending.model_name)
            }
            ToolDispatchOutcome::ApprovalRequired(req) => {
                self.pause_for_tool_approval(PendingToolApproval {
                    tool_call: pending.tool_call,
                    remaining_tool_calls: pending.remaining_tool_calls,
                    model_name: pending.model_name,
                    requested_path: req.requested_path.clone(),
                    approval_root: req.approval_root.clone(),
                });
                self.set_lsp_status(req.message);
                true
            }
        }
    }

    /// On first chat open in a no-repo session, ask once whether project tools
    /// may access the current folder as the project boundary.
    pub(crate) fn maybe_prompt_no_repo_session_folder_access_on_chat_open(&mut self) {
        if self.ai_repo_root().is_some() || self.ai_state.no_repo_session_prompted {
            return;
        }
        let Some(folder) = self.ai_no_repo_candidate_root() else {
            return;
        };

        self.ai_state.no_repo_session_prompted = true;
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.pending_no_repo_folder_approval = Some(folder.clone());
        }
        self.set_lsp_status(format!(
            "You're not in a git repo. Allow AI tool access to folder: {}? Press Ctrl-Y to allow, Ctrl-N to deny.",
            folder.display()
        ));
    }

    /// Resolve the first-chat-open no-repo folder access prompt.
    pub fn ai_chat_resolve_pending_no_repo_folder_approval(&mut self, allow: bool) -> bool {
        let pending_folder = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|c| c.pending_no_repo_folder_approval.take());

        let Some(folder) = pending_folder else {
            return false;
        };

        self.ai_state.no_repo_session_prompted = true;
        if allow {
            let root = normalize_path(&folder);
            self.ai_state.no_repo_session_allowed_root = Some(root.clone());
            self.set_lsp_status(format!(
                "Approved AI tool access for folder: {}",
                root.display()
            ));
        } else {
            self.ai_state.no_repo_session_allowed_root = None;
            self.set_lsp_status("Denied no-repo folder tool access".to_string());
        }
        true
    }

    fn execute_tool_call_batch(
        &mut self,
        tool_calls: Vec<ToolCallInfo>,
        model_name: String,
    ) -> bool {
        let max_tool_calls = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| c.opts.profile.as_ref())
            .and_then(|p| self.ai_state.config.resolve_profile(p))
            .map(|p| p.agent_loop.max_tool_calls)
            .unwrap_or(50);

        let mut executed_in_batch: u16 = 0;

        for (idx, tc) in tool_calls.iter().enumerate() {
            let used = self
                .ai_state
                .chat
                .as_ref()
                .map(|c| c.tool_call_count)
                .unwrap_or(0);
            if used.saturating_add(executed_in_batch) >= max_tool_calls {
                if let Some(conv) = self.conversation_mut() {
                    conv.append_error("Tool call iteration limit reached.".to_string());
                }
                self.clear_streaming_state();
                return true;
            }

            match self.dispatch_tool_call_with_approval(tc, None) {
                ToolDispatchOutcome::Completed(result) => {
                    self.record_tool_event_summary(tc, &result);
                    let result_content = self.format_tool_result_with_target(tc, &result);
                    if let Some(conv) = self.conversation_mut() {
                        conv.append_tool_result(tc.id.clone(), result_content);
                    }
                    executed_in_batch = executed_in_batch.saturating_add(1);
                }
                ToolDispatchOutcome::ApprovalRequired(req) => {
                    if let Some(chat) = self.ai_state.chat.as_mut() {
                        chat.tool_call_count =
                            chat.tool_call_count.saturating_add(executed_in_batch);
                    }
                    self.pause_for_tool_approval(PendingToolApproval {
                        tool_call: tc.clone(),
                        remaining_tool_calls: tool_calls[idx + 1..].to_vec(),
                        model_name,
                        requested_path: req.requested_path,
                        approval_root: req.approval_root,
                    });
                    self.set_lsp_status(req.message);
                    return true;
                }
            }
        }

        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.tool_call_count = chat.tool_call_count.saturating_add(executed_in_batch);
            chat.waiting = true;
        }

        if let Err(e) = self.spawn_streaming_request() {
            if let Some(conv) = self.conversation_mut() {
                conv.append_error(format!("Failed to continue: {e}"));
            }
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.waiting = false;
                chat.pending_job = None;
            }
        }

        true
    }

    fn pause_for_tool_approval(&mut self, pending: PendingToolApproval) {
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.pending_tool_approval = Some(pending);
            chat.waiting = false;
            chat.pending_job = None;
            chat.streaming_content = None;
            chat.streaming_thinking = None;
        }
    }

    fn record_tool_event_summary(&mut self, tc: &ToolCallInfo, result: &ToolResult) {
        if tc.id.is_empty() {
            return;
        }
        let summary = self.build_tool_event_summary(tc, result);
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.tool_event_summaries.insert(tc.id.clone(), summary);
        }
    }

    fn format_tool_result_with_target(&self, tc: &ToolCallInfo, result: &ToolResult) -> String {
        let target = tc
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(compact_tool_path)
            .unwrap_or_else(|| self.active_chat_target_display_path());
        let body = match result {
            ToolResult::Success(s) => s.as_str().to_string(),
            ToolResult::Error(s) => format!("Error: {s}"),
        };
        format!("Target: {target}\n{body}")
    }

    fn build_tool_event_summary(&self, tc: &ToolCallInfo, result: &ToolResult) -> ToolEventSummary {
        if let ToolResult::Error(err) = result {
            return ToolEventSummary {
                kind: ToolSummaryKind::Error,
                label: format!("{} {}", tc.name, compact_tool_label(err)),
            };
        }

        let target_path = self.active_chat_target_display_path();
        let explicit_path = tc
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(compact_tool_path);
        let mutation_target = explicit_path.clone().unwrap_or_else(|| target_path.clone());

        let (kind, label) = match tc.name.as_str() {
            "edit_range" => {
                let start = tc
                    .arguments
                    .get("start_line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as usize;
                let end = tc
                    .arguments
                    .get("end_line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(start as u64) as usize;
                let old_lines = end.saturating_sub(start).saturating_add(1);
                let new_lines = tc
                    .arguments
                    .get("new_text")
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        if s.is_empty() {
                            0
                        } else {
                            s.lines().count().max(1)
                        }
                    })
                    .unwrap_or(0);
                let added = new_lines.saturating_sub(old_lines);
                let removed = old_lines.saturating_sub(new_lines);
                (
                    ToolSummaryKind::Mutation,
                    format!("{mutation_target} +{added} -{removed}"),
                )
            }
            "insert_lines" => {
                let added = tc
                    .arguments
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        if s.is_empty() {
                            0
                        } else {
                            s.lines().count().max(1)
                        }
                    })
                    .unwrap_or(0);
                (
                    ToolSummaryKind::Mutation,
                    format!("{mutation_target} +{added} -0"),
                )
            }
            "delete_lines" => {
                let start = tc
                    .arguments
                    .get("start_line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as usize;
                let end = tc
                    .arguments
                    .get("end_line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(start as u64) as usize;
                let removed = end.saturating_sub(start).saturating_add(1);
                (
                    ToolSummaryKind::Mutation,
                    format!("{mutation_target} +0 -{removed}"),
                )
            }
            "write_file_at_path" => {
                let written = tc
                    .arguments
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        if s.is_empty() {
                            0
                        } else {
                            s.lines().count().max(1)
                        }
                    })
                    .unwrap_or(0);
                (
                    ToolSummaryKind::Mutation,
                    format!("{mutation_target} +{written} -*"),
                )
            }
            "create_file" => {
                let written = tc
                    .arguments
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        if s.is_empty() {
                            0
                        } else {
                            s.lines().count().max(1)
                        }
                    })
                    .unwrap_or(0);
                (
                    ToolSummaryKind::Mutation,
                    format!("{mutation_target} +{written} -0"),
                )
            }
            "open_file" => {
                let path = tc
                    .arguments
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(compact_tool_path)
                    .unwrap_or_else(|| target_path.clone());
                let line = tc
                    .arguments
                    .get("line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1);
                (ToolSummaryKind::Navigation, format!("{path}:{line}"))
            }
            "select_text" => {
                let start = tc
                    .arguments
                    .get("start_line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1);
                let end = tc
                    .arguments
                    .get("end_line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(start);
                (
                    ToolSummaryKind::Navigation,
                    format!("{target_path}:{start}-{end}"),
                )
            }
            "read_file_at_path" => {
                let path = tc
                    .arguments
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(compact_tool_path)
                    .unwrap_or_else(|| target_path.clone());
                let range = tool_line_range_suffix(&tc.arguments);
                (ToolSummaryKind::Read, format!("{path}{range}"))
            }
            "read_file" => {
                let range = tool_line_range_suffix(&tc.arguments);
                (ToolSummaryKind::Read, format!("{target_path}{range}"))
            }
            "list_files" => {
                let dir = tc
                    .arguments
                    .get("path")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(compact_tool_path)
                    .unwrap_or_else(|| ".".to_string());
                let count = tool_result_success(result)
                    .and_then(|s| first_number_in_text(s.lines().next().unwrap_or("")));
                let label = match count {
                    Some(n) => format!("{dir} {n} files"),
                    None => format!("{dir} files"),
                };
                (ToolSummaryKind::Search, label)
            }
            "search_project" => {
                let query = tc
                    .arguments
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let count = tool_result_success(result)
                    .and_then(|s| first_number_in_text(s.lines().next().unwrap_or("")));
                let label = match count {
                    Some(n) => format!("\"{}\" {n} matches", compact_tool_label(query)),
                    None => format!("\"{}\" search", compact_tool_label(query)),
                };
                (ToolSummaryKind::Search, label)
            }
            "read_diagnostics" => {
                let success = tool_result_success(result).unwrap_or_default();
                if success.starts_with("No diagnostics.") {
                    (
                        ToolSummaryKind::Diagnostics,
                        "diagnostics E0 W0".to_string(),
                    )
                } else {
                    let errors = success.matches("[error]").count();
                    let warnings = success.matches("[warning]").count();
                    (
                        ToolSummaryKind::Diagnostics,
                        format!("diagnostics E{errors} W{warnings}"),
                    )
                }
            }
            "read_project_diagnostics" => {
                let success = tool_result_success(result).unwrap_or_default();
                let summary = success
                    .lines()
                    .next()
                    .unwrap_or("project diagnostics")
                    .to_string();
                (ToolSummaryKind::Diagnostics, summary)
            }
            "snapshot_file" => (
                ToolSummaryKind::Other,
                format!("snapshot {}", mutation_target),
            ),
            "restore_file" => (
                ToolSummaryKind::Mutation,
                format!("{} restored", mutation_target),
            ),
            "document_symbols" | "hover" | "goto_definition" => {
                (ToolSummaryKind::Read, tc.name.clone())
            }
            _ => (ToolSummaryKind::Other, tc.name.clone()),
        };

        ToolEventSummary {
            kind,
            label: compact_tool_label(&label),
        }
    }

    fn active_chat_target_display_path(&self) -> String {
        let path = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| self.get_buffer_by_id(c.active_buffer_id))
            .and_then(|b| b.file_path())
            .map(PathBuf::from)
            .or_else(|| self.buffer().file_path().map(PathBuf::from));

        let Some(path) = path else {
            return "[No Name]".to_string();
        };
        let absolute = self.absolutize_path(&path);
        if let Some(root) = self.ai_effective_project_root() {
            let rel = to_relative_path_for_boundary(&absolute, &root);
            return compact_tool_path(&rel);
        }
        compact_tool_path(&absolute.display().to_string())
    }

    fn execute_read_file_at_path_tool(
        &mut self,
        tc: &ToolCallInfo,
        approved_once_root: Option<&PathBuf>,
    ) -> ToolDispatchOutcome {
        let Some(raw_path) = tc.arguments.get("path").and_then(|v| v.as_str()) else {
            return ToolDispatchOutcome::Completed(ToolResult::Error(
                "'path' parameter is required and must be non-empty".to_string(),
            ));
        };
        if raw_path.is_empty() {
            return ToolDispatchOutcome::Completed(ToolResult::Error(
                "'path' parameter is required and must be non-empty".to_string(),
            ));
        }

        let resolution = match self.resolve_tool_path_policy(
            raw_path,
            false,
            "read_file_at_path",
            approved_once_root,
        ) {
            Ok(r) => r,
            Err(e) => return ToolDispatchOutcome::Completed(ToolResult::Error(e)),
        };

        let (absolute_path, boundary_root) = match resolution {
            ToolPathResolution::Allowed {
                absolute_path,
                boundary_root,
            } => (absolute_path, boundary_root),
            ToolPathResolution::NeedsApproval(req) => {
                return ToolDispatchOutcome::ApprovalRequired(req)
            }
        };

        let rel_path = to_relative_path_for_boundary(&absolute_path, &boundary_root);
        let mut patched_call = tc.clone();
        if let Some(obj) = patched_call.arguments.as_object_mut() {
            obj.insert("path".to_string(), json!(rel_path));
        } else {
            return ToolDispatchOutcome::Completed(ToolResult::Error(
                "tool arguments must be an object".to_string(),
            ));
        }

        let mut ctx = self.build_tool_execution_context();
        ctx.scope_context.project_root = Some(boundary_root);
        let result = self.execute_tool_call(&patched_call, &ctx);
        ToolDispatchOutcome::Completed(result)
    }

    fn execute_list_files_tool(
        &mut self,
        tc: &ToolCallInfo,
        approved_once_root: Option<&PathBuf>,
    ) -> ToolDispatchOutcome {
        let mut patched_call = tc.clone();
        let boundary_root =
            if let Some(raw_path) = tc.arguments.get("path").and_then(|v| v.as_str()) {
                if raw_path.is_empty() {
                    match self.ai_effective_project_root() {
                        Some(root) => root,
                        None => {
                            return ToolDispatchOutcome::Completed(ToolResult::Error(
                                self.no_project_root_error(),
                            ))
                        }
                    }
                } else {
                    let resolution = match self.resolve_tool_path_policy(
                        raw_path,
                        true,
                        "list_files",
                        approved_once_root,
                    ) {
                        Ok(r) => r,
                        Err(e) => return ToolDispatchOutcome::Completed(ToolResult::Error(e)),
                    };
                    let (absolute_path, boundary_root) = match resolution {
                        ToolPathResolution::Allowed {
                            absolute_path,
                            boundary_root,
                        } => (absolute_path, boundary_root),
                        ToolPathResolution::NeedsApproval(req) => {
                            return ToolDispatchOutcome::ApprovalRequired(req)
                        }
                    };
                    let rel_path = to_relative_path_for_boundary(&absolute_path, &boundary_root);
                    if let Some(obj) = patched_call.arguments.as_object_mut() {
                        obj.insert("path".to_string(), json!(rel_path));
                    } else {
                        return ToolDispatchOutcome::Completed(ToolResult::Error(
                            "tool arguments must be an object".to_string(),
                        ));
                    }
                    boundary_root
                }
            } else {
                match self.ai_effective_project_root() {
                    Some(root) => root,
                    None => {
                        return ToolDispatchOutcome::Completed(ToolResult::Error(
                            self.no_project_root_error(),
                        ))
                    }
                }
            };

        let mut ctx = self.build_tool_execution_context();
        ctx.scope_context.project_root = Some(boundary_root);
        let result = self.execute_tool_call(&patched_call, &ctx);
        ToolDispatchOutcome::Completed(result)
    }

    fn execute_open_file_tool(
        &mut self,
        tc: &ToolCallInfo,
        approved_once_root: Option<&PathBuf>,
    ) -> ToolDispatchOutcome {
        let Some(raw_path) = tc.arguments.get("path").and_then(|v| v.as_str()) else {
            return ToolDispatchOutcome::Completed(ToolResult::Error(
                "'path' is required".to_string(),
            ));
        };
        if raw_path.is_empty() {
            return ToolDispatchOutcome::Completed(ToolResult::Error(
                "'path' is required".to_string(),
            ));
        }
        let create = tc
            .arguments
            .get("create")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let caps = self.build_chat_capabilities();
        let Some(tool_def) = self.ai_state.tool_registry.get("open_file") else {
            return ToolDispatchOutcome::Completed(ToolResult::Error(
                "unknown tool: open_file".into(),
            ));
        };
        if !caps.contains(&tool_def.required_scope) {
            return ToolDispatchOutcome::Completed(ToolResult::Error(
                "tool 'open_file' requires scope not granted by current context".to_string(),
            ));
        }

        let resolution =
            match self.resolve_tool_path_policy(raw_path, false, "open_file", approved_once_root) {
                Ok(r) => r,
                Err(e) => return ToolDispatchOutcome::Completed(ToolResult::Error(e)),
            };
        let absolute_path = match resolution {
            ToolPathResolution::Allowed { absolute_path, .. } => absolute_path,
            ToolPathResolution::NeedsApproval(req) => {
                return ToolDispatchOutcome::ApprovalRequired(req)
            }
        };

        ToolDispatchOutcome::Completed(self.handle_open_file_at_absolute_path(
            &absolute_path,
            &tc.arguments,
            create,
        ))
    }

    fn execute_path_scoped_mutation_tool(
        &mut self,
        tc: &ToolCallInfo,
        approved_once_root: Option<&PathBuf>,
    ) -> ToolDispatchOutcome {
        let name = tc.name.as_str();
        let requires_path = matches!(
            name,
            "write_file_at_path" | "create_file" | "snapshot_file" | "restore_file"
        );

        let raw_path = tc
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if requires_path && raw_path.is_none() {
            return ToolDispatchOutcome::Completed(ToolResult::Error(
                "'path' is required".to_string(),
            ));
        }

        if let Some(raw_path) = raw_path {
            let caps = self.build_chat_capabilities();
            if caps.file_scope < crate::ai::FileScope::Project {
                return ToolDispatchOutcome::Completed(ToolResult::Error(
                    "path parameter requires project file scope".to_string(),
                ));
            }

            let resolution =
                match self.resolve_tool_path_policy(raw_path, false, name, approved_once_root) {
                    Ok(r) => r,
                    Err(e) => return ToolDispatchOutcome::Completed(ToolResult::Error(e)),
                };
            let absolute_path = match resolution {
                ToolPathResolution::Allowed { absolute_path, .. } => absolute_path,
                ToolPathResolution::NeedsApproval(req) => {
                    return ToolDispatchOutcome::ApprovalRequired(req)
                }
            };

            if name == "create_file" && absolute_path.exists() {
                return ToolDispatchOutcome::Completed(ToolResult::Error(format!(
                    "'{}' already exists. Use write_file_at_path to overwrite.",
                    absolute_path.display()
                )));
            }

            let allow_create =
                matches!(name, "write_file_at_path" | "create_file" | "restore_file");
            if let Err(e) =
                self.ensure_mutation_target_buffer_for_path(&absolute_path, allow_create)
            {
                return ToolDispatchOutcome::Completed(ToolResult::Error(e));
            }
        }

        ToolDispatchOutcome::Completed(self.execute_mutation_tool(&tc.name, &tc.arguments))
    }

    fn ensure_mutation_target_buffer_for_path(
        &mut self,
        absolute_path: &Path,
        allow_create: bool,
    ) -> std::result::Result<(), String> {
        let normalized_target = normalize_path(absolute_path);

        if let Some(index) = self.buffers.iter().position(|buffer| {
            buffer
                .file_path()
                .map(|p| normalize_path(Path::new(p)) == normalized_target)
                .unwrap_or(false)
        }) {
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.active_buffer_id = self.buffers[index].id();
            }
            return Ok(());
        }

        if absolute_path.exists() {
            if !absolute_path.is_file() {
                return Err(format!(
                    "'{}' is not a file. Use list_files to inspect the directory.",
                    absolute_path.display()
                ));
            }
            let buffer = crate::buffer::Buffer::load_file(absolute_path)
                .map_err(|e| format!("failed to open '{}': {}", absolute_path.display(), e))?;
            self.buffers.push(buffer);
            self.lsp_state.needs_lsp_init = true;
            let idx = self.buffers.len().saturating_sub(1);
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.active_buffer_id = self.buffers[idx].id();
            }
            return Ok(());
        }

        if !allow_create {
            return Err(format!(
                "'{}' does not exist. Create it first with create_file or write_file_at_path.",
                absolute_path.display()
            ));
        }

        let Some(parent) = absolute_path.parent() else {
            return Err(format!(
                "cannot create '{}': invalid target path",
                absolute_path.display()
            ));
        };
        if !parent.exists() || !parent.is_dir() {
            return Err(format!(
                "cannot create '{}': parent directory '{}' does not exist",
                absolute_path.display(),
                parent.display()
            ));
        }

        let mut buffer = crate::buffer::Buffer::new();
        buffer.set_file_path(absolute_path.to_string_lossy().to_string());
        self.buffers.push(buffer);
        self.lsp_state.needs_lsp_init = true;
        let idx = self.buffers.len().saturating_sub(1);
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.active_buffer_id = self.buffers[idx].id();
        }
        Ok(())
    }

    fn handle_open_file_at_absolute_path(
        &mut self,
        absolute_path: &Path,
        args: &serde_json::Value,
        create: bool,
    ) -> ToolResult {
        if !absolute_path.exists() {
            if !create {
                return ToolResult::Error(format!(
                    "'{}' is not a file. Use list_files to see available files.",
                    absolute_path.display()
                ));
            }
            let Some(parent) = absolute_path.parent() else {
                return ToolResult::Error(format!(
                    "cannot create '{}': invalid path",
                    absolute_path.display()
                ));
            };
            if !parent.exists() || !parent.is_dir() {
                return ToolResult::Error(format!(
                    "cannot create '{}': parent directory '{}' does not exist",
                    absolute_path.display(),
                    parent.display()
                ));
            }
            let mut buffer = crate::buffer::Buffer::new();
            buffer.set_file_path(absolute_path.to_string_lossy().to_string());
            self.add_buffer(buffer);
        } else if !absolute_path.is_file() {
            return ToolResult::Error(format!(
                "'{}' is not a file. Use list_files to see available files.",
                absolute_path.display()
            ));
        } else if let Err(e) = self.open_file(absolute_path) {
            return ToolResult::Error(format!(
                "failed to open '{}': {}",
                absolute_path.display(),
                e
            ));
        }

        let line = args
            .get("line")
            .and_then(|v| v.as_u64())
            .map(|n| n.saturating_sub(1) as usize)
            .unwrap_or(0);
        let col = args
            .get("column")
            .and_then(|v| v.as_u64())
            .map(|n| n.saturating_sub(1) as usize)
            .unwrap_or(0);

        let max_line = self.buffer().rope().len_lines().saturating_sub(1);
        let target_line = line.min(max_line);
        self.buffer_mut()
            .cursor_mut()
            .set_position(target_line, col);
        self.buffer_mut().validate_cursor_position();
        self.center_cursor_in_viewport();

        let opened_buffer_id = self.buffer().id();
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.active_buffer_id = opened_buffer_id;
        }

        let actual_line = self.buffer().cursor().line() + 1;
        let actual_col = self.buffer().cursor().col() + 1;
        let total_lines = self.buffer().rope().len_lines();
        ToolResult::Success(format!(
            "Opened {} at line {}, column {} ({} lines total).",
            absolute_path.display(),
            actual_line,
            actual_col,
            total_lines
        ))
    }

    fn resolve_tool_path_policy(
        &self,
        raw_path: &str,
        treat_as_directory: bool,
        tool_name: &str,
        approved_once_root: Option<&PathBuf>,
    ) -> std::result::Result<ToolPathResolution, String> {
        let boundary_root = self
            .ai_effective_project_root()
            .ok_or_else(|| self.no_project_root_error())?;
        let boundary_root = normalize_path(&boundary_root);

        let requested_path = {
            let path = Path::new(raw_path);
            if path.is_absolute() {
                self.absolutize_path(path)
            } else {
                let joined = boundary_root.join(path);
                joined
                    .canonicalize()
                    .unwrap_or_else(|_| normalize_path(&joined))
            }
        };

        if requested_path.starts_with(&boundary_root) {
            return Ok(ToolPathResolution::Allowed {
                absolute_path: requested_path,
                boundary_root,
            });
        }

        if let Some(root) = approved_once_root {
            let root = normalize_path(root);
            if requested_path.starts_with(&root) {
                return Ok(ToolPathResolution::Allowed {
                    absolute_path: requested_path,
                    boundary_root: root,
                });
            }
        }

        if let Some(root) = self.current_session_approved_root_for(&requested_path) {
            return Ok(ToolPathResolution::Allowed {
                absolute_path: requested_path,
                boundary_root: root,
            });
        }

        let approval_root = if treat_as_directory {
            requested_path.clone()
        } else {
            requested_path
                .parent()
                .map(normalize_path)
                .unwrap_or_else(|| requested_path.clone())
        };

        Ok(ToolPathResolution::NeedsApproval(ToolApprovalRequest {
            requested_path: requested_path.clone(),
            approval_root: approval_root.clone(),
            message: format!(
                "Approval required: {} wants outside-project access to {}. Press Ctrl-Y to allow once, Ctrl-A to allow for this chat session, Ctrl-N to deny.",
                tool_name,
                requested_path.display()
            ),
        }))
    }

    fn current_session_approved_root_for(&self, path: &Path) -> Option<PathBuf> {
        let chat = self.ai_state.chat.as_ref()?;
        for root in &chat.approved_external_roots {
            let root = normalize_path(root);
            if path.starts_with(&root) {
                return Some(root);
            }
        }
        None
    }

    /// Effective project boundary for AI project-level tools.
    ///
    /// Prefers git repository root. Outside git, falls back to a
    /// session-approved folder root.
    pub(crate) fn ai_effective_project_root(&self) -> Option<PathBuf> {
        self.ai_repo_root().or_else(|| {
            self.ai_state
                .no_repo_session_allowed_root
                .as_ref()
                .map(|p| normalize_path(p))
        })
    }

    fn ai_project_start_path(&self) -> Option<PathBuf> {
        let origin_file = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| self.get_buffer_by_id(chat.origin_buffer_id))
            .and_then(|buf| buf.file_path())
            .map(PathBuf::from);
        let current_file = self.buffer().file_path().map(PathBuf::from);

        if let Some(file) = origin_file.or(current_file) {
            Some(self.absolutize_path(&file))
        } else {
            std::env::current_dir().ok()
        }
    }

    fn ai_no_repo_candidate_root(&self) -> Option<PathBuf> {
        let start = self.ai_project_start_path()?;
        if start.is_dir() {
            Some(normalize_path(&start))
        } else {
            start.parent().map(normalize_path)
        }
    }

    fn no_project_root_error(&self) -> String {
        "No project boundary available. You're not in a git repo and no folder access was approved for this session.".to_string()
    }

    /// Repository root for AI project-level tools.
    ///
    /// Resolves from current file (if available) or current working directory.
    pub(crate) fn ai_repo_root(&self) -> Option<PathBuf> {
        let start = self.ai_project_start_path()?;
        let mut dir = if start.is_dir() {
            start
        } else {
            start.parent()?.to_path_buf()
        };

        loop {
            if dir.join(".git").exists() {
                return Some(normalize_path(&dir));
            }
            if !dir.pop() {
                break;
            }
        }
        None
    }

    fn absolutize_path(&self, path: &Path) -> PathBuf {
        let joined = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        };
        joined
            .canonicalize()
            .unwrap_or_else(|_| normalize_path(&joined))
    }

    // -----------------------------------------------------------------
    // Editor state context (injected into system prompt)
    // -----------------------------------------------------------------

    /// Build a structured editor state block for the system prompt.
    ///
    /// Assembles context in priority order within `budget_chars`:
    /// file info, cursor, enclosing scope, selection, viewport code, diagnostics.
    pub(crate) fn build_editor_state_context(&self, budget_chars: usize) -> String {
        let buf = &self.buffers[self.current_buffer_index];
        let mut out = String::with_capacity(budget_chars.min(16384));
        let mut remaining = budget_chars;

        out.push_str("## Editor state\n\n");
        remaining = remaining.saturating_sub(out.len());

        // --- File info (always) ---
        let file_info = match buf.file_path() {
            Some(path) => {
                let lang =
                    crate::syntax::LanguageRegistry::get_lsp_language_id(path).unwrap_or("unknown");
                let total_lines = buf.rope().len_lines();
                let modified = if buf.is_modified() { ", modified" } else { "" };
                format!(
                    "File: {} ({}) — {} lines{}\n",
                    path, lang, total_lines, modified
                )
            }
            None => {
                out.push_str("No file open.\n");
                out.push_str(&format!("{}\n", self.no_file_open_guidance()));
                return out;
            }
        };
        out.push_str(&file_info);
        remaining = remaining.saturating_sub(file_info.len());

        // --- Cursor position (always) ---
        let cursor = buf.cursor();
        let cursor_line = format!(
            "Cursor: line {}, col {}\n",
            cursor.line() + 1,
            cursor.col() + 1
        );
        out.push_str(&cursor_line);
        remaining = remaining.saturating_sub(cursor_line.len());

        // --- Enclosing scope (if LSP symbols available) ---
        if let Some(sym) = find_enclosing_symbol(
            &self.lsp_state.available_document_symbols,
            cursor.line() as u32,
        ) {
            let kind = symbol_kind_label(sym.kind);
            let start = sym.range.start.line + 1;
            let end = sym.range.end.line + 1;
            let scope_line = format!(
                "Enclosing: {} {} (lines {}-{})\n",
                kind, sym.name, start, end
            );
            if scope_line.len() <= remaining {
                out.push_str(&scope_line);
                remaining = remaining.saturating_sub(scope_line.len());
            }
        }

        // --- Selection (if any, high priority) ---
        if let Some(sel) = &self.ai_state.active_selection {
            let sel_header = format!(
                "\n### Selection (lines {}-{})\n",
                sel.start_line + 1,
                sel.end_line + 1,
            );
            let sel_text = &sel.selected_text;
            let sel_block = format!("{}{}\n", sel_header, sel_text);
            if sel_block.len() <= remaining {
                out.push_str(&sel_block);
                remaining = remaining.saturating_sub(sel_block.len());
            }
        }

        // --- Viewport code (main budget consumer) ---
        let rope = buf.rope();
        let total_lines = rope.len_lines();
        let vp_start = self.viewport.scroll_offset;
        let vp_height = self.viewport.viewport_height.max(1);
        let vp_end = (vp_start + vp_height).min(total_lines);

        if vp_start < vp_end && remaining > 100 {
            // Calculate line number width for formatting
            let num_width = format!("{}", vp_end).len();

            // Estimate how many lines we can fit in the remaining budget
            // Reserve some space for the header and diagnostics (~200 chars)
            let code_budget = remaining.saturating_sub(200);
            let mut code_lines = Vec::new();
            let mut code_len = 0;

            // If viewport is too large for budget, center on cursor
            let (render_start, render_end) = if vp_start <= cursor.line() && cursor.line() < vp_end
            {
                (vp_start, vp_end)
            } else {
                // Cursor outside viewport (shouldn't happen often) — use viewport as-is
                (vp_start, vp_end)
            };

            let mut truncated_before = 0usize;
            let mut truncated_after = 0usize;

            for line_idx in render_start..render_end {
                let line_content = rope.line(line_idx).to_string();
                // Trim trailing newline from ropey line
                let line_content = line_content.trim_end_matches('\n');
                let formatted = format!(
                    "{:>width$} | {}\n",
                    line_idx + 1,
                    line_content,
                    width = num_width
                );
                if code_len + formatted.len() > code_budget {
                    truncated_after = render_end - line_idx;
                    break;
                }
                code_len += formatted.len();
                code_lines.push(formatted);
            }

            // If we couldn't fit from the start, center on cursor
            if truncated_after > 0 && cursor.line() >= render_start {
                // Try centering on cursor
                let half = code_lines.len() / 2;
                let cursor_offset = cursor.line().saturating_sub(render_start);
                if cursor_offset > half {
                    let skip = cursor_offset - half;
                    truncated_before = skip;
                    code_lines = code_lines[skip..].to_vec();
                }
            }

            let header = format!(
                "\n### Visible code (lines {}-{})\n",
                render_start + 1 + truncated_before,
                render_start + truncated_before + code_lines.len(),
            );
            if header.len() + code_len <= remaining {
                out.push_str(&header);
                if truncated_before > 0 {
                    out.push_str(&format!(
                        "[... {} more lines above ...]\n",
                        truncated_before
                    ));
                }
                for line in &code_lines {
                    out.push_str(line);
                }
                if truncated_after > 0 {
                    out.push_str(&format!("[... {} more lines below ...]\n", truncated_after));
                }
                remaining = remaining.saturating_sub(header.len() + code_len);
            }
        }

        // --- Diagnostics on visible lines (if budget remains) ---
        if remaining > 50 {
            let diags = self.all_diagnostics();
            let vp_diags: Vec<_> = diags
                .iter()
                .filter(|d| {
                    let line = d.range.start.line as usize;
                    line >= vp_start && line < vp_end
                })
                .collect();

            if !vp_diags.is_empty() {
                let mut diag_section =
                    format!("\n### Diagnostics ({} on visible lines)\n", vp_diags.len());
                for d in &vp_diags {
                    let severity = match d.severity {
                        Some(lsp_types::DiagnosticSeverity::ERROR) => "Error",
                        Some(lsp_types::DiagnosticSeverity::WARNING) => "Warning",
                        Some(lsp_types::DiagnosticSeverity::INFORMATION) => "Info",
                        Some(lsp_types::DiagnosticSeverity::HINT) => "Hint",
                        _ => "Unknown",
                    };
                    let line = format!(
                        "Line {}: [{}] {}\n",
                        d.range.start.line + 1,
                        severity,
                        d.message,
                    );
                    if diag_section.len() + line.len() > remaining {
                        break;
                    }
                    diag_section.push_str(&line);
                }
                out.push_str(&diag_section);
            }
        }

        out
    }

    // -----------------------------------------------------------------
    // LSP tool dispatch
    // -----------------------------------------------------------------

    /// Execute an LSP-backed tool (document_symbols, hover, goto_definition).
    pub(crate) fn execute_lsp_tool(&self, name: &str, args: &serde_json::Value) -> ToolResult {
        let target_index = self.active_chat_target_buffer_index();
        let buf = &self.buffers[target_index];
        let Some(file_path) = buf.file_path() else {
            return ToolResult::Error(self.no_file_open_guidance());
        };
        let language_id = crate::syntax::LanguageRegistry::get_lsp_language_id(file_path)
            .unwrap_or("unknown")
            .to_string();
        let Some(uri) = crate::lsp::uri_from_file_path(file_path) else {
            return ToolResult::Error(format!("Cannot create URI for path: {}", file_path));
        };

        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => Arc::clone(lsp),
            None => {
                // Fall back to cached data for document_symbols
                if name == "document_symbols" {
                    return format_document_symbols_cached(
                        &self.lsp_state.available_document_symbols,
                    );
                }
                return ToolResult::Error(
                    "LSP not available. The language server is not running for this file."
                        .to_string(),
                );
            }
        };

        // Clone cached symbols for fallback
        let cached_symbols = self.lsp_state.available_document_symbols.clone();

        match name {
            "document_symbols" => {
                handle_lsp_document_symbols(lsp, uri, language_id, cached_symbols)
            }
            "hover" => handle_lsp_hover(lsp, uri, language_id, args),
            "goto_definition" => handle_lsp_goto_definition(lsp, uri, language_id, args),
            _ => ToolResult::Error(format!("unknown LSP tool: {name}")),
        }
    }

    /// Build a context-aware system prompt for chat mode.
    ///
    /// This ensures the model responds in natural language instead of falling
    /// back to the profile's editing system prompt (which asks for JSON).
    fn build_chat_system_prompt(&self, profile: &crate::ai::AiProfileConfig) -> String {
        let caps = self.build_chat_capabilities();
        let tools = self
            .ai_state
            .tool_registry
            .tools_for_profile(profile, &caps);

        let allow_edits = self
            .ai_state
            .chat
            .as_ref()
            .map(|c| c.allow_edits)
            .unwrap_or(false);

        let mut prompt = String::from(
            "You are an expert developer embedded in the ovim code editor.\n\
             Respond in natural language. Do NOT return raw JSON.\n\n",
        );

        // Group tools by purpose
        if !tools.is_empty() {
            prompt.push_str("## Available tools\n\n");

            let read_tools: Vec<&str> = tools
                .iter()
                .filter(|t| t.side_effect == SideEffect::Read)
                .map(|t| t.name.as_str())
                .collect();
            if !read_tools.is_empty() {
                prompt.push_str(&format!("Read: {}\n", read_tools.join(", ")));
            }

            let nav_tools: Vec<&str> = tools
                .iter()
                .filter(|t| t.side_effect == SideEffect::Navigation)
                .map(|t| t.name.as_str())
                .collect();
            if !nav_tools.is_empty() {
                prompt.push_str(&format!("Navigate: {}\n", nav_tools.join(", ")));
            }

            if allow_edits {
                let mutation_tools: Vec<&str> = tools
                    .iter()
                    .filter(|t| t.side_effect == SideEffect::Mutation)
                    .map(|t| t.name.as_str())
                    .collect();
                if !mutation_tools.is_empty() {
                    prompt.push_str(&format!("Edit: {}\n", mutation_tools.join(", ")));
                }
            }

            prompt.push_str(
                "\n## How to work\n\n\
                 - The visible code is shown in \"Editor state\" below. Do NOT call read_file for those lines.\n\
                 - Use read_file only for lines outside the visible range or other files.\n\
                 - Explore FIRST: When asked about a project, start with list_files, then read key files.\n\
                 - Show, don't just tell: Use open_file to navigate to relevant code and select_text to highlight specific regions.\n\
                 - Read before write: Always read relevant code before making edits. Never assume file contents.\n\
                 - Verify after edit: After editing, use read_diagnostics to check for new errors.\n\
                 - Bottom-up editing: When making multiple edits to the same file, edit from bottom to top so line numbers remain valid.\n\n",
            );

            if !self.active_chat_target_has_file_path() {
                prompt.push_str(
                    "- No file is currently open. First call open_file(path) or open_file(path, create=true).\n\
                     - If the path is unknown, ask the user to open/select a file first.\n\n",
                );
            }
        }

        prompt
    }

    /// Resolve profile, collect messages, and spawn a streaming AI request.
    ///
    /// Shared by `submit_ai_chat_message` (initial send) and `process_tool_calls`
    /// (continuation after tool execution). Returns an error if no chat is active
    /// or the profile can't be resolved.
    pub(crate) fn spawn_streaming_request(&mut self) -> Result<()> {
        let chat = self
            .ai_state
            .chat
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no active chat session"))?;

        let profile_name = chat
            .opts
            .profile
            .clone()
            .unwrap_or_else(|| self.ai_state.active_profile.clone());
        let profile = self
            .ai_state
            .config
            .resolve_profile(&profile_name)
            .ok_or_else(|| anyhow::anyhow!("No AI profile '{}' configured", profile_name))?
            .clone();

        let model_name = profile.model.clone();

        // Resolution chain for chat system prompt:
        // 1. chat.opts.system_prompt (per-session override)
        // 2. profile.chat_prompt (per-profile, interpolated)
        // 3. config.prompts["chat"] (global template, interpolated)
        // 4. build_chat_system_prompt() (hardcoded fallback)
        let system_prompt = if let Some(ref sp) = chat.opts.system_prompt {
            Some(sp.clone())
        } else {
            let buf = &self.buffers[self.current_buffer_index];
            let file = buf.file_path().unwrap_or("[No Name]");
            let language = buf
                .file_path()
                .and_then(crate::syntax::LanguageRegistry::get_lsp_language_id)
                .unwrap_or("unknown");
            crate::ai::resolve_chat_system_prompt(
                &profile,
                &self.ai_state.config.prompts,
                file,
                language,
            )
            .or_else(|| Some(self.build_chat_system_prompt(&profile)))
        };
        // Append project context to system prompt
        let project_ctx = crate::ai::project_context::load_project_context(
            &self.ai_state.config.project_context,
            self.buffers[self.current_buffer_index].file_path(),
        );
        let system_prompt =
            system_prompt.map(|sp| crate::ai::append_project_context(&sp, &project_ctx));
        // Append editor state (viewport, cursor, diagnostics) regardless of prompt source
        let editor_state = self.build_editor_state_context(8000);
        let system_prompt = system_prompt.map(|sp| format!("{sp}\n\n{editor_state}"));
        let tool_schemas = self.build_tool_schemas_for_chat(&profile);
        let api_key_registry = self.ai_state.config.api_key_registry.clone();

        let messages: Vec<ChatMessage> = self
            .conversation()
            .map(|c| c.messages().to_vec())
            .unwrap_or_default();

        // Apply observation masking — only the API-bound copy gets masked;
        // the full conversation stays in ConversationTree for UI display.
        let messages = crate::ai::chat_types::apply_observation_mask(
            &messages,
            &self.ai_state.config.chat_context,
        );

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let tx_err = tx.clone();
        let task = tokio::spawn(async move {
            let tools_ref = if tool_schemas.is_empty() {
                None
            } else {
                Some(tool_schemas.as_slice())
            };
            if let Err(e) = stream_ai_chat(
                &profile,
                &messages,
                system_prompt.as_deref(),
                tools_ref,
                tx.clone(),
                &api_key_registry,
            )
            .await
            {
                let _ = tx_err.send(StreamChunk::Error(e.to_string()));
            }
        });

        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.pending_job = Some(PendingAiChatJob {
                receiver: rx,
                task,
                profile_name,
                model_name,
            });
            chat.pending_tool_approval = None;
            chat.streaming_content = Some(String::new());
            chat.streaming_thinking = None;
            chat.streaming_tool_calls.clear();
        }

        Ok(())
    }

    /// Get diagnostics for a specific buffer index.
    fn get_diagnostics_for_buffer_index(
        &self,
        buffer_index: usize,
    ) -> Vec<crate::ai::DiagnosticFact> {
        if buffer_index == self.current_buffer_index {
            return self
                .all_diagnostics()
                .iter()
                .map(|d| crate::ai::DiagnosticFact {
                    message: d.message.clone(),
                    severity: d.severity.map(|s| format!("{:?}", s)),
                    line: d.range.start.line,
                    start_character: d.range.start.character,
                    end_character: d.range.end.character,
                })
                .collect();
        }

        let Some(path) = self
            .buffers
            .get(buffer_index)
            .and_then(|buf| buf.file_path())
            .map(PathBuf::from)
        else {
            return Vec::new();
        };
        let Some(lsp) = self.lsp_state.lsp_manager.as_ref() else {
            return Vec::new();
        };
        let Some(uri) = crate::lsp::uri_from_file_path(&path) else {
            return Vec::new();
        };
        let Some(handle) = tokio::runtime::Handle::try_current().ok() else {
            return Vec::new();
        };

        tokio::task::block_in_place(|| {
            handle
                .block_on(async { lsp.get_diagnostics(&uri).await })
                .into_iter()
                .map(|d| crate::ai::DiagnosticFact {
                    message: d.message,
                    severity: d.severity.map(|s| format!("{:?}", s)),
                    line: d.range.start.line,
                    start_character: d.range.start.character,
                    end_character: d.range.end.character,
                })
                .collect()
        })
    }

    fn get_project_diagnostics_for_chat(&self) -> Vec<ProjectDiagnosticFile> {
        let Some(lsp) = self.lsp_state.lsp_manager.as_ref() else {
            return Vec::new();
        };
        let Some(handle) = tokio::runtime::Handle::try_current().ok() else {
            return Vec::new();
        };

        let project_root = self.ai_effective_project_root();
        tokio::task::block_in_place(|| {
            let all = handle.block_on(async { lsp.list_all_diagnostics().await });
            let mut out = Vec::new();
            for (uri, diagnostics) in all {
                let Some(path) = crate::lsp::uri_to_file_path(&uri) else {
                    continue;
                };
                let path_label = if let Some(root) = project_root.as_ref() {
                    if !path.starts_with(root) {
                        continue;
                    }
                    to_relative_path_for_boundary(&path, root)
                } else {
                    path.to_string_lossy().to_string()
                };
                let facts = diagnostics
                    .into_iter()
                    .map(|d| crate::ai::DiagnosticFact {
                        message: d.message,
                        severity: d.severity.map(|s| format!("{:?}", s)),
                        line: d.range.start.line,
                        start_character: d.range.start.character,
                        end_character: d.range.end.character,
                    })
                    .collect::<Vec<_>>();
                if !facts.is_empty() {
                    out.push(ProjectDiagnosticFile {
                        path: path_label,
                        diagnostics: facts,
                    });
                }
            }
            out.sort_by(|a, b| a.path.cmp(&b.path));
            out
        })
    }

    /// Clear all streaming state and mark the chat as no longer waiting.
    pub(crate) fn clear_streaming_state(&mut self) {
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.waiting = false;
            chat.pending_job = None;
            chat.pending_tool_approval = None;
            chat.streaming_content = None;
            chat.streaming_thinking = None;
            if chat.viewport.follow_latest {
                chat.viewport.row_scroll_from_bottom = 0;
                chat.viewport.pinned_base_total_rows = None;
                chat.history.selected_node_id = None;
            }
        }
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

fn to_relative_path_for_boundary(path: &Path, boundary_root: &Path) -> String {
    let rel = path.strip_prefix(boundary_root).unwrap_or(path);
    if rel.as_os_str().is_empty() {
        ".".to_string()
    } else {
        rel.to_string_lossy().to_string()
    }
}

// ---------------------------------------------------------------------------
// Free functions: enclosing symbol, symbol kind labels, LSP tool handlers
// ---------------------------------------------------------------------------

/// Walk a hierarchical `DocumentSymbol` tree to find the deepest symbol
/// whose range contains `cursor_line`.
pub(crate) fn find_enclosing_symbol(
    symbols: &[lsp_types::DocumentSymbol],
    cursor_line: u32,
) -> Option<&lsp_types::DocumentSymbol> {
    let mut best: Option<&lsp_types::DocumentSymbol> = None;

    for sym in symbols {
        let range = &sym.range;
        if cursor_line >= range.start.line && cursor_line <= range.end.line {
            // This symbol contains the cursor. Check if it's more specific than current best.
            let is_tighter = best
                .map(|b| {
                    let b_span = b.range.end.line - b.range.start.line;
                    let s_span = range.end.line - range.start.line;
                    s_span < b_span
                })
                .unwrap_or(true);
            if is_tighter {
                best = Some(sym);
            }
            // Recurse into children for a tighter match
            if let Some(children) = &sym.children {
                if let Some(child) = find_enclosing_symbol(children, cursor_line) {
                    let child_span = child.range.end.line - child.range.start.line;
                    let best_span = best
                        .map(|b| b.range.end.line - b.range.start.line)
                        .unwrap_or(u32::MAX);
                    if child_span < best_span {
                        best = Some(child);
                    }
                }
            }
        }
    }

    best
}

/// Human-readable label for an LSP SymbolKind.
fn symbol_kind_label(kind: lsp_types::SymbolKind) -> &'static str {
    match kind {
        lsp_types::SymbolKind::FILE => "File",
        lsp_types::SymbolKind::MODULE => "Module",
        lsp_types::SymbolKind::NAMESPACE => "Namespace",
        lsp_types::SymbolKind::PACKAGE => "Package",
        lsp_types::SymbolKind::CLASS => "Class",
        lsp_types::SymbolKind::METHOD => "Method",
        lsp_types::SymbolKind::PROPERTY => "Property",
        lsp_types::SymbolKind::FIELD => "Field",
        lsp_types::SymbolKind::CONSTRUCTOR => "Constructor",
        lsp_types::SymbolKind::ENUM => "Enum",
        lsp_types::SymbolKind::INTERFACE => "Interface",
        lsp_types::SymbolKind::FUNCTION => "Function",
        lsp_types::SymbolKind::VARIABLE => "Variable",
        lsp_types::SymbolKind::CONSTANT => "Constant",
        lsp_types::SymbolKind::STRUCT => "Struct",
        lsp_types::SymbolKind::ENUM_MEMBER => "EnumMember",
        lsp_types::SymbolKind::TYPE_PARAMETER => "TypeParameter",
        _ => "Symbol",
    }
}

/// Format a hierarchical symbol tree for the `document_symbols` tool output.
fn format_symbol_tree(symbols: &[lsp_types::DocumentSymbol], indent: usize, out: &mut String) {
    for sym in symbols {
        let kind = symbol_kind_label(sym.kind);
        let prefix = "  ".repeat(indent);
        out.push_str(&format!(
            "{}{} {} (lines {}-{})\n",
            prefix,
            kind,
            sym.name,
            sym.range.start.line + 1,
            sym.range.end.line + 1,
        ));
        if let Some(children) = &sym.children {
            format_symbol_tree(children, indent + 1, out);
        }
    }
}

/// Format cached document symbols (used when LSP is unavailable).
fn format_document_symbols_cached(symbols: &[lsp_types::DocumentSymbol]) -> ToolResult {
    if symbols.is_empty() {
        return ToolResult::Success(
            "No document symbols available. The language server may not be running \
             or hasn't finished indexing yet."
                .to_string(),
        );
    }
    let mut out = String::from("Document symbols (cached):\n");
    format_symbol_tree(symbols, 0, &mut out);
    ToolResult::Success(out)
}

/// Extract 1-indexed line/column from tool args, converting to 0-indexed.
fn extract_position(args: &serde_json::Value) -> Result<(u32, u32), String> {
    let line = args
        .get("line")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "'line' parameter is required (1-indexed)".to_string())?;
    let col = args
        .get("column")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "'column' parameter is required (1-indexed)".to_string())?;
    if line == 0 {
        return Err("'line' must be >= 1".to_string());
    }
    if col == 0 {
        return Err("'column' must be >= 1".to_string());
    }
    Ok(((line - 1) as u32, (col - 1) as u32))
}

fn handle_lsp_document_symbols(
    lsp: Arc<crate::lsp::LspManager>,
    uri: lsp_types::Uri,
    language_id: String,
    cached_symbols: Vec<lsp_types::DocumentSymbol>,
) -> ToolResult {
    let result = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(async { lsp.document_symbols(&uri, &language_id).await })
    });

    match result {
        Ok(symbols) if !symbols.is_empty() => {
            let mut out = String::from("Document symbols:\n");
            format_symbol_tree(&symbols, 0, &mut out);
            ToolResult::Success(out)
        }
        Ok(_) => {
            // Live LSP returned empty — fall back to cached
            format_document_symbols_cached(&cached_symbols)
        }
        Err(_) => {
            // LSP request failed — fall back to cached
            format_document_symbols_cached(&cached_symbols)
        }
    }
}

fn handle_lsp_hover(
    lsp: Arc<crate::lsp::LspManager>,
    uri: lsp_types::Uri,
    language_id: String,
    args: &serde_json::Value,
) -> ToolResult {
    let (line, col) = match extract_position(args) {
        Ok(pos) => pos,
        Err(e) => return ToolResult::Error(e),
    };

    let result = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(async { lsp.hover(&uri, line, col, &language_id).await })
    });

    match result {
        Ok(Some(content)) => ToolResult::Success(content),
        Ok(None) => {
            ToolResult::Success("No hover information available at this position.".to_string())
        }
        Err(e) => ToolResult::Error(format!("LSP hover failed: {e}")),
    }
}

fn handle_lsp_goto_definition(
    lsp: Arc<crate::lsp::LspManager>,
    uri: lsp_types::Uri,
    language_id: String,
    args: &serde_json::Value,
) -> ToolResult {
    let (line, col) = match extract_position(args) {
        Ok(pos) => pos,
        Err(e) => return ToolResult::Error(e),
    };

    let result = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(async { lsp.goto_definition(&uri, line, col, &language_id).await })
    });

    match result {
        Ok(Some(location)) => {
            let path = crate::lsp::uri_to_file_path(&location.uri)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| location.uri.as_str().to_string());
            let def_line = location.range.start.line + 1;
            let def_col = location.range.start.character + 1;
            ToolResult::Success(format!(
                "Definition found: {}:{} (col {})",
                path, def_line, def_col
            ))
        }
        Ok(None) => ToolResult::Success("No definition found at this position.".to_string()),
        Err(e) => ToolResult::Error(format!("LSP goto_definition failed: {e}")),
    }
}

fn tool_result_success(result: &ToolResult) -> Option<&str> {
    match result {
        ToolResult::Success(s) => Some(s.as_str()),
        ToolResult::Error(_) => None,
    }
}

fn first_number_in_text(text: &str) -> Option<usize> {
    let mut digits = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if !digits.is_empty() {
            break;
        }
    }
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn tool_line_range_suffix(args: &serde_json::Value) -> String {
    let start = args.get("start_line").and_then(|v| v.as_u64());
    let end = args.get("end_line").and_then(|v| v.as_u64());
    match (start, end) {
        (Some(s), Some(e)) => format!(":{s}-{e}"),
        (Some(s), None) => format!(":{s}"),
        _ => String::new(),
    }
}

fn compact_tool_label(text: &str) -> String {
    let single_line = text
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let max_chars = 72;
    if single_line.chars().count() <= max_chars {
        return single_line;
    }
    let mut out: String = single_line
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect();
    out.push('…');
    out
}

fn compact_tool_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return ".".to_string();
    }

    let keep = 3usize.min(parts.len());
    let tail = parts[parts.len() - keep..].join("/");
    let max_chars = 42usize;
    if tail.chars().count() <= max_chars {
        return tail;
    }

    let mut out: String = tail.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::{ChatOpts, ToolCallInfo};
    use std::fs;

    fn make_symbol(
        name: &str,
        kind: lsp_types::SymbolKind,
        start_line: u32,
        end_line: u32,
        children: Option<Vec<lsp_types::DocumentSymbol>>,
    ) -> lsp_types::DocumentSymbol {
        #[allow(deprecated)]
        lsp_types::DocumentSymbol {
            name: name.to_string(),
            detail: None,
            kind,
            tags: None,
            deprecated: None,
            range: lsp_types::Range {
                start: lsp_types::Position::new(start_line, 0),
                end: lsp_types::Position::new(end_line, 0),
            },
            selection_range: lsp_types::Range {
                start: lsp_types::Position::new(start_line, 0),
                end: lsp_types::Position::new(start_line, 10),
            },
            children,
        }
    }

    #[test]
    fn find_enclosing_symbol_finds_deepest() {
        let symbols = vec![make_symbol(
            "MyStruct",
            lsp_types::SymbolKind::STRUCT,
            10,
            50,
            Some(vec![
                make_symbol("new", lsp_types::SymbolKind::FUNCTION, 15, 25, None),
                make_symbol("update", lsp_types::SymbolKind::FUNCTION, 30, 45, None),
            ]),
        )];

        // Cursor inside `new` function
        let result = find_enclosing_symbol(&symbols, 20);
        assert_eq!(result.unwrap().name, "new");

        // Cursor inside `update` function
        let result = find_enclosing_symbol(&symbols, 35);
        assert_eq!(result.unwrap().name, "update");

        // Cursor inside struct but outside any function
        let result = find_enclosing_symbol(&symbols, 48);
        assert_eq!(result.unwrap().name, "MyStruct");

        // Cursor outside all symbols
        let result = find_enclosing_symbol(&symbols, 5);
        assert!(result.is_none());
    }

    #[test]
    fn find_enclosing_symbol_empty() {
        assert!(find_enclosing_symbol(&[], 10).is_none());
    }

    #[test]
    fn tool_summary_for_edit_range_reports_plus_minus_delta() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ChatOpts {
                name: "chat".to_string(),
                allow_edits: true,
                ..Default::default()
            })
            .expect("open chat");

        let tc = ToolCallInfo {
            id: "call_1".to_string(),
            name: "edit_range".to_string(),
            arguments: serde_json::json!({
                "start_line": 10,
                "end_line": 12,
                "new_text": "a\nb\n"
            }),
        };
        let summary = editor.build_tool_event_summary(&tc, &ToolResult::Success("ok".to_string()));
        assert_eq!(summary.kind, ToolSummaryKind::Mutation);
        assert!(summary.label.contains("+0 -1"), "{}", summary.label);
    }

    #[test]
    fn write_file_at_path_creates_missing_file() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let target = dir.path().join("new_module.rs");

            let mut editor = Editor::default();
            editor
                .open_ai_chat(ChatOpts {
                    name: "chat".to_string(),
                    allow_edits: true,
                    ..Default::default()
                })
                .expect("open chat");
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.approved_external_roots.push(dir.path().to_path_buf());
                let canonical =
                    std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf());
                if canonical != dir.path() {
                    chat.approved_external_roots.push(canonical);
                }
            }

            let tool_call = ToolCallInfo {
                id: "call_write".to_string(),
                name: "write_file_at_path".to_string(),
                arguments: serde_json::json!({
                    "path": target.to_string_lossy().to_string(),
                    "content": "pub fn generated() {}\n"
                }),
            };

            match editor.dispatch_tool_call_with_approval(&tool_call, None) {
                ToolDispatchOutcome::Completed(ToolResult::Success(_)) => {}
                ToolDispatchOutcome::Completed(ToolResult::Error(e)) => {
                    panic!("unexpected error: {e}");
                }
                ToolDispatchOutcome::ApprovalRequired(req) => {
                    panic!("unexpected approval request: {}", req.message);
                }
            }

            let content = fs::read_to_string(&target).expect("read target");
            assert!(content.contains("pub fn generated() {}"));
        });
    }

    #[test]
    fn edit_range_with_path_updates_target_file() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let target = dir.path().join("target.rs");
            fs::write(&target, "line1\nline2\n").expect("seed");

            let mut editor = Editor::default();
            editor
                .open_ai_chat(ChatOpts {
                    name: "chat".to_string(),
                    allow_edits: true,
                    ..Default::default()
                })
                .expect("open chat");
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.approved_external_roots.push(dir.path().to_path_buf());
                let canonical =
                    std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf());
                if canonical != dir.path() {
                    chat.approved_external_roots.push(canonical);
                }
            }

            let tool_call = ToolCallInfo {
                id: "call_edit".to_string(),
                name: "edit_range".to_string(),
                arguments: serde_json::json!({
                    "path": target.to_string_lossy().to_string(),
                    "start_line": 1,
                    "end_line": 1,
                    "new_text": "updated"
                }),
            };

            match editor.dispatch_tool_call_with_approval(&tool_call, None) {
                ToolDispatchOutcome::Completed(ToolResult::Success(_)) => {}
                ToolDispatchOutcome::Completed(ToolResult::Error(e)) => {
                    panic!("unexpected error: {e}");
                }
                ToolDispatchOutcome::ApprovalRequired(req) => {
                    panic!("unexpected approval request: {}", req.message);
                }
            }

            let content = fs::read_to_string(&target).expect("read target");
            assert!(content.starts_with("updated\n"));
        });
    }

    #[test]
    fn snapshot_and_restore_file_round_trip() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let target = dir.path().join("restore.rs");
            fs::write(&target, "alpha\nbeta\n").expect("seed");

            let mut editor = Editor::default();
            editor
                .open_ai_chat(ChatOpts {
                    name: "chat".to_string(),
                    allow_edits: true,
                    ..Default::default()
                })
                .expect("open chat");
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.approved_external_roots.push(dir.path().to_path_buf());
                let canonical =
                    std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf());
                if canonical != dir.path() {
                    chat.approved_external_roots.push(canonical);
                }
            }

            let snapshot_call = ToolCallInfo {
                id: "call_snap".to_string(),
                name: "snapshot_file".to_string(),
                arguments: serde_json::json!({
                    "path": target.to_string_lossy().to_string()
                }),
            };
            match editor.dispatch_tool_call_with_approval(&snapshot_call, None) {
                ToolDispatchOutcome::Completed(ToolResult::Success(_)) => {}
                ToolDispatchOutcome::Completed(ToolResult::Error(e)) => {
                    panic!("unexpected snapshot error: {e}");
                }
                ToolDispatchOutcome::ApprovalRequired(req) => {
                    panic!("unexpected approval request: {}", req.message);
                }
            }

            let snapshot_id = editor
                .ai_state
                .chat
                .as_ref()
                .and_then(|c| c.file_snapshots.keys().next().cloned())
                .expect("snapshot id");

            let edit_call = ToolCallInfo {
                id: "call_edit".to_string(),
                name: "edit_range".to_string(),
                arguments: serde_json::json!({
                    "path": target.to_string_lossy().to_string(),
                    "start_line": 1,
                    "end_line": 1,
                    "new_text": "changed"
                }),
            };
            match editor.dispatch_tool_call_with_approval(&edit_call, None) {
                ToolDispatchOutcome::Completed(ToolResult::Success(_)) => {}
                ToolDispatchOutcome::Completed(ToolResult::Error(e)) => {
                    panic!("unexpected edit error: {e}");
                }
                ToolDispatchOutcome::ApprovalRequired(req) => {
                    panic!("unexpected approval request: {}", req.message);
                }
            }

            let restore_call = ToolCallInfo {
                id: "call_restore".to_string(),
                name: "restore_file".to_string(),
                arguments: serde_json::json!({
                    "path": target.to_string_lossy().to_string(),
                    "snapshot_id": snapshot_id
                }),
            };
            match editor.dispatch_tool_call_with_approval(&restore_call, None) {
                ToolDispatchOutcome::Completed(ToolResult::Success(_)) => {}
                ToolDispatchOutcome::Completed(ToolResult::Error(e)) => {
                    panic!("unexpected restore error: {e}");
                }
                ToolDispatchOutcome::ApprovalRequired(req) => {
                    panic!("unexpected approval request: {}", req.message);
                }
            }

            let content = fs::read_to_string(&target).expect("read target");
            assert_eq!(content, "alpha\nbeta\n");
        });
    }

    #[test]
    fn tool_context_uses_active_chat_target_buffer() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let file_a = dir.path().join("a.rs");
            let file_b = dir.path().join("b.rs");
            fs::write(&file_a, "from_a\n").expect("seed a");
            fs::write(&file_b, "from_b\n").expect("seed b");

            let mut editor = Editor::default();
            editor.open_file(&file_a).expect("open a");
            editor
                .open_ai_chat(ChatOpts {
                    name: "chat".to_string(),
                    allow_edits: true,
                    ..Default::default()
                })
                .expect("open chat");
            let active_buffer_id = editor
                .ai_state
                .chat
                .as_ref()
                .map(|c| c.active_buffer_id)
                .expect("chat");

            // User switches current buffer, but active chat target should stay on file_a.
            editor.open_file(&file_b).expect("open b");
            let active_idx = editor
                .find_buffer_index_by_id(active_buffer_id)
                .expect("active buffer index");
            assert_ne!(editor.current_buffer_index(), active_idx);

            let ctx = editor.build_tool_execution_context();
            assert!(ctx
                .file_path
                .as_deref()
                .is_some_and(|p| p.ends_with("a.rs")));
            assert!(ctx.buffer_content.contains("from_a"));
        });
    }

    #[test]
    fn open_file_with_create_opens_missing_target() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let target = dir.path().join("new_file.rs");

            let mut editor = Editor::default();
            editor
                .open_ai_chat(ChatOpts {
                    name: "chat".to_string(),
                    allow_edits: true,
                    ..Default::default()
                })
                .expect("open chat");
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.approved_external_roots.push(dir.path().to_path_buf());
                let canonical =
                    std::fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf());
                if canonical != dir.path() {
                    chat.approved_external_roots.push(canonical);
                }
            }

            let tool_call = ToolCallInfo {
                id: "call_open".to_string(),
                name: "open_file".to_string(),
                arguments: serde_json::json!({
                    "path": target.to_string_lossy().to_string(),
                    "create": true
                }),
            };

            match editor.dispatch_tool_call_with_approval(&tool_call, None) {
                ToolDispatchOutcome::Completed(ToolResult::Success(_)) => {}
                ToolDispatchOutcome::Completed(ToolResult::Error(e)) => {
                    panic!("unexpected error: {e}");
                }
                ToolDispatchOutcome::ApprovalRequired(req) => {
                    panic!("unexpected approval request: {}", req.message);
                }
            }

            assert!(editor
                .buffer()
                .file_path()
                .is_some_and(|p| p.ends_with("new_file.rs")));
        });
    }

    #[test]
    fn no_file_open_limits_toolset_to_file_scope_and_keeps_open_file() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ChatOpts {
                name: "chat".to_string(),
                allow_edits: true,
                ..Default::default()
            })
            .expect("open chat");

        let active = editor.ai_state.active_profile.clone();
        let profile = editor
            .ai_state
            .config
            .resolve_profile(&active)
            .expect("profile");
        let caps = editor.build_chat_capabilities();
        let names: Vec<&str> = editor
            .ai_state
            .tool_registry
            .tools_for_profile(profile, &caps)
            .into_iter()
            .map(|t| t.name.as_str())
            .collect();

        assert!(names.contains(&"open_file"));
        assert!(!names.contains(&"list_files"));
        assert!(!names.contains(&"search_project"));
    }

    #[test]
    fn no_file_open_returns_consistent_guidance_for_non_open_tools() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ChatOpts {
                name: "chat".to_string(),
                allow_edits: true,
                ..Default::default()
            })
            .expect("open chat");

        let tool_call = ToolCallInfo {
            id: "call_read".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({}),
        };

        match editor.dispatch_tool_call_with_approval(&tool_call, None) {
            ToolDispatchOutcome::Completed(ToolResult::Error(err)) => {
                assert!(err.contains("No file open."));
                assert!(err.contains("open_file(path, create=true)"));
            }
            ToolDispatchOutcome::Completed(ToolResult::Success(ok)) => {
                panic!("expected guidance error, got success: {ok}");
            }
            ToolDispatchOutcome::ApprovalRequired(req) => {
                panic!("unexpected approval request: {}", req.message);
            }
        }
    }

    #[test]
    fn tool_dispatch_fails_when_active_target_buffer_id_is_invalid() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ChatOpts {
                name: "chat".to_string(),
                allow_edits: true,
                ..Default::default()
            })
            .expect("open chat");

        if let Some(chat) = editor.ai_state.chat.as_mut() {
            chat.active_buffer_id = u64::MAX;
        }

        let tool_call = ToolCallInfo {
            id: "call_read".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({}),
        };

        match editor.dispatch_tool_call_with_approval(&tool_call, None) {
            ToolDispatchOutcome::Completed(ToolResult::Error(err)) => {
                assert!(err.contains("Active chat target is no longer available"));
            }
            ToolDispatchOutcome::Completed(ToolResult::Success(ok)) => {
                panic!("expected invalid-target error, got success: {ok}");
            }
            ToolDispatchOutcome::ApprovalRequired(req) => {
                panic!("unexpected approval request: {}", req.message);
            }
        }
    }
}
