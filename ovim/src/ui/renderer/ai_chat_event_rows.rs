use ovim_core::ai::chat_types::{ChatMessage, ChatRole, ToolCallInfo, ToolSummaryKind};
use ovim_core::editor::QueuedChatInputKind;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::ai_chat_style::{
    ACCENT_SELECTED, BG_SELECTED_ROW, TEXT_DIM, TOOL_BG_DIAG, TOOL_BG_ERROR, TOOL_BG_MUT,
    TOOL_BG_NAV, TOOL_BG_OTHER, TOOL_BG_READ, TOOL_BG_SEARCH, TOOL_DIAG, TOOL_ERROR, TOOL_MUT,
    TOOL_NAV, TOOL_READ, TOOL_SEARCH,
};
use super::ai_chat_text::{text_display_width, truncate_with_ellipsis, word_wrap};

pub(super) fn is_hidden_tool_only_assistant(message: &ChatMessage) -> bool {
    message.role == ChatRole::Assistant
        && message.content.trim().is_empty()
        && !message.tool_calls.is_empty()
}

pub(super) fn fallback_tool_summary(message: &ChatMessage) -> (ToolSummaryKind, String) {
    let content = message.content.trim();
    if content.starts_with("Error:") {
        return (
            ToolSummaryKind::Error,
            content
                .trim_start_matches("Error:")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    (
        ToolSummaryKind::Other,
        content
            .split('\n')
            .next()
            .unwrap_or("tool result")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

pub(super) fn summarize_streaming_tool_call(tool_call: &ToolCallInfo) -> (ToolSummaryKind, String) {
    match tool_call.name.as_str() {
        "read_file_at_path" | "open_file" => {
            let path = tool_call
                .arguments
                .get("path")
                .and_then(|v| v.as_str())
                .map(compact_tool_path)
                .unwrap_or_else(|| "path".to_string());
            let kind = if tool_call.name == "open_file" {
                ToolSummaryKind::Navigation
            } else {
                ToolSummaryKind::Read
            };
            (kind, path)
        }
        "edit_range" | "insert_lines" | "delete_lines" => {
            (ToolSummaryKind::Mutation, "editing".to_string())
        }
        "write_file_at_path" => (ToolSummaryKind::Mutation, "writing file".to_string()),
        "create_file" => (ToolSummaryKind::Mutation, "creating file".to_string()),
        "snapshot_file" => (ToolSummaryKind::Other, "snapshot file".to_string()),
        "restore_file" => (ToolSummaryKind::Mutation, "restoring file".to_string()),
        "search_project" | "list_files" => (ToolSummaryKind::Search, tool_call.name.clone()),
        "read_diagnostics" | "read_project_diagnostics" => {
            (ToolSummaryKind::Diagnostics, "diagnostics".to_string())
        }
        "select_text" => (ToolSummaryKind::Navigation, "selecting text".to_string()),
        _ => (ToolSummaryKind::Other, tool_call.name.clone()),
    }
}

pub(super) fn render_tool_event_row(
    panel_width: usize,
    label: &str,
    kind: ToolSummaryKind,
    selected: bool,
    pending: bool,
    expanded: bool,
    action: Option<&'static str>,
) -> Line<'static> {
    let color = if selected {
        ACCENT_SELECTED
    } else {
        tool_kind_color(kind)
    };
    let bg = if selected {
        BG_SELECTED_ROW
    } else {
        tool_kind_background(kind)
    };
    let prefix = match kind {
        ToolSummaryKind::Mutation => "\u{0394}",
        ToolSummaryKind::Navigation => "\u{21aa}",
        ToolSummaryKind::Read => "\u{2263}",
        ToolSummaryKind::Search => "\u{2315}",
        ToolSummaryKind::Diagnostics => "\u{2691}",
        ToolSummaryKind::Error => "\u{00d7}",
        ToolSummaryKind::Other => "\u{2022}",
    };
    let disclosure = if pending {
        " "
    } else if expanded {
        "▾"
    } else {
        "▸"
    };
    let action_width = action.map_or(0, text_display_width);
    let label_width = panel_width.saturating_sub(action_width);
    let text = format!(" {disclosure} {prefix} {label}");
    let display = compact_tool_text(&text, label_width);
    let mut style = Style::default().fg(color).bg(bg);
    if pending {
        style = style.add_modifier(Modifier::DIM);
    }
    if selected {
        style = style.add_modifier(Modifier::BOLD);
    }
    let padded = format!(
        "{}{}",
        display,
        " ".repeat(label_width.saturating_sub(text_display_width(&display)))
    );
    let mut spans = vec![Span::styled(padded, style)];
    if let Some(action) = action {
        spans.push(Span::styled(
            action,
            Style::default()
                .fg(if selected { ACCENT_SELECTED } else { TOOL_NAV })
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}

pub(super) fn render_tool_event_details(
    panel_width: usize,
    call: Option<&ToolCallInfo>,
    result: &str,
) -> Vec<Line<'static>> {
    const MAX_DETAIL_LINES: usize = 80;
    let detail_width = panel_width.saturating_sub(4).max(1);
    let mut sections = Vec::new();
    if let Some(call) = call {
        sections.push(format!("tool: {}", call.name));
        let arguments = serde_json::to_string_pretty(&call.arguments)
            .unwrap_or_else(|_| call.arguments.to_string());
        sections.push(format!("arguments:\n{arguments}"));
    }
    sections.push(format!("result:\n{result}"));

    let mut detail_lines = Vec::new();
    for section in sections {
        for line in word_wrap(&section, detail_width) {
            if detail_lines.len() == MAX_DETAIL_LINES {
                detail_lines.push("… details truncated".to_string());
                break;
            }
            detail_lines.push(line);
        }
        if detail_lines.len() > MAX_DETAIL_LINES {
            break;
        }
    }

    let style = Style::default()
        .fg(TEXT_DIM)
        .bg(tool_kind_background(ToolSummaryKind::Other));
    detail_lines
        .into_iter()
        .map(|line| {
            let text = format!("    {line}");
            let display = truncate_with_ellipsis(&text, panel_width);
            let padding = panel_width.saturating_sub(text_display_width(&display));
            Line::from(Span::styled(
                format!("{display}{}", " ".repeat(padding)),
                style,
            ))
        })
        .collect()
}

pub(super) fn render_queued_input_row(
    panel_width: usize,
    kind: QueuedChatInputKind,
    content: &str,
    image_count: usize,
    selected: bool,
) -> Line<'static> {
    let (prefix, label, color, background) = match kind {
        QueuedChatInputKind::Steer => (
            "↳",
            "steer",
            Color::Rgb(115, 190, 255),
            Color::Rgb(25, 45, 64),
        ),
        QueuedChatInputKind::FollowUp => (
            "⌛",
            "queued",
            Color::Rgb(190, 170, 255),
            Color::Rgb(44, 36, 62),
        ),
        QueuedChatInputKind::Command => (
            "/",
            "command",
            Color::Rgb(255, 196, 105),
            Color::Rgb(60, 45, 25),
        ),
    };
    let attachment = match image_count {
        0 => String::new(),
        1 => " [📎 image]".to_string(),
        count => format!(" [📎 {count} images]"),
    };
    let text = format!(
        " {prefix} {label}: {}{attachment}",
        content.replace('\n', " ")
    );
    let display = truncate_with_ellipsis(&text, panel_width);
    let padding = panel_width.saturating_sub(text_display_width(&display));
    let style = Style::default()
        .fg(color)
        .bg(if selected {
            Color::Rgb(64, 82, 120)
        } else {
            background
        })
        .add_modifier(if selected {
            Modifier::BOLD
        } else {
            Modifier::DIM
        });
    Line::from(Span::styled(
        format!("{display}{}", " ".repeat(padding)),
        style,
    ))
}

fn compact_tool_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return ".".to_string();
    }
    let keep = 3usize.min(parts.len());
    let tail = parts[parts.len() - keep..].join("/");
    compact_tool_text(&tail, 42)
}

fn compact_tool_text(text: &str, max_width: usize) -> String {
    let single_line = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('\n', " ");
    truncate_with_ellipsis(&single_line, max_width)
}

fn tool_kind_color(kind: ToolSummaryKind) -> Color {
    match kind {
        ToolSummaryKind::Read => TOOL_READ,
        ToolSummaryKind::Navigation => TOOL_NAV,
        ToolSummaryKind::Mutation => TOOL_MUT,
        ToolSummaryKind::Search => TOOL_SEARCH,
        ToolSummaryKind::Diagnostics => TOOL_DIAG,
        ToolSummaryKind::Error => TOOL_ERROR,
        ToolSummaryKind::Other => TEXT_DIM,
    }
}

fn tool_kind_background(kind: ToolSummaryKind) -> Color {
    match kind {
        ToolSummaryKind::Read => TOOL_BG_READ,
        ToolSummaryKind::Navigation => TOOL_BG_NAV,
        ToolSummaryKind::Mutation => TOOL_BG_MUT,
        ToolSummaryKind::Search => TOOL_BG_SEARCH,
        ToolSummaryKind::Diagnostics => TOOL_BG_DIAG,
        ToolSummaryKind::Error => TOOL_BG_ERROR,
        ToolSummaryKind::Other => TOOL_BG_OTHER,
    }
}
