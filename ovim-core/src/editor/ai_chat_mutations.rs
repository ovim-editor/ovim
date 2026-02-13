use crate::ai::tools::ToolResult;

use super::ai_integration::remap_abs_char_through_edits;
use super::Editor;

// -----------------------------------------------------------------
// Arg extraction helpers
// -----------------------------------------------------------------

/// Extract a required u64 arg as usize, with minimum value check.
fn required_u64(args: &serde_json::Value, name: &str, min: u64) -> Result<usize, ToolResult> {
    match args.get(name).and_then(|v| v.as_u64()) {
        Some(n) if n >= min => Ok(n as usize),
        _ => Err(ToolResult::Error(format!(
            "'{name}' is required (>= {min})"
        ))),
    }
}

/// Extract a required string arg.
fn required_str(args: &serde_json::Value, name: &str) -> Result<String, ToolResult> {
    match args.get(name).and_then(|v| v.as_str()) {
        Some(s) => Ok(s.to_string()),
        None => Err(ToolResult::Error(format!("'{name}' is required"))),
    }
}

impl Editor {
    // -----------------------------------------------------------------
    // Mutation tool dispatch
    // -----------------------------------------------------------------

    /// Execute a mutation tool with `&mut self` access.
    pub(crate) fn execute_mutation_tool(
        &mut self,
        name: &str,
        args: &serde_json::Value,
    ) -> ToolResult {
        // Verify the tool exists and is a mutation
        let tool_def = match self.ai_state.tool_registry.get(name) {
            Some(t) => t.clone(),
            None => return ToolResult::Error(format!("unknown tool: {name}")),
        };

        // Check capabilities allow this side effect
        let caps = self.build_chat_capabilities();
        if !caps.allows_side_effect(tool_def.side_effect) {
            return ToolResult::Error(format!(
                "tool '{}' blocked: mutations not allowed in current context",
                name
            ));
        }
        if !caps.contains(&tool_def.required_scope) {
            return ToolResult::Error(format!(
                "tool '{}' requires scope not granted by current context",
                name
            ));
        }

        // Resolve target buffer from the chat session, then dispatch.
        let original = self.current_buffer_index;
        let target = self
            .ai_state
            .chat
            .as_ref()
            .map(|c| c.active_buffer_id)
            .unwrap_or(original);
        self.current_buffer_index = target;

        let result = match name {
            "edit_range" => self.handle_edit_range(args),
            "insert_lines" => self.handle_insert_lines(args),
            "delete_lines" => self.handle_delete_lines(args),
            _ => ToolResult::Error(format!("unknown mutation tool: {name}")),
        };

        self.current_buffer_index = original;
        result
    }

    /// Shared post-mutation refresh: rehighlight, diagnostics, mark dirty.
    fn post_mutation_refresh(&mut self) {
        if self.buffer().needs_rehighlight() {
            self.process_viewport_rehighlight();
        }
        self.request_diagnostics_refresh();
        self.ai_state.last_observed_buffer_version = self.buffer().version();
        self.mark_dirty();
    }

    // -----------------------------------------------------------------
    // Mutation handlers
    // -----------------------------------------------------------------

    fn handle_edit_range(&mut self, args: &serde_json::Value) -> ToolResult {
        let start_line = match required_u64(args, "start_line", 1) {
            Ok(n) => n,
            Err(e) => return e,
        };
        let end_line = match required_u64(args, "end_line", 1) {
            Ok(n) => n,
            Err(e) => return e,
        };
        let new_text = match required_str(args, "new_text") {
            Ok(s) => s,
            Err(e) => return e,
        };

        if start_line > end_line {
            return ToolResult::Error(format!(
                "start_line ({start_line}) must be <= end_line ({end_line})"
            ));
        }

        let line_count = self.buffer().rope().len_lines();
        if start_line > line_count {
            return ToolResult::Error(format!(
                "start_line ({start_line}) exceeds buffer line count ({line_count})"
            ));
        }
        let end_line = end_line.min(line_count);

        // Convert 1-indexed lines to char range
        let start_char = self.buffer().rope().line_to_char(start_line - 1);
        let end_char = if end_line >= line_count {
            self.buffer().rope().len_chars()
        } else {
            self.buffer().rope().line_to_char(end_line)
        };

        let cursor_before = self.cursor_position();
        let cursor_abs_before = self.cursor_abs_char();

        let ((), edits) = self.buffer_mut().record(|buf| {
            if end_char > start_char {
                buf.delete_char_range(start_char, end_char);
            }
            let insert_pos = start_char.min(buf.rope().len_chars());
            let line = buf.rope().char_to_line(insert_pos);
            let col = insert_pos - buf.rope().line_to_char(line);
            if !new_text.is_empty() {
                let mut text = new_text.clone();
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                buf.insert_text_at(line, col, &text);
            }
        });

        if edits.is_empty() {
            return ToolResult::Success("No changes made (content identical).".to_string());
        }

        let cursor_abs_after = remap_abs_char_through_edits(cursor_abs_before, &edits)
            .min(self.buffer().rope().len_chars());
        self.set_cursor_from_abs_char(cursor_abs_after);
        let cursor_after = self.cursor_position();
        self.push_recorded_undo(edits, cursor_before, cursor_after);
        self.post_mutation_refresh();

        let new_line_count = self.buffer().rope().len_lines();
        ToolResult::Success(format!(
            "Replaced lines {start_line}-{end_line} (buffer now has {new_line_count} lines)."
        ))
    }

    fn handle_insert_lines(&mut self, args: &serde_json::Value) -> ToolResult {
        let after_line = match required_u64(args, "after_line", 0) {
            Ok(n) => n,
            Err(e) => return e,
        };
        let text = match required_str(args, "text") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let line_count = self.buffer().rope().len_lines();
        if after_line > line_count {
            return ToolResult::Error(format!(
                "after_line ({after_line}) exceeds buffer line count ({line_count})"
            ));
        }

        // Compute insertion offset
        let insert_char = if after_line == 0 {
            0
        } else if after_line >= line_count {
            self.buffer().rope().len_chars()
        } else {
            self.buffer().rope().line_to_char(after_line)
        };

        let mut insert_text = text;
        if !insert_text.ends_with('\n') {
            insert_text.push('\n');
        }
        // If inserting at end and buffer doesn't end with newline, prepend one
        if after_line >= line_count && self.buffer().rope().len_chars() > 0 {
            let last_char = self
                .buffer()
                .rope()
                .char(self.buffer().rope().len_chars() - 1);
            if last_char != '\n' {
                insert_text = format!("\n{insert_text}");
            }
        }

        let cursor_before = self.cursor_position();
        let cursor_abs_before = self.cursor_abs_char();

        let ((), edits) = self.buffer_mut().record(|buf| {
            let pos = insert_char.min(buf.rope().len_chars());
            let line = buf.rope().char_to_line(pos);
            let col = pos - buf.rope().line_to_char(line);
            buf.insert_text_at(line, col, &insert_text);
        });

        if edits.is_empty() {
            return ToolResult::Success("No changes made.".to_string());
        }

        let cursor_abs_after = remap_abs_char_through_edits(cursor_abs_before, &edits)
            .min(self.buffer().rope().len_chars());
        self.set_cursor_from_abs_char(cursor_abs_after);
        let cursor_after = self.cursor_position();
        self.push_recorded_undo(edits, cursor_before, cursor_after);
        self.post_mutation_refresh();

        let new_line_count = self.buffer().rope().len_lines();
        ToolResult::Success(format!(
            "Inserted text after line {after_line} (buffer now has {new_line_count} lines)."
        ))
    }

    fn handle_delete_lines(&mut self, args: &serde_json::Value) -> ToolResult {
        let start_line = match required_u64(args, "start_line", 1) {
            Ok(n) => n,
            Err(e) => return e,
        };
        let end_line = match required_u64(args, "end_line", 1) {
            Ok(n) => n,
            Err(e) => return e,
        };

        if start_line > end_line {
            return ToolResult::Error(format!(
                "start_line ({start_line}) must be <= end_line ({end_line})"
            ));
        }

        let line_count = self.buffer().rope().len_lines();
        if start_line > line_count {
            return ToolResult::Error(format!(
                "start_line ({start_line}) exceeds buffer line count ({line_count})"
            ));
        }
        let end_line = end_line.min(line_count);

        let start_char = self.buffer().rope().line_to_char(start_line - 1);
        let end_char = if end_line >= line_count {
            self.buffer().rope().len_chars()
        } else {
            self.buffer().rope().line_to_char(end_line)
        };

        if end_char <= start_char {
            return ToolResult::Success("No lines to delete.".to_string());
        }

        let cursor_before = self.cursor_position();
        let cursor_abs_before = self.cursor_abs_char();

        let ((), edits) = self.buffer_mut().record(|buf| {
            buf.delete_char_range(start_char, end_char);
        });

        if edits.is_empty() {
            return ToolResult::Success("No changes made.".to_string());
        }

        let cursor_abs_after = remap_abs_char_through_edits(cursor_abs_before, &edits)
            .min(self.buffer().rope().len_chars());
        self.set_cursor_from_abs_char(cursor_abs_after);
        let cursor_after = self.cursor_position();
        self.push_recorded_undo(edits, cursor_before, cursor_after);
        self.post_mutation_refresh();

        let deleted_count = end_line - start_line + 1;
        let new_line_count = self.buffer().rope().len_lines();
        ToolResult::Success(format!(
            "Deleted {deleted_count} line(s) ({start_line}-{end_line}). Buffer now has {new_line_count} lines."
        ))
    }
}
