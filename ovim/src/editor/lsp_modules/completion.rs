//! LSP completion functionality
//!
//! This module handles code completion requests and application.
//! Completions are typically triggered by Ctrl+N or automatically in insert mode.

use super::super::Editor;
use crate::lsp::uri_from_file_path;
use anyhow::{anyhow, Result};

impl Editor {
    /// Request completion at current cursor position
    pub fn request_completion(&mut self) {
        self.queue_lsp_action(crate::editor::lsp_state::LspAction::Completion);
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

        let Some(file_path) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use completion".to_string());
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(file_path).to_string_lossy().to_string(),
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

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        self.set_lsp_status("Requesting completions...".to_string());

        let did_flush = self.ensure_lsp_document_synced().await;
        if did_flush {
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        }

        // Query all servers for this language (primary + companions)
        let server_ids = lsp.servers_for_language(language_id);
        let result = if server_ids.len() > 1 {
            lsp.completion_multi(&uri, line, character, &server_ids)
                .await
        } else {
            lsp.completion(&uri, line, character, language_id).await
        };

        match result {
            Ok(items) if !items.is_empty() => {
                self.lsp_state.available_completions = items.clone();
                // TODO: Open completion popup menu
                self.set_lsp_status(format!("Found {} completions", items.len()));
                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No completions available".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Completion request failed: {}", e));
                Err(e)
            }
        }
    }
}
