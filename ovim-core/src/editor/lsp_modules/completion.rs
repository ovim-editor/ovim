//! LSP completion functionality
//!
//! This module handles code completion requests and application.
//! Completions are typically triggered by Ctrl+N or automatically in insert mode.

use super::super::Editor;
use crate::unicode::{byte_offset_for_grapheme, grapheme_at_index, grapheme_count, grapheme_indices};
use crate::lsp::uri_from_file_path;
use anyhow::{anyhow, Result};

impl Editor {
    /// Request completion at current cursor position
    pub fn request_completion(&mut self) {
        self.queue_lsp_action(crate::editor::lsp_state::LspAction::Completion);
    }

    pub(crate) fn completion_trigger_context(&self) -> (usize, String) {
        let cursor = self.buffer().cursor();
        let line_idx = cursor.line();
        let cursor_col = cursor.col();

        let line_text = self
            .buffer()
            .line(line_idx)
            .unwrap_or_default()
            .trim_end_matches('\n')
            .to_string();

        completion_trigger_context_from_line(&line_text, cursor_col)
    }

    /// Apply a completion by index from available completions
    pub fn apply_completion(&mut self, completion_index: usize) {
        if completion_index >= self.lsp_state.available_completions.len() {
            self.set_lsp_status("Invalid completion index".to_string());
            return;
        }

        let completion = self.lsp_state.available_completions[completion_index].clone();

        // Extract the text to insert
        let insert_text = if let Some(text_edit) = completion.text_edit {
            match text_edit {
                lsp_types::CompletionTextEdit::Edit(edit) => edit.new_text,
                lsp_types::CompletionTextEdit::InsertAndReplace(insert_replace) => {
                    insert_replace.new_text
                }
            }
        } else if let Some(insert_text) = completion.insert_text {
            insert_text
        } else {
            completion.label
        };

        // Insert at cursor position
        let (line, col) = {
            let cursor = self.buffer().cursor();
            (cursor.line(), cursor.col())
        };
        self.buffer_mut().insert_text_at(line, col, &insert_text);

        // Clear completions after applying
        self.lsp_state.available_completions.clear();
        self.set_lsp_status("Completion applied".to_string());
    }

    /// Implementation of completion request
    pub(in crate::editor) async fn completion_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path().map(|s| s.to_string()) else {
            self.set_lsp_status("Save file first to use completion".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(&file_path).is_absolute() {
            file_path.clone()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(&file_path).to_string_lossy().to_string(),
                Err(_) => {
                    self.set_lsp_status("Failed to resolve file path".to_string());
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());
        let trigger_char = {
            let line_text = self
                .buffer()
                .line(cursor.line())
                .unwrap_or_default()
                .trim_end_matches('\n')
                .to_string();
            if cursor.col() > 0 {
                match grapheme_at_index(&line_text, cursor.col().saturating_sub(1)) {
                    Some(".") => Some('.'),
                    _ => None,
                }
            } else {
                None
            }
        };

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Cancel any pending completion request we already spawned.
        if let Some(pending) = self.lsp_state.pending_completion.take() {
            pending.request.task.abort();
        }

        // Snapshot sync state so we can flush document changes in the background without blocking UI.
        let state_key = file_path.clone();
        let content = self.buffer().rope().to_string();
        let (needs_did_open, needs_flush, old_content) = match self.lsp_state.document_sync.get(&state_key) {
            None => (true, true, None),
            Some(state) => {
                let did_open_missing = !state.did_open_sent;
                let old = state.last_synced_content.clone();
                // In insert mode we may not mark `buffer_modified` until leaving insert mode.
                // For completions, always treat the document as needing a flush if the content
                // differs from what we last synced.
                let content_changed = old.as_deref().is_none_or(|old| old != content);
                let needs_flush = state.is_modified() || content_changed;
                (did_open_missing, needs_flush, old)
            }
        };

        // Resolve all server_ids for this language (primary + companions)
        let server_ids = lsp.servers_for_language(language_id);

        self.lsp_state.completion_request_seq = self.lsp_state.completion_request_seq.wrapping_add(1);
        let seq = self.lsp_state.completion_request_seq;

        // Spawn completion request in background (non-blocking)
        let (tx, rx) = tokio::sync::oneshot::channel();
        let language_id = language_id.to_string();
        let task = tokio::spawn(async move {
            let mut synced_content: Option<String> = None;
            if needs_did_open {
                let ok = lsp
                    .did_open_broadcast(uri.clone(), &language_id, 1, content.clone())
                    .await;
                if ok.is_ok() {
                    synced_content = Some(content.clone());
                }
            } else if needs_flush {
                let ok = lsp
                    .did_change_broadcast(uri.clone(), &language_id, content.clone(), old_content)
                    .await;
                if ok.is_ok() {
                    synced_content = Some(content.clone());
                }
            }

            let result = if server_ids.len() > 1 {
                lsp.completion_multi(&uri, line, character, &server_ids, trigger_char)
                    .await
            } else {
                lsp.completion(&uri, line, character, &language_id, trigger_char)
                    .await
            };
            let task_result = result.map(|items| crate::editor::lsp_state::CompletionTaskResult {
                items,
                file_path: state_key,
                synced_content,
            });

            let _ = tx.send(task_result);

            Ok(crate::editor::lsp_state::CompletionTaskResult {
                items: Vec::new(),
                file_path: String::new(),
                synced_content: None,
            })
        });

        self.lsp_state.pending_completion = Some(crate::editor::lsp_state::PendingCompletionRequest {
            seq,
            request: crate::editor::lsp_state::PendingLspRequest {
                task,
                receiver: rx,
                started: std::time::Instant::now(),
            },
        });

        self.set_lsp_status("Requesting completions...".to_string());
        Ok(true)
    }
}

fn completion_trigger_context_from_line(line_text: &str, cursor_col: usize) -> (usize, String) {
    let cursor_byte = byte_offset_for_grapheme(line_text, cursor_col).unwrap_or(line_text.len());
    let before_cursor = &line_text[..cursor_byte.min(line_text.len())];

    let mut start_byte = before_cursor.len();
    let graphemes: Vec<(usize, &str)> = grapheme_indices(before_cursor).collect();
    for (byte_offset, grapheme) in graphemes.into_iter().rev() {
        let is_ident = grapheme
            .chars()
            .all(|c| c == '_' || c.is_alphanumeric());
        if !is_ident {
            break;
        }
        start_byte = byte_offset;
    }

    let trigger_col = grapheme_count(&line_text[..start_byte.min(line_text.len())]);
    let trigger_prefix = line_text[start_byte.min(cursor_byte)..cursor_byte]
        .to_string();

    (trigger_col, trigger_prefix)
}

#[cfg(test)]
mod tests {
    use super::completion_trigger_context_from_line;
    use crate::unicode::grapheme_at_index;

    #[test]
    fn completion_trigger_context_basic_word() {
        let (col, prefix) = completion_trigger_context_from_line("foobar", 6);
        assert_eq!(col, 0);
        assert_eq!(prefix, "foobar");
    }

    #[test]
    fn completion_trigger_context_after_dot() {
        let (col, prefix) = completion_trigger_context_from_line("foo.", 4);
        assert_eq!(col, 4);
        assert_eq!(prefix, "");
    }

    #[test]
    fn completion_trigger_context_member_prefix() {
        let (col, prefix) = completion_trigger_context_from_line("foo.bar", 7);
        assert_eq!(col, 4);
        assert_eq!(prefix, "bar");
    }

    #[test]
    fn completion_trigger_context_underscore_digits() {
        let (col, prefix) = completion_trigger_context_from_line("__x1", 4);
        assert_eq!(col, 0);
        assert_eq!(prefix, "__x1");
    }

    #[test]
    fn trigger_char_detection_dot() {
        let line = "s.";
        assert_eq!(grapheme_at_index(line, 1), Some("."));
    }
}
