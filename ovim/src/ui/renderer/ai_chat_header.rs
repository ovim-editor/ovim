use crate::editor::Editor;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::ai_chat_style::TEXT_DIM;
use super::ai_chat_text::text_display_width;

pub(super) fn render_chat_header(
    frame: &mut Frame,
    editor: &mut Editor,
    area: Rect,
) -> Option<Rect> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let yolo_enabled = editor.ai_chat_yolo_mode();
    let external_agent = editor.ai_chat_uses_external_agent();
    // Claude's bypass mode has no toggle to show, so it takes the slot: the
    // warning stays visible at any panel width.
    let skips_approvals = external_agent && editor.ai_chat_permission_mode_skips_approvals();
    let yolo_label = if skips_approvals {
        if area.width >= 60 {
            " BYPASS PERMISSIONS "
        } else {
            " BYPASS "
        }
    } else if external_agent {
        ""
    } else if yolo_enabled {
        " YOLO ON "
    } else {
        " YOLO OFF "
    };
    let yolo_width = text_display_width(yolo_label).min(area.width as usize) as u16;
    let yolo_x = area.right().saturating_sub(yolo_width);
    let yolo_style = if skips_approvals {
        Style::default()
            .fg(Color::White)
            .bg(Color::Rgb(176, 32, 32))
            .add_modifier(Modifier::BOLD)
    } else if yolo_enabled {
        Style::default()
            .fg(Color::Rgb(255, 220, 120))
            .bg(Color::Rgb(100, 48, 28))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(TEXT_DIM)
            .bg(Color::Rgb(35, 40, 50))
            .add_modifier(Modifier::DIM)
    };

    let comprehension = editor.ai_chat_comprehension_policy();
    let comprehension_label = if external_agent {
        ""
    } else {
        match comprehension {
            ovim_core::editor::ComprehensionPolicy::Off => " COMPREHENSION OFF ",
            ovim_core::editor::ComprehensionPolicy::Publish => " COMPREHENSION: PUBLISH ",
            ovim_core::editor::ComprehensionPolicy::Commit => " COMPREHENSION: COMMIT ",
        }
    };
    let available = area.width.saturating_sub(yolo_width);
    let comprehension_width =
        text_display_width(comprehension_label).min(available as usize) as u16;
    let comprehension_x = yolo_x.saturating_sub(comprehension_width);
    let comprehension_style = if comprehension == ovim_core::editor::ComprehensionPolicy::Off {
        Style::default()
            .fg(TEXT_DIM)
            .bg(Color::Rgb(35, 40, 50))
            .add_modifier(Modifier::DIM)
    } else {
        Style::default()
            .fg(Color::Rgb(190, 230, 255))
            .bg(Color::Rgb(35, 70, 92))
            .add_modifier(Modifier::BOLD)
    };
    // A session-wide shell grant (`sensitive_prompt` only) stays visible next
    // to the policy toggles for as long as it holds.
    let shell_label = if !external_agent && editor.ai_chat_shell_allowed_session() {
        " SHELL ALLOWED "
    } else {
        ""
    };
    let shell_width =
        text_display_width(shell_label).min(comprehension_x.saturating_sub(area.x) as usize) as u16;
    let shell_x = comprehension_x.saturating_sub(shell_width);
    let shell_style = Style::default()
        .fg(Color::Rgb(255, 220, 120))
        .bg(Color::Rgb(100, 48, 28))
        .add_modifier(Modifier::BOLD);
    let profile_key = editor.ai_chat_effective_profile();
    let profile_label = editor
        .ai_state
        .config
        .resolve_profile(&profile_key)
        .map(|profile| profile.display_name())
        .unwrap_or(&profile_key);
    let model_label = format!(" M:{profile_label} ▾ ");
    let effort_label = format!(" E:{} ▾ ", editor.ai_chat_reasoning_effort());
    let permission_label = editor
        .ai_chat_permission_mode()
        .and_then(|mode| {
            editor
                .ai_chat_permission_modes()
                .iter()
                .find(|option| option.id == mode)
        })
        .map(|option| format!(" P:{} ▾ ", option.label));
    let model_width = text_display_width(&model_label) as u16;
    let effort_width = text_display_width(&effort_label) as u16;
    let permission_width = permission_label
        .as_deref()
        .map(text_display_width)
        .unwrap_or_default() as u16;
    let controls_available = shell_x.saturating_sub(area.x);
    let show_model = model_width <= controls_available;
    let show_effort = show_model && model_width.saturating_add(effort_width) <= controls_available;
    let show_permission = show_effort
        && permission_label.is_some()
        && model_width
            .saturating_add(effort_width)
            .saturating_add(permission_width)
            <= controls_available;

    let mut spans = Vec::new();
    let mut controls_width = 0;
    if show_model {
        spans.push(Span::styled(
            model_label,
            Style::default()
                .fg(Color::Rgb(190, 220, 255))
                .bg(Color::Rgb(38, 55, 78))
                .add_modifier(Modifier::BOLD),
        ));
        controls_width += model_width;
    }
    if show_effort {
        spans.push(Span::styled(
            effort_label,
            Style::default()
                .fg(Color::Rgb(211, 196, 255))
                .bg(Color::Rgb(55, 45, 78)),
        ));
        controls_width += effort_width;
    }
    if show_permission {
        spans.push(Span::styled(
            permission_label.expect("permission label checked"),
            if skips_approvals {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(176, 32, 32))
            } else {
                Style::default()
                    .fg(Color::Rgb(180, 226, 210))
                    .bg(Color::Rgb(38, 66, 59))
            },
        ));
        controls_width += permission_width;
    }
    spans.push(Span::styled(shell_label, shell_style));
    spans.push(Span::styled(comprehension_label, comprehension_style));
    spans.push(Span::styled(yolo_label, yolo_style));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
        Rect::new(area.x, area.y, area.width, 1),
    );
    editor.render_cache.ai_chat_interactions.yolo_toggle = (!external_agent).then(|| {
        crate::key_convert::convert_ratatui_rect(Rect::new(yolo_x, area.y, yolo_width, 1))
    });
    editor
        .render_cache
        .ai_chat_interactions
        .comprehension_toggle = (!external_agent).then(|| {
        crate::key_convert::convert_ratatui_rect(Rect::new(
            comprehension_x,
            area.y,
            comprehension_width,
            1,
        ))
    });
    let controls_x = shell_x.saturating_sub(controls_width);
    if show_model {
        editor
            .render_cache
            .ai_chat_interactions
            .model_picker_trigger = Some(crate::key_convert::convert_ratatui_rect(Rect::new(
            controls_x,
            area.y,
            model_width,
            1,
        )));
    }
    if show_effort {
        editor
            .render_cache
            .ai_chat_interactions
            .effort_picker_trigger = Some(crate::key_convert::convert_ratatui_rect(Rect::new(
            controls_x + model_width,
            area.y,
            effort_width,
            1,
        )));
    }
    if show_permission {
        editor
            .render_cache
            .ai_chat_interactions
            .permission_picker_trigger = Some(crate::key_convert::convert_ratatui_rect(Rect::new(
            controls_x + model_width + effort_width,
            area.y,
            permission_width,
            1,
        )));
    }
    show_model.then_some(Rect::new(controls_x, area.y, controls_width, 1))
}
