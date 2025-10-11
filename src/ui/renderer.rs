use crate::editor::Editor;
use crate::LineStatus;
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
        // Get color scheme from editor, fall back to Tokyonight if not found
        let scheme = editor.get_color_scheme()
            .cloned()
            .unwrap_or_else(|| crate::syntax::ColorScheme::tokyonight());
        let theme = Theme::from_scheme(scheme);
        let cursor_pos = editor.buffer().cursor();
        let cursor_line = cursor_pos.line();
        let cursor_col = cursor_pos.col();

        // Layout: [file tree (optional)] + main buffer + status line
        let main_area = frame.area();

        // First split: file tree (if visible) + rest
        let (file_tree_area, content_area) = if editor.file_tree().is_visible() {
            let horizontal_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(30), Constraint::Min(1)].as_ref())
                .split(main_area);
            (Some(horizontal_chunks[0]), horizontal_chunks[1])
        } else {
            (None, main_area)
        };

        // Second split: buffer + status line
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
            .split(content_area);

        // Render the file tree if visible
        if let Some(tree_area) = file_tree_area {
            Self::render_file_tree(frame, editor, tree_area);
        }

        // Render the main text area
        let viewport_start = Self::render_buffer(frame, editor, &theme, chunks[0]);

        // Render the status line or command line or search line
        let status_chunk = chunks[1];
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
                frame.set_cursor_position((
                    picker_area.x + 1 + 2 + cursor_pos as u16, // +1 for border, +2 for "> " prefix
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
            // Position cursor in text buffer (accounting for gutter)
            let screen_line = cursor_line.saturating_sub(viewport_start);
            let cursor_x = cursor_col.min(chunks[0].width.saturating_sub(1) as usize);
            let cursor_y = screen_line.min(chunks[0].height.saturating_sub(1) as usize);

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
    pub fn render(&mut self, editor: &Editor) -> Result<()> {
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
                .constraints([
                    Constraint::Length(gutter_width),
                    Constraint::Min(1),
                ])
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
                    let (sign_text, sign_color) = match buffer.git_status().get_line_status(line_idx) {
                        Some(LineStatus::Added) => ("+ ", Color::Green),
                        Some(LineStatus::Modified) => ("~ ", Color::Yellow),
                        Some(LineStatus::Removed) => ("- ", Color::Red),
                        None => ("  ", Color::DarkGray),
                    };

                    // Highlight current line number
                    let line_num_style = if line_idx == cursor.line() {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };

                    // Build gutter with separate styles for sign and line number
                    let sign_span = Span::styled(sign_text, Style::default().fg(sign_color).add_modifier(Modifier::BOLD));
                    let line_num_span = Span::styled(line_num_text, line_num_style);

                    gutter_lines.push(Line::from(vec![sign_span, line_num_span]));
                }
            }

            let gutter_paragraph = Paragraph::new(gutter_lines);
            frame.render_widget(gutter_paragraph, gutter_area);
        }

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
                        editor.mode(),
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
        frame.render_widget(paragraph, text_area);

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

        let padding_len = (area.width as usize)
            .saturating_sub(mode_indicator.len())
            .saturating_sub(file.len())
            .saturating_sub(modified.len())
            .saturating_sub(diagnostics.len())
            .saturating_sub(lsp_status.len())
            .saturating_sub(position.len())
            .saturating_sub(1);

        let mut spans = vec![
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
        ];

        // Add diagnostics indicator if present
        if !diagnostics.is_empty() {
            spans.push(Span::styled(
                &diagnostics,
                Style::default()
                    .fg(Color::Black)
                    .bg(if errors > 0 { Color::Red } else { Color::Yellow }),
            ));
        }

        // Add LSP status if present
        if !lsp_status.is_empty() {
            let lsp_color = if editor.lsp_status().contains("Failed") || editor.lsp_status().contains("Error") {
                Color::Red
            } else if editor.lsp_status().contains("ready") {
                Color::Green
            } else {
                Color::Blue
            };
            spans.push(Span::styled(
                &lsp_status,
                Style::default()
                    .fg(Color::Black)
                    .bg(lsp_color),
            ));
        }

        spans.push(Span::styled(
            position,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Gray),
        ));

        let status_line = Line::from(spans);

        let paragraph = Paragraph::new(status_line)
            .style(Style::default().bg(Color::DarkGray));
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
            .style(Style::default()
                .bg(Color::Rgb(30, 30, 40))
                .fg(Color::Rgb(230, 230, 230)))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(255, 200, 100)))
                    .title(title)
                    .title_style(Style::default()
                        .fg(Color::Rgb(255, 200, 100))
                        .add_modifier(Modifier::BOLD)),
            )
            .wrap(ratatui::widgets::Wrap { trim: false });

        // Clear background and render window
        frame.render_widget(ratatui::widgets::Clear, window_area);
        frame.render_widget(paragraph, window_area);
    }

    /// Renders the completion menu popup
    fn render_completion_menu(frame: &mut Frame, editor: &Editor, buffer_area: Rect, viewport_start: usize) {
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
        let menu_x = buffer_area.x + gutter_width as u16 + cursor_col as u16;
        let menu_y = buffer_area.y + screen_line as u16 + 1; // Below current line

        // Determine menu dimensions
        let max_items_to_show = 10;
        let num_items = items.len().min(max_items_to_show);
        let menu_height = num_items as u16 + 2; // +2 for borders

        // Calculate width based on longest label
        let max_label_len = items.iter()
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
                Style::default()
                    .bg(Color::Rgb(40, 44, 52))
                    .fg(Color::White)
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
                    .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            )
            .style(Style::default().bg(Color::Rgb(30, 34, 42)));

        frame.render_widget(list, area);
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
    fn render_picker(frame: &mut Frame, editor: &Editor, _full_area: Rect) {
        let Some(picker) = editor.picker() else { return };

        let picker_area = Self::get_picker_area(frame.area());
        let show_preview = Self::should_show_preview(picker_area);

        // Create block with border
        let mode_name = match picker.mode() {
            crate::editor::PickerMode::FindFiles => "Find Files",
            crate::editor::PickerMode::LiveGrep => "Live Grep",
            crate::editor::PickerMode::Custom => "Select",
            crate::editor::PickerMode::Completion => "Completion",
            crate::editor::PickerMode::LspLocations => "LSP Navigation",
        };

        let block = Block::default()
            .title(mode_name)
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black).fg(Color::White));

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

        // Split left side into query line and results
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)].as_ref())
            .split(main_chunks[0]);

        let chunks = left_chunks;

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

                // Truncate the display path if needed
                let max_display_len = result_width.saturating_sub(4); // Leave room for "  " prefix and padding
                let display = crate::editor::Picker::truncate_path(&result.display, max_display_len);
                let text = format!("  {}", display);

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

        // Render preview panel if enabled and we have a selection
        if show_preview {
            if let Some(selected) = picker.selected_result() {
                Self::render_picker_preview(frame, editor, selected, main_chunks[1]);
            }
        }
    }

    /// Renders the file preview for the picker
    fn render_picker_preview(frame: &mut Frame, editor: &Editor, result: &crate::editor::PickerResult, area: Rect) {
        use std::path::Path;

        // Add border around preview
        let preview_block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(Color::DarkGray));

        let inner_area = preview_block.inner(area);
        frame.render_widget(preview_block, area);

        // Try to get cached preview
        let file_path = &result.location;
        let preview = match editor.get_preview_cache(file_path) {
            Some(p) => p,
            None => {
                // Show loading message (cache miss, will be loaded on next frame)
                let loading_msg = "Loading preview...";
                let paragraph = Paragraph::new(loading_msg)
                    .style(Style::default().fg(Color::DarkGray).bg(Color::Black));
                frame.render_widget(paragraph, inner_area);
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
                                let highlights = highlighter.highlights_for_line(line_idx, &preview.content);
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

                        // Get highlights from cache
                        let highlights = preview.highlighted_lines.borrow().get(&line_idx).cloned().unwrap_or_default();
                        let is_target_line = result.line > 0 && result.line < total_lines && line_idx == result.line;

                        // Build the line with syntax highlighting
                        let mut spans = Vec::new();

                        // Add line number prefix
                        let line_num = format!("{:>4} │ ", line_idx + 1);
                        let line_num_style = if is_target_line {
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        spans.push(Span::styled(line_num, line_num_style));

                        // Add syntax-highlighted content
                        let chars: Vec<char> = line_text.chars().collect();
                        let mut col_idx = 0;

                        while col_idx < chars.len() {
                            // Find the syntax group for this character
                            let syntax_group = highlights.iter()
                                .find(|(range, _)| range.contains(&col_idx))
                                .map(|(_, group)| *group);

                            // Find the end of this styled region
                            let mut end_col = col_idx + 1;
                            while end_col < chars.len() {
                                let next_group = highlights.iter()
                                    .find(|(range, _)| range.contains(&end_col))
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
                                style = style.bg(Color::Rgb(40, 40, 60));
                            }

                            spans.push(Span::styled(text, style));
                            col_idx = end_col;
                        }

                        lines_to_render.push(Line::from(spans));
                    }
                }
                Err(_) => {
                    // Fall back to plain text
                    Self::render_plain_preview(&preview.content, result, inner_area, &mut lines_to_render);
                }
            }
        } else {
            // No syntax highlighting available, show plain text
            Self::render_plain_preview(&preview.content, result, inner_area, &mut lines_to_render);
        }

        let paragraph = Paragraph::new(lines_to_render)
            .style(Style::default().bg(Color::Black));
        frame.render_widget(paragraph, inner_area);
    }

    /// Renders plain text preview without syntax highlighting
    fn render_plain_preview(content: &str, result: &crate::editor::PickerResult, area: Rect, lines: &mut Vec<Line<'static>>) {
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

            let is_target_line = result.line > 0 && result.line < total_lines && line_idx == result.line;

            let line_num = format!("{:>4} │ ", line_idx + 1);
            let line_num_style = if is_target_line {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let text_style = if is_target_line {
                Style::default().fg(Color::White).bg(Color::Rgb(40, 40, 60))
            } else {
                Style::default().fg(Color::White)
            };

            lines.push(Line::from(vec![
                Span::styled(line_num, line_num_style),
                Span::styled(line_text.to_string(), text_style),
            ]));
        }
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

        let mut col_idx = 0;
        while col_idx < chars.len() {
            // Check if this character is in visual selection
            let is_selected = if let Some(((sel_start_line, sel_start_col), (sel_end_line, sel_end_col))) = visual_selection {
                match mode {
                    crate::mode::Mode::VisualBlock => {
                        // Block mode: check if within the rectangular region
                        line_idx >= sel_start_line && line_idx <= sel_end_line &&
                        col_idx >= sel_start_col && col_idx <= sel_end_col
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
                    match mode {
                        crate::mode::Mode::VisualBlock => {
                            // Block mode: check if within the rectangular region
                            line_idx >= sel_start_line && line_idx <= sel_end_line &&
                            end_col >= sel_start_col && end_col <= sel_end_col
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
