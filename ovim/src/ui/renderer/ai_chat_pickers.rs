use crate::editor::Editor;
use ovim_core::ai::chat_types::ChatFocus;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use super::ai_chat_style::{
    ACCENT_SELECTED, ACCENT_USER, BG_INPUT, BG_PANEL, BG_SELECTED_ROW, TEXT_DIM, TEXT_NORMAL,
};
use super::ai_chat_text::text_display_width;

pub(super) fn render_slash_completion(
    frame: &mut Frame,
    editor: &mut Editor,
    input_area: Rect,
    minimum_y: u16,
) {
    let completions = editor.ai_chat_slash_completions();
    if completions.is_empty() || input_area.width < 12 {
        return;
    }
    let available_height = input_area.y.saturating_sub(minimum_y);
    if available_height < 3 {
        return;
    }

    let selected = editor.ai_chat_slash_completion_selected();
    let visible_count = completions
        .len()
        .min(6)
        .min(available_height.saturating_sub(2) as usize);
    if visible_count == 0 {
        return;
    }
    let scroll_offset = if selected >= visible_count {
        selected - visible_count + 1
    } else {
        0
    };
    let popup = Rect {
        x: input_area.x.saturating_add(1),
        y: input_area.y.saturating_sub(visible_count as u16 + 2),
        width: input_area.width.saturating_sub(2),
        height: visible_count as u16 + 2,
    };
    let items = completions
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_count)
        .map(|(index, completion)| {
            let is_selected = index == selected;
            let background = if is_selected {
                BG_SELECTED_ROW
            } else {
                BG_INPUT
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    if is_selected { "› " } else { "  " },
                    Style::default().fg(ACCENT_SELECTED).bg(background),
                ),
                Span::styled(
                    completion.usage,
                    Style::default()
                        .fg(if is_selected {
                            ACCENT_SELECTED
                        } else {
                            ACCENT_USER
                        })
                        .bg(background)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", completion.description),
                    Style::default().fg(TEXT_DIM).bg(background),
                ),
            ]))
            .style(Style::default().bg(background))
        })
        .collect::<Vec<_>>();

    frame.render_widget(Clear, popup);
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Commands ")
                .border_style(Style::default().fg(ACCENT_USER))
                .style(Style::default().bg(BG_INPUT)),
        ),
        popup,
    );

    editor.render_cache.ai_chat_interactions.slash_completions = (0..visible_count)
        .map(|row| {
            (
                ovim_core::Rect {
                    x: popup.x.saturating_add(1),
                    y: popup.y.saturating_add(1 + row as u16),
                    width: popup.width.saturating_sub(2),
                    height: 1,
                },
                scroll_offset + row,
            )
        })
        .collect();
}

// ---------------------------------------------------------------------------
// Model Selector Bar
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn render_model_selector_bar(frame: &mut Frame, editor: &Editor, area: Rect) {
    if area.height == 0 || area.width < 10 {
        return;
    }

    let focus = editor.ai_chat_focus();
    let is_focused = focus == ChatFocus::ModelSelector;
    let w = area.width as usize;
    let pending_no_repo_approval = editor.ai_chat_has_pending_no_repo_folder_approval();
    let pending_tool_approval = editor.ai_chat_has_pending_tool_approval();

    let mut profile_names = editor.ai_profile_names_sorted();
    if profile_names.is_empty() {
        profile_names.push(editor.ai_chat_effective_profile());
    }
    let active_profile = editor.ai_chat_effective_profile();

    let mut spans: Vec<Span> = Vec::new();
    let mut used_width = 0usize;

    // Arrow indicator
    let arrow = if is_focused { "▸ " } else { "  " };
    spans.push(Span::styled(
        arrow,
        Style::default()
            .fg(if is_focused { Color::Yellow } else { TEXT_DIM })
            .bg(BG_PANEL),
    ));
    used_width += 2;

    if pending_no_repo_approval || pending_tool_approval {
        let label = if pending_no_repo_approval {
            " ! folder access pending "
        } else {
            " ! tool approval pending "
        };
        let label_w = text_display_width(label);
        if used_width + label_w + 1 < w {
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(Color::Rgb(30, 30, 30))
                    .bg(Color::Rgb(240, 180, 50))
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                " ",
                Style::default().fg(TEXT_DIM).bg(BG_PANEL),
            ));
            used_width += label_w + 1;
        }
    }

    for name in &profile_names {
        let Some(profile) = editor.ai_state.config.resolve_profile(name) else {
            continue;
        };
        let model_short: String = profile.model.chars().take(20).collect();
        let label = format!(" {}:{} ", profile.display_name(), model_short);
        let label_w = text_display_width(&label);
        if used_width + label_w + 1 > w {
            break;
        }

        let is_active = name == &active_profile;
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
        spans.push(Span::styled(label, style));
        used_width += label_w;

        // Separator
        if used_width + 3 < w {
            spans.push(Span::styled(
                " │ ",
                Style::default().fg(TEXT_DIM).bg(BG_PANEL),
            ));
            used_width += 3;
        }
    }

    // Hints at right
    let allow_edits = editor.ai_chat_allow_edits();
    let hint = if pending_no_repo_approval {
        " [Enter allow] [Esc deny] "
    } else if pending_tool_approval && editor.ai_chat_uses_external_agent() {
        " [Enter allow once] [Esc deny] "
    } else if editor.ai_chat_has_external_question() {
        " [Enter answer] [Esc cancel] "
    } else if pending_tool_approval {
        " [Enter allow] [C-a allow chat] [Esc deny] "
    } else if allow_edits {
        " [Enter send] [PgUp/PgDn scroll code] [C-y copy] [Esc\u{00d7}2 close] "
    } else {
        " [?] [Enter send] [PgUp/PgDn scroll code] [C-y copy] [Esc\u{00d7}2 close] "
    };
    let hint_w = text_display_width(hint);
    if used_width + hint_w < w {
        let gap = w.saturating_sub(used_width + hint_w);
        spans.push(Span::styled(" ".repeat(gap), Style::default().bg(BG_PANEL)));
        spans.push(Span::styled(
            hint,
            Style::default().fg(TEXT_DIM).bg(BG_PANEL),
        ));
    } else {
        let remaining = w.saturating_sub(used_width);
        spans.push(Span::styled(
            " ".repeat(remaining),
            Style::default().bg(BG_PANEL),
        ));
    }

    frame.render_widget(Paragraph::new(vec![Line::from(spans)]), area);
}

pub(super) fn render_model_picker(
    frame: &mut Frame,
    editor: &mut Editor,
    anchor: Option<Rect>,
    area: Rect,
) {
    if area.height < 5 || area.width < 24 {
        return;
    }
    let model_options = editor.ai_chat_model_options();
    let active_model = editor.ai_chat_selected_model().to_string();
    let active_profile = editor.ai_chat_effective_profile();
    let active_effort = editor.ai_chat_reasoning_effort_selection();
    let permission_modes = editor.ai_chat_pickable_permission_modes();
    let active_permission = editor.ai_chat_permission_mode().map(str::to_owned);
    let section = editor.ai_chat_model_picker_section();
    let content_rows = model_options.len()
        + editor.ai_chat_reasoning_efforts().len()
        + permission_modes.len()
        + 2
        + usize::from(!permission_modes.is_empty());
    let height = (content_rows as u16 + 2).min(area.height);
    let width = area.width.clamp(24, 52);
    let anchor = anchor.unwrap_or(Rect::new(area.x, area.y.saturating_sub(1), width, 1));
    let popup = Rect {
        x: anchor.x.min(area.right().saturating_sub(width)).max(area.x),
        y: anchor
            .bottom()
            .min(area.bottom().saturating_sub(height))
            .max(area.y),
        width,
        height,
    };
    let mut items = vec![ListItem::new(" MODEL PROFILES").style(
        Style::default()
            .fg(
                if section == ovim_core::editor::ChatModelPickerSection::Model {
                    ACCENT_SELECTED
                } else {
                    TEXT_DIM
                },
            )
            .add_modifier(Modifier::BOLD),
    )];
    let mut model_rows = Vec::new();
    for option in &model_options {
        let selected = option.id == active_profile && option.model == active_model;
        let marker = if selected { "●" } else { "○" };
        let detail = format!("{marker} {}  {}", option.label, option.model);
        items.push(ListItem::new(detail).style(if selected {
            Style::default().fg(Color::White).bg(BG_SELECTED_ROW)
        } else {
            Style::default().fg(TEXT_NORMAL)
        }));
        model_rows.push(option.clone());
    }
    items.push(
        ListItem::new(" REASONING EFFORT").style(
            Style::default()
                .fg(
                    if section == ovim_core::editor::ChatModelPickerSection::Effort {
                        ACCENT_SELECTED
                    } else {
                        TEXT_DIM
                    },
                )
                .add_modifier(Modifier::BOLD),
        ),
    );
    for effort in editor.ai_chat_reasoning_efforts() {
        let selected = *effort == active_effort;
        let marker = if selected { "●" } else { "○" };
        let detail = if *effort == "default" {
            format!(
                "{marker} default  {}",
                editor.ai_chat_default_reasoning_effort()
            )
        } else {
            format!("{marker} {effort}")
        };
        items.push(ListItem::new(detail).style(if selected {
            Style::default().fg(Color::White).bg(BG_SELECTED_ROW)
        } else {
            Style::default().fg(TEXT_NORMAL)
        }));
    }
    if !permission_modes.is_empty() {
        items.push(
            ListItem::new(" PERMISSIONS").style(
                Style::default()
                    .fg(
                        if section == ovim_core::editor::ChatModelPickerSection::Permission {
                            ACCENT_SELECTED
                        } else {
                            TEXT_DIM
                        },
                    )
                    .add_modifier(Modifier::BOLD),
            ),
        );
        for option in &permission_modes {
            let selected = Some(option.id) == active_permission.as_deref();
            let marker = if selected { "●" } else { "○" };
            items.push(
                ListItem::new(format!("{marker} {}", option.label)).style(if selected {
                    Style::default().fg(Color::White).bg(BG_SELECTED_ROW)
                } else {
                    Style::default().fg(TEXT_NORMAL)
                }),
            );
        }
    }
    let selected_row = match section {
        ovim_core::editor::ChatModelPickerSection::Model => {
            1 + model_options
                .iter()
                .position(|option| option.id == active_profile && option.model == active_model)
                .unwrap_or(0)
        }
        ovim_core::editor::ChatModelPickerSection::Effort => {
            2 + model_options.len()
                + editor
                    .ai_chat_reasoning_efforts()
                    .iter()
                    .position(|effort| *effort == active_effort)
                    .unwrap_or(0)
        }
        ovim_core::editor::ChatModelPickerSection::Permission => {
            3 + model_options.len()
                + editor.ai_chat_reasoning_efforts().len()
                + permission_modes
                    .iter()
                    .position(|option| Some(option.id) == active_permission.as_deref())
                    .unwrap_or(0)
        }
    };
    let mut state = ratatui::widgets::ListState::default().with_selected(Some(selected_row));
    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(
        List::new(items).block(
            Block::default()
                .title(" Run settings · Tab section · ↑/↓ choose ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(82, 139, 255))),
        ),
        popup,
        &mut state,
    );

    let first_row = popup.y.saturating_add(1);
    let offset = state.offset();
    let visible_bottom = popup.bottom().saturating_sub(1);
    editor
        .render_cache
        .ai_chat_interactions
        .model_picker_options = model_rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, name)| {
            let row = (index + 1).checked_sub(offset)?;
            let y = first_row + row as u16;
            (y < visible_bottom).then(|| {
                (
                    crate::key_convert::convert_ratatui_rect(Rect::new(
                        popup.x + 1,
                        y,
                        popup.width.saturating_sub(2),
                        1,
                    )),
                    name,
                )
            })
        })
        .collect();
    editor
        .render_cache
        .ai_chat_interactions
        .effort_picker_options = editor
        .ai_chat_reasoning_efforts()
        .iter()
        .enumerate()
        .filter_map(|(index, effort)| {
            let row = (model_options.len() + 2 + index).checked_sub(offset)?;
            let y = first_row + row as u16;
            (y < visible_bottom).then(|| {
                (
                    crate::key_convert::convert_ratatui_rect(Rect::new(
                        popup.x + 1,
                        y,
                        popup.width.saturating_sub(2),
                        1,
                    )),
                    (*effort).to_string(),
                )
            })
        })
        .collect();
    editor
        .render_cache
        .ai_chat_interactions
        .permission_picker_options = permission_modes
        .iter()
        .enumerate()
        .filter_map(|(index, option)| {
            let row = (model_options.len() + editor.ai_chat_reasoning_efforts().len() + 3 + index)
                .checked_sub(offset)?;
            let y = first_row + row as u16;
            (y < visible_bottom).then(|| {
                (
                    crate::key_convert::convert_ratatui_rect(Rect::new(
                        popup.x + 1,
                        y,
                        popup.width.saturating_sub(2),
                        1,
                    )),
                    option.id.to_string(),
                )
            })
        })
        .collect();
}
