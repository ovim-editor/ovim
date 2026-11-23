//! "Replace in files" overlay: find / replace / files inputs, option toggles
//! and the grouped, checkable match list with an inline replacement preview.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::editor::search_replace::{ReviewRow, SearchReplaceField, SearchReplacePanel};

mod colors {
    use ratatui::style::Color;

    pub const BORDER: Color = Color::Rgb(80, 85, 110);
    pub const TITLE: Color = Color::Rgb(140, 160, 240);
    pub const SELECTED_BG: Color = Color::Rgb(45, 50, 70);
    pub const TEXT: Color = Color::Rgb(200, 205, 215);
    pub const MUTED: Color = Color::Rgb(100, 110, 140);
    pub const BRIGHT: Color = Color::Rgb(240, 240, 255);
    pub const GREEN: Color = Color::Rgb(80, 200, 120);
    pub const RED: Color = Color::Rgb(220, 80, 80);
    pub const YELLOW: Color = Color::Rgb(230, 200, 80);
    pub const KEY: Color = Color::Rgb(140, 160, 240);
    pub const INPUT_BG: Color = Color::Rgb(35, 38, 52);
}

const LABEL_WIDTH: u16 = 10;

pub fn get_search_replace_area(full_area: Rect) -> Rect {
    let width = ((full_area.width * 90) / 100).max(60).min(full_area.width);
    let height = ((full_area.height * 85) / 100)
        .max(14)
        .min(full_area.height);
    let x = full_area.width.saturating_sub(width) / 2;
    let y = full_area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

/// Where the hardware cursor goes for the focused input field, if any.
pub fn cursor_position(frame_area: Rect, panel: &SearchReplacePanel) -> Option<(u16, u16)> {
    let area = get_search_replace_area(frame_area);
    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    let (row, input) = match panel.focus {
        SearchReplaceField::Find => (0, &panel.find),
        SearchReplaceField::Replace => (1, &panel.replace),
        SearchReplaceField::Files => (2, &panel.files),
        SearchReplaceField::Results => return None,
    };
    let width = UnicodeWidthStr::width(&input.text()[..input.cursor()]) as u16;
    let x = (inner.x + LABEL_WIDTH + width).min(area.right().saturating_sub(2));
    Some((x, inner.y + row))
}

pub fn render_search_replace(frame: &mut Frame, panel: &SearchReplacePanel) {
    let area = get_search_replace_area(frame.area());
    frame.render_widget(Clear, area);

    let summary = if let Some(error) = &panel.error {
        format!(" {error} ")
    } else if panel.searching {
        " searching... ".to_string()
    } else if panel.searched {
        format!(
            " {} of {} checked in {} files{} ",
            panel.checked_matches(),
            panel.total_matches(),
            panel.results.len(),
            if panel.truncated { " (truncated)" } else { "" }
        )
    } else {
        String::new()
    };
    let block = Block::default()
        .title_top(Line::from(Span::styled(
            "  Replace in Files ",
            Style::default()
                .fg(colors::TITLE)
                .add_modifier(Modifier::BOLD),
        )))
        .title_top(
            Line::from(Span::styled(
                summary,
                Style::default().fg(if panel.error.is_some() {
                    colors::RED
                } else {
                    colors::MUTED
                }),
            ))
            .right_aligned(),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors::BORDER));
    frame.render_widget(&block, area);
    let inner = block.inner(area);
    if inner.height < 6 || inner.width < 30 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    render_input(
        frame,
        chunks[0],
        "Find",
        panel.find.text(),
        panel.focus == SearchReplaceField::Find,
        Some(option_spans(panel)),
    );
    render_input(
        frame,
        chunks[1],
        "Replace",
        panel.replace.text(),
        panel.focus == SearchReplaceField::Replace,
        None,
    );
    render_input(
        frame,
        chunks[2],
        "Files",
        panel.files.text(),
        panel.focus == SearchReplaceField::Files,
        None,
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(chunks[3].width as usize),
            Style::default().fg(colors::BORDER),
        ))),
        chunks[3],
    );
    render_results(frame, panel, chunks[4]);
    render_hints(frame, panel, chunks[5]);
}

fn option_spans(panel: &SearchReplacePanel) -> Vec<Span<'static>> {
    let toggle = |label: &'static str, on: bool| {
        Span::styled(
            format!(" {label} "),
            if on {
                Style::default()
                    .fg(Color::Black)
                    .bg(colors::YELLOW)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors::MUTED)
            },
        )
    };
    vec![
        toggle("Aa", panel.case_sensitive),
        toggle("W", panel.whole_word),
        toggle(".*", panel.regex),
    ]
}

fn render_input(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    text: &str,
    focused: bool,
    trailing: Option<Vec<Span<'static>>>,
) {
    let mut spans = vec![Span::styled(
        format!("{label:<width$}", width = LABEL_WIDTH as usize),
        Style::default().fg(if focused { colors::KEY } else { colors::MUTED }),
    )];
    spans.push(Span::styled(
        text.to_string(),
        Style::default().fg(colors::BRIGHT),
    ));
    let mut line = Line::from(spans);
    if let Some(trailing) = trailing {
        // Right-align the toggles by padding the gap.
        let used = LABEL_WIDTH as usize + UnicodeWidthStr::width(text);
        let toggles_width: usize = trailing.iter().map(|span| span.width()).sum();
        let gap = (area.width as usize).saturating_sub(used + toggles_width);
        line.spans.push(Span::raw(" ".repeat(gap)));
        line.spans.extend(trailing);
    }
    let bg = if focused {
        colors::INPUT_BG
    } else {
        Color::Reset
    };
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), area);
}

fn render_results(frame: &mut Frame, panel: &SearchReplacePanel, area: Rect) {
    if panel.results.is_empty() {
        let message = if panel.find.is_empty() {
            "Type something to find"
        } else if panel.searching {
            "Searching..."
        } else if panel.error.is_some() {
            ""
        } else {
            "No matches"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  {message}"),
                Style::default().fg(colors::MUTED),
            ))),
            area,
        );
        return;
    }

    let rows = panel.rows();
    let height = area.height as usize;
    let selected = panel.selected.min(rows.len().saturating_sub(1));
    let start = if selected >= height {
        selected + 1 - height
    } else {
        0
    };
    for (offset, row) in rows.iter().skip(start).take(height).enumerate() {
        let index = start + offset;
        let is_selected = index == selected;
        let line = match *row {
            ReviewRow::File(file) => {
                let file = &panel.results[file];
                let all = file.checked_count() == file.matches.len();
                let none = file.checked_count() == 0;
                let mark = if all {
                    "[x]"
                } else if none {
                    "[ ]"
                } else {
                    "[-]"
                };
                Line::from(vec![
                    Span::styled(format!(" {mark} "), Style::default().fg(colors::KEY)),
                    Span::styled(
                        file.rel.clone(),
                        Style::default()
                            .fg(colors::BRIGHT)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  {}", file.matches.len()),
                        Style::default().fg(colors::MUTED),
                    ),
                ])
            }
            ReviewRow::Match(file, m) => {
                let entry = &panel.results[file].matches[m];
                let replacement = panel.replacement_for(&entry.found);
                let text = &entry.found.line_text;
                let (before, matched, after) = (
                    &text[..entry.found.start_byte],
                    &text[entry.found.start_byte..entry.found.end_byte],
                    &text[entry.found.end_byte..],
                );
                let mark = if entry.checked { "[x]" } else { "[ ]" };
                let dim = !entry.checked;
                let plain = Style::default().fg(if dim { colors::MUTED } else { colors::TEXT });
                let removed = if dim {
                    plain
                } else {
                    Style::default()
                        .fg(colors::RED)
                        .add_modifier(Modifier::CROSSED_OUT)
                };
                let added = if dim {
                    plain
                } else {
                    Style::default().fg(colors::GREEN)
                };
                // A tab cell draws as nothing: expand tabs across the pieces
                // as if they were one row.
                let show_replacement = !panel.replace.is_empty() || !matched.is_empty();
                let [before, matched, replacement, after] = expand_preview_pieces([
                    before.trim_start(),
                    matched,
                    if show_replacement { &replacement } else { "" },
                    after.trim_end(),
                ]);
                let mut spans = vec![
                    Span::styled(format!("   {mark} "), Style::default().fg(colors::KEY)),
                    Span::styled(
                        format!("{:>5}  ", entry.found.line + 1),
                        Style::default().fg(colors::MUTED),
                    ),
                    Span::styled(before, plain),
                    Span::styled(matched, removed),
                ];
                if show_replacement {
                    spans.push(Span::styled(replacement, added));
                }
                spans.push(Span::styled(after, plain));
                Line::from(spans)
            }
        };
        let style = if is_selected {
            Style::default().bg(colors::SELECTED_BG)
        } else {
            Style::default()
        };
        frame.render_widget(
            Paragraph::new(line).style(style),
            Rect::new(area.x, area.y + offset as u16, area.width, 1),
        );
    }
}

/// Tab stops of the match preview rows.
const PREVIEW_TAB_WIDTH: usize = 4;

/// Expands tabs (and makes control characters visible) in the consecutive
/// pieces of one preview row, so a tab inside a later piece reaches the stop
/// the earlier pieces leave it at.
fn expand_preview_pieces(pieces: [&str; 4]) -> [String; 4] {
    // Everything before the current piece, already expanded.
    let mut row = String::new();
    pieces.map(|piece| {
        let expanded = super::helpers::expand_tabs(&format!("{row}{piece}"), PREVIEW_TAB_WIDTH);
        let shown = expanded[row.len()..].to_string();
        row = expanded;
        shown
    })
}

fn render_hints(frame: &mut Frame, panel: &SearchReplacePanel, area: Rect) {
    let hints: &[(&str, &str)] = if panel.focus == SearchReplaceField::Results {
        &[
            ("Space", "toggle"),
            ("a", "all"),
            ("Enter", "open"),
            ("A", "replace"),
            ("Tab", "field"),
            ("Esc", "close"),
        ]
    } else {
        &[
            ("Tab", "field"),
            ("C-t", "toggle"),
            ("C-a", "all"),
            ("M-c/w/r", "case/word/regex"),
            ("M-Enter", "replace"),
            ("Esc", "close"),
        ]
    };
    let mut spans = Vec::new();
    for (key, text) in hints {
        spans.push(Span::styled(
            format!(" {key}"),
            Style::default().fg(colors::KEY),
        ));
        spans.push(Span::styled(
            format!(" {text} "),
            Style::default().fg(colors::MUTED),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::search_replace::SearchReplacePanel;
    use ratatui::{backend::TestBackend, Terminal};

    fn screen(panel: &SearchReplacePanel, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render_search_replace(frame, panel))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn panel_shows_inputs_toggles_and_a_replacement_preview_per_match() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join("A.java"), "new Circle();\nCircle c;\n").unwrap();
        let mut editor = crate::editor::Editor::default();
        editor.open_file(root.join("A.java")).unwrap();
        editor.open_search_replace(Some("Circle".to_string()));
        editor.run_search_replace_now();
        let panel = editor.search_replace_panel_mut().unwrap();
        panel.replace = crate::editor::SingleLineInput::new("Disc");
        panel.selected = 2;
        panel.toggle_selected();

        let text = screen(editor.search_replace_panel().unwrap(), 100, 24);
        assert!(text.contains("Replace in Files"), "{text}");
        assert!(text.contains("Find      Circle"), "{text}");
        assert!(text.contains("Replace   Disc"), "{text}");
        assert!(text.contains("1 of 2 checked in 1 files"), "{text}");
        assert!(text.contains("A.java"), "{text}");
        // Inline preview: the old text, then the new text right after it.
        assert!(text.contains("new CircleDisc();"), "{text}");
        assert!(text.contains("[x]"), "{text}");
        assert!(text.contains("[ ]"), "{text}");
        assert!(text.contains("M-Enter replace"), "{text}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_match_preview_keeps_the_gap_where_the_source_has_a_tab() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join("Tabs.go"), "x\tCircle\ty\n").unwrap();
        let mut editor = crate::editor::Editor::default();
        editor.open_file(root.join("Tabs.go")).unwrap();
        editor.open_search_replace(Some("Circle".to_string()));
        editor.run_search_replace_now();
        let panel = editor.search_replace_panel_mut().unwrap();
        panel.replace = crate::editor::SingleLineInput::new("Disc");

        let text = screen(editor.search_replace_panel().unwrap(), 100, 24);
        assert!(text.contains("x   CircleDisc  y"), "{text}");
    }

    #[test]
    fn preview_tabs_reach_their_tab_stops_across_the_pieces() {
        let [before, matched, replacement, after] =
            expand_preview_pieces(["a\tb", "\tOld", "New", "\tend"]);
        // `a`, tab to column 4, `b`, tab to 8, then the pieces continue.
        assert_eq!(before, "a   b");
        assert_eq!(matched, "   Old");
        assert_eq!(replacement, "New");
        assert_eq!(after, "  end");
        assert_eq!(
            format!("{before}{matched}{replacement}{after}"),
            "a   b   OldNew  end"
        );
    }

    #[test]
    fn cursor_sits_at_the_end_of_the_focused_input() {
        let mut panel = SearchReplacePanel::new(std::path::PathBuf::from("."));
        panel.find = crate::editor::SingleLineInput::new("abc");
        panel.find.move_end();
        let area = Rect::new(0, 0, 100, 30);
        let (x, y) = cursor_position(area, &panel).unwrap();
        let inner = get_search_replace_area(area);
        assert_eq!(y, inner.y + 1);
        assert_eq!(x, inner.x + 1 + LABEL_WIDTH + 3);
        panel.focus = SearchReplaceField::Results;
        assert!(cursor_position(area, &panel).is_none());
    }
}
