use crate::syntax::Theme;
use ovim_core::ai::chat_types::{ChatMessage, ChatRole};
use ovim_core::editor::ChatLink;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::hash::{Hash, Hasher};

use super::ai_chat_style::{message_row_style, MessageRowStyle, ACCENT_USER, TEXT_DIM};
use super::ai_chat_text::{
    styled_word_wrap_line_with_ranges, text_display_width, truncate_with_ellipsis, word_wrap,
};
use super::line_cache::ChatBubbleCacheKey;

pub(super) struct BubbleImagePlacement {
    /// First inner image row relative to the message bubble.
    pub(super) row: usize,
    /// Inner image column relative to the history panel.
    pub(super) x: u16,
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) path: std::path::PathBuf,
}

pub(super) fn chat_bubble_cache_key(
    conversation_id: u64,
    node_id: u64,
    panel_width: usize,
    selected: bool,
    allow_edits: bool,
    thinking_expanded: bool,
    child_count: usize,
    branch_position: Option<(usize, usize)>,
    theme: &Theme,
    terminal_image_support: bool,
) -> ChatBubbleCacheKey {
    let mut theme_hasher = std::collections::hash_map::DefaultHasher::new();
    theme.scheme().name.hash(&mut theme_hasher);

    ChatBubbleCacheKey {
        conversation_id,
        node_id,
        panel_width,
        selected,
        allow_edits,
        thinking_expanded,
        child_count,
        branch_position,
        theme_hash: theme_hasher.finish(),
        terminal_image_support,
    }
}

pub(super) struct ChatBubbleRender {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) images: Vec<BubbleImagePlacement>,
    pub(super) links: Vec<ChatLink>,
    /// Pending math images must be polled again instead of freezing the raw
    /// LaTeX fallback in the chat-bubble cache.
    pub(super) cacheable: bool,
}

pub(super) fn card_text_width(panel_width: usize, accent_glyph: &str) -> usize {
    panel_width
        .saturating_sub(text_display_width(accent_glyph) + 1)
        .max(1)
}

pub(super) fn render_card_text_line(
    panel_width: usize,
    accent_glyph: &str,
    accent_color: Color,
    row_bg: Color,
    text: &str,
    text_style: Style,
) -> Line<'static> {
    let width = card_text_width(panel_width, accent_glyph);
    let display = truncate_with_ellipsis(text, width);
    let padding = width.saturating_sub(text_display_width(&display));
    let mut spans = Vec::with_capacity(3);
    spans.push(Span::styled(
        accent_glyph.to_string(),
        Style::default()
            .fg(accent_color)
            .bg(row_bg)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" ", Style::default().bg(row_bg)));
    spans.push(Span::styled(
        format!("{}{}", display, " ".repeat(padding)),
        text_style.bg(row_bg),
    ));
    Line::from(spans)
}

pub(super) fn branch_control_text(position: usize, count: usize) -> String {
    format!("[‹ {}/{} ›]", position + 1, count)
}

pub(super) fn render_card_header_line(
    panel_width: usize,
    accent_glyph: &str,
    row_style: MessageRowStyle,
    label: &str,
    branch_position: Option<(usize, usize)>,
) -> Line<'static> {
    let width = card_text_width(panel_width, accent_glyph);
    let control = branch_position
        .map(|(position, count)| branch_control_text(position, count))
        .filter(|text| text_display_width(text) < width);
    let control_width = control
        .as_ref()
        .map(|text| text_display_width(text))
        .unwrap_or(0);
    let label_width = width.saturating_sub(control_width + usize::from(control.is_some()));
    let display_label = truncate_with_ellipsis(label, label_width);
    let gap = width.saturating_sub(text_display_width(&display_label) + control_width);
    let label_style = Style::default()
        .fg(row_style.label_fg)
        .bg(row_style.label_bg)
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(
            accent_glyph.to_string(),
            Style::default()
                .fg(row_style.accent)
                .bg(row_style.label_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default().bg(row_style.label_bg)),
        Span::styled(display_label, label_style),
        Span::styled(" ".repeat(gap), Style::default().bg(row_style.label_bg)),
    ];
    if let Some(control) = control {
        spans.push(Span::styled(
            control,
            Style::default()
                .fg(Color::Rgb(155, 205, 255))
                .bg(Color::Rgb(38, 61, 88))
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}

pub(super) fn render_card_styled_line(
    panel_width: usize,
    accent_glyph: &str,
    accent_color: Color,
    row_bg: Color,
    row_spans: Vec<Span<'static>>,
) -> Line<'static> {
    let width = card_text_width(panel_width, accent_glyph);
    let mut used = 0usize;
    let mut spans = Vec::with_capacity(row_spans.len() + 3);
    spans.push(Span::styled(
        accent_glyph.to_string(),
        Style::default()
            .fg(accent_color)
            .bg(row_bg)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(" ", Style::default().bg(row_bg)));

    for span in row_spans {
        let remaining = width.saturating_sub(used);
        if remaining == 0 {
            break;
        }
        let content = truncate_with_ellipsis(&span.content, remaining);
        let span_width = text_display_width(&content);
        if span_width == 0 {
            continue;
        }
        let style = if span.style.bg.is_some() {
            span.style
        } else {
            span.style.bg(row_bg)
        };
        spans.push(Span::styled(content, style));
        used += span_width;
    }

    if used < width {
        spans.push(Span::styled(
            " ".repeat(width - used),
            Style::default().bg(row_bg),
        ));
    }

    Line::from(spans)
}

pub(super) fn render_chat_bubble(
    message: &ChatMessage,
    panel_width: usize,
    is_selected: bool,
    allow_edits: bool,
    is_thinking_expanded: bool,
    child_count: usize,
    branch_position: Option<(usize, usize)>,
    theme: &Theme,
    terminal_image_support: bool,
) -> ChatBubbleRender {
    let row_style = message_row_style(message.role.clone(), allow_edits, is_selected);
    let accent_glyph = if is_selected { "\u{258c}" } else { "\u{258d}" };
    let text_style = Style::default().fg(row_style.text_fg);
    let mut lines = Vec::new();
    let mut images = Vec::new();
    let mut links = Vec::new();
    let mut cacheable = true;

    let label = match message.role {
        ChatRole::User => "You".to_string(),
        ChatRole::Assistant => {
            if let Some(ref model) = message.model {
                model.clone()
            } else {
                "Assistant".to_string()
            }
        }
        ChatRole::Thinking => "Thinking".to_string(),
        ChatRole::Error => "Error".to_string(),
        ChatRole::Tool => "Tool".to_string(),
    };

    let header = if child_count > 1 {
        format!("{label}  \u{2442} {child_count}")
    } else {
        label
    };
    lines.push(render_card_header_line(
        panel_width,
        accent_glyph,
        row_style,
        &header,
        branch_position,
    ));

    let inner_width = card_text_width(panel_width, accent_glyph);

    let (code_attachment_label, visible_content) = if message.role == ChatRole::User {
        if let Some((label, content)) =
            ovim_core::editor::split_code_attachment_message(&message.content)
        {
            (Some(label), content)
        } else {
            (None, message.content.as_str())
        }
    } else {
        (None, message.content.as_str())
    };

    if let Some(label) = code_attachment_label {
        lines.push(render_card_text_line(
            panel_width,
            accent_glyph,
            row_style.accent,
            row_style.body_bg,
            &format!("📎 {label}"),
            Style::default().fg(ACCENT_USER).add_modifier(Modifier::DIM),
        ));
    }

    if terminal_image_support && !message.images.is_empty() {
        let (image_lines, image_placements) = render_message_image_boxes(
            &message.images,
            panel_width,
            accent_glyph,
            row_style,
            lines.len(),
        );
        lines.extend(image_lines);
        images.extend(image_placements);
    } else {
        for image in &message.images {
            lines.push(render_card_text_line(
                panel_width,
                accent_glyph,
                row_style.accent,
                row_style.body_bg,
                &format!("📎 {}", image.file_name()),
                Style::default().fg(ACCENT_USER).add_modifier(Modifier::DIM),
            ));
        }
    }

    // For thinking messages: collapsed vs expanded
    if message.role == ChatRole::Thinking && !is_thinking_expanded {
        let first_line = message.content.lines().next().unwrap_or("");
        lines.push(render_card_text_line(
            panel_width,
            accent_glyph,
            row_style.accent,
            row_style.body_bg,
            &format!("\u{25b8} {}", first_line),
            text_style,
        ));
    } else if message.role == ChatRole::Assistant {
        let md_elements = super::markdown::parse_markdown(&message.content);
        let content_start = lines.len();
        cacheable = append_assistant_markdown(
            &md_elements,
            &mut lines,
            &mut images,
            &mut links,
            panel_width,
            inner_width,
            accent_glyph,
            row_style,
            theme,
            terminal_image_support,
        );
        if lines.len() == content_start {
            lines.push(render_card_text_line(
                panel_width,
                accent_glyph,
                row_style.accent,
                row_style.body_bg,
                "",
                text_style,
            ));
        }
    } else {
        // Plain text for thinking, user, error, tool messages
        let display_content = if message.role == ChatRole::Thinking {
            format!("\u{25be} {}", message.content)
        } else {
            visible_content.to_string()
        };
        let wrapped = word_wrap(&display_content, inner_width);
        for row in &wrapped {
            lines.push(render_card_text_line(
                panel_width,
                accent_glyph,
                row_style.accent,
                row_style.body_bg,
                row,
                text_style,
            ));
        }
    }

    // Retry hint for error bubbles
    if message.role == ChatRole::Error {
        lines.push(render_card_text_line(
            panel_width,
            accent_glyph,
            row_style.accent,
            row_style.body_bg,
            "(submit again to retry)",
            Style::default().fg(TEXT_DIM).add_modifier(Modifier::DIM),
        ));
    }

    ChatBubbleRender {
        lines,
        images,
        links,
        cacheable,
    }
}

#[allow(clippy::too_many_arguments)]
fn append_assistant_markdown(
    elements: &[super::markdown::MarkdownElement],
    lines: &mut Vec<Line<'static>>,
    images: &mut Vec<BubbleImagePlacement>,
    links: &mut Vec<ChatLink>,
    panel_width: usize,
    inner_width: usize,
    accent_glyph: &str,
    row_style: MessageRowStyle,
    theme: &Theme,
    terminal_image_support: bool,
) -> bool {
    use super::display_math::{request_display_math, MathRenderStatus};
    use super::markdown::MarkdownElement;

    let mut cacheable = true;
    let mut ordinary = Vec::new();
    for element in elements {
        let MarkdownElement::DisplayMath(math) = element else {
            ordinary.push(element.clone());
            continue;
        };

        append_ordinary_markdown(
            &ordinary,
            lines,
            links,
            panel_width,
            inner_width,
            accent_glyph,
            row_style,
            theme,
        );
        ordinary.clear();

        let status = if terminal_image_support {
            let color = match row_style.text_fg {
                Color::Rgb(red, green, blue) => [red, green, blue],
                _ => [200, 208, 220],
            };
            request_display_math(math, inner_width as u16, color)
        } else {
            MathRenderStatus::Failed
        };

        if let MathRenderStatus::Ready(rendered) = status {
            let image_width = rendered.width.min(inner_width as u16);
            let x = 2usize
                .saturating_add(inner_width.saturating_sub(image_width as usize) / 2)
                .min(u16::MAX as usize) as u16;
            images.push(BubbleImagePlacement {
                row: lines.len(),
                x,
                width: image_width,
                height: rendered.height,
                path: rendered.path,
            });
            for _ in 0..rendered.height {
                lines.push(render_card_text_line(
                    panel_width,
                    accent_glyph,
                    row_style.accent,
                    row_style.body_bg,
                    "",
                    Style::default().fg(row_style.text_fg),
                ));
            }
        } else {
            if matches!(status, MathRenderStatus::Pending) {
                cacheable = false;
            }
            append_ordinary_markdown(
                &[MarkdownElement::DisplayMath(math.clone())],
                lines,
                links,
                panel_width,
                inner_width,
                accent_glyph,
                row_style,
                theme,
            );
        }
    }

    append_ordinary_markdown(
        &ordinary,
        lines,
        links,
        panel_width,
        inner_width,
        accent_glyph,
        row_style,
        theme,
    );
    cacheable
}

#[allow(clippy::too_many_arguments)]
fn append_ordinary_markdown(
    elements: &[super::markdown::MarkdownElement],
    lines: &mut Vec<Line<'static>>,
    links: &mut Vec<ChatLink>,
    panel_width: usize,
    inner_width: usize,
    accent_glyph: &str,
    row_style: MessageRowStyle,
    theme: &Theme,
) {
    if elements.is_empty() {
        return;
    }
    let (markdown_lines, markdown_links) =
        super::markdown::render_markdown_with_links(elements, inner_width, Some(theme));
    for (markdown_row, markdown_line) in markdown_lines.iter().enumerate() {
        let text: String = markdown_line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        for row in styled_word_wrap_line_with_ranges(markdown_line, inner_width) {
            for link in markdown_links
                .iter()
                .filter(|link| link.row == markdown_row)
            {
                let start = row.source.start.max(link.bytes.start);
                let end = row.source.end.min(link.bytes.end);
                if start < end {
                    let column = 2 + text_display_width(&text[row.source.start..start]);
                    links.push(ChatLink {
                        row: lines.len(),
                        columns: column..column + text_display_width(&text[start..end]),
                        destination: link.destination.clone(),
                    });
                }
            }
            lines.push(render_card_styled_line(
                panel_width,
                accent_glyph,
                row_style.accent,
                row_style.body_bg,
                row.spans,
            ));
        }
    }
}

fn render_message_image_boxes(
    attachments: &[ovim_core::ai::chat_types::ImageAttachment],
    panel_width: usize,
    accent_glyph: &str,
    row_style: MessageRowStyle,
    first_row: usize,
) -> (Vec<Line<'static>>, Vec<BubbleImagePlacement>) {
    const THUMB_WIDTH: usize = 14;
    const THUMB_HEIGHT: usize = 6;

    let content_width = card_text_width(panel_width, accent_glyph);
    let capacity = (content_width / THUMB_WIDTH).max(1);
    let mut lines = Vec::new();
    let mut placements = Vec::new();

    for group in attachments.chunks(capacity) {
        let group_row = first_row + lines.len();
        for row in 0..THUMB_HEIGHT {
            let mut content = String::new();
            for image in group {
                content.push_str(&thumbnail_box_row(
                    &image.file_name(),
                    THUMB_WIDTH.min(content_width),
                    row,
                    THUMB_HEIGHT,
                ));
            }
            lines.push(render_card_text_line(
                panel_width,
                accent_glyph,
                row_style.accent,
                row_style.body_bg,
                &content,
                Style::default().fg(ACCENT_USER).add_modifier(Modifier::DIM),
            ));
        }

        for (index, image) in group.iter().enumerate() {
            let outer_x = index * THUMB_WIDTH;
            let outer_width = THUMB_WIDTH.min(content_width.saturating_sub(outer_x));
            if outer_width < 4 {
                continue;
            }
            // Two columns precede card content: accent glyph and a space.
            placements.push(BubbleImagePlacement {
                row: group_row + 1,
                x: (outer_x + 3) as u16,
                width: outer_width.saturating_sub(2) as u16,
                height: THUMB_HEIGHT.saturating_sub(2) as u16,
                path: image.path.clone(),
            });
        }
    }

    (lines, placements)
}

fn thumbnail_box_row(name: &str, width: usize, row: usize, height: usize) -> String {
    if width < 2 {
        return " ".repeat(width);
    }
    let inner = width - 2;
    if row == 0 {
        let title = truncate_with_ellipsis(name, inner);
        return format!(
            "╭{title}{}╮",
            "─".repeat(inner.saturating_sub(text_display_width(&title)))
        );
    }
    if row + 1 == height {
        return format!("╰{}╯", "─".repeat(inner));
    }
    format!("│{}│", " ".repeat(inner))
}
