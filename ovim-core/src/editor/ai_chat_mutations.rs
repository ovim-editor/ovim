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

/// Extract a context snippet around a line range from a rope, with line numbers.
/// `start_line` and `end_line` are 1-indexed, inclusive.
fn snippet_around(
    rope: &ropey::Rope,
    start_line: usize,
    end_line: usize,
    context: usize,
) -> String {
    let total = rope.len_lines();
    if total == 0 {
        return String::new();
    }
    // Convert to 0-indexed
    let first = start_line.saturating_sub(1).saturating_sub(context);
    let last = end_line.min(total).saturating_add(context).min(total);

    let mut out = String::new();
    for i in first..last {
        let line_content = rope.line(i);
        // Trim trailing newline from ropey line slice
        let s = line_content.to_string();
        let s = s.trim_end_matches('\n');
        out.push_str(&format!("{:>4} | {}\n", i + 1, s));
    }
    out
}

impl Editor {
    // -----------------------------------------------------------------
    // Navigation tool dispatch
    // -----------------------------------------------------------------

    /// Execute a navigation tool with `&mut self` access.
    /// Navigation tools are always allowed, even when edits are disabled.
    pub(crate) fn execute_navigation_tool(
        &mut self,
        name: &str,
        args: &serde_json::Value,
    ) -> ToolResult {
        let tool_def = match self.ai_state.tool_registry.get(name) {
            Some(t) => t.clone(),
            None => return ToolResult::Error(format!("unknown tool: {name}")),
        };

        let caps = self.build_chat_capabilities();
        if !caps.contains(&tool_def.required_scope) {
            return ToolResult::Error(format!(
                "tool '{}' requires scope not granted by current context",
                name
            ));
        }

        match name {
            "open_file" => self.handle_open_file(args),
            "select_text" => self.handle_select_text(args),
            _ => ToolResult::Error(format!("unknown navigation tool: {name}")),
        }
    }

    // -----------------------------------------------------------------
    // Navigation handlers
    // -----------------------------------------------------------------

    fn handle_open_file(&mut self, args: &serde_json::Value) -> ToolResult {
        let rel_path = match required_str(args, "path") {
            Ok(s) => s,
            Err(e) => return e,
        };

        if rel_path.contains("..") {
            return ToolResult::Error("path traversal (..) not allowed".to_string());
        }

        let project_root =
            match std::env::current_dir() {
                Ok(root) => root,
                Err(_) => return ToolResult::Error(
                    "No project root detected. Project-level tools require a working directory."
                        .to_string(),
                ),
            };

        let candidate = project_root.join(&rel_path);
        // Validate it stays within project root
        let normalized = candidate.canonicalize().unwrap_or(candidate.clone());
        let root_normalized = project_root.canonicalize().unwrap_or(project_root.clone());
        if !normalized.starts_with(&root_normalized) {
            return ToolResult::Error("path is outside project root".to_string());
        }

        if !candidate.is_file() {
            return ToolResult::Error(format!(
                "'{}' is not a file. Use list_files to see available files.",
                rel_path
            ));
        }

        // Open the file (switches to existing buffer or creates new one)
        if let Err(e) = self.open_file(&candidate) {
            return ToolResult::Error(format!("failed to open '{}': {}", rel_path, e));
        }

        // Navigate to requested position (1-indexed -> 0-indexed)
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

        // Clamp to buffer bounds
        let max_line = self.buffer().rope().len_lines().saturating_sub(1);
        let target_line = line.min(max_line);
        self.buffer_mut()
            .cursor_mut()
            .set_position(target_line, col);
        self.buffer_mut().validate_cursor_position();
        self.center_cursor_in_viewport();

        // Also update the chat's active_buffer_id to point at the newly opened buffer.
        // Keep chat view mode unchanged; users can explicitly enter review focus.
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.active_buffer_id = self.current_buffer_index;
        }

        // Return a snippet around the target position
        let actual_line = self.buffer().cursor().line() + 1; // back to 1-indexed
        let actual_col = self.buffer().cursor().col() + 1;
        let total_lines = self.buffer().rope().len_lines();
        let snip = snippet_around(self.buffer().rope(), actual_line, actual_line, 5);
        ToolResult::Success(format!(
            "Opened {} at line {}, column {} ({} lines total).\n{}",
            rel_path, actual_line, actual_col, total_lines, snip
        ))
    }

    fn handle_select_text(&mut self, args: &serde_json::Value) -> ToolResult {
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

        // Convert 1-indexed to 0-indexed
        let start_line_0 = start_line - 1;
        let end_line_0 = end_line - 1;

        let start_col = args
            .get("start_column")
            .and_then(|v| v.as_u64())
            .map(|n| n.saturating_sub(1) as usize)
            .unwrap_or(0);

        let end_col = args
            .get("end_column")
            .and_then(|v| v.as_u64())
            .map(|n| n.saturating_sub(1) as usize)
            .unwrap_or_else(|| {
                // Default to end of the end_line
                let rope = self.buffer().rope();
                let line_str = rope.line(end_line_0).to_string();
                let trimmed = line_str.trim_end_matches('\n');
                crate::unicode::grapheme_count(trimmed)
            });

        // Compute char offsets for the selection snapshot
        let rope = self.buffer().rope();
        let start_char = rope.line_to_char(start_line_0)
            + crate::unicode::grapheme_to_char_col(&rope.line(start_line_0).to_string(), start_col);
        let end_char = rope.line_to_char(end_line_0)
            + crate::unicode::grapheme_to_char_col(&rope.line(end_line_0).to_string(), end_col);

        // Store as active selection (same structure used by AI prompt mode)
        let selected_text = if end_char > start_char {
            rope.slice(start_char..end_char.min(rope.len_chars()))
                .to_string()
        } else {
            String::new()
        };

        self.ai_state.active_selection = Some(crate::editor::ai_state::AiSelectionSnapshot {
            start_line: start_line_0,
            start_col,
            end_line: end_line_0,
            end_col,
            start_char,
            end_char,
            anchor_line: start_line_0,
            selected_text: selected_text.clone(),
            mode_before_prompt: self.mode(),
        });

        // Move cursor to the midpoint and center
        let mid_line = (start_line_0 + end_line_0) / 2;
        self.buffer_mut().cursor_mut().set_position(mid_line, 0);
        self.center_cursor_in_viewport();

        let file_label = self.buffer().file_path().unwrap_or("[No Name]").to_string();
        let snip = snippet_around(self.buffer().rope(), start_line, end_line, 2);
        ToolResult::Success(format!(
            "Selected lines {start_line}-{end_line} in {file_label} ({} chars).\n{snip}",
            selected_text.len()
        ))
    }

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

        // Validate target buffer index
        if target >= self.buffers.len() {
            return ToolResult::Error(format!(
                "target buffer index {} is out of range ({})",
                target,
                self.buffers.len()
            ));
        }

        self.current_buffer_index = target;

        let result = match name {
            "edit_range" => self.handle_edit_range(args),
            "insert_lines" => self.handle_insert_lines(args),
            "delete_lines" => self.handle_delete_lines(args),
            _ => ToolResult::Error(format!("unknown mutation tool: {name}")),
        };

        // Only restore if active_buffer_id didn't change during the mutation
        // (e.g., open_file could have changed it)
        let current_active = self.ai_state.chat.as_ref().map(|c| c.active_buffer_id);
        if current_active == Some(target) {
            self.current_buffer_index = original;
        }
        result
    }

    /// Shared post-mutation refresh: rehighlight, diagnostics, mark dirty, auto-save.
    fn post_mutation_refresh(&mut self) {
        if self.buffer().needs_rehighlight() {
            self.process_viewport_rehighlight();
        }
        self.request_diagnostics_refresh();
        self.ai_state.last_observed_buffer_version = self.buffer().version();
        self.mark_dirty();

        // Auto-save agent edits to disk when buffer was clean at chat start
        let should_save = self.buffer().file_path().is_some()
            && self
                .ai_state
                .chat
                .as_ref()
                .is_some_and(|c| c.buffer_was_clean_at_chat_start);
        if should_save && self.buffer_mut().save().is_ok() {
            self.mark_saved();
        }
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
        let new_text_lines = new_text.lines().count().max(1);
        let new_end = start_line + new_text_lines - 1;

        // Record agent edit (convert 1-indexed to 0-indexed)
        let buf_idx = self.current_buffer_index;
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.agent_edits
                .record_edit(buf_idx, start_line - 1, new_end - 1);
        }

        let snippet = snippet_around(self.buffer().rope(), start_line, new_end, 3);
        ToolResult::Success(format!(
            "Replaced lines {start_line}-{end_line} (buffer now has {new_line_count} lines).\n{snippet}"
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
        let inserted_lines = insert_text.lines().count().max(1);
        let ins_start = after_line + 1;
        let ins_end = after_line + inserted_lines;

        // Record agent edit and adjust existing ranges (convert 1-indexed to 0-indexed)
        let buf_idx = self.current_buffer_index;
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.agent_edits
                .adjust_for_insert(buf_idx, after_line, inserted_lines);
            chat.agent_edits
                .record_edit(buf_idx, ins_start - 1, ins_end - 1);
        }

        let snippet = snippet_around(self.buffer().rope(), ins_start, ins_end, 3);
        ToolResult::Success(format!(
            "Inserted text after line {after_line} (buffer now has {new_line_count} lines).\n{snippet}"
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

        // Adjust existing ranges for deletion (convert 1-indexed to 0-indexed)
        let buf_idx = self.current_buffer_index;
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.agent_edits
                .adjust_for_delete(buf_idx, start_line - 1, end_line - 1);
        }

        // Show context around the deletion point
        let snippet = snippet_around(self.buffer().rope(), start_line, start_line, 3);
        ToolResult::Success(format!(
            "Deleted {deleted_count} line(s) ({start_line}-{end_line}). Buffer now has {new_line_count} lines.\n{snippet}"
        ))
    }
}
