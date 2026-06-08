use ovim::editor::Editor;

/// Helper struct for viewport assertions in tests
pub struct ViewportAssertion<'a> {
    editor: &'a Editor,
}

#[allow(
    dead_code,
    reason = "Viewport helpers are consumed by specific viewport-focused tests, not by every target that includes this module."
)]
impl<'a> ViewportAssertion<'a> {
    pub fn new(editor: &'a Editor) -> Self {
        Self { editor }
    }

    pub fn cursor_line(&self) -> usize {
        self.editor.buffer().cursor().line()
    }

    pub fn cursor_col(&self) -> usize {
        self.editor.buffer().cursor().col().0
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

    /// The focused window's height in rows.
    pub fn viewport_height(&self) -> usize {
        if let Some(wm) = self.editor.window_manager() {
            if let Some(window) = wm.focused_window() {
                return window.height() as usize;
            }
        }
        self.editor.viewport_height()
    }

    /// The cursor's visual row measured from the top of the viewport, in
    /// *visual* (wrapped) rows. Requires soft-wrap to be on with a built wrap
    /// map. This is the ground truth for "where on screen is the cursor" under
    /// wrapping — e.g. `zz` should leave it near `viewport_height() / 2`.
    pub fn cursor_visual_row_from_top(&self) -> usize {
        let wm = self
            .editor
            .window_manager()
            .expect("window manager required for visual-row assertions");
        let window = wm.focused_window().expect("a focused window");
        let map = window
            .wrap_map()
            .expect("wrap map must be built (call ensure_wrap_map)");
        let cursor = self.editor.buffer().cursor();
        let line = cursor.line();
        let line_text = self.editor.buffer().line_text(line).unwrap_or_default();
        // Tests using this helper are ASCII (no tabs/decorations), so the
        // grapheme column equals the display column.
        let (cursor_visual, _) = map.cursor_to_visual(line, cursor.col().0, &line_text);
        let top_visual = map.logical_to_visual(window.scroll_offset());
        cursor_visual.saturating_sub(top_visual)
    }

    /// Whether the cursor's visual row currently falls within the viewport.
    pub fn cursor_is_visible(&self) -> bool {
        self.cursor_visual_row_from_top() < self.viewport_height()
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
