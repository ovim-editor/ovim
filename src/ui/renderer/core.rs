use crate::editor::Editor;
use crate::syntax::Theme;
use anyhow::Result;
use crossterm::cursor::SetCursorStyle;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::Paragraph,
    Frame, Terminal as RatatuiTerminal,
};
use std::io;

use super::buffer::render_buffer;
use super::helpers::char_col_to_display_col;
use super::widgets::{
    render_command_line, render_completion_menu, render_file_tree, render_hover_window,
    render_picker, render_progress_line, render_search_line, render_status_line, render_tab_bar,
};

/// Handles rendering the editor state to the terminal
pub struct Renderer {
    terminal: RatatuiTerminal<CrosstermBackend<io::Stdout>>,
    /// Reserved for future theme application
    #[allow(dead_code)]
    theme: Theme,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
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
    pub fn render_to_frame(frame: &mut Frame, editor: &mut Editor) {
        // Fill entire frame with blank lines to prevent artifacts from previous renders
        let area = frame.area();
        let blank_line = " ".repeat(area.width as usize);
        let blank_lines: Vec<Line> = (0..area.height)
            .map(|_| Line::from(blank_line.clone()))
            .collect();
        let bg_paragraph = Paragraph::new(blank_lines).style(Style::default().bg(Color::Reset));
        frame.render_widget(bg_paragraph, area);

        // Get color scheme from editor, fall back to Tokyonight if not found
        let scheme = editor
            .get_color_scheme()
            .cloned()
            .unwrap_or_else(crate::syntax::ColorScheme::tokyonight);
        let theme = Theme::from_scheme(scheme);

        // Layout: [tab bar (if multiple tabs)] + [file tree (optional)] + main buffer + status line
        let main_area = frame.area();

        // First split: tab bar (if multiple tabs) + rest
        let (tab_area, remaining_area) = if editor.tab_count() > 1 {
            let vertical_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)].as_ref())
                .split(main_area);
            (Some(vertical_chunks[0]), vertical_chunks[1])
        } else {
            (None, main_area)
        };

        // Render tab bar if we have multiple tabs
        if let Some(tab_area) = tab_area {
            render_tab_bar(frame, editor, tab_area);
        }

        // Second split: file tree (if visible) + rest
        let (file_tree_area, content_area) = if editor.file_tree().is_visible() {
            let horizontal_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(30), Constraint::Min(1)].as_ref())
                .split(remaining_area);
            (Some(horizontal_chunks[0]), horizontal_chunks[1])
        } else {
            (None, remaining_area)
        };

        // Second split: buffer + progress line (optional) + status line
        let has_progress = editor.lsp_progress_message().is_some();
        let chunks = if has_progress {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Min(1),
                        Constraint::Length(1), // progress line
                        Constraint::Length(1), // status line
                    ]
                    .as_ref(),
                )
                .split(content_area)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
                .split(content_area)
        };

        // Render the file tree if visible
        if let Some(tree_area) = file_tree_area {
            render_file_tree(frame, editor, tree_area);
        }

        // Calculate text area, centering if textwidth is set
        let buffer_area = if let Some(textwidth) = editor.options.textwidth {
            let max_width = textwidth as u16;
            if chunks[0].width > max_width {
                let margin = (chunks[0].width - max_width) / 2;
                Rect {
                    x: chunks[0].x + margin,
                    y: chunks[0].y,
                    width: max_width,
                    height: chunks[0].height,
                }
            } else {
                chunks[0]
            }
        } else {
            chunks[0]
        };

        // Render the main text area
        let viewport_start = render_buffer(frame, editor, &theme, buffer_area);

        // Render progress line if present
        if has_progress {
            if let Some(progress_msg) = editor.lsp_progress_message() {
                render_progress_line(frame, &progress_msg, chunks[1]);
            }
        }

        // Render the status line or command line or search line
        let status_chunk = if has_progress { chunks[2] } else { chunks[1] };
        if editor.mode() == crate::mode::Mode::Command {
            render_command_line(frame, editor, status_chunk);
        } else if editor.mode() == crate::mode::Mode::Search {
            render_search_line(frame, editor, status_chunk);
        } else {
            render_status_line(frame, editor, status_chunk);
        }

        // Render picker overlay if in Picker mode
        if editor.mode() == crate::mode::Mode::Picker {
            render_picker(frame, editor, frame.area());
        }

        // Render hover window if in HoverWindow mode
        if editor.mode() == crate::mode::Mode::HoverWindow {
            if let Some(hover_text) = editor.hover_info() {
                render_hover_window(frame, hover_text, editor.hover_scroll(), buffer_area);
            }
        }

        // Render completion menu if visible (in Insert mode)
        if editor.completion_menu().is_visible() {
            render_completion_menu(frame, editor, buffer_area, viewport_start);
        }

        // Set hardware cursor position
        Self::set_cursor_position(frame, editor, buffer_area, viewport_start, status_chunk);
    }

    /// Sets the hardware cursor position based on the current mode
    fn set_cursor_position(
        frame: &mut Frame,
        editor: &mut Editor,
        buffer_area: Rect,
        viewport_start: usize,
        status_chunk: Rect,
    ) {
        let cursor_pos = editor.buffer().cursor();
        let cursor_line = cursor_pos.line();
        let cursor_col = cursor_pos.col();

        if editor.mode() == crate::mode::Mode::Picker {
            // Position cursor in picker query line at the cursor position
            if let Some(picker) = editor.picker() {
                let cursor_pos = picker.query_cursor();
                let picker_area = super::widgets::get_picker_area(frame.area());
                // +1 for border, +2 for " " prefix (icon + space)
                let cursor_x = (picker_area.x + 1 + 2 + cursor_pos as u16)
                    .min(picker_area.x + picker_area.width.saturating_sub(2)); // Keep within bounds
                let cursor_y = picker_area.y + 1; // +1 for border
                frame.set_cursor_position((cursor_x, cursor_y));
            }
        } else if editor.mode() == crate::mode::Mode::Command {
            // Position cursor in command line
            let cmd_cursor_x = (editor.command_line().len() + 1)
                .min(status_chunk.width.saturating_sub(1) as usize);
            frame.set_cursor_position((status_chunk.x + cmd_cursor_x as u16, status_chunk.y));
        } else if editor.mode() == crate::mode::Mode::Search {
            // Position cursor in search line
            let search_cursor_x = (editor.search_buffer().len() + 1)
                .min(status_chunk.width.saturating_sub(1) as usize);
            frame.set_cursor_position((status_chunk.x + search_cursor_x as u16, status_chunk.y));
        } else {
            // Position cursor in text buffer (accounting for gutter, tabs, and wide chars)
            let screen_line = cursor_line.saturating_sub(viewport_start);
            let cursor_y = screen_line.min(buffer_area.height.saturating_sub(1) as usize);

            // Get the line text and convert character column to display column
            let rope = editor.buffer().rope();
            let line_text = if cursor_line < rope.len_lines() {
                rope.line(cursor_line).to_string()
            } else {
                String::new()
            };
            let line_text = line_text.trim_end_matches('\n');

            // Convert character column to display column (accounting for tabs and emojis)
            let tab_width = editor.options.tab_width;
            let display_col = char_col_to_display_col(line_text, cursor_col, tab_width);
            let cursor_x = display_col.min(buffer_area.width.saturating_sub(1) as usize);

            // Calculate gutter width for cursor offset
            let show_numbers = editor.options.number || editor.options.relative_number;
            let max_line_num = editor.buffer().rope().len_lines();
            let line_num_width = if show_numbers {
                max_line_num.to_string().len().max(3)
            } else {
                0
            };
            let sign_width = 2;
            let gutter_width = if show_numbers || sign_width > 0 {
                sign_width + line_num_width + 1
            } else {
                0
            };

            frame.set_cursor_position((
                buffer_area.x + gutter_width as u16 + cursor_x as u16,
                buffer_area.y + cursor_y as u16,
            ));
        }
    }

    /// Renders the editor state to the terminal
    pub fn render(&mut self, editor: &mut Editor) -> Result<()> {
        // Set cursor style based on mode
        let cursor_style = match editor.mode() {
            crate::mode::Mode::Insert => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Picker => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Command => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::Search => SetCursorStyle::BlinkingBar,
            crate::mode::Mode::HoverWindow => SetCursorStyle::SteadyBlock,
            _ => SetCursorStyle::SteadyBlock,
        };
        crossterm::execute!(io::stdout(), cursor_style)?;

        // Force autoresize to clear internal buffer state
        self.terminal.autoresize()?;

        self.terminal.draw(|frame| {
            Self::render_to_frame(frame, editor);
        })?;

        // Flush to ensure all changes are written
        use std::io::Write;
        io::stdout().flush()?;

        Ok(())
    }

    /// Clears the terminal
    pub fn clear(&mut self) -> Result<()> {
        self.terminal.clear()?;
        Ok(())
    }
}
