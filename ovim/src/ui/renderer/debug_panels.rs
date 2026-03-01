//! Debug panel rendering — stack trace, variables, and output.
//!
//! Shown when `debug_state.panels_visible` is true and a debug session is active.

use crate::editor::Editor;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Render the debug side panel (stack trace + variables).
pub fn render_debug_side_panel(frame: &mut Frame, editor: &Editor, area: Rect) {
    // Split vertically: stack trace (top 40%) + variables (bottom 60%)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    render_stack_trace(frame, editor, chunks[0]);
    render_variables(frame, editor, chunks[1]);
}

/// Render the debug output panel (bottom).
pub fn render_debug_output(frame: &mut Frame, editor: &Editor, area: Rect) {
    let state = editor.debug_state();

    let title = format!(" Output ({}) ", state.output_lines.len());
    let block = Block::default()
        .borders(Borders::TOP)
        .title(title)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Show the last N lines that fit
    let visible_height = inner.height as usize;
    let start = state.output_lines.len().saturating_sub(visible_height);
    let lines: Vec<Line> = state.output_lines[start..]
        .iter()
        .map(|l| Line::from(l.as_str().to_owned()))
        .collect();

    let paragraph = Paragraph::new(lines).style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, inner);
}

/// Render the stack trace section.
fn render_stack_trace(frame: &mut Frame, editor: &Editor, area: Rect) {
    let state = editor.debug_state();

    let status = if !state.session_active {
        "inactive"
    } else if state.is_running {
        "running"
    } else if let Some(ref reason) = state.stop_reason {
        reason.as_str()
    } else {
        "stopped"
    };

    let title = format!(" Call Stack ({}) ", status);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.stack_frames.is_empty() {
        let msg = if state.is_running {
            "Running..."
        } else if !state.session_active {
            "No debug session"
        } else {
            "No stack trace"
        };
        let paragraph =
            Paragraph::new(msg).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, inner);
        return;
    }

    let lines: Vec<Line> = state
        .stack_frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let source = f
                .source
                .as_ref()
                .and_then(|s| s.name.as_deref())
                .unwrap_or("?");
            let text = format!("{} {}:{}", f.name, source, f.line);

            let style = if i == state.selected_frame {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let marker = if i == state.selected_frame {
                "> "
            } else {
                "  "
            };
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(text, style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Render the variables section.
fn render_variables(frame: &mut Frame, editor: &Editor, area: Rect) {
    let state = editor.debug_state();

    let title = " Variables ";
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.scopes.is_empty() {
        let paragraph =
            Paragraph::new("No variables").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    for scope in &state.scopes {
        // Scope header
        lines.push(Line::from(Span::styled(
            format!("{}:", scope.name),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));

        // Variables in this scope (with recursive expansion)
        if let Some(vars) = state.variables.get(&scope.variables_reference) {
            for var in vars {
                render_variable_tree(&mut lines, state, var, 1);
            }
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Recursively render a variable and its children if expanded.
fn render_variable_tree<'a>(
    lines: &mut Vec<Line<'a>>,
    state: &ovim_core::dap::state::DebugState,
    var: &ovim_core::dap::types::DapVariable,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    let expand_marker = if var.variables_reference > 0 {
        if state.expanded_refs.contains(&var.variables_reference) {
            "▾ "
        } else {
            "▸ "
        }
    } else {
        "  "
    };

    let type_str = var
        .type_
        .as_deref()
        .map(|t| format!(" ({})", t))
        .unwrap_or_default();

    lines.push(Line::from(vec![
        Span::styled(
            format!("{}{}{}", indent, expand_marker, var.name),
            Style::default().fg(Color::White),
        ),
        Span::styled(" = ", Style::default().fg(Color::DarkGray)),
        Span::styled(var.value.clone(), Style::default().fg(Color::Green)),
        Span::styled(type_str, Style::default().fg(Color::DarkGray)),
    ]));

    if var.variables_reference > 0
        && state.expanded_refs.contains(&var.variables_reference)
        && depth < 10
    {
        if let Some(children) = state.variables.get(&var.variables_reference) {
            for child in children {
                render_variable_tree(lines, state, child, depth + 1);
            }
        }
    }
}
