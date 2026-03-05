use crate::editor::Editor;
use lsp_types::Position;

impl Editor {
    /// Refresh inlay hints for the visible region of the current buffer.
    ///
    /// Called from the event loop when diagnostics change (which implies the
    /// server has processed recent edits and hints may have changed too).
    pub async fn refresh_inlay_hints(&mut self) {
        let lsp = match &self.lsp_state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => return,
        };

        let file_path = match self.buffer().file_path() {
            Some(p) => p.to_string(),
            None => return,
        };

        let uri = match crate::lsp::uri_from_file_path(&file_path) {
            Some(u) => u,
            None => return,
        };

        let language_id =
            match crate::syntax::LanguageRegistry::get_lsp_language_id(&file_path) {
                Some(id) => id.to_string(),
                None => return,
            };

        // Request hints for the visible viewport plus some margin
        let start_line = self.scroll_offset();
        let end_line = start_line + self.viewport_height() + 10;
        let range = lsp_types::Range {
            start: Position {
                line: start_line as u32,
                character: 0,
            },
            end: Position {
                line: end_line as u32,
                character: 0,
            },
        };

        match lsp.inlay_hints(&uri, range, &language_id).await {
            Ok(hints) => {
                self.lsp_state.inlay_hints = hints;
            }
            Err(_) => {
                // Silently ignore — server may not support hints or may not be ready
            }
        }
    }

    /// Get inlay hints for a specific line (0-indexed).
    pub fn inlay_hints_for_line(&self, line: usize) -> Vec<&lsp_types::InlayHint> {
        self.lsp_state
            .inlay_hints
            .iter()
            .filter(|h| h.position.line as usize == line)
            .collect()
    }
}
