use ovim_core::editor::ai_chat_input::wrap_chat_input_rows;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Terminal display width of `text` in columns (wide chars occupy 2 columns).
///
/// Uses string-based measurement so multi-codepoint grapheme clusters (ZWJ
/// emoji like 👩‍🔬, combining marks) count their rendered width instead of
/// the sum of their component chars.
pub(super) fn text_display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Truncates `text` to at most `max_width` display columns, appending an
/// ellipsis when truncated. Column budgets must be measured in display width
/// (not chars) or wide characters (CJK, emoji) overflow their span. Truncation
/// happens on grapheme boundaries so ZWJ emoji sequences are never split.
pub(super) fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if text_display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let budget = max_width - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for grapheme in text.graphemes(true) {
        let width = UnicodeWidthStr::width(grapheme);
        if used + width > budget {
            break;
        }
        out.push_str(grapheme);
        used += width;
    }
    out.push('\u{2026}');
    out
}

// ---------------------------------------------------------------------------
// Styled Word Wrap
// ---------------------------------------------------------------------------

/// A wrapped row and its visible byte range in the original styled line.
pub(super) struct WrappedChatLine {
    pub(super) spans: Vec<Span<'static>>,
    pub(super) source: std::ops::Range<usize>,
}

/// Wrap styled text without splitting words that fit on a complete row.
pub(super) fn styled_word_wrap_line(line: &Line<'_>, max_width: usize) -> Vec<Vec<Span<'static>>> {
    styled_word_wrap_line_with_ranges(line, max_width)
        .into_iter()
        .map(|row| row.spans)
        .collect()
}

pub(super) fn styled_word_wrap_line_with_ranges(
    line: &Line<'_>,
    max_width: usize,
) -> Vec<WrappedChatLine> {
    if max_width == 0 {
        return vec![WrappedChatLine {
            spans: line
                .spans
                .iter()
                .map(|span| Span::styled(span.content.to_string(), span.style))
                .collect(),
            source: 0..line.spans.iter().map(|span| span.content.len()).sum(),
        }];
    }

    let mut text = String::new();
    let mut styled_ranges = Vec::new();
    for span in &line.spans {
        let start = text.len();
        text.push_str(span.content.as_ref());
        if text.len() > start {
            styled_ranges.push((start, text.len(), span.style));
        }
    }

    if text.is_empty() {
        return vec![WrappedChatLine {
            spans: vec![],
            source: 0..0,
        }];
    }

    wrap_chat_input_rows(&text, max_width, 4)
        .into_iter()
        .map(|row| {
            let row_text = &text[row.visible_start..row.end];
            let trailing_whitespace = row_text
                .char_indices()
                .rev()
                .take_while(|(_, character)| character.is_whitespace())
                .map(|(index, _)| index)
                .last();
            let visible_end = trailing_whitespace
                .map(|index| row.visible_start + index)
                .unwrap_or(row.end);
            let spans = styled_ranges
                .iter()
                .filter_map(|&(style_start, style_end, style)| {
                    let start = row.visible_start.max(style_start);
                    let end = visible_end.min(style_end);
                    (start < end).then(|| Span::styled(text[start..end].to_string(), style))
                })
                .collect();
            WrappedChatLine {
                spans,
                source: row.visible_start..visible_end,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Word Wrap
// ---------------------------------------------------------------------------

pub(super) fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    let line = Line::from(Span::raw(text.to_string()));
    styled_word_wrap_line(&line, max_width)
        .into_iter()
        .map(|spans| {
            spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect()
}

pub(super) fn center_text(text: &str, width: usize) -> String {
    let text_len = text_display_width(text);
    if text_len >= width {
        return text.to_string();
    }
    let padding = (width - text_len) / 2;
    format!(
        "{}{}{}",
        " ".repeat(padding),
        text,
        " ".repeat(width - padding - text_len)
    )
}
