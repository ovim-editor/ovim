//! The indexed-fragment pipeline: lines longer than 4096 bytes and scrolled
//! sub-rows render from source-aware layout fragments, so every temporary
//! string and style run is bounded by the visible rows.

use super::context::{BufferRenderContext, LineDecorations, LineFrame, RenderState};
use super::decorations::{
    apply_eol_decorations, decoration_to_ratatui_style, pad_line_to, place_eol_on_line,
    truncate_line_to_width, EolPlacement,
};
use super::gutter::WALKTHROUGH_SELECTION_BG;
use super::legacy::RemappedDiagnostic;
use super::rows::RowOutput;
use crate::display::grapheme_display_width;
use crate::syntax::{HighlightGroup, Theme, UiGroup};
use ovim_core::editor::decoration::Decoration;
use ovim_core::line_layout::{IndexedLineLayout, LayoutFragment, LayoutFragmentKind, LayoutRow};
use ovim_core::markdown_conceal::{ConcealedLink, LineTransform};
use ovim_core::text_index::LineIndex;
use ovim_core::unicode::{CharCol, GraphemeCol};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::ops::Range;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

/// Rendered fragments carry source coordinates, so overlays only allocate for
/// the visible rows. The legacy line renderer remains the small-line oracle.
pub(super) struct IndexedRowStyles<'a> {
    pub(super) theme: &'a Theme,
    pub(super) syntax: Vec<(Range<usize>, HighlightGroup)>,
    pub(super) selected: Option<Range<usize>>,
    pub(super) search: &'a [(usize, usize)],
    pub(super) diagnostics: Vec<RemappedDiagnostic>,
    pub(super) backgrounds: Vec<(Range<usize>, Color)>,
    pub(super) cursorline: bool,
    pub(super) yank: Option<Range<usize>>,
    pub(super) ai: Option<Range<usize>>,
    pub(super) links: &'a [ovim_core::markdown_conceal::ConcealedLink],
    pub(super) bracket: Option<usize>,
    pub(super) walkthrough: bool,
}

/// Resolve the original specificity/ordering rules with an interval sweep.
/// The result contains only style changes inside the visible byte interval.
pub(super) fn resolve_indexed_syntax(
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

pub(super) fn render_indexed_fragments(
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

/// One line's source text and the view text it renders from (the source with
/// markdown conceal applied, if any), with conversions between the two.
struct IndexedView<'a> {
    source_index: &'a LineIndex,
    view_index: &'a Arc<LineIndex>,
    transform: Option<&'a LineTransform>,
}

impl IndexedView<'_> {
    fn source_byte_to_view_char(&self, byte: usize) -> usize {
        self.transform
            .map(|map| {
                map.src_to_view
                    .get(byte)
                    .copied()
                    .unwrap_or(self.view_index.len_chars())
            })
            .unwrap_or_else(|| self.source_index.byte_to_char(byte))
    }

    fn source_char_to_view(&self, col: usize) -> usize {
        self.source_byte_to_view_char(self.source_index.char_to_byte(col))
    }

    fn source_byte_to_view_byte(&self, byte: usize) -> usize {
        if self.transform.is_some() {
            self.view_index
                .char_to_byte(self.source_byte_to_view_char(byte))
        } else {
            byte.min(self.view_index.len_bytes())
        }
    }
}

/// The layout rows to draw for an indexed line, plus the horizontal-scroll
/// indicators that frame the single row of a nowrap line.
struct IndexedRows {
    rows: Vec<LayoutRow>,
    precedes: bool,
    extends: bool,
    content_budget: usize,
    /// Blanks after the `<` standing in for the rest of a wide
    /// glyph the indicator cut in half.
    cut_cells: usize,
}

/// Selects the visible rows of an indexed line: the wrapped rows that fit in
/// `rows_left` (starting at the sub-row scroll on the top line), or the one
/// horizontally scrolled nowrap row.
fn indexed_visible_rows(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    layout: &IndexedLineLayout,
    rows_left: usize,
) -> IndexedRows {
    let text_width = ctx.text_width;
    let h_offset = ctx.h_offset;
    let mut precedes = false;
    let mut extends = false;
    let mut content_budget = text_width;
    let mut cut_cells = 0;
    let rows = if ctx.has_wrap {
        let first = if line_idx == ctx.start_line {
            ctx.top_skip
        } else {
            0
        };
        layout.row_fragments(first..first.saturating_add(rows_left))
    } else {
        let total = layout
            .display_range_for_row(layout.row_count().saturating_sub(1))
            .map(|range| range.end)
            .unwrap_or(0);
        // The `<` indicator covers the first cell, so text shows
        // from the next one and keeps its column.
        let requested_start = if total <= text_width {
            0
        } else {
            h_offset + usize::from(h_offset > 0)
        };
        let first =
            layout.fragments_for_display_range(requested_start..requested_start.saturating_add(1));
        let mut actual_start = first
            .first()
            .map(|fragment| fragment.display_start)
            .unwrap_or(requested_start);
        precedes = total > text_width && h_offset > 0;
        if precedes && actual_start < requested_start {
            let fragment_end = actual_start + first.first().map_or(0, |fragment| fragment.cells);
            cut_cells = fragment_end.saturating_sub(requested_start);
            actual_start = fragment_end;
        }
        let available = text_width
            .saturating_sub(usize::from(precedes))
            .saturating_sub(cut_cells);
        extends = total.saturating_sub(actual_start) > available && (!precedes || text_width > 1);
        content_budget = available.saturating_sub(usize::from(extends));
        vec![LayoutRow {
            index: 0,
            display_start: actual_start,
            display_end: actual_start.saturating_add(content_budget).min(total),
            fragments: layout.fragments_for_display_range(
                actual_start..actual_start.saturating_add(content_budget),
            ),
        }]
    };
    IndexedRows {
        rows,
        precedes,
        extends,
        content_budget,
        cut_cells,
    }
}

/// Syntax highlights covering the visible rows, mapped into view bytes and
/// resolved to the style changes inside the visible byte interval.
fn indexed_visible_syntax(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    view: &IndexedView<'_>,
    rows: &[LayoutRow],
) -> Vec<(Range<usize>, HighlightGroup)> {
    let visible_byte_start = rows
        .iter()
        .flat_map(|row| &row.fragments)
        .filter_map(|fragment| fragment.source.as_ref().map(|source| source.bytes.start))
        .min()
        .unwrap_or(0);
    let visible_byte_end = rows
        .iter()
        .flat_map(|row| &row.fragments)
        .filter_map(|fragment| fragment.source.as_ref().map(|source| source.bytes.end))
        .max()
        .unwrap_or(0);
    let source_byte_window = if let Some(mapped) = view.transform {
        let view_start = view.view_index.byte_to_char(visible_byte_start);
        let view_end = view.view_index.byte_to_char(visible_byte_end);
        let after_start = mapped.src_to_view.partition_point(|&col| col <= view_start);
        let prior = mapped
            .src_to_view
            .get(after_start.saturating_sub(1))
            .copied()
            .unwrap_or(0);
        let start = mapped.src_to_view.partition_point(|&col| col < prior);
        let end = mapped.src_to_view.partition_point(|&col| col < view_end);
        start..end.min(view.source_index.len_bytes())
    } else {
        visible_byte_start..visible_byte_end
    };
    let mapped_syntax: Vec<_> = ctx
        .buffer
        .highlights_in_byte_range(line_idx, source_byte_window)
        .iter()
        .map(|(range, group)| {
            (
                view.source_byte_to_view_byte(range.start)
                    ..view.source_byte_to_view_byte(range.end),
                *group,
            )
        })
        .collect();
    resolve_indexed_syntax(&mapped_syntax, visible_byte_start..visible_byte_end)
}

/// The visual selection on this line as a view char range.
fn indexed_selection(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    view: &IndexedView<'_>,
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
            view.source_index.grapheme_count()
        };
        Some(
            view.source_char_to_view(view.source_index.grapheme_to_char(GraphemeCol(start)).0)
                ..view.source_char_to_view(view.source_index.grapheme_to_char(GraphemeCol(end)).0),
        )
    })
}

/// Diagnostics covering this line as view char ranges.
fn indexed_diagnostics(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    view: &IndexedView<'_>,
) -> Vec<RemappedDiagnostic> {
    let source_index = view.source_index;
    ctx.projected_diagnostics
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
                start: view.source_char_to_view(start),
                end: view.source_char_to_view(end),
                color,
            })
        })
        .collect()
}

/// Diff-review background tints on this line as view byte ranges.
fn indexed_backgrounds(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    view: &IndexedView<'_>,
) -> Vec<(Range<usize>, Color)> {
    ctx.diff_review_tints
        .map(|state| {
            state
                .line_tints(line_idx)
                .into_iter()
                .map(|(range, added)| {
                    (
                        view.source_byte_to_view_byte(range.start)
                            ..view.source_byte_to_view_byte(range.end),
                        ctx.diff_tint_color(added),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The yank flash on this line as a view char range.
fn indexed_yank(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    view: &IndexedView<'_>,
) -> Option<Range<usize>> {
    ctx.editor
        .yank_flash()
        .filter(|flash| flash.contains_line(line_idx))
        .map(|flash| {
            flash
                .col_range_for_line(line_idx)
                .map(|(start, end)| {
                    view.source_char_to_view(start)..view.source_char_to_view(end.saturating_add(1))
                })
                .unwrap_or(0..view.view_index.len_chars())
        })
}

/// The AI selection on this line as a view char range.
fn indexed_ai_selection(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    view: &IndexedView<'_>,
) -> Option<Range<usize>> {
    let line_start_char = ctx.buffer.rope().line_to_char(line_idx);
    let source_index = view.source_index;
    ctx.ai_selection
        .filter(|selection| selection.contains_line(line_idx))
        .map(|selection| {
            let start = if selection.is_linewise() || line_idx != selection.start_line {
                0
            } else {
                selection
                    .start_char
                    .saturating_sub(line_start_char)
                    .min(source_index.len_chars())
            };
            let end = if selection.is_linewise() || line_idx != selection.end_line {
                source_index.len_chars()
            } else {
                selection
                    .end_char
                    .saturating_sub(line_start_char)
                    .min(source_index.len_chars())
            };
            view.source_char_to_view(start)..view.source_char_to_view(end)
        })
}

/// The matching-bracket highlight on this line as a view char column.
fn indexed_bracket(
    ctx: &BufferRenderContext<'_>,
    line_idx: usize,
    view: &IndexedView<'_>,
) -> Option<usize> {
    ctx.bracket_positions.and_then(|((l1, c1), (l2, c2))| {
        if line_idx == l1 {
            Some(view.source_char_to_view(c1))
        } else if line_idx == l2 {
            Some(view.source_char_to_view(c2))
        } else {
            None
        }
    })
}

/// Collects every overlay that can style the visible rows of an indexed line.
fn indexed_row_styles<'s>(
    ctx: &BufferRenderContext<'s>,
    line: &LineFrame<'_>,
    view: &IndexedView<'_>,
    syntax: Vec<(Range<usize>, HighlightGroup)>,
    search: &'s [(usize, usize)],
    links: &'s [ConcealedLink],
) -> IndexedRowStyles<'s> {
    let line_idx = line.line_idx;
    IndexedRowStyles {
        theme: ctx.theme,
        syntax,
        selected: indexed_selection(ctx, line_idx, view),
        search,
        diagnostics: indexed_diagnostics(ctx, line_idx, view),
        backgrounds: indexed_backgrounds(ctx, line_idx, view),
        cursorline: line.is_cursor_line,
        yank: indexed_yank(ctx, line_idx, view),
        ai: indexed_ai_selection(ctx, line_idx, view),
        links,
        bracket: indexed_bracket(ctx, line_idx, view),
        walkthrough: ctx.walkthrough,
    }
}

/// Draws a long line (or a scrolled sub-row of a wrapped one) from source-aware
/// layout fragments, computing styles only for the visible rows.
pub(super) fn emit_indexed_line(
    ctx: &BufferRenderContext<'_>,
    state: &mut RenderState<'_>,
    line: &LineFrame<'_>,
    source_index: &Arc<LineIndex>,
) {
    let line_idx = line.line_idx;
    let line_start_char = ctx.buffer.rope().line_to_char(line_idx);
    let mut decorations = LineDecorations::split(ctx.projected_decorations.for_line(line_idx));
    decorations
        .inline
        .sort_by_key(|d| (d.placement.char_offset(), d.priority));
    let map_layout = ctx
        .has_wrap
        .then(|| ctx.wrap_map.and_then(|map| map.line_layout(line_idx)))
        .flatten();
    let cached_indexed = if map_layout.is_none() {
        Some(state.line_cache.indexed_line(
            ctx.buffer_id,
            line_idx,
            source_index.clone(),
            ctx.text_width,
            ctx.tab_width,
            ctx.is_md_file && ctx.markdown_conceal && !line.is_cursor_line_for_conceal,
            line.dec_hash,
            &decorations.inline,
            line_start_char,
        ))
    } else {
        None
    };
    let indexed_layout = map_layout.unwrap_or_else(|| &cached_indexed.as_ref().unwrap().layout);
    let transform = if map_layout.is_some() {
        ctx.wrap_map.and_then(|map| map.line_transform(line_idx))
    } else {
        cached_indexed
            .as_ref()
            .and_then(|cached| cached.transform.as_deref())
    };
    let links = if map_layout.is_some() {
        ctx.wrap_map
            .map(|map| map.line_concealed_links(line_idx))
            .unwrap_or(&[])
    } else {
        cached_indexed
            .as_ref()
            .map(|cached| cached.links.as_ref())
            .unwrap_or(&[])
    };
    let view = IndexedView {
        source_index,
        view_index: indexed_layout.line(),
        transform,
    };
    // Indexed geometry seeks directly to a scrolled sub-row; never construct
    // the offscreen prefix just to discard it.
    let rows = indexed_visible_rows(
        ctx,
        line_idx,
        indexed_layout,
        ctx.visible_lines - state.out.visual_rows_used,
    );
    let syntax = indexed_visible_syntax(ctx, line_idx, &view, &rows.rows);
    let search = ctx
        .current_search
        .map(|search| search.find_all_in_index(view.view_index))
        .unwrap_or_default();
    let styles = indexed_row_styles(ctx, line, &view, syntax, &search, links);
    emit_indexed_rows(
        ctx,
        &mut state.out,
        line,
        indexed_layout.row_count(),
        rows,
        &styles,
        &decorations,
    );
}

/// Renders the selected layout rows into styled, padded text rows.
fn emit_indexed_rows(
    ctx: &BufferRenderContext<'_>,
    out: &mut RowOutput,
    line: &LineFrame<'_>,
    row_count: usize,
    visible: IndexedRows,
    styles: &IndexedRowStyles<'_>,
    decorations: &LineDecorations<'_>,
) {
    let text_width = ctx.text_width;
    let render_width = ctx.render_width;
    let trailing_background = ctx.trailing_background(line.line_idx);
    for row in visible.rows {
        let mut rendered = render_indexed_fragments(&row.fragments, styles, &decorations.inline);
        if !ctx.has_wrap {
            truncate_line_to_width(&mut rendered, visible.content_budget);
            pad_line_to(&mut rendered, visible.content_budget);
            if visible.precedes {
                rendered
                    .spans
                    .insert(0, Span::raw(format!("<{}", " ".repeat(visible.cut_cells))));
            }
            if visible.extends {
                rendered.spans.push(Span::raw(">"));
            }
            place_eol_on_line(&mut rendered, &decorations.eol, text_width, render_width);
        } else if row.index + 1 == row_count {
            place_eol_on_line(&mut rendered, &decorations.eol, text_width, render_width);
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
        ctx.pad_row(&mut rendered, trailing_background);
        out.push_row(
            ctx,
            line.line_idx,
            ctx.has_wrap && row.index > 0,
            line.diagnostics,
            rendered,
        );
    }
}
