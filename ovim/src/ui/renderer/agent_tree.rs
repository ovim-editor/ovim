//! Delegated-agent tree and inline-card presentation.
//!
//! This module deliberately consumes only the versioned, transport-neutral
//! `AgentControlPlaneSnapshot` used by the headless API. Renderer state is
//! limited to selection, following, and explicit expansion.

use ovim_core::agent_runtime::{AgentControlPlaneSnapshot, AgentSnapshot};
use ovim_core::run_log::{AgentProgressActivity, AgentReported, AgentUsageEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::ai_chat::{BG_PANEL, TEXT_DIM};

const BORDER_COLOR: Color = Color::Rgb(60, 66, 80);
const SELECTED_BG: Color = Color::Rgb(40, 50, 65);
const FOLLOWED_BG: Color = Color::Rgb(38, 56, 73);
const ATTENTION: Color = Color::Rgb(255, 191, 77);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentTone {
    Quiet,
    Active,
    Complete,
    Failed,
    Interrupted,
    Attention,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentCardView {
    pub agent_id: String,
    pub depth: usize,
    pub tone: AgentTone,
    pub attention_priority: u8,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentTreeView {
    pub header: String,
    pub cards: Vec<AgentCardView>,
    pub empty_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentApprovalPrompt {
    pub summary: String,
}

#[derive(Clone, Copy)]
pub(crate) struct AgentTreeRenderState<'a> {
    pub enabled: bool,
    pub focused: bool,
    pub cursor: usize,
    pub selected_agent_id: Option<&'a str>,
    pub followed_agent_id: Option<&'a str>,
    pub expanded: &'a HashSet<String>,
}

impl<'a> AgentTreeRenderState<'a> {
    #[cfg(test)]
    fn enabled(expanded: &'a HashSet<String>) -> Self {
        Self {
            enabled: true,
            focused: true,
            cursor: 0,
            selected_agent_id: None,
            followed_agent_id: None,
            expanded,
        }
    }
}

pub(crate) fn project_agent_tree(
    snapshot: Option<&AgentControlPlaneSnapshot>,
    width: usize,
    expanded: &HashSet<String>,
    enabled: bool,
) -> AgentTreeView {
    let Some(snapshot) = snapshot else {
        return AgentTreeView {
            header: " Agents".into(),
            cards: Vec::new(),
            empty_message: Some(if enabled {
                "No delegated-agent run".into()
            } else {
                "Delegation disabled".into()
            }),
        };
    };
    let attention = if snapshot.pending_attention > 0 {
        format!(" !{}", snapshot.pending_attention)
    } else {
        String::new()
    };
    let header = fit(
        &format!(" Agents {}{attention}", snapshot.agents.len()),
        width.saturating_sub(1),
    );
    let cards = snapshot
        .hierarchy()
        .into_iter()
        .map(|agent| project_agent_card(agent, width, expanded.contains(agent.agent_id.as_str())))
        .collect();
    AgentTreeView {
        header,
        cards,
        empty_message: snapshot
            .agents
            .is_empty()
            .then(|| "No delegated agents".into()),
    }
}

pub(crate) fn project_inline_agent_cards(
    snapshot: &AgentControlPlaneSnapshot,
    width: usize,
    expanded: &HashSet<String>,
) -> Vec<AgentCardView> {
    let mut cards = snapshot
        .hierarchy()
        .into_iter()
        .map(|agent| project_agent_card(agent, width, expanded.contains(agent.agent_id.as_str())))
        .collect::<Vec<_>>();
    // Pending decisions are the only safe reason to reorder the compact chat
    // cards. Stable hierarchy order is retained within each priority tier.
    cards.sort_by_key(|card| std::cmp::Reverse(card.attention_priority));
    cards
}

pub(crate) fn project_agent_approval_prompt(
    snapshot: &AgentControlPlaneSnapshot,
) -> Option<AgentApprovalPrompt> {
    let (agent, approval) = snapshot.oldest_pending_approval()?;
    let ancestry = agent
        .ancestry
        .iter()
        .map(|ancestor| {
            snapshot
                .agents
                .iter()
                .find(|candidate| candidate.agent_id == *ancestor)
                .map(|candidate| candidate.task_name.clone())
                .unwrap_or_else(|| {
                    if *ancestor == snapshot.root_agent_id {
                        "root".into()
                    } else {
                        short_id(ancestor.as_str())
                    }
                })
        })
        .collect::<Vec<_>>()
        .join(" › ");
    Some(AgentApprovalPrompt {
        summary: format!(
            "Child: {} ({})\nAncestry: {} › {} · role {}\nRoute: requested {}/{} → effective {}/{}{}\nTool: {} · effect {}\nWorkspace: {} · {} · {}\nReason: {}",
            agent.task_name,
            short_id(agent.agent_id.as_str()),
            if ancestry.is_empty() { "root" } else { &ancestry },
            agent.task_name,
            agent.role,
            agent.requested_route.catalog_model_id,
            agent.requested_route.reasoning_effort,
            agent.resolved_route.catalog_model_id,
            agent.resolved_route.reasoning_effort,
            agent
                .resolved_route
                .fallback_reason
                .as_deref()
                .map(|reason| format!(" · fallback: {reason}"))
                .unwrap_or_default(),
            approval.tool_name,
            approval.effect,
            workspace_label(&agent.workspace.strategy),
            agent.workspace.ownership,
            agent.workspace.root.as_deref().unwrap_or("root not reported"),
            approval.reason
        ),
    })
}

fn project_agent_card(agent: &AgentSnapshot, width: usize, expanded: bool) -> AgentCardView {
    let usable = width.saturating_sub(1).max(1);
    let attention_priority = if agent.attention.pending_approvals > 0 {
        3
    } else if agent.attention.pending_messages > 0 {
        2
    } else if agent.attention.required {
        1
    } else {
        0
    };
    let tone = tone(agent, attention_priority);
    let connector = tree_connector(agent.ancestry.len());
    let disclosure = if expanded { "▾" } else { "▸" };
    let fallback = if agent.resolved_route.resolution == "configured_fallback" {
        "↪ "
    } else {
        ""
    };
    let attention = if attention_priority > 0 { " !" } else { "" };
    let recovery = if agent.recovery_status != "none" {
        " ↻"
    } else {
        ""
    };
    let mut lines = vec![fit(
        &format!(
            "{connector}{disclosure} {} {} · {}{attention}{recovery}",
            lifecycle_icon(agent),
            agent.task_name,
            lifecycle_label(agent)
        ),
        usable,
    )];

    let compact = format!(
        "{}{} · {fallback}{}/{} · {} · {}",
        "  ".repeat(agent.ancestry.len().saturating_add(1)),
        agent.role,
        agent.resolved_route.model,
        agent.resolved_route.reasoning_effort,
        activity(agent),
        compact_usage(agent)
    );
    if width >= 28 || expanded {
        lines.push(fit(&compact, usable));
    }

    if expanded {
        let indent = "  ".repeat(agent.ancestry.len().saturating_add(1));
        let ancestry = if agent.ancestry.is_empty() {
            "root".into()
        } else {
            agent
                .ancestry
                .iter()
                .map(|id| short_id(id.as_str()))
                .collect::<Vec<_>>()
                .join(" › ")
        };
        lines.extend([
            fit(
                &format!(
                    "{indent}depth {} · ancestry {ancestry} · gen {}",
                    agent.ancestry.len(),
                    agent.turn_generation
                ),
                usable,
            ),
            fit(
                &format!(
                    "{indent}requested {}/{} · {}",
                    agent.requested_route.catalog_model_id,
                    agent.requested_route.reasoning_effort,
                    agent.requested_route.fallback_policy
                ),
                usable,
            ),
            fit(
                &format!(
                    "{indent}effective {}/{} · {}{}",
                    agent.resolved_route.catalog_model_id,
                    agent.resolved_route.reasoning_effort,
                    agent.resolved_route.resolution,
                    agent
                        .resolved_route
                        .fallback_reason
                        .as_deref()
                        .map(|reason| format!(": {reason}"))
                        .unwrap_or_default()
                ),
                usable,
            ),
            fit(
                &format!(
                    "{indent}elapsed {} · {}",
                    reported_elapsed(agent),
                    expanded_usage(&agent.usage)
                ),
                usable,
            ),
            fit(
                &format!(
                    "{indent}workspace {} · {}{}",
                    workspace_label(&agent.workspace.strategy),
                    agent.workspace.ownership,
                    if agent.workspace.read_only {
                        " · read-only"
                    } else {
                        ""
                    }
                ),
                usable,
            ),
        ]);
        if attention_priority > 0 {
            let pending_tool = agent
                .approvals
                .iter()
                .find(|approval| approval.state == "pending")
                .map(|approval| format!(" · {} ({})", approval.tool_name, approval.effect))
                .unwrap_or_default();
            lines.push(fit(
                &format!(
                    "{indent}! {} approval · {} message · {} notice{pending_tool}",
                    agent.attention.pending_approvals,
                    agent.attention.pending_messages,
                    agent.attention.pending_notifications
                ),
                usable,
            ));
        }
        if let Some(handoff) = &agent.handoff {
            lines.push(fit(
                &format!(
                    "{indent}handoff {} · {}e · {}a · {}",
                    handoff.status,
                    handoff.evidence.len(),
                    agent.artifact_handles.len(),
                    handoff.summary.replace('\n', " ")
                ),
                usable,
            ));
        } else {
            lines.push(fit(
                &format!(
                    "{indent}handoff pending · {} artifact",
                    agent.artifact_handles.len()
                ),
                usable,
            ));
        }
        if agent.recovery_status != "none" {
            lines.push(fit(
                &format!(
                    "{indent}recovery {} · follow-up can restart",
                    agent.recovery_status
                ),
                usable,
            ));
        }
    }

    AgentCardView {
        agent_id: agent.agent_id.to_string(),
        depth: agent.ancestry.len(),
        tone,
        attention_priority,
        lines,
    }
}

pub(crate) fn render_agent_tree_panel(
    frame: &mut Frame,
    snapshot: Option<&AgentControlPlaneSnapshot>,
    area: Rect,
    state: AgentTreeRenderState<'_>,
) {
    if area.width < 8 || area.height < 3 {
        return;
    }
    let view = project_agent_tree(snapshot, area.width as usize, state.expanded, state.enabled);
    let header_style = Style::default()
        .fg(if state.focused {
            Color::White
        } else {
            TEXT_DIM
        })
        .bg(BG_PANEL)
        .add_modifier(Modifier::BOLD);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad(&view.header, area.width.saturating_sub(1) as usize),
            header_style,
        ))),
        Rect::new(area.x, area.y, area.width.saturating_sub(1), 1),
    );

    let footer_height = if area.height >= 8 {
        2
    } else {
        u16::from(area.height >= 6)
    };
    let content_height = area.height.saturating_sub(1 + footer_height) as usize;
    let flat = flatten_cards(&view.cards);
    let selected_line = flat
        .iter()
        .position(|line| line.card_index == state.cursor)
        .unwrap_or(0);
    let scroll = selected_line.saturating_sub(content_height.saturating_sub(1));
    if let Some(empty) = &view.empty_message {
        render_text_row(frame, area, 1, empty, AgentTone::Quiet, false, false);
    } else {
        for (visible_row, line) in flat.iter().skip(scroll).take(content_height).enumerate() {
            let selected = (state.focused && line.card_index == state.cursor)
                || state.selected_agent_id == Some(line.agent_id.as_str());
            let followed = state.followed_agent_id == Some(line.agent_id.as_str());
            render_text_row(
                frame,
                area,
                1 + visible_row as u16,
                &line.text,
                line.tone,
                selected,
                followed,
            );
        }
    }

    if footer_height > 0 {
        let hint = if area.width >= 30 {
            " ↵ select · f/w follow · Space details"
        } else {
            " ↵ f Space · Tab"
        };
        render_text_row(
            frame,
            area,
            area.height - footer_height,
            hint,
            AgentTone::Quiet,
            false,
            false,
        );
        if footer_height > 1 {
            render_text_row(
                frame,
                area,
                area.height - 1,
                " m message · r follow-up · i interrupt · a/d approval",
                AgentTone::Quiet,
                false,
                false,
            );
        }
    }
    render_right_border(frame, area);
}

pub(crate) fn inline_card_lines(card: &AgentCardView, width: usize) -> Vec<Line<'static>> {
    card.lines
        .iter()
        .map(|text| {
            let style = Style::default().fg(tone_color(card.tone)).bg(BG_PANEL);
            Line::from(Span::styled(pad(text, width), style))
        })
        .collect()
}

struct FlatCardLine {
    card_index: usize,
    agent_id: String,
    tone: AgentTone,
    text: String,
}

fn flatten_cards(cards: &[AgentCardView]) -> Vec<FlatCardLine> {
    cards
        .iter()
        .enumerate()
        .flat_map(|(card_index, card)| {
            card.lines.iter().cloned().map(move |text| FlatCardLine {
                card_index,
                agent_id: card.agent_id.clone(),
                tone: card.tone,
                text,
            })
        })
        .collect()
}

fn render_text_row(
    frame: &mut Frame,
    area: Rect,
    relative_y: u16,
    text: &str,
    tone: AgentTone,
    selected: bool,
    followed: bool,
) {
    if relative_y >= area.height {
        return;
    }
    let width = area.width.saturating_sub(1) as usize;
    let bg = if selected {
        SELECTED_BG
    } else if followed {
        FOLLOWED_BG
    } else {
        BG_PANEL
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad(&fit(text, width), width),
            Style::default().fg(tone_color(tone)).bg(bg),
        ))),
        Rect::new(area.x, area.y + relative_y, area.width.saturating_sub(1), 1),
    );
}

fn render_right_border(frame: &mut Frame, area: Rect) {
    for row in 0..area.height {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "│",
                Style::default().fg(BORDER_COLOR).bg(BG_PANEL),
            ))),
            Rect::new(area.right().saturating_sub(1), area.y + row, 1, 1),
        );
    }
}

fn tone(agent: &AgentSnapshot, attention_priority: u8) -> AgentTone {
    if attention_priority > 0 || agent.lifecycle == "waiting_for_user" {
        AgentTone::Attention
    } else if agent.recovery_status != "none" || agent.lifecycle == "interrupted" {
        AgentTone::Interrupted
    } else {
        match agent.lifecycle.as_str() {
            "starting" | "running" | "waiting_for_agent" | "waiting_for_tool" => AgentTone::Active,
            "completed" => AgentTone::Complete,
            "failed" => AgentTone::Failed,
            _ => AgentTone::Quiet,
        }
    }
}

fn tone_color(tone: AgentTone) -> Color {
    match tone {
        AgentTone::Quiet => TEXT_DIM,
        AgentTone::Active => Color::Rgb(112, 175, 255),
        AgentTone::Complete => Color::Rgb(126, 211, 160),
        AgentTone::Failed => Color::Rgb(255, 107, 107),
        AgentTone::Interrupted => Color::Rgb(210, 164, 96),
        AgentTone::Attention => ATTENTION,
    }
}

fn lifecycle_icon(agent: &AgentSnapshot) -> &'static str {
    if agent.recovery_status != "none" {
        return "↻";
    }
    match agent.lifecycle.as_str() {
        "starting" | "running" => "▶",
        "waiting_for_agent" | "waiting_for_tool" => "◆",
        "waiting_for_user" => "!",
        "completed" => "✓",
        "failed" => "×",
        "interrupted" => "↻",
        _ => "○",
    }
}

fn lifecycle_label(agent: &AgentSnapshot) -> String {
    if agent.recovery_status != "none" {
        "interrupted/restart".into()
    } else {
        agent.lifecycle.replace('_', " ")
    }
}

fn activity(agent: &AgentSnapshot) -> String {
    match &agent.progress {
        AgentReported::Reported(progress) => progress
            .current_tool
            .clone()
            .or_else(|| progress.detail.clone())
            .unwrap_or_else(|| activity_name(&progress.activity).into()),
        AgentReported::NotReported => match agent.lifecycle.as_str() {
            "waiting_for_user" => "approval".into(),
            "waiting_for_agent" => "waiting".into(),
            _ => "activity n/r".into(),
        },
    }
}

fn activity_name(activity: &AgentProgressActivity) -> &'static str {
    match activity {
        AgentProgressActivity::StartingProvider => "starting provider",
        AgentProgressActivity::ProviderBound => "provider bound",
        AgentProgressActivity::ProviderCall => "provider call",
        AgentProgressActivity::Reasoning => "reasoning",
        AgentProgressActivity::Responding => "responding",
        AgentProgressActivity::WaitingForTool => "waiting for tool",
        AgentProgressActivity::WaitingForApproval => "waiting approval",
        AgentProgressActivity::FinalizingHandoff => "finalizing handoff",
        AgentProgressActivity::Other => "working",
    }
}

fn compact_usage(agent: &AgentSnapshot) -> String {
    match &agent.usage {
        AgentReported::Reported(usage) => match (&usage.input_tokens, &usage.output_tokens) {
            (AgentReported::Reported(input), AgentReported::Reported(output)) => {
                format!(
                    "{}t · {} tools",
                    input.saturating_add(*output),
                    usage.tool_calls
                )
            }
            _ => format!("tokens n/r · {} tools", usage.tool_calls),
        },
        AgentReported::NotReported => "usage n/r".into(),
    }
}

fn expanded_usage(usage: &AgentReported<AgentUsageEvent>) -> String {
    let AgentReported::Reported(usage) = usage else {
        return "usage not reported".into();
    };
    format!(
        "in {} · out {} · cache {} · {} tool · {} provider",
        reported_u64(&usage.input_tokens),
        reported_u64(&usage.output_tokens),
        reported_u64(&usage.cached_input_tokens),
        usage.tool_calls,
        usage.provider_calls
    )
}

fn reported_u64(value: &AgentReported<u64>) -> String {
    match value {
        AgentReported::Reported(value) => format!("{value}t"),
        AgentReported::NotReported => "n/r".into(),
    }
}

fn reported_elapsed(agent: &AgentSnapshot) -> String {
    match agent.elapsed_millis {
        AgentReported::Reported(elapsed) => format_elapsed(elapsed),
        AgentReported::NotReported => "not reported".into(),
    }
}

fn format_elapsed(millis: u64) -> String {
    if millis < 1_000 {
        format!("{millis}ms")
    } else if millis < 60_000 {
        format!("{:.1}s", millis as f64 / 1_000.0)
    } else {
        format!("{}m{:02}s", millis / 60_000, (millis / 1_000) % 60)
    }
}

fn workspace_label(strategy: &str) -> &str {
    match strategy {
        "read_only_snapshot" => "snapshot",
        "isolated_worktree" => "worktree",
        "shared_workspace" => "shared",
        other => other,
    }
}

fn tree_connector(depth: usize) -> String {
    if depth == 0 {
        String::new()
    } else {
        format!("{}└─", "│ ".repeat(depth.saturating_sub(1)))
    }
}

fn short_id(value: &str) -> String {
    value.chars().take(10).collect()
}

fn fit(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    if width == 1 {
        return "…".into();
    }
    let mut result = String::new();
    let budget = width - 1;
    for grapheme in text.graphemes(true) {
        if UnicodeWidthStr::width(result.as_str()) + UnicodeWidthStr::width(grapheme) > budget {
            break;
        }
        result.push_str(grapheme);
    }
    result.push('…');
    result
}

fn pad(text: &str, width: usize) -> String {
    let used = UnicodeWidthStr::width(text);
    format!("{text}{}", " ".repeat(width.saturating_sub(used)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ovim_core::agent_runtime::AgentControlPlaneSnapshot;
    use ratatui::{backend::TestBackend, Terminal};
    use serde_json::{json, Value};

    fn agent(id: &str, parent: &str, task: &str, lifecycle: &str) -> Value {
        json!({
            "agent_id": id,
            "parent_agent_id": parent,
            "ancestry": ["agt_root"],
            "children": [],
            "task_name": task,
            "role": "explorer",
            "objective": format!("Investigate {task}"),
            "requested_route": {
                "catalog_model_id": "codex/requested",
                "reasoning_effort": "medium",
                "fallback_policy": "fail_closed",
                "fallback_catalog_model_id": null,
                "fallback_reasoning_effort": null
            },
            "resolved_route": {
                "catalog_generation": "catalog-1",
                "catalog_model_id": "codex/effective",
                "profile_name": "codex",
                "provider": "openai",
                "model": "gpt-effective",
                "reasoning_effort": "high",
                "resolution": "exact",
                "fallback_reason": null
            },
            "lifecycle": lifecycle,
            "turn_generation": 0,
            "turn_id": "trn_1",
            "elapsed_millis": {"status": "reported", "value": 1250},
            "progress": {"status": "reported", "value": {
                "version": 1,
                "turn_generation": 0,
                "activity": "waiting_for_tool",
                "elapsed_millis": 1250,
                "current_tool": "read_file",
                "detail": "src/lib.rs"
            }},
            "usage": {"status": "reported", "value": {
                "version": 1,
                "turn_generation": 0,
                "provider_calls": 2,
                "tool_calls": 3,
                "input_tokens": {"status": "reported", "value": 100},
                "output_tokens": {"status": "reported", "value": 40},
                "cached_input_tokens": {"status": "not_reported"},
                "cost": {"status": "not_reported"}
            }},
            "workspace": {
                "workspace_id": format!("wsp_{id}"),
                "strategy": "read_only_snapshot",
                "manifest_id": "mft_1",
                "ownership": "ovim",
                "root": null,
                "read_only": true
            },
            "messages": [],
            "approvals": [],
            "handoff": null,
            "artifact_handles": [],
            "attention": {
                "required": false,
                "pending_approvals": 0,
                "pending_messages": 0,
                "pending_notifications": 0
            },
            "recovery_status": "none"
        })
    }

    fn snapshot(agents: Vec<Value>) -> AgentControlPlaneSnapshot {
        serde_json::from_value(json!({
            "schema_version": 1,
            "run_id": "run_1",
            "root_agent_id": "agt_root",
            "last_sequence": 20,
            "agents": agents,
            "pending_attention": 0
        }))
        .unwrap()
    }

    fn buffer_text(
        snapshot: Option<&AgentControlPlaneSnapshot>,
        width: u16,
        height: u16,
    ) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let expanded = HashSet::new();
        terminal
            .draw(|frame| {
                render_agent_tree_panel(
                    frame,
                    snapshot,
                    frame.area(),
                    AgentTreeRenderState::enabled(&expanded),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn hierarchy_is_parent_before_child_despite_completion_order() {
        let child = agent(
            "agt_child",
            "agt_parent",
            "child finished first",
            "completed",
        );
        let mut parent = agent("agt_parent", "agt_root", "parent still running", "running");
        parent["children"] = json!(["agt_child"]);
        let mut snapshot = snapshot(vec![child, parent]);
        snapshot.agents[0].ancestry = vec![
            ovim_core::run_log::AgentId::parse("agt_root").unwrap(),
            ovim_core::run_log::AgentId::parse("agt_parent").unwrap(),
        ];

        let view = project_agent_tree(Some(&snapshot), 44, &HashSet::new(), true);
        assert_eq!(view.cards[0].agent_id, "agt_parent");
        assert_eq!(view.cards[1].agent_id, "agt_child");
        assert_eq!(view.cards[1].depth, 2);
    }

    #[test]
    fn unknown_usage_and_configured_fallback_are_explicit() {
        let mut item = agent("agt_unknown", "agt_root", "route check", "running");
        item["usage"] = json!({"status": "not_reported"});
        item["elapsed_millis"] = json!({"status": "not_reported"});
        item["resolved_route"]["resolution"] = json!("configured_fallback");
        item["resolved_route"]["fallback_reason"] = json!("requested route unavailable");
        let snapshot = snapshot(vec![item]);
        let expanded = HashSet::from(["agt_unknown".to_string()]);
        let view = project_agent_tree(Some(&snapshot), 160, &expanded, true);
        let text = view.cards[0].lines.join("\n");

        assert!(text.contains("usage n/r"));
        assert!(text.contains("elapsed not reported"));
        assert!(text.contains("configured_fallback: requested route unavailable"));
    }

    #[test]
    fn simultaneous_approvals_are_prioritized_and_attributed() {
        let mut quiet = agent("agt_quiet", "agt_root", "quiet", "running");
        quiet["attention"] = json!({
            "required": true,
            "pending_approvals": 0,
            "pending_messages": 1,
            "pending_notifications": 0
        });
        let mut approval = agent(
            "agt_approval",
            "agt_root",
            "needs permission",
            "waiting_for_user",
        );
        approval["attention"] = json!({
            "required": true,
            "pending_approvals": 2,
            "pending_messages": 0,
            "pending_notifications": 0
        });
        approval["approvals"] = json!([{
            "request_event_id": "evt_approval",
            "operation_id": "op_approval",
            "state": "pending",
            "tool_name": "bash",
            "effect": "execute",
            "reason": "run focused tests",
            "created_at": "2026-01-01T00:00:00Z",
            "deadline_at": "2026-01-01T00:10:00Z",
            "decision": null,
            "resolution_source": null,
            "resolution_reason": null
        }]);
        let mut snapshot = snapshot(vec![quiet, approval]);
        snapshot.pending_attention = 3;
        let expanded = HashSet::from(["agt_approval".to_string()]);

        let cards = project_inline_agent_cards(&snapshot, 80, &expanded);
        assert_eq!(cards[0].agent_id, "agt_approval");
        assert!(cards[0].lines.join("\n").contains("2 approval"));
        assert!(cards[0].lines.join("\n").contains("bash (execute)"));

        let prompt = project_agent_approval_prompt(&snapshot).unwrap();
        assert!(prompt.summary.contains("Child: needs permission"));
        assert!(prompt.summary.contains("Ancestry: root › needs permission"));
        assert!(prompt
            .summary
            .contains("requested codex/requested/medium → effective codex/effective/high"));
        assert!(prompt.summary.contains("Tool: bash · effect execute"));
        assert!(prompt
            .summary
            .contains("Workspace: snapshot · ovim · root not reported"));
        assert!(prompt.summary.contains("Reason: run focused tests"));
    }

    #[test]
    fn interrupted_recovery_has_restart_specific_treatment() {
        let mut item = agent("agt_restart", "agt_root", "recover work", "interrupted");
        item["recovery_status"] = json!("interrupted_after_restart");
        let snapshot = snapshot(vec![item]);
        let expanded = HashSet::from(["agt_restart".to_string()]);
        let view = project_agent_tree(Some(&snapshot), 72, &expanded, true);

        assert_eq!(view.cards[0].tone, AgentTone::Interrupted);
        let text = view.cards[0].lines.join("\n");
        assert!(text.contains("interrupted/restart"));
        assert!(text.contains("follow-up can restart"));
    }

    #[test]
    fn completed_inline_card_bounds_handoff_and_shows_evidence_and_artifacts() {
        let mut item = agent("agt_done", "agt_root", "report findings", "completed");
        item["handoff"] = json!({
            "event_id": "evt_handoff",
            "status": "completed",
            "summary": "A deliberately long bounded handoff summary with enough detail to truncate in the compact card without hiding its state.",
            "evidence": [{"path": "src/lib.rs", "line": 12, "claim": "entry point"}],
            "changed_files": [],
            "blockers": [],
            "followups": [],
            "confidence": "high"
        });
        item["artifact_handles"] = json!([{
            "artifact_id": "art_report",
            "state": {"type": "missing", "reason": "fixture"},
            "media_type": "text/markdown",
            "retention": "run",
            "export_policy": "include"
        }]);
        let snapshot = snapshot(vec![item]);
        let expanded = HashSet::from(["agt_done".to_string()]);
        let cards = project_inline_agent_cards(&snapshot, 48, &expanded);
        let text = cards[0].lines.join("\n");

        assert!(text.contains("handoff completed · 1e · 1a"));
        assert!(cards[0]
            .lines
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 47));
    }

    #[test]
    fn narrow_real_buffer_stays_bounded_and_preserves_state() {
        let snapshot = snapshot(vec![agent(
            "agt_narrow",
            "agt_root",
            "an extremely long delegated task name",
            "failed",
        )]);
        let text = buffer_text(Some(&snapshot), 18, 7);

        assert!(text.contains("Agents 1"));
        assert!(text.contains('×'));
        assert_eq!(text.lines().count(), 7);
        assert!(text.lines().all(|line| UnicodeWidthStr::width(line) <= 18));
    }

    #[test]
    fn empty_and_disabled_states_render_to_real_buffer() {
        let empty = snapshot(Vec::new());
        assert!(buffer_text(Some(&empty), 28, 6).contains("No delegated agents"));

        let backend = TestBackend::new(28, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let expanded = HashSet::new();
        terminal
            .draw(|frame| {
                render_agent_tree_panel(
                    frame,
                    None,
                    frame.area(),
                    AgentTreeRenderState {
                        enabled: false,
                        ..AgentTreeRenderState::enabled(&expanded)
                    },
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = (0..6)
            .map(|y| (0..28).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Delegation disabled"));
    }
}
