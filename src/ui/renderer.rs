use crate::editor::Editor;
use crate::syntax::{HighlightGroup, Theme};
use crate::LineStatus;
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
use std::ops::Range;

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
            .unwrap_or_else(|| crate::syntax::ColorScheme::tokyonight());
        let theme = Theme::from_scheme(scheme);
        let cursor_pos = editor.buffer().cursor();
        let cursor_line = cursor_pos.line();
        let cursor_col = cursor_pos.col();

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
            Self::render_tab_bar(frame, editor, tab_area);
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
            Self::render_file_tree(frame, editor, tree_area);
        }

        // Render the main text area
        let viewport_start = Self::render_buffer(frame, editor, &theme, chunks[0]);

        // Render progress line if present
        if has_progress {
            if let Some(progress_msg) = editor.lsp_progress_message() {
                Self::render_progress_line(frame, &progress_msg, chunks[1]);
            }
        }

        // Render the status line or command line or search line
        let status_chunk = if has_progress { chunks[2] } else { chunks[1] };
        if editor.mode() == crate::mode::Mode::Command {
            Self::render_command_line(frame, editor, status_chunk);
        } else if editor.mode() == crate::mode::Mode::Search {
            Self::render_search_line(frame, editor, status_chunk);
        } else {
            Self::render_status_line(frame, editor, status_chunk);
        }

        // Render picker overlay if in Picker mode
        if editor.mode() == crate::mode::Mode::Picker {
            Self::render_picker(frame, editor, frame.area());
        }

        // Render hover window if in HoverWindow mode
        if editor.mode() == crate::mode::Mode::HoverWindow {
            if let Some(hover_text) = editor.hover_info() {
                Self::render_hover_window(frame, hover_text, editor.hover_scroll(), chunks[0]);
            }
        }

        // Render completion menu if visible (in Insert mode)
        if editor.completion_menu().is_visible() {
            Self::render_completion_menu(frame, editor, chunks[0], viewport_start);
        }

        // Set hardware cursor position
        if editor.mode() == crate::mode::Mode::Picker {
            // Position cursor in picker query line at the cursor position
            if let Some(picker) = editor.picker() {
                let cursor_pos = picker.query_cursor();
                let picker_area = Self::get_picker_area(frame.area());
                // +1 for border, +2 for " " prefix (icon + space)
                let cursor_x = (picker_area.x + 1 + 2 + cursor_pos as u16)
                    .min(picker_area.x + picker_area.width.saturating_sub(2)); // Keep within bounds
                let cursor_y = picker_area.y + 1; // +1 for border
                frame.set_cursor_position((cursor_x, cursor_y));
            }
        } else if editor.mode() == crate::mode::Mode::Command {
            // Position cursor in command line (use status_chunk, not chunks[1])
            let cmd_cursor_x = (editor.command_line().len() + 1)
                .min(status_chunk.width.saturating_sub(1) as usize);
            frame.set_cursor_position((status_chunk.x + cmd_cursor_x as u16, status_chunk.y));
        } else if editor.mode() == crate::mode::Mode::Search {
            // Position cursor in search line (use status_chunk, not chunks[1])
            let search_cursor_x = (editor.search_buffer().len() + 1)
                .min(status_chunk.width.saturating_sub(1) as usize);
            frame.set_cursor_position((status_chunk.x + search_cursor_x as u16, status_chunk.y));
        } else {
            // Position cursor in text buffer (accounting for gutter, tabs, and wide chars)
            let screen_line = cursor_line.saturating_sub(viewport_start);
            let cursor_y = screen_line.min(chunks[0].height.saturating_sub(1) as usize);

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
            let display_col = Self::char_col_to_display_col(line_text, cursor_col, tab_width);
            let cursor_x = display_col.min(chunks[0].width.saturating_sub(1) as usize);

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
                chunks[0].x + gutter_width as u16 + cursor_x as u16,
                chunks[0].y + cursor_y as u16,
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

    /// Expands tabs to spaces based on display width (accounts for wide chars like emojis)
    /// Returns both the expanded string and a mapping from original byte offsets to expanded byte offsets
    fn expand_tabs_with_mapping(text: &str, tab_width: usize) -> (String, Vec<(usize, usize)>) {
        use unicode_width::UnicodeWidthChar;

        let mut result = String::with_capacity(text.len() * 2);
        let mut display_col = 0;
        let mut byte_mapping = Vec::new(); // original_byte_idx -> expanded_byte_idx

        let mut expanded_byte_pos = 0;

        for (orig_byte_idx, ch) in text.char_indices() {
            // Record mapping from original position to expanded position
            byte_mapping.push((orig_byte_idx, expanded_byte_pos));

            if ch == '\t' {
                // Calculate spaces needed to reach next tab stop
                let spaces_to_add = tab_width - (display_col % tab_width);
                result.push_str(&" ".repeat(spaces_to_add));
                expanded_byte_pos += spaces_to_add;
                display_col += spaces_to_add;
            } else {
                result.push(ch);
                expanded_byte_pos += ch.len_utf8();
                // Use display width (emojis = 2, most chars = 1, zero-width = 0)
                display_col += ch.width().unwrap_or(1);
            }
        }

        // Add final mapping for end position
        byte_mapping.push((text.len(), expanded_byte_pos));

        (result, byte_mapping)
    }

    /// Expands tabs to spaces (simple version without mapping)
    fn expand_tabs(text: &str, tab_width: usize) -> String {
        Self::expand_tabs_with_mapping(text, tab_width).0
    }

    /// Converts a character column index to a display column, accounting for tabs and wide characters
    fn char_col_to_display_col(text: &str, char_col: usize, tab_width: usize) -> usize {
        use unicode_width::UnicodeWidthChar;

        let mut display_col = 0;
        let mut current_char_idx = 0;

        for ch in text.chars() {
            if current_char_idx >= char_col {
                break;
            }

            if ch == '\t' {
                // Move to next tab stop
                let spaces_to_add = tab_width - (display_col % tab_width);
                display_col += spaces_to_add;
            } else {
                // Use display width (emojis = 2, most chars = 1, zero-width = 0)
                display_col += ch.width().unwrap_or(1);
            }

            current_char_idx += 1;
        }

        display_col
    }

    /// Truncates text to fit within a display width, accounting for wide characters
    fn truncate_to_width(text: &str, max_width: usize) -> String {
        use unicode_width::UnicodeWidthChar;

        let mut result = String::new();
        let mut display_width = 0;

        for ch in text.chars() {
            let ch_width = ch.width().unwrap_or(1);
            if display_width + ch_width > max_width {
                break;
            }
            result.push(ch);
            display_width += ch_width;
        }

        result
    }

    /// Adjusts syntax highlight ranges based on tab expansion mapping
    fn remap_highlights(
        highlights: &[(Range<usize>, HighlightGroup)],
        byte_mapping: &[(usize, usize)],
    ) -> Vec<(Range<usize>, HighlightGroup)> {
        highlights
            .iter()
            .map(|(range, group)| {
                // Find mapped positions for start and end
                let new_start = byte_mapping
                    .iter()
                    .find(|(orig, _)| *orig >= range.start)
                    .map(|(_, expanded)| *expanded)
                    .unwrap_or(0);

                let new_end = byte_mapping
                    .iter()
                    .find(|(orig, _)| *orig >= range.end)
                    .map(|(_, expanded)| *expanded)
                    .unwrap_or(new_start);

                (new_start..new_end, *group)
            })
            .collect()
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

        // Calculate gutter width
        let show_numbers = editor.options.number || editor.options.relative_number;
        let max_line_num = rope.len_lines();
        let line_num_width = if show_numbers {
            max_line_num.to_string().len().max(3) // At least 3 chars for line numbers
        } else {
            0
        };
        let sign_width = 2; // Space for signs (git, diagnostics, etc.)
        let gutter_width = if show_numbers || sign_width > 0 {
            (sign_width + line_num_width + 1) as u16 // +1 for spacing
        } else {
            0
        };

        // Split area into gutter and text
        let (gutter_area, text_area) = if gutter_width > 0 {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(gutter_width), Constraint::Min(1)])
                .split(area);
            (Some(chunks[0]), chunks[1])
        } else {
            (None, area)
        };

        // Get visual selection if in visual mode
        let visual_selection = if editor.mode().is_visual() {
            editor.visual_selection()
        } else {
            None
        };

        // Get current search if active
        let current_search = editor.current_search();

        // Build the gutter lines
        if let Some(gutter_area) = gutter_area {
            let mut gutter_lines = Vec::new();
            for line_idx in start_line..end_line {
                if line_idx < rope.len_lines() {
                    let line_num_text = if editor.options.relative_number {
                        // Relative line numbers
                        let rel = if line_idx == cursor.line() {
                            line_idx + 1 // Show absolute for current line
                        } else {
                            line_idx.abs_diff(cursor.line())
                        };
                        format!("{:>width$} ", rel, width = line_num_width)
                    } else if editor.options.number {
                        // Absolute line numbers
                        format!("{:>width$} ", line_idx + 1, width = line_num_width)
                    } else {
                        "  ".to_string()
                    };

                    // Add sign column for git status indicators
                    let (sign_text, sign_color) =
                        match buffer.git_status().get_line_status(line_idx) {
                            Some(LineStatus::Added) => ("+ ", Color::Green),
                            Some(LineStatus::Modified) => ("~ ", Color::Yellow),
                            Some(LineStatus::Removed) => ("- ", Color::Red),
                            None => ("  ", Color::DarkGray),
                        };

                    // Highlight current line number
                    let line_num_style = if line_idx == cursor.line() {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };

                    // Build gutter with separate styles for sign and line number
                    let sign_span = Span::styled(
                        sign_text,
                        Style::default().fg(sign_color).add_modifier(Modifier::BOLD),
                    );
                    let line_num_span = Span::styled(line_num_text, line_num_style);

                    gutter_lines.push(Line::from(vec![sign_span, line_num_span]));
                }
            }

            let gutter_paragraph = Paragraph::new(gutter_lines);
            frame.render_widget(gutter_paragraph, gutter_area);
        }

        // Build the visible text with syntax highlighting
        let mut lines = Vec::new();
        let blank_line = " ".repeat(text_area.width as usize);
        let tab_width = editor.options.tab_width;

        for line_idx in start_line..end_line {
            if line_idx < rope.len_lines() {
                let line_text = rope.line(line_idx).to_string();
                // Remove trailing newline if present
                let line_text = line_text.trim_end_matches('\n');

                // Expand tabs to spaces for proper rendering and get byte mapping
                let (line_text, byte_mapping) =
                    Self::expand_tabs_with_mapping(line_text, tab_width);

                // Get syntax highlights for this line and remap them for expanded text
                let original_highlights = buffer.highlights_for_line(line_idx);
                let syntax_highlights = Self::remap_highlights(&original_highlights, &byte_mapping);

                // Check if we need special highlighting (visual selection or search)
                let has_visual_selection = visual_selection
                    .map(|((start_line, _), (end_line, _))| {
                        line_idx >= start_line && line_idx <= end_line
                    })
                    .unwrap_or(false);

                let search_matches = if let Some(search) = current_search {
                    search.find_all_in_line(&line_text)
                } else {
                    Vec::new()
                };

                // Always use character-by-character rendering if we have any highlighting
                let needs_detailed_rendering = has_visual_selection
                    || !search_matches.is_empty()
                    || !syntax_highlights.is_empty();

                if needs_detailed_rendering {
                    let mut line = Self::render_line_with_highlights(
                        theme,
                        &line_text,
                        line_idx,
                        visual_selection,
                        editor.mode(),
                        &search_matches,
                        &syntax_highlights,
                    );
                    // Pad line to clear previous content
                    let line_len: usize =
                        line.spans.iter().map(|s| s.content.chars().count()).sum();
                    if line_len < text_area.width as usize {
                        line.spans
                            .push(Span::raw(" ".repeat(text_area.width as usize - line_len)));
                    }
                    lines.push(line);
                } else {
                    // Pad simple lines too
                    let line_len = line_text.chars().count();
                    let line_text = if line_len < text_area.width as usize {
                        format!(
                            "{}{}",
                            line_text,
                            " ".repeat(text_area.width as usize - line_len)
                        )
                    } else {
                        line_text.to_string()
                    };
                    lines.push(Line::from(line_text));
                }
            } else {
                // Line beyond end of file - clear it
                lines.push(Line::from(blank_line.clone()));
            }
        }

        let paragraph = Paragraph::new(lines)
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().bg(Color::Reset));
        frame.render_widget(paragraph, text_area);

        start_line
    }

    /// Renders the LSP progress line (just above status line)
    fn render_progress_line(frame: &mut Frame, progress_msg: &str, area: Rect) {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::Paragraph;

        // Right-align the progress message
        let padding_len = area.width.saturating_sub(progress_msg.len() as u16 + 2);
        let progress_line = Line::from(vec![
            Span::raw(" ".repeat(padding_len as usize)),
            Span::styled(
                format!(" {} ", progress_msg),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]);

        let paragraph = Paragraph::new(progress_line).style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, area);
    }

    /// Renders the tab bar
    fn render_tab_bar(frame: &mut Frame, editor: &Editor, area: Rect) {
        let tabs = editor.tab_page_manager().tabs();
        let current_index = editor.current_tab_index();

        let mut spans = Vec::new();

        for (i, tab) in tabs.iter().enumerate() {
            let is_current = i == current_index;

            // Tab format: " {number} {title} "
            let tab_text = format!(" {} {} ", i + 1, tab.title());

            let style = if is_current {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
            };

            spans.push(Span::styled(tab_text, style));

            // Add separator between tabs
            if i < tabs.len() - 1 {
                spans.push(Span::styled(" ", Style::default().bg(Color::Black)));
            }
        }

        // Fill rest of line with background color
        let content_width: usize = spans.iter().map(|s| s.content.len()).sum();
        let remaining = (area.width as usize).saturating_sub(content_width);
        if remaining > 0 {
            spans.push(Span::styled(
                " ".repeat(remaining),
                Style::default().bg(Color::Black),
            ));
        }

        let tab_line = Line::from(spans);
        let paragraph = Paragraph::new(tab_line).style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, area);
    }

    /// Renders the status line
    fn render_status_line(frame: &mut Frame, editor: &Editor, area: Rect) {
        let mode = editor.mode();
        let buffer = editor.buffer();
        let cursor = buffer.cursor();

        // Build status line content
        let mode_indicator = format!(" {} ", mode.display_name());
        let recording_indicator = if editor.is_recording_macro() {
            if let Some(reg) = editor.recording_register() {
                format!(" recording @{} ", reg)
            } else {
                " recording ".to_string()
            }
        } else {
            String::new()
        };
        let position = format!(" {}:{} ", cursor.line() + 1, cursor.col() + 1);
        let modified = if buffer.is_modified() { " [+] " } else { " " };
        let file = buffer.file_path().unwrap_or("[No Name]");

        // Get diagnostic counts
        let (errors, warnings, _info, _hints) = editor.cached_diagnostic_count();
        let diagnostics = if errors > 0 || warnings > 0 {
            format!(" E:{} W:{} ", errors, warnings)
        } else {
            String::new()
        };

        // Get LSP status
        let lsp_status = if !editor.lsp_status().is_empty() {
            format!(" {} ", editor.lsp_status())
        } else if !editor.active_lsp_servers().is_empty() {
            " LSP ".to_string()
        } else {
            String::new()
        };

        // Calculate padding accounting for all elements including recording indicator
        let recording_len = if !recording_indicator.is_empty() {
            recording_indicator.len() + 1 // +1 for space after mode
        } else {
            1 // just the space after mode
        };

        let padding_len = (area.width as usize)
            .saturating_sub(mode_indicator.len())
            .saturating_sub(recording_len)
            .saturating_sub(file.len())
            .saturating_sub(modified.len())
            .saturating_sub(diagnostics.len())
            .saturating_sub(lsp_status.len())
            .saturating_sub(position.len());

        let mut spans = vec![Span::styled(
            &mode_indicator,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )];

        // Add recording indicator if recording
        if !recording_indicator.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                &recording_indicator,
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw(" "));
        }

        spans.push(Span::raw(file));
        spans.push(Span::raw(modified));
        spans.push(Span::raw(" ".repeat(padding_len)));

        // Add diagnostics indicator if present
        if !diagnostics.is_empty() {
            spans.push(Span::styled(
                &diagnostics,
                Style::default().fg(Color::Black).bg(if errors > 0 {
                    Color::Red
                } else {
                    Color::Yellow
                }),
            ));
        }

        // Add LSP status if present
        if !lsp_status.is_empty() {
            let lsp_color = if editor.lsp_status().contains("Failed")
                || editor.lsp_status().contains("Error")
            {
                Color::Red
            } else if editor.lsp_status().contains("ready") {
                Color::Green
            } else {
                Color::Blue
            };
            spans.push(Span::styled(
                &lsp_status,
                Style::default().fg(Color::Black).bg(lsp_color),
            ));
        }

        spans.push(Span::styled(
            position,
            Style::default().fg(Color::Black).bg(Color::Gray),
        ));

        let status_line = Line::from(spans);

        let paragraph = Paragraph::new(status_line).style(Style::default().bg(Color::DarkGray));
        frame.render_widget(paragraph, area);
    }

    /// Renders hover information as a scrollable floating window
    fn render_hover_window(
        frame: &mut Frame,
        hover_text: &str,
        scroll_offset: usize,
        buffer_area: Rect,
    ) {
        // Split text into lines
        let all_lines: Vec<&str> = hover_text.lines().collect();
        let total_lines = all_lines.len();

        // Calculate window dimensions (centered, large window)
        let window_width = (buffer_area.width * 80 / 100).min(120); // 80% of screen, max 120 cols
        let window_height = (buffer_area.height * 70 / 100).min(30); // 70% of screen, max 30 lines

        // Center the window
        let window_x = buffer_area.x + (buffer_area.width.saturating_sub(window_width)) / 2;
        let window_y = buffer_area.y + (buffer_area.height.saturating_sub(window_height)) / 2;

        let window_area = Rect {
            x: window_x,
            y: window_y,
            width: window_width,
            height: window_height,
        };

        // Calculate visible content height (minus borders and title)
        let content_height = window_height.saturating_sub(2) as usize;

        // Clamp scroll offset to valid range
        let max_scroll = total_lines.saturating_sub(content_height);
        let clamped_scroll = scroll_offset.min(max_scroll);

        // Get visible lines
        let visible_lines: Vec<String> = all_lines
            .iter()
            .skip(clamped_scroll)
            .take(content_height)
            .map(|line| format!(" {} ", line)) // Add padding
            .collect();

        let text = visible_lines.join("\n");

        // Create title with scroll indicator
        let title = if total_lines > content_height {
            format!(
                " Hover Info ({}/{} lines, q to close, j/k to scroll) ",
                clamped_scroll + 1,
                total_lines
            )
        } else {
            " Hover Info (q to close) ".to_string()
        };

        let paragraph = Paragraph::new(text)
            .style(
                Style::default()
                    .bg(Color::Rgb(30, 30, 40))
                    .fg(Color::Rgb(230, 230, 230)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(255, 200, 100)))
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Rgb(255, 200, 100))
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .wrap(ratatui::widgets::Wrap { trim: false });

        // Clear background and render window
        frame.render_widget(ratatui::widgets::Clear, window_area);
        frame.render_widget(paragraph, window_area);
    }

    /// Renders the completion menu popup
    fn render_completion_menu(
        frame: &mut Frame,
        editor: &Editor,
        buffer_area: Rect,
        viewport_start: usize,
    ) {
        let completion_menu = editor.completion_menu();
        if !completion_menu.is_visible() {
            return;
        }

        let items = completion_menu.items();
        if items.is_empty() {
            return;
        }

        // Get cursor position on screen
        let cursor = editor.buffer().cursor();
        let cursor_line = cursor.line();
        let cursor_col = cursor.col();
        let screen_line = cursor_line.saturating_sub(viewport_start);

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
        let display_col = Self::char_col_to_display_col(line_text, cursor_col, tab_width);

        // Calculate gutter width
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

        // Position menu below cursor
        let menu_x = buffer_area.x + gutter_width as u16 + display_col as u16;
        let menu_y = buffer_area.y + screen_line as u16 + 1; // Below current line

        // Determine menu dimensions
        let max_items_to_show = 10;
        let num_items = items.len().min(max_items_to_show);
        let menu_height = num_items as u16 + 2; // +2 for borders

        // Calculate width based on longest label
        let max_label_len = items
            .iter()
            .take(max_items_to_show)
            .map(|item| item.label.len())
            .max()
            .unwrap_or(20);
        let menu_width = (max_label_len + 4).min(60) as u16; // +4 for padding and borders

        // Adjust position if menu would go off screen
        let menu_x = menu_x.min(buffer_area.width.saturating_sub(menu_width));
        let menu_y = if menu_y + menu_height > buffer_area.y + buffer_area.height {
            // Show above cursor if not enough space below
            (buffer_area.y + screen_line as u16)
                .saturating_sub(menu_height)
                .max(buffer_area.y)
        } else {
            menu_y
        };

        let menu_area = Rect::new(
            menu_x,
            menu_y,
            menu_width,
            menu_height.min(buffer_area.height),
        );

        // Build menu lines
        let selected_index = completion_menu.selected_index();
        let mut lines = Vec::new();

        for (idx, item) in items.iter().take(max_items_to_show).enumerate() {
            let is_selected = idx == selected_index;
            let style = if is_selected {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(Color::Rgb(40, 44, 52)).fg(Color::White)
            };

            // Format: "label"  or "  label" with selection indicator
            let prefix = if is_selected { "> " } else { "  " };
            let text = format!("{}{}", prefix, item.label);

            lines.push(Line::from(Span::styled(text, style)));
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::Rgb(40, 44, 52)));

        let paragraph = Paragraph::new(lines).block(block);

        // Clear background and render menu
        frame.render_widget(ratatui::widgets::Clear, menu_area);
        frame.render_widget(paragraph, menu_area);
    }

    /// Renders the file tree explorer
    fn render_file_tree(frame: &mut Frame, editor: &Editor, area: Rect) {
        use ratatui::widgets::{Block, Borders, List, ListItem};

        if !editor.file_tree().is_visible() {
            return;
        }

        let tree = editor.file_tree();
        let flattened = tree.flattened();
        let selected_index = tree.selected_index();

        // Create list items from flattened tree
        let items: Vec<ListItem> = flattened
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                let indent = "  ".repeat(node.depth());
                let icon = if node.is_dir() {
                    if node.is_expanded() {
                        "▼ "
                    } else {
                        "▶ "
                    }
                } else {
                    "  "
                };

                let name = node.name();
                let display = format!("{}{}{}", indent, icon, name);

                let style = if idx == selected_index {
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else if node.is_dir() {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };

                ListItem::new(display).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::RIGHT)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(" Files ")
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .style(Style::default().bg(Color::Rgb(30, 34, 42)));

        frame.render_widget(list, area);
    }

    /// Renders the command line
    fn render_command_line(frame: &mut Frame, editor: &Editor, area: Rect) {
        let command_text = format!(":{}", editor.command_line());

        let command_line = Line::from(vec![Span::styled(
            command_text,
            Style::default().fg(Color::White).bg(Color::Black),
        )]);

        let paragraph = Paragraph::new(command_line).style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, area);
    }

    /// Renders the search line
    fn render_search_line(frame: &mut Frame, editor: &Editor, area: Rect) {
        let search_prefix = if editor.search_forward() { "/" } else { "?" };
        let search_text = format!("{}{}", search_prefix, editor.search_buffer());

        let search_line = Line::from(vec![Span::styled(
            search_text,
            Style::default().fg(Color::White).bg(Color::Black),
        )]);

        let paragraph = Paragraph::new(search_line).style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, area);
    }

    /// Calculates the picker overlay area (centered, takes up 80% of screen)
    fn get_picker_area(full_area: Rect) -> Rect {
        let width = (full_area.width * 80) / 100;
        let height = (full_area.height * 60) / 100;
        let x = (full_area.width - width) / 2;
        let y = (full_area.height - height) / 2;

        Rect::new(x, y, width.max(60), height.max(15))
    }

    /// Determines if we should show the preview panel based on available width
    fn should_show_preview(area: Rect) -> bool {
        // Show preview only if we have at least 100 columns total
        // This leaves ~40 cols for the list and ~60 for preview
        area.width >= 100
    }

    /// Renders the picker overlay
    fn render_picker(frame: &mut Frame, editor: &mut Editor, _full_area: Rect) {
        let Some(picker) = editor.picker() else {
            return;
        };

        let picker_area = Self::get_picker_area(frame.area());
        let show_preview = Self::should_show_preview(picker_area);

        // Create block with rounded border and styled colors
        let mode_name = match picker.mode() {
            crate::editor::PickerMode::FindFiles => " 󰈞 Find Files ",
            crate::editor::PickerMode::LiveGrep => " 󰺮 Live Grep ",
            crate::editor::PickerMode::Custom => " 󰒉 Select ",
            crate::editor::PickerMode::Completion => "  Completion ",
            crate::editor::PickerMode::LspLocations => " 󰘧 Navigation ",
        };

        // Richer background with gradient-like effect
        let block = Block::default()
            .title(mode_name)
            .title_style(
                Style::default()
                    .fg(Color::Rgb(165, 180, 252)) // Soft indigo
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(100, 116, 180))) // Muted purple-blue
            .border_type(ratatui::widgets::BorderType::Rounded)
            .style(Style::default().bg(Color::Rgb(20, 24, 35))); // Deep navy

        frame.render_widget(block.clone(), picker_area);

        // Split picker area into left (query + results) and right (preview)
        let inner_area = block.inner(picker_area);
        let main_chunks = if show_preview {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
                .split(inner_area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(100)].as_ref())
                .split(inner_area)
        };

        // Split left side into query line + separator + results
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(1), // Query line
                    Constraint::Length(1), // Separator
                    Constraint::Min(1),    // Results
                ]
                .as_ref(),
            )
            .split(main_chunks[0]);

        // Render query line with enhanced styling
        let query_text = picker.query();
        let cursor_pos = picker.query_cursor();
        let prompt_icon = " ";

        // Split query text at cursor position to render cursor in the right place
        let chars: Vec<char> = query_text.chars().collect();
        let before_cursor: String = chars.iter().take(cursor_pos).collect();
        let after_cursor: String = chars.iter().skip(cursor_pos).collect();

        // Calculate padding before moving strings into spans
        let query_line_width = left_chunks[0].width as usize;
        let content_len = 2 + before_cursor.len() + 1 + after_cursor.len(); // icon + space + cursor + text
        let padding = query_line_width.saturating_sub(content_len);

        let mut spans = vec![
            Span::styled(
                prompt_icon,
                Style::default()
                    .fg(Color::Rgb(129, 250, 183)) // Soft green
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(
                before_cursor,
                Style::default()
                    .fg(Color::Rgb(220, 220, 230)) // Near white
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "▊", // Cursor block
                Style::default()
                    .fg(Color::Rgb(165, 180, 252))
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ];

        if !after_cursor.is_empty() {
            spans.push(Span::styled(
                after_cursor,
                Style::default()
                    .fg(Color::Rgb(220, 220, 230))
                    .add_modifier(Modifier::BOLD),
            ));
        }

        // Add padding to fill the rest of the line with background color
        if padding > 0 {
            spans.push(Span::styled(
                " ".repeat(padding),
                Style::default().bg(Color::Rgb(20, 24, 35)),
            ));
        }

        let query_line = Line::from(spans);
        let query_paragraph =
            Paragraph::new(query_line).style(Style::default().bg(Color::Rgb(20, 24, 35)));
        frame.render_widget(query_paragraph, left_chunks[0]);

        // Render separator line
        let separator = "─".repeat(left_chunks[1].width as usize);
        let separator_line = Line::from(Span::styled(
            separator,
            Style::default()
                .fg(Color::Rgb(60, 70, 100)) // Subtle line
                .bg(Color::Rgb(20, 24, 35)), // Background color
        ));
        let separator_paragraph =
            Paragraph::new(separator_line).style(Style::default().bg(Color::Rgb(20, 24, 35)));
        frame.render_widget(separator_paragraph, left_chunks[1]);

        // Render results
        let results = picker.filtered_results();
        let selected_idx = picker.selected_index();
        let max_results = left_chunks[2].height as usize;
        let result_width = left_chunks[2].width as usize;

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

                // Truncate the display path if needed
                let max_display_len = result_width.saturating_sub(5); // Room for icon + prefix + padding
                let display =
                    crate::editor::Picker::truncate_path(&result.display, max_display_len);

                // Choose icon based on file type or result type
                let icon = if result.line > 0 {
                    " " // Search result icon
                } else if display.ends_with('/') {
                    " " // Directory icon
                } else {
                    " " // File icon
                };

                let (icon_style, text_style, bg_color) = if is_selected {
                    (
                        Style::default()
                            .fg(Color::Rgb(129, 250, 183)) // Bright green for icon
                            .add_modifier(Modifier::BOLD),
                        Style::default()
                            .fg(Color::Rgb(240, 240, 255)) // Bright text
                            .bg(Color::Rgb(55, 65, 95)) // Highlighted background
                            .add_modifier(Modifier::BOLD),
                        Color::Rgb(55, 65, 95),
                    )
                } else {
                    (
                        Style::default().fg(Color::Rgb(120, 130, 160)), // Muted icon
                        Style::default()
                            .fg(Color::Rgb(180, 185, 200)) // Light gray text
                            .bg(Color::Rgb(20, 24, 35)),
                        Color::Rgb(20, 24, 35),
                    )
                };

                let prefix = if is_selected { " ▸ " } else { "   " };
                let text_content = format!("{}{}", prefix, display);

                // Calculate padding
                let content_len = icon.chars().count() + text_content.chars().count();
                let padding = result_width.saturating_sub(content_len);

                Line::from(vec![
                    Span::styled(icon, icon_style),
                    Span::styled(text_content, text_style),
                    Span::styled(" ".repeat(padding), Style::default().bg(bg_color)),
                ])
            })
            .collect();

        // Show results or "No matches" message
        let mut all_lines = visible_results;

        if results.is_empty() {
            // Truly no matches
            let text = "  󰍉 No matches found";
            let padding = result_width.saturating_sub(text.chars().count());
            all_lines.push(Line::from(vec![
                Span::styled(
                    text,
                    Style::default()
                        .fg(Color::Rgb(240, 120, 120)) // Soft red
                        .bg(Color::Rgb(20, 24, 35)),
                ),
                Span::styled(
                    " ".repeat(padding),
                    Style::default().bg(Color::Rgb(20, 24, 35)),
                ),
            ]));
        } else {
            // Add result count at the bottom if there's space
            if all_lines.len() < max_results {
                let result_count = format!(
                    "  {} result{}",
                    results.len(),
                    if results.len() == 1 { "" } else { "s" }
                );
                let padding = result_width.saturating_sub(result_count.len());
                all_lines.push(Line::from(vec![
                    Span::styled(
                        result_count,
                        Style::default()
                            .fg(Color::Rgb(100, 110, 140)) // Very muted
                            .bg(Color::Rgb(20, 24, 35))
                            .add_modifier(Modifier::ITALIC),
                    ),
                    Span::styled(
                        " ".repeat(padding),
                        Style::default().bg(Color::Rgb(20, 24, 35)),
                    ),
                ]));
            }
        }

        // Fill remaining lines with empty spans that have background color
        let lines_to_fill = max_results.saturating_sub(all_lines.len());
        for _ in 0..lines_to_fill {
            all_lines.push(Line::from(vec![Span::styled(
                " ".repeat(result_width),
                Style::default().bg(Color::Rgb(20, 24, 35)),
            )]));
        }

        let results_paragraph =
            Paragraph::new(all_lines).style(Style::default().bg(Color::Rgb(20, 24, 35)));
        frame.render_widget(results_paragraph, left_chunks[2]);

        // Get selected result (need to clone to release immutable borrow of picker)
        let selected_result = picker.selected_result().cloned();

        // Drop immutable borrow of picker before calling functions that need mutable borrow
        drop(picker);

        // Render preview panel if enabled
        if show_preview {
            if let Some(selected) = selected_result {
                Self::render_picker_preview(frame, editor, &selected, main_chunks[1]);
            } else {
                // Render empty state when no selection
                Self::render_picker_empty_state(frame, main_chunks[1]);
            }
        }
    }

    /// Renders the file preview for the picker
    fn render_picker_preview(
        frame: &mut Frame,
        editor: &mut crate::editor::Editor,
        result: &crate::editor::PickerResult,
        area: Rect,
    ) {
        // Add border around preview with enhanced styling
        let preview_block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(Color::Rgb(60, 70, 100))) // Subtle divider
            .style(Style::default().bg(Color::Rgb(25, 29, 40))); // Slightly different background

        let inner_area = preview_block.inner(area);
        frame.render_widget(preview_block, area);

        // Try to get preview (only show exact match, no fallback to avoid scroll artifacts)
        let file_path = &result.location;
        let preview = match editor.get_preview_cache(file_path) {
            Some(p) => p,
            None => {
                // Not cached yet - show loading message
                let loading_msg = " 󰦖  Loading preview...";
                let paragraph = Paragraph::new(loading_msg)
                    .style(
                        Style::default()
                            .fg(Color::Rgb(120, 130, 160))
                            .bg(Color::Rgb(25, 29, 40))
                            .add_modifier(Modifier::ITALIC),
                    )
                    .alignment(ratatui::layout::Alignment::Center);

                // Center vertically
                let centered_area = Rect {
                    x: inner_area.x,
                    y: inner_area.y + inner_area.height / 2,
                    width: inner_area.width,
                    height: 1,
                };
                frame.render_widget(paragraph, centered_area);
                return;
            }
        };

        let theme = crate::syntax::Theme::default();
        let mut lines_to_render = Vec::new();

        let max_lines = inner_area.height as usize;
        let total_lines = preview.content.lines().count();

        // Calculate which lines to show
        let (start_line, end_line) = if result.line > 0 && result.line < total_lines {
            // For LiveGrep results, center around the matched line
            let context = max_lines / 2;
            let start = result.line.saturating_sub(context);
            let end = (result.line + context).min(total_lines);
            (start, end)
        } else {
            // For file finder, show from the top
            (0, max_lines.min(total_lines))
        };

        if let Some(lang) = preview.language {
            // Use syntax highlighting
            match crate::syntax::SyntaxHighlighter::new(lang) {
                Ok(mut highlighter) => {
                    // Parse only once if not already parsed
                    // Note: We can't cache the parsed tree easily, but we can cache the highlights per line
                    let mut need_parsing = false;

                    // Check if we need to parse (if any line in our range is not cached)
                    {
                        let cache = preview.highlighted_lines.borrow();
                        for line_idx in start_line..end_line {
                            if !cache.contains_key(&line_idx) {
                                need_parsing = true;
                                break;
                            }
                        }
                    }

                    // Parse if needed
                    if need_parsing {
                        highlighter.parse(&preview.content);

                        // Cache highlights for the visible range
                        let mut cache = preview.highlighted_lines.borrow_mut();
                        for line_idx in start_line..end_line {
                            if !cache.contains_key(&line_idx) {
                                let highlights =
                                    highlighter.highlights_for_line(line_idx, &preview.content);
                                cache.insert(line_idx, highlights);
                            }
                        }
                    }

                    for (line_idx, line_text) in preview.content.lines().enumerate() {
                        if line_idx < start_line {
                            continue;
                        }
                        if line_idx >= end_line {
                            break;
                        }

                        // Expand tabs in preview content and get byte mapping
                        let (line_text, tab_mapping) = Self::expand_tabs_with_mapping(line_text, 4); // Use default tab width for previews

                        // Truncate line to fit width (line number prefix is 7 chars: "  1 │ ")
                        let content_width = inner_area.width.saturating_sub(7) as usize;
                        let line_text = Self::truncate_to_width(&line_text, content_width);

                        // Get highlights from cache and remap for expanded tabs
                        let original_highlights = preview
                            .highlighted_lines
                            .borrow()
                            .get(&line_idx)
                            .cloned()
                            .unwrap_or_default();
                        let highlights = Self::remap_highlights(&original_highlights, &tab_mapping);
                        let is_target_line =
                            result.line > 0 && result.line < total_lines && line_idx == result.line;

                        // Build the line with syntax highlighting
                        let mut spans = Vec::new();

                        // Add line number prefix
                        let line_num = format!("{:>4} │ ", line_idx + 1);
                        let line_num_style = if is_target_line {
                            Style::default()
                                .fg(Color::Rgb(129, 250, 183))
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Rgb(100, 110, 140))
                        };
                        spans.push(Span::styled(line_num, line_num_style));

                        // Add syntax-highlighted content
                        let chars: Vec<char> = line_text.chars().collect();

                        // Build a map from character index to byte index
                        let mut byte_indices: Vec<usize> = Vec::with_capacity(chars.len() + 1);
                        byte_indices.push(0);
                        for (byte_idx, _) in line_text.char_indices().skip(1) {
                            byte_indices.push(byte_idx);
                        }
                        byte_indices.push(line_text.len());

                        let mut col_idx = 0;

                        while col_idx < chars.len() {
                            // Find the syntax group for this character (convert to byte index)
                            let byte_idx = byte_indices[col_idx];
                            let syntax_group = highlights
                                .iter()
                                .find(|(range, _)| range.contains(&byte_idx))
                                .map(|(_, group)| *group);

                            // Find the end of this styled region
                            let mut end_col = col_idx + 1;
                            while end_col < chars.len() {
                                let next_byte_idx = byte_indices[end_col];
                                let next_group = highlights
                                    .iter()
                                    .find(|(range, _)| range.contains(&next_byte_idx))
                                    .map(|(_, group)| *group);

                                if next_group != syntax_group {
                                    break;
                                }
                                end_col += 1;
                            }

                            let text: String = chars[col_idx..end_col].iter().collect();
                            let mut style = if let Some(group) = syntax_group {
                                let color = theme.get_color(group);
                                Style::default().fg(color)
                            } else {
                                Style::default().fg(Color::White)
                            };

                            // Highlight the target line
                            if is_target_line {
                                style = style.bg(Color::Rgb(55, 65, 95));
                            }

                            spans.push(Span::styled(text, style));
                            col_idx = end_col;
                        }

                        lines_to_render.push(Line::from(spans));
                    }
                }
                Err(_) => {
                    // Fall back to plain text
                    Self::render_plain_preview(
                        &preview.content,
                        result,
                        inner_area,
                        &mut lines_to_render,
                    );
                }
            }
        } else {
            // No syntax highlighting available, show plain text
            Self::render_plain_preview(&preview.content, result, inner_area, &mut lines_to_render);
        }

        let paragraph =
            Paragraph::new(lines_to_render).style(Style::default().bg(Color::Rgb(25, 29, 40)));
        frame.render_widget(paragraph, inner_area);
    }

    /// Renders plain text preview without syntax highlighting
    fn render_plain_preview(
        content: &str,
        result: &crate::editor::PickerResult,
        area: Rect,
        lines: &mut Vec<Line<'static>>,
    ) {
        let max_lines = area.height as usize;
        let total_lines = content.lines().count();

        // Calculate which lines to show
        let (start_line, end_line) = if result.line > 0 && result.line < total_lines {
            let context = max_lines / 2;
            let start = result.line.saturating_sub(context);
            let end = (result.line + context).min(total_lines);
            (start, end)
        } else {
            (0, max_lines.min(total_lines))
        };

        for (line_idx, line_text) in content.lines().enumerate() {
            if line_idx < start_line {
                continue;
            }
            if line_idx >= end_line {
                break;
            }

            // Expand tabs in plain preview
            let line_text = Self::expand_tabs(line_text, 4);

            // Truncate line to fit width (line number prefix is 7 chars: "  1 │ ")
            let content_width = area.width.saturating_sub(7) as usize;
            let line_text = Self::truncate_to_width(&line_text, content_width);

            let is_target_line =
                result.line > 0 && result.line < total_lines && line_idx == result.line;

            let line_num = format!("{:>4} │ ", line_idx + 1);
            let line_num_style = if is_target_line {
                Style::default()
                    .fg(Color::Rgb(129, 250, 183))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(100, 110, 140))
            };

            let text_style = if is_target_line {
                Style::default().fg(Color::White).bg(Color::Rgb(55, 65, 95))
            } else {
                Style::default().fg(Color::Rgb(200, 205, 220))
            };

            lines.push(Line::from(vec![
                Span::styled(line_num, line_num_style),
                Span::styled(line_text.to_string(), text_style),
            ]));
        }
    }

    /// Renders empty state for the picker preview panel
    fn render_picker_empty_state(frame: &mut Frame, area: Rect) {
        // Add border around preview with enhanced styling
        let preview_block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(Color::Rgb(60, 70, 100))) // Subtle divider
            .style(Style::default().bg(Color::Rgb(25, 29, 40))); // Slightly different background

        let inner_area = preview_block.inner(area);
        frame.render_widget(preview_block, area);

        // Show centered empty state message
        let empty_msg = " 󰈈  No file selected";
        let paragraph = Paragraph::new(empty_msg)
            .style(
                Style::default()
                    .fg(Color::Rgb(100, 110, 140)) // Muted color
                    .bg(Color::Rgb(25, 29, 40))
                    .add_modifier(Modifier::ITALIC),
            )
            .alignment(ratatui::layout::Alignment::Center);

        // Center vertically
        let centered_area = Rect {
            x: inner_area.x,
            y: inner_area.y + inner_area.height / 2,
            width: inner_area.width,
            height: 1,
        };
        frame.render_widget(paragraph, centered_area);
    }

    /// Renders a single line with all highlighting (syntax, visual selection, search)
    fn render_line_with_highlights(
        theme: &Theme,
        line_text: &str,
        line_idx: usize,
        visual_selection: Option<((usize, usize), (usize, usize))>,
        mode: crate::mode::Mode,
        search_matches: &[(usize, usize)],
        syntax_highlights: &[(std::ops::Range<usize>, crate::syntax::HighlightGroup)],
    ) -> Line<'static> {
        let chars: Vec<char> = line_text.chars().collect();
        let mut spans = Vec::new();

        // Build a map from character index to byte index
        let mut byte_indices: Vec<usize> = Vec::with_capacity(chars.len() + 1);
        byte_indices.push(0);
        for (byte_idx, _) in line_text.char_indices().skip(1) {
            byte_indices.push(byte_idx);
        }
        byte_indices.push(line_text.len()); // End position

        let mut col_idx = 0;
        while col_idx < chars.len() {
            // Check if this character is in visual selection
            let is_selected =
                if let Some(((sel_start_line, sel_start_col), (sel_end_line, sel_end_col))) =
                    visual_selection
                {
                    match mode {
                        crate::mode::Mode::VisualBlock => {
                            // Block mode: check if within the rectangular region
                            line_idx >= sel_start_line
                                && line_idx <= sel_end_line
                                && col_idx >= sel_start_col
                                && col_idx <= sel_end_col
                        }
                        _ => {
                            // Character-wise or line-wise visual mode
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
                        }
                    }
                } else {
                    false
                };

            // Check if this character is in a search match
            let is_search_match = search_matches
                .iter()
                .any(|(start, end)| col_idx >= *start && col_idx < *end);

            // Check if this character is in a syntax highlight (convert char index to byte index)
            let byte_idx = byte_indices[col_idx];
            let syntax_group = syntax_highlights
                .iter()
                .find(|(range, _)| range.contains(&byte_idx))
                .map(|(_, group)| *group);

            // Determine how many characters share the same styling
            let mut end_col = col_idx + 1;
            while end_col < chars.len() {
                let next_selected =
                    if let Some(((sel_start_line, sel_start_col), (sel_end_line, sel_end_col))) =
                        visual_selection
                    {
                        match mode {
                            crate::mode::Mode::VisualBlock => {
                                // Block mode: check if within the rectangular region
                                line_idx >= sel_start_line
                                    && line_idx <= sel_end_line
                                    && end_col >= sel_start_col
                                    && end_col <= sel_end_col
                            }
                            _ => {
                                // Character-wise or line-wise visual mode
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
                            }
                        }
                    } else {
                        false
                    };

                let next_search_match = search_matches
                    .iter()
                    .any(|(start, end)| end_col >= *start && end_col < *end);

                // Convert char index to byte index for syntax highlight lookup
                let next_byte_idx = byte_indices[end_col];
                let next_syntax_group = syntax_highlights
                    .iter()
                    .find(|(range, _)| range.contains(&next_byte_idx))
                    .map(|(_, group)| *group);

                // If styling changes, break
                if next_selected != is_selected
                    || next_search_match != is_search_match
                    || next_syntax_group != syntax_group
                {
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
