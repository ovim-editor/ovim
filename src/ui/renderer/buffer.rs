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

/// Apply a style to a specific column in a line
fn apply_style_at_column(line: &mut Line<'static>, target_col: usize, style: Style) {
    let mut current_col = 0;
    for span in &mut line.spans {
        let span_len = span.content.chars().count();
        if target_col >= current_col && target_col < current_col + span_len {
            // Target column is in this span - need to split it
            let offset = target_col - current_col;
            let chars: Vec<char> = span.content.chars().collect();
            if offset == 0 && chars.len() == 1 {
                // Simple case: span is exactly the bracket character
                span.style = span.style.patch(style);
            } else {
                // Need to split: this is complex, so just apply style to whole span for now
                // A proper implementation would split the span into 3 parts
                span.style = span.style.patch(style);
            }
            return;
        }
        current_col += span_len;
    }
}

/// Find matching bracket position if cursor is on a bracket
fn find_matching_bracket_position(buffer: &crate::buffer::Buffer) -> Option<(usize, usize)> {
    let cursor = buffer.cursor();
    let rope = buffer.rope();
    let line_idx = cursor.line();

    if line_idx >= rope.len_lines() {
        return None;
    }

    let line = rope.line(line_idx);
    let col = cursor.col();

    if col >= line.len_chars() {
        return None;
    }

    let current_char = line.char(col);

    // Check if on a bracket
    let (matching_char, search_forward) = match current_char {
        '(' => (')', true),
        ')' => ('(', false),
        '[' => (']', true),
        ']' => ('[', false),
        '{' => ('}', true),
        '}' => ('{', false),
        '<' => ('>', true),
        '>' => ('<', false),
        _ => return None,
    };

    // Calculate absolute position
    let abs_pos = rope.line_to_char(line_idx) + col;
    let total_chars = rope.len_chars();

    // Search for matching bracket
    let mut depth = 1;
    let mut pos = abs_pos;

    if search_forward {
        pos += 1;
        while pos < total_chars && depth > 0 {
            let c = rope.char(pos);
            if c == current_char {
                depth += 1;
            } else if c == matching_char {
                depth -= 1;
            }
            if depth > 0 {
                pos += 1;
            }
        }
    } else {
        if pos == 0 {
            return None;
        }
        pos -= 1;
        while depth > 0 {
            let c = rope.char(pos);
            if c == current_char {
                depth += 1;
            } else if c == matching_char {
                depth -= 1;
            }
            if depth > 0 {
                if pos == 0 {
                    return None;
                }
                pos -= 1;
            }
        }
    }

    if depth == 0 {
        // Convert absolute position to line/col
        let match_line = rope.char_to_line(pos);
        let line_start = rope.line_to_char(match_line);
        let match_col = pos - line_start;
        Some((match_line, match_col))
    } else {
        None
    }
}

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

    // Find matching bracket position if showmatch is enabled
    let bracket_positions: Option<((usize, usize), (usize, usize))> = if editor.options.showmatch {
        if let Some(matching_pos) = find_matching_bracket_position(buffer) {
            Some(((cursor.line(), cursor.col()), matching_pos))
        } else {
            None
        }
    } else {
        None
    };

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
    let cursorline = editor.options.cursorline;
    let cursor_line_idx = cursor.line();

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

            // Check if this is the cursor line and cursorline option is on
            let is_cursor_line = cursorline && line_idx == cursor_line_idx;

            // Check if this line has a bracket to highlight
            let bracket_col = bracket_positions.and_then(|((l1, c1), (l2, c2))| {
                if line_idx == l1 {
                    Some(c1)
                } else if line_idx == l2 {
                    Some(c2)
                } else {
                    None
                }
            });

            // Get diagnostics for this line
            let line_diagnostics = editor.diagnostics_for_line(line_idx);
            let has_diagnostics = !line_diagnostics.is_empty();

            // Always use character-by-character rendering if we have any highlighting
            let needs_detailed_rendering = has_visual_selection
                || !search_matches.is_empty()
                || !syntax_highlights.is_empty()
                || is_cursor_line
                || bracket_col.is_some()
                || has_diagnostics;

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

                // Add diagnostic virtual text if present
                if has_diagnostics {
                    use lsp_types::DiagnosticSeverity;
                    // Get the first (most severe) diagnostic
                    let diag = line_diagnostics[0];
                    let diag_color = match diag.severity {
                        Some(DiagnosticSeverity::ERROR) => Color::Red,
                        Some(DiagnosticSeverity::WARNING) => Color::Yellow,
                        Some(DiagnosticSeverity::INFORMATION) => Color::Cyan,
                        Some(DiagnosticSeverity::HINT) => Color::Gray,
                        _ => Color::Gray,
                    };
                    // Truncate message to fit on screen
                    let max_msg_len = (text_area.width as usize).saturating_sub(line_text.chars().count() + 3);
                    let msg = diag.message.lines().next().unwrap_or("");
                    let msg = if msg.chars().count() > max_msg_len {
                        format!("{}...", msg.chars().take(max_msg_len.saturating_sub(3)).collect::<String>())
                    } else {
                        msg.to_string()
                    };
                    line.spans.push(Span::styled(
                        format!(" // {}", msg),
                        Style::default().fg(diag_color).add_modifier(Modifier::ITALIC),
                    ));
                }

                // Pad line to clear previous content
                let line_len: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
                if line_len < text_area.width as usize {
                    line.spans
                        .push(Span::raw(" ".repeat(text_area.width as usize - line_len)));
                }
                // Apply cursorline background if this is the cursor line
                if is_cursor_line {
                    let cursorline_bg = Color::Rgb(40, 40, 50); // Subtle dark blue background
                    for span in &mut line.spans {
                        // Preserve foreground color but add background
                        span.style = span.style.bg(cursorline_bg);
                    }
                }
                // Apply bracket highlighting
                if let Some(col) = bracket_col {
                    let bracket_style = Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD);
                    apply_style_at_column(&mut line, col, bracket_style);
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
