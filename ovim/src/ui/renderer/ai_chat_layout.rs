use ovim_core::editor::ai_chat_input::{
    chat_input_cursor_row_col, chat_input_visible_start, wrap_chat_input_rows, ChatInputRow,
};
use ratatui::layout::Rect;

const CHAT_HEADER_HEIGHT: u16 = 1;
const MAX_COMPOSER_ROWS: usize = 5;
const COMPOSER_CHROME_WIDTH: usize = 7; // "│ " + prompt + " │"
const COMPOSER_CURSOR_PREFIX_WIDTH: u16 = 5; // "│ " + prompt
const IMAGE_GALLERY_HEIGHT: u16 = 6;

/// Split content area into buffer (left) and chat panel (right).
pub fn compute_chat_split(
    content_area: Rect,
    allow_edits: bool,
    preferred_percent: Option<u16>,
) -> (Rect, Rect) {
    let total = content_area.width;
    let chat_pct = preferred_percent
        .unwrap_or(if allow_edits { 40 } else { 35 })
        .clamp(1, 99);
    let min_chat = 30u16;
    let min_buffer = 40u16;

    let chat_width = ((u32::from(total) * u32::from(chat_pct) / 100) as u16)
        .max(min_chat)
        .min(total.saturating_sub(min_buffer));
    let buffer_width = total.saturating_sub(chat_width);

    (
        Rect::new(
            content_area.x,
            content_area.y,
            buffer_width,
            content_area.height,
        ),
        Rect::new(
            content_area.x + buffer_width,
            content_area.y,
            chat_width,
            content_area.height,
        ),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChatPanelLayout {
    pub tree_area: Option<Rect>,
    pub header_area: Rect,
    pub content_area: Rect,
    pub messages_area: Rect,
    pub gallery_area: Option<Rect>,
    pub input_area: Rect,
    pub input_rows: Vec<ChatInputRow>,
    pub input_visible_start: usize,
    cursor_row: usize,
    cursor_column: usize,
}

impl ChatPanelLayout {
    pub fn resolve(
        chat_area: Rect,
        tree_open: bool,
        input: &str,
        input_cursor: usize,
        tab_width: usize,
        terminal_image_support: bool,
        pending_image_count: usize,
    ) -> Option<Self> {
        if chat_area.width < 4 || chat_area.height < 3 {
            return None;
        }

        let (tree_area, main_area) = if tree_open && chat_area.width > 40 {
            let tree_width = tree_panel_width(chat_area.width);
            (
                Some(Rect::new(
                    chat_area.x,
                    chat_area.y,
                    tree_width,
                    chat_area.height,
                )),
                Rect::new(
                    chat_area.x + tree_width,
                    chat_area.y,
                    chat_area.width.saturating_sub(tree_width),
                    chat_area.height,
                ),
            )
        } else {
            (None, chat_area)
        };
        let header_area = Rect::new(
            main_area.x,
            main_area.y,
            main_area.width,
            CHAT_HEADER_HEIGHT,
        );
        let content_area = Rect::new(
            main_area.x,
            main_area.y.saturating_add(CHAT_HEADER_HEIGHT),
            main_area.width,
            main_area.height.saturating_sub(CHAT_HEADER_HEIGHT),
        );

        let input_content_width = (content_area.width as usize)
            .saturating_sub(COMPOSER_CHROME_WIDTH)
            .max(1);
        let input_rows = wrap_chat_input_rows(input, input_content_width, tab_width);
        let (cursor_row, cursor_column) =
            chat_input_cursor_row_col(input, input_cursor, &input_rows, tab_width);
        let max_input_rows = MAX_COMPOSER_ROWS.min(content_area.height.saturating_sub(1) as usize);
        let visible_input_rows = input_rows.len().min(max_input_rows).max(1);
        let input_visible_start =
            chat_input_visible_start(input_rows.len(), cursor_row, visible_input_rows);
        let input_height = (1 + visible_input_rows) as u16;
        let gallery_height =
            if terminal_image_support && pending_image_count > 0 && content_area.height >= 10 {
                IMAGE_GALLERY_HEIGHT
            } else {
                0
            };
        let messages_height = content_area
            .height
            .saturating_sub(input_height)
            .saturating_sub(gallery_height);
        let messages_area = Rect::new(
            content_area.x,
            content_area.y,
            content_area.width,
            messages_height,
        );
        let gallery_area = (gallery_height > 0).then(|| {
            Rect::new(
                content_area.x,
                content_area.y + messages_height,
                content_area.width,
                gallery_height,
            )
        });
        let input_area = Rect::new(
            content_area.x,
            content_area.y + messages_height + gallery_height,
            content_area.width,
            input_height,
        );

        Some(Self {
            tree_area,
            header_area,
            content_area,
            messages_area,
            gallery_area,
            input_area,
            input_rows,
            input_visible_start,
            cursor_row,
            cursor_column,
        })
    }

    pub fn cursor_position(&self) -> (u16, u16) {
        let x = self
            .input_area
            .x
            .saturating_add(COMPOSER_CURSOR_PREFIX_WIDTH)
            .saturating_add(self.cursor_column as u16)
            .min(self.input_area.x + self.input_area.width.saturating_sub(1));
        let y =
            self.input_area.y + 1 + self.cursor_row.saturating_sub(self.input_visible_start) as u16;
        (x, y)
    }
}

fn tree_panel_width(chat_width: u16) -> u16 {
    (chat_width / 4).clamp(20, 36)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_honors_preference_without_starving_buffer() {
        let area = Rect::new(0, 0, 100, 24);
        assert_eq!(compute_chat_split(area, true, Some(55)).0.width, 45);
        assert_eq!(compute_chat_split(area, true, Some(90)).0.width, 40);
        assert_eq!(compute_chat_split(area, false, None).1.width, 35);
    }

    #[test]
    fn composer_and_cursor_share_the_height_capped_projection() {
        let input = (0..40)
            .map(|index| format!("word{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let panel = Rect::new(40, 2, 42, 18);
        let layout =
            ChatPanelLayout::resolve(panel, false, &input, input.len(), 4, false, 0).unwrap();

        assert_eq!(layout.input_area.bottom(), panel.bottom());
        assert_eq!(layout.cursor_position().1, panel.bottom() - 1);
        assert!(layout.input_rows.len() > MAX_COMPOSER_ROWS);
        assert!(layout.input_visible_start > 0);
    }

    #[test]
    fn tree_and_gallery_are_part_of_the_same_projection() {
        let panel = Rect::new(40, 2, 80, 20);
        let layout = ChatPanelLayout::resolve(panel, true, "", 0, 4, true, 1).unwrap();

        assert!(layout.tree_area.is_some());
        assert_eq!(layout.gallery_area.unwrap().height, IMAGE_GALLERY_HEIGHT);
        assert_eq!(layout.input_area.bottom(), panel.bottom());
        assert_eq!(
            layout.messages_area.height
                + layout.gallery_area.unwrap().height
                + layout.input_area.height,
            layout.content_area.height
        );
    }
}
