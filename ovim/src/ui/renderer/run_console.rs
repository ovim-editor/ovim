//! Run console rendering: the bottom panel showing build / run / debug output.
//!
//! The panel follows live output (newest line at the bottom) until scrolled;
//! `<Space>rf` focuses it so lines can be selected and stack frames opened.
//! Each stream has its own colour so stderr and build noise stand apart from
//! program output, and jumpable lines (stack frames, compiler errors) are
//! underlined.

use ovim_core::launch::{ConsoleLine, LineKind, RunOutcome, RunPhase, RunRecord, RunStatus};
use ovim_core::mode::Mode;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::editor::Editor;

mod colors {
    use ratatui::style::Color;

    pub const OK: Color = Color::Rgb(166, 227, 161);
    pub const FAIL: Color = Color::Rgb(243, 139, 168);
    pub const ACTIVE: Color = Color::Rgb(249, 226, 175);
    pub const MUTED: Color = Color::Rgb(127, 132, 156);
    pub const TEXT: Color = Color::Rgb(205, 214, 244);
    pub const BUILD: Color = Color::Rgb(137, 180, 250);
    pub const DEBUGGER: Color = Color::Rgb(203, 166, 247);
    pub const CURSOR_BG: Color = Color::Rgb(49, 50, 68);
}

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn status_style(run: &RunRecord) -> (String, Color) {
    match &run.status {
        RunStatus::Active(RunPhase::Resolving) | RunStatus::Active(RunPhase::Building) => {
            (spinner(run), colors::ACTIVE)
        }
        RunStatus::Active(_) => (spinner(run), colors::ACTIVE),
        RunStatus::Done(RunOutcome::Succeeded) => ("✓".to_string(), colors::OK),
        RunStatus::Done(RunOutcome::Stopped) | RunStatus::Done(RunOutcome::Ended) => {
            ("■".to_string(), colors::MUTED)
        }
        RunStatus::Done(_) => ("✗".to_string(), colors::FAIL),
    }
}

fn spinner(run: &RunRecord) -> String {
    let idx = (run.started.elapsed().as_millis() / 100) as usize % SPINNER.len();
    SPINNER[idx].to_string()
}

fn line_style(line: &ConsoleLine) -> Style {
    let base = match line.kind {
        LineKind::Stdout => Style::default().fg(colors::TEXT),
        LineKind::Stderr => Style::default().fg(colors::FAIL),
        LineKind::Build => Style::default().fg(colors::BUILD),
        LineKind::System => Style::default()
            .fg(colors::MUTED)
            .add_modifier(Modifier::ITALIC),
        LineKind::Debugger => Style::default().fg(colors::DEBUGGER),
    };
    if line.location.is_some() {
        base.add_modifier(Modifier::UNDERLINED)
    } else {
        base
    }
}

pub fn render_run_console(frame: &mut Frame, editor: &Editor, area: Rect) {
    let console = editor.run_console();
    let focused = editor.mode() == Mode::RunConsole;

    let mut title: Vec<Span> = vec![Span::styled(
        " Run ",
        Style::default()
            .fg(colors::TEXT)
            .add_modifier(Modifier::BOLD),
    )];
    match console.viewed() {
        Some(run) => {
            let (icon, color) = status_style(run);
            title.push(Span::styled(
                format!("{icon} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
            title.push(Span::styled(
                format!("{} · {} ", run.title, run.status_text()),
                Style::default().fg(color),
            ));
            if console.runs.len() > 1 {
                let idx = console.viewed_index().map(|i| i + 1).unwrap_or(1);
                title.push(Span::styled(
                    format!("[{idx}/{}] ", console.runs.len()),
                    Style::default().fg(colors::MUTED),
                ));
            }
            if console.scroll > 0 {
                title.push(Span::styled(
                    format!("↑{} ", console.scroll),
                    Style::default().fg(colors::ACTIVE),
                ));
            }
        }
        None => title.push(Span::styled(
            "· nothing has run yet ",
            Style::default().fg(colors::MUTED),
        )),
    }
    if focused {
        title.push(Span::styled(
            "· j/k scroll  Enter open  [ ] runs  r rerun  s stop  x clear  q back ",
            Style::default().fg(colors::MUTED),
        ));
    }

    let block = Block::default()
        .borders(Borders::TOP)
        .title(Line::from(title))
        .border_style(Style::default().fg(if focused {
            colors::BUILD
        } else {
            Color::DarkGray
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let Some(run) = console.viewed() else {
        let hint = [
            "  <Space>rr  run the code at the cursor",
            "  <Space>rd  debug it (F5)",
            "  <Space>rl  rerun the last run",
            "  <Space>rc  choose a configuration (.ovim/debug.toml)",
        ];
        let lines: Vec<Line> = hint
            .iter()
            .map(|h| Line::from(Span::styled(*h, Style::default().fg(colors::MUTED))))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    };

    let height = inner.height as usize;
    let top = console.top_line(height);
    let lines: Vec<Line> = run
        .lines
        .iter()
        .enumerate()
        .skip(top)
        .take(height)
        .map(|(idx, line)| {
            let mut style = line_style(line);
            let mut text = line.text.replace('\t', "    ");
            if focused && idx == console.cursor {
                style = style.bg(colors::CURSOR_BG);
                // Pad so the highlight spans the row.
                let pad = (inner.width as usize).saturating_sub(text.chars().count());
                text.push_str(&" ".repeat(pad));
            }
            Line::from(Span::styled(text, style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}
