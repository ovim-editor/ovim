#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatTextPoint {
    pub row: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatTextSelection {
    pub anchor: ChatTextPoint,
    pub head: ChatTextPoint,
    pub moved: bool,
}

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
    /// Click target for the per-chat YOLO policy toggle.
    pub ai_chat_yolo_hitbox: Option<crate::Rect>,
    /// Cached AI chat message-history area from the last render.
    pub ai_chat_history_area: Option<crate::Rect>,
    /// Cached AI chat total rendered row count from last render pass.
    pub ai_chat_last_total_rows: usize,
    /// Cached visible chat row window start (inclusive) from last render.
    pub ai_chat_last_visible_start_row: usize,
    /// Cached visible chat row window end (exclusive) from last render.
    pub ai_chat_last_visible_end_row: usize,
    /// Cached row spans per message from last render (oldest..latest).
    pub ai_chat_last_message_row_spans: Vec<(usize, usize)>,
    /// Absolute rendered row spans for scheduled inputs, oldest first.
    pub ai_chat_last_queued_row_spans: Vec<(usize, usize)>,
    /// Plain text for every rendered history row, including off-screen rows.
    pub ai_chat_rendered_text_rows: Vec<String>,
    /// Mouse-selected range in absolute rendered-history coordinates.
    pub ai_chat_text_selection: Option<ChatTextSelection>,
    /// Whether a chat text-selection drag is currently active.
    pub ai_chat_text_selecting: bool,
    /// Cached AI chat input area from last render.
    pub ai_chat_input_area: Option<crate::Rect>,
    /// Visible composer rows and their source byte ranges.
    pub ai_chat_input_rows: Vec<(crate::Rect, usize, usize, usize)>,
    /// Display width available to composer text on the last render.
    pub ai_chat_input_content_width: usize,
    /// Cached AI chat input cursor position from last render.
    pub ai_chat_input_cursor_pos: Option<(u16, u16)>,
    /// Click target for the Exa API-key dashboard in the setup dialog.
    pub ai_chat_exa_dashboard_hitbox: Option<crate::Rect>,
    /// Hardware cursor position for the Exa key field.
    pub ai_chat_exa_input_cursor_pos: Option<(u16, u16)>,
    /// Whether the frontend selected a real terminal graphics protocol.
    pub terminal_image_support: bool,
    /// Focus regain invalidated terminal-owned image placements. The TUI
    /// consumes this before its next draw and forces a full surface refresh
    /// when the previous frame actually contained an image.
    pub terminal_image_refresh_requested: bool,
    /// Render rectangles for clickable chat-image thumbnails.
    pub ai_chat_image_thumbnails: Vec<(crate::Rect, std::path::PathBuf)>,
    /// Clickable previous/next controls for visible conversation forks.
    pub ai_chat_branch_hitboxes: Vec<(crate::Rect, crate::ai::chat_types::NodeId)>,
    /// Absolute 80ms animation bucket used by the AI chat working spinner.
    pub ai_chat_working_animation_tick: u128,
}
