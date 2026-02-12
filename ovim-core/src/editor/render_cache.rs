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
    /// Cached AI prompt input hit-test area from last render.
    pub ai_prompt_input_area: Option<crate::Rect>,
    /// Cached wrapped AI prompt input rows from last render:
    /// `(row_rect, start_byte, end_byte)`.
    pub ai_prompt_input_rows: Vec<(crate::Rect, usize, usize)>,
    /// Cached AI prompt model picker hitboxes from last render.
    pub ai_prompt_model_hitboxes: Vec<(crate::Rect, String)>,
    /// Cached AI chat input area from last render.
    pub ai_chat_input_area: Option<crate::Rect>,
    /// Cached AI chat input cursor position from last render.
    pub ai_chat_input_cursor_pos: Option<(u16, u16)>,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self {
            mouse_state: super::MouseState::default(),
            last_buffer_area: None,
            last_gutter_width: 0,
            last_text_width: 0,
            last_blame_width: 0,
            ai_prompt_input_area: None,
            ai_prompt_input_rows: Vec::new(),
            ai_prompt_model_hitboxes: Vec::new(),
            ai_chat_input_area: None,
            ai_chat_input_cursor_pos: None,
        }
    }
}
