use crate::editor::Editor;
use crate::syntax::Theme;
use ovim_core::ai::chat_types::{ChatFocus, ChatMessage, ChatRole};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

// ---------------------------------------------------------------------------
// Colors (pub(crate) so conversation_tree can reuse them)
// ---------------------------------------------------------------------------

pub(crate) const BG_PANEL: Color = Color::Reset;
const BG_INPUT: Color = Color::Rgb(28, 33, 42);

const BORDER_USER: Color = Color::Cyan;
const BORDER_ASSISTANT_EDIT: Color = Color::Green;
const BORDER_ASSISTANT_QUERY: Color = Color::Rgb(100, 149, 237); // cornflower blue
const BORDER_ERROR: Color = Color::Red;
const BORDER_THINKING: Color = Color::Rgb(80, 88, 100);
const BORDER_SELECTED: Color = Color::Yellow;

pub(crate) const TEXT_DIM: Color = Color::Rgb(128, 140, 155);
const TEXT_THINKING: Color = Color::Rgb(100, 112, 130);
pub(crate) const TEXT_NORMAL: Color = Color::Rgb(200, 208, 220);

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Split content area into buffer (left) and chat panel (right).
pub fn compute_chat_split(content_area: Rect, allow_edits: bool) -> (Rect, Rect) {
    let total = content_area.width;
    let chat_pct: u16 = if allow_edits { 40 } else { 35 };
    let min_chat = 30u16;
    let min_buffer = 40u16;

    let chat_width = (total * chat_pct / 100)
        .max(min_chat)
        .min(total.saturating_sub(min_buffer));
    let buffer_width = total.saturating_sub(chat_width);

    let buffer_rect = Rect {
        x: content_area.x,
        y: content_area.y,
        width: buffer_width,
        height: content_area.height,
    };
    let chat_rect = Rect {
        x: content_area.x + buffer_width,
        y: content_area.y,
        width: chat_width,
        height: content_area.height,
    };
    (buffer_rect, chat_rect)
}

/// Width of tree panel when open.
fn tree_panel_width(chat_width: u16) -> u16 {
    let quarter = chat_width / 4;
    quarter.max(20).min(36)
}

/// Render the full chat panel.
pub fn render_chat_panel(frame: &mut Frame, editor: &mut Editor, chat_area: Rect, theme: &Theme) {
    if chat_area.width < 4 || chat_area.height < 3 {
        return;
    }

    // Split for tree panel if open
    let tree_open = editor.ai_chat_tree_panel_open();
    let (tree_area, main_area) = if tree_open && chat_area.width > 40 {
        let tw = tree_panel_width(chat_area.width);
        let tree_rect = Rect {
            x: chat_area.x,
            y: chat_area.y,
            width: tw,
            height: chat_area.height,
        };
        let main_rect = Rect {
            x: chat_area.x + tw,
            y: chat_area.y,
            width: chat_area.width.saturating_sub(tw),
            height: chat_area.height,
        };
        (Some(tree_rect), main_rect)
    } else {
        (None, chat_area)
    };

    // Render tree panel if open
    if let Some(tree_rect) = tree_area {
        super::conversation_tree::render_tree_panel(frame, editor, tree_rect);
    }

    // Layout: [message_history | input_bar(dynamic) | model_selector(1)]
    let model_bar_height = 1u16;
    let input_content_width = (main_area.width as usize).saturating_sub(2 + 3 + 2); // "│ " + prompt + " │"
    let input_lines = if input_content_width > 0 {
        let input_text = editor.ai_chat_input();
        wrap_input_rows(input_text, input_content_width, editor.options.tab_width).len()
    } else {
        1
    };
    let input_height = (1 + input_lines as u16).min(6); // border + content, max ~5 lines
    let min_chrome = model_bar_height + input_height;

    if main_area.height <= min_chrome {
        // Too small — just render input
        render_text_input(frame, editor, main_area);
        return;
    }

    let messages_height = main_area.height - min_chrome;

    let messages_area = Rect {
        x: main_area.x,
        y: main_area.y,
        width: main_area.width,
        height: messages_height,
    };
    let input_area = Rect {
        x: main_area.x,
        y: main_area.y + messages_height,
        width: main_area.width,
        height: input_height,
    };
    let model_area = Rect {
        x: main_area.x,
        y: main_area.y + messages_height + input_height,
        width: main_area.width,
        height: model_bar_height,
    };

    render_message_history(frame, editor, messages_area, theme);
    render_text_input(frame, editor, input_area);
    render_model_selector_bar(frame, editor, model_area);

    // Show standalone waiting indicator only before first streaming chunk arrives.
    if editor.ai_chat_waiting() {
        let has_visible_streaming = editor
            .ai_chat_streaming_content()
            .is_some_and(|s| !s.is_empty())
            || editor
                .ai_chat_streaming_thinking()
                .is_some_and(|s| !s.is_empty());
        if !has_visible_streaming {
            render_waiting_indicator(frame, messages_area);
        }
    }
}

/// Returns cursor (x, y) for the chat input, if focused.
pub fn chat_cursor_info(editor: &Editor, chat_area: Rect) -> Option<(u16, u16)> {
    let focus = editor.ai_chat_focus();
    if focus != ChatFocus::TextInput {
        return None;
    }

    // Account for tree panel offset
    let tree_open = editor.ai_chat_tree_panel_open();
    let main_area = if tree_open && chat_area.width > 40 {
        let tw = tree_panel_width(chat_area.width);
        Rect {
            x: chat_area.x + tw,
            y: chat_area.y,
            width: chat_area.width.saturating_sub(tw),
            height: chat_area.height,
        }
    } else {
        chat_area
    };

    let content_width = (main_area.width as usize).saturating_sub(2 + 3 + 2); // "│ " + prompt + " │"
    let input = editor.ai_chat_input();
    let tab_width = editor.options.tab_width;
    let wrapped_rows = wrap_input_rows(input, content_width.max(1), tab_width);
    let input_line_count = wrapped_rows.len();
    let input_height = (1 + input_line_count as u16).min(6);
    let min_chrome = input_height + 1; // input + model(1)
    if main_area.height <= min_chrome {
        return None;
    }

    let messages_height = main_area.height - min_chrome;
    let input_y = main_area.y + messages_height;

    let cursor_byte = editor.ai_chat_input_cursor();
    let safe_cursor = cursor_byte.min(input.len());
    let (cursor_line, col) =
        input_cursor_row_col(input, safe_cursor, content_width.max(1), tab_width);

    // First line has "│ >> " prefix (border + space + prompt = 5), continuation lines same width
    let prefix_len = 5u16;
    let x = main_area
        .x
        .saturating_add(prefix_len)
        .saturating_add(col as u16)
        .min(main_area.x + main_area.width.saturating_sub(1));
    // +1 for the top border row, then offset by cursor_line
    let y = input_y + 1 + cursor_line as u16;

    Some((x, y))
}

// ---------------------------------------------------------------------------
// Message History
// ---------------------------------------------------------------------------

fn render_message_history(frame: &mut Frame, editor: &mut Editor, area: Rect, theme: &Theme) {
    let messages = editor.ai_chat_messages();
    if messages.is_empty() {
        editor.render_cache.ai_chat_last_total_rows = 0;
        editor.render_cache.ai_chat_last_visible_start_row = 0;
        editor.render_cache.ai_chat_last_visible_end_row = 0;
        editor.render_cache.ai_chat_last_message_row_spans.clear();

        // Empty state
        let help = if editor.ai_chat_allow_edits() {
            " Type a message and press Enter to chat with AI "
        } else {
            " Type a question and press Enter (read-only mode) "
        };
        let y = area.y + area.height / 2;
        if y < area.y + area.height {
            let line = Line::from(Span::styled(
                center_text(help, area.width as usize),
                Style::default().fg(TEXT_DIM).bg(BG_PANEL),
            ));
            let r = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            };
            frame.render_widget(Paragraph::new(vec![line]), r);
        }
        return;
    }

    let allow_edits = editor.ai_chat_allow_edits();
    let focus = editor.ai_chat_focus();
    let selected_offset = editor
        .ai_chat_history_cursor_offset()
        .min(messages.len().saturating_sub(1));
    let panel_width = area.width as usize;

    // Get node IDs for active branch (parallel to messages)
    let node_ids = editor
        .conversation()
        .map(|c| c.node_ids_for_active_branch().to_vec())
        .unwrap_or_default();

    // Render messages bottom-up with scroll
    let mut rendered_lines: Vec<(Line, bool)> = Vec::new(); // (line, is_bubble_border)
    let mut message_row_spans: Vec<(usize, usize)> = Vec::with_capacity(messages.len());
    for (idx, msg) in messages.iter().enumerate() {
        let is_selected = focus == ChatFocus::MessageHistory
            && idx == messages.len().saturating_sub(1 + selected_offset);

        // Look up NodeId for thinking expansion and child count
        let node_id = node_ids.get(idx).copied();
        let is_thinking_expanded = node_id
            .map(|id| editor.ai_chat_is_thinking_expanded(id))
            .unwrap_or(false);
        let child_count = node_id
            .and_then(|id| editor.conversation().map(|c| c.child_count(id)))
            .unwrap_or(0);

        let bubble_lines = render_chat_bubble(
            msg,
            panel_width,
            is_selected,
            allow_edits,
            is_thinking_expanded,
            child_count,
            theme,
        );
        let msg_row_start = rendered_lines.len();
        for line in bubble_lines {
            rendered_lines.push((line, false));
        }
        // Spacing between messages
        rendered_lines.push((
            Line::from(Span::styled(
                " ".repeat(panel_width),
                Style::default().bg(BG_PANEL),
            )),
            true,
        ));
        let msg_row_end = rendered_lines.len();
        message_row_spans.push((msg_row_start, msg_row_end));
    }

    // Streaming thinking bubble (if any)
    if let Some(thinking) = editor.ai_chat_streaming_thinking() {
        if !thinking.is_empty() {
            let streaming_thinking_msg = ChatMessage {
                role: ChatRole::Thinking,
                content: thinking.to_string(),
                model: None,
                timestamp: std::time::Instant::now(),
                tool_calls: vec![],
                tool_call_id: None,
            };
            let bubble_lines = render_chat_bubble(
                &streaming_thinking_msg,
                panel_width,
                false,
                allow_edits,
                true,
                0,
                theme,
            );
            for line in bubble_lines {
                rendered_lines.push((line, false));
            }
            rendered_lines.push((
                Line::from(Span::styled(
                    " ".repeat(panel_width),
                    Style::default().bg(BG_PANEL),
                )),
                true,
            ));
        }
    }

    // Streaming content bubble (if any)
    if let Some(content) = editor.ai_chat_streaming_content() {
        if !content.is_empty() {
            let display = format!("{}···", content);
            let streaming_msg = ChatMessage {
                role: ChatRole::Assistant,
                content: display,
                model: None,
                timestamp: std::time::Instant::now(),
                tool_calls: vec![],
                tool_call_id: None,
            };
            let bubble_lines = render_chat_bubble(
                &streaming_msg,
                panel_width,
                false,
                allow_edits,
                false,
                0,
                theme,
            );
            for line in bubble_lines {
                rendered_lines.push((line, false));
            }
            rendered_lines.push((
                Line::from(Span::styled(
                    " ".repeat(panel_width),
                    Style::default().bg(BG_PANEL),
                )),
                true,
            ));
        }
    }

    // Tool call status rows (dim indicators during tool execution)
    if let Some(chat) = editor.ai_state.chat.as_ref() {
        if !chat.streaming_tool_calls.is_empty() {
            for tc in &chat.streaming_tool_calls {
                let status_text = format!("  \u{26A1} {}(...)", tc.name);
                let padded = format!(
                    "{}{}",
                    status_text,
                    " ".repeat(panel_width.saturating_sub(status_text.chars().count()))
                );
                rendered_lines.push((
                    Line::from(Span::styled(
                        padded,
                        Style::default().fg(TEXT_DIM).bg(BG_PANEL),
                    )),
                    false,
                ));
            }
        }
    }

    // Display from bottom of area. While pinned, keep viewport stable even
    // when new streaming rows are appended.
    let visible_rows = area.height as usize;
    let total = rendered_lines.len();
    editor.render_cache.ai_chat_last_total_rows = total;
    let effective_scroll = editor.ai_chat_effective_message_scroll(total);
    let start = total.saturating_sub(visible_rows + effective_scroll);
    let end = total.saturating_sub(effective_scroll).min(total);
    editor.render_cache.ai_chat_last_visible_start_row = start;
    editor.render_cache.ai_chat_last_visible_end_row = end;
    editor.render_cache.ai_chat_last_message_row_spans = message_row_spans;

    for (row_idx, line_idx) in (start..end).enumerate() {
        if row_idx >= visible_rows {
            break;
        }
        let r = Rect {
            x: area.x,
            y: area.y + row_idx as u16,
            width: area.width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(vec![rendered_lines[line_idx].0.clone()]), r);
    }
}

fn render_chat_bubble(
    message: &ChatMessage,
    panel_width: usize,
    is_selected: bool,
    allow_edits: bool,
    is_thinking_expanded: bool,
    child_count: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let max_bubble_width = if panel_width < 60 {
        // Narrow panel: use full width minus minimal padding
        panel_width.saturating_sub(2)
    } else {
        (panel_width * 3 / 4)
            .max(20)
            .min(panel_width.saturating_sub(4))
    };
    let inner_width = max_bubble_width.saturating_sub(4); // borders + padding

    let border_color = if is_selected {
        BORDER_SELECTED
    } else {
        match message.role {
            ChatRole::User => BORDER_USER,
            ChatRole::Assistant => {
                if allow_edits {
                    BORDER_ASSISTANT_EDIT
                } else {
                    BORDER_ASSISTANT_QUERY
                }
            }
            ChatRole::Thinking => BORDER_THINKING,
            ChatRole::Error => BORDER_ERROR,
            ChatRole::Tool => {
                if message.content.starts_with("Error: ") {
                    BORDER_ERROR
                } else {
                    BORDER_ASSISTANT_QUERY
                }
            }
        }
    };

    let text_color = match message.role {
        ChatRole::Error => Color::Red,
        ChatRole::Thinking => TEXT_THINKING,
        _ => TEXT_NORMAL,
    };

    let is_user = message.role == ChatRole::User;
    let indent = if is_user {
        panel_width.saturating_sub(max_bubble_width)
    } else {
        1
    };

    let border_style = Style::default().fg(border_color).bg(BG_PANEL);
    let text_style = Style::default().fg(text_color).bg(BG_PANEL);
    let pad_style = Style::default().bg(BG_PANEL);

    let mut lines = Vec::new();

    // Role label
    let label_text = match message.role {
        ChatRole::User => " You ".to_string(),
        ChatRole::Assistant => {
            if let Some(ref model) = message.model {
                format!(" {} ", model)
            } else {
                " Assistant ".to_string()
            }
        }
        ChatRole::Thinking => " thinking ".to_string(),
        ChatRole::Error => " Error ".to_string(),
        ChatRole::Tool => " Tool ".to_string(),
    };

    // Branch indicator suffix for top border
    let branch_suffix = if child_count > 1 {
        format!(" \u{2442} {} ", child_count) // ⑂ N
    } else {
        String::new()
    };
    let suffix_display_len = branch_suffix.chars().count();

    // Top border: ╭─ label ──────── ⑂ N ─╮
    let label_display_len = label_text.chars().count();
    let top_fill = max_bubble_width.saturating_sub(2 + label_display_len + suffix_display_len);
    let top = format!(
        "{}╭{}{}{}╮{}",
        " ".repeat(indent),
        &label_text,
        "─".repeat(top_fill),
        &branch_suffix,
        " ".repeat(panel_width.saturating_sub(indent + max_bubble_width)),
    );
    lines.push(Line::from(Span::styled(top, border_style)));

    // For thinking messages: collapsed vs expanded
    if message.role == ChatRole::Thinking && !is_thinking_expanded {
        // Collapsed: single line with ▸ prefix + truncated content
        let prefix = "▸ ";
        let max_preview = inner_width.saturating_sub(prefix.len());
        let first_line = message.content.lines().next().unwrap_or("");
        let preview: String = first_line.chars().take(max_preview).collect();
        let row_chars = prefix.len() + preview.chars().count();
        let padding = inner_width.saturating_sub(row_chars);
        let spans = vec![
            Span::styled(" ".repeat(indent), pad_style),
            Span::styled("│ ", border_style),
            Span::styled(prefix, text_style),
            Span::styled(format!("{}{}", preview, " ".repeat(padding)), text_style),
            Span::styled(" │", border_style),
            Span::styled(
                " ".repeat(panel_width.saturating_sub(indent + max_bubble_width)),
                pad_style,
            ),
        ];
        lines.push(Line::from(spans));
    } else if message.role == ChatRole::Assistant {
        // Markdown-rendered content for assistant messages
        let md_elements = super::markdown::parse_markdown(&message.content);
        let md_lines = super::markdown::render_markdown(&md_elements, inner_width, Some(theme));
        for md_line in &md_lines {
            // Wrap each markdown line's spans into the bubble chrome.
            // First, compute the display width of the spans.
            let content_spans = styled_word_wrap_line(md_line, inner_width);
            for row_spans in content_spans {
                let row_width: usize = row_spans.iter().map(|s| s.content.chars().count()).sum();
                let padding = inner_width.saturating_sub(row_width);
                let mut spans = vec![
                    Span::styled(" ".repeat(indent), pad_style),
                    Span::styled("│ ", border_style),
                ];
                spans.extend(row_spans);
                if padding > 0 {
                    spans.push(Span::styled(" ".repeat(padding), pad_style));
                }
                spans.push(Span::styled(" │", border_style));
                spans.push(Span::styled(
                    " ".repeat(panel_width.saturating_sub(indent + max_bubble_width)),
                    pad_style,
                ));
                lines.push(Line::from(spans));
            }
        }
    } else {
        // Plain text for thinking, user, error, tool messages
        let display_content = if message.role == ChatRole::Thinking {
            format!("▾ {}", message.content)
        } else {
            message.content.clone()
        };
        let wrapped = word_wrap(&display_content, inner_width);
        for row in &wrapped {
            let row_chars: usize = row.chars().count();
            let padding = inner_width.saturating_sub(row_chars);
            let spans = vec![
                Span::styled(" ".repeat(indent), pad_style),
                Span::styled("│ ", border_style),
                Span::styled(format!("{}{}", row, " ".repeat(padding)), text_style),
                Span::styled(" │", border_style),
                Span::styled(
                    " ".repeat(panel_width.saturating_sub(indent + max_bubble_width)),
                    pad_style,
                ),
            ];
            lines.push(Line::from(spans));
        }
    }

    // Retry hint for error bubbles
    if message.role == ChatRole::Error {
        let hint = "(submit again to retry)";
        let hint_chars = hint.chars().count();
        let padding = inner_width.saturating_sub(hint_chars);
        let hint_style = Style::default().fg(TEXT_DIM).bg(BG_PANEL);
        let spans = vec![
            Span::styled(" ".repeat(indent), pad_style),
            Span::styled("│ ", border_style),
            Span::styled(format!("{}{}", hint, " ".repeat(padding)), hint_style),
            Span::styled(" │", border_style),
            Span::styled(
                " ".repeat(panel_width.saturating_sub(indent + max_bubble_width)),
                pad_style,
            ),
        ];
        lines.push(Line::from(spans));
    }

    // Bottom border: ╰──╯
    let bottom = format!(
        "{}╰{}╯{}",
        " ".repeat(indent),
        "─".repeat(max_bubble_width.saturating_sub(2)),
        " ".repeat(panel_width.saturating_sub(indent + max_bubble_width)),
    );
    lines.push(Line::from(Span::styled(bottom, border_style)));

    lines
}

// ---------------------------------------------------------------------------
// Text Input
// ---------------------------------------------------------------------------

fn render_text_input(frame: &mut Frame, editor: &Editor, area: Rect) {
    if area.height == 0 || area.width < 4 {
        return;
    }

    let focus = editor.ai_chat_focus();
    let waiting = editor.ai_chat_waiting();
    let input = editor.ai_chat_input();
    let allow_edits = editor.ai_chat_allow_edits();

    let border_color = if focus == ChatFocus::TextInput {
        Color::Rgb(82, 139, 255)
    } else {
        Color::Rgb(60, 66, 80)
    };

    let border_style = Style::default().fg(border_color).bg(BG_PANEL);
    let w = area.width as usize;

    // Top border of input box
    let top = format!("╭{}╮", "─".repeat(w.saturating_sub(2)));
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

    let prompt = if allow_edits { ">> " } else { "?  " };
    let prompt_len = prompt.len(); // 3
    let prefix_total = 2 + prompt_len; // "│ " + prompt = 5
    let suffix_len = 2; // " │"
    let content_width = w.saturating_sub(prefix_total + suffix_len);

    let input_fg = if waiting { TEXT_DIM } else { TEXT_NORMAL };
    let input_style = Style::default().fg(input_fg).bg(BG_INPUT);

    let display_input: &str = if waiting {
        "Waiting for response..."
    } else {
        input
    };

    // Wrap input without collapsing whitespace so cursor mapping stays accurate.
    let wrapped_rows = wrap_input_rows(display_input, content_width, editor.options.tab_width);

    for (row_idx, (start, end)) in wrapped_rows.iter().enumerate().take(content_rows) {
        let display = &display_input[*start..*end];
        let display_width = crate::display::display_width(display, editor.options.tab_width);
        let padding = content_width.saturating_sub(display_width);

        let row_prefix = if row_idx == 0 { prompt } else { "   " };

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
    }

    // Fill remaining content rows with empty bordered lines
    for row_idx in wrapped_rows.len()..content_rows {
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

// ---------------------------------------------------------------------------
// Model Selector Bar
// ---------------------------------------------------------------------------

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
        profile_names.push(editor.ai_state.active_profile.clone());
    }

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
        let summary = if pending_no_repo_approval {
            editor.ai_chat_pending_no_repo_folder_approval_summary()
        } else {
            editor.ai_chat_pending_tool_approval_summary()
        };
        if let Some(summary) = summary {
            let mut label = format!(" ! {} ", summary);
            let max_label = w.saturating_sub(4);
            if label.chars().count() > max_label {
                label = label
                    .chars()
                    .take(max_label.saturating_sub(1))
                    .collect::<String>();
                label.push('…');
            }
            let label_w = label.chars().count();
            if used_width + label_w + 1 < w {
                spans.push(Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    " ",
                    Style::default().fg(TEXT_DIM).bg(BG_PANEL),
                ));
                used_width += label_w + 1;
            }
        }
    }

    for name in &profile_names {
        let Some(profile) = editor.ai_state.config.resolve_profile(name) else {
            continue;
        };
        let model_short: String = profile.model.chars().take(20).collect();
        let label = format!(" {}:{} ", name, model_short);
        let label_w = label.chars().count();
        if used_width + label_w + 1 > w {
            break;
        }

        let is_active = *name == editor.ai_state.active_profile;
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
        " [C-y allow] [C-n deny] "
    } else if pending_tool_approval {
        " [C-y allow once] [C-a allow session] [C-n deny] "
    } else if allow_edits {
        " [Enter send] [PgUp/PgDn scroll code] [C-y copy] [Esc\u{00d7}2 close] "
    } else {
        " [?] [Enter send] [PgUp/PgDn scroll code] [C-y copy] [Esc\u{00d7}2 close] "
    };
    let hint_w = hint.chars().count();
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

// ---------------------------------------------------------------------------
// Waiting Indicator
// ---------------------------------------------------------------------------

fn render_waiting_indicator(frame: &mut Frame, messages_area: Rect) {
    if messages_area.height == 0 {
        return;
    }
    let y = messages_area.y + messages_area.height - 1;
    let dots = "  ···";
    let line = Line::from(Span::styled(
        dots,
        Style::default()
            .fg(Color::Rgb(120, 140, 180))
            .bg(BG_PANEL)
            .add_modifier(Modifier::DIM),
    ));
    frame.render_widget(
        Paragraph::new(vec![line]),
        Rect {
            x: messages_area.x,
            y,
            width: messages_area.width.min(dots.len() as u16),
            height: 1,
        },
    );
}

// ---------------------------------------------------------------------------
// Styled Word Wrap
// ---------------------------------------------------------------------------

/// Wraps a styled `Line` (multi-span) into rows fitting within `max_width`.
/// Preserves the style of each span across line breaks.
fn styled_word_wrap_line(line: &Line<'_>, max_width: usize) -> Vec<Vec<Span<'static>>> {
    if max_width == 0 {
        return vec![line
            .spans
            .iter()
            .map(|s| Span::styled(s.content.to_string(), s.style))
            .collect()];
    }

    // Flatten spans into (char, Style) pairs for uniform processing
    let mut chars_with_style: Vec<(char, Style)> = Vec::new();
    for span in &line.spans {
        for c in span.content.chars() {
            chars_with_style.push((c, span.style));
        }
    }

    if chars_with_style.is_empty() {
        return vec![vec![]];
    }

    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current_row: Vec<Span<'static>> = Vec::new();
    let mut current_width = 0usize;
    let mut current_text = String::new();
    let mut current_style = chars_with_style[0].1;

    for &(ch, style) in &chars_with_style {
        if ch == '\n' {
            // Flush current span and start new row
            if !current_text.is_empty() {
                current_row.push(Span::styled(current_text.clone(), current_style));
                current_text.clear();
            }
            rows.push(std::mem::take(&mut current_row));
            current_width = 0;
            current_style = style;
            continue;
        }

        // Check if we need to wrap
        if current_width >= max_width {
            if !current_text.is_empty() {
                current_row.push(Span::styled(current_text.clone(), current_style));
                current_text.clear();
            }
            rows.push(std::mem::take(&mut current_row));
            current_width = 0;
        }

        // Style change within the same row
        if style != current_style && !current_text.is_empty() {
            current_row.push(Span::styled(current_text.clone(), current_style));
            current_text.clear();
        }
        current_style = style;
        current_text.push(ch);
        current_width += 1;
    }

    // Flush remaining
    if !current_text.is_empty() {
        current_row.push(Span::styled(current_text, current_style));
    }
    if !current_row.is_empty() {
        rows.push(current_row);
    }

    if rows.is_empty() {
        rows.push(vec![]);
    }
    rows
}

// ---------------------------------------------------------------------------
// Word Wrap
// ---------------------------------------------------------------------------

/// Wrap input text into byte ranges per visible row without collapsing spaces.
/// Newlines force row breaks and are not included in row ranges.
fn wrap_input_rows(text: &str, max_width: usize, tab_width: usize) -> Vec<(usize, usize)> {
    if text.is_empty() {
        return vec![(0, 0)];
    }

    let mut rows = Vec::new();
    let mut row_start = 0usize;
    let row_width = max_width.max(1);

    while row_start < text.len() {
        let mut row_end = row_start;
        let mut row_display = 0usize;
        let mut consumed_newline = false;

        for (rel_idx, ch) in text[row_start..].char_indices() {
            let byte_idx = row_start + rel_idx;

            if ch == '\n' {
                consumed_newline = true;
                row_end = byte_idx;
                break;
            }

            let ch_width = if ch == '\t' {
                let tab_width = tab_width.max(1);
                tab_width - (row_display % tab_width)
            } else {
                crate::display::char_display_width(ch)
            };

            if row_end > row_start && row_display + ch_width > row_width {
                break;
            }

            row_display += ch_width;
            row_end = byte_idx + ch.len_utf8();
        }

        if row_end == row_start {
            match text[row_start..].chars().next() {
                Some('\n') => {
                    rows.push((row_start, row_start));
                    row_start += 1;
                    continue;
                }
                Some(ch) => {
                    // Always make progress even for very narrow widths / wide chars.
                    row_end = row_start + ch.len_utf8();
                }
                None => break,
            }
        }

        rows.push((row_start, row_end));
        row_start = if consumed_newline {
            row_end + 1
        } else {
            row_end
        };
    }

    if text.ends_with('\n') {
        rows.push((text.len(), text.len()));
    }

    if rows.is_empty() {
        rows.push((0, 0));
    }
    rows
}

/// Map an input cursor byte offset to wrapped (row, display_col).
fn input_cursor_row_col(
    text: &str,
    cursor_byte: usize,
    max_width: usize,
    tab_width: usize,
) -> (usize, usize) {
    let rows = wrap_input_rows(text, max_width, tab_width);
    let safe_cursor = cursor_byte.min(text.len());

    for (row_idx, (start, end)) in rows.iter().enumerate() {
        if safe_cursor <= *end {
            let col = crate::display::display_width(&text[*start..safe_cursor], tab_width);
            return (row_idx, col);
        }
    }

    let (start, end) = rows.last().copied().unwrap_or((0, 0));
    let col = crate::display::display_width(&text[start..end], tab_width);
    (rows.len().saturating_sub(1), col)
}

fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current_line = String::new();
        let mut current_width = 0usize;

        for word in paragraph.split_whitespace() {
            let word_width = word.chars().count();
            if current_width == 0 {
                // First word on line
                if word_width > max_width {
                    // Break long word
                    let mut chars = word.chars();
                    while current_width < word_width {
                        let remaining = max_width.saturating_sub(current_width);
                        let chunk: String = chars.by_ref().take(remaining).collect();
                        if chunk.is_empty() {
                            break;
                        }
                        current_line.push_str(&chunk);
                        current_width += chunk.chars().count();
                        if current_width >= max_width {
                            lines.push(std::mem::take(&mut current_line));
                            current_width = 0;
                        }
                    }
                } else {
                    current_line.push_str(word);
                    current_width = word_width;
                }
            } else if current_width + 1 + word_width <= max_width {
                current_line.push(' ');
                current_line.push_str(word);
                current_width += 1 + word_width;
            } else {
                lines.push(std::mem::take(&mut current_line));
                current_line = word.to_string();
                current_width = word_width;
            }
        }
        lines.push(current_line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{input_cursor_row_col, wrap_input_rows};

    #[test]
    fn wrap_input_rows_preserves_trailing_space() {
        let input = "abc ";
        let rows = wrap_input_rows(input, 20, 4);
        assert_eq!(rows, vec![(0, 4)]);
        assert_eq!(&input[rows[0].0..rows[0].1], "abc ");
    }

    #[test]
    fn cursor_stays_on_same_row_after_trailing_space() {
        let input = "abc ";
        let (row, col) = input_cursor_row_col(input, input.len(), 20, 4);
        assert_eq!(row, 0);
        assert_eq!(col, 4);
    }

    #[test]
    fn cursor_moves_to_next_row_after_newline() {
        let input = "abc\n";
        let (row, col) = input_cursor_row_col(input, input.len(), 20, 4);
        assert_eq!(row, 1);
        assert_eq!(col, 0);
    }
}

fn center_text(text: &str, width: usize) -> String {
    let text_len = text.chars().count();
    if text_len >= width {
        return text.to_string();
    }
    let padding = (width - text_len) / 2;
    format!(
        "{}{}{}",
        " ".repeat(padding),
        text,
        " ".repeat(width - padding - text_len)
    )
}
