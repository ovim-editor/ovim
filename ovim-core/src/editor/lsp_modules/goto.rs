//! LSP go-to operations
//!
//! This module handles go-to-definition, go-to-implementation, and go-to-type-definition.
//! These are fundamental LSP navigation features (triggered by 'gd', 'gi', 'gy' in Vim).

use super::super::Editor;
use crate::lsp::uri_from_file_path;
use anyhow::{anyhow, Result};

impl Editor {
    /// Request go-to-definition at current cursor position
    pub fn request_goto_definition(&mut self) {
        self.queue_lsp_action(crate::editor::lsp_state::LspAction::GoToDefinition);
    }

    /// Request go-to-definition at current cursor position, opening in a new tab
    pub fn request_goto_definition_new_tab(&mut self) {
        self.queue_lsp_action(crate::editor::lsp_state::LspAction::GoToDefinitionNewTab);
    }

    /// Request go-to-implementation at current cursor position
    pub fn request_goto_implementation(&mut self) {
        self.queue_lsp_action(crate::editor::lsp_state::LspAction::GoToImplementation);
    }

    /// Request go-to-implementation at current cursor position, opening in a new tab
    pub fn request_goto_implementation_new_tab(&mut self) {
        self.queue_lsp_action(crate::editor::lsp_state::LspAction::GoToImplementationNewTab);
    }

    /// Request go-to-type-definition at current cursor position
    pub fn request_goto_type(&mut self) {
        self.queue_lsp_action(crate::editor::lsp_state::LspAction::GoToType);
    }

    /// Implementation of goto-definition (optionally in a new tab)
    async fn goto_definition_common(&mut self, new_tab: bool) -> Result<bool> {
        // Check if LSP is enabled and clone the Arc to avoid borrow issues
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        // Get current file URI - must be absolute path
        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            self.set_lsp_status("Save file first to use goto-definition".to_string());
            return Ok(false);
        };

        // Convert to absolute path if needed
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

        // Get cursor position
        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col());

        // Detect language from file extension
        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        crate::lsp_debug!(
            "LSP-REQUEST",
            "goto_definition: file={}, line={}, col={}, char={}, uri={:?}",
            file_path,
            line,
            cursor.col(),
            character,
            uri
        );

        // Cancel any existing pending definition request by aborting the task
        if let Some((_, old)) = self.lsp_state.pending_lsp_responses.definition.take() {
            crate::lsp_debug!(
                "LSP-DEFINITION",
                "Aborting previous pending definition request"
            );
            old.task.abort();
        }

        // Ensure document is synced before making the request
        // CRITICAL: If we just typed something, the debounced didChange might not
        // have been sent yet. We need to flush it to get correct results.
        let did_flush = self.ensure_lsp_document_synced().await;
        if did_flush {
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        }

        // Resolve the server group responsible for this document.
        let server_ids = lsp.servers_for_document(language_id, std::path::Path::new(&file_path));

        // Spawn definition request in background (non-blocking)
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = if server_ids.len() > 1 {
                lsp.goto_definition_multi(&uri, line, character, &server_ids)
                    .await
            } else {
                lsp.goto_definition(&uri, line, character, language_id)
                    .await
            };
            let _ = tx.send(result);
            Ok(None)
        });

        // Store task handle and receiver for polling
        let pending = crate::editor::lsp_state::PendingLspRequest {
            task,
            receiver: rx,
            started: std::time::Instant::now(),
        };
        self.lsp_state.pending_lsp_responses.definition = Some((new_tab, pending));

        // Show loading status
        self.set_lsp_status("Jumping to definition...".to_string());

        Ok(false) // Return immediately - result will be processed by poll_pending_lsp_responses
    }

    /// Implementation of goto-definition (same buffer)
    pub(in crate::editor) async fn goto_definition_impl(&mut self) -> Result<bool> {
        self.goto_definition_common(false).await
    }

    /// Implementation of goto-definition (new tab)
    pub(in crate::editor) async fn goto_definition_new_tab_impl(&mut self) -> Result<bool> {
        self.goto_definition_common(true).await
    }

    /// Implementation of goto-implementation (optionally in a new tab)
    async fn goto_implementation_common(&mut self, new_tab: bool) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            self.set_lsp_status("Save file first to use goto-implementation".to_string());
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

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Cancel any existing pending implementation request by aborting the task
        if let Some((_, old)) = self.lsp_state.pending_lsp_responses.implementation.take() {
            crate::lsp_debug!(
                "LSP-IMPLEMENTATION",
                "Aborting previous pending implementation request"
            );
            old.task.abort();
        }

        let did_flush = self.ensure_lsp_document_synced().await;
        if did_flush {
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        }

        // Spawn implementation request in background (non-blocking)
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = lsp.implementation(&uri, line, character, language_id).await;
            let _ = tx.send(result); // Send to receiver (ignore if dropped)
            Ok(None) // Return dummy value for JoinHandle (we use receiver for actual result)
        });

        // Store task handle and receiver for polling
        let pending = crate::editor::lsp_state::PendingLspRequest {
            task,
            receiver: rx,
            started: std::time::Instant::now(),
        };
        self.lsp_state.pending_lsp_responses.implementation = Some((new_tab, pending));

        // Show loading status
        self.set_lsp_status("Jumping to implementation...".to_string());

        Ok(false) // Return immediately - result will be processed by poll_pending_lsp_responses
    }

    /// Implementation of goto-implementation (same buffer)
    pub(in crate::editor) async fn goto_implementation_impl(&mut self) -> Result<bool> {
        self.goto_implementation_common(false).await
    }

    /// Implementation of goto-implementation (new tab)
    pub(in crate::editor) async fn goto_implementation_new_tab_impl(&mut self) -> Result<bool> {
        self.goto_implementation_common(true).await
    }

    /// Implementation of goto-type-definition
    pub(in crate::editor) async fn goto_type_impl(&mut self) -> Result<bool> {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            self.set_lsp_status("Save file first to use goto-type".to_string());
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

        let language_id = match crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path) {
            Some(id) => id,
            None => {
                self.set_lsp_status("Language not supported for LSP".to_string());
                return Ok(false);
            }
        };

        // Cancel any existing pending type definition request by aborting the task
        if let Some(old) = self.lsp_state.pending_lsp_responses.type_definition.take() {
            crate::lsp_debug!(
                "LSP-TYPE",
                "Aborting previous pending type definition request"
            );
            old.task.abort();
        }

        let did_flush = self.ensure_lsp_document_synced().await;
        if did_flush {
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        }

        // Spawn type definition request in background (non-blocking)
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = lsp
                .type_definition(&uri, line, character, language_id)
                .await;
            let _ = tx.send(result); // Send to receiver (ignore if dropped)
            Ok(None) // Return dummy value for JoinHandle (we use receiver for actual result)
        });

        // Store task handle and receiver for polling
        self.lsp_state.pending_lsp_responses.type_definition =
            Some(crate::editor::lsp_state::PendingLspRequest {
                task,
                receiver: rx,
                started: std::time::Instant::now(),
            });

        // Show loading status
        self.set_lsp_status("Jumping to type definition...".to_string());

        Ok(false) // Return immediately - result will be processed by poll_pending_lsp_responses
    }
}
