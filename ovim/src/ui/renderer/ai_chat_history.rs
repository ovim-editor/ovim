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
use unicode_segmentation::UnicodeSegmentation;

use super::ai_chat_bubble::{
    branch_control_text, card_text_width, chat_bubble_cache_key, render_chat_bubble,
    BubbleImagePlacement, ChatBubbleRender,
};
use super::ai_chat_event_rows::{
    fallback_tool_summary, is_hidden_tool_only_assistant, render_queued_input_row,
    render_tool_event_details, render_tool_event_row, summarize_streaming_tool_call,
};
use super::ai_chat_style::{BG_PANEL, TEXT_DIM};
use super::ai_chat_text::{center_text, text_display_width};
use super::line_cache::{CachedChatBubble, CachedChatImage, LineRenderCache};

// ---------------------------------------------------------------------------
// Message History
// ---------------------------------------------------------------------------
pub(super) fn render_message_history(
    frame: &mut Frame,
    editor: &mut Editor,
    area: Rect,
    theme: &Theme,
    mut cache: Option<&mut LineRenderCache>,
    agent_snapshot: Option<&ovim_core::agent_runtime::AgentControlPlaneSnapshot>,
) {
    editor.render_cache.ai_chat_interactions.history =
        Some(crate::key_convert::convert_ratatui_rect(area));
    editor.render_cache.ai_chat_interactions.links.clear();
    editor.render_cache.ai_chat_last_queued_row_spans.clear();
    editor.render_cache.ai_chat_last_shell_row_spans.clear();
    let messages = editor.ai_chat_messages();
    let has_agent_cards = agent_snapshot.is_some_and(|snapshot| !snapshot.agents.is_empty());
    let has_live_shells = !editor.ai_chat_live_shell_tool_ids().is_empty();
    let has_streaming = editor
        .ai_chat_streaming_content()
        .is_some_and(|content| !content.is_empty())
        || editor
            .ai_chat_streaming_thinking()
            .is_some_and(|thinking| !thinking.is_empty());
    if messages.is_empty() && !has_agent_cards && !has_live_shells && !has_streaming {
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
    let conversation_id = editor
        .conversation()
        .map(|conversation| conversation.instance_id())
        .unwrap_or(0);

    // Render messages bottom-up with scroll
    let mut rendered_lines: Vec<(Line, bool)> = Vec::new(); // (line, is_bubble_border)
    let mut message_row_spans: Vec<(usize, usize)> = Vec::with_capacity(messages.len());
    let mut branch_controls = Vec::new();
    let mut tool_replay_controls = Vec::new();
    let mut inline_images = Vec::new();
    let mut history_links = Vec::new();
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
            let replay_action = tool_call_id
                .and_then(|id| editor.ai_chat_tool_replay_label(id))
                .map(|label| {
                    if label == "Open diff" {
                        "[open diff]"
                    } else {
                        "[↻ replay]"
                    }
                });
            rendered_lines.push((
                render_tool_event_row(
                    panel_width,
                    &label,
                    kind,
                    is_selected,
                    false,
                    expanded,
                    replay_action,
                ),
                false,
            ));
            if let Some((tool_call_id, action)) = tool_call_id.zip(replay_action) {
                tool_replay_controls.push((msg_row_start, tool_call_id.to_string(), action));
            }
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

        let branch_position = branch_navigation.map(|(position, count, _, _)| (position, count));
        let terminal_image_support = editor.render_cache.terminal_image_support;
        let bubble = if let (Some(node_id), Some(cache)) = (node_id, cache.as_deref_mut()) {
            let key = chat_bubble_cache_key(
                conversation_id,
                node_id,
                panel_width,
                is_selected,
                allow_edits,
                is_thinking_expanded,
                child_count,
                branch_position,
                theme,
                terminal_image_support,
            );
            if let Some(cached) = cache.get_chat_bubble(&key) {
                ChatBubbleRender {
                    lines: cached.lines,
                    links: cached.links,
                    images: cached
                        .images
                        .into_iter()
                        .map(|image| BubbleImagePlacement {
                            row: image.row,
                            x: image.x,
                            width: image.width,
                            height: image.height,
                            path: image.path,
                        })
                        .collect(),
                    cacheable: true,
                }
            } else {
                let bubble = render_chat_bubble(
                    msg,
                    panel_width,
                    is_selected,
                    allow_edits,
                    is_thinking_expanded,
                    child_count,
                    branch_position,
                    theme,
                    terminal_image_support,
                );
                if bubble.cacheable {
                    cache.insert_chat_bubble(
                        key,
                        CachedChatBubble {
                            lines: bubble.lines.clone(),
                            links: bubble.links.clone(),
                            images: bubble
                                .images
                                .iter()
                                .map(|image| CachedChatImage {
                                    row: image.row,
                                    x: image.x,
                                    width: image.width,
                                    height: image.height,
                                    path: image.path.clone(),
                                })
                                .collect(),
                        },
                    );
                }
                bubble
            }
        } else {
            render_chat_bubble(
                msg,
                panel_width,
                is_selected,
                allow_edits,
                is_thinking_expanded,
                child_count,
                branch_position,
                theme,
                terminal_image_support,
            )
        };
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
        for mut link in bubble.links {
            link.row += msg_row_start;
            history_links.push(link);
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
                editor.render_cache.terminal_image_support,
            );
            let msg_row_start = rendered_lines.len();
            for mut link in bubble.links {
                link.row += msg_row_start;
                history_links.push(link);
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
        }
    }

    // Tool call status rows during tool execution
    if let Some(chat) = editor.ai_state.chat.as_ref() {
        if !chat.streaming_tool_calls.is_empty() {
            let selected_shell = editor
                .ai_chat_history_selected_shell_tool_id()
                .map(str::to_owned);
            for tc in &chat.streaming_tool_calls {
                let (kind, label) = summarize_streaming_tool_call(tc);
                let status_text = editor
                    .ai_shell_process_row_label(&tc.id)
                    .unwrap_or_else(|| format!("running {label}"));
                let row_start = rendered_lines.len();
                rendered_lines.push((
                    render_tool_event_row(
                        panel_width,
                        &status_text,
                        kind,
                        focus == ChatFocus::MessageHistory
                            && selected_shell.as_deref() == Some(tc.id.as_str()),
                        true,
                        false,
                        None,
                    ),
                    false,
                ));
                if tc.name == "bash" && editor.ai_chat_live_shell_tool_ids().contains(&tc.id) {
                    editor
                        .render_cache
                        .ai_chat_last_shell_row_spans
                        .push((row_start, rendered_lines.len()));
                }
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

    // Agents are conversations, not footer cards. Keep a compact switcher at
    // the live edge; Down from an empty composer expands it, and Enter changes
    // the active conversation.
    if let Some(snapshot) = agent_snapshot.filter(|snapshot| !snapshot.agents.is_empty()) {
        let lines = super::agent_tree::agent_switcher_lines(
            snapshot,
            panel_width,
            focus == ChatFocus::TreePanel && editor.ai_agent_tree_focused(),
            editor.ai_agent_tree_cursor(),
            editor.ai_agent_selected_id(),
        );
        rendered_lines.extend(lines.into_iter().map(|line| (line, false)));
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
    editor.render_cache.ai_chat_interactions.links = history_links;
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
            let control_width =
                text_display_width(&branch_control_text(control.position, control.count)) as u16;
            if (control_width as usize) < card_text_width(area.width as usize, "\u{258d}") {
                let x = area.x + area.width - control_width;
                let left_width = control_width / 2;
                editor.render_cache.ai_chat_interactions.branches.push((
                    crate::key_convert::convert_ratatui_rect(Rect {
                        x,
                        y: r.y,
                        width: left_width,
                        height: 1,
                    }),
                    control.previous,
                ));
                editor.render_cache.ai_chat_interactions.branches.push((
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
        if let Some((_, tool_call_id, action)) = tool_replay_controls
            .iter()
            .find(|(row, _, _)| *row == line_idx)
        {
            let action_width = text_display_width(action) as u16;
            if action_width < area.width {
                editor.render_cache.ai_chat_interactions.tool_replays.push((
                    crate::key_convert::convert_ratatui_rect(Rect {
                        x: area.x + area.width - action_width,
                        y: r.y,
                        width: action_width,
                        height: 1,
                    }),
                    tool_call_id.clone(),
                ));
            }
        }
    }
}

pub(super) fn highlight_chat_selection(
    line: &Line<'_>,
    selection_start: usize,
    selection_end: usize,
) -> Line<'static> {
    let mut output = Vec::new();
    let mut display_column = 0usize;
    for span in &line.spans {
        let mut segment = String::new();
        let mut segment_selected = None;
        for grapheme in span.content.graphemes(true) {
            let width = crate::display::grapheme_display_width(grapheme).max(1);
            let grapheme_start = display_column;
            let grapheme_end = display_column.saturating_add(width);
            display_column = grapheme_end;
            let selected = grapheme_end > selection_start && grapheme_start < selection_end;
            if segment_selected.is_some_and(|current| current != selected) {
                let style = if segment_selected == Some(true) {
                    span.style.bg(Color::Rgb(74, 96, 145)).fg(Color::White)
                } else {
                    span.style
                };
                output.push(Span::styled(std::mem::take(&mut segment), style));
            }
            segment_selected = Some(selected);
            segment.push_str(grapheme);
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

#[derive(Clone, Copy)]
struct BranchRenderControl {
    row: usize,
    position: usize,
    count: usize,
    previous: ovim_core::ai::chat_types::NodeId,
    next: ovim_core::ai::chat_types::NodeId,
}

struct HistoryImagePlacement {
    /// First inner image row in absolute rendered-history coordinates.
    row: usize,
    x: u16,
    width: u16,
    height: u16,
    path: std::path::PathBuf,
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
    let used = text_display_width(&text);
    if used < width {
        spans.push(Span::styled(
            " ".repeat(width - used),
            Style::default().bg(BG_PANEL),
        ));
    }
    Line::from(spans)
}
