use crate::editor::Editor;
use crate::syntax::Theme;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::display::char_display_width;

use super::helpers::{expand_tabs_with_mapping, remap_char_col};
use super::layout::{BufferLayout, GUTTER_SPACING, SIGN_WIDTH};
use super::styles::{
    get_diagnostic_sign_style, get_diagnostic_virtual_text_style, get_git_sign_style,
    get_line_number_style, remap_highlights,
};
use crate::syntax::HighlightGroup;
use std::ops::Range;

/// Converts an expanded char index to a display column.
fn expanded_char_to_display_col(text: &str, char_idx: usize) -> usize {
    let mut display_col = 0;
    for (i, ch) in text.chars().enumerate() {
        if i >= char_idx {
            break;
        }
        display_col += char_display_width(ch);
    }
    display_col
}

/// Converts a display column to a char index within a string.
/// If the display column falls in the middle of a wide char, returns that char's index.
fn display_col_to_char_idx(text: &str, target_display_col: usize) -> usize {
    let mut display_col = 0;
    for (char_idx, ch) in text.chars().enumerate() {
        if display_col >= target_display_col {
            return char_idx;
        }
        display_col += char_display_width(ch);
        if display_col > target_display_col {
            return char_idx;
        }
    }
    text.chars().count()
}

/// Converts a UTF-16 offset to a char index within a line of text.
/// LSP uses UTF-16 offsets for character positions.
fn utf16_offset_to_char_idx(text: &str, utf16_offset: usize) -> usize {
    let mut utf16_count = 0;
    for (char_idx, ch) in text.chars().enumerate() {
        if utf16_count >= utf16_offset {
            return char_idx;
        }
        utf16_count += ch.len_utf16();
    }
    text.chars().count()
}

/// Slices a line for horizontal viewport with visual indicators.
/// h_offset and width are in display columns.
/// Returns (sliced_text, precedes_indicator, extends_indicator)
fn slice_horizontal_viewport(line: &str, h_offset: usize, width: usize) -> (String, bool, bool) {
    // Safety check: if width is 0 or too small, return empty or minimal content
    if width == 0 {
        return (String::new(), false, false);
    }

    // Calculate total display width of the line
    let total_display_width: usize = line.chars().map(char_display_width).sum();

    // Line fits entirely in viewport
    if total_display_width <= width {
        return (line.to_string(), false, false);
    }

    let precedes = h_offset > 0;
    let extends = h_offset + width < total_display_width;

    // Available width for actual content (reserve space for indicators)
    let indicator_cols = (if precedes { 1 } else { 0 }) + (if extends { 1 } else { 0 });
    let content_width = width.saturating_sub(indicator_cols);

    let mut result = String::new();

    // Add precedes indicator (<) if scrolled right
    if precedes {
        result.push('<');
    }

    // Walk chars to find the start position (skip h_offset display columns)
    let mut display_col = 0;
    let mut chars = line.chars().peekable();

    // Skip characters until we reach h_offset
    while let Some(&ch) = chars.peek() {
        let ch_width = char_display_width(ch);
        if display_col + ch_width > h_offset {
            break;
        }
        display_col += ch_width;
        chars.next();
    }

    // Collect characters that fit within content_width display columns
    let mut content_display_width = 0;
    while let Some(&ch) = chars.peek() {
        let ch_width = char_display_width(ch);
        if content_display_width + ch_width > content_width {
            break;
        }
        result.push(ch);
        content_display_width += ch_width;
        chars.next();
    }

    // Pad if a wide char didn't fit exactly
    while content_display_width < content_width {
        result.push(' ');
        content_display_width += 1;
    }

    // Add extends indicator (>) if content continues right
    if extends {
        result.push('>');
    }

    (result, precedes, extends)
}

/// Shifts syntax highlight ranges for horizontal viewport.
/// Highlights are in expanded byte ranges; h_offset and width are in display columns.
/// Returns byte ranges into the sliced text.
fn shift_highlights_for_viewport(
    highlights: &[(Range<usize>, HighlightGroup)],
    expanded_text: &str,
    sliced_text: &str,
    h_offset: usize,
    width: usize,
    precedes: bool,
) -> Vec<(Range<usize>, HighlightGroup)> {
    let offset_adjustment = if precedes { 1 } else { 0 }; // Account for '<' indicator

    // Build a byte-offset-to-display-column mapping for the expanded text
    let byte_to_display: Vec<usize> = {
        let mut mapping = Vec::with_capacity(expanded_text.len() + 1);
        let mut display_col = 0;
        for (byte_idx, ch) in expanded_text.char_indices() {
            while mapping.len() <= byte_idx {
                mapping.push(display_col);
            }
            display_col += char_display_width(ch);
        }
        while mapping.len() <= expanded_text.len() {
            mapping.push(display_col);
        }
        mapping
    };

    // Build display-column-to-byte-offset mapping for the sliced text
    let sliced_display_to_byte: Vec<usize> = {
        let mut mapping = Vec::new();
        let mut display_col = 0;
        for (byte_idx, ch) in sliced_text.char_indices() {
            let ch_width = char_display_width(ch);
            for _ in 0..ch_width {
                mapping.push(byte_idx);
            }
            display_col += ch_width;
        }
        mapping.push(sliced_text.len()); // sentinel
        mapping
    };

    let viewport_end = h_offset + width;

    highlights
        .iter()
        .filter_map(|(range, group)| {
            let start_display = if range.start < byte_to_display.len() {
                byte_to_display[range.start]
            } else {
                *byte_to_display.last().unwrap_or(&0)
            };
            let end_display = if range.end < byte_to_display.len() {
                byte_to_display[range.end]
            } else {
                *byte_to_display.last().unwrap_or(&0)
            };

            // Highlight is completely before viewport
            if end_display <= h_offset {
                return None;
            }
            // Highlight is completely after viewport
            if start_display >= viewport_end {
                return None;
            }

            // Clip to viewport display columns
            let clipped_start = start_display.saturating_sub(h_offset) + offset_adjustment;
            let clipped_end = end_display.saturating_sub(h_offset).min(width) + offset_adjustment;

            // Convert viewport display columns to byte offsets in sliced text
            let byte_start = if clipped_start < sliced_display_to_byte.len() {
                sliced_display_to_byte[clipped_start]
            } else {
                sliced_text.len()
            };
            let byte_end = if clipped_end < sliced_display_to_byte.len() {
                sliced_display_to_byte[clipped_end]
            } else {
                sliced_text.len()
            };

            if byte_start < byte_end {
                Some((byte_start..byte_end, *group))
            } else {
                None
            }
        })
        .collect()
}

/// Apply a style to a specific column in a line, splitting spans as needed.
fn apply_style_at_column(line: &mut Line<'static>, target_col: usize, style: Style) {
    let mut current_col = 0;
    for i in 0..line.spans.len() {
        let span_len = line.spans[i].content.chars().count();
        if target_col >= current_col && target_col < current_col + span_len {
            let offset = target_col - current_col;
            if offset == 0 && span_len == 1 {
                // Span is exactly the target character
                line.spans[i].style = line.spans[i].style.patch(style);
            } else {
                // Split the span into up to 3 parts: before, target char, after
                let chars: Vec<char> = line.spans[i].content.chars().collect();
                let base_style = line.spans[i].style;
                let mut new_spans = Vec::with_capacity(3);

                if offset > 0 {
                    let before: String = chars[..offset].iter().collect();
                    new_spans.push(Span::styled(before, base_style));
                }

                let target: String = chars[offset..=offset].iter().collect();
                new_spans.push(Span::styled(target, base_style.patch(style)));

                if offset + 1 < chars.len() {
                    let after: String = chars[offset + 1..].iter().collect();
                    new_spans.push(Span::styled(after, base_style));
                }

                // Replace the original span with the split parts
                line.spans.splice(i..=i, new_spans);
            }
            return;
        }
        current_col += span_len;
    }
}

/// Apply a background color to a column range within a Line.
/// Splits spans as needed to cover exactly the given range.
fn apply_bg_to_column_range(line: &mut Line<'static>, start_col: usize, end_col: usize, bg: Color) {
    let mut new_spans: Vec<Span<'static>> = Vec::new();
    let mut current_col = 0;

    for span in line.spans.drain(..) {
        let span_len = span.content.chars().count();
        let span_end = current_col + span_len;

        if span_end <= start_col || current_col > end_col {
            // Entirely outside the flash range
            new_spans.push(span);
        } else if current_col >= start_col && span_end <= end_col + 1 {
            // Entirely inside the flash range
            new_spans.push(Span::styled(span.content, span.style.bg(bg)));
        } else {
            // Partially overlapping - split the span
            let chars: Vec<char> = span.content.chars().collect();
            let flash_start = start_col.saturating_sub(current_col);
            let flash_end = (end_col + 1).saturating_sub(current_col).min(chars.len());

            if flash_start > 0 {
                let before: String = chars[..flash_start].iter().collect();
                new_spans.push(Span::styled(before, span.style));
            }
            if flash_start < flash_end {
                let middle: String = chars[flash_start..flash_end].iter().collect();
                new_spans.push(Span::styled(middle, span.style.bg(bg)));
            }
            if flash_end < chars.len() {
                let after: String = chars[flash_end..].iter().collect();
                new_spans.push(Span::styled(after, span.style));
            }
        }

        current_col = span_end;
    }

    line.spans = new_spans;
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

/// Builds a gutter line for a logical line (line number + git sign / diagnostic sign).
/// If `is_continuation` is true, produces a blank gutter row.
/// Diagnostic signs take priority over git signs when both are present.
fn build_gutter_line(
    editor: &Editor,
    buffer: &crate::buffer::Buffer,
    line_idx: usize,
    line_num_width: usize,
    cursor_line: usize,
    is_continuation: bool,
    line_diagnostics: &[&lsp_types::Diagnostic],
) -> Line<'static> {
    if is_continuation {
        // Blank gutter for wrap continuation rows
        let width = SIGN_WIDTH + line_num_width + GUTTER_SPACING;
        return Line::from(" ".repeat(width));
    }

    let line_num_text = if editor.options.relative_number {
        let rel = if line_idx == cursor_line {
            line_idx + 1
        } else {
            line_idx.abs_diff(cursor_line)
        };
        format!("{:>width$} ", rel, width = line_num_width)
    } else if editor.options.number {
        format!("{:>width$} ", line_idx + 1, width = line_num_width)
    } else {
        "  ".to_string()
    };

    // Diagnostic signs take priority over git signs
    let (sign_text, sign_color) = if !line_diagnostics.is_empty() {
        let severity = line_diagnostics[0].severity;
        get_diagnostic_sign_style(severity)
    } else {
        let git_status = buffer.git_status().get_line_status(line_idx);
        get_git_sign_style(git_status)
    };
    let line_num_style = get_line_number_style(line_idx == cursor_line);

    let sign_span = Span::styled(
        sign_text,
        Style::default().fg(sign_color).add_modifier(Modifier::BOLD),
    );
    let line_num_span = Span::styled(line_num_text, line_num_style);

    Line::from(vec![sign_span, line_num_span])
}

/// Splits a rendered Line into multiple visual rows for soft wrapping.
/// Each row fits within `width` display columns. Rows are padded to full width.
/// Wide characters (CJK, emoji) that don't fit at a row boundary are pushed to
/// the next row, with the remaining space padded (matching Neovim behavior).
fn split_line_into_rows(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    // Calculate total display width
    let total_width: usize = line
        .spans
        .iter()
        .map(|s| {
            s.content
                .chars()
                .map(char_display_width)
                .sum::<usize>()
        })
        .sum();

    if total_width <= width {
        // Line fits in one row - just pad it
        let mut row = line;
        let pad = width.saturating_sub(total_width);
        if pad > 0 {
            row.spans.push(Span::raw(" ".repeat(pad)));
        }
        return vec![row];
    }

    // Need to split spans across multiple rows
    let mut rows = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut current_width = 0;

    for span in line.spans {
        let style = span.style;
        let mut chunk = String::new();

        for ch in span.content.chars() {
            let ch_width = char_display_width(ch);

            if current_width + ch_width > width {
                // Flush accumulated chunk for this span
                if !chunk.is_empty() {
                    current_spans.push(Span::styled(chunk.clone(), style));
                    chunk.clear();
                }
                // Pad remaining space in current row
                let pad = width.saturating_sub(current_width);
                if pad > 0 {
                    current_spans.push(Span::raw(" ".repeat(pad)));
                }
                rows.push(Line::from(current_spans));
                current_spans = Vec::new();
                current_width = 0;
            }

            chunk.push(ch);
            current_width += ch_width;

            if current_width >= width {
                // Row exactly full, flush
                current_spans.push(Span::styled(chunk.clone(), style));
                chunk.clear();
                rows.push(Line::from(current_spans));
                current_spans = Vec::new();
                current_width = 0;
            }
        }

        if !chunk.is_empty() {
            current_spans.push(Span::styled(chunk, style));
        }
    }

    // Push remaining content as final row
    if !current_spans.is_empty() || rows.is_empty() {
        let pad = width.saturating_sub(current_width);
        if pad > 0 {
            current_spans.push(Span::raw(" ".repeat(pad)));
        }
        rows.push(Line::from(current_spans));
    }

    rows
}

/// Renders the buffer content and returns the viewport start line
pub fn render_buffer(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    layout: &BufferLayout,
) -> usize {
    let area = layout.buffer_area;
    let buffer = editor.buffer();
    let rope = buffer.rope();
    let cursor = buffer.cursor();
    // Use rope's raw line count so we render the trailing empty line after
    // a final newline — the cursor can legitimately be there (e.g. after
    // pressing Enter at EOF in insert mode).
    let line_count = rope.len_lines();

    // Calculate visible range using scroll offset (not centering)
    let visible_lines = area.height as usize;
    let start_line = editor.scroll_offset();

    // Get horizontal viewport settings
    let h_offset = editor.horizontal_offset();
    let wrap = editor.options.wrap;

    // Use layout-provided dimensions
    let line_num_width = layout.line_num_width;
    let gutter_width_u16 = layout.gutter_width as u16;

    // Split area into gutter and text
    let (gutter_area, text_area) = if layout.gutter_width > 0 {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(gutter_width_u16), Constraint::Min(1)])
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
    let current_search = editor.search.current_search.as_ref();

    // Find matching bracket position if showmatch is enabled
    let bracket_positions: Option<((usize, usize), (usize, usize))> = if editor.options.showmatch {
        find_matching_bracket_position(buffer)
            .map(|matching_pos| ((cursor.line(), cursor.col()), matching_pos))
    } else {
        None
    };

    // Build the visible text with syntax highlighting
    // Gutter lines are built inline to match wrap continuation rows
    let mut lines = Vec::new();
    let mut gutter_lines: Vec<Line<'static>> = Vec::new();
    let blank_line = " ".repeat(text_area.width as usize);
    let tab_width = editor.options.tab_width;
    let cursorline = editor.options.cursorline;
    let cursor_line_idx = cursor.line();
    let text_width = text_area.width as usize;
    let wrap_map = editor.wrap_map();
    let has_wrap = wrap && wrap_map.is_some();
    let mut visual_rows_used = 0;

    let mut line_idx = start_line;
    while line_idx < line_count && visual_rows_used < visible_lines {
        if line_idx < rope.len_lines() {
            let line_text_raw = rope.line(line_idx).to_string();
            // Remove trailing newline if present
            let line_text_original = line_text_raw.trim_end_matches('\n');

            // Expand tabs to spaces for proper rendering and get byte mapping
            let (line_text, byte_mapping, control_ranges, char_mapping) = expand_tabs_with_mapping(line_text_original, tab_width);

            // Keep a reference to expanded text before slicing (for highlight remapping)
            let expanded_text = line_text.clone();

            // Apply horizontal viewport slicing if nowrap is set
            let (line_text, precedes, _extends) = if !wrap {
                slice_horizontal_viewport(&line_text, h_offset, text_width)
            } else {
                (line_text.to_string(), false, false)
            };

            // Get syntax highlights for this line and remap them for expanded text
            let original_highlights = buffer.highlights_for_line(line_idx);
            let mut syntax_highlights = remap_highlights(&original_highlights, &byte_mapping);

            // Shift syntax highlights for horizontal viewport if nowrap
            if !wrap {
                syntax_highlights = shift_highlights_for_viewport(
                    &syntax_highlights,
                    &expanded_text,
                    &line_text,
                    h_offset,
                    text_width,
                    precedes,
                );
            }

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

            // Remap visual selection columns from original to expanded char indices
            let remapped_visual_selection = visual_selection.map(|((sl, sc), (el, ec))| {
                let sc = if line_idx == sl { remap_char_col(sc, &char_mapping) } else { sc };
                let ec = if line_idx == el { remap_char_col(ec, &char_mapping) } else { ec };
                ((sl, sc), (el, ec))
            });

            // Adjust visual selection for horizontal viewport if nowrap
            let remapped_visual_selection = if !wrap {
                remapped_visual_selection.map(|((sl, sc), (el, ec))| {
                    let adjust = |expanded_char_col: usize| -> usize {
                        let display_col = expanded_char_to_display_col(&expanded_text, expanded_char_col);
                        let viewport_display_col = display_col.saturating_sub(h_offset);
                        let offset_adjustment = if precedes { 1 } else { 0 };
                        display_col_to_char_idx(&line_text, viewport_display_col + offset_adjustment)
                    };
                    let sc = if line_idx == sl { adjust(sc) } else { sc };
                    let ec = if line_idx == el { adjust(ec) } else { ec };
                    ((sl, sc), (el, ec))
                })
            } else {
                remapped_visual_selection
            };

            // Check if this line has a bracket to highlight (remap through char_mapping)
            let bracket_col = bracket_positions.and_then(|((l1, c1), (l2, c2))| {
                if line_idx == l1 {
                    Some(remap_char_col(c1, &char_mapping))
                } else if line_idx == l2 {
                    Some(remap_char_col(c2, &char_mapping))
                } else {
                    None
                }
            });

            // Adjust bracket column for horizontal viewport if nowrap
            let bracket_col = if !wrap {
                bracket_col.and_then(|expanded_char_col| {
                    // Convert expanded char index to display column
                    let display_col = expanded_char_to_display_col(&expanded_text, expanded_char_col);
                    // Check if bracket is in visible horizontal range
                    if display_col >= h_offset && display_col < h_offset + text_width {
                        // Convert to char index in the sliced text
                        let viewport_display_col = display_col - h_offset;
                        let offset_adjustment = if precedes { 1 } else { 0 };
                        let sliced_char_idx = display_col_to_char_idx(&line_text, viewport_display_col + offset_adjustment);
                        Some(sliced_char_idx)
                    } else {
                        None // Bracket is outside viewport
                    }
                })
            } else {
                bracket_col
            };

            // Get diagnostics for this line and remap their ranges to expanded char indices
            let line_diagnostics = editor.diagnostics_for_line(line_idx);
            let has_diagnostics = !line_diagnostics.is_empty();
            let remapped_diagnostics: Vec<RemappedDiagnostic> = line_diagnostics.iter().filter_map(|d| {
                // Convert UTF-16 offsets to char indices, then remap through expansion
                let start_char = utf16_offset_to_char_idx(line_text_original, d.range.start.character as usize);
                let end_char = utf16_offset_to_char_idx(line_text_original, d.range.end.character as usize);
                let color = match d.severity {
                    Some(lsp_types::DiagnosticSeverity::ERROR) => Color::Red,
                    Some(lsp_types::DiagnosticSeverity::WARNING) => Color::Yellow,
                    Some(lsp_types::DiagnosticSeverity::INFORMATION) => Color::Cyan,
                    Some(lsp_types::DiagnosticSeverity::HINT) => Color::Gray,
                    _ => Color::Red,
                };
                let expanded_start = remap_char_col(start_char, &char_mapping);
                let expanded_end = remap_char_col(end_char, &char_mapping);

                // Adjust for horizontal viewport in nowrap mode
                if !wrap {
                    let start_display = expanded_char_to_display_col(&expanded_text, expanded_start);
                    let end_display = expanded_char_to_display_col(&expanded_text, expanded_end);
                    // Skip if entirely outside viewport
                    if end_display <= h_offset || start_display >= h_offset + text_width {
                        return None;
                    }
                    let offset_adj = if precedes { 1 } else { 0 };
                    let sliced_start = display_col_to_char_idx(&line_text, start_display.saturating_sub(h_offset) + offset_adj);
                    let sliced_end = display_col_to_char_idx(&line_text, end_display.saturating_sub(h_offset) + offset_adj);
                    Some(RemappedDiagnostic { start: sliced_start, end: sliced_end, color })
                } else {
                    Some(RemappedDiagnostic { start: expanded_start, end: expanded_end, color })
                }
            }).collect();

            // Check if this line is in a yank flash region
            let yank_flash = editor.yank_flash.as_ref().and_then(|flash| {
                if flash.contains_line(line_idx) {
                    Some(flash.col_range_for_line(line_idx))
                } else {
                    None
                }
            });

            // Always use character-by-character rendering if we have any highlighting
            let needs_detailed_rendering = has_visual_selection
                || !search_matches.is_empty()
                || !syntax_highlights.is_empty()
                || is_cursor_line
                || bracket_col.is_some()
                || has_diagnostics
                || yank_flash.is_some();

            if needs_detailed_rendering {
                let mut line = render_line_with_highlights(
                    theme,
                    &line_text,
                    line_idx,
                    remapped_visual_selection,
                    editor.mode(),
                    &search_matches,
                    &syntax_highlights,
                    &remapped_diagnostics,
                    &control_ranges,
                );

                // Add diagnostic virtual text if present (only on first visual row in wrap mode)
                if has_diagnostics {
                    // Get the first (most severe) diagnostic
                    let diag = line_diagnostics[0];
                    let (icon, fg_color, bg_color) =
                        get_diagnostic_virtual_text_style(diag.severity);
                    // Truncate message to fit on screen
                    let max_msg_len = text_width.saturating_sub(line_text.chars().count() + 4);
                    let msg = diag.message.lines().next().unwrap_or("");
                    let msg = if msg.chars().count() > max_msg_len {
                        format!(
                            "{}...",
                            msg.chars()
                                .take(max_msg_len.saturating_sub(3))
                                .collect::<String>()
                        )
                    } else {
                        msg.to_string()
                    };
                    let vtext_style = Style::default()
                        .fg(fg_color)
                        .bg(bg_color)
                        .add_modifier(Modifier::ITALIC);
                    // Plain gap between code and diagnostic (no background)
                    line.spans.push(Span::raw("  "));
                    line.spans
                        .push(Span::styled(format!("{} {}", icon, msg), vtext_style));
                }

                // Apply cursorline background if this is the cursor line
                if is_cursor_line && yank_flash.is_none() {
                    let cursorline_bg = Color::Rgb(40, 40, 50); // Subtle dark blue background
                    for span in &mut line.spans {
                        span.style = span.style.bg(cursorline_bg);
                    }
                }

                // Apply yank flash highlight
                if let Some(col_range) = yank_flash {
                    let flash_bg = Color::Rgb(60, 50, 20); // Warm amber glow
                    match col_range {
                        None => {
                            // Linewise flash: highlight entire line
                            for span in &mut line.spans {
                                span.style = span.style.bg(flash_bg);
                            }
                        }
                        Some((start_col, end_col)) => {
                            // Character-wise flash: highlight column range
                            apply_bg_to_column_range(&mut line, start_col, end_col, flash_bg);
                        }
                    }
                }

                // Apply bracket highlighting
                if let Some(col) = bracket_col {
                    let bracket_style = Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD);
                    apply_style_at_column(&mut line, col, bracket_style);
                }

                // Soft wrap: split into visual rows if needed
                if has_wrap {
                    let visual_rows = split_line_into_rows(line, text_width);
                    for (row_idx, row) in visual_rows.into_iter().enumerate() {
                        if visual_rows_used >= visible_lines {
                            break;
                        }
                        if gutter_area.is_some() {
                            gutter_lines.push(build_gutter_line(
                                editor,
                                buffer,
                                line_idx,
                                line_num_width,
                                cursor_line_idx,
                                row_idx > 0,
                                &line_diagnostics,
                            ));
                        }
                        lines.push(row);
                        visual_rows_used += 1;
                    }
                } else {
                    // No wrap: pad and push single line
                    let line_len: usize =
                        line.spans.iter().map(|s| s.content.chars().count()).sum();
                    if line_len < text_width {
                        line.spans
                            .push(Span::raw(" ".repeat(text_width - line_len)));
                    }
                    if gutter_area.is_some() {
                        gutter_lines.push(build_gutter_line(
                            editor,
                            buffer,
                            line_idx,
                            line_num_width,
                            cursor_line_idx,
                            false,
                            &line_diagnostics,
                        ));
                    }
                    lines.push(line);
                    visual_rows_used += 1;
                }
            } else {
                // Simple rendering path (no highlighting)
                if has_wrap {
                    let chars: Vec<char> = line_text.chars().collect();
                    if chars.is_empty() {
                        if gutter_area.is_some() {
                            gutter_lines.push(build_gutter_line(
                                editor,
                                buffer,
                                line_idx,
                                line_num_width,
                                cursor_line_idx,
                                false,
                                &[],
                            ));
                        }
                        lines.push(Line::from(" ".repeat(text_width)));
                        visual_rows_used += 1;
                    } else {
                        for (chunk_idx, chunk) in chars.chunks(text_width).enumerate() {
                            if visual_rows_used >= visible_lines {
                                break;
                            }
                            if gutter_area.is_some() {
                                gutter_lines.push(build_gutter_line(
                                    editor,
                                    buffer,
                                    line_idx,
                                    line_num_width,
                                    cursor_line_idx,
                                    chunk_idx > 0,
                                    &[],
                                ));
                            }
                            let text: String = chunk.iter().collect();
                            let pad = text_width.saturating_sub(chunk.len());
                            let padded = if pad > 0 {
                                format!("{}{}", text, " ".repeat(pad))
                            } else {
                                text
                            };
                            lines.push(Line::from(padded));
                            visual_rows_used += 1;
                        }
                    }
                } else {
                    // No wrap: pad simple lines too
                    if gutter_area.is_some() {
                        gutter_lines.push(build_gutter_line(
                            editor,
                            buffer,
                            line_idx,
                            line_num_width,
                            cursor_line_idx,
                            false,
                            &[],
                        ));
                    }
                    let line_len = line_text.chars().count();
                    let line_text = if line_len < text_width {
                        format!("{}{}", line_text, " ".repeat(text_width - line_len))
                    } else {
                        line_text.to_string()
                    };
                    lines.push(Line::from(line_text));
                    visual_rows_used += 1;
                }
            }
        } else {
            // Line beyond end of file - clear it
            lines.push(Line::from(blank_line.clone()));
            visual_rows_used += 1;
        }
        line_idx += 1;
    }

    // Fill remaining rows with blanks
    while visual_rows_used < visible_lines {
        lines.push(Line::from(blank_line.clone()));
        visual_rows_used += 1;
    }

    // Render gutter
    if let Some(gutter_area) = gutter_area {
        let gutter_paragraph = Paragraph::new(gutter_lines);
        frame.render_widget(gutter_paragraph, gutter_area);
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .style(Style::default().bg(Color::Reset));
    frame.render_widget(paragraph, text_area);

    start_line
}

/// A diagnostic range remapped to expanded char indices for rendering
pub struct RemappedDiagnostic {
    pub start: usize,
    pub end: usize,
    pub color: Color,
}

/// Renders a single line with all highlighting (syntax, visual selection, search, diagnostics, control chars)
#[allow(clippy::too_many_arguments)]
pub fn render_line_with_highlights(
    theme: &Theme,
    line_text: &str,
    line_idx: usize,
    visual_selection: Option<((usize, usize), (usize, usize))>,
    mode: crate::mode::Mode,
    search_matches: &[(usize, usize)],
    syntax_highlights: &[(std::ops::Range<usize>, crate::syntax::HighlightGroup)],
    diagnostics: &[RemappedDiagnostic],
    control_ranges: &[std::ops::Range<usize>],
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
            .filter(|(range, _)| range.contains(&byte_idx))
            .min_by_key(|(range, _)| range.end - range.start)
            .map(|(_, group)| *group);

        // Check if this character falls within a diagnostic range (underline)
        let diag_underline_color = diagnostics.iter().find_map(|d| {
            if col_idx >= d.start && col_idx < d.end {
                Some(d.color)
            } else {
                None
            }
        });

        // Check if this character is in a control char range
        let is_control = control_ranges.iter().any(|r| r.contains(&byte_idx));

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
                .filter(|(range, _)| range.contains(&next_byte_idx))
                .min_by_key(|(range, _)| range.end - range.start)
                .map(|(_, group)| *group);

            // Check diagnostic underline for next character
            let next_diag_underline_color = diagnostics.iter().find_map(|d| {
                if end_col >= d.start && end_col < d.end {
                    Some(d.color)
                } else {
                    None
                }
            });

            let next_is_control = control_ranges.iter().any(|r| r.contains(&next_byte_idx));

            // If styling changes, break
            if next_selected != is_selected
                || next_search_match != is_search_match
                || next_syntax_group != syntax_group
                || next_diag_underline_color != diag_underline_color
                || next_is_control != is_control
            {
                break;
            }

            end_col += 1;
        }

        // Build the span for this range
        let text: String = chars[col_idx..end_col].iter().collect();

        // Apply styling based on priority: visual selection > search match > control char > syntax > normal
        let mut style = if is_selected {
            Style::default().bg(Color::Blue).fg(Color::White)
        } else if is_search_match {
            Style::default().bg(Color::Yellow).fg(Color::Black)
        } else if is_control {
            let color = crate::key_convert::convert_core_color(theme.get_color(HighlightGroup::SpecialKey));
            Style::default().fg(color)
        } else if let Some(group) = syntax_group {
            let color = crate::key_convert::convert_core_color(theme.get_color(group));
            let mut style = Style::default().fg(color);

            // Add modifiers for markup elements
            match group {
                HighlightGroup::MarkupHeading => {
                    style = style.add_modifier(Modifier::BOLD);
                }
                HighlightGroup::MarkupBold => {
                    style = style.add_modifier(Modifier::BOLD);
                }
                HighlightGroup::MarkupItalic => {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                _ => {}
            }
            style
        } else {
            Style::default()
        };

        // Apply diagnostic underline (additive — works on top of any style)
        if let Some(underline_color) = diag_underline_color {
            style = style.fg(underline_color).add_modifier(Modifier::UNDERLINED);
        }

        spans.push(Span::styled(text, style));
        col_idx = end_col;
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_line_wide_char_at_boundary() {
        // Width=4, content "abc世d"
        // 'a'=1, 'b'=1, 'c'=1, '世'=2 -> doesn't fit (3+2=5 > 4), pad row 1
        // Row 1: "abc " (padded), Row 2: "世d  " (padded)
        let line = Line::from(vec![Span::raw("abc世d")]);
        let rows = split_line_into_rows(line, 4);
        assert_eq!(rows.len(), 2);

        let row0_text: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let row1_text: String = rows[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(row0_text, "abc ");
        assert_eq!(row1_text, "世d "); // 世=2 + d=1 = 3, pad 1 to fill width 4
    }

    #[test]
    fn test_split_line_ascii_no_wide() {
        let line = Line::from(vec![Span::raw("abcdefgh")]);
        let rows = split_line_into_rows(line, 4);
        assert_eq!(rows.len(), 2);

        let row0_text: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let row1_text: String = rows[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(row0_text, "abcd");
        assert_eq!(row1_text, "efgh");
    }

    #[test]
    fn test_split_line_fits_in_one_row() {
        let line = Line::from(vec![Span::raw("ab")]);
        let rows = split_line_into_rows(line, 4);
        assert_eq!(rows.len(), 1);

        let text: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "ab  "); // padded
    }
}
