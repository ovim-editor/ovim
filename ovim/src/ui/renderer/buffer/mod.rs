//! Renders the buffer text area and its gutter.
//!
//! Lines run through one of two pipelines: the legacy one for lines up to
//! 4096 bytes and the indexed-fragment one for longer lines and scrolled
//! sub-rows.

mod context;
mod decorations;
mod gutter;
mod indexed;
mod legacy;
mod overlays;
mod rows;
#[cfg(test)]
mod tests;
mod viewport;

use self::context::{BufferRenderContext, RenderState};
use self::indexed::emit_indexed_line;
use self::legacy::emit_legacy_line;
use self::rows::{emit_cached_line, RowOutput};
use self::viewport::HighlightShiftBuffers;
use crate::editor::Editor;
use crate::syntax::{Theme, UiGroup};
use ovim_core::buffer::Cursor;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::layout::BufferLayout;
use super::line_cache::LineRenderCache;

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

/// Splits the layout into the gutter (left of `buffer_area`) and text_area
/// (from end of gutter to right edge of render_area). When render_area equals
/// buffer_area (the common case), text_area is exactly buffer_area minus the
/// gutter — same as before. In centered mode render_area extends past
/// buffer_area, giving text_area a wider rect that includes the diagnostic
/// margin.
fn buffer_areas(layout: &BufferLayout) -> (Option<Rect>, Rect) {
    let area = layout.buffer_area;
    let gutter_width_u16 = layout.gutter_width as u16;
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
    (gutter_area, text_area)
}

/// Renders the buffer content and returns the viewport start line.
pub fn render_buffer(
    frame: &mut Frame,
    editor: &Editor,
    theme: &Theme,
    layout: &BufferLayout,
    line_cache: &mut LineRenderCache,
    window_context: Option<&WindowRenderContext>,
) -> usize {
    let ctx = BufferRenderContext::new(editor, theme, layout, window_context);
    let (gutter_area, text_area) = buffer_areas(layout);
    line_cache.begin_frame(ctx.cache_frame);

    // Build the visible text with syntax highlighting
    let mut state = RenderState {
        line_cache,
        hl_shift_buffers: HighlightShiftBuffers::new(),
        out: RowOutput::default(),
    };

    let mut line_idx = ctx.start_line;
    while line_idx < ctx.line_count && state.out.has_room(&ctx) {
        // Lines inside a closed fold are not drawn: skip the whole body.
        if let Some((start, end)) = ctx.buffer.fold_manager().closed_fold_at(line_idx) {
            if line_idx > start {
                line_idx = end + 1;
                continue;
            }
        }
        if line_idx < ctx.buffer.rope().len_lines() {
            render_logical_line(&ctx, &mut state, line_idx);
        } else {
            // Line beyond end of file - clear it
            state.out.push_blank(&ctx);
        }
        line_idx += 1;
    }

    // Fill remaining rows with blanks
    while state.out.has_room(&ctx) {
        state.out.push_blank(&ctx);
    }

    // Render gutter
    if let Some(gutter_area) = gutter_area {
        let gutter_paragraph = Paragraph::new(state.out.gutter_lines);
        frame.render_widget(gutter_paragraph, gutter_area);
    }

    let paragraph = Paragraph::new(state.out.lines)
        .block(Block::default().borders(Borders::NONE))
        .style(Style::default().bg(Color::Reset));
    frame.render_widget(paragraph, text_area);
    if let Some(rail) = layout.scrollbar_area {
        let (total, top) = ctx.scrollbar_extent();
        render_diff_scrollbar(frame, rail, total, top, theme);
    }

    ctx.start_line
}

/// Draws one logical line into `state.out`, picking a pipeline: the indexed
/// one for long lines and scrolled sub-rows, the cache for unchanged lines,
/// and the legacy one for everything else.
fn render_logical_line(
    ctx: &BufferRenderContext<'_>,
    state: &mut RenderState<'_>,
    line_idx: usize,
) {
    let line = ctx.line_frame(line_idx);

    // Long logical lines and sub-row viewports render from shared
    // source-aware fragments. All temporary strings/style runs are
    // bounded by the visible rows, including deeply scrolled lines.
    let source_index = ctx.buffer.line_index(line_idx);
    if source_index.len_bytes() > 4096 || (line_idx == ctx.start_line && ctx.top_skip > 0) {
        emit_indexed_line(ctx, state, &line, &source_index);
        return;
    }

    if line.is_stable {
        if let Some(cached_line) = state.line_cache.get(&line.cache_key) {
            let cached_line = cached_line.clone();
            emit_cached_line(ctx, &mut state.out, &line, cached_line);
            return;
        }
    }

    emit_legacy_line(ctx, state, &line);
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
