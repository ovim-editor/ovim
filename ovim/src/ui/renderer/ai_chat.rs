use crate::editor::Editor;
use crate::syntax::Theme;
use ovim_core::ai::chat_types::{ChatFocus, ChatMessage, ChatRole, ToolCallInfo, ToolSummaryKind};
use ovim_core::editor::ai_chat_input::{
    chat_input_cursor_row_col, chat_input_visible_start, wrap_chat_input_rows, ChatInputRow,
};
use ovim_core::editor::QueuedChatInputKind;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthChar;

// ---------------------------------------------------------------------------
// Colors (pub(crate) so conversation_tree can reuse them)
// ---------------------------------------------------------------------------

pub(crate) const BG_PANEL: Color = Color::Reset;
const BG_INPUT: Color = Color::Rgb(28, 33, 42);

const ACCENT_USER: Color = Color::Rgb(98, 176, 255);
const ACCENT_ASSISTANT_EDIT: Color = Color::Rgb(132, 209, 149);
const ACCENT_ASSISTANT_QUERY: Color = Color::Rgb(120, 165, 235);
const ACCENT_ERROR: Color = Color::Rgb(255, 107, 107);
const ACCENT_THINKING: Color = Color::Rgb(166, 152, 208);
const ACCENT_SELECTED: Color = Color::Rgb(255, 216, 107);

pub(crate) const TEXT_DIM: Color = Color::Rgb(128, 140, 155);
const TEXT_THINKING: Color = Color::Rgb(100, 112, 130);
pub(crate) const TEXT_NORMAL: Color = Color::Rgb(200, 208, 220);
const BG_USER_ROW: Color = Color::Rgb(25, 41, 64);
const BG_ASSISTANT_EDIT_ROW: Color = Color::Rgb(24, 49, 38);
const BG_ASSISTANT_QUERY_ROW: Color = Color::Rgb(30, 41, 63);
const BG_THINKING_ROW: Color = Color::Rgb(34, 38, 50);
const BG_ERROR_ROW: Color = Color::Rgb(66, 30, 33);
const BG_SELECTED_ROW: Color = Color::Rgb(48, 56, 74);
const BG_USER_LABEL: Color = Color::Rgb(37, 60, 91);
const BG_ASSISTANT_EDIT_LABEL: Color = Color::Rgb(38, 67, 52);
const BG_ASSISTANT_QUERY_LABEL: Color = Color::Rgb(42, 56, 84);
const BG_THINKING_LABEL: Color = Color::Rgb(50, 55, 70);
const BG_ERROR_LABEL: Color = Color::Rgb(86, 39, 42);
const BG_SELECTED_LABEL: Color = Color::Rgb(70, 80, 103);
const TOOL_READ: Color = Color::Rgb(112, 175, 255);
const TOOL_NAV: Color = Color::Rgb(126, 211, 160);
const TOOL_MUT: Color = Color::Rgb(151, 215, 110);
const TOOL_SEARCH: Color = Color::Rgb(224, 193, 110);
const TOOL_DIAG: Color = Color::Rgb(255, 173, 102);
const TOOL_ERROR: Color = Color::Rgb(255, 107, 107);
const TOOL_BG_READ: Color = Color::Rgb(28, 46, 72);
const TOOL_BG_NAV: Color = Color::Rgb(26, 50, 39);
const TOOL_BG_MUT: Color = Color::Rgb(30, 55, 32);
const TOOL_BG_SEARCH: Color = Color::Rgb(58, 46, 27);
const TOOL_BG_DIAG: Color = Color::Rgb(62, 41, 26);
const TOOL_BG_ERROR: Color = Color::Rgb(67, 28, 31);
const TOOL_BG_OTHER: Color = Color::Rgb(36, 40, 50);

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Split content area into buffer (left) and chat panel (right).
pub fn compute_chat_split(
    content_area: Rect,
    allow_edits: bool,
    preferred_percent: Option<u16>,
) -> (Rect, Rect) {
    let total = content_area.width;
    let chat_pct = preferred_percent
        .unwrap_or(if allow_edits { 40 } else { 35 })
        .clamp(1, 99);
    let min_chat = 30u16;
    let min_buffer = 40u16;

    let chat_width = ((u32::from(total) * u32::from(chat_pct) / 100) as u16)
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
    quarter.clamp(20, 36)
}

/// Render the full chat panel.
pub fn render_chat_panel(frame: &mut Frame, editor: &mut Editor, chat_area: Rect, theme: &Theme) {
    editor.render_cache.ai_chat_history_area = None;
    editor.render_cache.ai_chat_image_thumbnails.clear();
    editor.render_cache.ai_chat_branch_hitboxes.clear();
    editor.render_cache.ai_chat_yolo_hitbox = None;
    editor
        .render_cache
        .ai_chat_slash_completion_hitboxes
        .clear();
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

    render_chat_header(frame, editor, main_area);
    let main_area = Rect {
        x: main_area.x,
        y: main_area.y.saturating_add(1),
        width: main_area.width,
        height: main_area.height.saturating_sub(1),
    };

    // Layout: [message_history | input_bar(dynamic)]. `/model` opens a popup.
    let input_content_width = (main_area.width as usize).saturating_sub(2 + 3 + 2); // "│ " + prompt + " │"
    let input_rows = wrap_chat_input_rows(
        editor.ai_chat_input(),
        input_content_width.max(1),
        editor.options.tab_width,
    );
    let (cursor_row, _) = chat_input_cursor_row_col(
        editor.ai_chat_input(),
        editor.ai_chat_input_cursor(),
        &input_rows,
        editor.options.tab_width,
    );
    let max_input_rows = 5usize.min(main_area.height.saturating_sub(1) as usize);
    let visible_input_rows = input_rows.len().min(max_input_rows).max(1);
    let input_visible_start =
        chat_input_visible_start(input_rows.len(), cursor_row, visible_input_rows);
    let input_height = (1 + visible_input_rows) as u16;
    let gallery_paths = editor
        .ai_chat_pending_images()
        .iter()
        .map(|image| image.path.clone())
        .collect::<Vec<_>>();
    let gallery_height = chat_image_gallery_height(editor, main_area, &gallery_paths);
    let messages_height = main_area
        .height
        .saturating_sub(input_height)
        .saturating_sub(gallery_height);

    let messages_area = Rect {
        x: main_area.x,
        y: main_area.y,
        width: main_area.width,
        height: messages_height,
    };
    let input_area = Rect {
        x: main_area.x,
        y: main_area.y + messages_height + gallery_height,
        width: main_area.width,
        height: input_height,
    };
    if messages_area.height > 0 {
        render_message_history(frame, editor, messages_area, theme);
    } else {
        editor.render_cache.ai_chat_history_area = None;
    }
    if gallery_height > 0 {
        render_chat_image_gallery(
            frame,
            editor,
            Rect::new(
                main_area.x,
                main_area.y + messages_height,
                main_area.width,
                gallery_height,
            ),
            &gallery_paths,
        );
    }
    render_text_input(frame, editor, input_area, &input_rows, input_visible_start);
    render_slash_completion(frame, editor, input_area, messages_area.y);
    if editor.ai_chat_focus() == ChatFocus::ModelSelector {
        render_model_picker(frame, editor, main_area);
    }
}

fn render_slash_completion(
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

    editor.render_cache.ai_chat_slash_completion_hitboxes = (0..visible_count)
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
    let main_area = Rect {
        x: main_area.x,
        y: main_area.y.saturating_add(1),
        width: main_area.width,
        height: main_area.height.saturating_sub(1),
    };

    let content_width = (main_area.width as usize).saturating_sub(2 + 3 + 2); // "│ " + prompt + " │"
    let input = editor.ai_chat_input();
    let tab_width = editor.options.tab_width;
    let wrapped_rows = wrap_chat_input_rows(input, content_width.max(1), tab_width);
    let (cursor_line, col) = chat_input_cursor_row_col(
        input,
        editor.ai_chat_input_cursor(),
        &wrapped_rows,
        tab_width,
    );
    let max_input_rows = 5usize.min(main_area.height.saturating_sub(1) as usize);
    if max_input_rows == 0 {
        return None;
    }
    let visible_input_rows = wrapped_rows.len().min(max_input_rows).max(1);
    let visible_start =
        chat_input_visible_start(wrapped_rows.len(), cursor_line, visible_input_rows);
    let input_height = (1 + visible_input_rows) as u16;
    let gallery_paths = editor
        .ai_chat_pending_images()
        .iter()
        .map(|image| image.path.clone())
        .collect::<Vec<_>>();
    let gallery_height = chat_image_gallery_height(editor, main_area, &gallery_paths);
    let messages_height = main_area
        .height
        .saturating_sub(input_height)
        .saturating_sub(gallery_height);
    let input_y = main_area.y + messages_height + gallery_height;

    // First line has "│ >> " prefix (border + space + prompt = 5), continuation lines same width
    let prefix_len = 5u16;
    let x = main_area
        .x
        .saturating_add(prefix_len)
        .saturating_add(col as u16)
        .min(main_area.x + main_area.width.saturating_sub(1));
    // +1 for the top border row, then offset by cursor_line
    let y = input_y + 1 + cursor_line.saturating_sub(visible_start) as u16;

    Some((x, y))
}

fn render_chat_header(frame: &mut Frame, editor: &mut Editor, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let enabled = editor.ai_chat_yolo_mode();
    let label = if enabled { " YOLO ON " } else { " YOLO OFF " };
    let width = label.chars().count().min(area.width as usize) as u16;
    let x = area.right().saturating_sub(width);
    let style = if enabled {
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
    frame.render_widget(
        Paragraph::new(Span::styled(label, style)).alignment(Alignment::Right),
        Rect::new(area.x, area.y, area.width, 1),
    );
    editor.render_cache.ai_chat_yolo_hitbox = Some(crate::key_convert::convert_ratatui_rect(
        Rect::new(x, area.y, width, 1),
    ));
}

fn chat_image_gallery_height(editor: &Editor, area: Rect, paths: &[std::path::PathBuf]) -> u16 {
    if editor.render_cache.terminal_image_support && !paths.is_empty() && area.height >= 10 {
        6
    } else {
        0
    }
}

fn render_chat_image_gallery(
    frame: &mut Frame,
    editor: &mut Editor,
    area: Rect,
    paths: &[std::path::PathBuf],
) {
    const THUMB_WIDTH: u16 = 14;
    let capacity = (area.width / THUMB_WIDTH).max(1) as usize;
    let first = paths.len().saturating_sub(capacity);
    for (index, path) in paths[first..].iter().enumerate() {
        let outer = Rect::new(
            area.x + index as u16 * THUMB_WIDTH,
            area.y,
            THUMB_WIDTH.min(
                area.right()
                    .saturating_sub(area.x + index as u16 * THUMB_WIDTH),
            ),
            area.height,
        );
        if outer.width < 4 {
            break;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image");
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(82, 139, 255)))
                .title(truncate_with_ellipsis(
                    name,
                    outer.width.saturating_sub(2) as usize,
                )),
            outer,
        );
        let image_area = Rect::new(
            outer.x + 1,
            outer.y + 1,
            outer.width.saturating_sub(2),
            outer.height.saturating_sub(2),
        );
        editor.render_cache.ai_chat_image_thumbnails.push((
            crate::key_convert::convert_ratatui_rect(image_area),
            path.clone(),
        ));
    }
}

// ---------------------------------------------------------------------------
// Message History
// ---------------------------------------------------------------------------

fn render_message_history(frame: &mut Frame, editor: &mut Editor, area: Rect, theme: &Theme) {
    editor.render_cache.ai_chat_history_area = Some(crate::key_convert::convert_ratatui_rect(area));
    editor.render_cache.ai_chat_last_queued_row_spans.clear();
    let messages = editor.ai_chat_messages();
    if messages.is_empty() {
        editor.render_cache.ai_chat_last_total_rows = 0;
        editor.render_cache.ai_chat_last_visible_start_row = 0;
        editor.render_cache.ai_chat_last_visible_end_row = 0;
        editor.render_cache.ai_chat_last_message_row_spans.clear();
        editor.render_cache.ai_chat_rendered_text_rows.clear();
        editor.render_cache.ai_chat_text_selection = None;

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
    let selected_idx = editor.ai_chat_history_selected_index();
    let panel_width = area.width as usize;

    // Get node IDs for active branch (parallel to messages)
    let node_ids = editor
        .conversation()
        .map(|c| c.node_ids_for_active_branch().to_vec())
        .unwrap_or_default();

    // Render messages bottom-up with scroll
    let mut rendered_lines: Vec<(Line, bool)> = Vec::new(); // (line, is_bubble_border)
    let mut message_row_spans: Vec<(usize, usize)> = Vec::with_capacity(messages.len());
    let mut branch_controls = Vec::new();
    let mut inline_images = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        let is_selected = focus == ChatFocus::MessageHistory && Some(idx) == selected_idx;

        if is_hidden_tool_only_assistant(msg) {
            let pos = rendered_lines.len();
            message_row_spans.push((pos, pos));
            continue;
        }

        if msg.role == ChatRole::Tool {
            let msg_row_start = rendered_lines.len();
            let tool_call_id = msg.tool_call_id.as_deref();
            let (kind, label) = msg
                .tool_call_id
                .as_deref()
                .and_then(|id| editor.ai_chat_tool_event_summary_parts(id))
                .map(|(k, l)| (k, l.to_string()))
                .unwrap_or_else(|| fallback_tool_summary(msg));
            let expanded = tool_call_id.is_some_and(|id| editor.ai_chat_is_tool_event_expanded(id));
            rendered_lines.push((
                render_tool_event_row(panel_width, &label, kind, is_selected, false, expanded),
                false,
            ));
            if expanded {
                let call = tool_call_id.and_then(|id| editor.ai_chat_tool_event_call(id));
                for line in render_tool_event_details(panel_width, call, &msg.content) {
                    rendered_lines.push((line, false));
                }
            }
            let msg_row_end = rendered_lines.len();
            message_row_spans.push((msg_row_start, msg_row_end));
            continue;
        }

        // Look up NodeId for thinking expansion and child count
        let node_id = node_ids.get(idx).copied();
        let is_thinking_expanded = node_id
            .map(|id| editor.ai_chat_is_thinking_expanded(id))
            .unwrap_or(false);
        let child_count = node_id
            .and_then(|id| editor.conversation().map(|c| c.child_count(id)))
            .unwrap_or(0);
        let branch_navigation = node_id.and_then(|id| {
            editor
                .conversation()
                .and_then(|conversation| conversation.sibling_navigation(id))
        });

        let bubble = render_chat_bubble(
            msg,
            panel_width,
            is_selected,
            allow_edits,
            is_thinking_expanded,
            child_count,
            branch_navigation.map(|(position, count, _, _)| (position, count)),
            theme,
            editor.render_cache.terminal_image_support,
        );
        let msg_row_start = rendered_lines.len();
        if let Some((position, count, previous, next)) = branch_navigation {
            branch_controls.push(BranchRenderControl {
                row: msg_row_start,
                position,
                count,
                previous,
                next,
            });
        }
        for image in bubble.images {
            inline_images.push(HistoryImagePlacement {
                row: msg_row_start + image.row,
                x: image.x,
                width: image.width,
                height: image.height,
                path: image.path,
            });
        }
        for line in bubble.lines {
            rendered_lines.push((line, false));
        }
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
                images: vec![],
                tool_calls: vec![],
                tool_call_id: None,
                provider_state: vec![],
            };
            let bubble = render_chat_bubble(
                &streaming_thinking_msg,
                panel_width,
                false,
                allow_edits,
                true,
                0,
                None,
                theme,
                false,
            );
            for line in bubble.lines {
                rendered_lines.push((line, false));
            }
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
                images: vec![],
                tool_calls: vec![],
                tool_call_id: None,
                provider_state: vec![],
            };
            let bubble = render_chat_bubble(
                &streaming_msg,
                panel_width,
                false,
                allow_edits,
                false,
                0,
                None,
                theme,
                false,
            );
            for line in bubble.lines {
                rendered_lines.push((line, false));
            }
        }
    }

    // Tool call status rows during tool execution
    if let Some(chat) = editor.ai_state.chat.as_ref() {
        if !chat.streaming_tool_calls.is_empty() {
            for tc in &chat.streaming_tool_calls {
                let (kind, label) = summarize_streaming_tool_call(tc);
                let status_text = format!("running {label}");
                rendered_lines.push((
                    render_tool_event_row(panel_width, &status_text, kind, false, true, false),
                    false,
                ));
            }
        }
    }

    // Follow-ups submitted during a run stay visible above the composer.
    let selected_queued = editor.ai_chat_history_selected_queued_id();
    let queued_inputs = editor.ai_chat_queued_inputs().cloned().collect::<Vec<_>>();
    for queued in queued_inputs {
        let row_start = rendered_lines.len();
        rendered_lines.push((
            render_queued_input_row(
                panel_width,
                queued.kind,
                &queued.content,
                queued.images.len(),
                focus == ChatFocus::MessageHistory && selected_queued == Some(queued.id),
            ),
            false,
        ));
        editor
            .render_cache
            .ai_chat_last_queued_row_spans
            .push((row_start, rendered_lines.len()));
    }

    // Progress belongs to the run, not to an assistant message. Keep it as a
    // standalone animated row after the latest visible event for the entire
    // time the agent is working.
    if editor.ai_chat_waiting() {
        rendered_lines.push((
            render_working_indicator(panel_width, editor.ai_chat_working_animation_frame()),
            false,
        ));
    }

    // Display from bottom of area. While pinned, keep viewport stable even
    // when new streaming rows are appended.
    let visible_rows = area.height as usize;
    let total = rendered_lines.len();
    editor.render_cache.ai_chat_last_total_rows = total;
    let effective_scroll = editor.ai_chat_effective_message_scroll(total, visible_rows);
    let start = total.saturating_sub(visible_rows + effective_scroll);
    let end = total.saturating_sub(effective_scroll).min(total);
    editor.render_cache.ai_chat_last_visible_start_row = start;
    editor.render_cache.ai_chat_last_visible_end_row = end;
    editor.render_cache.ai_chat_last_message_row_spans = message_row_spans;
    editor.render_cache.ai_chat_rendered_text_rows = rendered_lines
        .iter()
        .map(|(line, _)| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect();

    // Terminal graphics protocols are not clipped by the chat's logical
    // history viewport. Only enqueue a thumbnail when its entire inner image
    // rectangle belongs to the visible row window; the text-drawn box itself
    // continues to clip normally at the viewport edges.
    for image in inline_images {
        let image_end = image.row.saturating_add(image.height as usize);
        if image.row < start || image_end > end {
            continue;
        }
        editor.render_cache.ai_chat_image_thumbnails.push((
            crate::key_convert::convert_ratatui_rect(Rect {
                x: area.x.saturating_add(image.x),
                y: area.y.saturating_add((image.row - start) as u16),
                width: image.width.min(area.width.saturating_sub(image.x)),
                height: image.height,
            }),
            image.path,
        ));
    }

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
        let mut line = rendered_lines[line_idx].0.clone();
        if let Some((selection_start, selection_end)) =
            editor.ai_chat_text_selection_range(line_idx)
        {
            line = highlight_chat_selection(&line, selection_start, selection_end);
        }
        frame.render_widget(Paragraph::new(vec![line]), r);
        if let Some(control) = branch_controls
            .iter()
            .find(|control| control.row == line_idx)
        {
            let control_width = branch_control_text(control.position, control.count)
                .chars()
                .count() as u16;
            if (control_width as usize) < card_text_width(area.width as usize, "\u{258d}") {
                let x = area.x + area.width - control_width;
                let left_width = control_width / 2;
                editor.render_cache.ai_chat_branch_hitboxes.push((
                    crate::key_convert::convert_ratatui_rect(Rect {
                        x,
                        y: r.y,
                        width: left_width,
                        height: 1,
                    }),
                    control.previous,
                ));
                editor.render_cache.ai_chat_branch_hitboxes.push((
                    crate::key_convert::convert_ratatui_rect(Rect {
                        x: x + left_width,
                        y: r.y,
                        width: control_width - left_width,
                        height: 1,
                    }),
                    control.next,
                ));
            }
        }
    }
}

fn highlight_chat_selection(
    line: &Line<'_>,
    selection_start: usize,
    selection_end: usize,
) -> Line<'static> {
    let mut output = Vec::new();
    let mut display_column = 0usize;
    for span in &line.spans {
        let mut segment = String::new();
        let mut segment_selected = None;
        for character in span.content.chars() {
            let width = character.width().unwrap_or(1).max(1);
            let character_start = display_column;
            let character_end = display_column.saturating_add(width);
            display_column = character_end;
            let selected = character_end > selection_start && character_start < selection_end;
            if segment_selected.is_some_and(|current| current != selected) {
                let style = if segment_selected == Some(true) {
                    span.style.bg(Color::Rgb(74, 96, 145)).fg(Color::White)
                } else {
                    span.style
                };
                output.push(Span::styled(std::mem::take(&mut segment), style));
            }
            segment_selected = Some(selected);
            segment.push(character);
        }
        if !segment.is_empty() {
            let style = if segment_selected == Some(true) {
                span.style.bg(Color::Rgb(74, 96, 145)).fg(Color::White)
            } else {
                span.style
            };
            output.push(Span::styled(segment, style));
        }
    }
    Line::from(output)
}

fn is_hidden_tool_only_assistant(message: &ChatMessage) -> bool {
    message.role == ChatRole::Assistant
        && message.content.trim().is_empty()
        && !message.tool_calls.is_empty()
}

fn fallback_tool_summary(message: &ChatMessage) -> (ToolSummaryKind, String) {
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

fn summarize_streaming_tool_call(tool_call: &ToolCallInfo) -> (ToolSummaryKind, String) {
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

fn render_tool_event_row(
    panel_width: usize,
    label: &str,
    kind: ToolSummaryKind,
    selected: bool,
    pending: bool,
    expanded: bool,
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
    let text = format!(" {disclosure} {prefix} {label}");
    let display = compact_tool_text(&text, panel_width);
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
        " ".repeat(panel_width.saturating_sub(display.chars().count()))
    );
    Line::from(Span::styled(padded, style))
}

fn render_tool_event_details(
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
            let padding = panel_width.saturating_sub(display.chars().count());
            Line::from(Span::styled(
                format!("{display}{}", " ".repeat(padding)),
                style,
            ))
        })
        .collect()
}

fn render_queued_input_row(
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
    let padding = panel_width.saturating_sub(display.chars().count());
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

fn compact_tool_text(text: &str, max_chars: usize) -> String {
    let single_line = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('\n', " ");
    if single_line.chars().count() <= max_chars {
        return single_line;
    }
    let mut out: String = single_line
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect();
    out.push('…');
    out
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

#[derive(Clone, Copy)]
struct MessageRowStyle {
    accent: Color,
    label_fg: Color,
    label_bg: Color,
    text_fg: Color,
    body_bg: Color,
}

#[derive(Clone, Copy)]
struct BranchRenderControl {
    row: usize,
    position: usize,
    count: usize,
    previous: ovim_core::ai::chat_types::NodeId,
    next: ovim_core::ai::chat_types::NodeId,
}

struct BubbleImagePlacement {
    /// First inner image row relative to the message bubble.
    row: usize,
    /// Inner image column relative to the history panel.
    x: u16,
    width: u16,
    height: u16,
    path: std::path::PathBuf,
}

struct HistoryImagePlacement {
    /// First inner image row in absolute rendered-history coordinates.
    row: usize,
    x: u16,
    width: u16,
    height: u16,
    path: std::path::PathBuf,
}

struct ChatBubbleRender {
    lines: Vec<Line<'static>>,
    images: Vec<BubbleImagePlacement>,
}

fn message_row_style(role: ChatRole, allow_edits: bool, selected: bool) -> MessageRowStyle {
    let mut style = match role {
        ChatRole::User => MessageRowStyle {
            accent: ACCENT_USER,
            label_fg: Color::White,
            label_bg: BG_USER_LABEL,
            text_fg: TEXT_NORMAL,
            body_bg: BG_USER_ROW,
        },
        ChatRole::Assistant => {
            if allow_edits {
                MessageRowStyle {
                    accent: ACCENT_ASSISTANT_EDIT,
                    label_fg: Color::White,
                    label_bg: BG_ASSISTANT_EDIT_LABEL,
                    text_fg: TEXT_NORMAL,
                    body_bg: BG_ASSISTANT_EDIT_ROW,
                }
            } else {
                MessageRowStyle {
                    accent: ACCENT_ASSISTANT_QUERY,
                    label_fg: Color::White,
                    label_bg: BG_ASSISTANT_QUERY_LABEL,
                    text_fg: TEXT_NORMAL,
                    body_bg: BG_ASSISTANT_QUERY_ROW,
                }
            }
        }
        ChatRole::Thinking => MessageRowStyle {
            accent: ACCENT_THINKING,
            label_fg: Color::Rgb(222, 216, 245),
            label_bg: BG_THINKING_LABEL,
            text_fg: TEXT_THINKING,
            body_bg: BG_THINKING_ROW,
        },
        ChatRole::Error => MessageRowStyle {
            accent: ACCENT_ERROR,
            label_fg: Color::White,
            label_bg: BG_ERROR_LABEL,
            text_fg: Color::Rgb(255, 198, 198),
            body_bg: BG_ERROR_ROW,
        },
        ChatRole::Tool => MessageRowStyle {
            accent: ACCENT_ASSISTANT_QUERY,
            label_fg: Color::White,
            label_bg: BG_ASSISTANT_QUERY_LABEL,
            text_fg: TEXT_NORMAL,
            body_bg: BG_ASSISTANT_QUERY_ROW,
        },
    };

    if selected {
        style.accent = ACCENT_SELECTED;
        style.label_bg = BG_SELECTED_LABEL;
        style.body_bg = BG_SELECTED_ROW;
    }

    style
}

fn card_text_width(panel_width: usize, accent_glyph: &str) -> usize {
    panel_width
        .saturating_sub(accent_glyph.chars().count() + 1)
        .max(1)
}

fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "\u{2026}".to_string();
    }
    let mut out: String = text.chars().take(max_chars - 1).collect();
    out.push('\u{2026}');
    out
}

fn render_card_text_line(
    panel_width: usize,
    accent_glyph: &str,
    accent_color: Color,
    row_bg: Color,
    text: &str,
    text_style: Style,
) -> Line<'static> {
    let width = card_text_width(panel_width, accent_glyph);
    let display = truncate_with_ellipsis(text, width);
    let padding = width.saturating_sub(display.chars().count());
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

fn branch_control_text(position: usize, count: usize) -> String {
    format!("[‹ {}/{} ›]", position + 1, count)
}

fn render_card_header_line(
    panel_width: usize,
    accent_glyph: &str,
    row_style: MessageRowStyle,
    label: &str,
    branch_position: Option<(usize, usize)>,
) -> Line<'static> {
    let width = card_text_width(panel_width, accent_glyph);
    let control = branch_position
        .map(|(position, count)| branch_control_text(position, count))
        .filter(|text| text.chars().count() < width);
    let control_width = control
        .as_ref()
        .map(|text| text.chars().count())
        .unwrap_or(0);
    let label_width = width.saturating_sub(control_width + usize::from(control.is_some()));
    let display_label = truncate_with_ellipsis(label, label_width);
    let gap = width.saturating_sub(display_label.chars().count() + control_width);
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

fn render_card_styled_line(
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
        let span_width = content.chars().count();
        if span_width == 0 {
            continue;
        }
        spans.push(Span::styled(content, span.style.bg(row_bg)));
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

fn render_chat_bubble(
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
        // Markdown-rendered content for assistant messages
        let md_elements = super::markdown::parse_markdown(&message.content);
        let md_lines = super::markdown::render_markdown(&md_elements, inner_width, Some(theme));
        for md_line in &md_lines {
            let content_spans = styled_word_wrap_line(md_line, inner_width);
            for row_spans in content_spans {
                lines.push(render_card_styled_line(
                    panel_width,
                    accent_glyph,
                    row_style.accent,
                    row_style.body_bg,
                    row_spans,
                ));
            }
        }
        if md_lines.is_empty() {
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
            message.content.clone()
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

    ChatBubbleRender { lines, images }
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
            "─".repeat(inner.saturating_sub(title.chars().count()))
        );
    }
    if row + 1 == height {
        return format!("╰{}╯", "─".repeat(inner));
    }
    format!("│{}│", " ".repeat(inner))
}

// ---------------------------------------------------------------------------
// Text Input
// ---------------------------------------------------------------------------

fn render_text_input(
    frame: &mut Frame,
    editor: &mut Editor,
    area: Rect,
    wrapped_rows: &[ChatInputRow],
    visible_start: usize,
) {
    editor.render_cache.ai_chat_input_area = Some(crate::key_convert::convert_ratatui_rect(area));
    editor.render_cache.ai_chat_input_rows.clear();
    if area.height == 0 || area.width < 4 {
        return;
    }

    let focus = editor.ai_chat_focus();
    let input = editor.ai_chat_input().to_string();
    let allow_edits = editor.ai_chat_allow_edits();

    let border_color = if focus == ChatFocus::TextInput {
        Color::Rgb(82, 139, 255)
    } else {
        Color::Rgb(60, 66, 80)
    };

    let border_style = Style::default().fg(border_color).bg(BG_PANEL);
    let w = area.width as usize;

    // Top border of input box
    let image_names = editor
        .ai_chat_pending_images()
        .iter()
        .map(|image| image.file_name())
        .collect::<Vec<_>>();
    let top = if image_names.is_empty() {
        format!("╭{}╮", "─".repeat(w.saturating_sub(2)))
    } else {
        let title = truncate_with_ellipsis(
            &format!(" 📎 {} ", image_names.join(", ")),
            w.saturating_sub(4),
        );
        let fill = w.saturating_sub(2 + title.chars().count());
        format!("╭{title}{}╮", "─".repeat(fill))
    };
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
    editor.render_cache.ai_chat_input_content_width = content_width;

    let show_active_hint = input.is_empty() && editor.ai_chat_round_active();
    let input_fg = if show_active_hint {
        TEXT_DIM
    } else {
        TEXT_NORMAL
    };
    let input_style = Style::default().fg(input_fg).bg(BG_INPUT);

    let visible_rows = wrapped_rows
        .iter()
        .skip(visible_start)
        .take(content_rows)
        .copied()
        .collect::<Vec<_>>();

    for (row_idx, row) in visible_rows.iter().enumerate() {
        let display = if show_active_hint {
            truncate_with_ellipsis(
                "Enter steers after tool · Tab queues next round",
                content_width,
            )
        } else {
            input[row.visible_start..row.end].to_string()
        };
        let display_width = crate::display::display_width(&display, editor.options.tab_width);
        let padding = content_width.saturating_sub(display_width);

        let absolute_row = visible_start + row_idx;
        let row_prefix = if absolute_row == 0 { prompt } else { "   " };

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
        editor.render_cache.ai_chat_input_rows.push((
            crate::key_convert::convert_ratatui_rect(Rect {
                x: area.x + prefix_total as u16,
                y: area.y + 1 + row_idx as u16,
                width: content_width as u16,
                height: 1,
            }),
            row.start,
            row.visible_start,
            row.end,
        ));
    }

    // Fill remaining content rows with empty bordered lines
    for row_idx in visible_rows.len()..content_rows {
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
        let label_w = label.chars().count();
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
        let label = format!(" {}:{} ", name, model_short);
        let label_w = label.chars().count();
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
    } else if pending_tool_approval {
        " [Enter allow] [C-a allow chat] [Esc deny] "
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

fn render_model_picker(frame: &mut Frame, editor: &Editor, area: Rect) {
    if area.height < 5 || area.width < 24 {
        return;
    }
    let profile_names = editor.ai_profile_names_sorted();
    let active_profile = editor.ai_chat_effective_profile();
    let height = (profile_names.len() as u16 + 2).min(area.height.saturating_sub(2));
    let width = area.width.saturating_sub(4).min(54);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    let items = profile_names.iter().filter_map(|name| {
        let profile = editor.ai_state.config.resolve_profile(name)?;
        let selected = name == &active_profile;
        let marker = if selected { "›" } else { " " };
        let style = if selected {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT_NORMAL)
        };
        Some(ListItem::new(format!("{marker} {name}  {}", profile.model)).style(style))
    });
    frame.render_widget(Clear, popup);
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(" Model · ↑/↓ choose · Enter close ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(82, 139, 255))),
        ),
        popup,
    );
}

// ---------------------------------------------------------------------------
// Waiting Indicator
// ---------------------------------------------------------------------------

fn render_working_indicator(width: usize, frame: usize) -> Line<'static> {
    const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    let text = format!("  {} Working", FRAMES[frame % FRAMES.len()]);
    let mut spans = vec![Span::styled(
        text.clone(),
        Style::default()
            .fg(Color::Rgb(120, 140, 180))
            .bg(BG_PANEL)
            .add_modifier(Modifier::DIM),
    )];
    let used = text.chars().count();
    if used < width {
        spans.push(Span::styled(
            " ".repeat(width - used),
            Style::default().bg(BG_PANEL),
        ));
    }
    Line::from(spans)
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
    use super::{
        chat_cursor_info, compute_chat_split, highlight_chat_selection,
        is_hidden_tool_only_assistant, render_queued_input_row, render_tool_event_details,
    };
    use ovim_core::ai::chat_types::{ChatMessage, ChatRole, ImageAttachment, ToolCallInfo};
    use ovim_core::editor::ai_chat_input::{chat_input_cursor_row_col, wrap_chat_input_rows};
    use ovim_core::editor::{Editor, QueuedChatInputKind};
    use ratatui::{
        backend::TestBackend,
        layout::Rect,
        style::{Color, Modifier, Style},
        text::{Line, Span},
        Terminal,
    };

    fn append_user_image_message(editor: &mut Editor, path: &str, content: &str) {
        let chat = editor.ai_state.chat.as_ref().unwrap();
        let key = (chat.origin_buffer_id, chat.opts.name.clone());
        editor
            .ai_state
            .conversations
            .get_mut(&key)
            .unwrap()
            .append_user_message_with_images(
                content.into(),
                vec![ImageAttachment {
                    path: std::path::PathBuf::from(path),
                    mime_type: "image/png".into(),
                    data: vec![1, 2, 3],
                }],
            );
    }

    #[test]
    fn chat_split_uses_user_width_and_preserves_minimum_buffer() {
        let area = Rect::new(0, 0, 100, 24);
        let (buffer, chat) = compute_chat_split(area, true, Some(55));
        assert_eq!((buffer.width, chat.width), (45, 55));

        let (buffer, chat) = compute_chat_split(area, true, Some(90));
        assert_eq!((buffer.width, chat.width), (40, 60));
    }

    #[test]
    fn chat_split_keeps_context_sensitive_default_without_user_resize() {
        let area = Rect::new(0, 0, 100, 24);
        assert_eq!(compute_chat_split(area, true, None).1.width, 40);
        assert_eq!(compute_chat_split(area, false, None).1.width, 35);
    }

    #[test]
    fn partial_slash_command_renders_completion_popup_and_hitboxes() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
            .unwrap();
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.input = "/".into();
        chat.input_cursor = 1;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

        terminal
            .draw(|frame| {
                super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
            })
            .unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("/clear"));
        assert!(rendered.contains("/model [profile]"));
        assert_eq!(
            editor.render_cache.ai_chat_slash_completion_hitboxes.len(),
            4
        );
    }

    #[test]
    fn wrap_input_rows_preserves_trailing_space() {
        let input = "abc ";
        let rows = wrap_chat_input_rows(input, 20, 4);
        assert_eq!(&input[rows[0].start..rows[0].end], "abc ");
    }

    #[test]
    fn cursor_stays_on_same_row_after_trailing_space() {
        let input = "abc ";
        let rows = wrap_chat_input_rows(input, 20, 4);
        let (row, col) = chat_input_cursor_row_col(input, input.len(), &rows, 4);
        assert_eq!(row, 0);
        assert_eq!(col, 4);
    }

    #[test]
    fn cursor_moves_to_next_row_after_newline() {
        let input = "abc\n";
        let rows = wrap_chat_input_rows(input, 20, 4);
        let (row, col) = chat_input_cursor_row_col(input, input.len(), &rows, 4);
        assert_eq!(row, 1);
        assert_eq!(col, 0);
    }

    #[test]
    fn composer_cursor_stays_inside_panel_after_height_cap() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
            .unwrap();
        let input = (0..20)
            .map(|index| format!("word{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let chat = editor.ai_state.chat.as_mut().unwrap();
        chat.input = input;
        chat.input_cursor = chat.input.len();
        let panel = Rect {
            x: 40,
            y: 2,
            width: 42,
            height: 18,
        };

        let (x, y) = chat_cursor_info(&editor, panel).unwrap();

        assert!(x >= panel.x && x < panel.x + panel.width);
        assert!(y >= panel.y && y < panel.y + panel.height);
        assert_eq!(y, panel.y + panel.height - 1);
    }

    #[test]
    fn supported_terminal_reserves_clickable_thumbnail_strip() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
            .unwrap();
        editor.render_cache.terminal_image_support = true;
        editor.ai_state.chat.as_mut().unwrap().pending_images.push(
            ovim_core::ai::chat_types::ImageAttachment {
                path: std::path::PathBuf::from("/tmp/preview.png"),
                mime_type: "image/png".into(),
                data: vec![1, 2, 3],
            },
        );
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

        terminal
            .draw(|frame| {
                super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
            })
            .unwrap();

        assert_eq!(editor.render_cache.ai_chat_image_thumbnails.len(), 1);
        assert_eq!(
            editor.render_cache.ai_chat_image_thumbnails[0].1,
            std::path::PathBuf::from("/tmp/preview.png")
        );
    }

    #[test]
    fn chat_header_renders_clickable_yolo_state_at_top_right() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
            .unwrap();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

        terminal
            .draw(|frame| {
                super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
            })
            .unwrap();
        let header = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .take(80)
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(header.contains("YOLO OFF"), "{header}");
        let hitbox = editor.render_cache.ai_chat_yolo_hitbox.unwrap();
        assert_eq!(hitbox.y, 0);
        assert_eq!(hitbox.x + hitbox.width, 80);

        assert!(editor.set_ai_chat_yolo_mode(true));
        terminal
            .draw(|frame| {
                super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
            })
            .unwrap();
        let header = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .take(80)
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(header.contains("YOLO ON"), "{header}");
    }

    #[test]
    fn sent_image_thumbnail_is_positioned_inside_its_visible_message() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
            .unwrap();
        editor.render_cache.terminal_image_support = true;
        append_user_image_message(&mut editor, "/tmp/sent-preview.png", "inspect this");

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());
        terminal
            .draw(|frame| {
                super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
            })
            .unwrap();

        assert_eq!(editor.render_cache.ai_chat_image_thumbnails.len(), 1);
        let (thumbnail, path) = &editor.render_cache.ai_chat_image_thumbnails[0];
        assert_eq!(path, &std::path::PathBuf::from("/tmp/sent-preview.png"));
        assert!(thumbnail.y < editor.render_cache.ai_chat_input_area.unwrap().y);
        assert!(editor.ai_chat_pending_images().is_empty());
    }

    #[test]
    fn offscreen_message_image_does_not_enqueue_terminal_rendering() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
            .unwrap();
        editor.render_cache.terminal_image_support = true;
        append_user_image_message(&mut editor, "/tmp/offscreen.png", "old image");
        let chat = editor.ai_state.chat.as_ref().unwrap();
        let key = (chat.origin_buffer_id, chat.opts.name.clone());
        let conversation = editor.ai_state.conversations.get_mut(&key).unwrap();
        for index in 0..16 {
            conversation
                .append_assistant_message(format!("later response {index}"), "model".into());
        }

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());
        terminal
            .draw(|frame| {
                super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
            })
            .unwrap();

        assert!(editor.render_cache.ai_chat_image_thumbnails.is_empty());
    }

    #[test]
    fn forked_message_renders_clickable_sibling_control() {
        let mut editor = Editor::default();
        editor
            .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
            .unwrap();
        let (main_user, fork_user) = {
            let chat = editor.ai_state.chat.as_ref().unwrap();
            let key = (chat.origin_buffer_id, chat.opts.name.clone());
            let conversation = editor.ai_state.conversations.get_mut(&key).unwrap();
            conversation.append_user_message("first".into());
            let first_reply = conversation.append_assistant_message("reply".into(), "model".into());
            let main_user = conversation.append_user_message("main continuation".into());
            conversation.append_assistant_message("main reply".into(), "model".into());
            conversation.fork_from(first_reply);
            let fork_user = conversation.append_user_message("fork continuation".into());
            conversation.append_assistant_message("fork reply".into(), "model".into());
            (main_user, fork_user)
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = crate::syntax::Theme::from_scheme(crate::syntax::ColorScheme::tokyonight());

        terminal
            .draw(|frame| {
                super::render_chat_panel(frame, &mut editor, Rect::new(40, 0, 40, 22), &theme)
            })
            .unwrap();

        let rendered = terminal.backend().buffer().content().to_vec();
        let rendered_text = rendered
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered_text.contains("[‹ 2/2 ›]"));
        assert_eq!(editor.render_cache.ai_chat_branch_hitboxes.len(), 2);
        assert_eq!(editor.render_cache.ai_chat_branch_hitboxes[0].1, main_user);
        assert_eq!(editor.render_cache.ai_chat_branch_hitboxes[1].1, main_user);
        assert_ne!(main_user, fork_user);
    }

    #[test]
    fn hides_empty_assistant_messages_with_only_tool_calls() {
        let msg = ChatMessage {
            role: ChatRole::Assistant,
            content: "  ".to_string(),
            model: Some("model".to_string()),
            timestamp: std::time::Instant::now(),
            images: vec![],
            tool_calls: vec![ToolCallInfo {
                id: "call_1".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({}),
            }],
            tool_call_id: None,
            provider_state: vec![],
        };
        assert!(is_hidden_tool_only_assistant(&msg));
    }

    #[test]
    fn does_not_hide_non_empty_assistant_messages() {
        let msg = ChatMessage {
            role: ChatRole::Assistant,
            content: "done".to_string(),
            model: Some("model".to_string()),
            timestamp: std::time::Instant::now(),
            images: vec![],
            tool_calls: vec![ToolCallInfo {
                id: "call_1".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({}),
            }],
            tool_call_id: None,
            provider_state: vec![],
        };
        assert!(!is_hidden_tool_only_assistant(&msg));
    }

    #[test]
    fn queued_commands_are_labeled_distinctly() {
        let line = render_queued_input_row(40, QueuedChatInputKind::Command, "/clear", 0, false);
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("/ command: /clear"));
    }

    #[test]
    fn selected_queued_input_is_visually_emphasized() {
        let line = render_queued_input_row(40, QueuedChatInputKind::FollowUp, "next", 0, true);
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(!line.spans[0].style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn expanded_tool_details_include_arguments_and_result() {
        let call = ToolCallInfo {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "src/main.rs"}),
        };
        let lines = render_tool_event_details(80, Some(&call), "Target: src/main.rs\nfn main() {}");
        let text = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("tool: read_file"));
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("fn main() {}"));
    }

    #[test]
    fn chat_text_selection_highlights_only_the_selected_columns() {
        let line = Line::from(Span::styled("abcdef", Style::default().fg(Color::Green)));
        let highlighted = highlight_chat_selection(&line, 2, 4);
        assert_eq!(
            highlighted
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "abcdef"
        );
        assert!(highlighted.spans.iter().any(|span| {
            span.content == "cd" && span.style.bg == Some(Color::Rgb(74, 96, 145))
        }));
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
