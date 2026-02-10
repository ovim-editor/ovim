use crate::editor::Editor;
use crate::syntax::{Theme, UiGroup};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Convert a core color to a ratatui color (convenience wrapper)
fn ui_color(theme: &Theme, group: UiGroup) -> Color {
    crate::key_convert::convert_core_color(theme.get_ui_color(group))
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
pub fn render_tab_bar(frame: &mut Frame, editor: &Editor, theme: &Theme, area: Rect) {
    let tabs = editor.tab_page_manager().tabs();
    let current_index = editor.current_tab_index();

    let tab_fill = ui_color(theme, UiGroup::TabFill);
    let tab_active_bg = ui_color(theme, UiGroup::TabActiveBg);
    let tab_active_fg = ui_color(theme, UiGroup::TabActiveFg);
    let tab_inactive_bg = ui_color(theme, UiGroup::TabInactiveBg);
    let tab_inactive_fg = ui_color(theme, UiGroup::TabInactiveFg);

    if tabs.is_empty() {
        let tab_line = Line::from(Span::styled(
            " ".repeat(area.width as usize),
            Style::default().bg(tab_fill),
        ));
        let paragraph = Paragraph::new(tab_line).style(Style::default().bg(tab_fill));
        frame.render_widget(paragraph, area);
        return;
    }

    let mut spans = Vec::new();
    let available_width = area.width as usize;

    const MIN_TAB_WIDTH: usize = 10;
    const SEPARATOR_WIDTH: usize = 1;
    const OVERFLOW_INDICATOR_WIDTH: usize = 12;

    let mut tab_widths: Vec<usize> = Vec::new();
    let mut total_width = 0;

    for (i, _tab) in tabs.iter().enumerate() {
        let title = editor.get_tab_title(i);
        let tab_text = format!(" {} {} ", i + 1, title);
        let tab_width = tab_text.len();
        tab_widths.push(tab_width);
        total_width += tab_width;
        if i < tabs.len() - 1 {
            total_width += SEPARATOR_WIDTH;
        }
    }

    let active_style = Style::default()
        .fg(tab_active_fg)
        .bg(tab_active_bg)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(tab_inactive_fg).bg(tab_inactive_bg);
    let separator_style = Style::default().bg(tab_fill);
    let overflow_style = Style::default()
        .fg(ui_color(theme, UiGroup::Warning))
        .bg(tab_inactive_bg)
        .add_modifier(Modifier::ITALIC);

    if total_width > available_width {
        let mut visible_tabs = Vec::new();

        let current_tab_width = tab_widths[current_index].max(MIN_TAB_WIDTH);
        visible_tabs.push(current_index);
        let mut used_width = current_tab_width + OVERFLOW_INDICATOR_WIDTH;

        let mut before_idx = current_index.saturating_sub(1);
        let mut after_idx = current_index + 1;
        let mut add_before = current_index > 0;
        let mut add_after = after_idx < tabs.len();

        while (add_before || add_after) && used_width < available_width {
            if add_before {
                let tab_width = tab_widths[before_idx].max(MIN_TAB_WIDTH) + SEPARATOR_WIDTH;
                if used_width + tab_width
                    <= available_width.saturating_sub(OVERFLOW_INDICATOR_WIDTH)
                {
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
                if used_width + tab_width
                    <= available_width.saturating_sub(OVERFLOW_INDICATOR_WIDTH)
                {
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

        let hidden_before = visible_tabs.first().copied().unwrap_or(0);
        if hidden_before > 0 {
            let overflow_text = format!(" +{} ", hidden_before);
            spans.push(Span::styled(overflow_text, overflow_style));
            spans.push(Span::styled(" ", separator_style));
        }

        for (idx, &tab_idx) in visible_tabs.iter().enumerate() {
            let is_current = tab_idx == current_index;
            let title = editor.get_tab_title(tab_idx);
            let tab_text = format!(" {} {} ", tab_idx + 1, title);

            let style = if is_current {
                active_style
            } else {
                inactive_style
            };

            spans.push(Span::styled(tab_text, style));

            if idx < visible_tabs.len() - 1 {
                spans.push(Span::styled(" ", separator_style));
            }
        }

        let hidden_after = tabs
            .len()
            .saturating_sub(visible_tabs.last().copied().unwrap_or(0) + 1);
        if hidden_after > 0 {
            spans.push(Span::styled(" ", separator_style));
            let overflow_text = format!(" +{} ", hidden_after);
            spans.push(Span::styled(overflow_text, overflow_style));
        }
    } else {
        for (i, _tab) in tabs.iter().enumerate() {
            let is_current = i == current_index;
            let title = editor.get_tab_title(i);
            let tab_text = format!(" {} {} ", i + 1, title);

            let style = if is_current {
                active_style
            } else {
                inactive_style
            };

            spans.push(Span::styled(tab_text, style));

            if i < tabs.len() - 1 {
                spans.push(Span::styled(" ", separator_style));
            }
        }
    }

    let content_width: usize = spans.iter().map(|s| s.content.len()).sum();
    let remaining = (area.width as usize).saturating_sub(content_width);
    if remaining > 0 {
        spans.push(Span::styled(
            " ".repeat(remaining),
            Style::default().bg(tab_fill),
        ));
    }

    let tab_line = Line::from(spans);
    let paragraph = Paragraph::new(tab_line).style(Style::default().bg(tab_fill));
    frame.render_widget(paragraph, area);
}

/// Renders the status line
pub fn render_status_line(frame: &mut Frame, editor: &Editor, theme: &Theme, area: Rect) {
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

    let status_bg = ui_color(theme, UiGroup::StatusLineBackground);
    let status_fg = ui_color(theme, UiGroup::StatusLineForeground);
    let accent_bg = ui_color(theme, UiGroup::TabActiveBg);
    let accent_fg = ui_color(theme, UiGroup::TabActiveFg);
    let error_color = ui_color(theme, UiGroup::Error);

    let mut spans = vec![Span::styled(
        &mode_indicator,
        Style::default()
            .fg(accent_fg)
            .bg(accent_bg)
            .add_modifier(Modifier::BOLD),
    )];

    // Add recording indicator if recording
    if !recording_indicator.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            &recording_indicator,
            Style::default()
                .fg(Color::White)
                .bg(error_color)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::raw(" "));
    }

    spans.push(Span::styled(file, Style::default().fg(status_fg)));
    spans.push(Span::styled(modified, Style::default().fg(status_fg)));
    spans.push(Span::raw(" ".repeat(padding_len)));

    // Add diagnostics indicator if present
    if !diagnostics.is_empty() {
        spans.push(Span::styled(
            &diagnostics,
            Style::default().fg(Color::Black).bg(if errors > 0 {
                error_color
            } else {
                ui_color(theme, UiGroup::Warning)
            }),
        ));
    }

    // Add LSP status if present
    if !lsp_status.is_empty() {
        let lsp_color =
            if editor.lsp_status().contains("Failed") || editor.lsp_status().contains("Error") {
                error_color
            } else if editor.lsp_status().contains("ready") {
                Color::Green
            } else {
                ui_color(theme, UiGroup::Info)
            };
        spans.push(Span::styled(
            &lsp_status,
            Style::default().fg(Color::Black).bg(lsp_color),
        ));
    }

    spans.push(Span::styled(
        position,
        Style::default()
            .fg(accent_fg)
            .bg(accent_bg)
            .add_modifier(Modifier::BOLD),
    ));

    let status_line = Line::from(spans);

    let paragraph = Paragraph::new(status_line).style(Style::default().bg(status_bg).fg(status_fg));
    frame.render_widget(paragraph, area);
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
            if e.is_dir {
                display_len + 1
            } else {
                display_len
            }
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

/// Renders the message line (command line area when not in command/search mode).
/// Shows LSP status messages, command feedback, or blank.
pub fn render_message_line(frame: &mut Frame, editor: &Editor, area: Rect) {
    let message = editor.lsp_status();
    let text = if message.is_empty() {
        String::new()
    } else {
        message.to_string()
    };

    let line = Line::from(vec![Span::styled(
        text,
        Style::default().fg(Color::White).bg(Color::Black),
    )]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(Color::Black));
    frame.render_widget(paragraph, area);
}

/// Renders the search line
pub fn render_search_line(frame: &mut Frame, editor: &Editor, area: Rect) {
    let search_prefix = if editor.search.search_forward {
        "/"
    } else {
        "?"
    };
    let search_text = format!("{}{}", search_prefix, &editor.search.search_buffer);

    let search_line = Line::from(vec![Span::styled(
        search_text,
        Style::default().fg(Color::White).bg(Color::Black),
    )]);

    let paragraph = Paragraph::new(search_line).style(Style::default().bg(Color::Black));
    frame.render_widget(paragraph, area);
}

/// Renders the rename input line
pub fn render_rename_input(frame: &mut Frame, editor: &Editor, area: Rect) {
    let text = format!("rename: {}", editor.rename_buffer());

    let line = Line::from(vec![Span::styled(
        text,
        Style::default().fg(Color::White).bg(Color::Black),
    )]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(Color::Black));
    frame.render_widget(paragraph, area);
}

/// Renders a diagnostic badge overlay in the top-right corner of the buffer area.
///
/// Shows error/warning counts as a colored badge (red bg for errors, yellow for warnings only).
/// Hidden when: counts are zero, badge is dismissed, or buffer is too narrow.
pub fn render_diagnostic_badge(frame: &mut Frame, editor: &Editor, buffer_area: Rect) {
    if editor.diagnostic_badge_dismissed() {
        return;
    }

    let (errors, warnings, _, _) = editor.cached_diagnostic_count();
    if errors == 0 && warnings == 0 {
        return;
    }

    // Build badge text
    let badge_text = if errors > 0 && warnings > 0 {
        format!(" E:{} W:{} ", errors, warnings)
    } else if errors > 0 {
        format!(" E:{} ", errors)
    } else {
        format!(" W:{} ", warnings)
    };

    let badge_width = badge_text.len() as u16;

    // Guard: skip if buffer area is too narrow
    if buffer_area.width < badge_width + 2 {
        return;
    }

    // Position: top-right of buffer area, 1 cell from right edge
    let badge_x = buffer_area.right().saturating_sub(badge_width + 1);
    let badge_y = buffer_area.y;

    let badge_area = Rect {
        x: badge_x,
        y: badge_y,
        width: badge_width,
        height: 1,
    };

    let bg_color = if errors > 0 {
        Color::Red
    } else {
        Color::Yellow
    };
    let fg_color = if errors > 0 {
        Color::White
    } else {
        Color::Black
    };

    let badge = Paragraph::new(Span::styled(
        badge_text,
        Style::default()
            .fg(fg_color)
            .bg(bg_color)
            .add_modifier(Modifier::BOLD),
    ));

    frame.render_widget(badge, badge_area);
}
