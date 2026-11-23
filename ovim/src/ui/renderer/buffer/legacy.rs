//! The legacy pipeline: lines up to 4096 bytes are tab-expanded and styled as
//! a whole, cached, then split into visual rows.

use super::context::{BufferRenderContext, LineDecorations, LineFrame, RenderState};
use super::decorations::apply_inline_decorations;
use super::gutter::WALKTHROUGH_SELECTION_BG;
use super::overlays::{
    apply_bg_to_column_range, apply_fg_modifier_to_column_range, apply_style_at_column,
};
use super::rows::{emit_unwrapped_row, emit_wrapped_rows, RowOutput};
use super::viewport::{
    display_col_to_char_idx, expanded_char_to_display_col, shift_highlights_for_viewport,
    slice_horizontal_viewport, HighlightShiftBuffers, HorizontalViewport,
};
use crate::display::grapheme_display_width;
use crate::syntax::{HighlightGroup, Theme, UiGroup};
use crate::ui::renderer::helpers::{
    compose_conceal_and_tabs, expand_tabs_with_mapping, remap_char_col,
};
use crate::ui::renderer::markdown_conceal::scan_markdown_conceal;
use crate::ui::renderer::styles::remap_highlights;
use ovim_core::markdown_conceal::ConcealedLink;
use ovim_core::unicode::{
    char_to_grapheme_col, grapheme_count, grapheme_to_char_col, CharCol, GraphemeCol,
};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// The text of one line prepared for the legacy renderer: markdown-concealed,
/// tab-expanded, and (without wrap) sliced to the horizontal viewport. All
/// overlay coordinates are relative to `visible()`.
struct LegacyLineText {
    /// Visible content of the line (trailing terminator stripped).
    original: String,
    expanded: String,
    conceal_byte_map: Vec<(usize, usize)>,
    control_ranges: Vec<Range<usize>>,
    char_mapping: Vec<usize>,
    concealed_links: Vec<ConcealedLink>,
    viewport: Option<HorizontalViewport>,
}

impl LegacyLineText {
    fn new(ctx: &BufferRenderContext<'_>, line_idx: usize, conceal_disabled: bool) -> Self {
        let tab_width = ctx.tab_width;
        let original = ovim_core::display::line_content(ctx.buffer.rope(), line_idx);

        // Optional markdown conceal (skip on cursor line so editing isn't blind)
        let (exp, concealed_links) = if ctx.is_md_file && ctx.markdown_conceal && !conceal_disabled
        {
            let spans = scan_markdown_conceal(&original);
            if spans.is_empty() {
                (expand_tabs_with_mapping(&original, tab_width), Vec::new())
            } else {
                let transform =
                    crate::ui::renderer::markdown_conceal::apply_conceal(&original, &spans);
                let links = crate::ui::renderer::markdown_conceal::extract_concealed_links(
                    &spans, &transform,
                );
                (
                    compose_conceal_and_tabs(&original, &transform, tab_width),
                    links,
                )
            }
        } else {
            (expand_tabs_with_mapping(&original, tab_width), Vec::new())
        };

        let viewport =
            (!ctx.wrap).then(|| slice_horizontal_viewport(&exp.text, ctx.h_offset, ctx.text_width));
        Self {
            original,
            expanded: exp.text,
            conceal_byte_map: exp.byte_mapping,
            control_ranges: exp.control_ranges,
            char_mapping: exp.char_mapping,
            concealed_links,
            viewport,
        }
    }

    /// Cells before the first displayed source character (see `HorizontalViewport`).
    fn left(&self) -> usize {
        self.viewport.as_ref().map_or(0, |view| view.left)
    }

    /// The text that is drawn: the viewport slice, or the whole expanded line.
    fn visible(&self) -> &str {
        self.viewport
            .as_ref()
            .map(|view| view.text.as_str())
            .unwrap_or(&self.expanded)
    }

    /// Shifts highlight ranges (in expanded bytes) into `visible()` bytes
    /// under the horizontal viewport.
    fn shift_for_viewport<T: Copy>(
        &self,
        ctx: &BufferRenderContext<'_>,
        highlights: &[(Range<usize>, T)],
        buffers: &mut HighlightShiftBuffers,
    ) -> Vec<(Range<usize>, T)> {
        shift_highlights_for_viewport(
            highlights,
            &self.expanded,
            self.visible(),
            ctx.h_offset,
            ctx.text_width,
            self.left(),
            buffers,
        )
    }
}

/// Every overlay that can style a legacy line, in `LegacyLineText::visible()`
/// coordinates.
struct LegacyOverlays {
    syntax_highlights: Vec<(Range<usize>, HighlightGroup)>,
    background_ranges: Vec<(Range<usize>, Color)>,
    search_matches: Vec<(usize, usize)>,
    visual_selection: Option<Range<usize>>,
    bracket_col: Option<usize>,
    diagnostics: Vec<RemappedDiagnostic>,
    ai_selection_ranges: Vec<(usize, usize)>,
    yank_flash: Option<Option<(usize, usize)>>,
}

/// Get syntax highlights for this line and remap them for expanded text
fn legacy_syntax_highlights(
    ctx: &BufferRenderContext<'_>,
    shift_buffers: &mut HighlightShiftBuffers,
    line_idx: usize,
    text: &LegacyLineText,
) -> Vec<(Range<usize>, HighlightGroup)> {
    let original_highlights = ctx.buffer.highlights_for_line(line_idx);
    let mut syntax_highlights = remap_highlights(&original_highlights, &text.conceal_byte_map);

    // Shift syntax highlights for horizontal viewport if nowrap
    if !ctx.wrap {
        syntax_highlights = text.shift_for_viewport(ctx, &syntax_highlights, shift_buffers);
    }
    syntax_highlights
}

/// Diff review tints and a just-entered snippet placeholder, in the same byte
/// space as the syntax highlights.
fn legacy_background_ranges(
    ctx: &BufferRenderContext<'_>,
    shift_buffers: &mut HighlightShiftBuffers,
    line_idx: usize,
    text: &LegacyLineText,
) -> Vec<(Range<usize>, Color)> {
    // Diff review: added/removed background tints, in the same byte
    // space as the syntax highlights above.
    let mut background_ranges: Vec<(Range<usize>, Color)> = ctx
        .diff_review_tints
        .map(|state| {
            state
                .line_tints(line_idx)
                .into_iter()
                .map(|(range, added)| (range, ctx.diff_tint_color(added)))
                .collect()
        })
        .unwrap_or_default();
    // A just-entered snippet placeholder reads as selected: typing
    // replaces it (see `Editor::snippet_placeholder_highlight`).
    if let Some((hl_line, start_col, end_col)) = ctx.editor.snippet_placeholder_highlight() {
        if hl_line == line_idx {
            let mut offsets = text
                .original
                .char_indices()
                .map(|(byte, _)| byte)
                .chain(std::iter::once(text.original.len()));
            let start = offsets.clone().nth(start_col);
            let end = offsets.nth(end_col);
            if let (Some(start), Some(end)) = (start, end) {
                background_ranges.push((start..end, Color::Rgb(62, 84, 140)));
            }
        }
    }
    if !background_ranges.is_empty() {
        background_ranges = remap_highlights(&background_ranges, &text.conceal_byte_map);
        if !ctx.wrap {
            background_ranges = text.shift_for_viewport(ctx, &background_ranges, shift_buffers);
        }
    }
    background_ranges
}

/// Project this row's inclusive grapheme selection to a half-open
/// scalar range before conceal, tab expansion or viewport slicing.
/// In block mode every row needs its own conversion: equal grapheme
/// columns need not have equal scalar or display columns.
fn legacy_visual_selection(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    text: &LegacyLineText,
) -> Option<Range<usize>> {
    ctx.visual_selection.and_then(|((sl, sc), (el, ec))| {
        if line_idx < sl || line_idx > el {
            return None;
        }
        let block = ctx.editor.mode() == crate::mode::Mode::VisualBlock;
        let start = if block || line_idx == sl { sc } else { 0 };
        let end = if block || line_idx == el {
            ec.saturating_add(1)
        } else {
            grapheme_count(&text.original)
        };
        let expand = |col| {
            remap_char_col(
                grapheme_to_char_col(&text.original, GraphemeCol(col)).0,
                &text.char_mapping,
            )
        };
        let range = expand(start)..expand(end);
        if let Some(viewport) = &text.viewport {
            viewport.project_range(range)
        } else {
            (!range.is_empty()).then_some(range)
        }
    })
}

/// Check if this line has a bracket to highlight (remap through char_mapping)
fn legacy_bracket_col(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    text: &LegacyLineText,
) -> Option<usize> {
    let bracket_col = ctx.bracket_positions.and_then(|((l1, c1), (l2, c2))| {
        if line_idx == l1 {
            Some(remap_char_col(c1, &text.char_mapping))
        } else if line_idx == l2 {
            Some(remap_char_col(c2, &text.char_mapping))
        } else {
            None
        }
    });

    // Adjust bracket column for horizontal viewport if nowrap
    if !ctx.wrap {
        bracket_col.and_then(|expanded_char_col| {
            // Convert expanded char index to display column
            let display_col = expanded_char_to_display_col(&text.expanded, expanded_char_col);
            // Check if bracket is in visible horizontal range
            if display_col >= ctx.h_offset + text.left()
                && display_col < ctx.h_offset + ctx.text_width
            {
                // Convert to char index in the sliced text
                let viewport_display_col = display_col - ctx.h_offset;
                let sliced_char_idx = display_col_to_char_idx(text.visible(), viewport_display_col);
                Some(sliced_char_idx)
            } else {
                None // Bracket is outside viewport
            }
        })
    } else {
        bracket_col
    }
}

/// Underlines cover the complete span. Gutter signs and EOL
/// messages still use the diagnostics starting on this line.
fn legacy_diagnostics(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    text: &LegacyLineText,
) -> Vec<RemappedDiagnostic> {
    let line_text_original = &text.original;
    ctx.projected_diagnostics
        .covering(line_idx)
        .filter_map(|diagnostic| {
            let range =
                ovim_core::lsp::diagnostic_char_range(diagnostic, line_idx, line_text_original)?;
            if range.is_empty() {
                return None;
            }
            // A terminal cell cannot underline half a grapheme. Round
            // outward before tabs/conceal so accents and ZWJ sequences
            // never get split into differently styled spans.
            let start = char_to_grapheme_col(line_text_original, CharCol(range.start));
            let last = char_to_grapheme_col(line_text_original, CharCol(range.end - 1));
            let start = grapheme_to_char_col(line_text_original, start).0;
            let end = grapheme_to_char_col(line_text_original, GraphemeCol(last.0 + 1)).0;
            let range =
                remap_char_col(start, &text.char_mapping)..remap_char_col(end, &text.char_mapping);
            let range = if let Some(viewport) = &text.viewport {
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
        .collect()
}

/// The AI selection on this line as inclusive (start, end) char column pairs
/// in `visible()` coordinates.
fn legacy_ai_selection_ranges(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    text: &LegacyLineText,
) -> Vec<(usize, usize)> {
    let Some(selection) = ctx.ai_selection else {
        return Vec::new();
    };
    if !selection.contains_line(line_idx) {
        return Vec::new();
    }
    let line_char_count = text.original.chars().count();
    let (start_col, end_col_inclusive) = if selection.is_linewise() {
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
        return Vec::new();
    }
    let mut start = remap_char_col(start_col, &text.char_mapping);
    let mut end_exclusive = remap_char_col(
        (end_col_inclusive + 1).min(line_char_count),
        &text.char_mapping,
    );

    if !ctx.wrap {
        let start_display = expanded_char_to_display_col(&text.expanded, start);
        let end_display = expanded_char_to_display_col(&text.expanded, end_exclusive);
        if end_display <= ctx.h_offset + text.left()
            || start_display >= ctx.h_offset + ctx.text_width
        {
            Vec::new()
        } else {
            start = display_col_to_char_idx(
                text.visible(),
                start_display.saturating_sub(ctx.h_offset).max(text.left()),
            );
            end_exclusive =
                display_col_to_char_idx(text.visible(), end_display.saturating_sub(ctx.h_offset));
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

/// Computes every overlay for a legacy line.
fn legacy_overlays(
    ctx: &BufferRenderContext<'_>,
    shift_buffers: &mut HighlightShiftBuffers,
    line_idx: usize,
    text: &LegacyLineText,
) -> LegacyOverlays {
    let syntax_highlights = legacy_syntax_highlights(ctx, shift_buffers, line_idx, text);
    let background_ranges = legacy_background_ranges(ctx, shift_buffers, line_idx, text);

    // Check if we need special highlighting (visual selection or search)
    let search_matches = if let Some(search) = ctx.current_search {
        search.find_all_in_line(text.visible())
    } else {
        Vec::new()
    };

    LegacyOverlays {
        syntax_highlights,
        background_ranges,
        search_matches,
        visual_selection: legacy_visual_selection(ctx, line_idx, text),
        bracket_col: legacy_bracket_col(ctx, line_idx, text),
        diagnostics: legacy_diagnostics(ctx, line_idx, text),
        ai_selection_ranges: legacy_ai_selection_ranges(ctx, line_idx, text),
        // Check if this line is in a yank flash region
        yank_flash: ctx.editor.yank_flash().and_then(|flash| {
            if flash.contains_line(line_idx) {
                Some(flash.col_range_for_line(line_idx))
            } else {
                None
            }
        }),
    }
}

/// Whether the line needs the character-by-character renderer. Inline
/// decorations (inlay hints) and EOL decorations (LSP diagnostics rendered as
/// virtual text) both need the detailed path: inline decs require
/// `apply_inline_decorations` to splice their spans into the rendered line,
/// and EOL decs require the EOL placement step that the simple path doesn't
/// run. Use the unprojected `for_line` here for a cheap BTreeMap lookup; the
/// detailed block does the projection.
fn needs_detailed_rendering(
    ctx: &BufferRenderContext<'_>,
    line: &LineFrame<'_>,
    text: &LegacyLineText,
    overlays: &LegacyOverlays,
) -> bool {
    let any_decoration = !ctx.editor.decorations.for_line(line.line_idx).is_empty();

    // Always use character-by-character rendering if we have any highlighting
    line.has_visual_on_line
        || !overlays.search_matches.is_empty()
        || !overlays.syntax_highlights.is_empty()
        || line.is_cursor_line
        || overlays.bracket_col.is_some()
        || !overlays.diagnostics.is_empty()
        || overlays.yank_flash.is_some()
        || !overlays.ai_selection_ranges.is_empty()
        || !text.concealed_links.is_empty()
        || !overlays.background_ranges.is_empty()
        || any_decoration
}

/// Draws a line of up to 4096 bytes: prepares its text, computes the
/// overlays, then emits it through the detailed or simple renderer.
pub(super) fn emit_legacy_line(
    ctx: &BufferRenderContext<'_>,
    state: &mut RenderState<'_>,
    line: &LineFrame<'_>,
) {
    let line_idx = line.line_idx;
    let text = LegacyLineText::new(ctx, line_idx, line.is_cursor_line_for_conceal);
    let overlays = legacy_overlays(ctx, &mut state.hl_shift_buffers, line_idx, &text);

    if needs_detailed_rendering(ctx, line, &text, &overlays) {
        emit_detailed_line(ctx, state, line, &text, &overlays);
    } else {
        emit_simple_line(ctx, state, line, text.visible());
    }
}

/// Styles a rendered line with the overlays that are applied after the
/// character-by-character pass: cursorline, yank flash, AI selection,
/// concealed links and the matching bracket.
fn apply_legacy_overlays(
    ctx: &BufferRenderContext<'_>,
    line: &LineFrame<'_>,
    text: &LegacyLineText,
    overlays: &LegacyOverlays,
    rendered: &mut Line<'static>,
) {
    // Apply cursorline background if this is the cursor line
    if line.is_cursor_line && overlays.yank_flash.is_none() {
        let cursorline_bg = Color::Rgb(40, 40, 50); // Subtle dark blue background
        for span in &mut rendered.spans {
            if span.style.bg.is_none() || span.style.bg == Some(Color::Reset) {
                span.style = span.style.bg(cursorline_bg);
            }
        }
    }

    // Apply yank flash highlight
    if let Some(col_range) = overlays.yank_flash {
        let flash_bg = Color::Rgb(60, 50, 20); // Warm amber glow
        match col_range {
            None => {
                // Linewise flash: highlight entire line
                for span in &mut rendered.spans {
                    span.style = span.style.bg(flash_bg);
                }
            }
            Some((start_col, end_col)) => {
                // Character-wise flash: highlight column range
                apply_bg_to_column_range(rendered, start_col, end_col, flash_bg);
            }
        }
    }

    // Preserve syntax colors while making an interactive walkthrough
    // more prominent than an ordinary AI-prompt selection.
    let ai_selection_bg = if ctx.walkthrough {
        WALKTHROUGH_SELECTION_BG
    } else {
        Color::Rgb(62, 70, 82)
    };
    for (start_col, end_col) in &overlays.ai_selection_ranges {
        apply_bg_to_column_range(rendered, *start_col, *end_col, ai_selection_bg);
    }

    // Apply concealed link underline styling
    if !text.concealed_links.is_empty() {
        let link_color = Color::Rgb(100, 149, 237); // Cornflower blue
        for link in &text.concealed_links {
            apply_fg_modifier_to_column_range(
                rendered,
                link.view_start,
                link.view_end,
                link_color,
                Modifier::UNDERLINED,
            );
        }
    }

    // Apply bracket highlighting
    if let Some(col) = overlays.bracket_col {
        let bracket_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        apply_style_at_column(rendered, col, bracket_style);
    }
}

/// Draws a line through the character-by-character renderer, caches it, and
/// emits its rows.
fn emit_detailed_line(
    ctx: &BufferRenderContext<'_>,
    state: &mut RenderState<'_>,
    line: &LineFrame<'_>,
    text: &LegacyLineText,
    overlays: &LegacyOverlays,
) {
    let line_idx = line.line_idx;
    let mut rendered = render_line_with_highlights(
        ctx.theme,
        text.visible(),
        overlays.visual_selection.clone(),
        &overlays.search_matches,
        &overlays.syntax_highlights,
        &overlays.diagnostics,
        &text.control_ranges,
        &overlays.background_ranges,
    );
    apply_legacy_overlays(ctx, line, text, overlays, &mut rendered);

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
    let decorations = LineDecorations::split(ctx.projected_decorations.for_line(line_idx));

    if !decorations.inline.is_empty() {
        let line_start_offset = ctx.buffer.rope().line_to_char(line_idx);
        apply_inline_decorations(
            &mut rendered,
            &decorations.inline,
            &text.char_mapping,
            ctx.h_offset,
            ctx.wrap,
            line_start_offset,
        );
    }

    // Store in cache AFTER decorations so cache-hit frames
    // match cursor positioning.
    state
        .line_cache
        .put(line.cache_key, rendered.clone(), line.is_stable);

    if ctx.has_wrap {
        emit_wrapped_rows(ctx, &mut state.out, line, rendered, &decorations.eol);
    } else {
        emit_unwrapped_row(ctx, &mut state.out, line, rendered, &decorations.eol);
    }
}

/// Draws a line with no highlighting: plain text padded to the render width
/// and, under soft wrap, split by display width.
fn emit_simple_line(
    ctx: &BufferRenderContext<'_>,
    state: &mut RenderState<'_>,
    line: &LineFrame<'_>,
    line_text: &str,
) {
    // Simple rendering path (no highlighting) — always stable
    let simple_line = Line::from(line_text.to_string());
    state.line_cache.put(line.cache_key, simple_line, true);

    if ctx.has_wrap {
        emit_simple_wrapped_rows(ctx, &mut state.out, line.line_idx, line_text);
    } else {
        // No wrap: pad simple lines too
        let line_display_len = unicode_width::UnicodeWidthStr::width(line_text);
        let line_text = if line_display_len < ctx.render_width {
            format!(
                "{}{}",
                line_text,
                " ".repeat(ctx.render_width - line_display_len)
            )
        } else {
            line_text.to_string()
        };
        state
            .out
            .push_row(ctx, line.line_idx, false, &[], Line::from(line_text));
    }
}

fn emit_simple_wrapped_rows(
    ctx: &BufferRenderContext<'_>,
    out: &mut RowOutput,
    line_idx: usize,
    line_text: &str,
) {
    if line_text.is_empty() {
        out.push_row(ctx, line_idx, false, &[], ctx.blank_row());
        return;
    }

    // Split by display width, not char count — CJK/emoji
    // characters are width 2 and would overflow the terminal
    // row if counted as 1.
    let mut chunk_idx = 0;
    let mut row_text = String::new();
    let mut row_width = 0;

    for grapheme in line_text.graphemes(true) {
        let ch_width = grapheme_display_width(grapheme);

        if row_width + ch_width > ctx.text_width && !row_text.is_empty() {
            // Flush current row
            if !out.has_room(ctx) {
                break;
            }
            let pad = ctx.render_width.saturating_sub(row_width);
            if pad > 0 {
                row_text.push_str(&" ".repeat(pad));
            }
            out.push_row(
                ctx,
                line_idx,
                chunk_idx > 0,
                &[],
                Line::from(std::mem::take(&mut row_text)),
            );
            chunk_idx += 1;
            row_width = 0;
        }

        row_text.push_str(grapheme);
        row_width += ch_width;
    }

    // Flush the last row
    if !row_text.is_empty() && out.has_room(ctx) {
        let pad = ctx.render_width.saturating_sub(row_width);
        if pad > 0 {
            row_text.push_str(&" ".repeat(pad));
        }
        out.push_row(ctx, line_idx, chunk_idx > 0, &[], Line::from(row_text));
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
