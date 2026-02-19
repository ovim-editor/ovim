use crate::editor::{AiRegionStatus, Editor};
use crate::syntax::{Theme, UiGroup};
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
    blame_color_for_hash, get_diagnostic_sign_style, get_diagnostic_virtual_text_style,
    get_git_sign_style, get_line_number_style, remap_highlights,
};
use crate::syntax::HighlightGroup;
use ovim_core::buffer::Cursor;
use std::ops::Range;

/// Window-specific rendering context for multi-window support.
/// When provided, these values override the editor's focused window state.
#[derive(Default)]
pub struct WindowRenderContext {
    /// Override cursor position (for non-focused windows)
    pub cursor: Option<Cursor>,
    /// Override scroll offset (for non-focused windows)
    pub scroll_offset: Option<usize>,
    /// Override horizontal scroll offset (for non-focused windows)
    pub horizontal_offset: Option<usize>,
}

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
        for (byte_idx, ch) in sliced_text.char_indices() {
            let ch_width = char_display_width(ch);
            for _ in 0..ch_width {
                mapping.push(byte_idx);
            }
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

/// Bracket character for blame grouping
#[derive(Debug, Clone, Copy, PartialEq)]
enum BlameBracket {
    /// Single-line commit (no bracket)
    None,
    /// First line of a multi-line group
    Top,
    /// Middle line of a multi-line group
    Mid,
    /// Last line of a multi-line group
    Bottom,
}

/// Pre-computes blame bracket characters for visible lines.
/// Returns a vec of (bracket, hash, author, color) for each line in the range.
fn compute_blame_brackets(
    blame: &crate::GitBlame,
    start_line: usize,
    end_line: usize,
    author_width: usize,
) -> Vec<(BlameBracket, String, String, Color)> {
    let mut result = Vec::with_capacity(end_line.saturating_sub(start_line));

    for line_idx in start_line..end_line {
        if let Some(info) = blame.get(line_idx) {
            let hash = &info.commit_hash;
            let color = blame_color_for_hash(hash);

            // Check if prev/next lines have the same commit
            let same_as_prev = line_idx > 0
                && blame
                    .get(line_idx - 1)
                    .map(|p| p.commit_hash == *hash)
                    .unwrap_or(false);
            let same_as_next = blame
                .get(line_idx + 1)
                .map(|n| n.commit_hash == *hash)
                .unwrap_or(false);

            let bracket = match (same_as_prev, same_as_next) {
                (false, false) => BlameBracket::None,
                (false, true) => BlameBracket::Top,
                (true, true) => BlameBracket::Mid,
                (true, false) => BlameBracket::Bottom,
            };

            // Truncate author to fit
            let author: String = info.author.chars().take(author_width).collect();

            result.push((bracket, hash.clone(), author, color));
        } else {
            result.push((
                BlameBracket::None,
                String::new(),
                String::new(),
                Color::DarkGray,
            ));
        }
    }

    result
}

/// Builds a gutter line for a logical line (line number + git sign / diagnostic sign).
/// If `is_continuation` is true, produces a blank gutter row.
/// Diagnostic signs take priority over git signs when both are present.
fn build_gutter_line(
    editor: &Editor,
    buffer: &crate::buffer::Buffer,
    theme: &Theme,
    line_idx: usize,
    line_num_width: usize,
    cursor_line: usize,
    is_continuation: bool,
    line_diagnostics: &[&lsp_types::Diagnostic],
    blame_info: Option<&(BlameBracket, String, String, Color)>,
    blame_width: usize,
) -> Line<'static> {
    if is_continuation {
        // Blank gutter for wrap continuation rows
        let width = blame_width + SIGN_WIDTH + line_num_width + GUTTER_SPACING;
        return Line::from(" ".repeat(width));
    }

    let mut spans = Vec::new();

    // Blame column (if active)
    if blame_width > 0 {
        if let Some((bracket, hash, author, color)) = blame_info {
            let bracket_ch = match bracket {
                BlameBracket::None => ' ',
                BlameBracket::Top => '╭',
                BlameBracket::Mid => '│',
                BlameBracket::Bottom => '╰',
            };

            // Show hash+author only on first line of group or single lines
            let show_info = *bracket == BlameBracket::None || *bracket == BlameBracket::Top;
            let content_width = blame_width - 2; // minus bracket + leading space

            let text = if show_info && !hash.is_empty() {
                let info_str = format!("{} {}", hash, author);
                format!(
                    "{} {:content_width$}",
                    bracket_ch,
                    info_str,
                    content_width = content_width
                )
            } else {
                format!(
                    "{} {:content_width$}",
                    bracket_ch,
                    "",
                    content_width = content_width
                )
            };

            // Truncate to blame_width
            let text: String = text.chars().take(blame_width).collect();
            spans.push(Span::styled(text, Style::default().fg(*color)));
        } else {
            spans.push(Span::raw(" ".repeat(blame_width)));
        }
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

    // Diagnostic signs take priority over agent edits > git signs
    let buffer_id = buffer.id();
    let is_agent_edit = editor
        .ai_chat_state()
        .map(|c| c.agent_edits.is_line_modified(buffer_id, line_idx))
        .unwrap_or(false);

    let (sign_text, sign_color) = if !line_diagnostics.is_empty() {
        let severity = line_diagnostics[0].severity;
        get_diagnostic_sign_style(severity)
    } else if is_agent_edit {
        ("▎ ", Color::Rgb(82, 139, 255))
    } else {
        let git_status = buffer.git_status().get_line_status(line_idx);
        get_git_sign_style(git_status)
    };
    let line_num_style = get_line_number_style(line_idx == cursor_line, theme);

    let sign_span = Span::styled(
        sign_text,
        Style::default().fg(sign_color).add_modifier(Modifier::BOLD),
    );
    let line_num_span = Span::styled(line_num_text, line_num_style);

    spans.push(sign_span);
    spans.push(line_num_span);

    Line::from(spans)
}

/// Appends diagnostic virtual text to a rendered row, if there's enough room.
/// The diagnostic is appended inline after the code text, truncated to fit within `text_width`.
/// Requires at least 6 columns of remaining space (for "  ⚠ X…").
fn append_diagnostic_virtual_text(
    row: &mut Line<'static>,
    diagnostics: &[&lsp_types::Diagnostic],
    text_width: usize,
) {
    if diagnostics.is_empty() {
        return;
    }

    // Remove trailing padding spans (spaces) so we can correctly measure and append.
    // Note: `split_line_into_rows` pads each wrapped row to `text_width`, so if we measure
    // before stripping, the row always appears "full" and virtual text never renders.
    while let Some(last) = row.spans.last() {
        if last.content.chars().all(|c| c == ' ') && last.style == Style::default() {
            row.spans.pop();
        } else {
            break;
        }
    }

    // Measure the row's current display width (after padding removal)
    let row_width: usize = row
        .spans
        .iter()
        .map(|s| s.content.chars().map(char_display_width).sum::<usize>())
        .sum();

    // Need at least 6 cols: "  " + icon + " " + 1 char message
    let remaining = text_width.saturating_sub(row_width);
    if remaining < 6 {
        // Re-pad to avoid shrinking the row if we removed padding spans above.
        if row_width < text_width {
            row.spans
                .push(Span::raw(" ".repeat(text_width - row_width)));
        }
        return;
    }

    let diag = diagnostics[0];
    let (icon, fg_color, bg_color) = get_diagnostic_virtual_text_style(diag.severity);
    // "  " prefix + icon + " " = 4 chars overhead (icon is 1 display col for nerdfont)
    let max_msg_len = remaining.saturating_sub(4);
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

    row.spans.push(Span::raw("  "));
    row.spans
        .push(Span::styled(format!("{} {}", icon, msg), vtext_style));
    // Re-pad to text_width
    let new_width: usize = row
        .spans
        .iter()
        .map(|s| s.content.chars().map(char_display_width).sum::<usize>())
        .sum();
    if new_width < text_width {
        row.spans
            .push(Span::raw(" ".repeat(text_width - new_width)));
    }
}

fn lock_ranges_for_line(
    buffer: &crate::buffer::Buffer,
    line_idx: usize,
    line_content_chars: usize,
    char_mapping: &[usize],
    expanded_text: &str,
    line_text: &str,
    wrap: bool,
    h_offset: usize,
    text_width: usize,
    precedes: bool,
) -> Vec<(usize, usize)> {
    let line_start_char = buffer.rope().line_to_char(line_idx);
    let line_end_char = line_start_char.saturating_add(line_content_chars);
    let viewport_end = h_offset.saturating_add(text_width);
    let mut ranges = Vec::new();

    for lock in buffer.ai_locks().iter().filter(|lock| lock.blocks_edits) {
        let start_abs = lock.start_char.max(line_start_char);
        let end_abs = lock.end_char.min(line_end_char);
        if end_abs <= start_abs {
            continue;
        }

        let mut start_col = remap_char_col(start_abs - line_start_char, char_mapping);
        let mut end_col_exclusive = remap_char_col(end_abs - line_start_char, char_mapping);

        if !wrap {
            let start_display = expanded_char_to_display_col(expanded_text, start_col);
            let end_display = expanded_char_to_display_col(expanded_text, end_col_exclusive);
            if end_display <= h_offset || start_display >= viewport_end {
                continue;
            }
            let offset_adjustment = if precedes { 1 } else { 0 };
            start_col = display_col_to_char_idx(
                line_text,
                start_display.saturating_sub(h_offset) + offset_adjustment,
            );
            end_col_exclusive = display_col_to_char_idx(
                line_text,
                end_display.saturating_sub(h_offset) + offset_adjustment,
            );
        }

        if end_col_exclusive > start_col {
            ranges.push((start_col, end_col_exclusive - 1));
        }
    }

    ranges
}

fn ai_region_ranges_for_line(
    editor: &Editor,
    line_idx: usize,
    line_content_chars: usize,
    char_mapping: &[usize],
    expanded_text: &str,
    line_text: &str,
    wrap: bool,
    h_offset: usize,
    text_width: usize,
    precedes: bool,
) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
    let line_start_char = editor.buffer().rope().line_to_char(line_idx);
    let line_end_char = line_start_char.saturating_add(line_content_chars);
    let viewport_end = h_offset.saturating_add(text_width);
    let selected_region_id = editor.ai_selected_region_id();
    let mut generated_ranges = Vec::new();
    let mut selected_ranges = Vec::new();

    for region in editor.ai_regions() {
        let show_generated = region.status == AiRegionStatus::Generated;
        let show_selected = selected_region_id == Some(region.id)
            && matches!(
                region.status,
                AiRegionStatus::Generated | AiRegionStatus::Running
            );
        if !show_generated && !show_selected {
            continue;
        }

        let start_abs = region.start_char.max(line_start_char);
        let end_abs = region.end_char.min(line_end_char);
        if end_abs <= start_abs {
            continue;
        }

        let mut start_col = remap_char_col(start_abs - line_start_char, char_mapping);
        let mut end_col_exclusive = remap_char_col(end_abs - line_start_char, char_mapping);

        if !wrap {
            let start_display = expanded_char_to_display_col(expanded_text, start_col);
            let end_display = expanded_char_to_display_col(expanded_text, end_col_exclusive);
            if end_display <= h_offset || start_display >= viewport_end {
                continue;
            }
            let offset_adjustment = if precedes { 1 } else { 0 };
            start_col = display_col_to_char_idx(
                line_text,
                start_display.saturating_sub(h_offset) + offset_adjustment,
            );
            end_col_exclusive = display_col_to_char_idx(
                line_text,
                end_display.saturating_sub(h_offset) + offset_adjustment,
            );
        }

        if end_col_exclusive > start_col {
            let range = (start_col, end_col_exclusive - 1);
            if show_generated {
                generated_ranges.push(range);
            }
            if show_selected {
                selected_ranges.push(range);
            }
        }
    }

    (generated_ranges, selected_ranges)
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
        .map(|s| s.content.chars().map(char_display_width).sum::<usize>())
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
    line_cache: &mut super::line_cache::LineRenderCache,
    render_diagnostic_virtual_text_inline: bool,
    window_context: Option<&WindowRenderContext>,
) -> usize {
    let area = layout.buffer_area;
    let buffer = editor.buffer();
    let rope = buffer.rope();

    // Use window-specific cursor if provided (for non-focused windows)
    let cursor = window_context
        .and_then(|ctx| ctx.cursor.as_ref())
        .unwrap_or_else(|| buffer.cursor());

    // Use Vim-compatible line count: trailing newline's phantom empty line
    // should not be rendered. The cursor is always bounded to real lines.
    let line_count = buffer.line_count();

    // Calculate visible range using scroll offset (not centering)
    // Use window-specific scroll offset if provided
    let visible_lines = area.height as usize;
    let start_line = window_context
        .and_then(|ctx| ctx.scroll_offset)
        .unwrap_or_else(|| editor.scroll_offset());

    // Get horizontal viewport settings
    // Use window-specific horizontal offset if provided
    let h_offset = window_context
        .and_then(|ctx| ctx.horizontal_offset)
        .unwrap_or_else(|| editor.horizontal_offset());
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
    let ai_prompt_selection = if editor.mode() == crate::mode::Mode::AiPrompt {
        editor.ai_state.active_selection.as_ref()
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
    let buffer_version = buffer.version();
    let buffer_id = editor.current_buffer_index();

    // Reset per-frame cache stats
    line_cache.reset_stats();

    // Pre-compute blame brackets for visible lines
    let blame_width = layout.blame_width;
    let blame_brackets = if blame_width > 0 {
        if let Some(blame) = buffer.git_blame() {
            let author_width = blame_width.saturating_sub(1 + 1 + 5 + 1 + 1); // bracket+sp+hash+sp+trailing_sp
            Some(compute_blame_brackets(
                blame,
                start_line,
                line_count.min(start_line + visible_lines + 50),
                author_width,
            ))
        } else {
            None
        }
    } else {
        None
    };

    let mut line_idx = start_line;
    while line_idx < line_count && visual_rows_used < visible_lines {
        if line_idx < rope.len_lines() {
            // --- Cache check: try to reuse a previously rendered stable line ---
            // Determine upfront if this line has transient overlays that prevent caching.
            let has_visual_on_line = visual_selection
                .map(|((sl, _), (el, _))| line_idx >= sl && line_idx <= el)
                .unwrap_or(false);
            let is_cursor_line_early = cursorline && line_idx == cursor_line_idx;
            let has_yank_flash = editor
                .yank_flash
                .as_ref()
                .map_or(false, |f| f.contains_line(line_idx));
            let line_diagnostics_early = editor.diagnostics_for_line(line_idx);
            let has_bracket = bracket_positions
                .map_or(false, |((l1, _), (l2, _))| line_idx == l1 || line_idx == l2);
            let has_search = current_search.is_some();
            let has_ai_prompt_selection_on_line = ai_prompt_selection
                .map(|selection| line_idx >= selection.start_line && line_idx <= selection.end_line)
                .unwrap_or(false);
            let line_start_char = rope.line_to_char(line_idx);
            let mut line_end_char = if line_idx + 1 < rope.len_lines() {
                rope.line_to_char(line_idx + 1)
            } else {
                rope.len_chars()
            };
            if line_end_char > line_start_char && rope.char(line_end_char - 1) == '\n' {
                line_end_char = line_end_char.saturating_sub(1);
            }
            let has_ai_lock_on_line = buffer
                .ai_locks()
                .iter()
                .filter(|lock| lock.blocks_edits)
                .any(|lock| lock.start_char < line_end_char && lock.end_char > line_start_char);
            let has_ai_generated_on_line = editor.ai_regions().iter().any(|region| {
                region.status == AiRegionStatus::Generated
                    && region.start_char < line_end_char
                    && region.end_char > line_start_char
            });
            let is_stable = !has_visual_on_line
                && !is_cursor_line_early
                && !has_yank_flash
                && !has_bracket
                && !has_search
                && line_diagnostics_early.is_empty()
                && !has_ai_prompt_selection_on_line
                && !has_ai_lock_on_line
                && !has_ai_generated_on_line;

            if is_stable {
                if let Some(cached_line) = line_cache.get(
                    buffer_id,
                    line_idx,
                    buffer_version,
                    h_offset,
                    text_width,
                    wrap,
                    tab_width,
                ) {
                    let cached_line = cached_line.clone();
                    // Use cached line — skip all expensive computation
                    if has_wrap {
                        let visual_rows = split_line_into_rows(cached_line, text_width);
                        for (row_idx, row) in visual_rows.into_iter().enumerate() {
                            if visual_rows_used >= visible_lines {
                                break;
                            }
                            if gutter_area.is_some() {
                                gutter_lines.push(build_gutter_line(
                                    editor,
                                    buffer,
                                    theme,
                                    line_idx,
                                    line_num_width,
                                    cursor_line_idx,
                                    row_idx > 0,
                                    &line_diagnostics_early,
                                    blame_brackets
                                        .as_ref()
                                        .and_then(|b| b.get(line_idx - start_line)),
                                    blame_width,
                                ));
                            }
                            lines.push(row);
                            visual_rows_used += 1;
                        }
                    } else {
                        let line_len: usize = cached_line
                            .spans
                            .iter()
                            .map(|s| s.content.chars().count())
                            .sum();
                        let mut padded = cached_line;
                        if line_len < text_width {
                            padded
                                .spans
                                .push(Span::raw(" ".repeat(text_width - line_len)));
                        }
                        if gutter_area.is_some() {
                            gutter_lines.push(build_gutter_line(
                                editor,
                                buffer,
                                theme,
                                line_idx,
                                line_num_width,
                                cursor_line_idx,
                                false,
                                &line_diagnostics_early,
                                blame_brackets
                                    .as_ref()
                                    .and_then(|b| b.get(line_idx - start_line)),
                                blame_width,
                            ));
                        }
                        lines.push(padded);
                        visual_rows_used += 1;
                    }
                    line_idx += 1;
                    continue;
                }
            }

            let line_text_raw = rope.line(line_idx).to_string();
            // Remove trailing newline if present
            let line_text_original = line_text_raw.trim_end_matches('\n');

            // Expand tabs to spaces for proper rendering and get byte mapping
            let (line_text, byte_mapping, control_ranges, char_mapping) =
                expand_tabs_with_mapping(line_text_original, tab_width);

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
            let has_visual_selection = has_visual_on_line;

            let search_matches = if let Some(search) = current_search {
                search.find_all_in_line(&line_text)
            } else {
                Vec::new()
            };

            // Check if this is the cursor line and cursorline option is on
            let is_cursor_line = is_cursor_line_early;

            // Remap visual selection columns from original to expanded char indices
            let remapped_visual_selection = visual_selection.map(|((sl, sc), (el, ec))| {
                let sc = if line_idx == sl {
                    remap_char_col(sc, &char_mapping)
                } else {
                    sc
                };
                let ec = if line_idx == el {
                    remap_char_col(ec, &char_mapping)
                } else {
                    ec
                };
                ((sl, sc), (el, ec))
            });

            // Adjust visual selection for horizontal viewport if nowrap
            let remapped_visual_selection = if !wrap {
                remapped_visual_selection.map(|((sl, sc), (el, ec))| {
                    let adjust = |expanded_char_col: usize| -> usize {
                        let display_col =
                            expanded_char_to_display_col(&expanded_text, expanded_char_col);
                        let viewport_display_col = display_col.saturating_sub(h_offset);
                        let offset_adjustment = if precedes { 1 } else { 0 };
                        display_col_to_char_idx(
                            &line_text,
                            viewport_display_col + offset_adjustment,
                        )
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
                    let display_col =
                        expanded_char_to_display_col(&expanded_text, expanded_char_col);
                    // Check if bracket is in visible horizontal range
                    if display_col >= h_offset && display_col < h_offset + text_width {
                        // Convert to char index in the sliced text
                        let viewport_display_col = display_col - h_offset;
                        let offset_adjustment = if precedes { 1 } else { 0 };
                        let sliced_char_idx = display_col_to_char_idx(
                            &line_text,
                            viewport_display_col + offset_adjustment,
                        );
                        Some(sliced_char_idx)
                    } else {
                        None // Bracket is outside viewport
                    }
                })
            } else {
                bracket_col
            };

            // Reuse diagnostics already fetched for cache check
            let line_diagnostics = line_diagnostics_early;
            let has_diagnostics = !line_diagnostics.is_empty();
            let remapped_diagnostics: Vec<RemappedDiagnostic> = line_diagnostics
                .iter()
                .filter_map(|d| {
                    // Convert UTF-16 offsets to char indices, then remap through expansion
                    let start_char = utf16_offset_to_char_idx(
                        line_text_original,
                        d.range.start.character as usize,
                    );
                    let end_char = utf16_offset_to_char_idx(
                        line_text_original,
                        d.range.end.character as usize,
                    );
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
                        let start_display =
                            expanded_char_to_display_col(&expanded_text, expanded_start);
                        let end_display =
                            expanded_char_to_display_col(&expanded_text, expanded_end);
                        // Skip if entirely outside viewport
                        if end_display <= h_offset || start_display >= h_offset + text_width {
                            return None;
                        }
                        let offset_adj = if precedes { 1 } else { 0 };
                        let sliced_start = display_col_to_char_idx(
                            &line_text,
                            start_display.saturating_sub(h_offset) + offset_adj,
                        );
                        let sliced_end = display_col_to_char_idx(
                            &line_text,
                            end_display.saturating_sub(h_offset) + offset_adj,
                        );
                        Some(RemappedDiagnostic {
                            start: sliced_start,
                            end: sliced_end,
                            color,
                        })
                    } else {
                        Some(RemappedDiagnostic {
                            start: expanded_start,
                            end: expanded_end,
                            color,
                        })
                    }
                })
                .collect();
            let line_char_count = line_text_original.chars().count();
            let ai_lock_ranges = lock_ranges_for_line(
                buffer,
                line_idx,
                line_char_count,
                &char_mapping,
                &expanded_text,
                &line_text,
                wrap,
                h_offset,
                text_width,
                precedes,
            );
            let (ai_generated_ranges, ai_selected_ranges) = ai_region_ranges_for_line(
                editor,
                line_idx,
                line_char_count,
                &char_mapping,
                &expanded_text,
                &line_text,
                wrap,
                h_offset,
                text_width,
                precedes,
            );
            let ai_prompt_ranges = if let Some(selection) = ai_prompt_selection {
                if line_idx < selection.start_line || line_idx > selection.end_line {
                    Vec::new()
                } else {
                    let (start_col, end_col_inclusive) =
                        if selection.mode_before_prompt == crate::mode::Mode::VisualLine {
                            if line_char_count == 0 {
                                (0, 0)
                            } else {
                                (0, line_char_count - 1)
                            }
                        } else if selection.start_line == selection.end_line {
                            (
                                selection.start_col,
                                selection.end_col.min(line_char_count.saturating_sub(1)),
                            )
                        } else if line_idx == selection.start_line {
                            (selection.start_col, line_char_count.saturating_sub(1))
                        } else if line_idx == selection.end_line {
                            (0, selection.end_col.min(line_char_count.saturating_sub(1)))
                        } else if line_char_count == 0 {
                            (0, 0)
                        } else {
                            (0, line_char_count - 1)
                        };

                    if line_char_count == 0 || end_col_inclusive < start_col {
                        Vec::new()
                    } else {
                        let mut start = remap_char_col(start_col, &char_mapping);
                        let mut end_exclusive = remap_char_col(
                            (end_col_inclusive + 1).min(line_char_count),
                            &char_mapping,
                        );

                        if !wrap {
                            let start_display = expanded_char_to_display_col(&expanded_text, start);
                            let end_display =
                                expanded_char_to_display_col(&expanded_text, end_exclusive);
                            if end_display <= h_offset || start_display >= h_offset + text_width {
                                Vec::new()
                            } else {
                                let offset_adj = if precedes { 1 } else { 0 };
                                start = display_col_to_char_idx(
                                    &line_text,
                                    start_display.saturating_sub(h_offset) + offset_adj,
                                );
                                end_exclusive = display_col_to_char_idx(
                                    &line_text,
                                    end_display.saturating_sub(h_offset) + offset_adj,
                                );
                                if end_exclusive > start {
                                    vec![(start, end_exclusive - 1)]
                                } else {
                                    Vec::new()
                                }
                            }
                        } else if end_exclusive > start {
                            vec![(start, end_exclusive - 1)]
                        } else {
                            Vec::new()
                        }
                    }
                }
            } else {
                Vec::new()
            };

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
                || yank_flash.is_some()
                || !ai_prompt_ranges.is_empty()
                || !ai_lock_ranges.is_empty()
                || !ai_generated_ranges.is_empty()
                || !ai_selected_ranges.is_empty();

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

                // Keep AI prompt selection visually distinct while typing the instruction.
                let ai_prompt_bg = Color::Rgb(62, 70, 82);
                for (start_col, end_col) in &ai_prompt_ranges {
                    apply_bg_to_column_range(&mut line, *start_col, *end_col, ai_prompt_bg);
                }

                // Generated AI edits stay visible after completion with a muted background.
                let ai_generated_bg = Color::Rgb(30, 48, 38);
                for (start_col, end_col) in &ai_generated_ranges {
                    apply_bg_to_column_range(&mut line, *start_col, *end_col, ai_generated_bg);
                }

                // Highlight active AI lock spans so users see protected regions.
                let ai_lock_bg = Color::Rgb(26, 44, 62);
                for (start_col, end_col) in &ai_lock_ranges {
                    apply_bg_to_column_range(&mut line, *start_col, *end_col, ai_lock_bg);
                }

                // When cursor is inside an AI block, highlight the whole block as selected.
                let ai_selected_bg = Color::Rgb(44, 64, 78);
                for (start_col, end_col) in &ai_selected_ranges {
                    apply_bg_to_column_range(&mut line, *start_col, *end_col, ai_selected_bg);
                }

                // Apply bracket highlighting
                if let Some(col) = bracket_col {
                    let bracket_style = Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD);
                    apply_style_at_column(&mut line, col, bracket_style);
                }

                // Store in cache (stable lines will be served from cache next frame)
                line_cache.put(
                    buffer_id,
                    line_idx,
                    buffer_version,
                    h_offset,
                    text_width,
                    wrap,
                    tab_width,
                    line.clone(),
                    is_stable,
                );

                // Soft wrap: split into visual rows if needed
                if has_wrap {
                    let mut visual_rows = split_line_into_rows(line, text_width);
                    // Append diagnostic virtual text to the first visual row (after splitting)
                    if has_diagnostics && render_diagnostic_virtual_text_inline {
                        if let Some(first_row) = visual_rows.first_mut() {
                            append_diagnostic_virtual_text(
                                first_row,
                                &line_diagnostics,
                                text_width,
                            );
                        }
                    }
                    for (row_idx, row) in visual_rows.into_iter().enumerate() {
                        if visual_rows_used >= visible_lines {
                            break;
                        }
                        if gutter_area.is_some() {
                            gutter_lines.push(build_gutter_line(
                                editor,
                                buffer,
                                theme,
                                line_idx,
                                line_num_width,
                                cursor_line_idx,
                                row_idx > 0,
                                &line_diagnostics,
                                blame_brackets
                                    .as_ref()
                                    .and_then(|b| b.get(line_idx - start_line)),
                                blame_width,
                            ));
                        }
                        lines.push(row);
                        visual_rows_used += 1;
                    }
                } else {
                    // No wrap: append diagnostic virtual text before padding
                    if has_diagnostics && render_diagnostic_virtual_text_inline {
                        append_diagnostic_virtual_text(&mut line, &line_diagnostics, text_width);
                    }
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
                            theme,
                            line_idx,
                            line_num_width,
                            cursor_line_idx,
                            false,
                            &line_diagnostics,
                            blame_brackets
                                .as_ref()
                                .and_then(|b| b.get(line_idx - start_line)),
                            blame_width,
                        ));
                    }
                    lines.push(line);
                    visual_rows_used += 1;
                }
            } else {
                // Simple rendering path (no highlighting) — always stable
                let simple_line = Line::from(line_text.to_string());
                line_cache.put(
                    buffer_id,
                    line_idx,
                    buffer_version,
                    h_offset,
                    text_width,
                    wrap,
                    tab_width,
                    simple_line,
                    true,
                );

                if has_wrap {
                    let chars: Vec<char> = line_text.chars().collect();
                    if chars.is_empty() {
                        if gutter_area.is_some() {
                            gutter_lines.push(build_gutter_line(
                                editor,
                                buffer,
                                theme,
                                line_idx,
                                line_num_width,
                                cursor_line_idx,
                                false,
                                &[],
                                blame_brackets
                                    .as_ref()
                                    .and_then(|b| b.get(line_idx - start_line)),
                                blame_width,
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
                                    theme,
                                    line_idx,
                                    line_num_width,
                                    cursor_line_idx,
                                    chunk_idx > 0,
                                    &[],
                                    blame_brackets
                                        .as_ref()
                                        .and_then(|b| b.get(line_idx - start_line)),
                                    blame_width,
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
                            theme,
                            line_idx,
                            line_num_width,
                            cursor_line_idx,
                            false,
                            &[],
                            blame_brackets
                                .as_ref()
                                .and_then(|b| b.get(line_idx - start_line)),
                            blame_width,
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

/// Render diagnostic virtual text as an overlay using the full available terminal width.
///
/// This is primarily used when `textwidth` centering is enabled. In that mode, the main buffer
/// rendering is intentionally constrained to `layout.buffer_area.width`, but users still expect
/// diagnostic virtual text to use the extra right-side margin space.
pub fn render_diagnostic_virtual_text_overlay(
    frame: &mut Frame,
    editor: &Editor,
    layout: &BufferLayout,
    full_area: ratatui::layout::Rect,
) {
    let buffer_area = layout.buffer_area;
    if full_area.width <= buffer_area.width {
        return;
    }

    let buffer = editor.buffer();
    let rope = buffer.rope();
    let wrap = editor.options.wrap;
    let h_offset = editor.horizontal_offset();
    let tab_width = editor.options.tab_width;

    let visible_rows = buffer_area.height as usize;
    let start_line = editor.scroll_offset();
    let line_count = buffer.line_count();

    let gutter_width_u16 = layout.gutter_width as u16;
    let text_area_x = buffer_area.x + gutter_width_u16;
    let wrap_width = layout.text_width.max(1);
    let full_right = full_area.x + full_area.width;

    let mut visual_rows_used = 0usize;
    let mut line_idx = start_line;

    while line_idx < line_count && visual_rows_used < visible_rows {
        let first_row_screen = visual_rows_used;
        let line_diagnostics = editor.diagnostics_for_line(line_idx);

        let line_text_raw = if line_idx < rope.len_lines() {
            rope.line(line_idx).to_string()
        } else {
            String::new()
        };
        let line_text_original = line_text_raw.trim_end_matches('\n');
        let (expanded, _byte_mapping, _control_ranges, _char_mapping) =
            expand_tabs_with_mapping(line_text_original, tab_width);

        let display_source = if !wrap {
            slice_horizontal_viewport(&expanded, h_offset, wrap_width).0
        } else {
            expanded
        };

        // Determine the first visual row's code width (excluding padding).
        let mut first_row_text = if wrap {
            let rows = split_line_into_rows(Line::from(display_source.to_string()), wrap_width);
            let first = rows.first().cloned().unwrap_or_else(|| Line::from(""));
            let text: String = first.spans.iter().map(|s| s.content.as_ref()).collect();
            visual_rows_used += rows.len().max(1);
            text
        } else {
            visual_rows_used += 1;
            display_source.to_string()
        };

        // Strip trailing padding spaces.
        while first_row_text.ends_with(' ') {
            first_row_text.pop();
        }
        let code_width: usize = first_row_text.chars().map(char_display_width).sum();

        if !line_diagnostics.is_empty() {
            let diag = line_diagnostics[0];
            let msg = diag.message.lines().next().unwrap_or("");
            let (icon, fg_color, bg_color) = get_diagnostic_virtual_text_style(diag.severity);
            let vtext_style = Style::default()
                .fg(fg_color)
                .bg(bg_color)
                .add_modifier(Modifier::ITALIC);

            // We want virtual text to start after the rendered code, even if that is past the
            // centered textwidth. This allows it to appear in the right margin.
            let mut x = text_area_x.saturating_add(code_width as u16);
            let y = buffer_area.y + first_row_screen as u16;

            if y < full_area.y || y >= full_area.y + full_area.height {
                line_idx += 1;
                continue;
            }
            if x >= full_right {
                line_idx += 1;
                continue;
            }

            let available = full_right.saturating_sub(x) as usize;
            if available >= 6 {
                // Draw unstyled gap, then styled message.
                let buf = frame.buffer_mut();
                let (nx, _) = buf.set_stringn(x, y, "  ", available, Style::default());
                x = nx;
                let available_after_gap = full_right.saturating_sub(x) as usize;
                if available_after_gap > 0 {
                    // Keep Vim-ish truncation at terminal edge.
                    let prefix_width: usize =
                        format!("{} ", icon).chars().map(char_display_width).sum();
                    let max_msg_len = available_after_gap.saturating_sub(prefix_width);
                    let mut render_msg = msg.to_string();
                    if render_msg.chars().count() > max_msg_len {
                        render_msg = format!(
                            "{}...",
                            render_msg
                                .chars()
                                .take(max_msg_len.saturating_sub(3))
                                .collect::<String>()
                        );
                    }
                    buf.set_stringn(
                        x,
                        y,
                        format!("{} {}", icon, render_msg),
                        available_after_gap,
                        vtext_style,
                    );
                }
            }
        }

        line_idx += 1;
    }
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
    let num_chars = chars.len();
    let mut spans = Vec::new();

    if num_chars == 0 {
        return Line::from(spans);
    }

    // Build a map from character index to byte index
    let mut byte_indices: Vec<usize> = Vec::with_capacity(num_chars + 1);
    byte_indices.push(0);
    for (byte_idx, _) in line_text.char_indices().skip(1) {
        byte_indices.push(byte_idx);
    }
    byte_indices.push(line_text.len()); // End position

    // --- Pre-compute per-character style attributes in one pass each ---

    // 1. Visual selection: computed inline (cheap — no array scan needed, just arithmetic)

    // 2. Search matches: mark each char position
    let mut search_flags: Vec<bool> = vec![false; num_chars];
    for &(start, end) in search_matches {
        let s = start.min(num_chars);
        let e = end.min(num_chars);
        for flag in search_flags[s..e].iter_mut() {
            *flag = true;
        }
    }

    // 3. Syntax highlights: resolve most-specific (smallest range) group per byte position,
    //    then map to char positions.
    let mut syntax_per_char: Vec<Option<crate::syntax::HighlightGroup>> = vec![None; num_chars];
    if !syntax_highlights.is_empty() {
        // For each byte, track (group, range_size) of the most specific highlight.
        let byte_len = line_text.len();
        let mut best_group: Vec<Option<(crate::syntax::HighlightGroup, usize)>> =
            vec![None; byte_len];

        for (range, group) in syntax_highlights {
            let range_size = range.end - range.start;
            let s = range.start.min(byte_len);
            let e = range.end.min(byte_len);
            for byte_pos in s..e {
                match best_group[byte_pos] {
                    Some((_, prev_size)) if prev_size <= range_size => {} // keep tighter
                    _ => best_group[byte_pos] = Some((*group, range_size)),
                }
            }
        }

        // Map byte-level results to char positions
        for (char_idx, &byte_idx) in byte_indices[..num_chars].iter().enumerate() {
            if byte_idx < byte_len {
                syntax_per_char[char_idx] = best_group[byte_idx].map(|(g, _)| g);
            }
        }
    }

    // 4. Diagnostics: mark each char position with underline color
    let mut diag_per_char: Vec<Option<Color>> = vec![None; num_chars];
    for d in diagnostics {
        let s = d.start.min(num_chars);
        let e = d.end.min(num_chars);
        for slot in diag_per_char[s..e].iter_mut() {
            if slot.is_none() {
                *slot = Some(d.color);
            }
        }
    }

    // 5. Control char ranges: mark each byte position, then map to chars
    let mut control_per_char: Vec<bool> = vec![false; num_chars];
    if !control_ranges.is_empty() {
        let byte_len = line_text.len();
        let mut control_bytes: Vec<bool> = vec![false; byte_len];
        for r in control_ranges {
            let s = r.start.min(byte_len);
            let e = r.end.min(byte_len);
            for flag in control_bytes[s..e].iter_mut() {
                *flag = true;
            }
        }
        for (char_idx, &byte_idx) in byte_indices[..num_chars].iter().enumerate() {
            if byte_idx < byte_len {
                control_per_char[char_idx] = control_bytes[byte_idx];
            }
        }
    }

    // --- Helper: compute visual selection for a given column ---
    let is_col_selected = |col: usize| -> bool {
        if let Some(((sel_start_line, sel_start_col), (sel_end_line, sel_end_col))) =
            visual_selection
        {
            match mode {
                crate::mode::Mode::VisualBlock => {
                    line_idx >= sel_start_line
                        && line_idx <= sel_end_line
                        && col >= sel_start_col
                        && col <= sel_end_col
                }
                _ => {
                    if line_idx == sel_start_line && line_idx == sel_end_line {
                        col >= sel_start_col && col <= sel_end_col
                    } else if line_idx == sel_start_line {
                        col >= sel_start_col
                    } else if line_idx == sel_end_line {
                        col <= sel_end_col
                    } else {
                        line_idx > sel_start_line && line_idx < sel_end_line
                    }
                }
            }
        } else {
            false
        }
    };

    // --- Main loop: group consecutive characters with identical styling ---
    let mut col_idx = 0;
    while col_idx < num_chars {
        let is_selected = is_col_selected(col_idx);
        let is_search_match = search_flags[col_idx];
        let syntax_group = syntax_per_char[col_idx];
        let diag_underline_color = diag_per_char[col_idx];
        let is_control = control_per_char[col_idx];

        // Extend span while styling is identical
        let mut end_col = col_idx + 1;
        while end_col < num_chars
            && is_col_selected(end_col) == is_selected
            && search_flags[end_col] == is_search_match
            && syntax_per_char[end_col] == syntax_group
            && diag_per_char[end_col] == diag_underline_color
            && control_per_char[end_col] == is_control
        {
            end_col += 1;
        }

        // Build the span for this range
        let text: String = chars[col_idx..end_col].iter().collect();

        // Apply styling based on priority: visual selection > search match > control char > syntax > normal
        let mut style = if is_selected {
            Style::default()
                .bg(crate::key_convert::convert_core_color(
                    theme.get_ui_color(UiGroup::Visual),
                ))
                .fg(Color::White)
        } else if is_search_match {
            Style::default()
                .bg(crate::key_convert::convert_core_color(
                    theme.get_ui_color(UiGroup::Search),
                ))
                .fg(Color::Black)
        } else if is_control {
            let color =
                crate::key_convert::convert_core_color(theme.get_color(HighlightGroup::SpecialKey));
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

    // --- slice_horizontal_viewport tests ---

    #[test]
    fn test_slice_viewport_ascii_fits() {
        let (text, precedes, extends) = slice_horizontal_viewport("hello", 0, 10);
        assert_eq!(text, "hello");
        assert!(!precedes);
        assert!(!extends);
    }

    #[test]
    fn test_slice_viewport_ascii_extends() {
        let (text, precedes, extends) = slice_horizontal_viewport("hello world!", 0, 6);
        assert_eq!(text.len(), 6);
        assert!(!precedes);
        assert!(extends);
        assert!(text.ends_with('>'));
    }

    #[test]
    fn test_slice_viewport_ascii_scrolled() {
        // "hello world!" scrolled 3 display cols, width 6
        let (text, precedes, extends) = slice_horizontal_viewport("hello world!", 3, 6);
        assert!(precedes);
        assert!(extends);
        assert_eq!(text.chars().next(), Some('<'));
        assert!(text.ends_with('>'));
        assert_eq!(text.chars().count(), 6);
    }

    #[test]
    fn test_slice_viewport_cjk_fits() {
        // "a世b" has display width 4 (1+2+1)
        let (text, precedes, extends) = slice_horizontal_viewport("a世b", 0, 5);
        assert_eq!(text, "a世b");
        assert!(!precedes);
        assert!(!extends);
    }

    #[test]
    fn test_slice_viewport_cjk_extends() {
        // "a世b世c" has display width 8 (1+2+1+2+1), viewport width 5
        let (text, precedes, extends) = slice_horizontal_viewport("a世b世c", 0, 5);
        assert!(!precedes);
        assert!(extends);
        // Should fit "a世b" (4 cols) + '>' = 5 display cols total
        assert!(text.ends_with('>'));
    }

    #[test]
    fn test_slice_viewport_cjk_scrolled() {
        // "a世b世c" scrolled past 'a' and '世' (3 display cols), width 5
        let (text, precedes, extends) = slice_horizontal_viewport("a世b世c", 3, 5);
        assert!(precedes);
        assert!(!extends);
        // Should show '<' + content from display col 3 + possibly '>'
        assert_eq!(text.chars().next(), Some('<'));
    }

    #[test]
    fn test_slice_viewport_zero_width() {
        let (text, precedes, extends) = slice_horizontal_viewport("hello", 0, 0);
        assert_eq!(text, "");
        assert!(!precedes);
        assert!(!extends);
    }

    // --- Helper function tests ---

    #[test]
    fn test_expanded_char_to_display_col() {
        // "a世b" → char 0='a'(width 1), char 1='世'(width 2), char 2='b'(width 1)
        assert_eq!(expanded_char_to_display_col("a世b", 0), 0);
        assert_eq!(expanded_char_to_display_col("a世b", 1), 1);
        assert_eq!(expanded_char_to_display_col("a世b", 2), 3);
    }

    #[test]
    fn test_display_col_to_char_idx_basic() {
        // "a世b" display cols: a=0, 世=1-2, b=3
        assert_eq!(display_col_to_char_idx("a世b", 0), 0);
        assert_eq!(display_col_to_char_idx("a世b", 1), 1);
        assert_eq!(display_col_to_char_idx("a世b", 2), 1); // mid-wide → same char
        assert_eq!(display_col_to_char_idx("a世b", 3), 2);
    }

    #[test]
    fn test_utf16_offset_to_char_idx() {
        // ASCII: 1 UTF-16 unit per char
        assert_eq!(utf16_offset_to_char_idx("hello", 2), 2);
        // BMP CJK: 1 UTF-16 unit per char
        assert_eq!(utf16_offset_to_char_idx("a世b", 1), 1);
        assert_eq!(utf16_offset_to_char_idx("a世b", 2), 2);
        // Supplementary (emoji): 2 UTF-16 units
        assert_eq!(utf16_offset_to_char_idx("a😀b", 1), 1); // start of emoji
        assert_eq!(utf16_offset_to_char_idx("a😀b", 3), 2); // after emoji (2 UTF-16 units)
    }

    #[test]
    fn test_append_diagnostic_virtual_text_on_padded_wrapped_row() {
        let base = Line::from("let x = 1;".to_string());
        let mut rows = split_line_into_rows(base, 30);
        let mut first = rows.remove(0);

        let diag = lsp_types::Diagnostic {
            range: lsp_types::Range {
                start: lsp_types::Position {
                    line: 0,
                    character: 0,
                },
                end: lsp_types::Position {
                    line: 0,
                    character: 1,
                },
            },
            severity: Some(lsp_types::DiagnosticSeverity::ERROR),
            message: "uh oh".to_string(),
            ..Default::default()
        };
        let diags = vec![&diag];

        append_diagnostic_virtual_text(&mut first, &diags, 30);

        let mut rendered = String::new();
        for span in &first.spans {
            rendered.push_str(span.content.as_ref());
        }
        assert!(rendered.contains("uh oh"));

        let display_width: usize = rendered.chars().map(char_display_width).sum();
        assert_eq!(display_width, 30);
    }

    #[test]
    fn test_lock_ranges_for_line_basic() {
        let mut buffer = crate::buffer::Buffer::new_from_str("hello world\n");
        buffer.add_ai_lock(1, 6, 11);

        let line_text = "hello world";
        let (expanded, _byte_mapping, _control_ranges, char_mapping) =
            expand_tabs_with_mapping(line_text, 4);

        let ranges = lock_ranges_for_line(
            &buffer,
            0,
            line_text.chars().count(),
            &char_mapping,
            &expanded,
            &expanded,
            true,
            0,
            80,
            false,
        );
        assert_eq!(ranges, vec![(6, 10)]);
    }

    #[test]
    fn test_render_line_empty_string() {
        let theme = Theme::default();
        let line = render_line_with_highlights(
            &theme,
            "",
            0,
            None,
            crate::mode::Mode::Normal,
            &[],
            &[],
            &[],
            &[],
        );
        assert!(line.spans.is_empty());
    }

    #[test]
    fn test_render_line_plain_text_single_span() {
        let theme = Theme::default();
        let line = render_line_with_highlights(
            &theme,
            "hello world",
            0,
            None,
            crate::mode::Mode::Normal,
            &[],
            &[],
            &[],
            &[],
        );
        // No highlights → should coalesce into one span
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content.as_ref(), "hello world");
    }

    #[test]
    fn test_render_line_syntax_highlight_splits_spans() {
        let theme = Theme::default();
        // Highlight bytes 0..2 ("fn") as Keyword
        let highlights = vec![(0..2, crate::syntax::HighlightGroup::Keyword)];
        let line = render_line_with_highlights(
            &theme,
            "fn main()",
            0,
            None,
            crate::mode::Mode::Normal,
            &[],
            &highlights,
            &[],
            &[],
        );
        // Should have at least 2 spans: "fn" (highlighted) and " main()" (default)
        assert!(line.spans.len() >= 2);
        assert_eq!(line.spans[0].content.as_ref(), "fn");
    }

    #[test]
    fn test_render_line_search_match_overrides_syntax() {
        let theme = Theme::default();
        let highlights = vec![(0..5, crate::syntax::HighlightGroup::Function)];
        // Search match on chars 0..5 ("hello")
        let search = vec![(0, 5)];
        let line = render_line_with_highlights(
            &theme,
            "hello world",
            0,
            None,
            crate::mode::Mode::Normal,
            &search,
            &highlights,
            &[],
            &[],
        );
        // First span should be search-highlighted, not syntax-highlighted
        assert!(line.spans.len() >= 2);
        assert_eq!(line.spans[0].content.as_ref(), "hello");
        // Search highlight has bg color (non-default style)
        assert_ne!(line.spans[0].style, Style::default());
    }

    #[test]
    fn test_render_line_multibyte_chars() {
        let theme = Theme::default();
        // "aé" - 'é' is 2 bytes in UTF-8. Highlight byte range 0..1 ("a" only).
        let highlights = vec![(0..1, crate::syntax::HighlightGroup::Keyword)];
        let line = render_line_with_highlights(
            &theme,
            "aéb",
            0,
            None,
            crate::mode::Mode::Normal,
            &[],
            &highlights,
            &[],
            &[],
        );
        assert!(line.spans.len() >= 2);
        assert_eq!(line.spans[0].content.as_ref(), "a");
    }

    #[test]
    fn test_render_line_diagnostic_underline() {
        let theme = Theme::default();
        let diags = vec![RemappedDiagnostic {
            start: 0,
            end: 5,
            color: Color::Red,
        }];
        let line = render_line_with_highlights(
            &theme,
            "error here",
            0,
            None,
            crate::mode::Mode::Normal,
            &[],
            &[],
            &diags,
            &[],
        );
        // First span should have underline modifier
        assert!(line.spans[0]
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
    }

    #[test]
    fn test_render_line_most_specific_syntax_wins() {
        let theme = Theme::default();
        // Two overlapping highlights: broad (0..10) and narrow (2..4).
        // The narrow one should win for chars at byte positions 2-3.
        let highlights = vec![
            (0..10, crate::syntax::HighlightGroup::Variable),
            (2..4, crate::syntax::HighlightGroup::Keyword),
        ];
        let line = render_line_with_highlights(
            &theme,
            "abcdefghij",
            0,
            None,
            crate::mode::Mode::Normal,
            &[],
            &highlights,
            &[],
            &[],
        );
        // Should have 3 spans: "ab" (Variable), "cd" (Keyword), "efghij" (Variable)
        assert!(line.spans.len() >= 3);
        assert_eq!(line.spans[0].content.as_ref(), "ab");
        assert_eq!(line.spans[1].content.as_ref(), "cd");
    }
}
