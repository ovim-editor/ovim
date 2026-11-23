//! Frame-wide context for one `render_buffer` pass and the per-line facts
//! derived from it.

use super::decorations::{line_decoration_cache_hash, pad_line_to, pad_line_to_styled};
use super::gutter::{build_gutter_line, visible_blame_brackets, BlameRow, GutterContext};
use super::overlays::find_matching_bracket_position;
use super::rows::RowOutput;
use super::viewport::HighlightShiftBuffers;
use super::WindowRenderContext;
use crate::editor::Editor;
use crate::syntax::{Theme, UiGroup};
use crate::ui::renderer::layout::BufferLayout;
use crate::ui::renderer::line_cache::{LineCacheFrame, LineCacheKey, LineRenderCache};
use ovim_core::buffer::Cursor;
use ovim_core::editor::decoration::{Decoration, DecorationPlacement, ProjectedDecorations};
use ovim_core::editor::{DiffReviewState, ProjectedDiagnostics, WrapMap};
use ovim_core::search::Search;
use ratatui::{style::Color, text::Line};
use std::sync::Arc;

/// The AI selection a code walkthrough highlights, reduced to the fields
/// rendering reads (the snapshot type is private to ovim-core).
#[derive(Clone, Copy)]
pub(super) struct AiSelectionSpan {
    pub(super) start_line: usize,
    pub(super) end_line: usize,
    pub(super) start_col: usize,
    pub(super) end_col: usize,
    pub(super) start_char: usize,
    pub(super) end_char: usize,
    pub(super) selection_mode: crate::mode::Mode,
}

impl AiSelectionSpan {
    pub(super) fn contains_line(&self, line_idx: usize) -> bool {
        line_idx >= self.start_line && line_idx <= self.end_line
    }

    pub(super) fn is_linewise(&self) -> bool {
        self.selection_mode == crate::mode::Mode::VisualLine
    }
}

/// Frame-wide, read-only inputs shared by every line of one `render_buffer`
/// pass: viewport geometry, the overlays that can touch a line, and the
/// per-frame projections of decorations and diagnostics.
pub(super) struct BufferRenderContext<'a> {
    pub(super) editor: &'a Editor,
    pub(super) buffer: &'a crate::buffer::Buffer,
    pub(super) theme: &'a Theme,
    pub(super) line_count: usize,
    // Viewport.
    pub(super) visible_lines: usize,
    pub(super) start_line: usize,
    /// Visual sub-row offset within `start_line`: the first `top_skip` wrapped
    /// rows of the top logical line are scrolled off the top edge. Only
    /// meaningful under soft wrap, so it is 0 otherwise.
    pub(super) top_skip: usize,
    pub(super) h_offset: usize,
    pub(super) wrap: bool,
    pub(super) has_wrap: bool,
    pub(super) wrap_map: Option<&'a WrapMap>,
    /// The document code-box (fixed at the layout's setting, i.e. textwidth
    /// in centered mode); `render_width` is how wide the line actually renders
    /// into. They differ in centered mode by exactly the diagnostic margin
    /// width.
    pub(super) text_width: usize,
    pub(super) render_width: usize,
    pub(super) tab_width: usize,
    // Overlays.
    pub(super) cursorline: bool,
    pub(super) cursor_line_idx: usize,
    pub(super) visual_selection: Option<((usize, usize), (usize, usize))>,
    pub(super) ai_selection: Option<AiSelectionSpan>,
    pub(super) walkthrough: bool,
    pub(super) current_search: Option<&'a Search>,
    pub(super) bracket_positions: Option<((usize, usize), (usize, usize))>,
    pub(super) is_md_file: bool,
    pub(super) markdown_conceal: bool,
    /// The branch diff review paints added/removed backgrounds per row; every
    /// other buffer skips this entirely.
    pub(super) diff_review_tints: Option<&'a DiffReviewState>,
    // Per-frame projections.
    pub(super) projected_decorations: Arc<ProjectedDecorations>,
    pub(super) projected_diagnostics: ProjectedDiagnostics,
    pub(super) buffer_id: u64,
    pub(super) cache_frame: LineCacheFrame,
    // Gutter (built inline so it matches wrap continuation rows).
    pub(super) has_gutter: bool,
    pub(super) gutter: GutterContext<'a>,
    pub(super) blame_brackets: Option<Vec<BlameRow>>,
}

impl<'a> BufferRenderContext<'a> {
    pub(super) fn new(
        editor: &'a Editor,
        theme: &'a Theme,
        layout: &BufferLayout,
        window_context: Option<&'a WindowRenderContext>,
    ) -> Self {
        let buffer = editor.buffer();
        // Use window-specific cursor if provided (for non-focused windows)
        let cursor = window_context
            .and_then(|ctx| ctx.cursor.as_ref())
            .unwrap_or_else(|| buffer.cursor());

        // Use Vim-compatible line count: trailing newline's phantom empty line
        // should not be rendered. The cursor is always bounded to real lines.
        let line_count = buffer.line_count();

        // Calculate visible range using scroll offset (not centering)
        // Use window-specific scroll offset if provided
        let visible_lines = layout.buffer_area.height as usize;
        let start_line = window_context
            .and_then(|ctx| ctx.scroll_offset)
            .unwrap_or_else(|| editor.scroll_offset());
        let top_skip = window_context
            .and_then(|ctx| ctx.scroll_subrow)
            .unwrap_or_else(|| editor.scroll_subrow());

        // Get horizontal viewport settings
        // Use window-specific horizontal offset if provided
        let h_offset = window_context
            .and_then(|ctx| ctx.horizontal_offset)
            .unwrap_or_else(|| editor.horizontal_offset());
        let wrap = editor.options.wrap;

        // Get visual selection if in visual mode
        let visual_selection = if editor.mode().is_visual() {
            editor.visual_selection()
        } else {
            None
        };
        let has_code_walkthrough = editor.ai_chat_has_pending_code_explanation();
        let ai_selection = if has_code_walkthrough {
            editor
                .ai_state
                .active_selection
                .as_ref()
                .map(|selection| AiSelectionSpan {
                    start_line: selection.start_line,
                    end_line: selection.end_line,
                    start_col: selection.start_col,
                    end_col: selection.end_col,
                    start_char: selection.start_char,
                    end_char: selection.end_char,
                    selection_mode: selection.selection_mode,
                })
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
        let bracket_positions = if editor.options.showmatch {
            matching_bracket_pair(buffer, cursor)
        } else {
            None
        };

        let tab_width = editor.indent_options().tab_width;
        let cursor_line_idx = cursor.line();
        let text_width = layout.text_width;
        let render_width = layout.render_width();
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
        let top_skip = if has_wrap { top_skip } else { 0 };
        let buffer_id = buffer.id();
        let cache_frame = LineCacheFrame {
            buffer_id,
            buffer_version: buffer.version(),
            highlight_generation: buffer.highlight_projection_generation(),
            h_offset,
            text_width,
            wrap,
            tab_width,
            markdown_conceal: editor.options.markdown_conceal,
        };

        // Pre-compute blame brackets for visible lines
        let blame_width = layout.blame_width;
        let blame_brackets = visible_blame_brackets(
            buffer,
            blame_width,
            start_line,
            line_count.min(start_line + visible_lines + 50),
        );

        let gutter = GutterContext {
            editor,
            buffer,
            theme,
            line_num_width: layout.line_num_width,
            cursor_line: cursor_line_idx,
            blame_width,
            fold_width: layout.fold_width,
            walkthrough_range,
        };

        let is_md_file = buffer
            .file_path()
            .map(|p| p.ends_with(".md"))
            .unwrap_or(false);

        let diff_review_tints = editor
            .diff_review()
            .filter(|state| state.buffer_id == editor.buffer().id());

        // Decorations projected through the edit log, shared with cursor math and
        // wrap layout (re-projected only after an edit or a decoration change).
        // Per-line lookups in the loop below read from a line-keyed map.
        let projected_decorations = if editor.ai_code_explanation_is_presenting_snapshot() {
            // Walkthrough code pages render an immutable virtual snapshot, not the
            // live LSP document. Reusing the editor-global decoration map here can
            // attach diagnostics and inlay hints from a different document state to
            // coincidentally matching lines in the snapshot.
            Default::default()
        } else {
            editor.projected_decorations()
        };
        // Raw diagnostics projected through the same edit log, once per frame:
        // the squiggle, gutter sign, and echo must land on the same line as the
        // projected EOL virtual text above. (OV-00328)
        let projected_diagnostics = if editor.ai_code_explanation_is_presenting_snapshot() {
            ProjectedDiagnostics::default()
        } else {
            editor.project_diagnostics()
        };

        Self {
            editor,
            buffer,
            theme,
            line_count,
            visible_lines,
            start_line,
            top_skip,
            h_offset,
            wrap,
            has_wrap,
            wrap_map,
            text_width,
            render_width,
            tab_width,
            cursorline: editor.options.cursorline,
            cursor_line_idx,
            visual_selection,
            ai_selection,
            walkthrough: walkthrough_range.is_some(),
            current_search,
            bracket_positions,
            is_md_file,
            markdown_conceal: editor.options.markdown_conceal,
            diff_review_tints,
            projected_decorations,
            projected_diagnostics,
            buffer_id,
            cache_frame,
            has_gutter: layout.gutter_width > 0,
            gutter,
            blame_brackets,
        }
    }

    /// Background for a diff-review added/removed tint.
    pub(super) fn diff_tint_color(&self, added: bool) -> Color {
        crate::key_convert::convert_core_color(self.theme.get_ui_color(if added {
            UiGroup::DiffAddedBg
        } else {
            UiGroup::DiffRemovedBg
        }))
    }

    /// A tint that runs to the end of the line keeps going through the
    /// padding, so a changed row reads as a full-width band.
    pub(super) fn trailing_background(&self, line_idx: usize) -> Option<Color> {
        self.diff_review_tints
            .and_then(|state| state.line_trailing_tint(line_idx))
            .map(|added| self.diff_tint_color(added))
    }

    /// Pads a row to the render width, extending a trailing tint if any.
    pub(super) fn pad_row(&self, row: &mut Line<'static>, trailing_background: Option<Color>) {
        match trailing_background {
            Some(color) => pad_line_to_styled(row, self.render_width, color),
            None => pad_line_to(row, self.render_width),
        }
    }

    pub(super) fn blank_row(&self) -> Line<'static> {
        Line::from(" ".repeat(self.render_width))
    }

    pub(super) fn gutter_row(
        &self,
        line_idx: usize,
        is_continuation: bool,
        line_diagnostics: &[lsp_types::Diagnostic],
    ) -> Line<'static> {
        build_gutter_line(
            &self.gutter,
            line_idx,
            is_continuation,
            line_diagnostics,
            self.blame_brackets
                .as_ref()
                .and_then(|brackets| brackets.get(line_idx - self.start_line)),
        )
    }

    /// Scrollbar thumb geometry in visual rows: (total rows, top row).
    pub(super) fn scrollbar_extent(&self) -> (usize, usize) {
        if self.has_wrap {
            self.wrap_map
                .map(|map| {
                    (
                        map.total_visual_lines(),
                        map.viewport_top_visual_row(self.start_line, self.top_skip),
                    )
                })
                .unwrap_or((self.line_count, self.start_line))
        } else {
            (self.line_count, self.start_line)
        }
    }

    /// Facts about one logical line that every pipeline needs before it
    /// chooses how to draw it.
    pub(super) fn line_frame(&self, line_idx: usize) -> LineFrame<'_> {
        // Determine upfront if this line has transient overlays that prevent caching.
        let has_visual_on_line = self
            .visual_selection
            .map(|((sl, _), (el, _))| line_idx >= sl && line_idx <= el)
            .unwrap_or(false);
        let is_cursor_line = self.cursorline && line_idx == self.cursor_line_idx;
        let is_cursor_line_for_conceal =
            line_idx == self.cursor_line_idx && self.markdown_conceal && self.is_md_file;
        let has_yank_flash = self
            .editor
            .yank_flash()
            .is_some_and(|f| f.contains_line(line_idx));
        let diagnostics = self.projected_diagnostics.for_line(line_idx);
        let has_bracket = self
            .bracket_positions
            .is_some_and(|((l1, _), (l2, _))| line_idx == l1 || line_idx == l2);
        let has_search = self.current_search.is_some();
        let has_ai_selection_on_line = self
            .ai_selection
            .map(|selection| selection.contains_line(line_idx))
            .unwrap_or(false);
        let is_stable = !has_visual_on_line
            && !is_cursor_line
            && !is_cursor_line_for_conceal
            && !has_yank_flash
            && !has_bracket
            && !has_search
            && !has_ai_selection_on_line;

        // Per-line decoration hash from the per-frame projection. Lets the
        // line cache invalidate only the lines whose decorations actually
        // changed (vs. the previous global generation counter that wiped
        // every cached line on any LSP push).
        let dec_hash = line_decoration_cache_hash(
            &self.projected_decorations,
            &self.projected_diagnostics,
            line_idx,
        );
        LineFrame {
            line_idx,
            has_visual_on_line,
            is_cursor_line,
            is_cursor_line_for_conceal,
            is_stable,
            diagnostics,
            dec_hash,
            cache_key: self.cache_frame.key(line_idx, dec_hash),
        }
    }
}

/// Per-line facts shared by both line pipelines.
pub(super) struct LineFrame<'a> {
    pub(super) line_idx: usize,
    pub(super) has_visual_on_line: bool,
    /// The cursorline option is on and the cursor is on this line.
    pub(super) is_cursor_line: bool,
    /// Markdown conceal is skipped on the cursor line so editing isn't blind.
    pub(super) is_cursor_line_for_conceal: bool,
    /// No transient overlay prevents caching the rendered line.
    pub(super) is_stable: bool,
    /// Diagnostics starting on this line, for the gutter sign.
    pub(super) diagnostics: &'a [lsp_types::Diagnostic],
    pub(super) dec_hash: u64,
    pub(super) cache_key: LineCacheKey,
}

/// Mutable per-frame state threaded through the line pipelines.
pub(super) struct RenderState<'a> {
    pub(super) line_cache: &'a mut LineRenderCache,
    /// Reusable scratch buffers for shift_highlights_for_viewport — avoids
    /// allocating two Vec<usize> per visible line per frame.
    pub(super) hl_shift_buffers: HighlightShiftBuffers,
    pub(super) out: RowOutput,
}

/// A line's inline and end-of-line decorations, split by placement.
pub(super) struct LineDecorations<'a> {
    pub(super) inline: Vec<&'a Decoration>,
    pub(super) eol: Vec<&'a Decoration>,
}

impl<'a> LineDecorations<'a> {
    pub(super) fn split(decorations: &'a [Decoration]) -> Self {
        Self {
            inline: decorations
                .iter()
                .filter(|d| matches!(d.placement, DecorationPlacement::Inline { .. }))
                .collect(),
            eol: decorations
                .iter()
                .filter(|d| matches!(d.placement, DecorationPlacement::EndOfLine { .. }))
                .collect(),
        }
    }
}

/// Finds the bracket pair to highlight as (cursor position, matching
/// position), both as (line, char column).
fn matching_bracket_pair(
    buffer: &crate::buffer::Buffer,
    cursor: &Cursor,
) -> Option<((usize, usize), (usize, usize))> {
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
}
