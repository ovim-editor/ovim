use super::ai_state::{AiEditRegion, AiRegionStatus, AiSelectionSnapshot, PendingAiJob};
use super::Editor;
use crate::ai::request_ai_edit;
use crate::editor::lsp_state::HoverContentType;
use crate::mode::Mode;
use anyhow::{anyhow, Result};
use std::time::Instant;
use tokio::sync::oneshot;
use tokio::sync::oneshot::error::TryRecvError;

impl Editor {
    /// Returns configured AI profile names sorted for deterministic picker navigation.
    pub fn ai_profile_names_sorted(&self) -> Vec<String> {
        let mut names: Vec<String> = self.ai_state.config.profiles.keys().cloned().collect();
        names.sort();
        names
    }

    /// Select a specific AI profile. Returns false when profile is unknown.
    pub fn ai_set_profile(&mut self, profile_name: &str) -> bool {
        let Some(profile) = self.ai_state.config.resolve_profile(profile_name) else {
            return false;
        };
        self.ai_state.active_profile = profile_name.to_string();
        self.ai_state.extraction = profile.extraction;
        self.set_lsp_status(format!(
            "AI profile: {} ({}/{})",
            profile_name, profile.provider, profile.model
        ));
        true
    }

    /// Cycle AI profile selection in prompt mode.
    pub fn ai_cycle_profile(&mut self, forward: bool) {
        let names = self.ai_profile_names_sorted();
        if names.is_empty() {
            return;
        }

        let current_idx = names
            .iter()
            .position(|name| name == &self.ai_state.active_profile)
            .unwrap_or(0);

        let next_idx = if forward {
            (current_idx + 1) % names.len()
        } else if current_idx == 0 {
            names.len() - 1
        } else {
            current_idx - 1
        };

        let _ = self.ai_set_profile(&names[next_idx]);
    }

    /// Start AI prompt from the current visual selection.
    pub fn start_ai_prompt_from_visual(&mut self) -> Result<()> {
        if self.mode() == Mode::VisualBlock {
            self.set_lsp_status("AI edit does not support visual block mode".to_string());
            return Ok(());
        }

        let Some(((start_line, start_col), (end_line, end_col))) = self.visual_selection() else {
            self.set_lsp_status("No visual selection to edit".to_string());
            return Ok(());
        };

        let rope = self.buffer().rope();
        let rope_len = rope.len_chars();

        let (start_char, end_char) = match self.mode() {
            Mode::VisualLine => {
                let start = rope.line_to_char(start_line).min(rope_len);
                let end = if end_line + 1 < self.buffer().raw_line_count() {
                    rope.line_to_char(end_line + 1)
                } else {
                    rope_len
                };
                (start, end.min(rope_len))
            }
            _ => {
                let start = (rope.line_to_char(start_line) + start_col).min(rope_len);
                let end = (rope.line_to_char(end_line) + end_col + 1).min(rope_len);
                (start, end)
            }
        };

        if end_char <= start_char {
            self.set_lsp_status("Visual selection is empty".to_string());
            return Ok(());
        }

        let selected_text = rope.slice(start_char..end_char).to_string();
        let snapshot = AiSelectionSnapshot {
            start_line,
            start_col,
            end_line,
            end_col,
            start_char,
            end_char,
            anchor_line: start_line,
            selected_text,
            mode_before_prompt: self.mode(),
        };

        self.ai_state.active_selection = Some(snapshot);
        self.ai_state.prompt.input.clear();
        self.ai_state.prompt.cursor = 0;
        self.set_mode(Mode::AiPrompt);
        self.set_lsp_status("AI prompt: type instruction and press Enter".to_string());
        Ok(())
    }

    /// Submit current AI prompt as an async job.
    pub fn submit_ai_prompt_job(&mut self) -> Result<()> {
        let prompt = self.ai_state.prompt.input.trim().to_string();
        if prompt.is_empty() {
            self.set_lsp_status("AI prompt is empty".to_string());
            return Ok(());
        }

        let Some(selection) = self.ai_state.active_selection.clone() else {
            self.set_lsp_status("No selection queued for AI edit".to_string());
            self.set_mode(Mode::Normal);
            return Ok(());
        };

        let profile_name = self.ai_state.active_profile.clone();
        let Some(profile) = self.ai_state.config.resolve_profile(&profile_name).cloned() else {
            self.set_lsp_status(format!("Unknown AI profile: {}", profile_name));
            return Ok(());
        };

        let extraction = self.ai_state.extraction;
        let (request, mut prep_trace) = self.build_ai_request_for_selection(
            &profile,
            prompt.clone(),
            &selection,
            extraction,
        );
        prep_trace.push("waiting for model response...".to_string());

        let lock_id = self.ai_state.next_lock_id;
        self.ai_state.next_lock_id = self.ai_state.next_lock_id.saturating_add(1);
        self.buffer_mut()
            .add_ai_lock(lock_id, selection.start_char, selection.end_char);

        let job_id = self.ai_state.next_job_id;
        self.ai_state.next_job_id = self.ai_state.next_job_id.saturating_add(1);

        let provider_label = format!("{}/{}", profile.provider, profile.model);
        let now = Instant::now();
        self.ai_state.regions.retain(|region| region.id != lock_id);
        self.ai_state.regions.push(AiEditRegion {
            id: lock_id,
            start_char: selection.start_char,
            end_char: selection.end_char,
            status: AiRegionStatus::Running,
            prompt: prompt.clone(),
            original_text: selection.selected_text.clone(),
            generated_text: String::new(),
            profile_name: profile_name.clone(),
            provider_label: provider_label.clone(),
            extraction,
            reasoning_lines: prep_trace,
            raw_output: None,
            created_at: now,
            updated_at: now,
        });
        self.ai_state.selected_region_id = Some(lock_id);
        self.ai_state.selection_hold_until_exit = false;

        let (tx, rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let result = request_ai_edit(&profile, &request).await;
            let clone_for_channel = match &result {
                Ok(ok) => Ok(ok.clone()),
                Err(err) => Err(anyhow!(err.to_string())),
            };
            let _ = tx.send(clone_for_channel);
            result
        });

        self.ai_state.pending_jobs.push(PendingAiJob {
            job_id,
            lock_id,
            selection,
            submitted_at: Instant::now(),
            task,
            receiver: rx,
        });

        self.ai_state.prompt.input.clear();
        self.ai_state.prompt.cursor = 0;
        self.ai_state.active_selection = None;
        self.set_mode(Mode::Normal);
        self.set_lsp_status(format!("AI job {} started ({})", job_id, provider_label));
        Ok(())
    }

    /// Polls AI jobs; returns true when editor state changed.
    pub fn poll_pending_ai_jobs(&mut self) -> bool {
        let mut changed = false;
        let mut idx = 0;

        while idx < self.ai_state.pending_jobs.len() {
            let mut finished = false;
            let result = {
                let pending = &mut self.ai_state.pending_jobs[idx];
                match pending.receiver.try_recv() {
                    Ok(result) => {
                        finished = true;
                        Some(result)
                    }
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Closed) => {
                        finished = true;
                        Some(Err(anyhow!("AI job channel closed")))
                    }
                }
            };

            if !finished {
                idx += 1;
                continue;
            }

            let job = self.ai_state.pending_jobs.remove(idx);
            let _ = job.task.is_finished();
            changed = true;

            match result {
                Some(Ok(job_result)) => {
                    if let Err(err) = self.apply_ai_job_result(job.lock_id, &job_result) {
                        self.buffer_mut().remove_ai_lock(job.lock_id);
                        self.update_ai_region_failure(
                            job.lock_id,
                            format!("apply failed: {}", err),
                            AiRegionStatus::Failed,
                        );
                        self.set_lsp_status(format!("AI apply failed: {}", err));
                    } else {
                        self.set_lsp_status(format!(
                            "AI edit applied ({}/{})",
                            job_result.provider, job_result.model
                        ));
                    }
                }
                Some(Err(err)) => {
                    self.buffer_mut().remove_ai_lock(job.lock_id);
                    self.update_ai_region_failure(
                        job.lock_id,
                        format!("request failed: {}", err),
                        AiRegionStatus::Failed,
                    );
                    self.set_lsp_status(format!("AI job failed: {}", err));
                }
                None => {}
            }
        }

        changed
    }

    pub fn ai_regions(&self) -> &[AiEditRegion] {
        &self.ai_state.regions
    }

    pub fn ai_selected_region_id(&self) -> Option<u64> {
        self.ai_state.selected_region_id
    }

    pub fn ai_region_by_id(&self, id: u64) -> Option<&AiEditRegion> {
        self.ai_state.regions.iter().find(|region| region.id == id)
    }

    pub fn ai_show_reasoning_for_selected_region(&mut self) -> bool {
        let Some(region_id) = self.ai_state.selected_region_id else {
            return false;
        };
        let Some(region) = self
            .ai_state
            .regions
            .iter()
            .find(|region| region.id == region_id)
        else {
            return false;
        };

        let reasoning_text = if region.reasoning_lines.is_empty() {
            "No reasoning available".to_string()
        } else {
            region.reasoning_lines.join("\n")
        };

        let mut message = String::new();
        message.push_str("**AI Edit Details**\n");
        message.push_str(&format!(
            "- profile: {}\n- provider/model: {}\n- extraction: {}\n- status: {:?}\n\n",
            region.profile_name, region.provider_label, region.extraction, region.status
        ));
        message.push_str("**Prompt**\n");
        message.push_str(&region.prompt);
        message.push_str("\n\n**Model Notes**\n");
        message.push_str(&reasoning_text);
        if let Some(raw) = &region.raw_output {
            message.push_str("\n\n**Raw Output**\n");
            message.push_str(raw);
        }

        let (line, col) = self.abs_char_to_line_col(region.start_char);
        self.lsp_state.hover_info = Some(message);
        self.lsp_state.hover_scroll = 0;
        self.lsp_state.hover_position = Some((line, col));
        self.lsp_state.hover_content_type = HoverContentType::AiReasoning;
        self.set_mode(Mode::HoverPreview);
        self.mark_dirty();
        true
    }

    pub fn ai_accept_selected_region(&mut self) -> bool {
        let Some(region_id) = self.ai_state.selected_region_id else {
            return false;
        };

        self.ai_remove_region(region_id);
        self.ai_state.selected_region_id = None;
        self.ai_state.selection_hold_until_exit = true;
        if self.hover_content_type() == HoverContentType::AiReasoning {
            self.clear_hover();
        }
        self.set_lsp_status("AI region accepted".to_string());
        true
    }

    pub fn ai_revert_selected_region(&mut self) -> Result<bool> {
        let Some(region_id) = self.ai_state.selected_region_id else {
            return Ok(false);
        };
        let Some(region) = self
            .ai_state
            .regions
            .iter()
            .find(|region| region.id == region_id)
            .cloned()
        else {
            return Ok(false);
        };

        if region.status == AiRegionStatus::Running {
            return Ok(self.ai_cancel_selected_region());
        }

        let rope_len = self.buffer().rope().len_chars();
        let start_char = region.start_char.min(rope_len);
        let end_char = region.end_char.min(rope_len).max(start_char);
        let replacement = region.original_text.clone();

        let cursor_before = self.cursor_position();
        let ((), edits) = self.buffer_mut().with_ai_lock_bypass(|buf| {
            buf.record(|buf| {
                if end_char > start_char {
                    buf.delete_char_range(start_char, end_char);
                }
                let insert_pos = start_char.min(buf.rope().len_chars());
                let line = buf.rope().char_to_line(insert_pos);
                let col = insert_pos - buf.rope().line_to_char(line);
                if !replacement.is_empty() {
                    buf.insert_text_at(line, col, &replacement);
                }
            })
        });

        if !edits.is_empty() {
            let cursor_after = self.cursor_position();
            self.push_recorded_undo(edits, cursor_before, cursor_after);
        }

        self.ai_remove_region(region_id);
        self.ai_state.selected_region_id = None;
        self.ai_state.selection_hold_until_exit = false;
        if self.hover_content_type() == HoverContentType::AiReasoning {
            self.clear_hover();
        }

        if self.buffer().needs_rehighlight() {
            self.process_viewport_rehighlight();
        }
        self.request_diagnostics_refresh();
        self.ai_state.last_observed_buffer_version = self.buffer().version();
        self.set_lsp_status("AI region reverted".to_string());
        self.mark_dirty();
        Ok(true)
    }

    pub fn ai_retry_selected_region(&mut self) -> Result<bool> {
        let Some(region_id) = self.ai_state.selected_region_id else {
            return Ok(false);
        };
        let Some(region_idx) = self
            .ai_state
            .regions
            .iter()
            .position(|region| region.id == region_id)
        else {
            return Ok(false);
        };

        if self.ai_state.regions[region_idx].status == AiRegionStatus::Running {
            self.set_lsp_status("AI region is already generating".to_string());
            return Ok(true);
        }

        let start_char = self.ai_state.regions[region_idx].start_char;
        let end_char = self.ai_state.regions[region_idx].end_char;
        let prompt = self.ai_state.regions[region_idx].prompt.clone();
        let profile_name = self.ai_state.regions[region_idx].profile_name.clone();
        let extraction = self.ai_state.regions[region_idx].extraction;

        let selected_text = self
            .slice_text_by_chars(start_char, end_char)
            .unwrap_or_default();
        let (start_line, start_col) = self.abs_char_to_line_col(start_char);
        let end_for_col = end_char.saturating_sub(1).max(start_char);
        let (end_line, end_col) = self.abs_char_to_line_col(end_for_col);
        let selection = AiSelectionSnapshot {
            start_line,
            start_col,
            end_line,
            end_col,
            start_char,
            end_char,
            anchor_line: start_line,
            selected_text: selected_text.clone(),
            mode_before_prompt: Mode::Normal,
        };

        let Some(profile) = self.ai_state.config.resolve_profile(&profile_name).cloned() else {
            self.set_lsp_status(format!("Unknown AI profile: {}", profile_name));
            return Ok(true);
        };

        let (request, mut prep_trace) = self.build_ai_request_for_selection(
            &profile,
            prompt.clone(),
            &selection,
            extraction,
        );
        prep_trace.push("retrying with same prompt...".to_string());

        // Replace tracking lock with blocking lock while generation is in flight.
        self.buffer_mut().remove_ai_lock(region_id);
        self.buffer_mut()
            .add_ai_lock(region_id, start_char, end_char);

        let provider_label = format!("{}/{}", profile.provider, profile.model);
        {
            let region = &mut self.ai_state.regions[region_idx];
            region.status = AiRegionStatus::Running;
            region.original_text = selected_text.clone();
            region.generated_text.clear();
            region.provider_label = provider_label.clone();
            region.reasoning_lines = prep_trace;
            region.raw_output = None;
            region.updated_at = Instant::now();
        }

        let job_id = self.ai_state.next_job_id;
        self.ai_state.next_job_id = self.ai_state.next_job_id.saturating_add(1);

        let (tx, rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let result = request_ai_edit(&profile, &request).await;
            let clone_for_channel = match &result {
                Ok(ok) => Ok(ok.clone()),
                Err(err) => Err(anyhow!(err.to_string())),
            };
            let _ = tx.send(clone_for_channel);
            result
        });

        self.ai_state.pending_jobs.push(PendingAiJob {
            job_id,
            lock_id: region_id,
            selection,
            submitted_at: Instant::now(),
            task,
            receiver: rx,
        });

        self.ai_state.selection_hold_until_exit = false;
        self.set_lsp_status(format!("AI retry started ({})", provider_label));
        Ok(true)
    }

    pub fn ai_cancel_selected_region(&mut self) -> bool {
        let Some(region_id) = self.ai_state.selected_region_id else {
            return false;
        };

        if let Some(idx) = self
            .ai_state
            .pending_jobs
            .iter()
            .position(|pending| pending.lock_id == region_id)
        {
            let pending = self.ai_state.pending_jobs.remove(idx);
            pending.task.abort();
        }

        let running = self
            .ai_state
            .regions
            .iter()
            .find(|region| region.id == region_id)
            .map(|region| region.status == AiRegionStatus::Running)
            .unwrap_or(false);

        if running {
            self.buffer_mut().remove_ai_lock(region_id);
            self.ai_remove_region(region_id);
            self.ai_state.selected_region_id = None;
            self.ai_state.selection_hold_until_exit = false;
            if self.hover_content_type() == HoverContentType::AiReasoning {
                self.clear_hover();
            }
            self.set_lsp_status("AI generation cancelled".to_string());
            return true;
        }

        self.ai_state.selected_region_id = None;
        self.ai_state.selection_hold_until_exit = true;
        if self.hover_content_type() == HoverContentType::AiReasoning {
            self.clear_hover();
        }
        self.set_lsp_status("AI region selection cleared".to_string());
        true
    }

    pub fn ai_post_input_refresh(&mut self) {
        let current_version = self.buffer().version();
        if self.ai_state.last_observed_buffer_version != current_version {
            self.ai_sync_regions_with_locks();
            self.ai_drop_modified_generated_regions();
            self.ai_state.last_observed_buffer_version = current_version;
        }
        self.ai_refresh_region_selection_from_cursor();
    }

    fn apply_ai_job_result(&mut self, lock_id: u64, result: &crate::ai::AiJobResult) -> Result<()> {
        let lock = self
            .buffer()
            .ai_locks()
            .iter()
            .find(|lock| lock.id == lock_id)
            .copied()
            .ok_or_else(|| anyhow!("missing AI lock {}", lock_id))?;

        self.buffer_mut().remove_ai_lock(lock_id);

        let mut top_text = String::new();
        if !result.top_insertions.is_empty() {
            top_text = result.top_insertions.join("\n");
            if !top_text.ends_with('\n') {
                top_text.push('\n');
            }
        }

        let cursor_before = self.cursor_position();
        let replacement = result.replacement.clone();
        let insert_pos = lock.start_char.min(self.buffer().rope().len_chars());
        let ((), edits) = self.buffer_mut().with_ai_lock_bypass(|buf| {
            buf.record(|buf| {
                if lock.end_char > lock.start_char {
                    buf.delete_char_range(lock.start_char, lock.end_char);
                }

                let current_insert = insert_pos.min(buf.rope().len_chars());
                let line = buf.rope().char_to_line(current_insert);
                let col = current_insert - buf.rope().line_to_char(line);
                if !replacement.is_empty() {
                    buf.insert_text_at(line, col, &replacement);
                }

                if !top_text.is_empty() {
                    buf.insert_text_at(0, 0, &top_text);
                }
            })
        });

        if edits.is_empty() {
            return Err(anyhow!("AI produced no edits"));
        }

        let cursor_after = self.cursor_position();
        self.push_recorded_undo(edits, cursor_before, cursor_after);

        let top_chars = top_text.chars().count();
        let new_start = insert_pos.saturating_add(top_chars);
        let new_end = new_start.saturating_add(replacement.chars().count());
        self.buffer_mut()
            .add_ai_lock_with_mode(lock_id, new_start, new_end, false);

        if let Some(region) = self
            .ai_state
            .regions
            .iter_mut()
            .find(|region| region.id == lock_id)
        {
            region.start_char = new_start;
            region.end_char = new_end;
            region.status = AiRegionStatus::Generated;
            region.generated_text = replacement;
            region.provider_label = format!("{}/{}", result.provider, result.model);
            region.profile_name = result.profile_name.clone();
            region.reasoning_lines = if result.log_lines.is_empty() {
                vec!["generation completed".to_string()]
            } else {
                result.log_lines.clone()
            };
            region.raw_output = Some(result.raw_output.clone());
            region.updated_at = Instant::now();
        }

        self.ai_state.selected_region_id = Some(lock_id);
        self.ai_state.selection_hold_until_exit = false;
        self.ai_state.last_observed_buffer_version = self.buffer().version();

        if self.buffer().needs_rehighlight() {
            self.process_viewport_rehighlight();
        }
        self.request_diagnostics_refresh();
        self.mark_dirty();
        Ok(())
    }

    fn update_ai_region_failure(&mut self, lock_id: u64, message: String, status: AiRegionStatus) {
        if let Some(region) = self
            .ai_state
            .regions
            .iter_mut()
            .find(|region| region.id == lock_id)
        {
            region.status = status;
            region.reasoning_lines = vec![message];
            region.updated_at = Instant::now();
        }
    }

    fn ai_refresh_region_selection_from_cursor(&mut self) {
        let cursor_abs = self.cursor_abs_char();
        let hovered_region = self
            .ai_state
            .regions
            .iter()
            .find(|region| {
                matches!(
                    region.status,
                    AiRegionStatus::Running | AiRegionStatus::Generated
                ) && region_contains_char(region, cursor_abs)
            })
            .map(|region| region.id);

        if self.ai_state.selection_hold_until_exit {
            if hovered_region.is_none() {
                self.ai_state.selection_hold_until_exit = false;
            }
            return;
        }

        self.ai_state.selected_region_id = hovered_region;
    }

    fn ai_drop_modified_generated_regions(&mut self) {
        let mut removed_ids = Vec::new();
        for region in &self.ai_state.regions {
            if region.status != AiRegionStatus::Generated {
                continue;
            }
            let matches_current = self
                .slice_text_by_chars(region.start_char, region.end_char)
                .map(|text| text == region.generated_text)
                .unwrap_or(false);
            if !matches_current {
                removed_ids.push(region.id);
            }
        }

        self.ai_state
            .regions
            .retain(|region| !removed_ids.contains(&region.id));

        for id in removed_ids {
            self.buffer_mut().remove_ai_lock(id);
            if self.ai_state.selected_region_id == Some(id) {
                self.ai_state.selected_region_id = None;
            }
        }

        if self.ai_state.selected_region_id.is_none()
            && self.hover_content_type() == HoverContentType::AiReasoning
        {
            self.clear_hover();
        }
    }

    fn ai_sync_regions_with_locks(&mut self) {
        let locks: Vec<(u64, usize, usize)> = self
            .buffer()
            .ai_locks()
            .iter()
            .map(|lock| (lock.id, lock.start_char, lock.end_char))
            .collect();

        for region in &mut self.ai_state.regions {
            if let Some((_, start, end)) = locks.iter().find(|(id, _, _)| *id == region.id) {
                region.start_char = *start;
                region.end_char = *end;
            }
        }
    }

    fn ai_remove_region(&mut self, region_id: u64) {
        self.ai_state
            .regions
            .retain(|region| region.id != region_id);
        self.buffer_mut().remove_ai_lock(region_id);
    }

    fn cursor_abs_char(&self) -> usize {
        let cursor = self.buffer().cursor();
        let rope = self.buffer().rope();
        if rope.len_lines() == 0 {
            return 0;
        }

        let line = cursor.line().min(rope.len_lines().saturating_sub(1));
        let line_start = rope.line_to_char(line);
        let line_end = if line + 1 < rope.len_lines() {
            rope.line_to_char(line + 1)
        } else {
            rope.len_chars()
        };
        let line_content_end = if line_end > line_start && rope.char(line_end - 1) == '\n' {
            line_end.saturating_sub(1)
        } else {
            line_end
        };
        let max_col = line_content_end.saturating_sub(line_start);
        line_start + cursor.col().min(max_col)
    }

    fn abs_char_to_line_col(&self, abs_char: usize) -> (usize, usize) {
        let rope = self.buffer().rope();
        let clamped = abs_char.min(rope.len_chars());
        let line = rope.char_to_line(clamped);
        let col = clamped.saturating_sub(rope.line_to_char(line));
        (line, col)
    }

    fn slice_text_by_chars(&self, start_char: usize, end_char: usize) -> Option<String> {
        let rope = self.buffer().rope();
        let len = rope.len_chars();
        let start = start_char.min(len);
        let end = end_char.min(len);
        if end < start {
            return None;
        }
        Some(rope.slice(start..end).to_string())
    }
}

fn region_contains_char(region: &AiEditRegion, abs_char: usize) -> bool {
    if region.end_char > region.start_char {
        abs_char >= region.start_char && abs_char < region.end_char
    } else {
        abs_char == region.start_char
    }
}
