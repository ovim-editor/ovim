use crate::ai::chat_types::ToolCallInfo;
use crate::ai::tools::ToolResult;
use serde_json::json;
use std::path::{Component, Path, PathBuf};

use super::ai_chat_state::{CodeExplanationContinuation, PendingCodeExplanation};
use super::code_explanation::{
    comment_rows_for_viewport, safe_code_rows, CodeExplanationStep, CodeExplanationView,
    MAX_WALKTHROUGH_COMMENT_BYTES, MAX_WALKTHROUGH_COMMENT_ROWS, MAX_WALKTHROUGH_STEPS,
};
use super::Editor;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodeExplanationSourceMetrics {
    line_count: usize,
    visual_rows: Option<usize>,
    line_visual_rows: Option<Vec<(usize, usize)>>,
}

impl Editor {
    /// Conservative range that remains visible above the walkthrough card.
    /// The upper bound keeps the schema useful before the first render and on
    /// unusually tall terminals without encouraging broad, unfocused blocks.
    pub fn ai_code_explanation_safe_range_lines(&self) -> usize {
        let viewport = self
            .render_cache
            .last_buffer_area
            .map(|area| area.height as usize)
            .unwrap_or_else(|| self.viewport_height());
        safe_code_rows((viewport > 0).then_some(viewport))
    }

    pub fn ai_code_explanation_view(&self) -> Option<CodeExplanationView> {
        let pending = self
            .ai_state
            .chat
            .as_ref()?
            .pending_code_explanation
            .as_ref()?;
        let step = pending.steps.get(pending.current)?;
        Some(CodeExplanationView {
            current: pending.current + 1,
            total: pending.steps.len(),
            path: step.path.clone(),
            start_line: step.start_line,
            end_line: step.end_line,
            comment: step.comment.clone(),
        })
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
        let presentation_width = self.ai_code_explanation_presentation_width();
        let mut steps = Vec::with_capacity(raw_steps.len());

        for (index, raw) in raw_steps.iter().enumerate() {
            let step_number = index + 1;
            let path = required_step_string(raw, "path", step_number)?;
            let comment = required_step_string(raw, "comment", step_number)?;
            if comment.len() > MAX_WALKTHROUGH_COMMENT_BYTES {
                return Err(ToolResult::Error(format!(
                    "step {step_number} comment exceeds {MAX_WALKTHROUGH_COMMENT_BYTES} bytes"
                )));
            }
            let comment_rows =
                comment_rows_for_viewport(presentation_width, &comment, self.options.tab_width);
            if comment_rows > MAX_WALKTHROUGH_COMMENT_ROWS {
                return Err(ToolResult::Error(format!(
                    "step {step_number} comment wraps to {comment_rows} rows; keep it within {MAX_WALKTHROUGH_COMMENT_ROWS} rows or split the explanation into focused steps"
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
                    "step {step_number} '{path}:{start_line}-{end_line}' spans {range_lines} physical lines, but the current maximum is {safe_range}; split it into smaller conceptual steps"
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
            let wrap_width =
                (self.options.wrap && self.render_cache.last_text_width > 0).then(|| {
                    self.render_cache.last_text_width.saturating_add(
                        self.render_cache
                            .last_chat_area
                            .map_or(0, |area| area.width as usize),
                    )
                });
            let metrics = self.code_explanation_source_metrics(
                &absolute_path,
                start_line,
                end_line,
                wrap_width,
            );
            if start_line > metrics.line_count {
                return Err(ToolResult::Error(format!(
                    "step {step_number} start_line ({start_line}) exceeds '{path}' line count ({})",
                    metrics.line_count
                )));
            }
            let end_line = end_line.min(metrics.line_count);
            if let Some(visual_rows) = metrics.visual_rows {
                if visual_rows > safe_range {
                    let guidance = metrics
                        .line_visual_rows
                        .as_deref()
                        .map(|rows| visual_overflow_guidance(rows, safe_range))
                        .unwrap_or_default();
                    return Err(ToolResult::Error(format!(
                        "step {step_number} '{path}:{start_line}-{end_line}' occupies {visual_rows} visual rows after soft wrapping (maximum {safe_range}).{guidance} Split by concept rather than truncating the explanation arbitrarily."
                    )));
                }
            }

            steps.push(CodeExplanationStep {
                path,
                absolute_path,
                start_line,
                end_line,
                comment,
            });
        }
        Ok(steps)
    }

    fn ai_code_explanation_presentation_width(&self) -> Option<u16> {
        self.render_cache.last_buffer_area.map(|buffer| {
            buffer.width.saturating_add(
                self.render_cache
                    .last_chat_area
                    .map_or(0, |chat| chat.width),
            )
        })
    }

    fn code_explanation_source_metrics(
        &self,
        path: &Path,
        start_line: usize,
        end_line: usize,
        wrap_width: Option<usize>,
    ) -> CodeExplanationSourceMetrics {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Some(buffer) = self.buffers.iter().find(|buffer| {
            buffer.file_path().is_some_and(|candidate| {
                PathBuf::from(candidate)
                    .canonicalize()
                    .unwrap_or_else(|_| PathBuf::from(candidate))
                    == canonical
            })
        }) {
            let line_count = buffer.rope().len_lines();
            let line_visual_rows = wrap_width.map(|width| {
                (start_line.saturating_sub(1)..end_line.min(line_count))
                    .map(|line| {
                        (
                            line + 1,
                            crate::wrap::visual_line_count(
                                buffer.line_text(line).as_deref().unwrap_or(""),
                                width,
                                self.options.tab_width,
                            ),
                        )
                    })
                    .collect::<Vec<_>>()
            });
            let visual_rows = line_visual_rows
                .as_ref()
                .map(|rows| rows.iter().map(|(_, count)| count).sum());
            return CodeExplanationSourceMetrics {
                line_count,
                visual_rows,
                line_visual_rows,
            };
        }

        let content = std::fs::read_to_string(path).unwrap_or_default();
        let lines = content.lines().collect::<Vec<_>>();
        let line_count = lines.len().max(1);
        let line_visual_rows = wrap_width.map(|width| {
            (start_line.saturating_sub(1)..end_line.min(line_count))
                .map(|line| {
                    (
                        line + 1,
                        crate::wrap::visual_line_count(
                            lines.get(line).copied().unwrap_or(""),
                            width,
                            self.options.tab_width,
                        ),
                    )
                })
                .collect::<Vec<_>>()
        });
        let visual_rows = line_visual_rows
            .as_ref()
            .map(|rows| rows.iter().map(|(_, count)| count).sum());
        CodeExplanationSourceMetrics {
            line_count,
            visual_rows,
            line_visual_rows,
        }
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
        // `select_text` centers the midpoint for general navigation. A
        // walkthrough instead owns the bottom rows with its card, so pin the
        // range's first line to the top and let the validated visual-row budget
        // flow downward without being obscured.
        self.buffer_mut().cursor_mut().set_position(
            step.start_line.saturating_sub(1),
            crate::unicode::GraphemeCol(0),
        );
        self.move_cursor_line_to_top_with_offset(0);
        Ok(())
    }
}

fn visual_overflow_guidance(line_rows: &[(usize, usize)], safe_range: usize) -> String {
    let mut used = 0usize;
    let mut safe_endpoint = None;
    for (line, rows) in line_rows {
        if used.saturating_add(*rows) > safe_range {
            break;
        }
        used = used.saturating_add(*rows);
        safe_endpoint = Some(*line);
    }

    let endpoint = if let Some(line) = safe_endpoint {
        format!(" Suggested safe endpoint from this start: line {line}.")
    } else if let Some((line, rows)) = line_rows.first() {
        format!(" No endpoint from this start fits: line {line} alone occupies {rows} visual rows.")
    } else {
        String::new()
    };

    let mut longest = line_rows
        .iter()
        .copied()
        .filter(|(_, rows)| *rows > 1)
        .collect::<Vec<_>>();
    longest.sort_by_key(|(line, rows)| (std::cmp::Reverse(*rows), *line));
    let longest = longest
        .into_iter()
        .take(3)
        .map(|(line, rows)| format!("{line} ({rows} rows)"))
        .collect::<Vec<_>>();
    let longest = if longest.is_empty() {
        String::new()
    } else {
        format!(" Longest wrapped lines: {}.", longest.join(", "))
    };

    format!("{endpoint}{longest}")
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
        editor.set_last_layout(
            crate::Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 18,
            },
            0,
            72,
            0,
        );
        assert_eq!(editor.ai_code_explanation_safe_range_lines(), 8);
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
        assert_eq!(editor.ai_code_explanation_view().unwrap().current, 1);
        let selection = editor.ai_state.active_selection.as_ref().unwrap();
        assert_eq!((selection.start_line, selection.end_line), (1, 3));
        assert_eq!(editor.scroll_offset(), 1);

        assert!(editor.move_code_explanation(true));
        assert_eq!(
            PathBuf::from(editor.buffer().file_path().unwrap())
                .canonicalize()
                .unwrap(),
            second.canonicalize().unwrap()
        );
        assert_eq!(editor.ai_code_explanation_view().unwrap().current, 2);

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
        assert!(error.contains("maximum is 14"), "{error}");
        assert!(error.contains("first.rs:1-17"), "{error}");
        assert!(!editor.ai_chat_has_pending_code_explanation());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn walkthrough_rejects_logical_ranges_that_overflow_after_wrapping() {
        let (dir, mut editor, _first, _second) = setup_editor();
        let wrapped = dir.path().join("wrapped.rs");
        std::fs::write(
            &wrapped,
            (1..=20)
                .map(|line| format!("// line {line} has enough words to wrap several times\n"))
                .collect::<String>(),
        )
        .unwrap();
        editor.options.wrap = true;
        editor.set_last_layout(
            crate::Rect {
                x: 0,
                y: 0,
                width: 24,
                height: 24,
            },
            4,
            12,
            0,
        );
        let tool_call = call(json!([{
            "path": "wrapped.rs",
            "start_line": 1,
            "end_line": 4,
            "comment": "A logically short but visually tall range."
        }]));

        let error = editor
            .begin_code_explanation(tool_call, batch_continuation())
            .expect_err("wrapped range should fail")
            .0;
        let ToolResult::Error(error) = error else {
            panic!("expected tool error")
        };
        assert!(error.contains("visual rows"), "{error}");
        assert!(error.contains("maximum 14"), "{error}");
        assert!(error.contains("wrapped.rs:1-4"), "{error}");
        assert!(error.contains("Suggested safe endpoint"), "{error}");
        assert!(error.contains("Longest wrapped lines"), "{error}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn walkthrough_validation_uses_the_full_width_that_replaces_chat() {
        let (dir, mut editor, _first, _second) = setup_editor();
        let wide = dir.path().join("wide.rs");
        std::fs::write(
            &wide,
            (1..=4)
                .map(|line| format!("// {line} {}\n", "word ".repeat(10)))
                .collect::<String>(),
        )
        .unwrap();
        editor.options.wrap = true;
        editor.set_last_layout(
            crate::Rect {
                x: 0,
                y: 0,
                width: 30,
                height: 24,
            },
            4,
            10,
            0,
        );
        editor.render_cache.last_chat_area = Some(crate::Rect {
            x: 30,
            y: 0,
            width: 50,
            height: 24,
        });
        let tool_call = call(json!([{
            "path": "wide.rs",
            "start_line": 1,
            "end_line": 4,
            "comment": "Four related declarations form one cohesive block."
        }]));

        if let Err((error, _)) = editor.begin_code_explanation(tool_call, batch_continuation()) {
            panic!("full-width walkthrough should fit: {error:?}");
        }
        assert!(editor.ai_chat_has_pending_code_explanation());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn source_metrics_prefer_unsaved_buffer_content_over_disk() {
        let (_dir, mut editor, _first, _second) = setup_editor();
        editor
            .buffer_mut()
            .replace_all("an unsaved line that wraps\nsecond line\n");
        let path = PathBuf::from(editor.buffer().file_path().unwrap());

        let metrics = editor.code_explanation_source_metrics(&path, 1, 1, Some(8));

        assert_eq!(metrics.line_count, 3);
        assert!(metrics.visual_rows.unwrap() > 1);
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
        assert_eq!(editor.ai_code_explanation_view().unwrap().current, 2);
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
            .contains("at most 14 visual code rows"));
        assert!(schema["function"]["description"]
            .as_str()
            .unwrap()
            .contains("hides chat"));
        assert!(schema["function"]["description"]
            .as_str()
            .unwrap()
            .contains("explain something about the code or codebase"));
        assert!(schema["function"]["description"]
            .as_str()
            .unwrap()
            .contains("revisit the same line or range"));
        let steps = &schema["function"]["parameters"]["properties"]["steps"];
        assert_eq!(steps["type"], "array");
        assert_eq!(
            steps["items"]["required"],
            json!(["path", "start_line", "comment"])
        );
        assert!(steps["items"]["properties"]["end_line"].is_object());
        assert!(steps["description"]
            .as_str()
            .unwrap()
            .contains("single-line anchors"));
        assert!(steps["description"]
            .as_str()
            .unwrap()
            .contains("Repeating a range is encouraged"));
        assert!(steps["items"]["properties"]["comment"]["description"]
            .as_str()
            .unwrap()
            .contains("do not merely paraphrase"));
    }
}
