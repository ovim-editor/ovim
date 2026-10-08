//! Horizontal viewport slicing for nowrap lines: which cells of a tab-expanded
//! line are on screen, and where highlight ranges land inside the slice.

use crate::display::grapheme_display_width;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Converts an expanded char index to a display column.
///
/// Thin wrapper over the shared grapheme-aware conversion: the input is
/// already tab-expanded, so the tab width is irrelevant (any value works).
pub(super) fn expanded_char_to_display_col(text: &str, char_idx: usize) -> usize {
    crate::display::char_col_to_display_col(text, char_idx, 1)
}

/// Converts a display column to a char index within a string.
/// If the display column falls in the middle of a wide grapheme, returns the
/// char index of that grapheme's first char. Input is already tab-expanded.
pub(super) fn display_col_to_char_idx(text: &str, target_display_col: usize) -> usize {
    crate::display::display_col_to_char_col(text, target_display_col, 1)
}

/// A horizontal slice retains the exact source scalars it displays. Styling
/// uses this mapping too, including when scrolling snaps to a wide grapheme.
pub(super) struct HorizontalViewport {
    pub(super) text: String,
    source_chars: Range<usize>,
    /// Cells (all single-char) before the first displayed source character:
    /// the `<` indicator drawn over the first cell of a scrolled line, plus
    /// blanks standing in for the rest of a wide grapheme it cut in half.
    pub(super) left: usize,
}

impl HorizontalViewport {
    pub(super) fn project_range(&self, range: Range<usize>) -> Option<Range<usize>> {
        let start = range.start.max(self.source_chars.start);
        let end = range.end.min(self.source_chars.end);
        let left = self.left;
        (start < end)
            .then(|| start - self.source_chars.start + left..end - self.source_chars.start + left)
    }
}

/// Slice by display columns while preserving complete graphemes. Indicators
/// and padding are outside `source_chars` and cannot acquire text highlights.
///
/// A scrolled line draws the `<` indicator over its first visible cell, as Vim
/// does for `precedes`, so every other character stays exactly `h_offset`
/// columns left of its display column — what the cursor and mouse mapping
/// assume.
pub(super) fn slice_horizontal_viewport(
    line: &str,
    h_offset: usize,
    width: usize,
) -> HorizontalViewport {
    // Safety check: if width is 0 or too small, return empty or minimal content
    if width == 0 {
        return HorizontalViewport {
            text: String::new(),
            source_chars: 0..0,
            left: 0,
        };
    }

    // Calculate total display width of the line
    let total_display_width: usize = line.graphemes(true).map(grapheme_display_width).sum();

    // Line fits entirely in viewport
    if total_display_width <= width {
        return HorizontalViewport {
            text: line.to_string(),
            source_chars: 0..line.chars().count(),
            left: 0,
        };
    }

    // The first visible cell is the `<` indicator, so text shows from the
    // next cell. Skip every grapheme that starts before it; one that was cut
    // in half by the edge leaves blanks so the rest keeps its column.
    let precedes = h_offset > 0;
    let content_start = h_offset + usize::from(precedes);
    let mut display_col = 0;
    let mut graphemes = line.graphemes(true).peekable();
    let mut source_start = 0;
    while let Some(&grapheme) = graphemes.peek() {
        if display_col >= content_start {
            break;
        }
        display_col += grapheme_display_width(grapheme);
        source_start += grapheme.chars().count();
        graphemes.next();
    }

    let left = (usize::from(precedes) + display_col.saturating_sub(content_start)).min(width);
    let extends = total_display_width - display_col > width - left && (!precedes || width > 1);
    let content_width = (width - left).saturating_sub(usize::from(extends));
    let mut result = String::new();
    if precedes {
        result.push('<');
    }
    result.extend(std::iter::repeat_n(' ', left.saturating_sub(1)));

    // Collect graphemes that fit within content_width display columns
    let mut content_display_width = 0;
    let mut source_end = source_start;
    while let Some(&grapheme) = graphemes.peek() {
        let g_width = grapheme_display_width(grapheme);
        if content_display_width + g_width > content_width {
            break;
        }
        result.push_str(grapheme);
        source_end += grapheme.chars().count();
        content_display_width += g_width;
        graphemes.next();
    }

    // Pad if a wide grapheme didn't fit exactly
    while content_display_width < content_width {
        result.push(' ');
        content_display_width += 1;
    }

    // Add extends indicator (>) if content continues right
    if extends {
        result.push('>');
    }

    HorizontalViewport {
        text: result,
        source_chars: source_start..source_end,
        left,
    }
}

/// Reusable scratch buffers for `shift_highlights_for_viewport` to avoid
/// allocating two `Vec<usize>` per visible line per frame.
pub(super) struct HighlightShiftBuffers {
    byte_to_display: Vec<usize>,
    display_to_byte: Vec<usize>,
}

impl HighlightShiftBuffers {
    pub(super) fn new() -> Self {
        Self {
            byte_to_display: Vec::with_capacity(256),
            display_to_byte: Vec::with_capacity(256),
        }
    }
}

/// Shifts syntax highlight ranges for horizontal viewport.
/// Highlights are in expanded byte ranges; h_offset and width are in display columns.
/// `left` is the number of leading cells of the slice that are not source text.
/// Returns byte ranges into the sliced text.
///
/// `buffers` provides reusable scratch space — the caller keeps one instance
/// across all lines in the render pass, eliminating per-line allocation.
pub(super) fn shift_highlights_for_viewport<T: Copy>(
    highlights: &[(Range<usize>, T)],
    expanded_text: &str,
    sliced_text: &str,
    h_offset: usize,
    width: usize,
    left: usize,
    buffers: &mut HighlightShiftBuffers,
) -> Vec<(Range<usize>, T)> {
    // Build a byte-offset-to-display-column mapping for the expanded text
    let byte_to_display = &mut buffers.byte_to_display;
    byte_to_display.clear();
    {
        let mut display_col = 0;
        for (byte_idx, grapheme) in expanded_text.grapheme_indices(true) {
            while byte_to_display.len() <= byte_idx {
                byte_to_display.push(display_col);
            }
            display_col += grapheme_display_width(grapheme);
        }
        while byte_to_display.len() <= expanded_text.len() {
            byte_to_display.push(display_col);
        }
    }

    // Build display-column-to-byte-offset mapping for the sliced text
    let sliced_display_to_byte = &mut buffers.display_to_byte;
    sliced_display_to_byte.clear();
    {
        for (byte_idx, grapheme) in sliced_text.grapheme_indices(true) {
            let g_width = grapheme_display_width(grapheme);
            for _ in 0..g_width {
                sliced_display_to_byte.push(byte_idx);
            }
        }
        sliced_display_to_byte.push(sliced_text.len()); // sentinel
    }

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

            // Highlight is completely before the text (the `<` indicator and
            // blanks in front of it carry no highlights)
            if end_display <= h_offset + left {
                return None;
            }
            // Highlight is completely after viewport
            if start_display >= viewport_end {
                return None;
            }

            // Clip to viewport display columns
            let clipped_start = start_display.saturating_sub(h_offset).max(left);
            let clipped_end = end_display.saturating_sub(h_offset).min(width);

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
