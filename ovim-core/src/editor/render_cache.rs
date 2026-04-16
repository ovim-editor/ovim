/// Cached rendering state used by the UI layer and mouse input.
///
/// Groups fields that bridge between the renderer and input handling:
/// mouse drag state and cached layout geometry from the last render pass.
#[derive(Default)]
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
    /// Cached AI prompt model picker trigger hitbox from last render.
    pub ai_prompt_model_trigger_hitbox: Option<crate::Rect>,
    /// Cached AI chat panel area from last render (for mouse scroll hit-testing).
    pub last_chat_area: Option<crate::Rect>,
    /// Cached AI chat total rendered row count from last render pass.
    pub ai_chat_last_total_rows: usize,
    /// Cached visible chat row window start (inclusive) from last render.
    pub ai_chat_last_visible_start_row: usize,
    /// Cached visible chat row window end (exclusive) from last render.
    pub ai_chat_last_visible_end_row: usize,
    /// Cached row spans per message from last render (oldest..latest).
    pub ai_chat_last_message_row_spans: Vec<(usize, usize)>,
    /// Cached AI chat input area from last render.
    pub ai_chat_input_area: Option<crate::Rect>,
    /// Cached AI chat input cursor position from last render.
    pub ai_chat_input_cursor_pos: Option<(u16, u16)>,
}
