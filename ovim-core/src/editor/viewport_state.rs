use super::WrapMap;

/// Viewport and scroll state for the editor.
///
/// Groups fields related to vertical scrolling, viewport sizing,
/// and soft-wrap rendering.
pub struct ViewportState {
    /// Viewport height (rows) - updated from UI layer
    pub viewport_height: usize,
    /// Scroll offset (top visible line) - maintained with scrolloff
    pub scroll_offset: usize,
    /// Visual sub-row offset within `scroll_offset` (no-window-manager fallback
    /// mirror of `Window::scroll_subrow`). Rows of the top wrapped line hidden
    /// above the top edge; always 0 when wrap is off.
    pub scroll_subrow: usize,
    /// One-shot post-input viewport policy. Kept private so commands express
    /// intent through `preserve_after_input` rather than coordinating a flag.
    preserve_after_input: bool,
    /// Wrap map for soft wrap rendering — the **no-window-manager fallback**
    /// slot (headless mode, the test harness). When a `WindowManager` exists,
    /// each `Window` owns its wrap map and `Editor::wrap_map()` /
    /// `Editor::ensure_wrap_map` route through the focused window instead.
    /// (roadmap 19)
    pub wrap_map: Option<WrapMap>,
    /// Decoration generation when `wrap_map` was last built. When decorations
    /// change (e.g. inlay hints arrive), the wrap map must be rebuilt to
    /// account for new inline widths.
    pub wrap_decoration_generation: u64,
}

impl ViewportState {
    pub(super) fn preserve_after_input(&mut self) {
        self.preserve_after_input = true;
    }

    pub(super) fn should_preserve_after_input(&self) -> bool {
        self.preserve_after_input
    }

    /// Consumes the one-shot policy at the shared post-input boundary.
    pub(super) fn take_preserve_after_input(&mut self) -> bool {
        std::mem::take(&mut self.preserve_after_input)
    }
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            viewport_height: 24,
            scroll_offset: 0,
            scroll_subrow: 0,
            preserve_after_input: false,
            wrap_map: None,
            wrap_decoration_generation: 0,
        }
    }
}
