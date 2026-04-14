//! LSP completion functionality
//!
//! This module handles code completion requests and application.
//! Completions are typically triggered by Ctrl+N or automatically in insert mode.

use super::super::Editor;
use crate::lsp::uri_from_file_path;
use crate::unicode::{
    byte_offset_for_grapheme, grapheme_at_index, grapheme_count, grapheme_indices,
};
use anyhow::{anyhow, Result};
use std::collections::HashSet;

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

        completion_trigger_context_from_line(&line_text, cursor_col.0)
    }

    /// Apply a completion by index from available completions
    pub fn apply_completion(&mut self, completion_index: usize) {
        if completion_index >= self.lsp.state.available_completions.len() {
            self.set_lsp_status("Invalid completion index".to_string());
            return;
        }

        let completion = self.lsp.state.available_completions[completion_index].clone();

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
        self.buffer_mut().insert_text_at(line, col.0, &insert_text);

        // Clear completions after applying
        self.lsp.state.available_completions.clear();
        self.set_lsp_status("Completion applied".to_string());
    }

    /// Implementation of completion request
    pub(in crate::editor) async fn completion_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp.state.lsp_manager {
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
        let col = cursor.col().0;
        let character = self.col_to_utf16(cursor.line(), col);
        let raw_trigger_char = {
            let line_text = self
                .buffer()
                .line(cursor.line())
                .unwrap_or_default()
                .trim_end_matches('\n')
                .to_string();
            if col > 0 {
                if col >= 2 {
                    let g1 = grapheme_at_index(&line_text, col.saturating_sub(1));
                    let g2 = grapheme_at_index(&line_text, col.saturating_sub(2));
                    if g2 == Some(":") && g1 == Some(":") {
                        Some(':')
                    } else if g2 == Some("-") && g1 == Some(">") {
                        Some('>')
                    } else {
                        match g1 {
                            Some(".") => Some('.'),
                            _ => None,
                        }
                    }
                } else {
                    match grapheme_at_index(&line_text, col.saturating_sub(1)) {
                        Some(".") => Some('.'),
                        _ => None,
                    }
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

        // Sync document content to LSP on the main thread before spawning.
        // This avoids the spawned task racing with send_lsp_changes_if_modified()
        // over the debouncer — a source of timing bugs where stale content
        // was sent from the background task, overwriting newer content.
        self.ensure_lsp_document_synced().await;

        // Resolve the server group responsible for this document.
        let server_ids = lsp.servers_for_document(language_id, std::path::Path::new(&file_path));

        // No LSP servers for this language — nothing to complete.
        if server_ids.is_empty() {
            return Ok(false);
        }

        let buffer_version = self.buffer().version() as u64;

        // Spawn completion request in background (non-blocking).
        // Document sync already happened above via ensure_lsp_document_synced().
        // The task only makes the LSP request — no debouncer interaction.
        let (tx, rx) = tokio::sync::oneshot::channel();
        let language_id = language_id.to_string();
        let file_path_for_task = file_path.clone();
        let task = tokio::spawn(async move {
            let mut supported_triggers: HashSet<char> = lsp
                .completion_trigger_characters_for_servers(&server_ids)
                .await
                .into_iter()
                .collect();
            for ch in crate::lsp::fallback_completion_trigger_characters(&language_id) {
                supported_triggers.insert(*ch);
            }
            let trigger_char = filter_supported_trigger(raw_trigger_char, &supported_triggers);

            let result = if server_ids.len() > 1 {
                lsp.completion_multi(&uri, line, character, &server_ids, trigger_char)
                    .await
            } else {
                lsp.completion(&uri, line, character, &language_id, trigger_char)
                    .await
            };
            let task_result =
                result.map(|items| crate::editor::lsp_slot::CompletionResult {
                    items,
                    file_path: file_path_for_task,
                    synced_content: None,
                    synced_lsp_version: None,
                });

            let _ = tx.send(task_result);
        });

        self.lsp.slots.completion.fire(task, rx, buffer_version);

        self.set_lsp_status("Requesting completions...".to_string());
        Ok(true)
    }
}

fn filter_supported_trigger(trigger: Option<char>, supported: &HashSet<char>) -> Option<char> {
    let ch = trigger?;
    if supported.contains(&ch) {
        Some(ch)
    } else {
        None
    }
}

fn completion_trigger_context_from_line(line_text: &str, cursor_col: usize) -> (usize, String) {
    let cursor_byte = byte_offset_for_grapheme(line_text, cursor_col).unwrap_or(line_text.len());
    let before_cursor = &line_text[..cursor_byte.min(line_text.len())];

    let mut start_byte = before_cursor.len();
    let graphemes: Vec<(usize, &str)> = grapheme_indices(before_cursor).collect();
    for (byte_offset, grapheme) in graphemes.into_iter().rev() {
        let is_ident = grapheme.chars().all(|c| c == '_' || c.is_alphanumeric());
        if !is_ident {
            break;
        }
        start_byte = byte_offset;
    }

    let trigger_col = grapheme_count(&line_text[..start_byte.min(line_text.len())]);
    let trigger_prefix = line_text[start_byte.min(cursor_byte)..cursor_byte].to_string();

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
    fn completion_trigger_context_double_colon() {
        let (col, prefix) = completion_trigger_context_from_line("foo::bar", 8);
        assert_eq!(col, 5);
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
