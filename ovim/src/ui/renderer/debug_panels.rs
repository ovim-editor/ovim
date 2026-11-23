//! Debug panel rendering: call stack, variables, watches, breakpoints and
//! exception filters as one scrollable list (see `ovim_core::dap::panel`).
//!
//! Shown while `debug_state.panels_visible` is true. `<Space>df` focuses it
//! (mode `DEBUG`): the cursor row is highlighted and the list scrolls to keep
//! it visible. Output of the debuggee lives in the run console.

use crate::editor::Editor;
use ovim_core::dap::panel::{PanelRow, RowKind};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn render_debug_side_panel(frame: &mut Frame, editor: &Editor, area: Rect) {
    let focused = editor.mode() == crate::mode::Mode::DebugPanel;
    let state = editor.debug_state();
    let title = if focused {
        " Debug  j/k move · Enter act · d del · e toggle · a watch · </> resize "
    } else {
        " Debug "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 4 || inner.height < 1 {
        return;
    }

    let rows = ovim_core::dap::panel::rows(state);
    let height = inner.height as usize;
    // Scroll to keep the cursor on screen (the renderer owns the view height).
    let mut cursor = state.panel.cursor;
    let mut scroll = state.panel.scroll.get();
    ovim_core::dap::panel::clamp_view(&rows, &mut cursor, &mut scroll, height);
    editor.debug_panel_view(height, scroll);

    let width = inner.width as usize;
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(height)
        .map(|(index, row)| row_line(row, focused && index == cursor, width))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn row_line(row: &PanelRow, highlighted: bool, width: usize) -> Line<'static> {
    let indent = "  ".repeat(row.depth);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let base = Style::default().fg(Color::White);
    match &row.kind {
        RowKind::Header => {
            return Line::from(Span::styled(
                row.label.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        RowKind::Note => {
            return Line::from(Span::styled(
                format!("{indent}{}", row.label),
                Style::default().fg(Color::DarkGray),
            ));
        }
        RowKind::Frame { selected, .. } => {
            let style = if *selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                base
            };
            spans.push(Span::styled(
                format!("{}{}", if *selected { "> " } else { "  " }, row.label),
                style,
            ));
        }
        RowKind::Variable { var_ref, expanded }
        | RowKind::Watch {
            var_ref, expanded, ..
        } => {
            let marker = match (*var_ref > 0, *expanded) {
                (true, true) => "▾ ",
                (true, false) => "▸ ",
                _ => "  ",
            };
            spans.push(Span::styled(format!("{indent}{marker}{}", row.label), base));
            spans.push(Span::styled(" = ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                row.value.clone().unwrap_or_default(),
                Style::default().fg(Color::Green),
            ));
            if let Some(type_) = &row.type_ {
                spans.push(Span::styled(
                    format!(" ({type_})"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        RowKind::Breakpoint {
            enabled,
            verified,
            conditional,
            ..
        } => {
            let (glyph, color) = match (*enabled, *conditional) {
                (false, _) => ("○", Color::DarkGray),
                (true, true) => ("◆", Color::Red),
                (true, false) if *verified => ("●", Color::Red),
                (true, false) => ("●", Color::Rgb(180, 100, 100)),
            };
            spans.push(Span::styled(
                format!("{indent}{glyph} "),
                Style::default().fg(color),
            ));
            spans.push(Span::styled(
                row.label.clone(),
                if *enabled {
                    base
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            ));
            if let Some(condition) = &row.value {
                spans.push(Span::styled(
                    format!("  {condition}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        RowKind::Thread { selected, .. } => {
            let style = if *selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                base
            };
            spans.push(Span::styled(
                format!(
                    "{indent}{}{}",
                    if *selected { "> " } else { "  " },
                    row.label
                ),
                style,
            ));
            if let Some(note) = &row.value {
                spans.push(Span::styled(
                    format!("  {note}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        RowKind::Exception { enabled, .. } => {
            spans.push(Span::styled(
                format!(
                    "{indent}{} {}",
                    if *enabled { "[x]" } else { "[ ]" },
                    row.label
                ),
                base,
            ));
        }
    }
    if highlighted {
        // Pad to the panel width so the highlight covers the whole row.
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        if used < width {
            spans.push(Span::raw(" ".repeat(width - used)));
        }
        for span in &mut spans {
            span.style = span.style.bg(Color::Rgb(50, 55, 80));
        }
    }
    Line::from(spans)
}
