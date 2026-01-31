use crate::editor::Editor;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use super::layout::OverlayContext;

/// Renders hover information as a floating window positioned near the cursor
///
/// In preview mode: renders markdown, any key dismisses
/// In navigate mode: shows raw text, scrollable
#[allow(clippy::too_many_arguments)]
pub fn render_hover_window(
    frame: &mut Frame,
    editor: &Editor,
    hover_text: &str,
    scroll_offset: usize,
    ctx: &OverlayContext,
    hover_position: Option<(usize, usize)>,
    is_preview: bool,
    theme: &crate::syntax::Theme,
    content_type: crate::editor::HoverContentType,
) {
    let layout = ctx.layout;
    let viewport_start = ctx.viewport_start;
    let buffer_area = layout.buffer_area;
    use super::markdown::{colors, parse_markdown, render_markdown};

    const MIN_WIDTH: u16 = 30;
    const MAX_WIDTH: u16 = 80;
    const MIN_HEIGHT: u16 = 3;
    const MAX_HEIGHT: u16 = 15;

    // Parse markdown for preview mode
    let elements = parse_markdown(hover_text);
    let rendered_lines = render_markdown(&elements, MAX_WIDTH as usize, Some(theme));
    let total_lines = if is_preview {
        rendered_lines.len()
    } else {
        hover_text.lines().count()
    };

    // Calculate content dimensions
    let content_width = if is_preview {
        rendered_lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.len())
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(30)
    } else {
        hover_text.lines().map(|l| l.len()).max().unwrap_or(30)
    };

    let window_width = (content_width as u16 + 4)
        .clamp(MIN_WIDTH, MAX_WIDTH)
        .min(buffer_area.width.saturating_sub(4));

    let window_height = (total_lines as u16 + 2)
        .clamp(MIN_HEIGHT, MAX_HEIGHT)
        .min(buffer_area.height.saturating_sub(2));

    // Calculate cursor screen position
    let (cursor_line, cursor_col) = hover_position.unwrap_or_else(|| {
        let cursor = editor.buffer().cursor();
        (cursor.line(), cursor.col())
    });

    let gutter_width = layout.gutter_width;

    // Convert cursor to screen coordinates
    let screen_line = cursor_line.saturating_sub(viewport_start);
    let rope = editor.buffer().rope();
    let line_text = if cursor_line < editor.buffer().line_count() {
        rope.line(cursor_line).to_string()
    } else {
        String::new()
    };
    let line_text = line_text.trim_end_matches('\n');
    let tab_width = editor.options.tab_width;
    let display_col = super::helpers::char_col_to_display_col(line_text, cursor_col, tab_width);

    let cursor_screen_x = buffer_area.x + gutter_width as u16 + display_col as u16;
    let cursor_screen_y = buffer_area.y + screen_line as u16;

    // Determine vertical position (prefer below, fallback to above)
    let space_below = buffer_area.bottom().saturating_sub(cursor_screen_y + 1);
    let space_above = cursor_screen_y.saturating_sub(buffer_area.y);

    let window_y = if space_below >= window_height || space_below >= space_above {
        // Position below cursor
        (cursor_screen_y + 1).min(buffer_area.bottom().saturating_sub(window_height))
    } else {
        // Position above cursor
        cursor_screen_y.saturating_sub(window_height)
    };

    // Determine horizontal position (start at cursor, shift left if needed)
    let window_x = cursor_screen_x
        .min(buffer_area.right().saturating_sub(window_width))
        .max(buffer_area.x);

    let window_area = Rect {
        x: window_x,
        y: window_y,
        width: window_width,
        height: window_height,
    };

    // Calculate visible content height
    let content_height = window_height.saturating_sub(2) as usize;

    // Clamp scroll offset
    let max_scroll = total_lines.saturating_sub(content_height);
    let clamped_scroll = scroll_offset.min(max_scroll);

    // Create title based on content type
    let title = match (is_preview, content_type) {
        (true, crate::editor::HoverContentType::Diagnostic) => " Diagnostic ".to_string(),
        (true, crate::editor::HoverContentType::LspHover) => " K: navigate ".to_string(),
        (false, _) if total_lines > content_height => {
            format!(" {}/{} j/k:scroll q:close ", clamped_scroll + 1, total_lines)
        }
        _ => " q to close ".to_string(),
    };

    // Render content based on mode
    if is_preview {
        // Render styled markdown
        let visible_lines: Vec<ratatui::text::Line> = rendered_lines
            .into_iter()
            .skip(clamped_scroll)
            .take(content_height)
            .collect();

        let paragraph = Paragraph::new(visible_lines)
            .style(Style::default().bg(colors::BG))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .border_style(Style::default().fg(colors::BORDER))
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(colors::BORDER)
                            .add_modifier(Modifier::BOLD),
                    ),
            );

        frame.render_widget(ratatui::widgets::Clear, window_area);
        frame.render_widget(paragraph, window_area);
    } else {
        // Render raw text (navigate mode)
        let all_lines: Vec<&str> = hover_text.lines().collect();
        let visible_lines: Vec<String> = all_lines
            .iter()
            .skip(clamped_scroll)
            .take(content_height)
            .map(|line| format!(" {} ", line))
            .collect();

        let text = visible_lines.join("\n");

        let paragraph = Paragraph::new(text)
            .style(
                Style::default()
                    .bg(Color::Rgb(30, 30, 40))
                    .fg(Color::Rgb(230, 230, 230)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Rgb(137, 180, 250)))
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Rgb(137, 180, 250))
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .wrap(ratatui::widgets::Wrap { trim: false });

        frame.render_widget(ratatui::widgets::Clear, window_area);
        frame.render_widget(paragraph, window_area);
    }
}

/// Renders the completion menu popup
pub fn render_completion_menu(
    frame: &mut Frame,
    editor: &Editor,
    ctx: &OverlayContext,
) {
    let layout = ctx.layout;
    let viewport_start = ctx.viewport_start;
    let buffer_area = layout.buffer_area;
    let completion_menu = editor.completion_menu();
    if !completion_menu.is_visible() {
        return;
    }

    let items = completion_menu.items();
    if items.is_empty() {
        return;
    }

    // Get cursor position on screen
    let cursor = editor.buffer().cursor();
    let cursor_line = cursor.line();
    let cursor_col = cursor.col();
    let screen_line = cursor_line.saturating_sub(viewport_start);

    // Get the line text and convert character column to display column
    let rope = editor.buffer().rope();
    let line_count = editor.buffer().line_count();
    let line_text = if cursor_line < line_count {
        rope.line(cursor_line).to_string()
    } else {
        String::new()
    };
    let line_text = line_text.trim_end_matches('\n');

    // Convert character column to display column (accounting for tabs and emojis)
    let tab_width = editor.options.tab_width;
    let display_col = super::helpers::char_col_to_display_col(line_text, cursor_col, tab_width);

    let gutter_width = layout.gutter_width;

    // Position menu below cursor
    let menu_x = buffer_area.x + gutter_width as u16 + display_col as u16;
    let menu_y = buffer_area.y + screen_line as u16 + 1; // Below current line

    // Determine menu dimensions
    let max_items_to_show = 10;
    let num_items = items.len().min(max_items_to_show);
    let menu_height = num_items as u16 + 2; // +2 for borders

    // Calculate width based on longest label
    // Use UnicodeWidthStr::width() instead of len() because CJK characters
    // are 2 columns wide while ASCII characters are 1 column wide
    let max_label_len = items
        .iter()
        .take(max_items_to_show)
        .map(|item| item.label.width())
        .max()
        .unwrap_or(20);
    let menu_width = (max_label_len + 4).min(60) as u16; // +4 for padding and borders

    // Adjust position if menu would go off screen
    let menu_x = menu_x.min(buffer_area.width.saturating_sub(menu_width));
    let menu_y = if menu_y + menu_height > buffer_area.y + buffer_area.height {
        // Show above cursor if not enough space below
        (buffer_area.y + screen_line as u16)
            .saturating_sub(menu_height)
            .max(buffer_area.y)
    } else {
        menu_y
    };

    let menu_area = Rect::new(
        menu_x,
        menu_y,
        menu_width,
        menu_height.min(buffer_area.height),
    );

    // Build menu lines
    let selected_index = completion_menu.selected_index();
    let mut lines = Vec::new();

    for (idx, item) in items.iter().take(max_items_to_show).enumerate() {
        let is_selected = idx == selected_index;
        let style = if is_selected {
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(Color::Rgb(40, 44, 52)).fg(Color::White)
        };

        // Format: "label"  or "  label" with selection indicator
        let prefix = if is_selected { "> " } else { "  " };
        let text = format!("{}{}", prefix, item.label);

        lines.push(Line::from(Span::styled(text, style)));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Rgb(40, 44, 52)));

    let paragraph = Paragraph::new(lines).block(block);

    // Clear background and render menu
    frame.render_widget(ratatui::widgets::Clear, menu_area);
    frame.render_widget(paragraph, menu_area);
}
