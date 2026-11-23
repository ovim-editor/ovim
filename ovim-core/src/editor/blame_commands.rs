//! Git blame interaction commands: `gb` (hover popup) and `gB` (full diff tab).

use super::Editor;
use crate::git::{commit_diff, commit_info, is_zero_oid};

impl Editor {
    /// Shows a hover popup with commit metadata for the current line (`gb`).
    pub fn show_blame_info(&mut self) {
        let line = self.buffer().cursor().line();
        let col = self.buffer().cursor().col().0;
        self.show_blame_info_at(line, col);
    }

    /// Show commit details at a source line without moving the editing cursor.
    pub(crate) fn show_blame_info_at(&mut self, cursor_line: usize, cursor_col: usize) {
        let file_path = match self.buffer().file_path() {
            Some(p) => p.to_string(),
            None => return,
        };

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

    pub fn blame_mouse_hover_active(&self) -> bool {
        self.lsp.state.blame_mouse_hover
    }

    pub(crate) fn dismiss_blame_mouse_hover(&mut self) {
        if self.blame_mouse_hover_active() {
            self.clear_hover();
            self.mark_dirty();
        }
    }

    pub(crate) fn hover_blame_line(&mut self, line: usize) {
        if self.blame_mouse_hover_active() && self.hover_position() == Some((line, 0)) {
            return;
        }
        // Pointer previews must not take over insert/visual modes or other overlays.
        if self.mode != crate::mode::Mode::Normal {
            return;
        }
        self.show_blame_info_at(line, 0);
        if self.mode == crate::mode::Mode::HoverPreview {
            self.mode = crate::mode::Mode::Normal;
            self.lsp.state.blame_mouse_hover = true;
        }
    }

    /// Opens the full commit diff in a new scratch buffer tab (`gB`).
    pub fn show_blame_diff(&mut self) {
        self.show_blame_diff_at(self.buffer().cursor().line());
    }

    /// Open the clicked annotation's commit without relocating the source cursor.
    pub(crate) fn show_blame_diff_at(&mut self, cursor_line: usize) {
        let file_path = match self.buffer().file_path() {
            Some(p) => p.to_string(),
            None => return,
        };

        let cursor_col = self.buffer().cursor().col().0;

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
                self.open_diff_buffer_in_new_tab(&title, &diff_text);
                self.buffer_mut()
                    .enable_syntax_highlighting_for_path("commit.diff");
                self.set_mode(crate::mode::Mode::Normal);
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
        self.lsp.state.blame_mouse_hover = false;
        self.lsp.state.hover_info = Some(text.to_string());
        self.lsp.state.hover_scroll = 0;
        self.lsp.state.hover_h_scroll = 0;
        self.lsp.state.hover_position = Some((line, col));
        self.lsp.state.hover_content_type = crate::editor::lsp_state::HoverContentType::BlameInfo;
        self.mode = crate::mode::Mode::HoverPreview;
        self.mark_dirty();
    }

    /// Resolves the blame OID for a given line.
    ///
    /// First checks if the buffer already has blame data loaded (from `:blame`),
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
