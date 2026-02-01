//! LSP hover functionality
//!
//! This module handles hover information display (the "K" command in Vim).
//! It includes hover request, caching, scrolling, and display management.

use super::super::Editor;
use crate::lsp::uri_from_file_path;
use anyhow::{anyhow, Result};

impl Editor {
    /// Request hover info at current cursor position
    /// This will set the pending action flag, which will be processed
    /// in the next event loop iteration via process_pending_lsp_actions()
    pub fn request_hover(&mut self) {
        self.queue_lsp_action(crate::editor::lsp_state::LspAction::ShowHover);
    }

    /// Get current hover info text
    pub fn hover_info(&self) -> Option<&str> {
        self.lsp_state.hover_info.as_deref()
    }

    /// Get current hover content type (LSP hover or diagnostic)
    pub fn hover_content_type(&self) -> crate::editor::lsp_state::HoverContentType {
        self.lsp_state.hover_content_type
    }

    /// Clear hover info
    pub fn clear_hover(&mut self) {
        self.lsp_state.hover_info = None;
        self.lsp_state.hover_scroll = 0;
    }

    /// Set hover info directly (used for command output display)
    /// Also switches to HoverPreview mode so the popup is visible
    pub fn set_hover_info(&mut self, info: String) {
        self.lsp_state.hover_info = Some(info);
        self.lsp_state.hover_scroll = 0;
        self.lsp_state.hover_content_type = crate::editor::lsp_state::HoverContentType::LspHover;
        self.mode = crate::mode::Mode::HoverPreview;
        self.mark_dirty();
    }

    /// Get hover scroll position
    pub fn hover_scroll(&self) -> usize {
        self.lsp_state.hover_scroll
    }

    /// Get the cursor position where hover was triggered
    pub fn hover_position(&self) -> Option<(usize, usize)> {
        self.lsp_state.hover_position
    }

    /// Scroll hover window down
    pub fn scroll_hover_down(&mut self, lines: usize) {
        if self.lsp_state.hover_info.is_some() {
            self.lsp_state.hover_scroll = self.lsp_state.hover_scroll.saturating_add(lines);
        }
    }

    /// Scroll hover window up
    pub fn scroll_hover_up(&mut self, lines: usize) {
        self.lsp_state.hover_scroll = self.lsp_state.hover_scroll.saturating_sub(lines);
    }

    /// Implementation of hover request
    pub(in crate::editor) async fn hover_impl(&mut self) -> Result<bool> {
        ovim_core::lsp_debug!("LSP-HOVER", "hover_impl() called");
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => {
                ovim_core::lsp_debug!("LSP-HOVER", "LSP manager found, cloning Arc");
                lsp.clone()
            }
            None => {
                ovim_core::lsp_debug!("LSP-HOVER", "No LSP manager in hover_impl");
                self.set_lsp_status("LSP not available".to_string());
                return Ok(false);
            }
        };

        let Some(file_path_str) = self.buffer().file_path() else {
            self.set_lsp_status("Save file first to use hover".to_string());
            return Ok(false);
        };
        let file_path = file_path_str.to_string(); // Clone to avoid borrow issues

        // Check hover cache first (extract values to avoid borrow issues)
        let cursor = self.buffer().cursor();
        let buffer_version = self.buffer().version();
        let cursor_line = cursor.line();
        let cursor_col = cursor.col();

        if let Some(ref cache) = self.lsp_state.hover_cache {
            if cache.is_valid(&file_path, cursor_line, cursor_col, buffer_version) {
                ovim_core::lsp_info!("LSP-HOVER", "Cache HIT");
                self.lsp_state.hover_info = Some(cache.hover_text.clone());
                self.lsp_state.hover_scroll = 0;
                self.lsp_state.hover_position = Some((cursor_line, cursor_col));
                self.lsp_state.hover_content_type =
                    crate::editor::lsp_state::HoverContentType::LspHover;
                self.mode = crate::mode::Mode::HoverPreview;
                self.mark_dirty();
                self.set_lsp_status(String::new());
                return Ok(true);
            }
        }

        // Cancel any existing pending hover request by aborting the task
        if let Some(crate::editor::lsp_state::PendingLspResponse::Hover(old)) =
            self.lsp_state.pending_lsp_response.take()
        {
            ovim_core::lsp_debug!("LSP-HOVER", "Aborting previous pending hover request");
            old.task.abort();
        }

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

        ovim_core::lsp_debug!(
            "LSP-HOVER",
            "Requesting hover: file={}, line={}, col={}, char={}, uri={:?}",
            file_path,
            line,
            cursor.col(),
            character,
            uri
        );

        // Ensure document is synced before making the request
        let did_flush = self.ensure_lsp_document_synced().await;
        if did_flush {
            // Only sleep if we flushed changes (gives LSP time to process)
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        }

        // Resolve all server_ids for this language (primary + companions)
        let server_ids = lsp.servers_for_language(language_id);

        // Spawn hover request in background (non-blocking)
        // Uses multi-server fan-out to query all servers concurrently
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = if server_ids.len() > 1 {
                lsp.hover_multi(&uri, line, character, &server_ids).await
            } else {
                lsp.hover(&uri, line, character, language_id).await
            };
            let _ = tx.send(result);
            Ok(None)
        });

        // Store task handle and receiver for polling
        self.lsp_state.pending_lsp_response =
            Some(crate::editor::lsp_state::PendingLspResponse::Hover(
                crate::editor::lsp_state::PendingLspRequest {
                    task,
                    receiver: rx,
                    started: std::time::Instant::now(),
                },
            ));

        // Show loading status
        self.set_lsp_status("Loading hover...".to_string());
        ovim_core::lsp_debug!("LSP-HOVER", "Spawned hover request, waiting for response");

        // Return immediately - no blocking!
        Ok(false) // No state change yet
    }
}
