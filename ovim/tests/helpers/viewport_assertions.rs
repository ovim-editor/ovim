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

    /// The absolute visual (wrapped) row currently drawn at the top of the
    /// viewport. Requires soft-wrap to be on with a built wrap map.
    ///
    /// This is `logical_to_visual(scroll_offset) + scroll_subrow` — the single
    /// place the visual-row origin is computed, so the visual assertions below
    /// all follow from it.
    pub fn viewport_top_visual_row(&self) -> usize {
        let wm = self
            .editor
            .window_manager()
            .expect("window manager required for visual-row assertions");
        let window = wm.focused_window().expect("a focused window");
        let map = window
            .wrap_map()
            .expect("wrap map must be built (call ensure_wrap_map)");
        map.viewport_top_visual_row(window.scroll_offset(), window.scroll_subrow())
    }

    /// The absolute visual (wrapped) row the cursor sits on.
    pub fn cursor_absolute_visual_row(&self) -> usize {
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
        cursor_visual
    }

    /// The cursor's visual row measured from the top of the viewport, in
    /// *visual* (wrapped) rows. This is the ground truth for "where on screen is
    /// the cursor" under wrapping — e.g. `zz` should leave it near
    /// `viewport_height() / 2`.
    pub fn cursor_visual_row_from_top(&self) -> usize {
        self.cursor_absolute_visual_row()
            .saturating_sub(self.viewport_top_visual_row())
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
