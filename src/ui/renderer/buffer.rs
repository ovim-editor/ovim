use crate::editor::Editor;
use crate::syntax::Theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::helpers::expand_tabs_with_mapping;
use super::styles::{get_git_sign_style, get_line_number_style, remap_highlights};

/// Renders the buffer content and returns the viewport start line
pub fn render_buffer(frame: &mut Frame, editor: &Editor, theme: &Theme, area: Rect) -> usize {
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
        render_gutter(
            frame,
            editor,
            buffer,
            gutter_area,
            start_line,
            end_line,
            line_num_width,
            cursor.line(),
        );
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
            let (line_text, byte_mapping) = expand_tabs_with_mapping(line_text, tab_width);

            // Get syntax highlights for this line and remap them for expanded text
            let original_highlights = buffer.highlights_for_line(line_idx);
            let syntax_highlights = remap_highlights(&original_highlights, &byte_mapping);

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
                let mut line = render_line_with_highlights(
                    theme,
                    &line_text,
                    line_idx,
                    visual_selection,
                    editor.mode(),
                    &search_matches,
                    &syntax_highlights,
                );
                // Pad line to clear previous content
                let line_len: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
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

/// Renders the gutter (line numbers and git signs)
fn render_gutter(
    frame: &mut Frame,
    editor: &Editor,
    buffer: &crate::buffer::Buffer,
    area: Rect,
    start_line: usize,
    end_line: usize,
    line_num_width: usize,
    cursor_line: usize,
) {
    let rope = buffer.rope();
    let mut gutter_lines = Vec::new();

    for line_idx in start_line..end_line {
        if line_idx < rope.len_lines() {
            let line_num_text = if editor.options.relative_number {
                // Relative line numbers
                let rel = if line_idx == cursor_line {
                    line_idx + 1 // Show absolute for current line
                } else {
                    line_idx.abs_diff(cursor_line)
                };
                format!("{:>width$} ", rel, width = line_num_width)
            } else if editor.options.number {
                // Absolute line numbers
                format!("{:>width$} ", line_idx + 1, width = line_num_width)
            } else {
                "  ".to_string()
            };

            // Add sign column for git status indicators
            let git_status = buffer.git_status().get_line_status(line_idx);
            let (sign_text, sign_color) = get_git_sign_style(git_status);

            // Highlight current line number
            let line_num_style = get_line_number_style(line_idx == cursor_line);

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
    frame.render_widget(gutter_paragraph, area);
}

/// Renders a single line with all highlighting (syntax, visual selection, search)
pub fn render_line_with_highlights(
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
                        } else {
                            line_idx > sel_start_line && line_idx < sel_end_line
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
                            } else {
                                line_idx > sel_start_line && line_idx < sel_end_line
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
