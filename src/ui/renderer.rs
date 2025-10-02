use crate::buffer::Buffer;
use crate::editor::Editor;
use crate::syntax::Theme;
use anyhow::Result;
use crossterm::cursor::SetCursorStyle;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal as RatatuiTerminal,
};
use std::io;

/// Handles rendering the editor state to the terminal
pub struct Renderer {
    terminal: RatatuiTerminal<CrosstermBackend<io::Stdout>>,
    theme: Theme,
}

impl Renderer {
    /// Creates a new renderer
    pub fn new() -> Self {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = RatatuiTerminal::new(backend).expect("Failed to create terminal");
        Self {
            terminal,
            theme: Theme::default(),
        }
    }

    /// Renders editor to a frame (used by both TUI and headless rendering)
    pub fn render_to_frame(frame: &mut Frame, editor: &Editor) {
        let theme = Theme::default();
        let cursor_pos = editor.buffer().cursor();
        let cursor_line = cursor_pos.line();
        let cursor_col = cursor_pos.col();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
            .split(frame.area());

        // Render the main text area
        let viewport_start = Self::render_buffer(frame, editor, &theme, chunks[0]);

        // Render the status line or command line or search line
        if editor.mode() == crate::mode::Mode::Command {
            Self::render_command_line(frame, editor, chunks[1]);
        } else if editor.mode() == crate::mode::Mode::Search {
            Self::render_search_line(frame, editor, chunks[1]);
        } else {
            Self::render_status_line(frame, editor, chunks[1]);
        }

        // Render picker overlay if in Picker mode
        if editor.mode() == crate::mode::Mode::Picker {
            Self::render_picker(frame, editor, frame.area());
        }

        // Set hardware cursor position
        if editor.mode() == crate::mode::Mode::Picker {
            // Position cursor in picker query line (after the query text)
            if let Some(picker) = editor.picker() {
                let query_len = picker.query().len();
                let picker_area = Self::get_picker_area(frame.area());
                frame.set_cursor_position((
                    picker_area.x + 1 + 2 + query_len as u16, // +1 for border, +2 for "> " prefix
                    picker_area.y + 1, // +1 for border
                ));
            }
        } else if editor.mode() == crate::mode::Mode::Command {
            // Position cursor in command line
            let cmd_cursor_x = (editor.command_line().len() + 1).min(chunks[1].width.saturating_sub(1) as usize);
            frame.set_cursor_position((
                chunks[1].x + cmd_cursor_x as u16,
                chunks[1].y,
            ));
        } else if editor.mode() == crate::mode::Mode::Search {
            // Position cursor in search line
            let search_cursor_x = (editor.search_buffer().len() + 1).min(chunks[1].width.saturating_sub(1) as usize);
            frame.set_cursor_position((
                chunks[1].x + search_cursor_x as u16,
                chunks[1].y,
            ));
        } else {
            // Position cursor in text buffer
            let screen_line = cursor_line.saturating_sub(viewport_start);
            let cursor_x = cursor_col.min(chunks[0].width.saturating_sub(1) as usize);
            let cursor_y = screen_line.min(chunks[0].height.saturating_sub(1) as usize);
            frame.set_cursor_position((
                chunks[0].x + cursor_x as u16,
                chunks[0].y + cursor_y as u16,
            ));
        }
    }

    /// Renders the editor state to the terminal
    pub fn render(&mut self, editor: &Editor) -> Result<()> {
        // Set cursor style based on mode
        let cursor_style = match editor.mode() {
            crate::mode::Mode::Insert => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Picker => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Command => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Search => SetCursorStyle::BlinkingBar,
            _ => SetCursorStyle::SteadyBlock,
        };
        crossterm::execute!(io::stdout(), cursor_style)?;

        self.terminal.draw(|frame| {
            Self::render_to_frame(frame, editor);
        })?;
        Ok(())
    }

    /// Renders the buffer content and returns the viewport start line
    fn render_buffer(frame: &mut Frame, editor: &Editor, theme: &Theme, area: Rect) -> usize {
        let buffer = editor.buffer();
        let rope = buffer.rope();
        let cursor = buffer.cursor();

        // Calculate visible range
        let visible_lines = area.height as usize;
        let start_line = cursor.line().saturating_sub(visible_lines / 2);
        let end_line = (start_line + visible_lines).min(rope.len_lines());

        // Get visual selection if in visual mode
        let visual_selection = if editor.mode().is_visual() {
            editor.visual_selection()
        } else {
            None
        };

        // Get current search if active
        let current_search = editor.current_search();

        // Build the visible text with syntax highlighting
        let mut lines = Vec::new();
        for line_idx in start_line..end_line {
            if line_idx < rope.len_lines() {
                let line_text = rope.line(line_idx).to_string();
                // Remove trailing newline if present
                let line_text = line_text.trim_end_matches('\n');

                // Get syntax highlights for this line
                let syntax_highlights = buffer.highlights_for_line(line_idx);

                // Check if we need special highlighting (visual selection or search)
                let has_visual_selection = visual_selection
                    .map(|((start_line, _), (end_line, _))| line_idx >= start_line && line_idx <= end_line)
                    .unwrap_or(false);

                let search_matches = if let Some(search) = current_search {
                    search.find_all_in_line(line_text)
                } else {
                    Vec::new()
                };

                // Always use character-by-character rendering if we have any highlighting
                let needs_detailed_rendering = has_visual_selection || !search_matches.is_empty() || !syntax_highlights.is_empty();

                if needs_detailed_rendering {
                    let line = Self::render_line_with_highlights(
                        theme,
                        line_text,
                        line_idx,
                        visual_selection,
                        &search_matches,
                        &syntax_highlights,
                    );
                    lines.push(line);
                } else {
                    lines.push(Line::from(line_text.to_string()));
                }
            }
        }

        let paragraph = Paragraph::new(lines)
            .block(Block::default().borders(Borders::NONE));
        frame.render_widget(paragraph, area);

        start_line
    }

    /// Renders the status line
    fn render_status_line(frame: &mut Frame, editor: &Editor, area: Rect) {
        let mode = editor.mode();
        let buffer = editor.buffer();
        let cursor = buffer.cursor();

        // Build status line content
        let mode_indicator = format!(" {} ", mode.display_name());
        let position = format!(" {}:{} ", cursor.line() + 1, cursor.col() + 1);
        let modified = if buffer.is_modified() { " [+] " } else { " " };
        let file = buffer.file_path().unwrap_or("[No Name]");

        let padding_len = (area.width as usize)
            .saturating_sub(mode_indicator.len())
            .saturating_sub(file.len())
            .saturating_sub(modified.len())
            .saturating_sub(position.len())
            .saturating_sub(1);

        let status_line = Line::from(vec![
            Span::styled(
                &mode_indicator,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::raw(file),
            Span::raw(modified),
            Span::raw(" ".repeat(padding_len)),
            Span::styled(
                position,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Gray),
            ),
        ]);

        let paragraph = Paragraph::new(status_line)
            .style(Style::default().bg(Color::DarkGray));
        frame.render_widget(paragraph, area);
    }

    /// Renders the command line
    fn render_command_line(frame: &mut Frame, editor: &Editor, area: Rect) {
        let command_text = format!(":{}", editor.command_line());

        let command_line = Line::from(vec![
            Span::styled(
                command_text,
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Black),
            ),
        ]);

        let paragraph = Paragraph::new(command_line)
            .style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, area);
    }

    /// Renders the search line
    fn render_search_line(frame: &mut Frame, editor: &Editor, area: Rect) {
        let search_prefix = if editor.search_forward() { "/" } else { "?" };
        let search_text = format!("{}{}", search_prefix, editor.search_buffer());

        let search_line = Line::from(vec![
            Span::styled(
                search_text,
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Black),
            ),
        ]);

        let paragraph = Paragraph::new(search_line)
            .style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, area);
    }

    /// Calculates the picker overlay area (centered, takes up 60% of screen)
    fn get_picker_area(full_area: Rect) -> Rect {
        let width = (full_area.width * 60) / 100;
        let height = (full_area.height * 60) / 100;
        let x = (full_area.width - width) / 2;
        let y = (full_area.height - height) / 2;

        Rect::new(x, y, width.max(40), height.max(15))
    }

    /// Renders the picker overlay
    fn render_picker(frame: &mut Frame, editor: &Editor, _full_area: Rect) {
        let Some(picker) = editor.picker() else { return };

        let picker_area = Self::get_picker_area(frame.area());

        // Create block with border
        let mode_name = match picker.mode() {
            crate::editor::PickerMode::FindFiles => "Find Files",
            crate::editor::PickerMode::LiveGrep => "Live Grep",
        };

        let block = Block::default()
            .title(mode_name)
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black).fg(Color::White));

        frame.render_widget(block.clone(), picker_area);

        // Split picker area into query line and results
        let inner_area = block.inner(picker_area);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)].as_ref())
            .split(inner_area);

        // Render query line
        let query_text = format!("> {}", picker.query());
        let query_line = Line::from(vec![
            Span::styled(
                query_text,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        let query_paragraph = Paragraph::new(query_line)
            .style(Style::default().bg(Color::Black));
        frame.render_widget(query_paragraph, chunks[0]);

        // Render results
        let results = picker.filtered_results();
        let selected_idx = picker.selected_index();
        let max_results = chunks[1].height as usize;
        let result_width = chunks[1].width as usize;

        // Calculate scroll offset to keep selected item visible
        let scroll_offset = if selected_idx >= max_results {
            selected_idx - max_results + 1
        } else {
            0
        };

        let visible_results: Vec<Line> = results
            .iter()
            .skip(scroll_offset)
            .take(max_results)
            .enumerate()
            .map(|(idx, result)| {
                let actual_idx = idx + scroll_offset;
                let is_selected = actual_idx == selected_idx;

                let text = format!("  {}", result.display);
                let style = if is_selected {
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray).bg(Color::Black)
                };

                // Pad line to fill width with background color
                let padding = result_width.saturating_sub(text.len());
                Line::from(vec![
                    Span::styled(text, style),
                    Span::styled(" ".repeat(padding), style),
                ])
            })
            .collect();

        // Show results or "No matches" message
        let mut all_lines = visible_results;

        if results.is_empty() {
            // Truly no matches
            let text = "  No matches";
            let padding = result_width.saturating_sub(text.len());
            all_lines.push(Line::from(vec![
                Span::styled(
                    text,
                    Style::default().fg(Color::Red).bg(Color::Black),
                ),
                Span::styled(
                    " ".repeat(padding),
                    Style::default().bg(Color::Black),
                ),
            ]));
        } else {
            // Add result count at the end if there's space
            let result_count = format!("  {} matches", results.len());
            if all_lines.len() < max_results {
                let padding = result_width.saturating_sub(result_count.len());
                all_lines.push(Line::from(vec![
                    Span::styled(
                        result_count,
                        Style::default().fg(Color::DarkGray).bg(Color::Black),
                    ),
                    Span::styled(
                        " ".repeat(padding),
                        Style::default().bg(Color::Black),
                    ),
                ]));
            }
        }

        // Fill remaining lines with empty spans that have background color
        let lines_to_fill = max_results.saturating_sub(all_lines.len());
        for _ in 0..lines_to_fill {
            all_lines.push(Line::from(vec![
                Span::styled(
                    " ".repeat(result_width),
                    Style::default().bg(Color::Black),
                ),
            ]));
        }

        let results_paragraph = Paragraph::new(all_lines)
            .style(Style::default().bg(Color::Black));
        frame.render_widget(results_paragraph, chunks[1]);
    }

    /// Renders a single line with all highlighting (syntax, visual selection, search)
    fn render_line_with_highlights(
        theme: &Theme,
        line_text: &str,
        line_idx: usize,
        visual_selection: Option<((usize, usize), (usize, usize))>,
        search_matches: &[(usize, usize)],
        syntax_highlights: &[(std::ops::Range<usize>, crate::syntax::HighlightGroup)],
    ) -> Line<'static> {
        let chars: Vec<char> = line_text.chars().collect();
        let mut spans = Vec::new();

        let mut col_idx = 0;
        while col_idx < chars.len() {
            // Check if this character is in visual selection
            let is_selected = if let Some(((sel_start_line, sel_start_col), (sel_end_line, sel_end_col))) = visual_selection {
                if line_idx == sel_start_line && line_idx == sel_end_line {
                    col_idx >= sel_start_col && col_idx <= sel_end_col
                } else if line_idx == sel_start_line {
                    col_idx >= sel_start_col
                } else if line_idx == sel_end_line {
                    col_idx <= sel_end_col
                } else if line_idx > sel_start_line && line_idx < sel_end_line {
                    true
                } else {
                    false
                }
            } else {
                false
            };

            // Check if this character is in a search match
            let is_search_match = search_matches.iter().any(|(start, end)| {
                col_idx >= *start && col_idx < *end
            });

            // Check if this character is in a syntax highlight
            let syntax_group = syntax_highlights.iter()
                .find(|(range, _)| range.contains(&col_idx))
                .map(|(_, group)| *group);

            // Determine how many characters share the same styling
            let mut end_col = col_idx + 1;
            while end_col < chars.len() {
                let next_selected = if let Some(((sel_start_line, sel_start_col), (sel_end_line, sel_end_col))) = visual_selection {
                    if line_idx == sel_start_line && line_idx == sel_end_line {
                        end_col >= sel_start_col && end_col <= sel_end_col
                    } else if line_idx == sel_start_line {
                        end_col >= sel_start_col
                    } else if line_idx == sel_end_line {
                        end_col <= sel_end_col
                    } else if line_idx > sel_start_line && line_idx < sel_end_line {
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                let next_search_match = search_matches.iter().any(|(start, end)| {
                    end_col >= *start && end_col < *end
                });

                let next_syntax_group = syntax_highlights.iter()
                    .find(|(range, _)| range.contains(&end_col))
                    .map(|(_, group)| *group);

                // If styling changes, break
                if next_selected != is_selected
                    || next_search_match != is_search_match
                    || next_syntax_group != syntax_group {
                    break;
                }

                end_col += 1;
            }

            // Build the span for this range
            let text: String = chars[col_idx..end_col].iter().collect();

            // Apply styling based on priority: visual selection > search match > syntax > normal
            let style = if is_selected {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else if is_search_match {
                Style::default().bg(Color::Yellow).fg(Color::Black)
            } else if let Some(group) = syntax_group {
                let color = theme.get_color(group);
                Style::default().fg(color)
            } else {
                Style::default()
            };

            spans.push(Span::styled(text, style));
            col_idx = end_col;
        }

        Line::from(spans)
    }

    /// Clears the terminal
    pub fn clear(&mut self) -> Result<()> {
        self.terminal.clear()?;
        Ok(())
    }
}
