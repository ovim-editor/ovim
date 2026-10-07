use crate::editor::Editor;
use ovim_core::ai::chat_types::ChatFocus;
use ovim_core::editor::ai_chat_input::ChatInputRow;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::ai_chat_style::{BG_INPUT, BG_PANEL, TEXT_DIM, TEXT_NORMAL};
use super::ai_chat_text::{text_display_width, truncate_with_ellipsis};

pub(super) fn render_chat_image_gallery(
    frame: &mut Frame,
    editor: &mut Editor,
    area: Rect,
    paths: &[std::path::PathBuf],
) {
    const THUMB_WIDTH: u16 = 14;
    let capacity = (area.width / THUMB_WIDTH).max(1) as usize;
    let first = paths.len().saturating_sub(capacity);
    for (index, path) in paths[first..].iter().enumerate() {
        let outer = Rect::new(
            area.x + index as u16 * THUMB_WIDTH,
            area.y,
            THUMB_WIDTH.min(
                area.right()
                    .saturating_sub(area.x + index as u16 * THUMB_WIDTH),
            ),
            area.height,
        );
        if outer.width < 4 {
            break;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image");
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(82, 139, 255)))
                .title(truncate_with_ellipsis(
                    name,
                    outer.width.saturating_sub(2) as usize,
                )),
            outer,
        );
        let image_area = Rect::new(
            outer.x + 1,
            outer.y + 1,
            outer.width.saturating_sub(2),
            outer.height.saturating_sub(2),
        );
        editor.render_cache.ai_chat_image_thumbnails.push((
            crate::key_convert::convert_ratatui_rect(image_area),
            path.clone(),
        ));
    }
}

// ---------------------------------------------------------------------------
// Text Input
// ---------------------------------------------------------------------------

pub(super) fn render_text_input(
    frame: &mut Frame,
    editor: &mut Editor,
    area: Rect,
    wrapped_rows: &[ChatInputRow],
    visible_start: usize,
) {
    editor.render_cache.ai_chat_input_area = Some(crate::key_convert::convert_ratatui_rect(area));
    editor.render_cache.ai_chat_input_rows.clear();
    if area.height == 0 || area.width < 4 {
        return;
    }

    let focus = editor.ai_chat_focus();
    let input = editor.ai_chat_input().to_string();
    let allow_edits = editor.ai_chat_allow_edits();

    let border_color = if focus == ChatFocus::TextInput {
        Color::Rgb(82, 139, 255)
    } else {
        Color::Rgb(60, 66, 80)
    };

    let border_style = Style::default().fg(border_color).bg(BG_PANEL);
    let w = area.width as usize;

    // Top border of input box
    let image_names = editor
        .ai_chat_pending_images()
        .iter()
        .map(|image| image.file_name())
        .collect::<Vec<_>>();
    let code_attachment = editor.ai_chat_pending_code_attachment().map(|attachment| {
        let suffix = if editor.ai_chat_pending_code_attachment_modified() {
            " · modified"
        } else {
            ""
        };
        format!("{}{suffix}", attachment.label())
    });
    let composer_title = editor.ai_agent_composer_title();
    let top = if let Some(title) = composer_title {
        let title = truncate_with_ellipsis(&title, w.saturating_sub(4));
        let fill = w.saturating_sub(2 + text_display_width(&title));
        format!("╭{title}{}╮", "─".repeat(fill))
    } else if image_names.is_empty() && code_attachment.is_none() {
        format!("╭{}╮", "─".repeat(w.saturating_sub(2)))
    } else {
        let mut attachments = image_names;
        if let Some(label) = code_attachment {
            attachments.push(label);
        }
        let title = truncate_with_ellipsis(
            &format!(" 📎 {} ", attachments.join(", ")),
            w.saturating_sub(4),
        );
        let fill = w.saturating_sub(2 + text_display_width(&title));
        format!("╭{title}{}╮", "─".repeat(fill))
    };
    let top_line = Line::from(Span::styled(top, border_style));
    frame.render_widget(
        Paragraph::new(vec![top_line]),
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );

    // Input content lines
    let content_rows = (area.height as usize).saturating_sub(1); // minus top border
    if content_rows == 0 {
        return;
    }

    let prompt =
        editor
            .ai_agent_composer_prompt()
            .unwrap_or(if allow_edits { ">> " } else { "?  " });
    let prompt_len = prompt.len(); // 3
    let prefix_total = 2 + prompt_len; // "│ " + prompt = 5
    let suffix_len = 2; // " │"
    let content_width = w.saturating_sub(prefix_total + suffix_len);
    editor.render_cache.ai_chat_input_content_width = content_width;

    let show_active_hint = input.is_empty() && editor.ai_chat_round_active();
    let input_fg = if show_active_hint {
        TEXT_DIM
    } else {
        TEXT_NORMAL
    };
    let input_style = Style::default().fg(input_fg).bg(BG_INPUT);

    let visible_rows = wrapped_rows
        .iter()
        .skip(visible_start)
        .take(content_rows)
        .copied()
        .collect::<Vec<_>>();

    for (row_idx, row) in visible_rows.iter().enumerate() {
        let display = if show_active_hint {
            truncate_with_ellipsis(
                "Enter steers after tool · Tab queues next round",
                content_width,
            )
        } else {
            input[row.visible_start..row.end].to_string()
        };
        let display_width =
            crate::display::display_width(&display, editor.indent_options().tab_width);
        let padding = content_width.saturating_sub(display_width);

        let absolute_row = visible_start + row_idx;
        let row_prefix = if absolute_row == 0 { prompt } else { "   " };

        let line = Line::from(vec![
            Span::styled("│ ", border_style),
            Span::styled(
                row_prefix,
                Style::default().fg(Color::Rgb(82, 139, 255)).bg(BG_INPUT),
            ),
            Span::styled(format!("{display}{}", " ".repeat(padding)), input_style),
            Span::styled(" │", border_style),
        ]);
        frame.render_widget(
            Paragraph::new(vec![line]),
            Rect {
                x: area.x,
                y: area.y + 1 + row_idx as u16,
                width: area.width,
                height: 1,
            },
        );
        editor.render_cache.ai_chat_input_rows.push((
            crate::key_convert::convert_ratatui_rect(Rect {
                x: area.x + prefix_total as u16,
                y: area.y + 1 + row_idx as u16,
                width: content_width as u16,
                height: 1,
            }),
            row.start,
            row.visible_start,
            row.end,
        ));
    }

    // Fill remaining content rows with empty bordered lines
    for row_idx in visible_rows.len()..content_rows {
        let padding = content_width + prompt_len;
        let line = Line::from(vec![
            Span::styled("│ ", border_style),
            Span::styled(" ".repeat(padding), Style::default().bg(BG_INPUT)),
            Span::styled(" │", border_style),
        ]);
        frame.render_widget(
            Paragraph::new(vec![line]),
            Rect {
                x: area.x,
                y: area.y + 1 + row_idx as u16,
                width: area.width,
                height: 1,
            },
        );
    }
}
