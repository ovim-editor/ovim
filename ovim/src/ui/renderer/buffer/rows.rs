//! Row emission shared by both line pipelines: the output buffers, soft-wrap
//! row splitting, and emission of cached and already styled lines.

use super::context::{BufferRenderContext, LineFrame};
use super::decorations::{place_eol_on_line, place_eol_on_visual_rows};
use crate::display::grapheme_display_width;
use ovim_core::editor::decoration::Decoration;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

/// Rows emitted so far: text rows, their gutter rows, and the visual-row budget.
#[derive(Default)]
pub(super) struct RowOutput {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) gutter_lines: Vec<Line<'static>>,
    pub(super) visual_rows_used: usize,
}

impl RowOutput {
    pub(super) fn has_room(&self, ctx: &BufferRenderContext<'_>) -> bool {
        self.visual_rows_used < ctx.visible_lines
    }

    /// Appends a text row and, when a gutter is drawn, its gutter row.
    pub(super) fn push_row(
        &mut self,
        ctx: &BufferRenderContext<'_>,
        line_idx: usize,
        is_continuation: bool,
        line_diagnostics: &[lsp_types::Diagnostic],
        row: Line<'static>,
    ) {
        if ctx.has_gutter {
            self.gutter_lines
                .push(ctx.gutter_row(line_idx, is_continuation, line_diagnostics));
        }
        self.lines.push(row);
        self.visual_rows_used += 1;
    }

    /// Appends a blank text row with no gutter row.
    pub(super) fn push_blank(&mut self, ctx: &BufferRenderContext<'_>) {
        self.lines.push(ctx.blank_row());
        self.visual_rows_used += 1;
    }
}

/// Splits a rendered Line into multiple visual rows for soft wrapping.
/// Each row fits within `width` display columns. Rows are padded to full width.
/// Wide characters (CJK, emoji) that don't fit at a row boundary are pushed to
/// the next row, with the remaining space padded (matching Neovim behavior).
pub(super) fn split_line_into_rows(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
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

/// Emits a line that was rendered on an earlier frame.
pub(super) fn emit_cached_line(
    ctx: &BufferRenderContext<'_>,
    out: &mut RowOutput,
    line: &LineFrame<'_>,
    cached_line: Line<'static>,
) {
    // Cached line has inline decorations but needs
    // EOL decorations (diagnostics) applied fresh. The
    // per-frame projection cache (built before the loop)
    // avoids re-scanning every decoration here.
    let eol_decs: Vec<&Decoration> = ctx.projected_decorations.eol_for_line(line.line_idx);
    if ctx.has_wrap {
        emit_wrapped_rows(ctx, out, line, cached_line, &eol_decs);
    } else {
        emit_unwrapped_row(ctx, out, line, cached_line, &eol_decs);
    }
}

/// Soft wrap: splits a styled line into visual rows and emits as many as fit.
pub(super) fn emit_wrapped_rows(
    ctx: &BufferRenderContext<'_>,
    out: &mut RowOutput,
    line: &LineFrame<'_>,
    rendered: Line<'static>,
    eol_decs: &[&Decoration],
) {
    let mut visual_rows = split_line_into_rows(rendered, ctx.text_width);
    place_eol_on_visual_rows(&mut visual_rows, eol_decs, ctx.text_width, ctx.render_width);
    for (row_idx, row) in visual_rows.into_iter().enumerate() {
        if !out.has_room(ctx) {
            break;
        }
        out.push_row(ctx, line.line_idx, row_idx > 0, line.diagnostics, row);
    }
}

/// No wrap: in non-centered mode, ratatui clips overflow
/// at text_area's right edge, so we don't truncate (cursor
/// tracking would desync). In centered mode, AtBoxEdge
/// explicitly clips at the code-box and pads into the
/// diagnostic margin — `place_eol_on_line` picks the
/// right strategy.
pub(super) fn emit_unwrapped_row(
    ctx: &BufferRenderContext<'_>,
    out: &mut RowOutput,
    line: &LineFrame<'_>,
    mut rendered: Line<'static>,
    eol_decs: &[&Decoration],
) {
    place_eol_on_line(&mut rendered, eol_decs, ctx.text_width, ctx.render_width);
    ctx.pad_row(&mut rendered, ctx.trailing_background(line.line_idx));
    out.push_row(ctx, line.line_idx, false, line.diagnostics, rendered);
}
