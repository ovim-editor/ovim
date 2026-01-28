use crate::editor::Editor;
use crate::syntax::HighlightGroup;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Widget},
    Frame,
};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

use super::helpers::{expand_tabs, expand_tabs_with_mapping, truncate_to_width};
use super::layout::OverlayContext;
use super::styles::remap_highlights;

// Picker color palette — single point of tuning for the entire fuzzy finder UI
mod picker_colors {
    use ratatui::style::Color;

    pub const BG: Color = Color::Rgb(22, 22, 30);
    pub const BG_ALT: Color = Color::Rgb(24, 24, 32);
    pub const SELECTED: Color = Color::Rgb(45, 50, 70);
    pub const BORDER: Color = Color::Rgb(80, 85, 110);
    pub const SEPARATOR: Color = Color::Rgb(50, 55, 75);
    pub const TITLE: Color = Color::Rgb(140, 160, 240);
    pub const TEXT: Color = Color::Rgb(200, 205, 215);
    pub const TEXT_BRIGHT: Color = Color::Rgb(240, 240, 255);
    pub const TEXT_MUTED: Color = Color::Rgb(100, 110, 140);
    pub const GREEN: Color = Color::Rgb(129, 250, 183);
}

/// A widget that fills every cell in an area with a styled space.
/// This is the proper way to create a solid background - unlike Block which
/// only renders borders, Fill writes to every cell, preventing bleed-through.
struct Fill {
    style: Style,
}

impl Fill {
    fn new(style: Style) -> Self {
        Self { style }
    }

    fn bg(color: Color) -> Self {
        Self::new(Style::default().bg(color))
    }
}

impl Widget for Fill {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_char(' ').set_style(self.style);
            }
        }
    }
}

/// Binary search for the highlight group at a given byte index.
/// Highlights must be sorted by range.start (ascending).
/// Returns the highlight group if byte_idx falls within any range.
/// O(log n) instead of O(n) linear search.
#[inline]
fn find_highlight_at_byte(
    highlights: &[(Range<usize>, HighlightGroup)],
    byte_idx: usize,
) -> Option<HighlightGroup> {
    if highlights.is_empty() {
        return None;
    }

    // Binary search: find the first range where start > byte_idx
    // All ranges that could contain byte_idx have start <= byte_idx
    let partition = highlights.partition_point(|(range, _)| range.start <= byte_idx);

    if partition == 0 {
        // No range starts at or before byte_idx
        return None;
    }

    // Check ranges from partition-1 backwards (most specific/last defined wins)
    // In practice, we usually only need to check 1-2 ranges
    for i in (0..partition).rev() {
        let (range, group) = &highlights[i];
        if range.end > byte_idx {
            return Some(*group);
        }
        // Optimization: if this range ends before byte_idx and the previous
        // range also starts before this one ends, we can stop
        if i > 0 && highlights[i - 1].0.end <= range.start {
            break;
        }
    }

    None
}

/// Renders the LSP progress line (just above status line)
pub fn render_progress_line(frame: &mut Frame, progress_msg: &str, area: Rect) {
    // Right-align the progress message
    let padding_len = area.width.saturating_sub(progress_msg.len() as u16 + 2);
    let progress_line = Line::from(vec![
        Span::raw(" ".repeat(padding_len as usize)),
        Span::styled(
            format!(" {} ", progress_msg),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        ),
    ]);

    let paragraph = Paragraph::new(progress_line).style(Style::default().bg(Color::Black));
    frame.render_widget(paragraph, area);
}

/// Renders the tab bar with overflow handling
pub fn render_tab_bar(frame: &mut Frame, editor: &Editor, area: Rect) {
    let tabs = editor.tab_page_manager().tabs();
    let current_index = editor.current_tab_index();

    if tabs.is_empty() {
        // No tabs to render
        let tab_line = Line::from(Span::styled(
            " ".repeat(area.width as usize),
            Style::default().bg(Color::Black),
        ));
        let paragraph = Paragraph::new(tab_line).style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, area);
        return;
    }

    let mut spans = Vec::new();
    let available_width = area.width as usize;

    // Calculate tab widths
    const MIN_TAB_WIDTH: usize = 10; // Minimum width per tab: " 1 file "
    const SEPARATOR_WIDTH: usize = 1; // Space between tabs
    const OVERFLOW_INDICATOR_WIDTH: usize = 12; // Width for " +N more"

    // Calculate how much space each tab would need
    let mut tab_widths: Vec<usize> = Vec::new();
    let mut total_width = 0;

    for (i, _tab) in tabs.iter().enumerate() {
        let title = editor.get_tab_title(i);
        let tab_text = format!(" {} {} ", i + 1, title);
        let tab_width = tab_text.len();
        tab_widths.push(tab_width);
        total_width += tab_width;
        if i < tabs.len() - 1 {
            total_width += SEPARATOR_WIDTH; // Account for separators
        }
    }

    // Check if we need to handle overflow
    if total_width > available_width {
        // Too many tabs - need to show subset
        // Always show the current tab, then fill in surrounding tabs
        let mut visible_tabs = Vec::new();

        // Start with current tab
        let current_tab_width = tab_widths[current_index].max(MIN_TAB_WIDTH);
        visible_tabs.push(current_index);
        let mut used_width = current_tab_width + OVERFLOW_INDICATOR_WIDTH;

        // Try to add tabs before and after current tab alternately
        let mut before_idx = current_index.saturating_sub(1);
        let mut after_idx = current_index + 1;
        let mut add_before = current_index > 0;
        let mut add_after = after_idx < tabs.len();

        while (add_before || add_after) && used_width < available_width {
            if add_before {
                let tab_width = tab_widths[before_idx].max(MIN_TAB_WIDTH) + SEPARATOR_WIDTH;
                if used_width + tab_width <= available_width.saturating_sub(OVERFLOW_INDICATOR_WIDTH) {
                    visible_tabs.insert(0, before_idx);
                    used_width += tab_width;
                    if before_idx > 0 {
                        before_idx -= 1;
                    } else {
                        add_before = false;
                    }
                } else {
                    add_before = false;
                }
            }

            if add_after && used_width < available_width {
                let tab_width = tab_widths[after_idx].max(MIN_TAB_WIDTH) + SEPARATOR_WIDTH;
                if used_width + tab_width <= available_width.saturating_sub(OVERFLOW_INDICATOR_WIDTH) {
                    visible_tabs.push(after_idx);
                    used_width += tab_width;
                    after_idx += 1;
                    if after_idx >= tabs.len() {
                        add_after = false;
                    }
                } else {
                    add_after = false;
                }
            }
        }

        // Show overflow indicator at the beginning if needed
        let hidden_before = visible_tabs.first().copied().unwrap_or(0);
        if hidden_before > 0 {
            let overflow_text = format!(" +{} ", hidden_before);
            spans.push(Span::styled(
                overflow_text,
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ));
            spans.push(Span::styled(" ", Style::default().bg(Color::Black)));
        }

        // Render visible tabs
        for (idx, &tab_idx) in visible_tabs.iter().enumerate() {
            let is_current = tab_idx == current_index;
            let title = editor.get_tab_title(tab_idx);
            let tab_text = format!(" {} {} ", tab_idx + 1, title);

            let style = if is_current {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            };

            spans.push(Span::styled(tab_text, style));

            // Add separator between tabs
            if idx < visible_tabs.len() - 1 {
                spans.push(Span::styled(" ", Style::default().bg(Color::Black)));
            }
        }

        // Show overflow indicator at the end if needed
        let hidden_after = tabs.len().saturating_sub(visible_tabs.last().copied().unwrap_or(0) + 1);
        if hidden_after > 0 {
            spans.push(Span::styled(" ", Style::default().bg(Color::Black)));
            let overflow_text = format!(" +{} ", hidden_after);
            spans.push(Span::styled(
                overflow_text,
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ));
        }
    } else {
        // All tabs fit - render normally
        for (i, _tab) in tabs.iter().enumerate() {
            let is_current = i == current_index;
            let title = editor.get_tab_title(i);
            let tab_text = format!(" {} {} ", i + 1, title);

            let style = if is_current {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            };

            spans.push(Span::styled(tab_text, style));

            // Add separator between tabs
            if i < tabs.len() - 1 {
                spans.push(Span::styled(" ", Style::default().bg(Color::Black)));
            }
        }
    }

    // Fill rest of line with background color
    let content_width: usize = spans.iter().map(|s| s.content.len()).sum();
    let remaining = (area.width as usize).saturating_sub(content_width);
    if remaining > 0 {
        spans.push(Span::styled(
            " ".repeat(remaining),
            Style::default().bg(Color::Black),
        ));
    }

    let tab_line = Line::from(spans);
    let paragraph = Paragraph::new(tab_line).style(Style::default().bg(Color::Black));
    frame.render_widget(paragraph, area);
}

/// Renders the status line
pub fn render_status_line(frame: &mut Frame, editor: &Editor, area: Rect) {
    let mode = editor.mode();
    let buffer = editor.buffer();
    let cursor = buffer.cursor();

    // Build status line content
    let mode_indicator = format!(" {} ", mode.display_name());
    let recording_indicator = if editor.is_recording_macro() {
        if let Some(reg) = editor.recording_register() {
            format!(" recording @{} ", reg)
        } else {
            " recording ".to_string()
        }
    } else {
        String::new()
    };
    let position = format!(" {}:{} ", cursor.line() + 1, cursor.col() + 1);
    let modified = if editor.is_modified() { " [+] " } else { " " };
    let file = buffer.file_path().unwrap_or("[No Name]");

    // Get diagnostic counts
    let (errors, warnings, _info, _hints) = editor.cached_diagnostic_count();
    let diagnostics = if errors > 0 || warnings > 0 {
        format!(" E:{} W:{} ", errors, warnings)
    } else {
        String::new()
    };

    // Get LSP status
    let lsp_status = if !editor.lsp_status().is_empty() {
        format!(" {} ", editor.lsp_status())
    } else if !editor.active_lsp_servers().is_empty() {
        " LSP ".to_string()
    } else {
        String::new()
    };

    // Calculate padding accounting for all elements including recording indicator
    let recording_len = if !recording_indicator.is_empty() {
        recording_indicator.len() + 1 // +1 for space after mode
    } else {
        1 // just the space after mode
    };

    let padding_len = (area.width as usize)
        .saturating_sub(mode_indicator.len())
        .saturating_sub(recording_len)
        .saturating_sub(file.len())
        .saturating_sub(modified.len())
        .saturating_sub(diagnostics.len())
        .saturating_sub(lsp_status.len())
        .saturating_sub(position.len());

    let mut spans = vec![Span::styled(
        &mode_indicator,
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];

    // Add recording indicator if recording
    if !recording_indicator.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            &recording_indicator,
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::raw(" "));
    }

    spans.push(Span::raw(file));
    spans.push(Span::raw(modified));
    spans.push(Span::raw(" ".repeat(padding_len)));

    // Add diagnostics indicator if present
    if !diagnostics.is_empty() {
        spans.push(Span::styled(
            &diagnostics,
            Style::default().fg(Color::Black).bg(if errors > 0 {
                Color::Red
            } else {
                Color::Yellow
            }),
        ));
    }

    // Add LSP status if present
    if !lsp_status.is_empty() {
        let lsp_color = if editor.lsp_status().contains("Failed")
            || editor.lsp_status().contains("Error")
        {
            Color::Red
        } else if editor.lsp_status().contains("ready") {
            Color::Green
        } else {
            Color::Blue
        };
        spans.push(Span::styled(
            &lsp_status,
            Style::default().fg(Color::Black).bg(lsp_color),
        ));
    }

    spans.push(Span::styled(
        position,
        Style::default().fg(Color::Black).bg(Color::Gray),
    ));

    let status_line = Line::from(spans);

    let paragraph = Paragraph::new(status_line).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

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

/// Renders the file tree explorer
pub fn render_file_tree(frame: &mut Frame, editor: &Editor, area: Rect) {
    if !editor.file_tree().is_visible() {
        return;
    }

    let tree = editor.file_tree();
    let flattened = tree.flattened();
    let selected_index = tree.selected_index();
    let scroll_offset = tree.scroll_offset();

    // Calculate viewport height (area height minus border rows)
    let viewport_height = area.height.saturating_sub(1) as usize; // -1 for right border title area

    // Create list items from the visible portion of the flattened tree
    let items: Vec<ListItem> = flattened
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(viewport_height)
        .map(|(idx, node)| {
            let indent = "  ".repeat(node.depth());
            let icon = if node.is_dir() {
                if node.is_expanded() {
                    "▼ "
                } else {
                    "▶ "
                }
            } else {
                "  "
            };

            let name = node.name();
            let display = format!("{}{}{}", indent, icon, name);

            let style = if idx == selected_index {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if node.is_dir() {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(display).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" Files ")
                .title_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .style(Style::default().bg(Color::Rgb(30, 34, 42)));

    frame.render_widget(list, area);
}

/// Renders the command line
pub fn render_command_line(frame: &mut Frame, editor: &Editor, area: Rect) {
    let command_text = format!(":{}", editor.command_line());

    let command_line = Line::from(vec![Span::styled(
        command_text,
        Style::default().fg(Color::White).bg(Color::Black),
    )]);

    let paragraph = Paragraph::new(command_line).style(Style::default().bg(Color::Black));
    frame.render_widget(paragraph, area);
}

/// Renders the path completion popup above the command line.
pub fn render_path_completion(frame: &mut Frame, editor: &Editor, status_area: Rect) {
    let state = editor.path_completion();
    if !state.is_visible() {
        return;
    }

    let entries = state.entries();
    let selected = state.selected_index();

    let max_visible = 10usize;
    let num_items = entries.len().min(max_visible);
    if num_items == 0 {
        return;
    }

    // Scroll window so selected item is always visible.
    let scroll_offset = if selected >= max_visible {
        selected - max_visible + 1
    } else {
        0
    };

    let menu_height = num_items as u16 + 2; // +2 for borders
    let max_name_len = entries
        .iter()
        .skip(scroll_offset)
        .take(max_visible)
        .map(|e| {
            let display_len = e.name.width();
            if e.is_dir { display_len + 1 } else { display_len }
        })
        .max()
        .unwrap_or(20);
    let menu_width = (max_name_len + 4).min(60).max(20) as u16;

    // Position above the status line, left-aligned.
    let menu_y = status_area.y.saturating_sub(menu_height);
    let menu_x = status_area.x;
    let menu_area = Rect::new(
        menu_x,
        menu_y,
        menu_width.min(status_area.width),
        menu_height,
    );

    // Build list items.
    let items: Vec<ListItem> = entries
        .iter()
        .skip(scroll_offset)
        .take(max_visible)
        .enumerate()
        .map(|(i, entry)| {
            let display = if entry.is_dir {
                format!("{}/", entry.name)
            } else {
                entry.name.clone()
            };
            let is_selected = (i + scroll_offset) == selected;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if entry.is_dir {
                Style::default().fg(Color::Blue)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(display, style)))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .style(Style::default().bg(Color::Black)),
    );

    frame.render_widget(ratatui::widgets::Clear, menu_area);
    frame.render_widget(list, menu_area);
}

/// Renders the search line
pub fn render_search_line(frame: &mut Frame, editor: &Editor, area: Rect) {
    let search_prefix = if editor.search.search_forward { "/" } else { "?" };
    let search_text = format!("{}{}", search_prefix, &editor.search.search_buffer);

    let search_line = Line::from(vec![Span::styled(
        search_text,
        Style::default().fg(Color::White).bg(Color::Black),
    )]);

    let paragraph = Paragraph::new(search_line).style(Style::default().bg(Color::Black));
    frame.render_widget(paragraph, area);
}

/// Calculates the picker overlay area (centered, takes up 80% of screen)
pub fn get_picker_area(full_area: Rect) -> Rect {
    let width = ((full_area.width * 80) / 100).max(60).min(full_area.width);
    let height = ((full_area.height * 60) / 100).max(15).min(full_area.height);
    let x = full_area.width.saturating_sub(width) / 2;
    let y = full_area.height.saturating_sub(height) / 2;

    Rect::new(x, y, width, height)
}

/// Determines if we should show the preview panel based on available width
fn should_show_preview(area: Rect) -> bool {
    // Show preview only if we have at least 100 columns total
    // This leaves ~40 cols for the list and ~60 for preview
    area.width >= 100
}

/// Renders the picker overlay
pub fn render_picker(frame: &mut Frame, editor: &mut Editor) {
    let Some(picker) = editor.picker() else {
        return;
    };

    let picker_area = get_picker_area(frame.area());
    let show_preview = should_show_preview(picker_area);

    // Clear underlying content, then fill with picker background
    frame.render_widget(ratatui::widgets::Clear, picker_area);
    frame.render_widget(Fill::bg(picker_colors::BG), picker_area);

    let mode_name = match picker.mode() {
        crate::editor::PickerMode::FindFiles => " 󰈞 Find Files ",
        crate::editor::PickerMode::LiveGrep => " 󰺮 Live Grep ",
        crate::editor::PickerMode::Custom => " 󰒉 Select ",
        crate::editor::PickerMode::Completion => "  Completion ",
        crate::editor::PickerMode::LspLocations => " 󰘧 Navigation ",
    };

    // Build right-aligned result count for the title bar
    let result_count_title = {
        let filtered = picker.filtered_results().len();
        let total = picker.all_results_count();
        format!(" {}/{} ", filtered, total)
    };

    let block = Block::default()
        .title_top(Line::from(Span::styled(
            mode_name,
            Style::default()
                .fg(picker_colors::TITLE)
                .add_modifier(Modifier::BOLD),
        )))
        .title_top(Line::from(Span::styled(
            result_count_title,
            Style::default().fg(picker_colors::TEXT_MUTED),
        )).alignment(Alignment::Right))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(picker_colors::BORDER))
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(picker_colors::BG));

    frame.render_widget(block.clone(), picker_area);

    // Split picker area into left (query + results) and right (preview)
    let inner_area = block.inner(picker_area);
    let main_chunks = if show_preview {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
            .split(inner_area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)].as_ref())
            .split(inner_area)
    };

    // Split left side into query line + separator + results
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(1), // Query line
                Constraint::Length(1), // Separator
                Constraint::Min(1),    // Results
            ]
            .as_ref(),
        )
        .split(main_chunks[0]);

    render_picker_query(frame, picker, left_chunks[0]);
    render_picker_separator(frame, left_chunks[1]);
    render_picker_results(frame, picker, left_chunks[2]);

    // Get selected result (need to clone to release immutable borrow of picker)
    let selected_result = picker.selected_result().cloned();

    // Drop immutable borrow of picker before calling functions that need mutable borrow
    let _ = picker;

    // Render preview panel if enabled
    if show_preview {
        if let Some(selected) = selected_result {
            render_picker_preview(frame, editor, &selected, main_chunks[1]);
        } else {
            // Render empty state when no selection
            render_picker_empty_state(frame, main_chunks[1]);
        }
    }
}

/// Renders the picker query line
fn render_picker_query(frame: &mut Frame, picker: &crate::editor::Picker, area: Rect) {
    let query_text = picker.query();
    let prompt_icon = " ";

    // No fake cursor character — the terminal's native BlinkingBar cursor is
    // positioned by set_cursor_position() in core.rs, so we just render the text.
    let query_line_width = area.width as usize;
    let content_len = 2 + query_text.len(); // icon + space + query text
    let padding = query_line_width.saturating_sub(content_len);

    let mut spans = vec![
        Span::styled(
            prompt_icon,
            Style::default()
                .fg(picker_colors::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default()),
        Span::styled(
            query_text.to_string(),
            Style::default()
                .fg(picker_colors::TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if padding > 0 {
        spans.push(Span::styled(
            " ".repeat(padding),
            Style::default().bg(picker_colors::BG),
        ));
    }

    let query_line = Line::from(spans);
    let query_paragraph =
        Paragraph::new(query_line).style(Style::default().bg(picker_colors::BG));
    frame.render_widget(query_paragraph, area);
}

/// Renders the picker separator line
fn render_picker_separator(frame: &mut Frame, area: Rect) {
    let separator = "─".repeat(area.width as usize);
    let separator_line = Line::from(Span::styled(
        separator,
        Style::default()
            .fg(picker_colors::SEPARATOR)
            .bg(picker_colors::BG),
    ));
    let separator_paragraph =
        Paragraph::new(separator_line).style(Style::default().bg(picker_colors::BG));
    frame.render_widget(separator_paragraph, area);
}

/// Lightweight re-match of query against (possibly truncated) display string.
/// Returns character indices of matched characters, for highlighting purposes.
fn rematch_positions(query: &str, display: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut positions = Vec::new();
    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let display_lower: Vec<char> = display.to_lowercase().chars().collect();

    // Handle space-separated tokens: each token must match independently
    let token_ranges: Vec<(usize, usize)> = {
        let mut ranges = Vec::new();
        let mut start = 0;
        for (i, &c) in query_lower.iter().enumerate() {
            if c == ' ' {
                if i > start {
                    ranges.push((start, i));
                }
                start = i + 1;
            }
        }
        if start < query_lower.len() {
            ranges.push((start, query_lower.len()));
        }
        ranges
    };

    for (token_start, token_end) in token_ranges {
        let token = &query_lower[token_start..token_end];
        let mut qi = 0;
        for (di, &dc) in display_lower.iter().enumerate() {
            if qi < token.len() && token[qi] == dc {
                positions.push(di);
                qi += 1;
            }
        }
    }

    positions.sort_unstable();
    positions.dedup();
    positions
}

/// Builds spans for a display string with matched positions highlighted.
fn build_highlighted_spans(
    display: &str,
    match_positions: &[usize],
    text_color: Color,
    bg_color: Color,
    is_selected: bool,
) -> Vec<Span<'static>> {
    use std::collections::HashSet;
    let matched: HashSet<usize> = match_positions.iter().copied().collect();
    let chars: Vec<char> = display.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let is_match = matched.contains(&i);
        let start = i;
        while i < chars.len() && matched.contains(&i) == is_match {
            i += 1;
        }
        let segment: String = chars[start..i].iter().collect();
        let style = if is_match {
            Style::default()
                .fg(picker_colors::GREEN)
                .bg(bg_color)
                .add_modifier(Modifier::UNDERLINED)
                .add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() })
        } else {
            Style::default().fg(text_color).bg(bg_color)
                .add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() })
        };
        spans.push(Span::styled(segment, style));
    }

    spans
}

/// Renders the picker results list
fn render_picker_results(frame: &mut Frame, picker: &crate::editor::Picker, area: Rect) {
    let results = picker.filtered_results();
    let selected_idx = picker.selected_index();
    let max_results = area.height as usize;
    let result_width = area.width as usize;
    let total = results.len();
    let query = picker.query();

    // Center-scroll: keep selected item in the middle of visible area
    let scroll_offset = if total <= max_results {
        0
    } else {
        let half = max_results / 2;
        if selected_idx < half {
            0
        } else if selected_idx + half >= total {
            total.saturating_sub(max_results)
        } else {
            selected_idx.saturating_sub(half)
        }
    };

    let is_live_grep = matches!(picker.mode(), crate::editor::PickerMode::LiveGrep);

    let visible_results: Vec<Line> = results
        .iter()
        .skip(scroll_offset)
        .take(max_results)
        .enumerate()
        .map(|(idx, result)| {
            let actual_idx = idx + scroll_offset;
            let is_selected = actual_idx == selected_idx;

            let max_display_len = result_width.saturating_sub(5);
            let display = crate::editor::Picker::truncate_path(&result.display, max_display_len);

            let icon = if result.line > 0 {
                "\u{f002}"
            } else if display.ends_with('/') {
                "\u{f024b}"
            } else {
                "\u{f15b}"
            };

            let (icon_color, text_color, bg_color) = if is_selected {
                (picker_colors::GREEN, picker_colors::TEXT_BRIGHT, picker_colors::SELECTED)
            } else {
                (picker_colors::TEXT_MUTED, picker_colors::TEXT, picker_colors::BG)
            };

            let icon_style = Style::default().fg(icon_color).bg(bg_color)
                .add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() });

            let prefix = if is_selected { " ▸ " } else { "   " };

            let mut spans = vec![
                Span::styled(icon.to_string(), icon_style),
                Span::styled(
                    prefix.to_string(),
                    Style::default().fg(text_color).bg(bg_color)
                        .add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() }),
                ),
            ];

            if is_live_grep {
                if let Some(content) = &result.content {
                    // Two-part display: dim location + separator + highlighted content
                    let muted_color = picker_colors::TEXT_MUTED;
                    let muted_style = Style::default().fg(muted_color).bg(bg_color)
                        .add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() });
                    spans.push(Span::styled(display.clone(), muted_style));
                    spans.push(Span::styled("  ", Style::default().bg(bg_color)));

                    // Truncate content to fit remaining width
                    let used = icon.chars().count() + prefix.chars().count() + display.chars().count() + 2;
                    let content_max = result_width.saturating_sub(used);
                    let truncated_content: String = content.chars().take(content_max).collect();

                    // Match highlights apply to the content (user searches content, not filenames)
                    let positions = rematch_positions(query, &truncated_content);
                    spans.extend(build_highlighted_spans(&truncated_content, &positions, text_color, bg_color, is_selected));

                    let total_len = used + truncated_content.chars().count();
                    let padding = result_width.saturating_sub(total_len);
                    if padding > 0 {
                        spans.push(Span::styled(" ".repeat(padding), Style::default().bg(bg_color)));
                    }
                } else {
                    // Fallback: no content field, render display as-is
                    let positions = rematch_positions(query, &display);
                    spans.extend(build_highlighted_spans(&display, &positions, text_color, bg_color, is_selected));
                    let content_len = icon.chars().count() + prefix.chars().count() + display.chars().count();
                    let padding = result_width.saturating_sub(content_len);
                    if padding > 0 {
                        spans.push(Span::styled(" ".repeat(padding), Style::default().bg(bg_color)));
                    }
                }
            } else {
                // Standard display with fuzzy match highlighting
                let positions = rematch_positions(query, &display);
                spans.extend(build_highlighted_spans(&display, &positions, text_color, bg_color, is_selected));

                let content_len = icon.chars().count() + prefix.chars().count() + display.chars().count();
                let padding = result_width.saturating_sub(content_len);
                if padding > 0 {
                    spans.push(Span::styled(" ".repeat(padding), Style::default().bg(bg_color)));
                }
            }

            Line::from(spans)
        })
        .collect();

    let mut all_lines = visible_results;

    if results.is_empty() {
        let text = "  󰍉 No matches found";
        let padding = result_width.saturating_sub(text.chars().count());
        all_lines.push(Line::from(vec![
            Span::styled(
                text,
                Style::default()
                    .fg(Color::Rgb(240, 120, 120))
                    .bg(picker_colors::BG),
            ),
            Span::styled(
                " ".repeat(padding),
                Style::default().bg(picker_colors::BG),
            ),
        ]));
    }
    // Result count is shown in the title bar — no footer needed

    // Fill remaining lines with background
    let lines_to_fill = max_results.saturating_sub(all_lines.len());
    for _ in 0..lines_to_fill {
        all_lines.push(Line::from(vec![Span::styled(
            " ".repeat(result_width),
            Style::default().bg(picker_colors::BG),
        )]));
    }

    let results_paragraph =
        Paragraph::new(all_lines).style(Style::default().bg(picker_colors::BG));
    frame.render_widget(results_paragraph, area);
}

/// Renders the file preview for the picker
fn render_picker_preview(
    frame: &mut Frame,
    editor: &mut crate::editor::Editor,
    result: &crate::editor::PickerResult,
    area: Rect,
) {
    let preview_block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(picker_colors::SEPARATOR))
        .style(Style::default().bg(picker_colors::BG_ALT));

    let inner_area = preview_block.inner(area);
    frame.render_widget(preview_block, area);

    frame.render_widget(Fill::bg(picker_colors::BG_ALT), inner_area);

    // Try to get preview with fallback - show stale preview while new one loads
    // This eliminates the jarring "Loading..." flash when navigating quickly
    let file_path = &result.location;
    let (preview, _is_stale) = match editor.get_preview_with_fallback(file_path) {
        Some((p, is_stale)) => (p, is_stale),
        None => {
            // No preview available at all (first time opening picker)
            let loading_msg = " 󰦖  Loading preview...";
            let paragraph = Paragraph::new(loading_msg)
                .style(
                    Style::default()
                        .fg(picker_colors::TEXT_MUTED)
                        .bg(picker_colors::BG_ALT)
                        .add_modifier(Modifier::ITALIC),
                )
                .alignment(Alignment::Center);

            // Center vertically
            let centered_area = Rect {
                x: inner_area.x,
                y: inner_area.y + inner_area.height / 2,
                width: inner_area.width,
                height: 1,
            };
            frame.render_widget(paragraph, centered_area);
            return;
        }
    };

    let theme = crate::syntax::Theme::default();
    let mut lines_to_render = Vec::new();

    let max_lines = inner_area.height as usize;
    let total_lines = preview.content.lines().count();

    // Calculate which lines to show
    let (start_line, end_line) = if result.line > 0 && result.line < total_lines {
        // For LiveGrep results, center around the matched line
        let context = max_lines / 2;
        let start = result.line.saturating_sub(context);
        let end = (result.line + context).min(total_lines);
        (start, end)
    } else {
        // For file finder, show from the top
        (0, max_lines.min(total_lines))
    };

    // Skip expensive syntax highlighting during rapid scrolling for responsive feel.
    // Plain text rendering is ~10x faster than syntax-highlighted rendering.
    let use_syntax = preview.language.is_some() && !editor.is_picker_scrolling_rapidly();

    if use_syntax {
        // Use syntax highlighting (only when not scrolling rapidly)
        match crate::syntax::SyntaxHighlighter::new(preview.language.unwrap()) {
            Ok(mut highlighter) => {
                render_preview_with_syntax(
                    frame,
                    &mut highlighter,
                    preview,
                    result,
                    &theme,
                    inner_area,
                    start_line,
                    end_line,
                    total_lines,
                    &mut lines_to_render,
                );
            }
            Err(_) => {
                // Fall back to plain text
                render_plain_preview(&preview.content, result, inner_area, &mut lines_to_render);
            }
        }
    } else {
        // Plain text preview (fast path for rapid scrolling or unsupported languages)
        render_plain_preview(&preview.content, result, inner_area, &mut lines_to_render);
    }

    let paragraph =
        Paragraph::new(lines_to_render).style(Style::default().bg(picker_colors::BG_ALT));
    frame.render_widget(paragraph, inner_area);
}

/// Renders preview with syntax highlighting
#[allow(clippy::too_many_arguments)]
fn render_preview_with_syntax(
    _frame: &mut Frame,
    highlighter: &mut crate::syntax::SyntaxHighlighter,
    preview: &crate::editor::PreviewCache,
    result: &crate::editor::PickerResult,
    theme: &crate::syntax::Theme,
    area: Rect,
    start_line: usize,
    end_line: usize,
    total_lines: usize,
    lines_to_render: &mut Vec<Line<'static>>,
) {
    // Parse only once if not already parsed
    let mut need_parsing = false;

    // Check if we need to parse (if any line in our range is not cached)
    {
        let cache = preview.highlighted_lines.borrow();
        for line_idx in start_line..end_line {
            if !cache.contains_key(&line_idx) {
                need_parsing = true;
                break;
            }
        }
    }

    // Parse if needed
    if need_parsing {
        highlighter.parse(&preview.content);

        // Cache highlights for the visible range
        let mut cache = preview.highlighted_lines.borrow_mut();
        for line_idx in start_line..end_line {
            cache.entry(line_idx).or_insert_with(|| {
                highlighter.highlights_for_line(line_idx, &preview.content)
            });
        }
    }

    for (line_idx, line_text) in preview.content.lines().enumerate() {
        if line_idx < start_line {
            continue;
        }
        if line_idx >= end_line {
            break;
        }

        // Expand tabs in preview content and get byte mapping
        let (line_text, tab_mapping) = expand_tabs_with_mapping(line_text, 4); // Use default tab width for previews

        // Truncate line to fit width (line number prefix is 7 chars: "  1 │ ")
        let content_width = area.width.saturating_sub(7) as usize;
        let line_text = truncate_to_width(&line_text, content_width);

        // Get highlights from cache and remap for expanded tabs
        let original_highlights = preview
            .highlighted_lines
            .borrow()
            .get(&line_idx)
            .cloned()
            .unwrap_or_default();
        let highlights = remap_highlights(&original_highlights, &tab_mapping);
        let is_target_line = result.line > 0 && result.line < total_lines && line_idx == result.line;

        // Build the line with syntax highlighting
        let mut spans = Vec::new();

        // Add line number prefix
        let line_num = format!("{:>4} │ ", line_idx + 1);
        let line_num_style = if is_target_line {
            Style::default()
                .fg(picker_colors::GREEN)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(picker_colors::TEXT_MUTED)
        };
        spans.push(Span::styled(line_num, line_num_style));

        // Add syntax-highlighted content
        let chars: Vec<char> = line_text.chars().collect();

        // Build a map from character index to byte index
        let mut byte_indices: Vec<usize> = Vec::with_capacity(chars.len() + 1);
        byte_indices.push(0);
        for (byte_idx, _) in line_text.char_indices().skip(1) {
            byte_indices.push(byte_idx);
        }
        byte_indices.push(line_text.len());

        let mut col_idx = 0;

        while col_idx < chars.len() {
            // Find the syntax group for this character (convert to byte index)
            // Uses O(log n) binary search instead of O(n) linear search
            let byte_idx = byte_indices[col_idx];
            let syntax_group = find_highlight_at_byte(&highlights, byte_idx);

            // Find the end of this styled region
            let mut end_col = col_idx + 1;
            while end_col < chars.len() {
                let next_byte_idx = byte_indices[end_col];
                let next_group = find_highlight_at_byte(&highlights, next_byte_idx);

                if next_group != syntax_group {
                    break;
                }
                end_col += 1;
            }

            let text: String = chars[col_idx..end_col].iter().collect();
            let mut style = if let Some(group) = syntax_group {
                let color = theme.get_color(group);
                Style::default().fg(color)
            } else {
                Style::default().fg(Color::White)
            };

            if is_target_line {
                style = style.bg(picker_colors::SELECTED);
            }

            spans.push(Span::styled(text, style));
            col_idx = end_col;
        }

        lines_to_render.push(Line::from(spans));
    }
}

/// Renders plain text preview without syntax highlighting
fn render_plain_preview(
    content: &str,
    result: &crate::editor::PickerResult,
    area: Rect,
    lines: &mut Vec<Line<'static>>,
) {
    let max_lines = area.height as usize;
    let total_lines = content.lines().count();

    // Calculate which lines to show
    let (start_line, end_line) = if result.line > 0 && result.line < total_lines {
        let context = max_lines / 2;
        let start = result.line.saturating_sub(context);
        let end = (result.line + context).min(total_lines);
        (start, end)
    } else {
        (0, max_lines.min(total_lines))
    };

    for (line_idx, line_text) in content.lines().enumerate() {
        if line_idx < start_line {
            continue;
        }
        if line_idx >= end_line {
            break;
        }

        // Expand tabs in plain preview
        let line_text = expand_tabs(line_text, 4);

        // Truncate line to fit width (line number prefix is 7 chars: "  1 │ ")
        let content_width = area.width.saturating_sub(7) as usize;
        let line_text = truncate_to_width(&line_text, content_width);

        let is_target_line = result.line > 0 && result.line < total_lines && line_idx == result.line;

        let line_num = format!("{:>4} │ ", line_idx + 1);
        let line_num_style = if is_target_line {
            Style::default()
                .fg(picker_colors::GREEN)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(picker_colors::TEXT_MUTED)
        };

        let text_style = if is_target_line {
            Style::default().fg(Color::White).bg(picker_colors::SELECTED)
        } else {
            Style::default().fg(picker_colors::TEXT)
        };

        lines.push(Line::from(vec![
            Span::styled(line_num, line_num_style),
            Span::styled(line_text.to_string(), text_style),
        ]));
    }
}

/// Renders empty state for the picker preview panel
fn render_picker_empty_state(frame: &mut Frame, area: Rect) {
    let preview_block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(picker_colors::SEPARATOR))
        .style(Style::default().bg(picker_colors::BG_ALT));

    let inner_area = preview_block.inner(area);
    frame.render_widget(preview_block, area);

    frame.render_widget(Fill::bg(picker_colors::BG_ALT), inner_area);

    let empty_msg = " 󰈈  No file selected";
    let paragraph = Paragraph::new(empty_msg)
        .style(
            Style::default()
                .fg(picker_colors::TEXT_MUTED)
                .bg(picker_colors::BG_ALT)
                .add_modifier(Modifier::ITALIC),
        )
        .alignment(Alignment::Center);

    // Center vertically
    let centered_area = Rect {
        x: inner_area.x,
        y: inner_area.y + inner_area.height / 2,
        width: inner_area.width,
        height: 1,
    };
    frame.render_widget(paragraph, centered_area);
}
