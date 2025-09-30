use crate::buffer::Buffer;
use crate::editor::Editor;
use anyhow::Result;
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
}

impl Renderer {
    /// Creates a new renderer
    pub fn new() -> Self {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = RatatuiTerminal::new(backend).expect("Failed to create terminal");
        Self { terminal }
    }

    /// Renders the editor state to the terminal
    pub fn render(&mut self, editor: &Editor) -> Result<()> {
        let cursor_pos = editor.buffer().cursor();
        let cursor_line = cursor_pos.line();
        let cursor_col = cursor_pos.col();

        self.terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
                .split(frame.area());

            // Render the main text area
            let viewport_start = Self::render_buffer(frame, editor, chunks[0]);

            // Render the status line or command line
            if editor.mode() == crate::mode::Mode::Command {
                Self::render_command_line(frame, editor, chunks[1]);
            } else {
                Self::render_status_line(frame, editor, chunks[1]);
            }

            // Set hardware cursor position
            if editor.mode() == crate::mode::Mode::Command {
                // Position cursor in command line
                let cmd_cursor_x = (editor.command_line().len() + 1).min(chunks[1].width.saturating_sub(1) as usize);
                frame.set_cursor_position((
                    chunks[1].x + cmd_cursor_x as u16,
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
        })?;
        Ok(())
    }

    /// Renders the buffer content and returns the viewport start line
    fn render_buffer(frame: &mut Frame, editor: &Editor, area: Rect) -> usize {
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

        // Build the visible text
        let mut lines = Vec::new();
        for line_idx in start_line..end_line {
            if line_idx < rope.len_lines() {
                let line_text = rope.line(line_idx).to_string();
                // Remove trailing newline if present
                let line_text = line_text.trim_end_matches('\n');

                // Check if this line is part of visual selection
                if let Some(((sel_start_line, sel_start_col), (sel_end_line, sel_end_col))) = visual_selection {
                    if line_idx >= sel_start_line && line_idx <= sel_end_line {
                        // Line is in selection - highlight it
                        let chars: Vec<char> = line_text.chars().collect();
                        let mut spans = Vec::new();

                        for (col_idx, ch) in chars.iter().enumerate() {
                            let is_selected = if line_idx == sel_start_line && line_idx == sel_end_line {
                                // Selection on single line
                                col_idx >= sel_start_col && col_idx <= sel_end_col
                            } else if line_idx == sel_start_line {
                                // First line of selection
                                col_idx >= sel_start_col
                            } else if line_idx == sel_end_line {
                                // Last line of selection
                                col_idx <= sel_end_col
                            } else {
                                // Middle line - fully selected
                                true
                            };

                            if is_selected {
                                spans.push(Span::styled(
                                    ch.to_string(),
                                    Style::default().bg(Color::Blue).fg(Color::White),
                                ));
                            } else {
                                spans.push(Span::raw(ch.to_string()));
                            }
                        }

                        lines.push(Line::from(spans));
                    } else {
                        lines.push(Line::from(line_text.to_string()));
                    }
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

    /// Clears the terminal
    pub fn clear(&mut self) -> Result<()> {
        self.terminal.clear()?;
        Ok(())
    }
}
