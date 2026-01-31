/// Cached rendering state used by the UI layer and mouse input.
///
/// Groups fields that bridge between the renderer and input handling:
/// mouse drag state and cached layout geometry from the last render pass.
pub struct RenderCache {
    /// Mouse interaction state (dragging, drag origin)
    pub mouse_state: super::MouseState,
    /// Cached buffer area from last render (for screen-to-buffer coordinate conversion)
    pub last_buffer_area: Option<ratatui::layout::Rect>,
    /// Cached gutter width from last render
    pub last_gutter_width: usize,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self {
            mouse_state: super::MouseState::default(),
            last_buffer_area: None,
            last_gutter_width: 0,
        }
    }
}
