//! Narrow MCP facade over canonical editor operations. Requests reach the editor
//! only through the owning provider job, never through global session discovery.
use super::{ai_chat_state::CodeExplanationContinuation, Editor};
use crate::ai::{
    tools::{schema, EditorBridgeTool},
    ToolCallInfo, ToolResult,
};
use crate::mcp::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use serde_json::{json, Value};
use tokio::sync::oneshot;

fn reply(id: Value, result: Result<Value, JsonRpcError>) -> Value {
    let (result, error) = match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    };
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id: Some(id),
        result,
        error,
    })
    .expect("RPC serializes")
}

pub(super) fn tool_reply(id: Value, result: ToolResult) -> Value {
    let (text, failed) = match result {
        ToolResult::Success(text) => (text, false),
        ToolResult::Error(text) => (text, true),
    };
    reply(
        id,
        Ok(json!({"content":[{"type":"text", "text":text}], "isError":failed})),
    )
}

pub(super) fn walkthrough_reply(id: Value, request_id: &str, result: ToolResult) -> Value {
    match result {
        ToolResult::Success(outcome) => tool_reply(
            id,
            ToolResult::Success(
                json!({"outcome":outcome, "ovim_walkthrough_id":request_id}).to_string(),
            ),
        ),
        error => tool_reply(id, error),
    }
}

fn bridge_schema(tool: &crate::ai::ToolDefinition, operation: EditorBridgeTool) -> Value {
    let mut input = schema::input_schema(tool);
    if operation == EditorBridgeTool::OpenFile {
        // Navigation is available in read-only queries; creation is not.
        input["properties"]["create"] = json!({"type":"boolean", "const":false, "description":"This editor connection opens existing files only."});
        input["properties"]["column"]["minimum"] = json!(1);
    }
    input
}

impl Editor {
    pub(crate) fn handle_editor_mcp_request(
        &mut self,
        request_id: String,
        value: Value,
        response: oneshot::Sender<Value>,
    ) {
        if response.is_closed() {
            return;
        }
        let rpc_id = value.get("id").cloned().unwrap_or(Value::Null);
        let parsed = serde_json::from_value::<JsonRpcRequest>(value);
        let request = match parsed {
            Ok(request)
                if request.jsonrpc == "2.0"
                    && request
                        .id
                        .as_ref()
                        .is_some_and(|id| id.is_string() || id.is_number()) =>
            {
                request
            }
            _ => {
                let _ = response.send(reply(rpc_id, Err(JsonRpcError::invalid_request())));
                return;
            }
        };
        if self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.external_agent.as_ref())
            .is_none()
        {
            let _ = response.send(reply(
                rpc_id,
                Err(JsonRpcError::internal_error(
                    "Editor connection is no longer active",
                )),
            ));
            return;
        }
        let result = match request.method.as_str() {
            "initialize" => {
                match serde_json::from_value::<crate::mcp::InitializeParams>(request.params) {
                    Ok(_) => Ok(
                        json!({"protocolVersion":"2025-03-26", "capabilities":{"tools":{}}, "serverInfo":{"name":"ovim-editor", "version":env!("CARGO_PKG_VERSION")}}),
                    ),
                    Err(error) => Err(JsonRpcError::invalid_params(&error.to_string())),
                }
            }
            "ping" => Ok(json!({})),
            "tools/list" => {
                let mut tools = self.ai_state.tool_registry.editor_bridge_tools().map(|(tool, operation)| {
                    json!({"name":tool.name, "description":tool.description, "inputSchema":bridge_schema(tool, operation),
                        "annotations":{"readOnlyHint":operation == EditorBridgeTool::Context, "destructiveHint":false, "openWorldHint":false}})
                }).collect::<Vec<_>>();
                tools.sort_by_key(|tool| tool["name"].as_str().unwrap_or_default().to_owned());
                Ok(json!({"tools":tools}))
            }
            "tools/call" => {
                self.call_editor_mcp_tool(request_id, rpc_id, request.params, response);
                return;
            }
            _ => Err(JsonRpcError::method_not_found(&request.method)),
        };
        let _ = response.send(reply(rpc_id, result));
    }

    fn call_editor_mcp_tool(
        &mut self,
        request_id: String,
        rpc_id: Value,
        params: Value,
        response: oneshot::Sender<Value>,
    ) {
        let name = params["name"].as_str().unwrap_or_default();
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let tool = self
            .ai_state
            .tool_registry
            .editor_bridge_tools()
            .find(|(tool, _)| tool.name == name)
            .map(|(tool, op)| (bridge_schema(tool, op), op));
        let Some((input_schema, operation)) = tool else {
            let _ = response.send(tool_reply(
                rpc_id,
                ToolResult::Error("Tool is not available on this editor connection".into()),
            ));
            return;
        };
        if let Err(error) = jsonschema::validate(&input_schema, &args) {
            let _ = response.send(tool_reply(
                rpc_id,
                ToolResult::Error(format!("Invalid tool input: {error}")),
            ));
            return;
        }
        // A walkthrough owns navigation until the user finishes or dismisses it.
        if operation != EditorBridgeTool::Context && self.ai_chat_has_pending_code_explanation() {
            let _ = response.send(tool_reply(
                rpc_id,
                ToolResult::Error(
                    "Finish or dismiss the current walkthrough before navigating".into(),
                ),
            ));
            return;
        }
        let result = match operation {
            EditorBridgeTool::Context => {
                let mut context = self.build_tool_execution_context();
                context.scope_context.project_root = self
                    .ai_state
                    .chat
                    .as_ref()
                    .and_then(|chat| chat.external_agent.as_ref())
                    .map(|state| state.root.clone());
                match crate::ai::tools::builtins::execute_builtin(name, &args, &context) {
                    ToolResult::Success(orientation) => ToolResult::Success(format!(
                        "{orientation}\n\n{}",
                        self.build_editor_state_context(4_000)
                    )),
                    error => error,
                }
            }
            EditorBridgeTool::OpenFile => {
                match self.editor_mcp_path(args["path"].as_str().unwrap_or_default()) {
                    Ok(path) => self.handle_open_file_at_absolute_path(&path, &args, false),
                    Err(error) => ToolResult::Error(error),
                }
            }
            EditorBridgeTool::Explain => {
                let mut args = args;
                // Resolve against the turn's original workspace, not a later UI selection.
                for step in args["steps"].as_array_mut().expect("validated steps") {
                    if let Some(path) = step.get("path").and_then(Value::as_str) {
                        match self.editor_mcp_path(path) {
                            Ok(path) => step["path"] = json!(path),
                            Err(error) => {
                                let _ = response.send(tool_reply(rpc_id, ToolResult::Error(error)));
                                return;
                            }
                        }
                    }
                }
                let call = ToolCallInfo {
                    id: request_id.clone(),
                    name: name.into(),
                    arguments: args,
                };
                let continuation = CodeExplanationContinuation::EditorMcp {
                    request_id,
                    rpc_id,
                    response,
                };
                if let Err((error, continuation)) = self.begin_code_explanation(call, continuation)
                {
                    if let CodeExplanationContinuation::EditorMcp {
                        rpc_id, response, ..
                    } = *continuation
                    {
                        let _ = response.send(tool_reply(rpc_id, error));
                    }
                }
                return;
            }
        };
        let _ = response.send(tool_reply(rpc_id, result));
    }

    fn editor_mcp_path(&self, path: &str) -> Result<std::path::PathBuf, String> {
        let state = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.external_agent.as_ref())
            .ok_or("Editor connection ended")?;
        let path = state
            .root
            .join(path)
            .canonicalize()
            .map_err(|error| format!("Cannot open path: {error}"))?;
        if !path.starts_with(&state.root) || !path.is_file() {
            return Err(
                "Editor navigation is limited to existing files in this turn's workspace".into(),
            );
        }
        Ok(path)
    }

    /// Keep invocation-time snapshots available under the provider's tool ID
    /// for the existing replay UI. The marker comes from our own MCP result.
    pub(crate) fn bind_editor_walkthrough_result(&mut self, tool_id: &str, content: &str) {
        fn marker(value: &Value) -> Option<String> {
            if let Some(id) = value.get("ovim_walkthrough_id").and_then(Value::as_str) {
                return Some(id.into());
            }
            if let Some(blocks) = value.as_array() {
                return blocks
                    .iter()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .filter_map(|text| serde_json::from_str::<Value>(text).ok())
                    .find_map(|value| marker(&value));
            }
            None
        }
        let Some(id) = serde_json::from_str::<Value>(content)
            .ok()
            .and_then(|value| marker(&value))
        else {
            return;
        };
        if let Some(chat) = self.ai_state.chat.as_mut() {
            if let Some(entry) = chat
                .code_explanation_cache
                .iter_mut()
                .find(|entry| entry.tool_call_id == id)
            {
                entry.tool_call_id = tool_id.into();
            }
        }
    }

    pub(crate) fn abort_editor_mcp_walkthrough(&mut self) {
        let pending = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.pending_code_explanation.as_mut());
        let Some(pending) = pending else {
            return;
        };
        if !matches!(
            pending.continuation,
            Some(CodeExplanationContinuation::EditorMcp { .. })
        ) {
            return;
        }
        if let Some(CodeExplanationContinuation::EditorMcp {
            rpc_id, response, ..
        }) = pending.continuation.take()
        {
            let _ = response.send(tool_reply(
                rpc_id,
                ToolResult::Error("Provider turn ended before the walkthrough completed".into()),
            ));
        }
        self.finish_code_explanation(true);
        self.set_status_message("Provider turn ended; walkthrough closed");
    }

    pub(crate) fn cancel_editor_mcp_request(&mut self, id: &str) {
        let matches = self.ai_state.chat.as_ref().and_then(|chat| chat.pending_code_explanation.as_ref())
            .and_then(|pending| pending.continuation.as_ref())
            .is_some_and(|continuation| matches!(continuation, CodeExplanationContinuation::EditorMcp { request_id, .. } if request_id == id));
        if matches {
            self.finish_code_explanation(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ai_external_agent::tests::{attach, editor};
    use super::*;
    use crate::ai::StreamChunk;

    fn request(
        editor: &mut Editor,
        id: &str,
        method: &str,
        params: Value,
    ) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        editor.handle_editor_mcp_request(
            id.into(),
            json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}),
            tx,
        );
        rx
    }

    #[tokio::test]
    async fn bridge_exposes_only_canonical_context_and_navigation_schemas() {
        let mut editor = editor();
        let _tx = attach(&mut editor);
        let initialized = request(&mut editor, "init", "initialize", json!({"protocolVersion":"2025-03-26", "capabilities":{}, "clientInfo":{"name":"test", "version":"1"}})).await.unwrap();
        assert_eq!(initialized["result"]["protocolVersion"], "2025-03-26");
        let list = request(&mut editor, "list", "tools/list", json!({}))
            .await
            .unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["explain_with_codebase", "open_file", "workspace_context"]
        );
        assert_eq!(
            tools[1]["inputSchema"]["properties"]["create"]["const"],
            false
        );
        for params in [
            json!({"name":"send_keys", "arguments":{"keys":":q!"}}),
            json!({"name":"open_file", "arguments":{"path":"new.rs", "create":true}}),
            json!({"name":"workspace_context", "arguments":{"session":"other-window"}}),
        ] {
            assert_eq!(
                request(&mut editor, "bad", "tools/call", params)
                    .await
                    .unwrap()["result"]["isError"],
                true
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn navigation_preserves_unsaved_buffers_and_stays_in_original_workspace() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("first.rs"), "fn main() {}\n").unwrap();
        std::fs::write(directory.path().join("second.rs"), "// Norsk 🦦\n").unwrap();
        let mut editor = editor();
        let _tx = attach(&mut editor);
        editor
            .ai_state
            .chat
            .as_mut()
            .unwrap()
            .external_agent
            .as_mut()
            .unwrap()
            .root = directory.path().canonicalize().unwrap();
        let mut result = request(
            &mut editor,
            "open",
            "tools/call",
            json!({"name":"open_file", "arguments":{"path":"first.rs"}}),
        )
        .await
        .unwrap();
        assert_eq!(result["result"]["isError"], false, "{result}");
        editor.buffer_mut().replace_all("// unsaved\n");
        let first = editor.buffer().id();
        result = request(
            &mut editor,
            "next",
            "tools/call",
            json!({"name":"open_file", "arguments":{"path":"second.rs"}}),
        )
        .await
        .unwrap();
        assert_eq!(result["result"]["isError"], false, "{result}");
        assert!(editor
            .buffers
            .iter()
            .any(|buffer| buffer.id() == first && buffer.rope().to_string().contains("unsaved")));
        let context = request(
            &mut editor,
            "context",
            "tools/call",
            json!({"name":"workspace_context"}),
        )
        .await
        .unwrap();
        assert!(context.to_string().contains("second.rs"));
        assert!(!context.to_string().contains("Workspace: unavailable"));
        let mut walkthrough = request(
            &mut editor,
            "code",
            "tools/call",
            json!({"name":"explain_with_codebase", "arguments":{"steps":[{"type":"code", "path":"second.rs", "start_line":1, "comment":"The file contains the example."}]}}),
        );
        let early = walkthrough.try_recv();
        assert!(
            early.is_err(),
            "Code walkthrough returned early instead of presenting: {early:?}"
        );
        assert!(editor.ai_chat_has_pending_code_explanation());
        assert!(editor.finish_code_explanation(false));
        assert_eq!(walkthrough.await.unwrap()["result"]["isError"], false);
        let outside = tempfile::NamedTempFile::new().unwrap();
        result = request(
            &mut editor,
            "outside",
            "tools/call",
            json!({"name":"open_file", "arguments":{"path":outside.path()}}),
        )
        .await
        .unwrap();
        assert_eq!(result["result"]["isError"], true);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), directory.path().join("escape.rs")).unwrap();
            result = request(
                &mut editor,
                "escape",
                "tools/call",
                json!({"name":"open_file", "arguments":{"path":"escape.rs"}}),
            )
            .await
            .unwrap();
            assert_eq!(result["result"]["isError"], true);
        }
    }

    #[tokio::test]
    async fn walkthrough_waits_for_user_and_cancellation_is_correlated() {
        let mut editor = editor();
        let _tx = attach(&mut editor);
        let mut answer = request(
            &mut editor,
            "walk",
            "tools/call",
            json!({"name":"explain_with_codebase", "arguments":{"steps":[{"type":"concept", "title":"Overview", "body":"One idea."}]}}),
        );
        assert!(answer.try_recv().is_err());
        assert!(editor.ai_chat_has_pending_code_explanation());
        editor.cancel_editor_mcp_request("different-request");
        assert!(editor.ai_chat_has_pending_code_explanation());
        editor.cancel_editor_mcp_request("walk");
        let result = answer.await.unwrap();
        assert!(result.to_string().contains("dismissed"));
        assert!(!editor.ai_chat_has_pending_code_explanation());
        let answer = request(
            &mut editor,
            "finish",
            "tools/call",
            json!({"name":"explain_with_codebase", "arguments":{"steps":[{"type":"concept", "title":"Overview", "body":"One idea."}]}}),
        );
        assert!(editor.finish_code_explanation(false));
        assert!(answer.await.unwrap().to_string().contains("completed"));
    }

    #[tokio::test]
    async fn walkthrough_question_answers_in_current_turn_without_duplicate_queue() {
        let mut editor = editor();
        let tx = attach(&mut editor);
        let answer = request(
            &mut editor,
            "walk",
            "tools/call",
            json!({"name":"explain_with_codebase", "arguments":{"steps":[{"type":"concept", "title":"Overview", "body":"One idea."}]}}),
        );
        assert!(editor.begin_code_explanation_question());
        for ch in "Why 🦦?".chars() {
            editor.insert_code_explanation_question_char(ch);
        }
        assert!(editor.submit_code_explanation_question().unwrap());
        assert!(answer.await.unwrap().to_string().contains("Why 🦦?"));
        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .queued_inputs
            .is_empty());
        tx.send(StreamChunk::Content("Because.".into())).unwrap();
        editor.poll_pending_ai_chat_job();
        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .pending_code_explanation
            .as_ref()
            .unwrap()
            .threads[0][0]
            .answer
            .contains("Because."));
    }
    #[tokio::test]
    async fn mcp_walkthrough_stream_preserves_native_history_and_replay() {
        let mut editor = editor();
        let stream = attach(&mut editor);
        let args = json!({"steps":[{"type":"concept", "title":"Overview", "body":"One idea."}]});
        stream
            .send(StreamChunk::ExternalToolStart(ToolCallInfo {
                id: "native-call".into(),
                name: "mcp__ovim__explain_with_codebase".into(),
                arguments: args.clone(),
            }))
            .unwrap();
        let (response, answer) = oneshot::channel();
        stream.send(StreamChunk::ExternalEditorRequest {
            id:"bridge-call".into(), request:json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"explain_with_codebase", "arguments":args}}), response,
        }).unwrap();
        editor.poll_pending_ai_chat_job();
        assert!(editor.finish_code_explanation(false));
        let result = answer.await.unwrap();
        stream
            .send(StreamChunk::ExternalToolResult {
                id: "native-call".into(),
                content: result["result"]["content"].to_string(),
                error: false,
            })
            .unwrap();
        stream
            .send(StreamChunk::ExternalSession("native-session".into()))
            .unwrap();
        stream.send(StreamChunk::Done).unwrap();
        editor.poll_pending_ai_chat_job();
        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .code_explanation_cache
            .iter()
            .any(|entry| entry.tool_call_id == "native-call"));
        assert!(editor.replay_code_explanation("native-call"));
        assert!(editor.ai_chat_has_pending_code_explanation());
    }
    #[tokio::test]
    async fn provider_failure_closes_pending_walkthrough_and_completes_response() {
        let mut editor = editor();
        let stream = attach(&mut editor);
        let answer = request(
            &mut editor,
            "walk",
            "tools/call",
            json!({"name":"explain_with_codebase", "arguments":{"steps":[{"type":"concept", "title":"Overview", "body":"One idea."}]}}),
        );
        stream
            .send(StreamChunk::Error("Provider disconnected".into()))
            .unwrap();
        editor.poll_pending_ai_chat_job();
        assert_eq!(answer.await.unwrap()["result"]["isError"], true);
        assert!(!editor.ai_chat_has_pending_code_explanation());
        assert!(editor
            .ai_state
            .chat
            .as_ref()
            .unwrap()
            .external_agent
            .is_none());
    }
}
