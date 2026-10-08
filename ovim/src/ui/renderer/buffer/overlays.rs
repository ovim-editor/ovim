//! Restyling of already rendered lines (single columns and column ranges) and
//! the matching-bracket search behind the bracket highlight.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// Apply a style to a specific column in a line, splitting spans as needed.
pub(super) fn apply_style_at_column(line: &mut Line<'static>, target_col: usize, style: Style) {
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
pub(super) fn apply_bg_to_column_range(
    line: &mut Line<'static>,
    start_col: usize,
    end_col: usize,
    bg: Color,
) {
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

/// Apply a foreground color and modifier to a column range of a rendered line.
pub(super) fn apply_fg_modifier_to_column_range(
    line: &mut Line<'static>,
    start_col: usize,
    end_col_exclusive: usize,
    fg: Color,
    modifier: Modifier,
) {
    let mut new_spans: Vec<Span<'static>> = Vec::new();
    let mut current_col = 0;

    for span in line.spans.drain(..) {
        let span_len = span.content.chars().count();
        let span_end = current_col + span_len;

        if span_end <= start_col || current_col >= end_col_exclusive {
            new_spans.push(span);
        } else if current_col >= start_col && span_end <= end_col_exclusive {
            new_spans.push(Span::styled(
                span.content,
                span.style.fg(fg).add_modifier(modifier),
            ));
        } else {
            let chars: Vec<char> = span.content.chars().collect();
            let range_start = start_col.saturating_sub(current_col);
            let range_end = end_col_exclusive
                .saturating_sub(current_col)
                .min(chars.len());

            if range_start > 0 {
                let before: String = chars[..range_start].iter().collect();
                new_spans.push(Span::styled(before, span.style));
            }
            if range_start < range_end {
                let middle: String = chars[range_start..range_end].iter().collect();
                new_spans.push(Span::styled(
                    middle,
                    span.style.fg(fg).add_modifier(modifier),
                ));
            }
            if range_end < chars.len() {
                let after: String = chars[range_end..].iter().collect();
                new_spans.push(Span::styled(after, span.style));
            }
        }

        current_col = span_end;
    }

    line.spans = new_spans;
}

/// Find matching bracket position if cursor is on a bracket
pub(super) fn find_matching_bracket_position(
    buffer: &crate::buffer::Buffer,
) -> Option<(usize, usize)> {
    let cursor = buffer.cursor();
    let rope = buffer.rope();
    let line_idx = cursor.line();

    if line_idx >= rope.len_lines() {
        return None;
    }

    let line = rope.line(line_idx);
    let col = buffer.cursor_char_col().0;

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
