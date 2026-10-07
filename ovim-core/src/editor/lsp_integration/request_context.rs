//! Request preparation: the shared request context and the UTF-16 position
//! conversions LSP uses.

use super::*;
use anyhow::{anyhow, Result};
use std::sync::Arc;

/// Context for making an LSP request, encapsulating all the common setup.
pub(in crate::editor) struct LspRequestContext {
    pub lsp: Arc<crate::lsp::LspManager>,
    pub uri: lsp_types::Uri,
    pub file_path: String,
    pub line: u32,
    pub character: u32,
    pub language_id: String,
    /// All server_ids serving this language (primary + companions)
    pub server_ids: Vec<String>,
}

impl Editor {
    // -------------------------------------------------------------------------
    // UTF-16 Conversion Helpers (LSP uses UTF-16 code units for positions)
    // -------------------------------------------------------------------------

    /// Converts a **grapheme** column (from `cursor.col()`) to UTF-16 code units
    /// for LSP `Position.character`.
    ///
    /// The conversion chain is: grapheme index → char index → UTF-16 code units.
    /// Skipping the grapheme→char step was OV-00226 — combining characters (é = e + ◌́)
    /// caused every outbound LSP position to be wrong.
    pub(crate) fn col_to_utf16(&self, line: usize, grapheme_col: usize) -> u32 {
        let rope = self.buffer().rope();
        if line >= rope.len_lines() {
            return 0;
        }

        let line_text = rope.line(line);

        // rope.line() includes the trailing line terminator — strip it for LSP.
        // We strip `\r` too: the rope is LF-only by convention, but a stray `\r`
        // can slip past the input seams (mixed line endings), and the server
        // never saw it (we strip on send), so it must not count toward the
        // UTF-16 offset. (OV-00268)
        let line_str: String = line_text
            .chars()
            .take_while(|&c| c != '\n' && c != '\r')
            .collect();

        // Step 1: grapheme index → char index
        let char_col = crate::unicode::grapheme_to_char_col(
            &line_str,
            crate::unicode::GraphemeCol(grapheme_col),
        );
        let safe_col = char_col.0.min(line_str.chars().count());

        // Step 2: char index → UTF-16 code units
        line_str
            .chars()
            .take(safe_col)
            .map(|c| c.len_utf16() as u32)
            .sum()
    }

    /// Converts UTF-16 code units (from LSP) to a **char** column index.
    ///
    /// Returns a char index suitable for rope operations (`insert_text_at`,
    /// `delete_range`). For cursor positioning (which needs grapheme indices),
    /// use [`utf16_to_grapheme_col`] instead.
    pub(crate) fn utf16_to_col(&self, line: usize, utf16_col: u32) -> crate::unicode::CharCol {
        let rope = self.buffer().rope();
        if line >= rope.len_lines() {
            return crate::unicode::CharCol::ZERO;
        }

        let line_text = rope.line(line);
        let mut utf16_offset = 0u32;
        let mut char_position = 0usize;

        for ch in line_text.chars() {
            if utf16_offset >= utf16_col {
                break;
            }
            // Stop at the line terminator: a server-supplied offset past the
            // end of the line content must not advance into `\n` / `\r`
            // (mirrors the `\r`-strip in `col_to_utf16`). (OV-00268)
            if ch == '\n' || ch == '\r' {
                break;
            }
            utf16_offset += ch.len_utf16() as u32;
            char_position += 1;
        }

        crate::unicode::CharCol(char_position)
    }

    /// Converts UTF-16 code units (from LSP) to a **grapheme** column index.
    ///
    /// Returns a grapheme index suitable for `cursor.set_position()`. This
    /// is the correct conversion for goto-definition targets, reference
    /// locations, and any LSP position that becomes a cursor position.
    pub(crate) fn utf16_to_grapheme_col(&self, line: usize, utf16_col: u32) -> usize {
        let char_col = self.utf16_to_col(line, utf16_col);

        let rope = self.buffer().rope();
        if line >= rope.len_lines() {
            return 0;
        }
        let line_text = rope.line(line);
        let line_str: String = line_text
            .chars()
            .take_while(|&c| c != '\n' && c != '\r')
            .collect();

        crate::unicode::char_to_grapheme_col(&line_str, char_col).0
    }

    /// Prepare common context for an LSP request.
    /// Handles: LSP manager check, file path resolution, URI creation,
    /// cursor position (UTF-16), language detection, and document sync flush.
    pub(in crate::editor) async fn prepare_lsp_request(
        &mut self,
        feature_name: &str,
    ) -> Result<LspRequestContext> {
        let lsp = self
            .lsp
            .state
            .lsp_manager
            .clone()
            .ok_or_else(|| anyhow!("LSP not available"))?;

        let file_path = self
            .buffer()
            .file_path()
            .ok_or_else(|| anyhow!("Save file first to use {}", feature_name))?
            .to_string();

        let abs_path = if std::path::Path::new(&file_path).is_absolute() {
            file_path.clone()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&file_path).to_string_lossy().to_string())
                .map_err(|_| anyhow!("Failed to resolve file path"))?
        };

        let uri = crate::lsp::uri_from_file_path(&abs_path)
            .ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor = self.buffer().cursor();
        let line = cursor.line() as u32;
        let character = self.col_to_utf16(cursor.line(), cursor.col().0);

        let language_id = self
            .language_id_for_path(&file_path)
            .ok_or_else(|| anyhow!("Language not supported for LSP"))?
            .to_string();

        // Flush pending document changes so LSP has the latest content
        self.ensure_lsp_document_synced().await;

        // Resolve the server group responsible for this document (primary + companions).
        let server_ids = lsp.servers_for_document(&language_id, std::path::Path::new(&abs_path));
        if server_ids.is_empty() {
            return Err(anyhow!(
                "No LSP server available for {} in {}",
                feature_name,
                abs_path
            ));
        }

        Ok(LspRequestContext {
            lsp,
            uri,
            file_path,
            line,
            character,
            language_id,
            server_ids,
        })
    }
}
