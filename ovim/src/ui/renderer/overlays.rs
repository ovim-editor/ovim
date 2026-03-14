use crate::editor::Editor;
use crate::syntax::{Theme, UiGroup};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use super::helpers::grapheme_col_to_display_col;
use super::layout::OverlayContext;

/// Renders hover information as a floating window positioned near the cursor
///
/// Both modes are scrollable (j/k vertical, h/l horizontal):
/// - Preview mode: styled markdown rendering
/// - Navigate mode: raw text view (K from preview to switch)
#[allow(clippy::too_many_arguments)]
pub fn render_hover_window(
    frame: &mut Frame,
    editor: &Editor,
    hover_text: &str,
    scroll_offset: usize,
    ctx: &OverlayContext,
    hover_position: Option<(usize, usize)>,
    is_preview: bool,
    theme: &Theme,
    content_type: crate::editor::HoverContentType,
) {
    let layout = ctx.layout;
    let viewport_start = ctx.viewport_start;
    let buffer_area = layout.buffer_area;
    use super::markdown::{colors, parse_markdown, render_markdown};

    let h_scroll = editor.hover_h_scroll();

    const MIN_WIDTH: u16 = 30;
    const MIN_HEIGHT: u16 = 3;

    // Adaptive max dimensions: use up to 80% of available space, but cap at sane limits.
    let max_width = (buffer_area.width * 4 / 5).max(MIN_WIDTH).min(120);
    let max_height = (buffer_area.height * 4 / 5).max(MIN_HEIGHT).min(40);

    // Parse markdown for preview mode
    let elements = parse_markdown(hover_text);
    let rendered_lines = render_markdown(&elements, max_width as usize, Some(theme));
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
        .clamp(MIN_WIDTH, max_width)
        .min(buffer_area.width.saturating_sub(4));

    let window_height = (total_lines as u16 + 2)
        .clamp(MIN_HEIGHT, max_height)
        .min(buffer_area.height.saturating_sub(2));

    // Calculate cursor screen position
    let (cursor_line, cursor_col) = hover_position.unwrap_or_else(|| {
        let cursor = editor.buffer().cursor();
        (cursor.line(), cursor.col())
    });

    let gutter_width = layout.gutter_width;

    // Convert cursor to screen coordinates
    let rope = editor.buffer().rope();
    let line_text = if cursor_line < editor.buffer().line_count() {
        rope.line(cursor_line).to_string()
    } else {
        String::new()
    };
    let line_text = line_text.trim_end_matches('\n');
    let tab_width = editor.options.tab_width;
    let display_col = grapheme_col_to_display_col(line_text, cursor_col, tab_width);
    let text_width = layout.text_width;

    let screen_line = if editor.options.wrap && text_width > 0 {
        if let Some(wrap_map) = editor.wrap_map() {
            let (abs_row, _) = wrap_map.cursor_to_visual(cursor_line, display_col, line_text);
            let viewport_row = wrap_map.logical_to_visual(viewport_start);
            abs_row.saturating_sub(viewport_row)
        } else {
            cursor_line.saturating_sub(viewport_start)
        }
    } else {
        cursor_line.saturating_sub(viewport_start)
    };

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

    let scrollable = total_lines > content_height;

    // Create title based on content type and scrollability
    let title = match (is_preview, content_type) {
        (true, crate::editor::HoverContentType::Diagnostic) if scrollable => {
            format!(" Diagnostic {}/{} ", clamped_scroll + 1, total_lines)
        }
        (true, crate::editor::HoverContentType::Diagnostic) => " Diagnostic ".to_string(),
        (true, crate::editor::HoverContentType::BlameInfo) => " Blame ".to_string(),
        (true, crate::editor::HoverContentType::AiReasoning) if scrollable => {
            format!(" AI reasoning {}/{} ", clamped_scroll + 1, total_lines)
        }
        (true, crate::editor::HoverContentType::AiReasoning) => " AI reasoning ".to_string(),
        (true, _) if scrollable => {
            format!(
                " {}/{} j/k:scroll q:close ",
                clamped_scroll + 1,
                total_lines
            )
        }
        (true, _) => " q:close K:raw ".to_string(),
        (false, _) if scrollable => {
            format!(
                " {}/{} j/k:scroll q:close ",
                clamped_scroll + 1,
                total_lines
            )
        }
        _ => " q to close ".to_string(),
    };

    // Render content based on mode
    if is_preview {
        // Render styled markdown with scroll support
        let visible_lines: Vec<ratatui::text::Line> = rendered_lines
            .into_iter()
            .skip(clamped_scroll)
            .take(content_height)
            .collect();

        let paragraph = Paragraph::new(visible_lines)
            .style(Style::default().bg(colors::BG))
            .scroll((0, h_scroll as u16))
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
        // Render raw text (navigate mode) — no wrapping, uses h/l for horizontal scroll
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
            .scroll((0, h_scroll as u16))
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
            );

        frame.render_widget(ratatui::widgets::Clear, window_area);
        frame.render_widget(paragraph, window_area);
    }
}

/// Renders the completion menu popup
pub fn render_completion_menu(frame: &mut Frame, editor: &Editor, ctx: &OverlayContext) {
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
    let display_col = grapheme_col_to_display_col(line_text, cursor_col, tab_width);
    let text_width = layout.text_width;

    let screen_line = if editor.options.wrap && text_width > 0 {
        if let Some(wrap_map) = editor.wrap_map() {
            let (abs_row, _) = wrap_map.cursor_to_visual(cursor_line, display_col, line_text);
            let viewport_row = wrap_map.logical_to_visual(viewport_start);
            abs_row.saturating_sub(viewport_row)
        } else {
            cursor_line.saturating_sub(viewport_start)
        }
    } else {
        cursor_line.saturating_sub(viewport_start)
    };

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

/// Compact floating help card for AI chat review mode.
///
/// Uses a rounded border and transparent panel background while clearing
/// underlying content for readability.
pub fn render_ai_review_shortcuts(frame: &mut Frame, theme: &Theme, buffer_area: Rect) {
    if buffer_area.width < 34 || buffer_area.height < 8 {
        return;
    }

    let shortcuts = vec![
        ("\u{2190}/\u{2192}", "navigate edits"),
        ("Enter", "accept"),
        ("Ctrl-r", "back to chat"),
        ("Esc", "close"),
    ];

    let title = " Review Keys ";
    let content_width = shortcuts
        .iter()
        .map(|(k, v)| 1 + k.width() + 3 + v.width())
        .max()
        .unwrap_or(20)
        .max(title.width())
        .max(20);

    let max_panel_width = buffer_area.width.saturating_sub(2) as usize;
    if max_panel_width < 12 {
        return;
    }
    let panel_width = (content_width + 2).min(max_panel_width).max(24) as u16;
    let max_panel_height = buffer_area.height.saturating_sub(2);
    if max_panel_height < 4 {
        return;
    }
    let visible_rows = shortcuts
        .len()
        .min(max_panel_height.saturating_sub(2) as usize);
    let panel_height = (visible_rows + 2) as u16;

    let x = buffer_area
        .x
        .saturating_add(buffer_area.width.saturating_sub(panel_width + 1));
    let y = buffer_area.y.saturating_add(1);
    let panel_area = Rect {
        x,
        y,
        width: panel_width,
        height: panel_height,
    };

    let border_color = crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::Info));
    let key_color =
        crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::TabActiveFg));
    let text_color =
        crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::StatusLineForeground));
    let dash_color = crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::Border));

    let mut lines = Vec::with_capacity(visible_rows);
    for (key, desc) in shortcuts.into_iter().take(visible_rows) {
        let key_text = format!("{key:<7}");
        lines.push(Line::from(vec![
            Span::styled(" ", Style::default().bg(Color::Reset)),
            Span::styled(
                key_text,
                Style::default()
                    .fg(key_color)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " \u{2022} ",
                Style::default().fg(dash_color).bg(Color::Reset),
            ),
            Span::styled(
                desc.to_string(),
                Style::default().fg(text_color).bg(Color::Reset),
            ),
        ]));
    }

    let card = Paragraph::new(lines)
        .style(Style::default().bg(Color::Reset))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(border_color).bg(Color::Reset))
                .title(title)
                .title_style(
                    Style::default()
                        .fg(border_color)
                        .bg(Color::Reset)
                        .add_modifier(Modifier::BOLD),
                ),
        );

    frame.render_widget(ratatui::widgets::Clear, panel_area);
    frame.render_widget(card, panel_area);
}

fn centered_area(full: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(full.width).max(1);
    let height = height.min(full.height).max(1);
    let x = full.x + full.width.saturating_sub(width) / 2;
    let y = full.y + full.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

/// Centered consent dialog for LSP auto-install requests.
pub fn render_lsp_install_dialog(frame: &mut Frame, editor: &Editor, theme: &Theme) {
    let Some((language, server, method)) = editor.pending_lsp_install_summary() else {
        return;
    };

    let full = frame.area();
    if full.width < 40 || full.height < 7 {
        return;
    }
    let width = ((full.width * 70) / 100)
        .max(48)
        .min(100)
        .min(full.width.saturating_sub(2));
    let height = 9u16.min(full.height.saturating_sub(2)).max(7);
    let area = centered_area(full, width, height);

    let border_color = crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::Info));
    let title_color =
        crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::TabActiveFg));
    let text_color =
        crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::StatusLineForeground));
    let hint_color = crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::Border));

    let content = vec![
        Line::from(Span::styled(
            format!("Install {} for {} support?", server, language),
            Style::default().fg(text_color).bg(Color::Reset),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("Method: {}", method),
            Style::default().fg(hint_color).bg(Color::Reset),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Enter: install   A: always auto-install   Esc: skip",
            Style::default()
                .fg(title_color)
                .bg(Color::Reset)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    let dialog = Paragraph::new(content).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(border_color).bg(Color::Reset))
            .title(" Install Language Server ")
            .title_style(
                Style::default()
                    .fg(title_color)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ),
    );

    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(dialog, area);
}

/// Centered permission dialog for AI chat approval requests.
///
/// This is used for high-attention, blocking prompts (tool approval and
/// no-repo folder approval) instead of relying on low-visibility status bars.
pub fn render_ai_chat_permission_dialog(frame: &mut Frame, editor: &Editor, theme: &Theme) {
    let pending_no_repo = editor.ai_chat_has_pending_no_repo_folder_approval();
    let pending_tool = editor.ai_chat_has_pending_tool_approval();
    if !pending_no_repo && !pending_tool {
        return;
    }

    let (title, summary, hints) = if pending_no_repo {
        (
            " Folder Access Permission ",
            editor
                .ai_chat_pending_no_repo_folder_approval_summary()
                .unwrap_or_else(|| "Allow folder access for this chat session?".to_string()),
            "Enter/Ctrl-Y allow   Esc/Ctrl-N deny",
        )
    } else {
        (
            " Tool Permission ",
            editor
                .ai_chat_pending_tool_approval_summary()
                .unwrap_or_else(|| "Allow requested tool action?".to_string()),
            "Enter/Ctrl-Y allow once   Ctrl-A allow for chat   Esc/Ctrl-N deny",
        )
    };

    let full = frame.area();
    if full.width < 40 || full.height < 7 {
        return;
    }
    let width = ((full.width * 70) / 100)
        .max(48)
        .min(100)
        .min(full.width.saturating_sub(2));
    let height = 9u16.min(full.height.saturating_sub(2)).max(7);
    let area = centered_area(full, width, height);

    let border_color = crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::Info));
    let title_color =
        crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::TabActiveFg));
    let text_color =
        crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::StatusLineForeground));
    let hint_color = crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::Border));

    let content = vec![
        Line::from(Span::styled(
            summary,
            Style::default().fg(text_color).bg(Color::Reset),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "This request blocks agent progress until resolved.",
            Style::default().fg(hint_color).bg(Color::Reset),
        )),
        Line::from(""),
        Line::from(Span::styled(
            hints,
            Style::default()
                .fg(title_color)
                .bg(Color::Reset)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    let dialog = Paragraph::new(content).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(border_color).bg(Color::Reset))
            .title(title)
            .title_style(
                Style::default()
                    .fg(title_color)
                    .bg(Color::Reset)
                    .add_modifier(Modifier::BOLD),
            ),
    );

    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_widget(dialog, area);
}
