use crate::ai::chat_types::ToolCallInfo;
use crate::ai::tools::ToolResult;
use serde_json::json;
use std::path::{Component, Path, PathBuf};

use super::ai_chat_input::wrap_chat_input_rows;
use super::ai_chat_state::{
    CodeExplanationContinuation, CodeExplanationStep, PendingCodeExplanation,
};
use super::Editor;

const FALLBACK_SAFE_RANGE_LINES: usize = 40;
const WALKTHROUGH_CARD_RESERVED_ROWS: usize = 10;
const MAX_WALKTHROUGH_STEPS: usize = 32;
const MAX_COMMENT_BYTES: usize = 4 * 1024;
const MAX_COMMENT_ROWS: usize = 5;
const FALLBACK_COMMENT_WIDTH: usize = 76;

impl Editor {
    /// Conservative range that remains visible above the walkthrough card.
    /// The upper bound keeps the schema useful before the first render and on
    /// unusually tall terminals without encouraging broad, unfocused blocks.
    pub fn ai_code_explanation_safe_range_lines(&self) -> usize {
        let viewport = self.viewport_height();
        if viewport == 0 {
            FALLBACK_SAFE_RANGE_LINES
        } else {
            viewport
                .saturating_sub(WALKTHROUGH_CARD_RESERVED_ROWS)
                .clamp(1, FALLBACK_SAFE_RANGE_LINES)
        }
    }

    pub fn ai_code_explanation_summary(
        &self,
    ) -> Option<(usize, usize, String, usize, usize, String)> {
        let pending = self
            .ai_state
            .chat
            .as_ref()?
            .pending_code_explanation
            .as_ref()?;
        let step = pending.steps.get(pending.current)?;
        Some((
            pending.current + 1,
            pending.steps.len(),
            step.path.clone(),
            step.start_line,
            step.end_line,
            step.comment.clone(),
        ))
    }

    pub fn ai_chat_has_pending_code_explanation(&self) -> bool {
        self.ai_state
            .chat
            .as_ref()
            .is_some_and(|chat| chat.pending_code_explanation.is_some())
    }

    pub(super) fn begin_code_explanation(
        &mut self,
        tool_call: ToolCallInfo,
        continuation: CodeExplanationContinuation,
    ) -> Result<(), (ToolResult, Box<CodeExplanationContinuation>)> {
        let steps = match self.parse_code_explanation_steps(&tool_call.arguments) {
            Ok(steps) => steps,
            Err(error) => return Err((error, Box::new(continuation))),
        };
        let original_active_buffer_id = self
            .ai_state
            .chat
            .as_ref()
            .map(|chat| chat.active_buffer_id);
        let Some(original_active_buffer_id) = original_active_buffer_id else {
            return Err((
                ToolResult::Error("AI chat is not open".to_string()),
                Box::new(continuation),
            ));
        };

        let Some(chat) = self.ai_state.chat.as_mut() else {
            return Err((
                ToolResult::Error("AI chat is not open".to_string()),
                Box::new(continuation),
            ));
        };
        chat.pending_code_explanation = Some(PendingCodeExplanation {
            tool_call,
            steps,
            current: 0,
            original_active_buffer_id,
            continuation,
        });
        chat.waiting = false;

        if let Err(error) = self.show_current_code_explanation_step() {
            if let Some(pending) = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|chat| chat.pending_code_explanation.take())
            {
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.active_buffer_id = original_active_buffer_id;
                }
                return Err((error, Box::new(pending.continuation)));
            } else if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.active_buffer_id = original_active_buffer_id;
            }
            unreachable!("installed code walkthrough disappeared before activation");
        }

        self.ai_state.ai_attention_generation =
            self.ai_state.ai_attention_generation.saturating_add(1);
        self.set_lsp_status(
            "Code walkthrough ready — Left/Right steps, Enter advances, Esc dismisses".into(),
        );
        Ok(())
    }

    pub fn move_code_explanation(&mut self, forward: bool) -> bool {
        let changed = {
            let Some(pending) = self
                .ai_state
                .chat
                .as_mut()
                .and_then(|chat| chat.pending_code_explanation.as_mut())
            else {
                return false;
            };
            let next = if forward {
                (pending.current + 1).min(pending.steps.len().saturating_sub(1))
            } else {
                pending.current.saturating_sub(1)
            };
            if next == pending.current {
                false
            } else {
                pending.current = next;
                true
            }
        };
        if changed {
            if let Err(error) = self.show_current_code_explanation_step() {
                self.set_lsp_status(format!("Could not show walkthrough step: {error:?}"));
            }
        }
        changed
    }

    /// Enter advances through a walkthrough and only unblocks the agent from
    /// the final step. This avoids completing a multi-step explanation with
    /// one accidental key press on its first card.
    pub fn advance_or_finish_code_explanation(&mut self) -> bool {
        let is_last = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.pending_code_explanation.as_ref())
            .is_some_and(|pending| pending.current + 1 >= pending.steps.len());
        if is_last {
            self.finish_code_explanation(false)
        } else {
            self.move_code_explanation(true)
        }
    }

    pub fn finish_code_explanation(&mut self, dismissed: bool) -> bool {
        let Some(pending) = self
            .ai_state
            .chat
            .as_mut()
            .and_then(|chat| chat.pending_code_explanation.take())
        else {
            return false;
        };

        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.active_buffer_id = pending.original_active_buffer_id;
        }
        self.ai_state.active_selection = None;

        let outcome = if dismissed {
            format!(
                "User dismissed the code walkthrough at step {} of {}.",
                pending.current + 1,
                pending.steps.len()
            )
        } else {
            format!(
                "User completed the code walkthrough ({} steps).",
                pending.steps.len()
            )
        };
        let result = ToolResult::Success(outcome.clone());

        match pending.continuation {
            CodeExplanationContinuation::Dynamic {
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
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.waiting = true;
                }
            }
            CodeExplanationContinuation::Batch {
                runtime_tool,
                runtime_turn,
                remaining_tool_calls,
                model_name,
            } => {
                if let (Some(turn), Some(tool)) = (runtime_turn.as_ref(), runtime_tool.as_ref()) {
                    if let Err(error) = self.ai_runtime_finish_tool(turn, tool, &result) {
                        self.ai_runtime_fail_turn(format!(
                            "failed to record code walkthrough result: {error}"
                        ));
                        self.clear_streaming_state();
                        return true;
                    }
                }
                self.record_tool_event_summary(&pending.tool_call, &result);
                let result_content =
                    self.format_tool_result_with_target(&pending.tool_call, &result);
                if let Some(conversation) = self.conversation_mut() {
                    conversation.append_tool_result(pending.tool_call.id.clone(), result_content);
                }
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    chat.tool_call_count = chat.tool_call_count.saturating_add(1);
                    chat.waiting = true;
                }
                self.execute_tool_call_batch(remaining_tool_calls, model_name);
            }
        }

        self.set_lsp_status(outcome);
        true
    }

    fn parse_code_explanation_steps(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<Vec<CodeExplanationStep>, ToolResult> {
        let raw_steps = arguments
            .get("steps")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ToolResult::Error("'steps' must be a non-empty array".to_string()))?;
        if raw_steps.is_empty() {
            return Err(ToolResult::Error(
                "'steps' must contain at least one walkthrough step".to_string(),
            ));
        }
        if raw_steps.len() > MAX_WALKTHROUGH_STEPS {
            return Err(ToolResult::Error(format!(
                "walkthrough has {} steps; the maximum is {MAX_WALKTHROUGH_STEPS}",
                raw_steps.len()
            )));
        }

        let root = self
            .ai_effective_project_root()
            .ok_or_else(|| ToolResult::Error(self.no_project_root_error()))?;
        let root = root.canonicalize().unwrap_or(root);
        let safe_range = self.ai_code_explanation_safe_range_lines();
        let comment_width = self
            .render_cache
            .last_buffer_area
            // The card itself is capped at 100 columns with two border columns.
            .map(|area| area.width.saturating_sub(4).min(98) as usize)
            .filter(|width| *width > 0)
            .unwrap_or(FALLBACK_COMMENT_WIDTH);
        let mut steps = Vec::with_capacity(raw_steps.len());

        for (index, raw) in raw_steps.iter().enumerate() {
            let step_number = index + 1;
            let path = required_step_string(raw, "path", step_number)?;
            let comment = required_step_string(raw, "comment", step_number)?;
            if comment.len() > MAX_COMMENT_BYTES {
                return Err(ToolResult::Error(format!(
                    "step {step_number} comment exceeds {MAX_COMMENT_BYTES} bytes"
                )));
            }
            let comment_rows =
                wrap_chat_input_rows(&comment, comment_width, self.options.tab_width).len();
            if comment_rows > MAX_COMMENT_ROWS {
                return Err(ToolResult::Error(format!(
                    "step {step_number} comment wraps to {comment_rows} rows; keep it within {MAX_COMMENT_ROWS} rows or split the explanation into focused steps"
                )));
            }
            let start_line = required_step_line(raw, "start_line", step_number)?;
            let end_line = raw
                .get("end_line")
                .and_then(serde_json::Value::as_u64)
                .map(|line| line as usize)
                .unwrap_or(start_line);
            if end_line < start_line {
                return Err(ToolResult::Error(format!(
                    "step {step_number} end_line ({end_line}) must be >= start_line ({start_line})"
                )));
            }
            let range_lines = end_line - start_line + 1;
            if range_lines > safe_range {
                return Err(ToolResult::Error(format!(
                    "step {step_number} spans {range_lines} lines, but the current safe range is {safe_range}; split it into smaller conceptual steps"
                )));
            }

            let relative = Path::new(&path);
            if relative.is_absolute()
                || relative.components().any(|component| {
                    matches!(
                        component,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                return Err(ToolResult::Error(format!(
                    "step {step_number} path must be project-relative without '..': {path}"
                )));
            }
            let candidate = root.join(relative);
            let absolute_path = candidate.canonicalize().map_err(|error| {
                ToolResult::Error(format!("step {step_number} cannot open '{path}': {error}"))
            })?;
            if !absolute_path.starts_with(&root) || !absolute_path.is_file() {
                return Err(ToolResult::Error(format!(
                    "step {step_number} path is not a file inside the project: {path}"
                )));
            }
            let line_count = self.code_explanation_line_count(&absolute_path);
            if start_line > line_count {
                return Err(ToolResult::Error(format!(
                    "step {step_number} start_line ({start_line}) exceeds '{path}' line count ({line_count})"
                )));
            }

            steps.push(CodeExplanationStep {
                path,
                absolute_path,
                start_line,
                end_line: end_line.min(line_count),
                comment,
            });
        }
        Ok(steps)
    }

    fn code_explanation_line_count(&self, path: &Path) -> usize {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.buffers
            .iter()
            .find(|buffer| {
                buffer.file_path().is_some_and(|candidate| {
                    PathBuf::from(candidate)
                        .canonicalize()
                        .unwrap_or_else(|_| PathBuf::from(candidate))
                        == canonical
                })
            })
            .map(|buffer| buffer.rope().len_lines())
            .or_else(|| {
                std::fs::read_to_string(path)
                    .ok()
                    .map(|content| content.lines().count().max(1))
            })
            .unwrap_or(1)
    }

    fn show_current_code_explanation_step(&mut self) -> Result<(), ToolResult> {
        let step = self
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.pending_code_explanation.as_ref())
            .and_then(|pending| pending.steps.get(pending.current))
            .cloned()
            .ok_or_else(|| ToolResult::Error("code walkthrough has no current step".to_string()))?;

        let opened = self.handle_open_file_at_absolute_path(
            &step.absolute_path,
            &json!({ "line": step.start_line, "column": 1 }),
            false,
        );
        if let ToolResult::Error(error) = opened {
            return Err(ToolResult::Error(error));
        }
        let selected = self.execute_navigation_tool(
            "select_text",
            &json!({
                "start_line": step.start_line,
                "end_line": step.end_line,
            }),
        );
        if let ToolResult::Error(error) = selected {
            return Err(ToolResult::Error(error));
        }
        Ok(())
    }
}

fn required_step_string(
    raw: &serde_json::Value,
    field: &str,
    step: usize,
) -> Result<String, ToolResult> {
    raw.get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolResult::Error(format!("step {step} requires non-empty '{field}'")))
}

fn required_step_line(
    raw: &serde_json::Value,
    field: &str,
    step: usize,
) -> Result<usize, ToolResult> {
    raw.get(field)
        .and_then(serde_json::Value::as_u64)
        .filter(|line| *line > 0)
        .map(|line| line as usize)
        .ok_or_else(|| ToolResult::Error(format!("step {step} requires '{field}' >= 1")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::ChatOpts;

    fn setup_editor() -> (tempfile::TempDir, Editor, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".git")).expect("git marker");
        let first = dir.path().join("first.rs");
        let second = dir.path().join("second.rs");
        std::fs::write(
            &first,
            (1..=50)
                .map(|line| format!("// first {line}\n"))
                .collect::<String>(),
        )
        .expect("first file");
        std::fs::write(
            &second,
            (1..=20)
                .map(|line| format!("// second {line}\n"))
                .collect::<String>(),
        )
        .expect("second file");

        let mut editor = Editor::default();
        editor.open_file(&first).expect("open first");
        editor
            .open_ai_chat(ChatOpts {
                allow_edits: true,
                ..Default::default()
            })
            .expect("open chat");
        let profile = editor.ai_state.active_profile.clone();
        editor
            .ai_state
            .config
            .profiles
            .get_mut(&profile)
            .expect("active profile")
            .scope
            .files = crate::ai::FileScope::Project;
        editor.set_viewport_height(24);
        (dir, editor, first, second)
    }

    fn call(steps: serde_json::Value) -> ToolCallInfo {
        ToolCallInfo {
            id: "explain-1".into(),
            name: "explain_with_codebase".into(),
            arguments: json!({ "steps": steps }),
        }
    }

    fn batch_continuation() -> CodeExplanationContinuation {
        CodeExplanationContinuation::Batch {
            runtime_tool: None,
            runtime_turn: None,
            remaining_tool_calls: Vec::new(),
            model_name: "test".into(),
        }
    }

    #[test]
    fn safe_range_reserves_space_for_the_walkthrough_card() {
        let mut editor = Editor::default();
        editor.set_viewport_height(0);
        assert_eq!(editor.ai_code_explanation_safe_range_lines(), 40);
        editor.set_viewport_height(24);
        assert_eq!(editor.ai_code_explanation_safe_range_lines(), 14);
        editor.set_viewport_height(6);
        assert_eq!(editor.ai_code_explanation_safe_range_lines(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn walkthrough_opens_and_selects_each_step_without_retargeting_the_agent() {
        let (_dir, mut editor, first, second) = setup_editor();
        let original_target = editor.ai_state.chat.as_ref().unwrap().active_buffer_id;
        let tool_call = call(json!([
            {
                "path": "first.rs",
                "start_line": 2,
                "end_line": 4,
                "comment": "The entry point validates the request."
            },
            {
                "path": "second.rs",
                "start_line": 7,
                "comment": "The handoff occurs here."
            }
        ]));

        if let Err((error, _)) = editor.begin_code_explanation(tool_call, batch_continuation()) {
            panic!("could not start walkthrough: {error:?}");
        }
        assert!(editor.ai_chat_has_pending_code_explanation());
        assert_eq!(
            PathBuf::from(editor.buffer().file_path().unwrap())
                .canonicalize()
                .unwrap(),
            first.canonicalize().unwrap()
        );
        assert_eq!(editor.ai_code_explanation_summary().unwrap().0, 1);
        let selection = editor.ai_state.active_selection.as_ref().unwrap();
        assert_eq!((selection.start_line, selection.end_line), (1, 3));

        assert!(editor.move_code_explanation(true));
        assert_eq!(
            PathBuf::from(editor.buffer().file_path().unwrap())
                .canonicalize()
                .unwrap(),
            second.canonicalize().unwrap()
        );
        assert_eq!(editor.ai_code_explanation_summary().unwrap().0, 2);

        editor.finish_code_explanation(true);
        assert!(!editor.ai_chat_has_pending_code_explanation());
        assert_eq!(
            editor.ai_state.chat.as_ref().unwrap().active_buffer_id,
            original_target
        );
        assert!(editor.ai_state.active_selection.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn walkthrough_rejects_ranges_larger_than_the_live_safe_range() {
        let (_dir, mut editor, _first, _second) = setup_editor();
        let tool_call = call(json!([{
            "path": "first.rs",
            "start_line": 1,
            "end_line": 17,
            "comment": "Too broad."
        }]));

        let error = editor
            .begin_code_explanation(tool_call, batch_continuation())
            .expect_err("oversized range should fail")
            .0;
        let ToolResult::Error(error) = error else {
            panic!("expected tool error")
        };
        assert!(error.contains("safe range is 14"), "{error}");
        assert!(!editor.ai_chat_has_pending_code_explanation());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enter_advances_before_it_completes_the_walkthrough() {
        let (_dir, mut editor, _first, _second) = setup_editor();
        let tool_call = call(json!([
            {
                "path": "first.rs",
                "start_line": 2,
                "comment": "First concept."
            },
            {
                "path": "second.rs",
                "start_line": 7,
                "comment": "Second concept."
            }
        ]));
        if let Err((error, _)) = editor.begin_code_explanation(tool_call, batch_continuation()) {
            panic!("could not start walkthrough: {error:?}");
        }

        assert!(editor.advance_or_finish_code_explanation());
        assert_eq!(editor.ai_code_explanation_summary().unwrap().0, 2);
        assert!(editor.ai_chat_has_pending_code_explanation());

        assert!(editor.advance_or_finish_code_explanation());
        assert!(!editor.ai_chat_has_pending_code_explanation());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn walkthrough_rejects_comments_that_cannot_fit_in_the_card() {
        let (_dir, mut editor, _first, _second) = setup_editor();
        editor.set_last_layout(
            crate::Rect {
                x: 0,
                y: 0,
                width: 32,
                height: 24,
            },
            0,
            32,
            0,
        );
        let tool_call = call(json!([{
            "path": "first.rs",
            "start_line": 1,
            "comment": "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty twenty-one twenty-two twenty-three twenty-four twenty-five twenty-six twenty-seven twenty-eight"
        }]));

        let error = editor
            .begin_code_explanation(tool_call, batch_continuation())
            .expect_err("long comment should fail")
            .0;
        let ToolResult::Error(error) = error else {
            panic!("expected tool error")
        };
        assert!(error.contains("comment wraps to"), "{error}");
        assert!(error.contains("within 5 rows"), "{error}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn walkthrough_schema_contains_structured_steps_and_live_range_guidance() {
        let (_dir, editor, _first, _second) = setup_editor();
        let profile = editor
            .ai_state
            .config
            .resolve_profile(&editor.ai_state.active_profile)
            .unwrap();
        let schemas = editor.build_tool_schemas_for_chat(profile);
        let schema = schemas
            .iter()
            .find(|schema| schema["function"]["name"] == "explain_with_codebase")
            .expect("walkthrough schema");

        assert!(schema["function"]["description"]
            .as_str()
            .unwrap()
            .contains("at most 14 code lines"));
        let steps = &schema["function"]["parameters"]["properties"]["steps"];
        assert_eq!(steps["type"], "array");
        assert_eq!(
            steps["items"]["required"],
            json!(["path", "start_line", "comment"])
        );
        assert!(steps["items"]["properties"]["end_line"].is_object());
    }
}
