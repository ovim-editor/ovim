/// Cached rendering state used by the UI layer and mouse input.
///
/// Groups fields that bridge between the renderer and input handling:
/// mouse drag state and cached layout geometry from the last render pass.
pub struct RenderCache {
    /// Mouse interaction state (dragging, drag origin)
    pub mouse_state: super::MouseState,
    /// Cached buffer area from last render (for screen-to-buffer coordinate conversion)
    pub last_buffer_area: Option<crate::Rect>,
    /// Cached gutter width from last render
    pub last_gutter_width: usize,
    /// Cached text width from last render (buffer area width minus gutter, used for wrap calculations)
    pub last_text_width: usize,
    /// Cached blame column width from last render (0 when blame is off)
    pub last_blame_width: usize,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self {
            mouse_state: super::MouseState::default(),
            last_buffer_area: None,
            last_gutter_width: 0,
            last_text_width: 0,
            last_blame_width: 0,
        }
    }
}
