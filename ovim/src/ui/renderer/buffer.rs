use crate::editor::Editor;
use crate::syntax::{Theme, UiGroup};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::display::grapheme_display_width;
use crate::ui::renderer::markdown_conceal::scan_markdown_conceal;
use unicode_segmentation::UnicodeSegmentation;

use super::helpers::{compose_conceal_and_tabs, expand_tabs_with_mapping, remap_char_col};
use super::layout::{BufferLayout, GUTTER_SPACING, SIGN_WIDTH};
use super::styles::{
    blame_color_for_hash, blame_style, get_diagnostic_sign_style, get_git_sign_style,
    get_line_number_style, remap_highlights,
};
use crate::syntax::HighlightGroup;
use ovim_core::buffer::Cursor;
use ovim_core::line_layout::{LayoutFragment, LayoutFragmentKind, LayoutRow};
use ovim_core::unicode::{
    char_to_grapheme_col, grapheme_count, grapheme_to_char_col, CharCol, GraphemeCol,
};
use std::ops::Range;

/// Window-specific rendering context for multi-window support.
/// When provided, these values override the editor's focused window state.
#[derive(Default)]
pub struct WindowRenderContext {
    /// Override cursor position (for non-focused windows)
    pub cursor: Option<Cursor>,
    /// Override scroll offset (for non-focused windows)
    pub scroll_offset: Option<usize>,
    /// Override visual sub-row offset within the top line (for non-focused windows)
    pub scroll_subrow: Option<usize>,
    /// Override horizontal scroll offset (for non-focused windows)
    pub horizontal_offset: Option<usize>,
    /// Use this window's own soft-wrap map (built at *this* window's content
    /// width) instead of the editor-global one. `None` ⇒ use `editor.wrap_map()`
    /// (single-window / focused-window path). (roadmap 19 / OV-00209)
    pub wrap_map_window_index: Option<usize>,
}

/// Converts an expanded char index to a display column.
///
/// Thin wrapper over the shared grapheme-aware conversion: the input is
/// already tab-expanded, so the tab width is irrelevant (any value works).
fn expanded_char_to_display_col(text: &str, char_idx: usize) -> usize {
    crate::display::char_col_to_display_col(text, char_idx, 1)
}

/// Converts a display column to a char index within a string.
/// If the display column falls in the middle of a wide grapheme, returns the
/// char index of that grapheme's first char. Input is already tab-expanded.
fn display_col_to_char_idx(text: &str, target_display_col: usize) -> usize {
    crate::display::display_col_to_char_col(text, target_display_col, 1)
}

/// A horizontal slice retains the exact source scalars it displays. Styling
/// uses this mapping too, including when scrolling snaps to a wide grapheme.
struct HorizontalViewport {
    text: String,
    source_chars: Range<usize>,
    precedes: bool,
}

impl HorizontalViewport {
    fn project_range(&self, range: Range<usize>) -> Option<Range<usize>> {
        let start = range.start.max(self.source_chars.start);
        let end = range.end.min(self.source_chars.end);
        let left = usize::from(self.precedes);
        (start < end)
            .then(|| start - self.source_chars.start + left..end - self.source_chars.start + left)
    }
}

/// Slice by display columns while preserving complete graphemes. Indicators
/// and padding are outside `source_chars` and cannot acquire text highlights.
fn slice_horizontal_viewport(line: &str, h_offset: usize, width: usize) -> HorizontalViewport {
    // Safety check: if width is 0 or too small, return empty or minimal content
    if width == 0 {
        return HorizontalViewport {
            text: String::new(),
            source_chars: 0..0,
            precedes: false,
        };
    }

    // Calculate total display width of the line
    let total_display_width: usize = line.graphemes(true).map(grapheme_display_width).sum();

    // Line fits entirely in viewport
    if total_display_width <= width {
        return HorizontalViewport {
            text: line.to_string(),
            source_chars: 0..line.chars().count(),
            precedes: false,
        };
    }

    // Walk graphemes to find the start position (skip h_offset display columns)
    let mut display_col = 0;
    let mut graphemes = line.graphemes(true).peekable();

    let mut source_start = 0;
    // Skip graphemes until we reach h_offset
    while let Some(&grapheme) = graphemes.peek() {
        let g_width = grapheme_display_width(grapheme);
        if display_col + g_width > h_offset {
            break;
        }
        display_col += g_width;
        source_start += grapheme.chars().count();
        graphemes.next();
    }

    let precedes = h_offset > 0;
    let left_width = usize::from(precedes);
    // Use the snapped start and account for the left indicator: both can
    // leave more text offscreen than `h_offset + width` would suggest.
    let extends =
        total_display_width - display_col > width - left_width && (!precedes || width > 1);
    let content_width = width - left_width - usize::from(extends);
    let mut result = String::new();
    if precedes {
        result.push('<');
    }

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
        precedes,
    }
}

/// Reusable scratch buffers for `shift_highlights_for_viewport` to avoid
/// allocating two `Vec<usize>` per visible line per frame.
struct HighlightShiftBuffers {
    byte_to_display: Vec<usize>,
    display_to_byte: Vec<usize>,
}

impl HighlightShiftBuffers {
    fn new() -> Self {
        Self {
            byte_to_display: Vec::with_capacity(256),
            display_to_byte: Vec::with_capacity(256),
        }
    }
}

/// Shifts syntax highlight ranges for horizontal viewport.
/// Highlights are in expanded byte ranges; h_offset and width are in display columns.
/// Returns byte ranges into the sliced text.
///
/// `buffers` provides reusable scratch space — the caller keeps one instance
/// across all lines in the render pass, eliminating per-line allocation.
fn shift_highlights_for_viewport<T: Copy>(
    highlights: &[(Range<usize>, T)],
    expanded_text: &str,
    sliced_text: &str,
    h_offset: usize,
    width: usize,
    precedes: bool,
    buffers: &mut HighlightShiftBuffers,
) -> Vec<(Range<usize>, T)> {
    let offset_adjustment = if precedes { 1 } else { 0 }; // Account for '<' indicator

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

/// Apply a foreground color and modifier to a column range of a rendered line.
fn apply_fg_modifier_to_column_range(
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
fn find_matching_bracket_position(buffer: &crate::buffer::Buffer) -> Option<(usize, usize)> {
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
/// Invariant context for gutter rendering within a single render pass.
/// Constructed once before the line loop and passed to all `build_gutter_line` calls.
struct GutterContext<'a> {
    editor: &'a Editor,
    buffer: &'a crate::buffer::Buffer,
    theme: &'a Theme,
    line_num_width: usize,
    cursor_line: usize,
    blame_width: usize,
    fold_width: usize,
    walkthrough_range: Option<(usize, usize)>,
}

const WALKTHROUGH_SELECTION_BG: Color = Color::Rgb(34, 57, 76);
const WALKTHROUGH_GUTTER_FG: Color = Color::Rgb(96, 176, 255);

fn line_is_in_walkthrough(range: Option<(usize, usize)>, line_idx: usize) -> bool {
    range.is_some_and(|(start, end)| line_idx >= start && line_idx <= end)
}

fn build_gutter_line(
    ctx: &GutterContext,
    line_idx: usize,
    is_continuation: bool,
    line_diagnostics: &[lsp_types::Diagnostic],
    blame_info: Option<&(BlameBracket, String, String, Color)>,
) -> Line<'static> {
    let editor = ctx.editor;
    let buffer = ctx.buffer;
    let theme = ctx.theme;
    let line_num_width = ctx.line_num_width;
    let cursor_line = ctx.cursor_line;
    let blame_width = ctx.blame_width;
    let fold_width = ctx.fold_width;

    if is_continuation {
        // Blank gutter for wrap continuation rows
        let width = blame_width + fold_width + SIGN_WIDTH + line_num_width + GUTTER_SPACING;
        if blame_width > 0 {
            if let Some((_, _, _, color)) = blame_info {
                return Line::from(vec![
                    Span::styled(" ".repeat(blame_width), blame_style(*color, theme)),
                    Span::raw(" ".repeat(width - blame_width)),
                ]);
            }
        }
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
            spans.push(Span::styled(text, blame_style(*color, theme)));
        } else {
            spans.push(Span::raw(" ".repeat(blame_width)));
        }
    }

    // Fold column: `-` heads an open fold, `+` a closed one, `|` inside.
    if fold_width > 0 {
        let cells: String = editor
            .fold_gutter_cells(line_idx, fold_width)
            .into_iter()
            .map(|mark| mark.glyph())
            .collect();
        let has_mark = cells.chars().any(|c| c != ' ');
        spans.push(Span::styled(
            cells,
            Style::default().fg(if has_mark {
                Color::DarkGray
            } else {
                Color::Reset
            }),
        ));
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

    // Sign priority: breakpoint+exec > breakpoint > execution line > diagnostics >
    // walkthrough focus > agent edits > git. The walkthrough marker makes the
    // explained block visible even on blank or very short lines.
    let line_1based = (line_idx + 1) as u64;
    let breakpoint = editor.breakpoint_marker_at(line_1based);
    let has_breakpoint = breakpoint.is_some();
    let is_exec_line = editor.execution_line_in_current_buffer() == Some(line_1based);

    let buffer_id = buffer.id();
    let is_agent_edit = editor
        .ai_chat_state()
        .map(|c| c.agent_edits.is_line_modified(buffer_id, line_idx))
        .unwrap_or(false);
    let is_walkthrough_line = line_is_in_walkthrough(ctx.walkthrough_range, line_idx);

    use ovim_core::editor::BreakpointMarker;
    let (bp_glyph, bp_exec_glyph, bp_color) = match breakpoint {
        Some(BreakpointMarker::Disabled) => ("○ ", "○▶", Color::DarkGray),
        Some(BreakpointMarker::Conditional) => ("◆ ", "◆▶", Color::Red),
        _ => ("● ", "●▶", Color::Red),
    };
    let (sign_text, sign_color) = if has_breakpoint && is_exec_line {
        (bp_exec_glyph, bp_color)
    } else if has_breakpoint {
        (bp_glyph, bp_color)
    } else if is_exec_line {
        ("▶ ", Color::Yellow)
    } else if !line_diagnostics.is_empty() {
        let severity = line_diagnostics[0].severity;
        get_diagnostic_sign_style(severity)
    } else if is_walkthrough_line {
        ("▎ ", WALKTHROUGH_GUTTER_FG)
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

// ---------------------------------------------------------------------------
// Unified decoration rendering
// ---------------------------------------------------------------------------

use ovim_core::editor::decoration::{
    Decoration, DecorationPlacement, DecorationStyle as DecStyle, ProjectedDecorations,
};
use ovim_core::editor::ProjectedDiagnostics;

/// Per-line render cache fingerprint: the projected EOL/inline decorations
/// plus the line's full diagnostic set (ranges + severities). The underline
/// squiggle is baked into cached rows and the diagnostic set can change
/// without a buffer edit (save → republish), so the decoration hash alone —
/// which only sees the line's single best-severity EOL message — is not
/// enough to invalidate. (OV-00329)
fn line_decoration_cache_hash(
    decorations: &ProjectedDecorations,
    diagnostics: &ProjectedDiagnostics,
    line_idx: usize,
) -> u64 {
    decorations
        .line_hash(line_idx)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ diagnostics.line_hash(line_idx)
}

/// Convert a `DecorationStyle` (framework-independent) to a ratatui `Style`.
fn decoration_to_ratatui_style(ds: &DecStyle) -> Style {
    let mut style = Style::default();
    if let Some(fg) = &ds.fg {
        style = style.fg(ovim_color_to_ratatui(*fg));
    }
    if let Some(bg) = &ds.bg {
        style = style.bg(ovim_color_to_ratatui(*bg));
    }
    if ds.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if ds.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if ds.underline {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

fn ovim_color_to_ratatui(c: ovim_core::color::Color) -> Color {
    use ovim_core::color::Color as C;
    match c {
        C::Black => Color::Black,
        C::Red => Color::Red,
        C::Green => Color::Green,
        C::Yellow => Color::Yellow,
        C::Blue => Color::Blue,
        C::Magenta => Color::Magenta,
        C::Cyan => Color::Cyan,
        C::White => Color::White,
        C::DarkGray => Color::DarkGray,
        C::LightRed => Color::LightRed,
        C::LightGreen => Color::LightGreen,
        C::LightYellow => Color::LightYellow,
        C::LightBlue => Color::LightBlue,
        C::LightMagenta => Color::LightMagenta,
        C::LightCyan => Color::LightCyan,
        C::Gray => Color::Gray,
        C::Rgb(r, g, b) => Color::Rgb(r, g, b),
        C::Indexed(i) => Color::Indexed(i),
        C::Reset => Color::Reset,
    }
}

/// Truncate a rendered line to `max_width` display columns.
///
/// Walks spans left-to-right, keeping whole characters that fit.  Partial
/// spans at the boundary are split so the total display width is exactly
/// `max_width` (or less if the last character is wide).
fn truncate_line_to_width(line: &mut Line<'static>, max_width: usize) {
    let mut total: usize = 0;
    let mut keep_spans = 0;

    // First pass: find the truncation point.
    let mut split_at: Option<(usize, usize)> = None; // (span_index, budget_cols)
    for (i, span) in line.spans.iter().enumerate() {
        let span_width: usize = span
            .content
            .graphemes(true)
            .map(grapheme_display_width)
            .sum();
        if total + span_width <= max_width {
            total += span_width;
            keep_spans += 1;
        } else {
            split_at = Some((i, max_width - total));
            break;
        }
    }

    // Second pass: apply truncation.
    if let Some((span_idx, budget)) = split_at {
        let style = line.spans[span_idx].style;
        let mut kept = String::new();
        let mut used = 0;
        for grapheme in line.spans[span_idx].content.graphemes(true) {
            let w = grapheme_display_width(grapheme);
            if used + w > budget {
                break;
            }
            kept.push_str(grapheme);
            used += w;
        }
        line.spans.truncate(keep_spans);
        if !kept.is_empty() {
            line.spans.push(Span::styled(kept, style));
        }
    }
    // All spans fit — nothing to truncate.
}

/// Apply inline decorations to a rendered line by splicing styled spans.
///
/// Decorations are inserted right-to-left (highest char_idx first) so earlier
/// insertions don't shift the positions of later ones.
fn apply_inline_decorations(
    line: &mut Line<'static>,
    decorations: &[&Decoration],
    char_mapping: &[usize],
    h_offset: usize,
    wrap: bool,
    line_start_offset: usize,
) {
    if decorations.is_empty() {
        return;
    }

    // Sort right-to-left by char_offset
    let mut sorted: Vec<&&Decoration> = decorations.iter().collect();
    sorted.sort_by(|a, b| {
        let a_off = a.placement.char_offset();
        let b_off = b.placement.char_offset();
        b_off.cmp(&a_off) // reverse order
    });

    for dec in sorted {
        // Derive line-relative char_idx from absolute char_offset.
        let char_idx = match &dec.placement {
            DecorationPlacement::Inline { char_offset } => {
                char_offset.saturating_sub(line_start_offset)
            }
            _ => continue,
        };

        // Map through char_mapping (handles tab expansion)
        let expanded_col = if char_idx < char_mapping.len() {
            char_mapping[char_idx]
        } else if !char_mapping.is_empty() {
            *char_mapping.last().unwrap() + 1
        } else {
            char_idx
        };

        // Adjust for horizontal scroll in nowrap mode
        let insert_col = if !wrap {
            if expanded_col < h_offset {
                continue;
            }
            expanded_col - h_offset
        } else {
            expanded_col
        };

        let style = decoration_to_ratatui_style(&dec.style);

        // Walk spans to find insertion point, then split and insert
        let mut char_count = 0;
        let mut span_idx = 0;
        let mut found = false;

        while span_idx < line.spans.len() {
            let span_chars: usize = line.spans[span_idx].content.chars().count();
            if char_count + span_chars > insert_col {
                let offset_in_span = insert_col - char_count;
                let content = line.spans[span_idx].content.to_string();
                let span_style = line.spans[span_idx].style;

                let before: String = content.chars().take(offset_in_span).collect();
                let after: String = content.chars().skip(offset_in_span).collect();

                line.spans.remove(span_idx);
                let mut insert_at = span_idx;
                if !before.is_empty() {
                    line.spans
                        .insert(insert_at, Span::styled(before, span_style));
                    insert_at += 1;
                }
                line.spans
                    .insert(insert_at, Span::styled(dec.text.clone(), style));
                insert_at += 1;
                if !after.is_empty() {
                    line.spans
                        .insert(insert_at, Span::styled(after, span_style));
                }
                found = true;
                break;
            } else if char_count + span_chars == insert_col {
                line.spans
                    .insert(span_idx + 1, Span::styled(dec.text.clone(), style));
                found = true;
                break;
            }
            char_count += span_chars;
            span_idx += 1;
        }

        if !found {
            line.spans.push(Span::styled(dec.text.clone(), style));
        }
    }
}

/// Width of the gap (in columns) between rendered code and an EOL diagnostic.
const EOL_DIAG_GAP: usize = 2;

/// Truncate `text` to at most `max_chars` characters, appending `...` when
/// truncation happens. If `max_chars` is too small to fit even the ellipsis,
/// returns whatever prefix fits with no marker.
///
/// Note: char count, not display width — a wide-char (CJK, emoji) message
/// can render up to 2x the budget. Pre-existing behavior; sharpening this
/// is a separate concern.
fn fit_with_ellipsis(text: &str, max_chars: usize) -> String {
    if text.graphemes(true).count() <= max_chars {
        return text.to_string();
    }
    if max_chars < 3 {
        return text.graphemes(true).take(max_chars).collect();
    }
    let prefix: String = text.graphemes(true).take(max_chars - 3).collect();
    format!("{prefix}...")
}

/// Total display width of all spans in a Line.
fn line_display_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|s| {
            s.content
                .graphemes(true)
                .map(grapheme_display_width)
                .sum::<usize>()
        })
        .sum()
}

/// Display width of the row's real content: trailing all-space spans with a
/// default style (the padding `split_line_into_rows` appends) are ignored.
/// Placement decisions must use this, not `line_display_width` — under soft
/// wrap every row arrives padded to `text_width`, and measuring the padding
/// made `place_eol_on_line` treat every short line as "full", pushing its
/// diagnostic to the far screen edge instead of next to the code.
fn content_display_width(line: &Line<'_>) -> usize {
    let mut spans = line.spans.as_slice();
    while let Some(last) = spans.last() {
        if last.content.chars().all(|c| c == ' ') && last.style == Style::default() {
            spans = &spans[..spans.len() - 1];
        } else {
            break;
        }
    }
    spans
        .iter()
        .map(|s| {
            s.content
                .graphemes(true)
                .map(grapheme_display_width)
                .sum::<usize>()
        })
        .sum()
}

/// Pad the line with trailing space spans until it reaches `target_width`.
/// No-op if the line already meets or exceeds `target_width`.
fn pad_line_to(line: &mut Line<'static>, target_width: usize) {
    let width = line_display_width(line);
    if width < target_width {
        line.spans.push(Span::raw(" ".repeat(target_width - width)));
    }
}

/// Pad the line with trailing spaces carrying `color` as their background, so
/// a diff row's tint reaches the right edge of the viewport.
fn pad_line_to_styled(line: &mut Line<'static>, target_width: usize, color: Color) {
    let width = line_display_width(line);
    if width < target_width {
        line.spans.push(Span::styled(
            " ".repeat(target_width - width),
            Style::default().bg(color),
        ));
    }
}

/// Where an EOL decoration should be placed within a rendered row.
#[derive(Debug, Clone, Copy)]
enum EolPlacement {
    /// Append the diagnostic right after the row's existing content,
    /// pad the row to `text_width`. Used when the line lives inside a
    /// single budget: non-centered mode, single-row no-overflow case.
    Append { text_width: usize },
    /// Clip the row at `code_box_width` (no bleed past the box), anchor
    /// the diagnostic immediately after the rendered code (so it sits
    /// close to the line, not at the far edge), and pad to `render_width`.
    /// Used in centered (textwidth) mode where lines render into a wider
    /// rect than the code-box: code stays inside the code-box, but the
    /// diagnostic is free to extend into the right margin.
    AtBoxEdge {
        code_box_width: usize,
        render_width: usize,
    },
}

/// Apply end-of-line decorations to a rendered row.
///
/// Strips trailing padding, appends each decoration's styled text (with a
/// `EOL_DIAG_GAP`-column gap), truncates to fit, and re-pads. The exact
/// anchor and final width depend on `placement` — see [`EolPlacement`].
///
/// `AtBoxEdge` performs its clip + anchor + pad work even when there are
/// no decorations, so callers can use it to enforce no-bleed geometry on
/// every row of a wrapped line in centered mode. `Append` short-circuits
/// when there's nothing to do (caller has already padded to text_width).
fn apply_eol_decorations(
    row: &mut Line<'static>,
    decorations: &[&Decoration],
    placement: EolPlacement,
) {
    // Append with no decorations: caller's padding already handles this.
    if decorations.is_empty() && matches!(placement, EolPlacement::Append { .. }) {
        return;
    }

    // Remove trailing padding spans so we know where the code actually ends.
    while let Some(last) = row.spans.last() {
        if last.content.chars().all(|c| c == ' ') && last.style == Style::default() {
            row.spans.pop();
        } else {
            break;
        }
    }

    // Resolve where the diagnostic anchors and what we pad to. AtBoxEdge
    // also clips any content past the code-box edge so hints/code don't
    // bleed into the diagnostic margin, then anchors the diagnostic right
    // after the (clipped) rendered code — short lines get the diagnostic
    // close, long lines (clipped at code_box_width) get it at the box edge.
    let (diag_start, final_width) = match placement {
        EolPlacement::Append { text_width } => (line_display_width(row), text_width),
        EolPlacement::AtBoxEdge {
            code_box_width,
            render_width,
        } => {
            truncate_line_to_width(row, code_box_width);
            (line_display_width(row), render_width)
        }
    };

    // Pad the row up to the anchor (extends short lines to a consistent column).
    pad_line_to(row, diag_start);

    // Append the diagnostic if we have one and there's room for gap + ≥1 char.
    let remaining = final_width.saturating_sub(diag_start);
    if !decorations.is_empty() && remaining >= EOL_DIAG_GAP + 4 {
        // First decoration wins (already priority-sorted). The message may
        // use everything up to the row edge: the space right of the code is
        // otherwise unused, and diagnostics are exactly what the user wants
        // to read there.
        let dec = &decorations[0];
        let msg = fit_with_ellipsis(&dec.text, remaining - EOL_DIAG_GAP);
        let style = decoration_to_ratatui_style(&dec.style);
        row.spans.push(Span::raw(" ".repeat(EOL_DIAG_GAP)));
        row.spans.push(Span::styled(msg, style));
    }

    pad_line_to(row, final_width);
}

/// Place an EOL decoration on a single rendered line, choosing the right
/// strategy from `(text_width, render_width, line_width, has_decs)`:
///
/// - **Centered (render_width > text_width)** → `AtBoxEdge`, which clips the
///   line at the code-box edge (no bleed into the margin) and anchors the
///   diagnostic immediately after the rendered code. Short lines get the
///   diagnostic close; lines that reach (or exceed) the box edge get the
///   diagnostic at the box edge — the message is free to extend into the
///   right margin in both cases.
/// - **Non-centered, line + hints overflow text_width with decs present**
///   → `overlay_eol_decoration_at_edge`, which steals the rightmost columns
///   so the diagnostic stays visible.
/// - **Non-centered, line fits or no decs** → `Append`, the default
///   "diagnostic floats after code" behavior.
fn place_eol_on_line(
    line: &mut Line<'static>,
    eol_decs: &[&Decoration],
    text_width: usize,
    render_width: usize,
) {
    if render_width > text_width {
        apply_eol_decorations(
            line,
            eol_decs,
            EolPlacement::AtBoxEdge {
                code_box_width: text_width,
                render_width,
            },
        );
    } else if !eol_decs.is_empty() && content_display_width(line) >= text_width {
        overlay_eol_decoration_at_edge(line, eol_decs, text_width);
    } else {
        apply_eol_decorations(line, eol_decs, EolPlacement::Append { text_width });
    }
}

/// Place EOL decorations across the visual rows of a wrapped line. The
/// last row gets the diagnostic via [`place_eol_on_line`]; in centered
/// mode every other row is clipped + padded to `render_width` so they
/// don't bleed into the diagnostic margin.
fn place_eol_on_visual_rows(
    rows: &mut [Line<'static>],
    eol_decs: &[&Decoration],
    text_width: usize,
    render_width: usize,
) {
    if rows.is_empty() {
        return;
    }
    let centered = render_width > text_width;
    let last = rows.len() - 1;
    for (i, row) in rows.iter_mut().enumerate() {
        if i == last {
            place_eol_on_line(row, eol_decs, text_width, render_width);
        } else if centered {
            apply_eol_decorations(
                row,
                &[],
                EolPlacement::AtBoxEdge {
                    code_box_width: text_width,
                    render_width,
                },
            );
        }
    }
}

/// Overlay an EOL decoration at the right edge of a line that already exceeds
/// `text_width` (typically because inline decorations pushed it beyond the
/// viewport). The diagnostic replaces the rightmost columns of the rendered
/// line so it's always visible without affecting cursor positioning.
fn overlay_eol_decoration_at_edge(
    line: &mut Line<'static>,
    decorations: &[&Decoration],
    text_width: usize,
) {
    if decorations.is_empty() || text_width < EOL_DIAG_GAP + 6 {
        return;
    }

    let dec = &decorations[0];
    let style = decoration_to_ratatui_style(&dec.style);

    // Budget: gap + message at the right edge, message capped at 1/3 of viewport.
    let msg = fit_with_ellipsis(&dec.text, text_width / 3);
    let overlay_width = EOL_DIAG_GAP
        + msg
            .graphemes(true)
            .map(grapheme_display_width)
            .sum::<usize>();

    // Truncate code to make room, pad up to the truncation point, then push
    // gap + styled message + final padding.
    let truncate_to = text_width.saturating_sub(overlay_width);
    truncate_line_to_width(line, truncate_to);
    pad_line_to(line, truncate_to);
    line.spans.push(Span::raw(" ".repeat(EOL_DIAG_GAP)));
    line.spans.push(Span::styled(msg, style));
    pad_line_to(line, text_width);
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
                .graphemes(true)
                .map(grapheme_display_width)
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

        // Split at grapheme boundaries: a VS16/ZWJ emoji is one 2-column
        // cell and must move to the next row whole, never mid-sequence.
        for grapheme in span.content.graphemes(true) {
            let ch_width = grapheme_display_width(grapheme);

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

            chunk.push_str(grapheme);
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

/// Rendered fragments carry source coordinates, so overlays only allocate for
/// the visible rows. The legacy line renderer remains the small-line oracle.
struct IndexedRowStyles<'a> {
    theme: &'a Theme,
    syntax: Vec<(Range<usize>, HighlightGroup)>,
    selected: Option<Range<usize>>,
    search: &'a [(usize, usize)],
    diagnostics: Vec<RemappedDiagnostic>,
    backgrounds: Vec<(Range<usize>, Color)>,
    cursorline: bool,
    yank: Option<Range<usize>>,
    ai: Option<Range<usize>>,
    links: &'a [ovim_core::markdown_conceal::ConcealedLink],
    bracket: Option<usize>,
    walkthrough: bool,
}

/// Resolve the original specificity/ordering rules with an interval sweep.
/// The result contains only style changes inside the visible byte interval.
fn resolve_indexed_syntax(
    highlights: &[(Range<usize>, HighlightGroup)],
    visible: Range<usize>,
) -> Vec<(Range<usize>, HighlightGroup)> {
    use std::{cmp::Reverse, collections::BinaryHeap};
    let mut events = Vec::new();
    let mut boundaries = vec![visible.start, visible.end];
    for (ordinal, (range, _)) in highlights.iter().enumerate() {
        let start = range.start.max(visible.start);
        let end = range.end.min(visible.end);
        if start < end {
            events.push((start, range.end.saturating_sub(range.start), ordinal, end));
            boundaries.extend([start, end]);
        }
    }
    events.sort_unstable();
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut active = BinaryHeap::new();
    let mut event = 0;
    let mut result: Vec<(Range<usize>, HighlightGroup)> = Vec::new();
    for pair in boundaries.windows(2) {
        let start = pair[0];
        while event < events.len() && events[event].0 <= start {
            let (_, size, ordinal, end) = events[event];
            active.push(Reverse((size, ordinal, end)));
            event += 1;
        }
        while active
            .peek()
            .is_some_and(|Reverse((_, _, end))| *end <= start)
        {
            active.pop();
        }
        if let Some(&Reverse((_, ordinal, _))) = active.peek() {
            let group = highlights[ordinal].1;
            if let Some((range, previous)) = result.last_mut() {
                if *previous == group && range.end == start {
                    range.end = pair[1];
                    continue;
                }
            }
            result.push((start..pair[1], group));
        }
    }
    result
}

impl IndexedRowStyles<'_> {
    fn style(&self, byte: usize, chars: Range<usize>, control: bool) -> Style {
        let overlaps = |range: &Range<usize>| range.start < chars.end && chars.start < range.end;
        let selected = self.selected.as_ref().is_some_and(overlaps);
        let search_pos = self.search.partition_point(|&(_, end)| end <= chars.start);
        let search = self
            .search
            .get(search_pos)
            .is_some_and(|&(start, end)| overlaps(&(start..end)));
        let syntax_pos = self.syntax.partition_point(|(range, _)| range.end <= byte);
        let syntax = self
            .syntax
            .get(syntax_pos)
            .filter(|(range, _)| range.contains(&byte))
            .map(|(_, group)| *group);
        let mut style = if selected {
            Style::default()
                .bg(crate::key_convert::convert_core_color(
                    self.theme.get_ui_color(UiGroup::Visual),
                ))
                .fg(Color::White)
        } else if search {
            Style::default()
                .bg(crate::key_convert::convert_core_color(
                    self.theme.get_ui_color(UiGroup::Search),
                ))
                .fg(Color::Black)
        } else if control {
            Style::default().fg(crate::key_convert::convert_core_color(
                self.theme.get_color(HighlightGroup::SpecialKey),
            ))
        } else if let Some(group) = syntax {
            let mut style = Style::default().fg(crate::key_convert::convert_core_color(
                self.theme.get_color(group),
            ));
            if matches!(
                group,
                HighlightGroup::MarkupHeading | HighlightGroup::MarkupBold
            ) {
                style = style.add_modifier(Modifier::BOLD);
            }
            if group == HighlightGroup::MarkupItalic {
                style = style.add_modifier(Modifier::ITALIC);
            }
            style
        } else {
            Style::default()
        };
        if !selected && !search {
            if let Some((_, color)) = self
                .backgrounds
                .iter()
                .rev()
                .find(|(range, _)| range.contains(&byte))
            {
                style = style.bg(*color);
            }
        }
        if let Some(diagnostic) = self
            .diagnostics
            .iter()
            .find(|d| overlaps(&(d.start..d.end)))
        {
            style = style
                .fg(diagnostic.color)
                .add_modifier(Modifier::UNDERLINED);
        }
        if self.cursorline
            && self.yank.is_none()
            && (style.bg.is_none() || style.bg == Some(Color::Reset))
        {
            style = style.bg(Color::Rgb(40, 40, 50));
        }
        if self.yank.as_ref().is_some_and(overlaps) {
            style = style.bg(Color::Rgb(60, 50, 20));
        }
        if self.ai.as_ref().is_some_and(overlaps) {
            style = style.bg(if self.walkthrough {
                WALKTHROUGH_SELECTION_BG
            } else {
                Color::Rgb(62, 70, 82)
            });
        }
        let link_pos = self
            .links
            .partition_point(|link| link.view_end <= chars.start);
        if self
            .links
            .get(link_pos)
            .is_some_and(|link| overlaps(&(link.view_start..link.view_end)))
        {
            style = style
                .fg(Color::Rgb(100, 149, 237))
                .add_modifier(Modifier::UNDERLINED);
        }
        if self.bracket.is_some_and(|col| chars.contains(&col)) {
            style = style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
        }
        style
    }
}

fn push_indexed_span(spans: &mut Vec<Span<'static>>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.style == style {
            last.content.to_mut().push_str(text);
            return;
        }
    }
    spans.push(Span::styled(text.to_owned(), style));
}

fn inline_fragment_text(text: &str, offset: usize, cells: usize) -> String {
    let mut position = 0;
    let mut result = String::new();
    let mut used = 0;
    for g in text.graphemes(true) {
        let width = grapheme_display_width(g);
        if position >= offset + cells {
            break;
        }
        if position >= offset && position + width <= offset + cells {
            result.push_str(g);
            used += width;
        } else if position + width > offset && position < offset + cells {
            let clipped = (position + width).min(offset + cells) - position.max(offset);
            result.push_str(&" ".repeat(clipped));
            used += clipped;
        }
        position += width;
    }
    if used < cells {
        result.push_str(&" ".repeat(cells - used));
    }
    result
}

fn render_indexed_fragments(
    fragments: &[LayoutFragment],
    styles: &IndexedRowStyles<'_>,
    inline: &[&Decoration],
) -> Line<'static> {
    let mut spans = Vec::new();
    for fragment in fragments {
        match &fragment.kind {
            LayoutFragmentKind::InlineDecoration { index, cell_offset } => {
                if let Some(decoration) = inline.get(*index) {
                    let text = if fragment.text.is_empty() {
                        inline_fragment_text(&decoration.text, *cell_offset, fragment.cells)
                    } else {
                        fragment.text.clone()
                    };
                    push_indexed_span(
                        &mut spans,
                        &text,
                        decoration_to_ratatui_style(&decoration.style),
                    );
                } else {
                    push_indexed_span(&mut spans, &" ".repeat(fragment.cells), Style::default());
                }
            }
            LayoutFragmentKind::Padding => {
                push_indexed_span(&mut spans, &fragment.text, Style::default())
            }
            LayoutFragmentKind::Tab | LayoutFragmentKind::Control => {
                let style = fragment
                    .source
                    .as_ref()
                    .map(|source| {
                        styles.style(
                            source.bytes.start,
                            source.chars.clone(),
                            matches!(fragment.kind, LayoutFragmentKind::Control),
                        )
                    })
                    .unwrap_or_default();
                push_indexed_span(&mut spans, &fragment.text, style);
            }
            LayoutFragmentKind::Text => {
                if let Some(source) = &fragment.source {
                    let mut char_col = source.chars.start;
                    for (offset, grapheme) in fragment.text.grapheme_indices(true) {
                        let end = char_col + grapheme.chars().count();
                        let style = styles.style(source.bytes.start + offset, char_col..end, false);
                        push_indexed_span(&mut spans, grapheme, style);
                        char_col = end;
                    }
                }
            }
        }
    }
    Line::from(spans)
}

/// Renders the buffer content and returns the viewport start line.
pub fn render_buffer(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    layout: &BufferLayout,
    line_cache: &mut super::line_cache::LineRenderCache,
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
    // Visual sub-row offset within `start_line`: the first `top_skip` wrapped
    // rows of the top logical line are scrolled off the top edge. Only meaningful
    // under soft wrap (set to 0 otherwise below, once `has_wrap` is known).
    let mut top_skip = window_context
        .and_then(|ctx| ctx.scroll_subrow)
        .unwrap_or_else(|| editor.scroll_subrow());

    // Get horizontal viewport settings
    // Use window-specific horizontal offset if provided
    let h_offset = window_context
        .and_then(|ctx| ctx.horizontal_offset)
        .unwrap_or_else(|| editor.horizontal_offset());
    let wrap = editor.options.wrap;

    // Use layout-provided dimensions
    let line_num_width = layout.line_num_width;
    let gutter_width_u16 = layout.gutter_width as u16;

    // Split layout into gutter (left of buffer_area) and text_area (from
    // end of gutter to right edge of render_area). When render_area equals
    // buffer_area (the common case), text_area is exactly buffer_area
    // minus the gutter — same as before. In centered mode render_area
    // extends past buffer_area, giving text_area a wider rect that
    // includes the diagnostic margin.
    let render_area = layout.render_area;
    let render_right = render_area.x + render_area.width;
    let gutter_area = if layout.gutter_width > 0 {
        Some(Rect {
            x: area.x,
            y: area.y,
            width: gutter_width_u16,
            height: area.height,
        })
    } else {
        None
    };
    let text_area = Rect {
        x: area.x + gutter_width_u16,
        y: area.y,
        width: render_right.saturating_sub(area.x + gutter_width_u16),
        height: area.height,
    };

    // Get visual selection if in visual mode
    let visual_selection = if editor.mode().is_visual() {
        editor.visual_selection()
    } else {
        None
    };
    let has_code_walkthrough = editor.ai_chat_has_pending_code_explanation();
    let ai_selection = if has_code_walkthrough {
        editor.ai_state.active_selection.as_ref()
    } else {
        None
    };
    let walkthrough_range = if has_code_walkthrough {
        ai_selection.map(|selection| (selection.start_line, selection.end_line))
    } else {
        None
    };

    // Get current search if active
    let current_search = editor.search.current_search.as_ref();

    // Find matching bracket position if showmatch is enabled
    let bracket_positions: Option<((usize, usize), (usize, usize))> = if editor.options.showmatch {
        find_matching_bracket_position(buffer).map(|matching_pos| {
            (
                (
                    cursor.line(),
                    buffer
                        .line_index(cursor.line())
                        .grapheme_to_char(cursor.col())
                        .0,
                ),
                matching_pos,
            )
        })
    } else {
        None
    };

    // Build the visible text with syntax highlighting
    // Gutter lines are built inline to match wrap continuation rows
    let mut lines = Vec::new();
    let mut gutter_lines: Vec<Line<'static>> = Vec::new();
    let tab_width = editor.indent_options().tab_width;
    let cursorline = editor.options.cursorline;
    let cursor_line_idx = cursor.line();
    // `text_width` is the document code-box (fixed at the layout's setting,
    // i.e. textwidth in centered mode); `render_width` is how wide the line
    // actually renders into. They differ in centered mode by exactly the
    // diagnostic margin width.
    let text_width = layout.text_width;
    let render_width = layout.render_width();
    let blank_line = " ".repeat(render_width);
    // Non-focused split panes carry the index of their own wrap map (built at
    // their own width); the single-window / focused path uses the global one.
    let wrap_map = match window_context.and_then(|ctx| ctx.wrap_map_window_index) {
        Some(window_idx) => editor
            .window_manager()
            .and_then(|wm| wm.get_window(window_idx))
            .and_then(|w| w.wrap_map()),
        None => editor.wrap_map(),
    };
    let has_wrap = wrap && wrap_map.is_some();
    // Sub-row scroll only applies under soft wrap — the renderer can't begin
    // partway into a logical line otherwise.
    if !has_wrap {
        top_skip = 0;
    }
    // Indexed geometry seeks directly to a scrolled sub-row; never construct
    // the offscreen prefix just to discard it.
    let emit_budget = visible_lines;
    let mut visual_rows_used = 0;
    let buffer_id = buffer.id();
    let cache_frame = super::line_cache::LineCacheFrame {
        buffer_id,
        buffer_version: buffer.version(),
        highlight_generation: buffer.highlight_projection_generation(),
        h_offset,
        text_width,
        wrap,
        tab_width,
        markdown_conceal: editor.options.markdown_conceal,
    };
    line_cache.begin_frame(cache_frame);

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

    let gutter_ctx = GutterContext {
        editor,
        buffer,
        theme,
        line_num_width,
        cursor_line: cursor_line_idx,
        blame_width,
        fold_width: layout.fold_width,
        walkthrough_range,
    };

    let is_md_file = buffer
        .file_path()
        .map(|p| p.ends_with(".md"))
        .unwrap_or(false);

    // Reusable scratch buffers for shift_highlights_for_viewport — avoids
    // allocating two Vec<usize> per visible line per frame.
    let mut hl_shift_buffers = HighlightShiftBuffers::new();

    // The branch diff review paints added/removed backgrounds per row; every
    // other buffer skips this entirely.
    let diff_review_tints = editor
        .diff_review()
        .filter(|state| state.buffer_id == editor.buffer().id());
    let diff_tint_color = |added: bool| {
        crate::key_convert::convert_core_color(theme.get_ui_color(if added {
            UiGroup::DiffAddedBg
        } else {
            UiGroup::DiffRemovedBg
        }))
    };

    // Project every decoration through the edit log ONCE per render. Per-line
    // lookups in the loop below then read from a line-keyed map instead of
    // re-scanning `iter_all()` and re-projecting each decoration.
    let projected_decorations = if editor.ai_code_explanation_is_presenting_snapshot() {
        // Walkthrough code pages render an immutable virtual snapshot, not the
        // live LSP document. Reusing the editor-global decoration map here can
        // attach diagnostics and inlay hints from a different document state to
        // coincidentally matching lines in the snapshot.
        Default::default()
    } else {
        editor.decorations.project_all(rope, buffer.edit_log())
    };
    // Raw diagnostics projected through the same edit log, once per frame:
    // the squiggle, gutter sign, and echo must land on the same line as the
    // projected EOL virtual text above. (OV-00328)
    let projected_diagnostics = if editor.ai_code_explanation_is_presenting_snapshot() {
        ovim_core::editor::ProjectedDiagnostics::default()
    } else {
        editor.project_diagnostics()
    };

    let mut line_idx = start_line;
    while line_idx < line_count && visual_rows_used < emit_budget {
        // Lines inside a closed fold are not drawn: skip the whole body.
        if let Some((start, end)) = buffer.fold_manager().closed_fold_at(line_idx) {
            if line_idx > start {
                line_idx = end + 1;
                continue;
            }
        }
        if line_idx < rope.len_lines() {
            // --- Cache check: try to reuse a previously rendered stable line ---
            // Determine upfront if this line has transient overlays that prevent caching.
            let has_visual_on_line = visual_selection
                .map(|((sl, _), (el, _))| line_idx >= sl && line_idx <= el)
                .unwrap_or(false);
            let is_cursor_line_early = cursorline && line_idx == cursor_line_idx;
            let is_cursor_line_for_conceal =
                line_idx == cursor_line_idx && editor.options.markdown_conceal && is_md_file;
            let has_yank_flash = editor
                .yank_flash()
                .is_some_and(|f| f.contains_line(line_idx));
            let line_diagnostics_early = projected_diagnostics.for_line(line_idx);
            let has_bracket = bracket_positions
                .is_some_and(|((l1, _), (l2, _))| line_idx == l1 || line_idx == l2);
            let has_search = current_search.is_some();
            let has_ai_selection_on_line = ai_selection
                .map(|selection| line_idx >= selection.start_line && line_idx <= selection.end_line)
                .unwrap_or(false);
            let is_stable = !has_visual_on_line
                && !is_cursor_line_early
                && !is_cursor_line_for_conceal
                && !has_yank_flash
                && !has_bracket
                && !has_search
                && !has_ai_selection_on_line;

            let md_conceal = editor.options.markdown_conceal;
            // Per-line decoration hash from the per-frame projection. Lets the
            // line cache invalidate only the lines whose decorations actually
            // changed (vs. the previous global generation counter that wiped
            // every cached line on any LSP push).
            let dec_hash = line_decoration_cache_hash(
                &projected_decorations,
                &projected_diagnostics,
                line_idx,
            );
            let cache_key = cache_frame.key(line_idx, dec_hash);

            // Long logical lines and sub-row viewports render from shared
            // source-aware fragments. All temporary strings/style runs are
            // bounded by the visible rows, including deeply scrolled lines.
            let source_index = buffer.line_index(line_idx);
            if source_index.len_bytes() > 4096 || (line_idx == start_line && top_skip > 0) {
                let line_start_char = rope.line_to_char(line_idx);
                let line_decorations = projected_decorations.for_line(line_idx);
                let mut inline: Vec<&Decoration> = line_decorations
                    .iter()
                    .filter(|d| matches!(d.placement, DecorationPlacement::Inline { .. }))
                    .collect();
                inline.sort_by_key(|d| (d.placement.char_offset(), d.priority));
                let eol: Vec<&Decoration> = line_decorations
                    .iter()
                    .filter(|d| matches!(d.placement, DecorationPlacement::EndOfLine { .. }))
                    .collect();
                let map_layout = has_wrap
                    .then(|| wrap_map.and_then(|map| map.line_layout(line_idx)))
                    .flatten();
                let cached_indexed = if map_layout.is_none() {
                    Some(line_cache.indexed_line(
                        buffer_id,
                        line_idx,
                        source_index.clone(),
                        text_width,
                        tab_width,
                        is_md_file && md_conceal && !is_cursor_line_for_conceal,
                        dec_hash,
                        &inline,
                        line_start_char,
                    ))
                } else {
                    None
                };
                let indexed_layout =
                    map_layout.unwrap_or_else(|| &cached_indexed.as_ref().unwrap().layout);
                let transform = if map_layout.is_some() {
                    wrap_map.and_then(|map| map.line_transform(line_idx))
                } else {
                    cached_indexed
                        .as_ref()
                        .and_then(|cached| cached.transform.as_deref())
                };
                let links = if map_layout.is_some() {
                    wrap_map
                        .map(|map| map.line_concealed_links(line_idx))
                        .unwrap_or(&[])
                } else {
                    cached_indexed
                        .as_ref()
                        .map(|cached| cached.links.as_ref())
                        .unwrap_or(&[])
                };
                let view_index = indexed_layout.line();
                let source_byte_to_view_char = |byte: usize| -> usize {
                    transform
                        .map(|map| {
                            map.src_to_view
                                .get(byte)
                                .copied()
                                .unwrap_or(view_index.len_chars())
                        })
                        .unwrap_or_else(|| source_index.byte_to_char(byte))
                };
                let source_char_to_view = |col: usize| -> usize {
                    source_byte_to_view_char(source_index.char_to_byte(col))
                };
                let source_byte_to_view_byte = |byte: usize| -> usize {
                    if transform.is_some() {
                        view_index.char_to_byte(source_byte_to_view_char(byte))
                    } else {
                        byte.min(view_index.len_bytes())
                    }
                };
                let mut precedes = false;
                let mut extends = false;
                let mut content_budget = text_width;
                let rows = if has_wrap {
                    let first = if line_idx == start_line { top_skip } else { 0 };
                    indexed_layout
                        .row_fragments(first..first.saturating_add(emit_budget - visual_rows_used))
                } else {
                    let total = indexed_layout
                        .display_range_for_row(indexed_layout.row_count().saturating_sub(1))
                        .map(|range| range.end)
                        .unwrap_or(0);
                    let requested_start = if total <= text_width { 0 } else { h_offset };
                    let first = indexed_layout.fragments_for_display_range(
                        requested_start..requested_start.saturating_add(1),
                    );
                    let actual_start = first
                        .first()
                        .map(|fragment| fragment.display_start)
                        .unwrap_or(requested_start);
                    precedes = total > text_width && h_offset > 0;
                    let available = text_width.saturating_sub(usize::from(precedes));
                    extends = total.saturating_sub(actual_start) > available
                        && (!precedes || text_width > 1);
                    content_budget = available.saturating_sub(usize::from(extends));
                    vec![LayoutRow {
                        index: 0,
                        display_start: actual_start,
                        display_end: actual_start.saturating_add(content_budget).min(total),
                        fragments: indexed_layout.fragments_for_display_range(
                            actual_start..actual_start.saturating_add(content_budget),
                        ),
                    }]
                };
                let visible_byte_start = rows
                    .iter()
                    .flat_map(|row| &row.fragments)
                    .filter_map(|fragment| {
                        fragment.source.as_ref().map(|source| source.bytes.start)
                    })
                    .min()
                    .unwrap_or(0);
                let visible_byte_end = rows
                    .iter()
                    .flat_map(|row| &row.fragments)
                    .filter_map(|fragment| fragment.source.as_ref().map(|source| source.bytes.end))
                    .max()
                    .unwrap_or(0);
                let source_byte_window = if let Some(mapped) = transform {
                    let view_start = view_index.byte_to_char(visible_byte_start);
                    let view_end = view_index.byte_to_char(visible_byte_end);
                    let after_start = mapped.src_to_view.partition_point(|&col| col <= view_start);
                    let prior = mapped
                        .src_to_view
                        .get(after_start.saturating_sub(1))
                        .copied()
                        .unwrap_or(0);
                    let start = mapped.src_to_view.partition_point(|&col| col < prior);
                    let end = mapped.src_to_view.partition_point(|&col| col < view_end);
                    start..end.min(source_index.len_bytes())
                } else {
                    visible_byte_start..visible_byte_end
                };
                let mapped_syntax: Vec<_> = buffer
                    .highlights_in_byte_range(line_idx, source_byte_window)
                    .iter()
                    .map(|(range, group)| {
                        (
                            source_byte_to_view_byte(range.start)
                                ..source_byte_to_view_byte(range.end),
                            *group,
                        )
                    })
                    .collect();
                let syntax =
                    resolve_indexed_syntax(&mapped_syntax, visible_byte_start..visible_byte_end);
                let selected = visual_selection.and_then(|((sl, sc), (el, ec))| {
                    if line_idx < sl || line_idx > el {
                        return None;
                    }
                    let block = editor.mode() == crate::mode::Mode::VisualBlock;
                    let start = if block || line_idx == sl { sc } else { 0 };
                    let end = if block || line_idx == el {
                        ec.saturating_add(1)
                    } else {
                        source_index.grapheme_count()
                    };
                    Some(
                        source_char_to_view(source_index.grapheme_to_char(GraphemeCol(start)).0)
                            ..source_char_to_view(
                                source_index.grapheme_to_char(GraphemeCol(end)).0,
                            ),
                    )
                });
                let search = current_search
                    .map(|search| search.find_all_in_index(view_index))
                    .unwrap_or_default();
                let diagnostics = projected_diagnostics
                    .covering(line_idx)
                    .filter_map(|diagnostic| {
                        let start = if line_idx == diagnostic.range.start.line as usize {
                            source_index.utf16_to_char(diagnostic.range.start.character as usize)
                        } else {
                            0
                        };
                        let end = if line_idx == diagnostic.range.end.line as usize {
                            source_index.utf16_to_char(diagnostic.range.end.character as usize)
                        } else {
                            source_index.len_chars()
                        };
                        if start >= end {
                            return None;
                        }
                        let start = source_index
                            .grapheme_to_char(source_index.char_to_grapheme(CharCol(start)))
                            .0;
                        let last = source_index.char_to_grapheme(CharCol(end - 1));
                        let end = source_index.grapheme_to_char(GraphemeCol(last.0 + 1)).0;
                        let color = match diagnostic.severity {
                            Some(lsp_types::DiagnosticSeverity::WARNING) => Color::Yellow,
                            Some(lsp_types::DiagnosticSeverity::INFORMATION) => Color::Cyan,
                            Some(lsp_types::DiagnosticSeverity::HINT) => Color::Gray,
                            _ => Color::Red,
                        };
                        Some(RemappedDiagnostic {
                            start: source_char_to_view(start),
                            end: source_char_to_view(end),
                            color,
                        })
                    })
                    .collect();
                let backgrounds = diff_review_tints
                    .map(|state| {
                        state
                            .line_tints(line_idx)
                            .into_iter()
                            .map(|(range, added)| {
                                (
                                    source_byte_to_view_byte(range.start)
                                        ..source_byte_to_view_byte(range.end),
                                    diff_tint_color(added),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let trailing_background = diff_review_tints
                    .and_then(|state| state.line_trailing_tint(line_idx))
                    .map(diff_tint_color);
                let yank = editor
                    .yank_flash()
                    .filter(|flash| flash.contains_line(line_idx))
                    .map(|flash| {
                        flash
                            .col_range_for_line(line_idx)
                            .map(|(start, end)| {
                                source_char_to_view(start)
                                    ..source_char_to_view(end.saturating_add(1))
                            })
                            .unwrap_or(0..view_index.len_chars())
                    });
                let ai = ai_selection
                    .filter(|selection| {
                        line_idx >= selection.start_line && line_idx <= selection.end_line
                    })
                    .map(|selection| {
                        let start = if selection.selection_mode == crate::mode::Mode::VisualLine
                            || line_idx != selection.start_line
                        {
                            0
                        } else {
                            selection
                                .start_char
                                .saturating_sub(line_start_char)
                                .min(source_index.len_chars())
                        };
                        let end = if selection.selection_mode == crate::mode::Mode::VisualLine
                            || line_idx != selection.end_line
                        {
                            source_index.len_chars()
                        } else {
                            selection
                                .end_char
                                .saturating_sub(line_start_char)
                                .min(source_index.len_chars())
                        };
                        source_char_to_view(start)..source_char_to_view(end)
                    });
                let bracket = bracket_positions.and_then(|((l1, c1), (l2, c2))| {
                    if line_idx == l1 {
                        Some(source_char_to_view(c1))
                    } else if line_idx == l2 {
                        Some(source_char_to_view(c2))
                    } else {
                        None
                    }
                });
                let styles = IndexedRowStyles {
                    theme,
                    syntax,
                    selected,
                    search: &search,
                    diagnostics,
                    backgrounds,
                    cursorline: is_cursor_line_early,
                    yank,
                    ai,
                    links,
                    bracket,
                    walkthrough: walkthrough_range.is_some(),
                };
                for row in rows {
                    let mut rendered = render_indexed_fragments(&row.fragments, &styles, &inline);
                    if !has_wrap {
                        truncate_line_to_width(&mut rendered, content_budget);
                        pad_line_to(&mut rendered, content_budget);
                        if precedes {
                            rendered.spans.insert(0, Span::raw("<"));
                        }
                        if extends {
                            rendered.spans.push(Span::raw(">"));
                        }
                        place_eol_on_line(&mut rendered, &eol, text_width, render_width);
                    } else if row.index + 1 == indexed_layout.row_count() {
                        place_eol_on_line(&mut rendered, &eol, text_width, render_width);
                    } else if render_width > text_width {
                        apply_eol_decorations(
                            &mut rendered,
                            &[],
                            EolPlacement::AtBoxEdge {
                                code_box_width: text_width,
                                render_width,
                            },
                        );
                    }
                    if let Some(color) = trailing_background {
                        pad_line_to_styled(&mut rendered, render_width, color);
                    } else {
                        pad_line_to(&mut rendered, render_width);
                    }
                    if gutter_area.is_some() {
                        gutter_lines.push(build_gutter_line(
                            &gutter_ctx,
                            line_idx,
                            has_wrap && row.index > 0,
                            line_diagnostics_early,
                            blame_brackets
                                .as_ref()
                                .and_then(|brackets| brackets.get(line_idx - start_line)),
                        ));
                    }
                    lines.push(rendered);
                    visual_rows_used += 1;
                }
                line_idx += 1;
                continue;
            }

            if is_stable {
                if let Some(cached_line) = line_cache.get(&cache_key) {
                    let mut cached_line = cached_line.clone();
                    // Cached line has inline decorations but needs
                    // EOL decorations (diagnostics) applied fresh. The
                    // per-frame projection cache (built before the loop)
                    // avoids re-scanning every decoration here.
                    let eol_decs: Vec<&Decoration> = projected_decorations.eol_for_line(line_idx);

                    if has_wrap {
                        let mut visual_rows = split_line_into_rows(cached_line, text_width);
                        place_eol_on_visual_rows(
                            &mut visual_rows,
                            &eol_decs,
                            text_width,
                            render_width,
                        );
                        for (row_idx, row) in visual_rows.into_iter().enumerate() {
                            if visual_rows_used >= emit_budget {
                                break;
                            }
                            if gutter_area.is_some() {
                                gutter_lines.push(build_gutter_line(
                                    &gutter_ctx,
                                    line_idx,
                                    row_idx > 0,
                                    line_diagnostics_early,
                                    blame_brackets
                                        .as_ref()
                                        .and_then(|b| b.get(line_idx - start_line)),
                                ));
                            }
                            lines.push(row);
                            visual_rows_used += 1;
                        }
                    } else {
                        place_eol_on_line(&mut cached_line, &eol_decs, text_width, render_width);
                        match diff_review_tints.and_then(|state| state.line_trailing_tint(line_idx))
                        {
                            Some(added) => pad_line_to_styled(
                                &mut cached_line,
                                render_width,
                                diff_tint_color(added),
                            ),
                            None => pad_line_to(&mut cached_line, render_width),
                        }
                        if gutter_area.is_some() {
                            gutter_lines.push(build_gutter_line(
                                &gutter_ctx,
                                line_idx,
                                false,
                                line_diagnostics_early,
                                blame_brackets
                                    .as_ref()
                                    .and_then(|b| b.get(line_idx - start_line)),
                            ));
                        }
                        lines.push(cached_line);
                        visual_rows_used += 1;
                    }
                    line_idx += 1;
                    continue;
                }
            }

            // Visible content of the line (trailing terminator stripped).
            let line_text_original = ovim_core::display::line_content(rope, line_idx);

            // Optional markdown conceal (skip on cursor line so editing isn't blind)
            let (exp, concealed_links) =
                if is_md_file && editor.options.markdown_conceal && !is_cursor_line_for_conceal {
                    let spans = scan_markdown_conceal(&line_text_original);
                    if spans.is_empty() {
                        (
                            expand_tabs_with_mapping(&line_text_original, tab_width),
                            Vec::new(),
                        )
                    } else {
                        let transform = crate::ui::renderer::markdown_conceal::apply_conceal(
                            &line_text_original,
                            &spans,
                        );
                        let links = crate::ui::renderer::markdown_conceal::extract_concealed_links(
                            &spans, &transform,
                        );
                        (
                            compose_conceal_and_tabs(&line_text_original, &transform, tab_width),
                            links,
                        )
                    }
                } else {
                    (
                        expand_tabs_with_mapping(&line_text_original, tab_width),
                        Vec::new(),
                    )
                };
            let expanded_text = exp.text;
            let conceal_byte_map = exp.byte_mapping;
            let control_ranges = exp.control_ranges;
            let char_mapping = exp.char_mapping;

            let viewport =
                (!wrap).then(|| slice_horizontal_viewport(&expanded_text, h_offset, text_width));
            let precedes = viewport.as_ref().is_some_and(|view| view.precedes);
            let line_text = viewport
                .as_ref()
                .map(|view| view.text.as_str())
                .unwrap_or(&expanded_text);

            // Get syntax highlights for this line and remap them for expanded text
            let original_highlights = buffer.highlights_for_line(line_idx);
            let mut syntax_highlights = remap_highlights(&original_highlights, &conceal_byte_map);

            // Shift syntax highlights for horizontal viewport if nowrap
            if !wrap {
                syntax_highlights = shift_highlights_for_viewport(
                    &syntax_highlights,
                    &expanded_text,
                    line_text,
                    h_offset,
                    text_width,
                    precedes,
                    &mut hl_shift_buffers,
                );
            }

            // Diff review: added/removed background tints, in the same byte
            // space as the syntax highlights above.
            let mut background_ranges: Vec<(Range<usize>, Color)> = diff_review_tints
                .map(|state| {
                    state
                        .line_tints(line_idx)
                        .into_iter()
                        .map(|(range, added)| (range, diff_tint_color(added)))
                        .collect()
                })
                .unwrap_or_default();
            // A just-entered snippet placeholder reads as selected: typing
            // replaces it (see `Editor::snippet_placeholder_highlight`).
            if let Some((hl_line, start_col, end_col)) = editor.snippet_placeholder_highlight() {
                if hl_line == line_idx {
                    let mut offsets = line_text_original
                        .char_indices()
                        .map(|(byte, _)| byte)
                        .chain(std::iter::once(line_text_original.len()));
                    let start = offsets.clone().nth(start_col);
                    let end = offsets.nth(end_col);
                    if let (Some(start), Some(end)) = (start, end) {
                        background_ranges.push((start..end, Color::Rgb(62, 84, 140)));
                    }
                }
            }
            if !background_ranges.is_empty() {
                background_ranges = remap_highlights(&background_ranges, &conceal_byte_map);
                if !wrap {
                    background_ranges = shift_highlights_for_viewport(
                        &background_ranges,
                        &expanded_text,
                        line_text,
                        h_offset,
                        text_width,
                        precedes,
                        &mut hl_shift_buffers,
                    );
                }
            }
            // A tint that runs to the end of the line keeps going through the
            // padding, so a changed row reads as a full-width band. Derived
            // from the unsliced row so a cache hit pads identically.
            let trailing_background = diff_review_tints
                .and_then(|state| state.line_trailing_tint(line_idx))
                .map(diff_tint_color);

            // Check if we need special highlighting (visual selection or search)
            let has_visual_selection = has_visual_on_line;

            let search_matches = if let Some(search) = current_search {
                search.find_all_in_line(line_text)
            } else {
                Vec::new()
            };

            // Check if this is the cursor line and cursorline option is on
            let is_cursor_line = is_cursor_line_early;

            // Project this row's inclusive grapheme selection to a half-open
            // scalar range before conceal, tab expansion or viewport slicing.
            // In block mode every row needs its own conversion: equal grapheme
            // columns need not have equal scalar or display columns.
            let remapped_visual_selection = visual_selection.and_then(|((sl, sc), (el, ec))| {
                if line_idx < sl || line_idx > el {
                    return None;
                }
                let block = editor.mode() == crate::mode::Mode::VisualBlock;
                let start = if block || line_idx == sl { sc } else { 0 };
                let end = if block || line_idx == el {
                    ec.saturating_add(1)
                } else {
                    grapheme_count(&line_text_original)
                };
                let expand = |col| {
                    remap_char_col(
                        grapheme_to_char_col(&line_text_original, GraphemeCol(col)).0,
                        &char_mapping,
                    )
                };
                let range = expand(start)..expand(end);
                if let Some(viewport) = &viewport {
                    viewport.project_range(range)
                } else {
                    (!range.is_empty()).then_some(range)
                }
            });

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
                            line_text,
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

            // Underlines cover the complete span. Gutter signs and EOL
            // messages still use the diagnostics starting on this line.
            let remapped_diagnostics: Vec<RemappedDiagnostic> = projected_diagnostics
                .covering(line_idx)
                .filter_map(|diagnostic| {
                    let range = ovim_core::lsp::diagnostic_char_range(
                        diagnostic,
                        line_idx,
                        &line_text_original,
                    )?;
                    if range.is_empty() {
                        return None;
                    }
                    // A terminal cell cannot underline half a grapheme. Round
                    // outward before tabs/conceal so accents and ZWJ sequences
                    // never get split into differently styled spans.
                    let start = char_to_grapheme_col(&line_text_original, CharCol(range.start));
                    let last = char_to_grapheme_col(&line_text_original, CharCol(range.end - 1));
                    let start = grapheme_to_char_col(&line_text_original, start).0;
                    let end = grapheme_to_char_col(&line_text_original, GraphemeCol(last.0 + 1)).0;
                    let range =
                        remap_char_col(start, &char_mapping)..remap_char_col(end, &char_mapping);
                    let range = if let Some(viewport) = &viewport {
                        viewport.project_range(range)?
                    } else if range.is_empty() {
                        return None;
                    } else {
                        range
                    };
                    let color = match diagnostic.severity {
                        Some(lsp_types::DiagnosticSeverity::ERROR) => Color::Red,
                        Some(lsp_types::DiagnosticSeverity::WARNING) => Color::Yellow,
                        Some(lsp_types::DiagnosticSeverity::INFORMATION) => Color::Cyan,
                        Some(lsp_types::DiagnosticSeverity::HINT) => Color::Gray,
                        _ => Color::Red,
                    };
                    Some(RemappedDiagnostic {
                        start: range.start,
                        end: range.end,
                        color,
                    })
                })
                .collect();
            let has_diagnostics = !remapped_diagnostics.is_empty();
            let line_char_count = line_text_original.chars().count();
            let ai_selection_ranges = if let Some(selection) = ai_selection {
                if line_idx < selection.start_line || line_idx > selection.end_line {
                    Vec::new()
                } else {
                    let (start_col, end_col_inclusive) =
                        if selection.selection_mode == crate::mode::Mode::VisualLine {
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
                                    line_text,
                                    start_display.saturating_sub(h_offset) + offset_adj,
                                );
                                end_exclusive = display_col_to_char_idx(
                                    line_text,
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
            let yank_flash = editor.yank_flash().and_then(|flash| {
                if flash.contains_line(line_idx) {
                    Some(flash.col_range_for_line(line_idx))
                } else {
                    None
                }
            });

            // Inline decorations (inlay hints) and EOL decorations (LSP
            // diagnostics rendered as virtual text) both need the detailed
            // path: inline decs require `apply_inline_decorations` to splice
            // their spans into the rendered line, and EOL decs require the
            // EOL placement step that the simple path doesn't run. Use the
            // unprojected `for_line` here for a cheap BTreeMap lookup; the
            // detailed block does the projection.
            let any_decoration = !editor.decorations.for_line(line_idx).is_empty();

            // Always use character-by-character rendering if we have any highlighting
            let needs_detailed_rendering = has_visual_selection
                || !search_matches.is_empty()
                || !syntax_highlights.is_empty()
                || is_cursor_line
                || bracket_col.is_some()
                || has_diagnostics
                || yank_flash.is_some()
                || !ai_selection_ranges.is_empty()
                || !concealed_links.is_empty()
                || !background_ranges.is_empty()
                || any_decoration;

            if needs_detailed_rendering {
                let mut line = render_line_with_highlights(
                    theme,
                    line_text,
                    remapped_visual_selection,
                    &search_matches,
                    &syntax_highlights,
                    &remapped_diagnostics,
                    &control_ranges,
                    &background_ranges,
                );

                // Apply cursorline background if this is the cursor line
                if is_cursor_line && yank_flash.is_none() {
                    let cursorline_bg = Color::Rgb(40, 40, 50); // Subtle dark blue background
                    for span in &mut line.spans {
                        if span.style.bg.is_none() || span.style.bg == Some(Color::Reset) {
                            span.style = span.style.bg(cursorline_bg);
                        }
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

                // Preserve syntax colors while making an interactive walkthrough
                // more prominent than an ordinary AI-prompt selection.
                let ai_selection_bg = if walkthrough_range.is_some() {
                    WALKTHROUGH_SELECTION_BG
                } else {
                    Color::Rgb(62, 70, 82)
                };
                for (start_col, end_col) in &ai_selection_ranges {
                    apply_bg_to_column_range(&mut line, *start_col, *end_col, ai_selection_bg);
                }

                // Apply concealed link underline styling
                if !concealed_links.is_empty() {
                    let link_color = Color::Rgb(100, 149, 237); // Cornflower blue
                    for link in &concealed_links {
                        apply_fg_modifier_to_column_range(
                            &mut line,
                            link.view_start,
                            link.view_end,
                            link_color,
                            Modifier::UNDERLINED,
                        );
                    }
                }

                // Apply bracket highlighting
                if let Some(col) = bracket_col {
                    let bracket_style = Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD);
                    apply_style_at_column(&mut line, col, bracket_style);
                }

                // Apply decorations from the unified DecorationMap BEFORE
                // caching.  The cache key includes dec_gen so it misses when
                // decorations change.  Storing the decorated line ensures
                // cache-hit frames render the same decorations that cursor
                // positioning (inline_width_before) accounts for.
                //
                // Step E: read projected offsets from the per-frame
                // projection cache built before the loop.  In steady state
                // (accumulator keeps `source_version` current) this yields
                // the same result as the stored offsets; when Step F removes
                // the accumulator the projection becomes the sole source of
                // truth.
                let line_decorations = projected_decorations.for_line(line_idx);
                let inline_decs: Vec<&Decoration> = line_decorations
                    .iter()
                    .filter(|d| matches!(d.placement, DecorationPlacement::Inline { .. }))
                    .collect();
                let eol_decs: Vec<&Decoration> = line_decorations
                    .iter()
                    .filter(|d| matches!(d.placement, DecorationPlacement::EndOfLine { .. }))
                    .collect();

                if !inline_decs.is_empty() {
                    let line_start_offset = editor.buffer().rope().line_to_char(line_idx);
                    apply_inline_decorations(
                        &mut line,
                        &inline_decs,
                        &char_mapping,
                        h_offset,
                        wrap,
                        line_start_offset,
                    );
                }

                // Store in cache AFTER decorations so cache-hit frames
                // match cursor positioning.
                line_cache.put(cache_key, line.clone(), is_stable);

                // Soft wrap: split into visual rows if needed
                if has_wrap {
                    let mut visual_rows = split_line_into_rows(line, text_width);
                    place_eol_on_visual_rows(&mut visual_rows, &eol_decs, text_width, render_width);
                    for (row_idx, row) in visual_rows.into_iter().enumerate() {
                        if visual_rows_used >= emit_budget {
                            break;
                        }
                        if gutter_area.is_some() {
                            gutter_lines.push(build_gutter_line(
                                &gutter_ctx,
                                line_idx,
                                row_idx > 0,
                                line_diagnostics_early,
                                blame_brackets
                                    .as_ref()
                                    .and_then(|b| b.get(line_idx - start_line)),
                            ));
                        }
                        lines.push(row);
                        visual_rows_used += 1;
                    }
                } else {
                    // No wrap: in non-centered mode, ratatui clips overflow
                    // at text_area's right edge, so we don't truncate (cursor
                    // tracking would desync). In centered mode, AtBoxEdge
                    // explicitly clips at the code-box and pads into the
                    // diagnostic margin — `place_eol_on_line` picks the
                    // right strategy.
                    place_eol_on_line(&mut line, &eol_decs, text_width, render_width);
                    match trailing_background {
                        Some(color) => pad_line_to_styled(&mut line, render_width, color),
                        None => pad_line_to(&mut line, render_width),
                    }
                    if gutter_area.is_some() {
                        gutter_lines.push(build_gutter_line(
                            &gutter_ctx,
                            line_idx,
                            false,
                            line_diagnostics_early,
                            blame_brackets
                                .as_ref()
                                .and_then(|b| b.get(line_idx - start_line)),
                        ));
                    }
                    lines.push(line);
                    visual_rows_used += 1;
                }
            } else {
                // Simple rendering path (no highlighting) — always stable
                let simple_line = Line::from(line_text.to_string());
                line_cache.put(cache_key, simple_line, true);

                if has_wrap {
                    if line_text.is_empty() {
                        if gutter_area.is_some() {
                            gutter_lines.push(build_gutter_line(
                                &gutter_ctx,
                                line_idx,
                                false,
                                &[],
                                blame_brackets
                                    .as_ref()
                                    .and_then(|b| b.get(line_idx - start_line)),
                            ));
                        }
                        lines.push(Line::from(" ".repeat(render_width)));
                        visual_rows_used += 1;
                    } else {
                        // Split by display width, not char count — CJK/emoji
                        // characters are width 2 and would overflow the terminal
                        // row if counted as 1.
                        let mut chunk_idx = 0;
                        let mut row_text = String::new();
                        let mut row_width = 0;

                        for grapheme in line_text.graphemes(true) {
                            let ch_width = grapheme_display_width(grapheme);

                            if row_width + ch_width > text_width && !row_text.is_empty() {
                                // Flush current row
                                if visual_rows_used >= emit_budget {
                                    break;
                                }
                                if gutter_area.is_some() {
                                    gutter_lines.push(build_gutter_line(
                                        &gutter_ctx,
                                        line_idx,
                                        chunk_idx > 0,
                                        &[],
                                        blame_brackets
                                            .as_ref()
                                            .and_then(|b| b.get(line_idx - start_line)),
                                    ));
                                }
                                let pad = render_width.saturating_sub(row_width);
                                if pad > 0 {
                                    row_text.push_str(&" ".repeat(pad));
                                }
                                lines.push(Line::from(std::mem::take(&mut row_text)));
                                visual_rows_used += 1;
                                chunk_idx += 1;
                                row_width = 0;
                            }

                            row_text.push_str(grapheme);
                            row_width += ch_width;
                        }

                        // Flush the last row
                        if !row_text.is_empty() && visual_rows_used < emit_budget {
                            if gutter_area.is_some() {
                                gutter_lines.push(build_gutter_line(
                                    &gutter_ctx,
                                    line_idx,
                                    chunk_idx > 0,
                                    &[],
                                    blame_brackets
                                        .as_ref()
                                        .and_then(|b| b.get(line_idx - start_line)),
                                ));
                            }
                            let pad = render_width.saturating_sub(row_width);
                            if pad > 0 {
                                row_text.push_str(&" ".repeat(pad));
                            }
                            lines.push(Line::from(row_text));
                            visual_rows_used += 1;
                        }
                    }
                } else {
                    // No wrap: pad simple lines too
                    if gutter_area.is_some() {
                        gutter_lines.push(build_gutter_line(
                            &gutter_ctx,
                            line_idx,
                            false,
                            &[],
                            blame_brackets
                                .as_ref()
                                .and_then(|b| b.get(line_idx - start_line)),
                        ));
                    }
                    let line_display_len = unicode_width::UnicodeWidthStr::width(line_text);
                    let line_text = if line_display_len < render_width {
                        format!(
                            "{}{}",
                            line_text,
                            " ".repeat(render_width - line_display_len)
                        )
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
    if let Some(rail) = layout.scrollbar_area {
        let (total, top) = if has_wrap {
            wrap_map
                .map(|map| {
                    (
                        map.total_visual_lines(),
                        map.viewport_top_visual_row(start_line, top_skip),
                    )
                })
                .unwrap_or((line_count, start_line))
        } else {
            (line_count, start_line)
        };
        render_diff_scrollbar(frame, rail, total, top, theme);
    }

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
    visual_selection: Option<Range<usize>>,
    search_matches: &[(usize, usize)],
    syntax_highlights: &[(std::ops::Range<usize>, crate::syntax::HighlightGroup)],
    diagnostics: &[RemappedDiagnostic],
    control_ranges: &[std::ops::Range<usize>],
    background_ranges: &[(std::ops::Range<usize>, Color)],
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
            for slot in best_group[s..e].iter_mut() {
                match slot {
                    Some((_, prev_size)) if *prev_size <= range_size => {} // keep tighter
                    _ => *slot = Some((*group, range_size)),
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

    // 3b. Background tints (the diff review's added/removed rows): byte
    //     ranges, resolved like syntax spans but never overlapping.
    let mut bg_per_char: Vec<Option<Color>> = vec![None; num_chars];
    if !background_ranges.is_empty() {
        let byte_len = line_text.len();
        let mut bg_bytes: Vec<Option<Color>> = vec![None; byte_len];
        for (range, color) in background_ranges {
            let s = range.start.min(byte_len);
            let e = range.end.min(byte_len);
            for slot in bg_bytes[s..e].iter_mut() {
                *slot = Some(*color);
            }
        }
        for (char_idx, &byte_idx) in byte_indices[..num_chars].iter().enumerate() {
            if byte_idx < byte_len {
                bg_per_char[char_idx] = bg_bytes[byte_idx];
            }
        }
    }

    // 4. Diagnostics: mark each char position with underline color
    let mut diag_per_char: Vec<Option<Color>> = vec![None; num_chars];
    for d in diagnostics {
        let s = d.start.min(num_chars);
        let e = d.end.min(num_chars);
        let (s, e) = if s <= e { (s, e) } else { (e, s) };
        if s == e {
            continue;
        }
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

    let is_col_selected = |col: usize| {
        visual_selection
            .as_ref()
            .is_some_and(|range| range.contains(&col))
    };

    // --- Main loop: group consecutive characters with identical styling ---
    let mut col_idx = 0;
    while col_idx < num_chars {
        let is_selected = is_col_selected(col_idx);
        let is_search_match = search_flags[col_idx];
        let syntax_group = syntax_per_char[col_idx];
        let diag_underline_color = diag_per_char[col_idx];
        let is_control = control_per_char[col_idx];
        let background = bg_per_char[col_idx];

        // Extend span while styling is identical
        let mut end_col = col_idx + 1;
        while end_col < num_chars
            && is_col_selected(end_col) == is_selected
            && search_flags[end_col] == is_search_match
            && syntax_per_char[end_col] == syntax_group
            && diag_per_char[end_col] == diag_underline_color
            && control_per_char[end_col] == is_control
            && bg_per_char[end_col] == background
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

        // Apply the diff tint under everything except selection and search,
        // which own the background themselves.
        if let (Some(background), false, false) = (background, is_selected, is_search_match) {
            style = style.bg(background);
        }

        // Apply diagnostic underline (additive — works on top of any style)
        if let Some(underline_color) = diag_underline_color {
            style = style.fg(underline_color).add_modifier(Modifier::UNDERLINED);
        }

        spans.push(Span::styled(text, style));
        col_idx = end_col;
    }

    Line::from(spans)
}

fn render_diff_scrollbar(frame: &mut Frame, area: Rect, total: usize, top: usize, theme: &Theme) {
    let height = usize::from(area.height);
    if height == 0 {
        return;
    }
    let thumb = (height.saturating_mul(height) / total.max(1)).clamp(1, height);
    let travel = height - thumb;
    let scrollable = total.saturating_sub(height);
    let start = top.min(scrollable).saturating_mul(travel) / scrollable.max(1);
    for row in 0..height {
        let selected = (start..start + thumb).contains(&row);
        let group = if selected {
            UiGroup::LineNumberCurrent
        } else {
            UiGroup::LineNumber
        };
        let style = Style::default()
            .fg(crate::key_convert::convert_core_color(
                theme.get_ui_color(group),
            ))
            .bg(crate::key_convert::convert_core_color(
                theme.get_ui_color(UiGroup::Background),
            ));
        frame.render_widget(
            Paragraph::new(if selected { "┃" } else { "│" }).style(style),
            Rect::new(area.x, area.y + row as u16, 1, 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walkthrough_range_marks_every_inclusive_logical_line() {
        let range = Some((4, 6));
        assert!(!line_is_in_walkthrough(range, 3));
        assert!(line_is_in_walkthrough(range, 4));
        assert!(line_is_in_walkthrough(range, 5));
        assert!(line_is_in_walkthrough(range, 6));
        assert!(!line_is_in_walkthrough(range, 7));
        assert!(!line_is_in_walkthrough(None, 5));
    }

    fn plain_indexed_styles(theme: &Theme) -> IndexedRowStyles<'_> {
        IndexedRowStyles {
            theme,
            syntax: Vec::new(),
            selected: None,
            search: &[],
            diagnostics: Vec::new(),
            backgrounds: Vec::new(),
            cursorline: false,
            yank: None,
            ai: None,
            links: &[],
            bracket: None,
            walkthrough: false,
        }
    }

    #[test]
    fn indexed_visible_rows_match_legacy_tabs_wide_and_overlay_priority() {
        let theme = Theme::default();
        let text = "a\t中e\u{301}yz";
        let index = ovim_core::text_index::LineIndex::from_text(text);
        let geometry =
            ovim_core::line_layout::IndexedLineLayout::new(index, 5, 4, std::sync::Arc::from([]));
        let expanded = expand_tabs_with_mapping(text, 4);
        let legacy = split_line_into_rows(Line::from(expanded.text), 5);
        let rows = geometry.row_fragments(0..geometry.row_count());
        let plain = plain_indexed_styles(&theme);
        let rendered: Vec<_> = rows
            .iter()
            .map(|row| {
                let mut line = render_indexed_fragments(&row.fragments, &plain, &[]);
                pad_line_to(&mut line, 5);
                line
            })
            .collect();
        let strings = |lines: &[Line<'_>]| {
            lines
                .iter()
                .map(|line| {
                    line.spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(strings(&rendered), strings(&legacy));

        let mut styled = plain_indexed_styles(&theme);
        styled.selected = Some(1..3); // tab and CJK glyph, including split-tab spaces
        styled.search = &[(0, 4)];
        styled.diagnostics = vec![RemappedDiagnostic {
            start: 1,
            end: 3,
            color: Color::Red,
        }];
        let row = render_indexed_fragments(&rows[0].fragments, &styled, &[]);
        let tab = row
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "   ")
            .unwrap();
        assert_eq!(
            tab.style.bg,
            Some(crate::key_convert::convert_core_color(
                theme.get_ui_color(UiGroup::Visual)
            ))
        );
        assert_eq!(tab.style.fg, Some(Color::Red));
        assert!(tab.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn indexed_syntax_sweep_preserves_original_specificity_after_clipping() {
        let highlights = vec![
            (0..100, HighlightGroup::String),
            (45..55, HighlightGroup::Keyword),
            (49..51, HighlightGroup::Function),
        ];
        let resolved = resolve_indexed_syntax(&highlights, 48..53);
        assert_eq!(
            resolved,
            vec![
                (48..49, HighlightGroup::Keyword),
                (49..51, HighlightGroup::Function),
                (51..53, HighlightGroup::Keyword)
            ]
        );
    }

    #[test]
    fn long_line_subrow_renderer_seeks_directly_and_continues_to_next_line() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut editor = Editor::with_content(&format!("{}END\ntail\n", "a".repeat(10_000)));
        editor.options.wrap = true;
        editor.options.cursorline = false;
        editor.options.showmatch = false;
        editor.ensure_wrap_map(5);
        let layout = BufferLayout {
            buffer_area: Rect::new(0, 0, 5, 3),
            render_area: Rect::new(0, 0, 5, 3),
            gutter_width: 0,
            text_width: 5,
            line_num_width: 0,
            blame_width: 0,
            fold_width: 0,
            scrollbar_area: None,
        };
        let context = WindowRenderContext {
            scroll_offset: Some(0),
            scroll_subrow: Some(2000),
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(5, 3)).unwrap();
        let mut cache = super::super::line_cache::LineRenderCache::new();
        terminal
            .draw(|frame| {
                render_buffer(
                    frame,
                    &editor,
                    &Theme::default(),
                    &layout,
                    &mut cache,
                    Some(&context),
                );
            })
            .unwrap();
        let cells = terminal.backend().buffer();
        let row = |y| (0..5).map(|x| cells[(x, y)].symbol()).collect::<String>();
        assert_eq!(row(0), "END  ");
        assert_eq!(row(1), "tail ");
        assert_eq!(row(2), "     ");
    }

    #[test]
    fn long_nowrap_viewport_snaps_left_wide_glyph_and_keeps_indicators() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut editor = Editor::with_content(&format!("{}界word", "x".repeat(5000)));
        editor.options.wrap = false;
        editor.options.cursorline = false;
        editor.options.showmatch = false;
        let layout = BufferLayout {
            buffer_area: Rect::new(0, 0, 5, 1),
            render_area: Rect::new(0, 0, 5, 1),
            gutter_width: 0,
            text_width: 5,
            line_num_width: 0,
            blame_width: 0,
            fold_width: 0,
            scrollbar_area: None,
        };
        let context = WindowRenderContext {
            scroll_offset: Some(0),
            horizontal_offset: Some(5001),
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(5, 1)).unwrap();
        let mut cache = super::super::line_cache::LineRenderCache::new();
        terminal
            .draw(|frame| {
                render_buffer(
                    frame,
                    &editor,
                    &Theme::default(),
                    &layout,
                    &mut cache,
                    Some(&context),
                );
            })
            .unwrap();
        let cells = terminal.backend().buffer();
        assert_eq!(cells[(0, 0)].symbol(), "<");
        assert_eq!(cells[(1, 0)].symbol(), "界");
        assert_eq!(cells[(3, 0)].symbol(), "w");
        assert_eq!(cells[(4, 0)].symbol(), ">");
    }

    #[test]
    fn long_concealed_line_uses_cached_source_mapping_for_visible_link_style() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut editor = Editor::with_content(&format!(
            "cursor\n{}[link](https://example.test)x\n",
            "a".repeat(5000)
        ));
        editor.buffer_mut().set_file_path("long.md".to_string());
        editor.options.wrap = true;
        editor.options.markdown_conceal = true;
        editor.options.cursorline = false;
        editor.options.showmatch = false;
        editor.ensure_wrap_map(5);
        let layout = BufferLayout {
            buffer_area: Rect::new(0, 0, 5, 1),
            render_area: Rect::new(0, 0, 5, 1),
            gutter_width: 0,
            text_width: 5,
            line_num_width: 0,
            blame_width: 0,
            fold_width: 0,
            scrollbar_area: None,
        };
        let context = WindowRenderContext {
            scroll_offset: Some(1),
            scroll_subrow: Some(1000),
            ..Default::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(5, 1)).unwrap();
        let mut cache = super::super::line_cache::LineRenderCache::new();
        terminal
            .draw(|frame| {
                render_buffer(
                    frame,
                    &editor,
                    &Theme::default(),
                    &layout,
                    &mut cache,
                    Some(&context),
                );
            })
            .unwrap();
        let cells = terminal.backend().buffer();
        assert_eq!(
            (0..5).map(|x| cells[(x, 0)].symbol()).collect::<String>(),
            "linkx"
        );
        assert_eq!(cells[(0, 0)].fg, Color::Rgb(100, 149, 237));
        assert!(cells[(0, 0)].modifier.contains(Modifier::UNDERLINED));
        assert!(!cells[(4, 0)].modifier.contains(Modifier::UNDERLINED));
    }

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

    #[test]
    fn horizontal_viewport_preserves_graphemes_and_excludes_indicators_and_padding() {
        for (line, offset, width, expected, highlighted) in [
            ("hello", 0, 10, "hello", Some(0..5)),
            ("hello world!", 0, 6, "hello>", Some(0..5)),
            ("hello world!", 3, 6, "<lo w>", Some(1..5)),
            ("a世b", 0, 5, "a世b", Some(0..3)),
            ("a世b世c", 0, 5, "a世b>", Some(0..3)),
            ("a世b世c", 3, 5, "<b世c", Some(1..4)),
            ("a界b界c", 0, 3, "a >", Some(0..1)),
            ("a界b界c", 1, 4, "<界>", Some(1..2)),
            ("界word", 1, 5, "<界w>", Some(1..3)),
            ("hello", 0, 0, "", None),
            ("hello", 0, 1, ">", None),
            ("hello", 3, 1, "<", None),
        ] {
            let viewport = slice_horizontal_viewport(line, offset, width);
            assert_eq!(
                viewport.text, expected,
                "line={line:?}, offset={offset}, width={width}"
            );
            assert_eq!(
                viewport.project_range(0..usize::MAX),
                highlighted,
                "line={line:?}"
            );
        }
    }

    #[test]
    fn horizontal_viewport_clips_ranges_to_the_displayed_source_characters() {
        let viewport = slice_horizontal_viewport("é word tail", 2, 6);
        assert_eq!(viewport.text, "<word>");
        assert_eq!(viewport.project_range(0..2), None);
        assert_eq!(viewport.project_range(3..5), Some(1..3));
        assert_eq!(viewport.project_range(5..20), Some(3..5));
        assert_eq!(viewport.project_range(7..20), None);
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
    fn test_apply_eol_decorations_on_padded_wrapped_row() {
        use ovim_core::editor::decoration::*;

        let base = Line::from("let x = 1;".to_string());
        let mut rows = split_line_into_rows(base, 30);
        let mut first = rows.remove(0);

        let dec = Decoration {
            placement: DecorationPlacement::EndOfLine { char_offset: 0 },
            source: DecorationSource::Diagnostic,
            text: "\u{f057} uh oh".to_string(),
            display_width: 7,
            style: DecorationStyle::new(ovim_core::color::Color::Red).with_italic(),
            priority: 0,
            source_version: 0,
        };

        apply_eol_decorations(&mut first, &[&dec], EolPlacement::Append { text_width: 30 });

        let mut rendered = String::new();
        for span in &first.spans {
            rendered.push_str(span.content.as_ref());
        }
        assert!(rendered.contains("uh oh"));

        let display_width: usize = rendered
            .chars()
            .map(crate::display::char_display_width)
            .sum();
        assert_eq!(display_width, 30);
    }

    /// Regression: under soft wrap (the default), `split_line_into_rows` pads
    /// every row to `text_width`. Measuring that padding made
    /// `place_eol_on_line` treat every short line as "full" and take the
    /// overlay branch — the diagnostic landed right-aligned at the screen
    /// edge, capped to a third of the width, far away from its code.
    #[test]
    fn test_padded_wrapped_row_gets_adjacent_eol_not_edge_overlay() {
        use ovim_core::editor::decoration::*;

        let text_width = 60;
        let base = Line::from("let x = 1;".to_string());
        let mut rows = split_line_into_rows(base, text_width);
        assert_eq!(rows.len(), 1);
        let mut row = rows.remove(0);

        let dec = Decoration {
            placement: DecorationPlacement::EndOfLine { char_offset: 0 },
            source: DecorationSource::Diagnostic,
            text: "unused variable: `x`".to_string(),
            display_width: 20,
            style: DecorationStyle::new(ovim_core::color::Color::Red).with_italic(),
            priority: 0,
            source_version: 0,
        };

        place_eol_on_line(&mut row, &[&dec], text_width, text_width);

        let rendered: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.starts_with("let x = 1;  unused variable: `x`"),
            "diagnostic must sit {EOL_DIAG_GAP} columns after the code, not at the far edge; got {rendered:?}"
        );
        let display_width: usize = rendered
            .chars()
            .map(crate::display::char_display_width)
            .sum();
        assert_eq!(display_width, text_width, "row is re-padded to full width");
    }

    /// A row whose real content fills the box still uses the overlay so the
    /// diagnostic stays visible.
    #[test]
    fn test_full_content_row_still_overlays_at_edge() {
        use ovim_core::editor::decoration::*;

        let text_width = 40;
        let full: String = "x".repeat(text_width);
        let mut row = Line::from(full);

        let dec = Decoration {
            placement: DecorationPlacement::EndOfLine { char_offset: 0 },
            source: DecorationSource::Diagnostic,
            text: "too long".to_string(),
            display_width: 8,
            style: DecorationStyle::new(ovim_core::color::Color::Red).with_italic(),
            priority: 0,
            source_version: 0,
        };

        place_eol_on_line(&mut row, &[&dec], text_width, text_width);

        let rendered: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.ends_with("too long"),
            "overlay should place the message at the right edge; got {rendered:?}"
        );
        let display_width: usize = rendered
            .chars()
            .map(crate::display::char_display_width)
            .sum();
        assert_eq!(display_width, text_width);
    }

    /// OV-00329 regression: a diagnostic republish WITHOUT a buffer edit
    /// (save → cargo-check adds a second diagnostic at a new span while the
    /// top message stays the same) must change the line's render cache key.
    /// The decoration fingerprint alone — the only diagnostic-derived key
    /// component before the fix — collides in this scenario, so the cached
    /// row (with the old squiggle baked in) would be served until an
    /// edit/scroll/resize.
    #[test]
    fn test_diagnostic_republish_without_edit_changes_line_cache_key() {
        use crate::ui::renderer::line_cache::{LineCacheFrame, LineRenderCache};
        use ovim_core::editor::decoration::{decorations_from_diagnostics, DecorationSource};

        let mut editor = Editor::with_content("let x = 1;\n");

        let top = lsp_types::Diagnostic {
            range: lsp_types::Range::new(
                lsp_types::Position::new(0, 4),
                lsp_types::Position::new(0, 5),
            ),
            severity: Some(lsp_types::DiagnosticSeverity::ERROR),
            message: "top message".to_string(),
            ..lsp_types::Diagnostic::default()
        };
        let extra = lsp_types::Diagnostic {
            range: lsp_types::Range::new(
                lsp_types::Position::new(0, 8),
                lsp_types::Position::new(0, 9),
            ),
            severity: Some(lsp_types::DiagnosticSeverity::WARNING),
            message: "second span".to_string(),
            ..lsp_types::Diagnostic::default()
        };

        let rope = editor.buffer().rope().clone();
        let version = editor.buffer().version() as u64;

        editor.set_test_diagnostics(vec![top.clone()]);
        editor.decorations.replace_source(
            DecorationSource::Diagnostic,
            decorations_from_diagnostics(std::slice::from_ref(&top), &rope, version),
            &rope,
        );
        let decs1 = editor
            .decorations
            .project_all(&rope, editor.buffer().edit_log());
        let h1 = line_decoration_cache_hash(&decs1, &editor.project_diagnostics(), 0);

        // Republish: second diagnostic at a new span, top message unchanged,
        // no buffer edit.
        editor.set_test_diagnostics(vec![top.clone(), extra.clone()]);
        editor.decorations.replace_source(
            DecorationSource::Diagnostic,
            decorations_from_diagnostics(&[top, extra], &rope, version),
            &rope,
        );
        let decs2 = editor
            .decorations
            .project_all(&rope, editor.buffer().edit_log());

        // The pre-fix key component cannot see the change (same best-severity
        // EOL decoration) — this is exactly the collision the fix closes.
        assert_eq!(decs1.line_hash(0), decs2.line_hash(0));

        let h2 = line_decoration_cache_hash(&decs2, &editor.project_diagnostics(), 0);
        assert_ne!(
            h2, h1,
            "cache key must change when the diagnostic set changes"
        );

        // And a row cached under the old key re-renders under the new one.
        let mut cache = LineRenderCache::new();
        let frame = LineCacheFrame {
            buffer_id: 1,
            buffer_version: 1,
            highlight_generation: 0,
            h_offset: 0,
            text_width: 80,
            wrap: false,
            tab_width: 4,
            markdown_conceal: false,
        };
        cache.begin_frame(frame);
        cache.put(frame.key(0, h1), Line::from("row"), true);
        assert!(cache.get(&frame.key(0, h1)).is_some());
        assert!(cache.get(&frame.key(0, h2)).is_none());
    }

    #[test]
    fn test_render_line_empty_string() {
        let theme = Theme::default();
        let line = render_line_with_highlights(&theme, "", None, &[], &[], &[], &[], &[]);
        assert!(line.spans.is_empty());
    }

    #[test]
    fn test_render_line_plain_text_single_span() {
        let theme = Theme::default();
        let line =
            render_line_with_highlights(&theme, "hello world", None, &[], &[], &[], &[], &[]);
        // No highlights → should coalesce into one span
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content.as_ref(), "hello world");
    }

    #[test]
    fn test_render_line_syntax_highlight_splits_spans() {
        let theme = Theme::default();
        // Highlight bytes 0..2 ("fn") as Keyword
        let highlights = vec![(0..2, crate::syntax::HighlightGroup::Keyword)];
        let line =
            render_line_with_highlights(&theme, "fn main()", None, &[], &highlights, &[], &[], &[]);
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
            None,
            &search,
            &highlights,
            &[],
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
        let line =
            render_line_with_highlights(&theme, "aéb", None, &[], &highlights, &[], &[], &[]);
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
        let line =
            render_line_with_highlights(&theme, "error here", None, &[], &[], &diags, &[], &[]);
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
            None,
            &[],
            &highlights,
            &[],
            &[],
            &[],
        );
        // Should have 3 spans: "ab" (Variable), "cd" (Keyword), "efghij" (Variable)
        assert!(line.spans.len() >= 3);
        assert_eq!(line.spans[0].content.as_ref(), "ab");
        assert_eq!(line.spans[1].content.as_ref(), "cd");
    }
}
