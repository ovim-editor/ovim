use ovim::editor::Editor;

/// Helper struct for viewport assertions in tests
pub struct ViewportAssertion<'a> {
    editor: &'a Editor,
}

impl<'a> ViewportAssertion<'a> {
    pub fn new(editor: &'a Editor) -> Self {
        Self { editor }
    }

    pub fn cursor_line(&self) -> usize {
        self.editor.buffer().cursor().line()
    }

    pub fn cursor_col(&self) -> usize {
        self.editor.buffer().cursor().col()
    }

    pub fn scroll_offset(&self) -> usize {
        if let Some(wm) = self.editor.window_manager() {
            if let Some(window) = wm.focused_window() {
                return window.scroll_offset();
            }
        }
        self.editor.scroll_offset()
    }

    pub fn line_at_viewport_position(&self, pos: usize) -> usize {
        self.scroll_offset() + pos
    }

    pub fn visible_line_numbers(&self) -> Vec<usize> {
        let offset = self.scroll_offset();
        let height = self.editor.viewport_height();
        let max_line = self.editor.buffer().line_count().saturating_sub(1);

        (offset..offset + height)
            .take_while(|&line| line <= max_line)
            .collect()
    }

    pub fn debug_display(&self) -> String {
        let cursor = self.editor.buffer().cursor();
        format!(
            "Cursor: L{}:C{} | Scroll: {} | Viewport: {} | Visible: {:?}",
            cursor.line(),
            cursor.col(),
            self.scroll_offset(),
            self.editor.viewport_height(),
            self.visible_line_numbers()
        )
    }
}
