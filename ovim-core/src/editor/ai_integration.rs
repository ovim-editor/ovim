use super::ai_state::{AiJobStatus, AiLogBlock, AiSelectionSnapshot, PendingAiJob};
use super::Editor;
use crate::ai::{request_ai_edit, AiRequest};
use crate::mode::Mode;
use anyhow::{anyhow, Result};
use std::time::Instant;
use tokio::sync::oneshot;
use tokio::sync::oneshot::error::TryRecvError;

impl Editor {
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
        let file_path = self.buffer().file_path().map(ToString::to_string);
        let language_id = file_path
            .as_deref()
            .and_then(crate::syntax::LanguageRegistry::get_lsp_language_id)
            .map(ToString::to_string);

        let request = AiRequest {
            prompt: prompt.clone(),
            selected_text: selection.selected_text.clone(),
            language_id,
            file_path,
            extraction,
        };

        let lock_id = self.ai_state.next_lock_id;
        self.ai_state.next_lock_id = self.ai_state.next_lock_id.saturating_add(1);
        self.buffer_mut()
            .add_ai_lock(lock_id, selection.start_char, selection.end_char);

        let job_id = self.ai_state.next_job_id;
        self.ai_state.next_job_id = self.ai_state.next_job_id.saturating_add(1);

        let provider_label = format!("{}/{}", profile.provider, profile.model);
        self.ai_state.logs.retain(|log| log.lock_id != lock_id);
        self.ai_state.logs.push(AiLogBlock {
            lock_id,
            anchor_line: selection.anchor_line,
            status: AiJobStatus::Running,
            provider_label: provider_label.clone(),
            lines: vec![
                format!("prompt: {}", trim_for_log(&prompt, 120)),
                "waiting for model response...".to_string(),
            ],
            updated_at: Instant::now(),
        });

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
                        self.update_ai_log(
                            job.lock_id,
                            AiJobStatus::Failed,
                            vec![format!("apply failed: {}", err)],
                        );
                        self.set_lsp_status(format!("AI apply failed: {}", err));
                    } else {
                        let mut log_lines = vec![
                            format!(
                                "applied in {} ms",
                                job.submitted_at.elapsed().as_millis()
                            ),
                            format!("replacement chars: {}", job_result.replacement.chars().count()),
                        ];
                        if !job_result.log_lines.is_empty() {
                            log_lines.extend(job_result.log_lines.clone());
                        }
                        self.update_ai_log(job.lock_id, AiJobStatus::Succeeded, log_lines);
                        self.set_lsp_status(format!(
                            "AI edit applied ({}/{})",
                            job_result.provider, job_result.model
                        ));
                    }
                }
                Some(Err(err)) => {
                    self.buffer_mut().remove_ai_lock(job.lock_id);
                    self.update_ai_log(
                        job.lock_id,
                        AiJobStatus::Failed,
                        vec![format!("request failed: {}", err)],
                    );
                    self.set_lsp_status(format!("AI job failed: {}", err));
                }
                None => {}
            }
        }

        changed
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
        let ((), edits) = self
            .buffer_mut()
            .with_ai_lock_bypass(|buf| {
                buf.record(|buf| {
                    if lock.end_char > lock.start_char {
                        buf.delete_char_range(lock.start_char, lock.end_char);
                    }

                    let insert_pos = lock.start_char.min(buf.rope().len_chars());
                    let line = buf.rope().char_to_line(insert_pos);
                    let col = insert_pos - buf.rope().line_to_char(line);
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
        self.mark_dirty();
        Ok(())
    }

    fn update_ai_log(&mut self, lock_id: u64, status: AiJobStatus, lines: Vec<String>) {
        if let Some(log) = self.ai_state.logs.iter_mut().find(|log| log.lock_id == lock_id) {
            log.status = status;
            log.lines = lines;
            log.updated_at = Instant::now();
        }
    }
}

fn trim_for_log(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let mut out = text.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    out.push('…');
    out
}
