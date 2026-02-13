use crate::editor::{Editor, ToastLevel};
use crate::syntax::{Theme, UiGroup};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    let branch_display = editor
        .git_branch()
        .map(|b| format!(" {}", b))
        .unwrap_or_default();

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
    if !branch_display.is_empty() {
        spans.push(Span::styled(
            &branch_display,
            Style::default().fg(status_fg).add_modifier(Modifier::DIM),
        ));
    }

    // Right-side widgets differ for AI chat mode
    let is_ai_chat = mode == crate::mode::Mode::AiChat;
    let mut right_spans: Vec<Span> = Vec::new();

    if is_ai_chat {
        // AI chat right-side: profile:model, tool iterations, streaming status, position
        let active_profile = &editor.ai_state.active_profile;
        let model_display = editor
            .ai_state
            .config
            .resolve_profile(active_profile)
            .map(|p| {
                let short: String = p.model.chars().take(16).collect();
                format!(" {}:{} ", active_profile, short)
            })
            .unwrap_or_else(|| format!(" {} ", active_profile));
        right_spans.push(Span::styled(
            model_display,
            Style::default()
                .fg(Color::Rgb(180, 188, 202))
                .bg(Color::Rgb(46, 52, 64)),
        ));

        if let Some(chat) = editor.ai_state.chat.as_ref() {
            if chat.tool_iterations > 0 {
                let max_iter = chat
                    .opts
                    .profile
                    .as_ref()
                    .and_then(|p| editor.ai_state.config.resolve_profile(p))
                    .map(|p| p.context_policy.max_iterations)
                    .unwrap_or(4);
                let iter_text = format!(" \u{26A1}{}/{} ", chat.tool_iterations, max_iter);
                right_spans.push(Span::styled(
                    iter_text,
                    Style::default().fg(Color::Yellow).bg(status_bg),
                ));
            }

            if chat.waiting {
                let status_text = if chat.streaming_content.is_some() {
                    " streaming... "
                } else if chat.streaming_thinking.is_some() {
                    " thinking... "
                } else {
                    " waiting... "
                };
                right_spans.push(Span::styled(
                    status_text,
                    Style::default()
                        .fg(Color::Rgb(120, 180, 255))
                        .bg(status_bg)
                        .add_modifier(Modifier::ITALIC),
                ));
            }
        }

        right_spans.push(Span::styled(
            &position,
            Style::default()
                .fg(accent_fg)
                .bg(accent_bg)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        // Normal right-side: diagnostics, LSP, position
        let (errors, warnings, _info, _hints) = editor.cached_diagnostic_count();
        let diagnostics = if errors > 0 || warnings > 0 {
            format!(" E:{} W:{} ", errors, warnings)
        } else {
            String::new()
        };

        let lsp_status = if !editor.lsp_status().is_empty() {
            format!(" {} ", editor.lsp_status())
        } else if !editor.active_lsp_servers().is_empty() {
            " LSP ".to_string()
        } else {
            String::new()
        };

        if !diagnostics.is_empty() {
            right_spans.push(Span::styled(
                diagnostics,
                Style::default().fg(Color::Black).bg(if errors > 0 {
                    error_color
                } else {
                    ui_color(theme, UiGroup::Warning)
                }),
            ));
        }

        if !lsp_status.is_empty() {
            let lsp_color = if editor.lsp_status().contains("Failed")
                || editor.lsp_status().contains("Error")
            {
                error_color
            } else if editor.lsp_status().contains("ready") {
                Color::Green
            } else {
                ui_color(theme, UiGroup::Info)
            };
            right_spans.push(Span::styled(
                lsp_status,
                Style::default().fg(Color::Black).bg(lsp_color),
            ));
        }

        right_spans.push(Span::styled(
            &position,
            Style::default()
                .fg(accent_fg)
                .bg(accent_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Calculate padding
    let recording_len = if !recording_indicator.is_empty() {
        recording_indicator.len() + 1
    } else {
        1
    };
    let left_used = mode_indicator.len()
        + recording_len
        + file.len()
        + modified.len()
        + branch_display.len();
    let right_used: usize = right_spans.iter().map(|s| s.content.len()).sum();
    let padding_len = (area.width as usize).saturating_sub(left_used + right_used);

    spans.push(Span::raw(" ".repeat(padding_len)));
    spans.extend(right_spans);

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

fn truncate_to_width(input: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    if input.width() <= max_width {
        return input.to_string();
    }

    if max_width == 1 {
        return "~".to_string();
    }

    let mut out = String::new();
    let mut used = 0usize;

    for ch in input.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w >= max_width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('~');
    out
}

fn toast_colors(theme: &Theme, level: ToastLevel) -> (Color, Color) {
    match level {
        ToastLevel::Error => (Color::White, ui_color(theme, UiGroup::Error)),
        ToastLevel::Warning => (Color::Black, ui_color(theme, UiGroup::Warning)),
        ToastLevel::Success => (Color::Black, Color::Green),
        ToastLevel::Info => (Color::Black, ui_color(theme, UiGroup::Info)),
    }
}

/// Renders a top-right toast stack over the buffer area.
///
/// Slot 0 is reserved for persistent diagnostics (when present), and additional rows
/// show transient toasts from the editor's toast center.
pub fn render_top_right_toasts(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    buffer_area: Rect,
) {
    let mut rows: Vec<(String, ToastLevel)> = Vec::new();

    if !editor.diagnostic_badge_dismissed() {
        let (errors, warnings, _, _) = editor.cached_diagnostic_count();
        if errors > 0 || warnings > 0 {
            let text = if errors > 0 && warnings > 0 {
                format!(" E:{} W:{} ", errors, warnings)
            } else if errors > 0 {
                format!(" E:{} ", errors)
            } else {
                format!(" W:{} ", warnings)
            };
            rows.push((
                text,
                if errors > 0 {
                    ToastLevel::Error
                } else {
                    ToastLevel::Warning
                },
            ));
        }
    }

    for toast in editor.visible_toasts_newest_first(4) {
        let mut text = format!(" [{}] ", toast.source.label());
        if let Some(title) = &toast.title {
            text.push_str(title);
            text.push_str(": ");
        }
        text.push_str(&toast.message);
        if toast.repeat > 1 {
            text.push_str(&format!(" x{}", toast.repeat));
        }
        text.push(' ');
        rows.push((text, toast.level));
    }

    if rows.is_empty() {
        return;
    }

    let max_rows = buffer_area.height.min(5) as usize;
    for (index, (raw_text, level)) in rows.into_iter().take(max_rows).enumerate() {
        let y = buffer_area.y.saturating_add(index as u16);
        if y >= buffer_area.bottom() {
            break;
        }

        let available = buffer_area.width.saturating_sub(2) as usize;
        if available < 4 {
            continue;
        }

        let text = truncate_to_width(&raw_text, available);
        let width = text.width() as u16;
        if width == 0 {
            continue;
        }

        let x = buffer_area.right().saturating_sub(width + 1);
        let area = Rect {
            x,
            y,
            width,
            height: 1,
        };

        let (fg, bg) = toast_colors(theme, level);
        let badge = Paragraph::new(Span::styled(
            text,
            Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
        ));
        frame.render_widget(badge, area);
    }
}

/// Renders contextual widgets in the left and right margins when textwidth centering
/// creates extra space. Left margin shows git info, right margin shows diagnostics/LSP.
pub fn render_margin_widgets(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    full_area: Rect,
    buffer_area: Rect,
) {
    let dim_style = Style::default()
        .fg(ui_color(theme, UiGroup::StatusLineForeground))
        .add_modifier(Modifier::DIM);

    // ── Left margin: git branch + change summary ──
    let left_margin_width = buffer_area.x.saturating_sub(full_area.x) as usize;
    if left_margin_width >= 12 {
        let mut parts: Vec<Span> = Vec::new();

        if let Some(branch) = editor.git_branch() {
            // Truncate branch name to fit margin (leave room for change stats)
            let max_branch = left_margin_width.saturating_sub(3); // 1 gap + some padding
            let display = if branch.len() > max_branch {
                format!("{}~", &branch[..max_branch.saturating_sub(1)])
            } else {
                branch.to_string()
            };
            parts.push(Span::styled(format!(" {}", display), dim_style));
        }

        // Change summary from git status
        let (added, modified, removed) = editor.buffer().git_status().change_counts();
        if added > 0 || modified > 0 || removed > 0 {
            let mut summary = String::new();
            if added > 0 {
                summary.push_str(&format!(" +{}", added));
            }
            if modified > 0 {
                summary.push_str(&format!(" ~{}", modified));
            }
            if removed > 0 {
                summary.push_str(&format!(" -{}", removed));
            }
            parts.push(Span::styled(summary, dim_style));
        }

        if !parts.is_empty() {
            // Right-align within the left margin, 1 col gap before buffer
            let content_width: usize = parts.iter().map(|s| s.width()).sum();
            let padding = left_margin_width.saturating_sub(content_width + 1);

            let mut spans = vec![Span::raw(" ".repeat(padding))];
            spans.extend(parts);

            let line = Line::from(spans);
            let area = Rect {
                x: full_area.x,
                y: full_area.y,
                width: left_margin_width as u16,
                height: 1,
            };
            frame.render_widget(Paragraph::new(line), area);
        }
    }

    // ── Right margin: diagnostic counts + LSP status ──
    let right_margin_start = buffer_area.x + buffer_area.width;
    let right_margin_width =
        (full_area.x + full_area.width).saturating_sub(right_margin_start) as usize;
    if right_margin_width >= 12 {
        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::raw(" ")); // 1 col gap after buffer

        let (errors, warnings, _, _) = editor.cached_diagnostic_count();
        if errors > 0 {
            spans.push(Span::styled(
                format!("E:{}", errors),
                Style::default().fg(ui_color(theme, UiGroup::Error)),
            ));
            spans.push(Span::raw(" "));
        }
        if warnings > 0 {
            spans.push(Span::styled(
                format!("W:{}", warnings),
                Style::default().fg(ui_color(theme, UiGroup::Warning)),
            ));
            spans.push(Span::raw(" "));
        }

        // LSP status badge
        if !editor.active_lsp_servers().is_empty() {
            let status_text = if !editor.lsp_status().is_empty() {
                editor.lsp_status().to_string()
            } else {
                "LSP ready".to_string()
            };
            let lsp_color = if status_text.contains("Failed") || status_text.contains("Error") {
                ui_color(theme, UiGroup::Error)
            } else if status_text.contains("ready") {
                Color::Green
            } else {
                ui_color(theme, UiGroup::Info)
            };
            // Truncate if too long for margin
            let max_len = right_margin_width
                .saturating_sub(spans.iter().map(|s| s.width()).sum::<usize>() + 1);
            let display = if status_text.len() > max_len {
                format!("{}~", &status_text[..max_len.saturating_sub(1)])
            } else {
                status_text
            };
            spans.push(Span::styled(display, Style::default().fg(lsp_color)));
        }

        let line = Line::from(spans);
        let area = Rect {
            x: right_margin_start,
            y: full_area.y,
            width: right_margin_width as u16,
            height: 1,
        };
        frame.render_widget(Paragraph::new(line), area);
    }
}

#[derive(Default)]
pub struct AiPromptRenderLayout {
    pub input_area: Option<ovim_core::Rect>,
    pub input_rows: Vec<(ovim_core::Rect, usize, usize)>,
    pub model_hitboxes: Vec<(ovim_core::Rect, String)>,
}

const AI_PROMPT_PREFIX: &str = " prompt > ";
const AI_PROMPT_BORDER_ROWS: u16 = 2;
const AI_PROMPT_STATIC_ROWS: u16 = 2;
const AI_PROMPT_MIN_HEIGHT: u16 = 5;
const AI_PROMPT_MAX_HEIGHT: u16 = 12;

fn wrap_prompt_rows(
    prompt: &str,
    first_row_width: usize,
    continuation_row_width: usize,
    max_rows: usize,
) -> Vec<(usize, usize)> {
    if max_rows == 0 {
        return Vec::new();
    }

    let mut rows = Vec::new();
    let mut row_start = 0usize;
    let first_limit = first_row_width.max(1);
    let continuation_limit = continuation_row_width.max(1);
    let mut row_limit = first_limit;

    while row_start < prompt.len() && rows.len() < max_rows {
        let mut row_end = row_start;
        let mut row_display = 0usize;
        for (rel_idx, ch) in prompt[row_start..].char_indices() {
            let byte_idx = row_start + rel_idx;
            let ch_width = crate::display::char_display_width(ch);

            if row_end > row_start && row_display + ch_width > row_limit {
                break;
            }

            row_display += ch_width;
            row_end = byte_idx + ch.len_utf8();
        }

        if row_end == row_start {
            if let Some(ch) = prompt[row_start..].chars().next() {
                row_end = row_start + ch.len_utf8();
            } else {
                break;
            }
        }

        rows.push((row_start, row_end));
        row_start = row_end;
        row_limit = continuation_limit;
    }

    if rows.is_empty() {
        rows.push((0, 0));
    }

    rows
}

pub fn ai_prompt_panel_height(editor: &Editor, panel_width: u16, max_height: u16) -> u16 {
    let available = max_height.max(1);
    let min_height = AI_PROMPT_MIN_HEIGHT.min(available);
    let max_height = AI_PROMPT_MAX_HEIGHT.min(available);

    let inner_width = panel_width.saturating_sub(2) as usize;
    let prefix_width = AI_PROMPT_PREFIX.width().min(inner_width.saturating_sub(1));
    let first_row_width = inner_width.saturating_sub(prefix_width);
    let continuation_row_width = inner_width;

    let reserved_rows = AI_PROMPT_BORDER_ROWS + AI_PROMPT_STATIC_ROWS;
    let max_input_rows = available.saturating_sub(reserved_rows).max(1) as usize;
    let needed_rows = wrap_prompt_rows(
        editor.ai_prompt_input(),
        first_row_width,
        continuation_row_width,
        usize::MAX,
    )
    .len();
    let visible_rows = needed_rows.min(max_input_rows).max(1);

    (reserved_rows + visible_rows as u16).clamp(min_height, max_height)
}

/// Renders the expanded AI prompt panel and returns hit-test layout data.
pub fn render_ai_prompt_line(
    frame: &mut Frame,
    editor: &Editor,
    area: Rect,
) -> AiPromptRenderLayout {
    let mut layout = AiPromptRenderLayout::default();
    if area.width == 0 || area.height == 0 {
        return layout;
    }

    let panel_bg = Color::Rgb(20, 23, 30);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(82, 90, 105)))
        .style(Style::default().bg(panel_bg));
    frame.render_widget(block.clone(), area);

    let inner = block.inner(area);
    if inner.width == 0 || inner.height == 0 {
        return layout;
    }

    let rows = if inner.height >= 3 {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![Constraint::Min(1); inner.height as usize])
            .split(inner)
    };

    let mut profile_names = editor.ai_profile_names_sorted();
    if profile_names.is_empty() {
        profile_names.push(editor.ai_state.active_profile.clone());
    }

    if let Some(row) = rows.first() {
        let header = format!(
            " AI Edit  profile: {}  extraction: {}  • Enter submit • Esc cancel • Tab/Shift-Tab switch model",
            editor.ai_state.active_profile, editor.ai_state.extraction
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                header,
                Style::default().fg(Color::Rgb(176, 184, 196)).bg(panel_bg),
            ))),
            *row,
        );
    }

    if rows.len() >= 2 {
        let row = rows[1];
        let mut spans = vec![Span::styled(
            " models ",
            Style::default().fg(Color::Rgb(128, 140, 155)).bg(panel_bg),
        )];
        let mut cursor_x = " models ".width();

        for name in profile_names {
            let Some(profile) = editor.ai_state.config.resolve_profile(&name) else {
                continue;
            };
            let model_short: String = profile.model.chars().take(24).collect();
            let label = format!(" {}:{} ", name, model_short);
            let label_w = label.width();
            if cursor_x + label_w > row.width as usize {
                break;
            }

            let is_active = name == editor.ai_state.active_profile;
            let style = if is_active {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(66, 86, 112))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Rgb(180, 188, 202))
                    .bg(Color::Rgb(46, 52, 64))
            };
            spans.push(Span::styled(label.clone(), style));

            layout.model_hitboxes.push((
                ovim_core::Rect {
                    x: row.x + cursor_x as u16,
                    y: row.y,
                    width: label_w as u16,
                    height: 1,
                },
                name.clone(),
            ));
            cursor_x += label_w;
        }

        if cursor_x < row.width as usize {
            spans.push(Span::raw(" ".repeat(row.width as usize - cursor_x)));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), row);
    }

    let input_rows_area = *rows.last().unwrap_or(&inner);
    let prompt = editor.ai_prompt_input();
    let prefix_width = AI_PROMPT_PREFIX
        .width()
        .min(input_rows_area.width.saturating_sub(1) as usize);
    let first_row_width = input_rows_area.width.saturating_sub(prefix_width as u16) as usize;
    let continuation_row_width = input_rows_area.width as usize;
    let wrapped_rows = wrap_prompt_rows(
        prompt,
        first_row_width,
        continuation_row_width,
        input_rows_area.height.max(1) as usize,
    );

    for (idx, (start_byte, end_byte)) in wrapped_rows.iter().enumerate() {
        let row = Rect {
            x: input_rows_area.x,
            y: input_rows_area.y + idx as u16,
            width: input_rows_area.width,
            height: 1,
        };
        let input_bg = Color::Rgb(28, 33, 42);
        let mut spans: Vec<Span> = Vec::new();

        let input_rect = if idx == 0 {
            let prefix_text = &AI_PROMPT_PREFIX[..prefix_width];
            spans.push(Span::styled(
                prefix_text.to_string(),
                Style::default().fg(Color::Rgb(144, 160, 180)).bg(input_bg),
            ));
            ovim_core::Rect {
                x: row.x + prefix_width as u16,
                y: row.y,
                width: row.width.saturating_sub(prefix_width as u16),
                height: 1,
            }
        } else {
            ovim_core::Rect {
                x: row.x,
                y: row.y,
                width: row.width,
                height: 1,
            }
        };

        let row_text = &prompt[*start_byte..*end_byte];
        spans.push(Span::styled(
            row_text.to_string(),
            Style::default().fg(Color::White).bg(input_bg),
        ));

        let used_width = if idx == 0 {
            prefix_width + row_text.width()
        } else {
            row_text.width()
        };
        if used_width < row.width as usize {
            spans.push(Span::styled(
                " ".repeat(row.width as usize - used_width),
                Style::default().bg(input_bg),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row);

        if input_rect.width > 0 {
            layout.input_rows.push((input_rect, *start_byte, *end_byte));
            if layout.input_area.is_none() {
                layout.input_area = Some(input_rect);
            }
        }
    }

    layout
}

#[cfg(test)]
mod tests {
    use super::wrap_prompt_rows;

    #[test]
    fn test_wrap_prompt_rows_preserves_text_across_rows() {
        let prompt = "abcdefghij";
        let rows = wrap_prompt_rows(prompt, 3, 4, 8);

        assert_eq!(rows, vec![(0, 3), (3, 7), (7, 10)]);

        let rebuilt = rows
            .iter()
            .map(|(start, end)| &prompt[*start..*end])
            .collect::<String>();
        assert_eq!(rebuilt, prompt);
    }

    #[test]
    fn test_wrap_prompt_rows_handles_empty_input() {
        let rows = wrap_prompt_rows("", 3, 4, 8);
        assert_eq!(rows, vec![(0, 0)]);
    }
}
