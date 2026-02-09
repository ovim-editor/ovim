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
    /// Skip scroll update flag - set by viewport commands (zz, zt, zb) to prevent auto-scroll
    pub skip_scroll_update: bool,
    /// Wrap map for soft wrap rendering (computed lazily when wrap=true)
    pub wrap_map: Option<WrapMap>,
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            viewport_height: 24,
            scroll_offset: 0,
            skip_scroll_update: false,
            wrap_map: None,
        }
    }
}
