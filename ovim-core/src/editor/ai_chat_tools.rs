use crate::ai::chat_types::{ChatMessage, StreamChunk, ToolCallInfo};
use crate::ai::scope::{Capabilities, ScopeContext};
use crate::ai::stream_ai_chat;
use crate::ai::tools::builtins::{self, ToolExecutionContext};
use crate::ai::tools::schema;
use crate::ai::tools::{SideEffect, ToolResult};
use crate::ai::FileScope;
use anyhow::Result;

use super::ai_chat_state::PendingAiChatJob;
use super::Editor;

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

        // If edits not allowed, restrict to read-only (File scope max)
        if !allow_edits {
            caps.file_scope = std::cmp::min(caps.file_scope, FileScope::File);
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
        let buf = &self.buffers[self.current_buffer_index];
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

        // Get diagnostics for current buffer
        let diagnostics = self.get_diagnostics_for_current_buffer();

        let current_file = buf.file_path().map(std::path::PathBuf::from);
        let project_root = std::env::current_dir().ok();

        ToolExecutionContext {
            buffer_content,
            file_path,
            cursor,
            selection,
            diagnostics,
            scope_context: ScopeContext {
                current_file,
                project_root,
            },
            capabilities: self.build_chat_capabilities(),
        }
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
    pub(crate) fn dispatch_tool_call(&mut self, tc: &ToolCallInfo) -> ToolResult {
        match self
            .ai_state
            .tool_registry
            .get(&tc.name)
            .map(|t| t.side_effect)
        {
            Some(SideEffect::Read) => {
                let ctx = self.build_tool_execution_context();
                self.execute_tool_call(tc, &ctx)
            }
            Some(SideEffect::Mutation) => self.execute_mutation_tool(&tc.name, &tc.arguments),
            Some(SideEffect::External) => {
                ToolResult::Error("external tools not yet supported".into())
            }
            None => ToolResult::Error(format!("unknown tool: {}", tc.name)),
        }
    }

    /// Execute tool calls from a completed stream response, record results,
    /// and continue the conversation. Returns true to signal state changed.
    pub(crate) fn process_tool_calls(
        &mut self,
        tool_calls: Vec<ToolCallInfo>,
        content: String,
        model_name: &str,
    ) -> bool {
        let iterations = self
            .ai_state
            .chat
            .as_ref()
            .map(|c| c.tool_iterations)
            .unwrap_or(0);
        let max_iterations = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|c| c.opts.profile.as_ref())
            .and_then(|p| self.ai_state.config.resolve_profile(p))
            .map(|p| (p.agent_loop.max_tool_calls / 10).min(255) as u8)
            .unwrap_or(4);

        if iterations >= max_iterations {
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

        // 1. Commit content + tool_calls as assistant message
        if let Some(conv) = self.conversation_mut() {
            conv.append_assistant_message_with_tools(
                content,
                model_name.to_string(),
                tool_calls.clone(),
            );
        }

        // 2. Execute each tool with bifurcated dispatch
        for tc in &tool_calls {
            let result = self.dispatch_tool_call(tc);
            let result_content = match &result {
                ToolResult::Success(s) => s.clone(),
                ToolResult::Error(s) => format!("Error: {s}"),
            };
            if let Some(conv) = self.conversation_mut() {
                conv.append_tool_result(tc.id.clone(), result_content);
            }
        }

        // 3. Increment iterations
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.tool_iterations += 1;
        }

        // 4. Start new streaming request
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

        let buf = &self.buffers[self.current_buffer_index];
        let file_info = match buf.file_path() {
            Some(path) => {
                let lang =
                    crate::syntax::LanguageRegistry::get_lsp_language_id(path).unwrap_or("unknown");
                format!("Current file: {} ({})", path, lang)
            }
            None => "No file open.".to_string(),
        };

        let allow_edits = self
            .ai_state
            .chat
            .as_ref()
            .map(|c| c.allow_edits)
            .unwrap_or(false);

        let mut prompt = String::from(
            "You are a coding assistant inside the ovim editor.\n\
             Respond in natural language. Do NOT return raw JSON.\n",
        );

        if !tools.is_empty() {
            let read_tools: Vec<&str> = tools
                .iter()
                .filter(|t| t.side_effect == SideEffect::Read)
                .map(|t| t.name.as_str())
                .collect();
            if !read_tools.is_empty() {
                prompt.push_str(&format!(
                    "You have read tools: {}. Use them when the user asks about code.\n",
                    read_tools.join(", ")
                ));
            }

            if allow_edits {
                let mutation_tools: Vec<&str> = tools
                    .iter()
                    .filter(|t| t.side_effect == SideEffect::Mutation)
                    .map(|t| t.name.as_str())
                    .collect();
                if !mutation_tools.is_empty() {
                    prompt.push_str(&format!(
                        "You can edit files using: {}.\n",
                        mutation_tools.join(", ")
                    ));
                }
            }
        }

        prompt.push_str(&file_info);
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
        let tool_schemas = self.build_tool_schemas_for_chat(&profile);
        let api_key_registry = self.ai_state.config.api_key_registry.clone();

        let messages: Vec<ChatMessage> = self
            .conversation()
            .map(|c| c.messages().to_vec())
            .unwrap_or_default();

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
            chat.streaming_content = Some(String::new());
            chat.streaming_thinking = None;
            chat.streaming_tool_calls.clear();
        }

        Ok(())
    }

    /// Get diagnostics for the current buffer (read from LSP state).
    pub(crate) fn get_diagnostics_for_current_buffer(&self) -> Vec<crate::ai::DiagnosticFact> {
        let diags = self.all_diagnostics();
        diags
            .iter()
            .map(|d| crate::ai::DiagnosticFact {
                message: d.message.clone(),
                severity: d.severity.map(|s| format!("{:?}", s)),
                line: d.range.start.line,
                start_character: d.range.start.character,
                end_character: d.range.end.character,
            })
            .collect()
    }

    /// Clear all streaming state and mark the chat as no longer waiting.
    pub(crate) fn clear_streaming_state(&mut self) {
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.waiting = false;
            chat.pending_job = None;
            chat.streaming_content = None;
            chat.streaming_thinking = None;
            chat.message_scroll = 0;
        }
    }
}
