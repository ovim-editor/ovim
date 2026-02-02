//! Git blame interaction commands: `gb` (hover popup) and `gB` (full diff tab).

use super::Editor;
use crate::git::{commit_diff, commit_info, is_zero_oid};

impl Editor {
    /// Shows a hover popup with commit metadata for the current line (`gb`).
    pub fn show_blame_info(&mut self) {
        let file_path = match self.buffer().file_path() {
            Some(p) => p.to_string(),
            None => return,
        };

        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col();

        // Resolve the OID for this line
        let oid = self.resolve_blame_oid(&file_path, cursor_line);
        let oid = match oid {
            Some(o) => o,
            None => {
                self.show_blame_popup("No blame data for this line", cursor_line, cursor_col);
                return;
            }
        };

        if is_zero_oid(&oid) {
            self.show_blame_popup("Not yet committed", cursor_line, cursor_col);
            return;
        }

        match commit_info(&file_path, &oid) {
            Ok(info) => {
                let mut text = format!("commit {}\n", info.oid_hex);
                text.push_str(&format!("Author: {}\n", info.author));
                text.push_str(&format!("Date:   {}\n", info.date));
                text.push_str(&format!("\n    {}", info.subject));
                if !info.body.is_empty() {
                    text.push_str(&format!("\n\n{}", info.body));
                }
                self.show_blame_popup(&text, cursor_line, cursor_col);
            }
            Err(e) => {
                self.show_blame_popup(
                    &format!("Error reading commit: {}", e),
                    cursor_line,
                    cursor_col,
                );
            }
        }
    }

    /// Opens the full commit diff in a new scratch buffer tab (`gB`).
    pub fn show_blame_diff(&mut self) {
        let file_path = match self.buffer().file_path() {
            Some(p) => p.to_string(),
            None => return,
        };

        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col();

        let oid = self.resolve_blame_oid(&file_path, cursor_line);
        let oid = match oid {
            Some(o) => o,
            None => {
                self.show_blame_popup("No blame data for this line", cursor_line, cursor_col);
                return;
            }
        };

        if is_zero_oid(&oid) {
            self.show_blame_popup("Not yet committed", cursor_line, cursor_col);
            return;
        }

        match commit_diff(&file_path, &oid) {
            Ok(diff_text) => {
                let short = &oid[..7.min(oid.len())];
                let title = format!("Diff {}", short);
                self.open_scratch_buffer_in_new_tab(&title, &diff_text);
            }
            Err(e) => {
                self.show_blame_popup(
                    &format!("Error reading diff: {}", e),
                    cursor_line,
                    cursor_col,
                );
            }
        }
    }

    /// Shows a blame hover popup with the given text.
    fn show_blame_popup(&mut self, text: &str, line: usize, col: usize) {
        self.lsp_state.hover_info = Some(text.to_string());
        self.lsp_state.hover_scroll = 0;
        self.lsp_state.hover_position = Some((line, col));
        self.lsp_state.hover_content_type =
            crate::editor::lsp_state::HoverContentType::BlameInfo;
        self.mode = crate::mode::Mode::HoverPreview;
        self.mark_dirty();
    }

    /// Resolves the blame OID for a given line.
    ///
    /// First checks if the buffer already has blame data loaded (from `:set blame`),
    /// otherwise loads blame on demand and caches it on the buffer for future lookups.
    fn resolve_blame_oid(&mut self, _file_path: &str, line: usize) -> Option<String> {
        // Try cached blame first
        if let Some(blame) = self.buffer().git_blame() {
            return blame.get(line).map(|info| info.commit_oid.clone());
        }

        // Load blame on demand and cache it on the buffer
        self.buffer_mut().load_git_blame();
        self.buffer()
            .git_blame()
            .and_then(|blame| blame.get(line))
            .map(|info| info.commit_oid.clone())
    }
}
