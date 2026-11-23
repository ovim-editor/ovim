use crate::editor::Editor;
use crate::syntax::Theme;
use ovim_core::ai::chat_types::ChatFocus;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::ai_chat_header::render_chat_header;
use super::ai_chat_history::render_message_history;
use super::ai_chat_input::{render_chat_image_gallery, render_text_input};
use super::ai_chat_layout::ChatPanelLayout;
use super::ai_chat_pickers::{render_model_picker, render_slash_completion};
use super::ai_chat_style::{ACCENT_ASSISTANT_EDIT, ACCENT_USER};
use super::line_cache::LineRenderCache;

pub(crate) use super::ai_chat_style::{BG_PANEL, TEXT_DIM, TEXT_NORMAL};
pub(super) use super::ai_chat_text::styled_word_wrap_line;

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Render the full chat panel.
#[cfg(test)]
pub fn render_chat_panel(frame: &mut Frame, editor: &mut Editor, chat_area: Rect, theme: &Theme) {
    render_chat_panel_impl(frame, editor, chat_area, theme, None);
}

fn render_agent_conversation(
    frame: &mut Frame,
    editor: &Editor,
    area: Rect,
    snapshot: &ovim_core::agent_runtime::AgentControlPlaneSnapshot,
    agent_id: &str,
) {
    let Some(agent) = snapshot
        .agents
        .iter()
        .find(|agent| agent.agent_id.as_str() == agent_id)
    else {
        return;
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                agent.task_name.clone(),
                Style::default()
                    .fg(ACCENT_USER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", agent.lifecycle.replace('_', " ")),
                Style::default().fg(TEXT_DIM),
            ),
        ]),
        Line::from(Span::styled(
            format!(
                "{} · {} · {}",
                agent.resolved_route.model,
                agent.resolved_route.reasoning_effort,
                if agent.workspace.read_only {
                    "read-only snapshot"
                } else {
                    "writable workspace"
                }
            ),
            Style::default().fg(TEXT_DIM),
        )),
        Line::from(agent.objective.clone()),
        Line::from(""),
        Line::from(Span::styled(
            "Conversation",
            Style::default()
                .fg(TEXT_NORMAL)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    let message_start = agent.messages.len().saturating_sub(3);
    for message in &agent.messages[message_start..] {
        let sender = if message.sender_agent_id == snapshot.root_agent_id {
            "You"
        } else {
            "Agent"
        };
        let delivery = if message.state == "queued" {
            " · queued for next boundary"
        } else {
            ""
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{sender}: "),
                Style::default()
                    .fg(if sender == "You" {
                        ACCENT_USER
                    } else {
                        ACCENT_ASSISTANT_EDIT
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(message.content.replace('\n', " ")),
            Span::styled(delivery, Style::default().fg(TEXT_DIM)),
        ]));
    }
    if let Some(handoff) = &agent.handoff {
        lines.push(Line::from(vec![
            Span::styled(
                "Agent: ",
                Style::default()
                    .fg(ACCENT_ASSISTANT_EDIT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(handoff.summary.replace('\n', " ")),
        ]));
    }
    if agent.messages.is_empty() && agent.handoff.is_none() {
        lines.push(Line::from(Span::styled(
            "No messages yet",
            Style::default().fg(TEXT_DIM),
        )));
    }
    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            "Activity",
            Style::default()
                .fg(TEXT_NORMAL)
                .add_modifier(Modifier::BOLD),
        )),
    ]);
    let switcher = super::agent_tree::agent_switcher_lines(
        snapshot,
        area.width as usize,
        editor.ai_chat_focus() == ChatFocus::TreePanel && editor.ai_agent_tree_focused(),
        editor.ai_agent_tree_cursor(),
        Some(agent_id),
    );
    let available = area
        .height
        .saturating_sub(lines.len() as u16)
        .saturating_sub(switcher.len() as u16) as usize;
    let start = agent.trace.len().saturating_sub(available);
    for event in &agent.trace[start..] {
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "#{:<4} {:<16}",
                    event.sequence,
                    event.kind.replace('_', " ")
                ),
                Style::default().fg(TEXT_DIM),
            ),
            Span::raw(event.summary.replace('\n', " ")),
        ]));
    }
    if agent.trace.is_empty() {
        lines.push(Line::from(Span::styled(
            "No activity reported yet",
            Style::default().fg(TEXT_DIM),
        )));
    }
    lines.extend(switcher);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(TEXT_NORMAL).bg(BG_PANEL)),
        area,
    );
}

pub fn render_chat_panel_cached(
    frame: &mut Frame,
    editor: &mut Editor,
    chat_area: Rect,
    theme: &Theme,
    cache: &mut LineRenderCache,
) {
    let started = std::time::Instant::now();
    let hits_before = cache.chat_hits;
    let misses_before = cache.chat_misses;
    render_chat_panel_impl(frame, editor, chat_area, theme, Some(cache));
    editor.render_cache.ai_chat_last_render_micros = started.elapsed().as_micros();
    editor.render_cache.ai_chat_last_cache_hits = cache.chat_hits.saturating_sub(hits_before);
    editor.render_cache.ai_chat_last_cache_misses = cache.chat_misses.saturating_sub(misses_before);
}

fn render_chat_panel_impl(
    frame: &mut Frame,
    editor: &mut Editor,
    chat_area: Rect,
    theme: &Theme,
    cache: Option<&mut LineRenderCache>,
) {
    editor.render_cache.ai_chat_interactions.begin_frame();
    editor.render_cache.ai_chat_image_thumbnails.clear();
    let Some(layout) = ChatPanelLayout::resolve(
        chat_area,
        editor.ai_chat_tree_panel_open(),
        editor.ai_chat_input(),
        editor.ai_chat_input_cursor(),
        editor.indent_options().tab_width,
        editor.render_cache.terminal_image_support,
        editor.ai_chat_pending_images().len(),
    ) else {
        return;
    };

    let agent_snapshot = editor.ai_agent_current_snapshot().ok().flatten();

    // Delegated agents are switched in the conversation surface below; the
    // optional side tree remains dedicated to primary conversation branches.
    if let Some(tree_rect) = layout.tree_area {
        super::conversation_tree::render_tree_panel(frame, editor, tree_rect);
    }

    let model_picker_anchor = render_chat_header(frame, editor, layout.header_area);
    let gallery_paths = editor
        .ai_chat_pending_images()
        .iter()
        .map(|image| image.path.clone())
        .collect::<Vec<_>>();
    if layout.messages_area.height > 0 {
        if let (Some(snapshot), Some(agent_id)) =
            (agent_snapshot.as_ref(), editor.ai_agent_selected_id())
        {
            render_agent_conversation(frame, editor, layout.messages_area, snapshot, agent_id);
        } else {
            render_message_history(
                frame,
                editor,
                layout.messages_area,
                theme,
                cache,
                agent_snapshot.as_ref(),
            );
        }
    } else {
        editor.render_cache.ai_chat_interactions.history = None;
    }
    if let Some(gallery_area) = layout.gallery_area {
        render_chat_image_gallery(frame, editor, gallery_area, &gallery_paths);
    }
    render_text_input(
        frame,
        editor,
        layout.input_area,
        &layout.input_rows,
        layout.input_visible_start,
    );
    render_slash_completion(frame, editor, layout.input_area, layout.messages_area.y);
    if editor.ai_chat_focus() == ChatFocus::ModelSelector {
        render_model_picker(frame, editor, model_picker_anchor, layout.content_area);
    }
}

/// Returns cursor (x, y) for the chat input, if focused.
pub fn chat_cursor_info(editor: &Editor, chat_area: Rect) -> Option<(u16, u16)> {
    let focus = editor.ai_chat_focus();
    if focus != ChatFocus::TextInput {
        return None;
    }

    let layout = ChatPanelLayout::resolve(
        chat_area,
        editor.ai_chat_tree_panel_open(),
        editor.ai_chat_input(),
        editor.ai_chat_input_cursor(),
        editor.indent_options().tab_width,
        editor.render_cache.terminal_image_support,
        editor.ai_chat_pending_images().len(),
    )?;
    Some(layout.cursor_position())
}

#[cfg(test)]
mod tests;
