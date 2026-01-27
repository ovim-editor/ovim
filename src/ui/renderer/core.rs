use crate::editor::{Editor, SplitDirection, WindowNode};
use crate::syntax::Theme;
use anyhow::Result;
use crossterm::cursor::SetCursorStyle;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal as RatatuiTerminal,
};
use std::io;

use super::buffer::render_buffer;
use super::dashboard::render_dashboard;
use super::helpers::char_col_to_display_col;
use super::widgets::{
    render_command_line, render_completion_menu, render_file_tree, render_hover_window,
    render_picker, render_progress_line, render_search_line, render_status_line, render_tab_bar,
};

/// Recursively renders windows in a split layout
/// Returns (viewport_start, buffer_area) for the focused window (for cursor positioning)
fn render_window_tree(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    node: &WindowNode,
    area: Rect,
    focused_index: usize,
    current_index: &mut usize,
) -> Option<(usize, Rect)> {
    match node {
        WindowNode::Leaf(_window) => {
            let is_focused = *current_index == focused_index;
            *current_index += 1;

            // Render this window's buffer
            let viewport_start = render_buffer(frame, editor, theme, area);

            if is_focused {
                Some((viewport_start, area))
            } else {
                None
            }
        }
        WindowNode::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            // Calculate split areas (including 1-pixel separator)
            let (first_area, sep_area, second_area) = match direction {
                SplitDirection::Horizontal => {
                    // Windows stacked vertically (horizontal separator line)
                    let first_height = (area.height as f32 * *ratio) as u16;
                    let sep_height = 1u16;
                    let second_height = area.height.saturating_sub(first_height + sep_height);

                    let first_rect = Rect {
                        x: area.x,
                        y: area.y,
                        width: area.width,
                        height: first_height,
                    };
                    let sep_rect = Rect {
                        x: area.x,
                        y: area.y + first_height,
                        width: area.width,
                        height: sep_height,
                    };
                    let second_rect = Rect {
                        x: area.x,
                        y: area.y + first_height + sep_height,
                        width: area.width,
                        height: second_height,
                    };
                    (first_rect, sep_rect, second_rect)
                }
                SplitDirection::Vertical => {
                    // Windows side by side (vertical separator line)
                    let first_width = (area.width as f32 * *ratio) as u16;
                    let sep_width = 1u16;
                    let second_width = area.width.saturating_sub(first_width + sep_width);

                    let first_rect = Rect {
                        x: area.x,
                        y: area.y,
                        width: first_width,
                        height: area.height,
                    };
                    let sep_rect = Rect {
                        x: area.x + first_width,
                        y: area.y,
                        width: sep_width,
                        height: area.height,
                    };
                    let second_rect = Rect {
                        x: area.x + first_width + sep_width,
                        y: area.y,
                        width: second_width,
                        height: area.height,
                    };
                    (first_rect, sep_rect, second_rect)
                }
            };

            // Render separator
            render_separator(frame, sep_area, *direction);

            // Recursively render children
            let first_result =
                render_window_tree(frame, editor, theme, first, first_area, focused_index, current_index);
            let second_result =
                render_window_tree(frame, editor, theme, second, second_area, focused_index, current_index);

            first_result.or(second_result)
        }
    }
}

/// Renders a separator line between split windows
fn render_separator(frame: &mut Frame, area: Rect, direction: SplitDirection) {
    let sep_char = match direction {
        SplitDirection::Horizontal => '─', // Horizontal line for horizontal split
        SplitDirection::Vertical => '│',   // Vertical line for vertical split
    };

    let sep_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::DIM);

    match direction {
        SplitDirection::Horizontal => {
            // Draw horizontal line
            let line_text = sep_char.to_string().repeat(area.width as usize);
            let line = Line::from(Span::styled(line_text, sep_style));
            let paragraph = Paragraph::new(vec![line]);
            frame.render_widget(paragraph, area);
        }
        SplitDirection::Vertical => {
            // Draw vertical line (multiple rows)
            let lines: Vec<Line> = (0..area.height)
                .map(|_| Line::from(Span::styled(sep_char.to_string(), sep_style)))
                .collect();
            let paragraph = Paragraph::new(lines);
            frame.render_widget(paragraph, area);
        }
    }
}

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

        // Initialize window manager with terminal dimensions if not already initialized
        // This is needed for viewport commands (zz, zt, zb, etc.) to work
        editor.init_window_manager(area.width, area.height);

        let blank_line = " ".repeat(area.width as usize);
        let blank_lines: Vec<Line> = (0..area.height)
            .map(|_| Line::from(blank_line.clone()))
            .collect();
        let bg_paragraph = Paragraph::new(blank_lines).style(Style::default().bg(Color::Reset));
        frame.render_widget(bg_paragraph, area);

        // Render dashboard if in Dashboard mode
        if editor.should_show_dashboard() {
            render_dashboard(frame, editor, area);
            return;
        }

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

        // Check if we have split windows
        let has_splits = editor
            .window_manager()
            .map(|wm| wm.root().count_windows() > 1)
            .unwrap_or(false);

        // Ensure wrap map is up to date before rendering
        if editor.options.wrap {
            // Estimate text width: frame width minus gutter
            let show_numbers = editor.options.number || editor.options.relative_number;
            let line_count = editor.buffer().line_count();
            let line_num_width = if show_numbers {
                line_count.to_string().len().max(3)
            } else {
                0
            };
            let gutter_width = if show_numbers || true {
                // sign_width(2) + line_num_width + spacing(1)
                2 + line_num_width + 1
            } else {
                0
            };
            let text_width = (chunks[0].width as usize).saturating_sub(gutter_width);
            editor.ensure_wrap_map(text_width);
        }

        // Render buffer area(s) - either single buffer or split windows
        let (viewport_start, buffer_area) = if has_splits {
            // Render split windows recursively
            if let Some(wm) = editor.window_manager() {
                let focused_index = wm.focused_window_index();
                let mut current_index = 0;
                if let Some((vs, ba)) = render_window_tree(
                    frame,
                    editor,
                    &theme,
                    wm.root(),
                    chunks[0],
                    focused_index,
                    &mut current_index,
                ) {
                    (vs, ba)
                } else {
                    // Fallback: render single buffer
                    let viewport_start = render_buffer(frame, editor, &theme, chunks[0]);
                    (viewport_start, chunks[0])
                }
            } else {
                // No window manager - render single buffer
                let viewport_start = render_buffer(frame, editor, &theme, chunks[0]);
                (viewport_start, chunks[0])
            }
        } else {
            // Single window - apply textwidth centering if set
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
            (viewport_start, buffer_area)
        };

        // Update editor's viewport height for accurate scroll calculations
        editor.set_viewport_height(buffer_area.height as usize);

        // Update window manager dimensions to match actual buffer area
        // This ensures viewport commands (zt, zb, zz) use the correct height
        if let Some(wm) = editor.window_manager_mut() {
            wm.update_dimensions(buffer_area.width, buffer_area.height);
        }

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

        // Render hover window if in a hover mode
        if editor.mode().is_hover() {
            if let Some(hover_text) = editor.hover_info() {
                let is_preview = editor.mode() == crate::mode::Mode::HoverPreview;
                let hover_pos = editor.hover_position();
                let content_type = editor.hover_content_type();
                render_hover_window(
                    frame,
                    editor,
                    hover_text,
                    editor.hover_scroll(),
                    buffer_area,
                    viewport_start,
                    hover_pos,
                    is_preview,
                    &theme,
                    content_type,
                );
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
            let search_cursor_x = (editor.search.search_buffer.len() + 1)
                .min(status_chunk.width.saturating_sub(1) as usize);
            frame.set_cursor_position((status_chunk.x + search_cursor_x as u16, status_chunk.y));
        } else {
            // Position cursor in text buffer (accounting for gutter, tabs, and wide chars)
            let rope = editor.buffer().rope();
            let line_count = editor.buffer().line_count();
            let line_text = if cursor_line < line_count {
                rope.line(cursor_line).to_string()
            } else {
                String::new()
            };
            let line_text = line_text.trim_end_matches('\n');

            // Convert character column to display column (accounting for tabs and emojis)
            let tab_width = editor.options.tab_width;
            let display_col = char_col_to_display_col(line_text, cursor_col, tab_width);

            // Calculate gutter width for cursor offset
            let show_numbers = editor.options.number || editor.options.relative_number;
            let max_line_num = line_count;
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

            let text_width = buffer_area.width.saturating_sub(gutter_width as u16) as usize;

            // Calculate cursor screen position, accounting for soft wrap
            let (cursor_y, cursor_x) = if editor.options.wrap && text_width > 0 {
                if let Some(wrap_map) = editor.wrap_map() {
                    // Visual row of this cursor in the entire document
                    let (abs_visual_row, _) = wrap_map.cursor_to_visual(cursor_line, display_col);
                    // Visual row of the viewport start
                    let viewport_visual_row = wrap_map.logical_to_visual(viewport_start);
                    let screen_row = abs_visual_row.saturating_sub(viewport_visual_row);
                    let screen_col = display_col % text_width;
                    (
                        screen_row.min(buffer_area.height.saturating_sub(1) as usize),
                        screen_col.min(text_width.saturating_sub(1)),
                    )
                } else {
                    let screen_line = cursor_line.saturating_sub(viewport_start);
                    (
                        screen_line.min(buffer_area.height.saturating_sub(1) as usize),
                        display_col.min(buffer_area.width.saturating_sub(1) as usize),
                    )
                }
            } else {
                let screen_line = cursor_line.saturating_sub(viewport_start);
                (
                    screen_line.min(buffer_area.height.saturating_sub(1) as usize),
                    display_col.min(buffer_area.width.saturating_sub(1) as usize),
                )
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
            crate::mode::Mode::HoverPreview | crate::mode::Mode::HoverNavigate => {
                SetCursorStyle::SteadyBlock
            }
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
