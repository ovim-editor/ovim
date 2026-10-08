//! Viewport height, scroll offsets and the soft-wrap map.

use super::{decoration, wrap_map, Editor, WrapMap};
use crate::unicode::{grapheme_to_char_col, GraphemeCol};

impl Editor {
    /// Sets the viewport height (called from UI layer)
    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport.viewport_height = height;
    }

    /// Caches the buffer layout from the last render (for mouse coordinate conversion)
    pub fn set_last_layout(
        &mut self,
        buffer_area: crate::Rect,
        gutter_width: usize,
        text_width: usize,
        blame_width: usize,
    ) {
        self.render_cache.last_buffer_area = Some(buffer_area);
        self.render_cache.last_gutter_width = gutter_width;
        self.render_cache.last_text_width = text_width;
        self.render_cache.last_blame_width = blame_width;
    }

    /// Gets the viewport height
    pub fn viewport_height(&self) -> usize {
        self.viewport.viewport_height
    }

    /// Gets the scroll offset (top visible line)
    pub fn scroll_offset(&self) -> usize {
        // If we have a window manager, use the focused window's scroll offset
        // This allows viewport commands (zz, zt, zb) to control scrolling
        if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                return window.scroll_offset();
            }
        }
        // Fall back to editor-level scroll offset for headless/test mode
        self.viewport.scroll_offset
    }

    /// Gets the visual sub-row offset within the top visible logical line.
    ///
    /// Mirrors [`scroll_offset`](Self::scroll_offset): the focused window's value
    /// when there's a window manager, otherwise the editor-global fallback.
    /// Always 0 when soft wrap is off.
    pub fn scroll_subrow(&self) -> usize {
        if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                return window.scroll_subrow();
            }
        }
        self.viewport.scroll_subrow
    }

    /// Gets a reference to the wrap map for the active viewport (the focused
    /// window when there's a window manager, otherwise the editor-global slot
    /// used in headless / no-window-manager contexts).
    ///
    /// This is the map the cursor overlay and focused-pane content render
    /// against; non-focused split panes use their own via `Window::wrap_map`.
    pub fn wrap_map(&self) -> Option<&WrapMap> {
        match self
            .window_manager
            .as_ref()
            .and_then(|wm| wm.focused_window())
        {
            Some(window) => window.wrap_map(),
            None => self.viewport.wrap_map.as_ref(),
        }
    }

    /// Ensures the active viewport's wrap map is built and up-to-date for
    /// `text_width` columns — the focused window's map when there's a window
    /// manager, otherwise the editor-global `ViewportState::wrap_map`. Called
    /// from the rendering layer before drawing wrapped lines.
    pub fn ensure_wrap_map(&mut self, text_width: usize) {
        if let Some(focused_idx) = self
            .window_manager
            .as_ref()
            .map(|wm| wm.focused_window_index())
        {
            self.ensure_wrap_map_for_window(focused_idx, text_width);
            return;
        }
        let existing = self.viewport.wrap_map.take();
        let (map, generation) = self.refresh_wrap_map(
            text_width,
            existing,
            self.viewport.wrap_decoration_generation,
        );
        self.viewport.wrap_map = map;
        self.viewport.wrap_decoration_generation = generation;
    }

    /// Refresh the requested window without cloning its cached line geometry.
    pub fn ensure_wrap_map_for_window(&mut self, window_idx: usize, text_width: usize) {
        let Some(window) = self
            .window_manager
            .as_mut()
            .and_then(|wm| wm.get_window_mut(window_idx))
        else {
            return;
        };
        let generation = window.wrap_decoration_generation();
        let existing = window.wrap_map_mut().take().map(|map| *map);
        let (map, generation) = self.refresh_wrap_map(text_width, existing, generation);
        if let Some(window) = self
            .window_manager
            .as_mut()
            .and_then(|wm| wm.get_window_mut(window_idx))
        {
            *window.wrap_map_mut() = map.map(Box::new);
            window.set_wrap_decoration_generation(generation);
        }
    }

    /// The current buffer's decorations projected through its edit log, shared
    /// by wrap layout, cursor math and rendering until the text or the
    /// decorations change.
    pub fn projected_decorations(&self) -> std::sync::Arc<decoration::ProjectedDecorations> {
        let buffer = self.buffer();
        self.decorations.projected(
            buffer.id(),
            buffer.version(),
            buffer.rope(),
            buffer.edit_log(),
        )
    }

    /// Text edits replay line splices onto the existing count index. Geometry
    /// changes or lost mutation history rebuild safely, preserving per-window policy.
    fn refresh_wrap_map(
        &self,
        text_width: usize,
        mut existing: Option<WrapMap>,
        existing_dec_gen: u64,
    ) -> (Option<WrapMap>, u64) {
        // Only inline decorations change how lines wrap.
        let dec_gen = self.decorations.inline_generation();
        if !self.options.wrap {
            return (None, dec_gen);
        }
        let width = text_width.max(1);
        let tab_width = self.indent_options().tab_width.max(1);
        let buffer = self.buffer();
        let version = buffer.version();
        let line_count = buffer.rope().len_lines();
        let conceal_active = self.options.markdown_conceal
            && buffer.file_path().is_some_and(|path| path.ends_with(".md"));
        let cursor_line = buffer.cursor().line();
        let conceal_cursor_line = conceal_active.then_some(cursor_line);
        // Lines hidden by closed folds take no visual rows.
        let hidden_ranges = buffer.fold_manager().hidden_ranges();
        let projected = self.projected_decorations();
        let make_layout = |line: usize| {
            let mut transform = None;
            let mut links = Vec::new();
            let index = if conceal_active && line != cursor_line {
                let raw = buffer.line_text(line).unwrap_or_default();
                let spans = crate::markdown_conceal::scan_markdown_conceal(&raw);
                if spans.is_empty() {
                    buffer.line_index(line)
                } else {
                    let view = crate::markdown_conceal::apply_conceal(&raw, &spans);
                    links = crate::markdown_conceal::extract_concealed_links(&spans, &view);
                    let index = crate::text_index::LineIndex::from_text(&view.text);
                    transform = Some(std::sync::Arc::new(view));
                    index
                }
            } else {
                buffer.line_index(line)
            };
            let line_start = buffer.rope().line_to_char(line);
            let mut inline: Vec<(usize, std::sync::Arc<str>)> = projected
                .for_line(line)
                .iter()
                .filter_map(|decoration| match decoration.placement {
                    decoration::DecorationPlacement::Inline { char_offset } => Some((
                        char_offset.saturating_sub(line_start),
                        decoration.text.as_str().into(),
                    )),
                    decoration::DecorationPlacement::EndOfLine { .. } => None,
                })
                .collect();
            // Decoration anchors originate in raw character space; conceal
            // changes that space before wrapping and must transform them too.
            if let Some(view) = transform.as_ref() {
                let raw = buffer.line_index(line);
                for (column, _) in &mut inline {
                    let byte = raw.char_to_byte(*column);
                    *column = view
                        .src_to_view
                        .get(byte)
                        .copied()
                        .unwrap_or(index.len_chars());
                }
            }
            wrap_map::IndexedWrapLine {
                layout: std::sync::Arc::new(
                    crate::line_layout::IndexedLineLayout::with_inline_text(
                        index,
                        width,
                        tab_width,
                        inline.into(),
                    ),
                ),
                transform,
                links: links.into(),
            }
        };
        if let Some(map) = existing.as_mut() {
            let same_policy = map.source_buffer_id() == Some(buffer.id())
                && map.wrap_width() == width
                && map.tab_width() == tab_width
                && map.conceal_cursor_line().is_some() == conceal_cursor_line.is_some();
            if same_policy {
                if let Some(changes) = buffer.line_changes_since(map.buffer_version()) {
                    // Lines of the map's snapshot carried through the edits
                    // into final line coordinates. A replaced line is already
                    // dirty in the journal.
                    let carry = |line: usize| {
                        changes.iter().try_fold(line, |line, change| {
                            let end = change.start_line + change.old_line_count;
                            if line < change.start_line {
                                Some(line)
                            } else if line >= end {
                                Some(line - change.old_line_count + change.new_line_count)
                            } else {
                                None
                            }
                        })
                    };
                    // The old revealed line belongs to the map's snapshot.
                    // Carry it through structural edits before invalidating
                    // reveal/conceal geometry in final line coordinates.
                    let old_revealed = map.conceal_cursor_line().and_then(carry);
                    let mut extra = Vec::new();
                    if old_revealed != conceal_cursor_line {
                        extra.extend(old_revealed);
                        extra.extend(conceal_cursor_line);
                    }
                    // Changed inline decorations only affect the lines that had
                    // one and the lines that have one now.
                    if existing_dec_gen != dec_gen {
                        extra.extend(map.lines_with_inline_text().filter_map(carry));
                        extra.extend(projected.inline_lines());
                    }
                    if map.refresh_indexed(&changes, line_count, version, &extra, make_layout) {
                        map.set_conceal_cursor_line(conceal_cursor_line);
                        map.set_hidden_ranges(hidden_ranges);
                        return (existing, dec_gen);
                    }
                }
            }
        }
        let mut map = WrapMap::from_layouts(
            (0..line_count).map(make_layout).collect(),
            width,
            tab_width,
            version,
        );
        map.set_source_buffer_id(buffer.id());
        map.set_conceal_cursor_line(conceal_cursor_line);
        map.set_hidden_ranges(hidden_ranges);
        (Some(map), dec_gen)
    }

    fn cursor_grapheme_to_char_col(&self, line_idx: usize, grapheme_col: GraphemeCol) -> usize {
        self.buffer()
            .line_index(line_idx)
            .grapheme_to_char(grapheme_col)
            .0
    }

    pub(super) fn cursor_visual_position(
        &self,
        line: usize,
        col: GraphemeCol,
    ) -> Option<(usize, usize)> {
        let map = self.wrap_map()?;
        let char_col = self.buffer().line_index(line).grapheme_to_char(col).0;
        if let Some(layout) = map.line_layout(line) {
            let position = layout.position_for_char(char_col);
            Some((map.logical_to_visual(line) + position.row, position.column))
        } else {
            let text = self.cursor_line_text(line);
            let display = self
                .buffer()
                .line_index(line)
                .char_to_display(char_col, self.indent_options().tab_width);
            Some(map.cursor_to_visual(line, display, &text))
        }
    }

    fn cursor_line_text(&self, line_idx: usize) -> String {
        self.buffer()
            .line_text(line_idx)
            .unwrap_or_default()
            .to_string()
    }

    /// Gets the horizontal scroll offset (leftmost visible column)
    pub fn horizontal_offset(&self) -> usize {
        if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                return window.horizontal_offset();
            }
        }
        // Fall back to 0 for headless/test mode
        0
    }

    /// Updates scroll offset to keep cursor visible
    ///
    /// Uses scrolloff for comfortable cursor positioning during normal movements.
    /// Viewport commands (zt, zz, zb) can override this by requesting viewport preservation.
    pub fn update_scroll_offset(&mut self) {
        // Skip if viewport command just ran - it has full control over positioning
        if self.viewport.should_preserve_after_input() {
            return;
        }

        // Reuse indexed geometry and update only changed lines before scrolling.
        // Macro playback can edit without an intervening render pass.
        if self.options.wrap {
            let width = self.wrap_map().map(WrapMap::wrap_width).or_else(|| {
                (self.render_cache.last_text_width > 0).then_some(self.render_cache.last_text_width)
            });
            if let Some(width) = width {
                self.ensure_wrap_map(width);
            }
        }
        let cursor_line = self.buffer().cursor().line();
        let visible_lines = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.height() as usize).max(1)
            } else {
                self.viewport.viewport_height.max(1)
            }
        } else {
            self.viewport.viewport_height.max(1)
        };
        let current_offset = self.scroll_offset();
        let max_line = self.buffer().line_count().saturating_sub(1);

        // Only use wrap-aware scrolling if the wrap map covers the current buffer.
        // After edits (e.g. `o` inserting a line) the map is stale until the next
        // render pass rebuilds it.  Using stale data causes cursor_to_visual to
        // return 0 for the new line, jumping the viewport to the top.
        //
        // Consult the *active* map via `wrap_map()` — the focused window owns its
        // map when there's a window manager (the common case); `self.viewport.
        // wrap_map` is only the no-window-manager fallback slot.
        let len_lines = self.buffer().rope().len_lines();
        let wrap_map_usable =
            self.options.wrap && self.wrap_map().is_some_and(|m| m.line_count() >= len_lines);

        // In wrap mode, each logical line can consume multiple visual rows. Clamping using
        // logical line counts can prevent scrolling far enough to reveal the final logical
        // lines when a wrapped line appears near EOF. Instead, derive the maximum scroll
        // offset from total visual rows.
        let wrap_width_known = self.wrap_map().is_some() || self.render_cache.last_text_width > 0;

        let max_scroll = if wrap_map_usable {
            self.wrap_map()
                .map(|m| Self::compute_wrap_max_scroll_offset(m, visible_lines, max_line))
                .unwrap_or_else(|| max_line.saturating_sub(visible_lines.saturating_sub(1)))
        } else if self.options.wrap && wrap_width_known {
            // Wrap enabled but map stale: allow scrolling all the way to the last logical line.
            // This prevents the viewport from getting "stuck" above EOF between the edit and
            // the next render pass (which rebuilds the wrap map).
            max_line
        } else {
            max_line.saturating_sub(visible_lines.saturating_sub(1))
        };

        // Clamp scrolloff so top and bottom margins don't overlap.
        // When scrolloff >= ceil(visible_lines/2), both margins would claim
        // the same lines, causing the viewport to oscillate on every movement.
        let scrolloff = self
            .options
            .scrolloff
            .min(visible_lines.saturating_sub(1) / 2);

        // Calculate new scroll offset (and, in wrap mode, the visual sub-row
        // offset within the top logical line).
        let current_subrow = self.scroll_subrow();
        let mut new_offset;
        let mut new_subrow = 0usize;
        // Set when the wrap path has already bounded the position in visual-row
        // space; skip the logical `max_scroll` clamp below, which knows nothing
        // about sub-rows and would desync `new_offset` from `new_subrow`.
        let mut visual_clamped = false;

        if wrap_map_usable {
            if let Some(wrap_map) = self.wrap_map() {
                // Wrap-aware scrolling: work in absolute visual rows.
                let (cursor_visual_row, _) = self
                    .cursor_visual_position(cursor_line, self.buffer().cursor().col())
                    .unwrap_or((0, 0));
                let viewport_visual_start =
                    wrap_map.logical_to_visual(current_offset) + current_subrow;

                if cursor_visual_row < viewport_visual_start + scrolloff {
                    // Cursor above the top margin — scroll up. Begin at the start
                    // of the logical line containing the target row (conservative:
                    // a line boundary, matching long-standing behaviour).
                    let target_visual = cursor_visual_row.saturating_sub(scrolloff);
                    let (new_line, _) = wrap_map.visual_to_logical(target_visual);
                    new_offset = new_line;
                    new_subrow = 0;
                } else if cursor_visual_row + scrolloff >= viewport_visual_start + visible_lines {
                    // Cursor below the bottom margin — scroll down by exact visual
                    // rows, beginning *partway into* a wrapped line when needed so
                    // a logical line taller than the viewport stays fully reachable
                    // (its tail no longer falls off the bottom edge).
                    let total_visual = wrap_map.total_visual_lines();
                    let max_visual_start = total_visual.saturating_sub(visible_lines);
                    let target_visual = (cursor_visual_row + scrolloff + 1)
                        .saturating_sub(visible_lines)
                        .min(max_visual_start);
                    let (new_line, sub_line) = wrap_map.visual_to_logical(target_visual);
                    new_offset = new_line;
                    new_subrow = sub_line;
                    visual_clamped = true;
                } else {
                    // Cursor already visible — hold position, including the sub-row
                    // offset so a mid-line scroll isn't snapped back to the top.
                    new_offset = current_offset;
                    new_subrow = current_subrow;
                }
            } else {
                // Wrap enabled but no wrap map yet — use logical line fallback
                new_offset = Self::compute_logical_scroll_offset(
                    cursor_line,
                    current_offset,
                    visible_lines,
                    scrolloff,
                );
            }
        } else if self.options.wrap {
            // Wrap enabled but wrap map stale (e.g. immediately after inserting/removing
            // newlines). Do a cheap on-the-fly wrap-aware scroll calculation limited to
            // the current viewport region so cursor visibility stays correct until the
            // next render pass rebuilds the wrap map.
            new_offset = self.compute_fallback_wrap_scroll_offset(
                cursor_line,
                current_offset,
                visible_lines,
                scrolloff,
            );
        } else {
            // Closed folds hide lines: scroll in visible-line space.
            let folds = self.buffer().fold_manager();
            if folds.hidden_ranges().is_empty() {
                new_offset = Self::compute_logical_scroll_offset(
                    cursor_line,
                    current_offset,
                    visible_lines,
                    scrolloff,
                );
            } else {
                let visible_offset = Self::compute_logical_scroll_offset(
                    folds.visible_index(cursor_line),
                    folds.visible_index(current_offset),
                    visible_lines,
                    scrolloff,
                );
                new_offset = folds.line_at_visible_index(visible_offset);
            }
        };

        // Clamp to max_scroll only when the viewport actually needs to move for
        // cursor visibility.  When the cursor is already visible the scroll paths
        // return `current_offset` unchanged — clamping that would snap away the
        // deliberate positioning set by viewport commands (zt/zz/zb) near EOF.
        // The wrap down-scroll path has already clamped in visual-row space.
        new_offset = if !visual_clamped && new_offset != current_offset {
            new_offset.min(max_scroll)
        } else {
            new_offset
        };

        // Update both editor-level and window-level scroll offsets
        self.viewport.scroll_offset = new_offset;
        self.viewport.scroll_subrow = new_subrow;

        // Extract cursor column and options before mutably borrowing window_manager
        // Convert char column to display column for proper horizontal scrolling
        let cursor_line = self.buffer().cursor().line();
        let tab_width = self.indent_options().tab_width;
        let cursor_char_col =
            self.cursor_grapheme_to_char_col(cursor_line, self.buffer().cursor().col());
        let cursor_display_col = {
            let raw_col = self
                .buffer()
                .line_index(cursor_line)
                .char_to_display(cursor_char_col, tab_width);
            // Include inline decoration widths (inlay hints) so horizontal
            // scroll keeps the *decorated* cursor position visible.  Without
            // this, h_offset is set from raw text only, but the renderer adds
            // decoration widths to the cursor, causing it to float right.
            raw_col
                + self.projected_decorations().inline_width_before(
                    cursor_line,
                    cursor_char_col,
                    self.buffer().rope(),
                )
        };
        let wrap = self.options.wrap;
        let sidescroll = self.options.sidescroll;
        let sidescrolloff = self.options.sidescrolloff;
        let text_width = self.render_cache.last_text_width;

        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_position(new_offset, new_subrow);

                // Update horizontal scroll offset to keep cursor visible horizontally
                if window.ensure_cursor_visible_horizontal(
                    cursor_display_col,
                    text_width,
                    wrap,
                    sidescroll,
                    sidescrolloff,
                ) {
                    // Horizontal offset changed, mark for re-render
                    self.mark_dirty();
                }
            }
        }
    }

    /// Computes scroll offset using logical line counting (non-wrap path).
    /// Caller is responsible for clamping scrolloff so top/bottom margins
    /// don't overlap (scrolloff <= (visible_lines - 1) / 2).
    fn compute_logical_scroll_offset(
        cursor_line: usize,
        current_offset: usize,
        visible_lines: usize,
        scrolloff: usize,
    ) -> usize {
        if cursor_line < current_offset + scrolloff {
            cursor_line.saturating_sub(scrolloff)
        } else if cursor_line + scrolloff >= current_offset + visible_lines {
            cursor_line + scrolloff + 1 - visible_lines
        } else {
            current_offset
        }
    }

    /// Compute the maximum logical scroll offset in wrap mode based on total visual rows.
    ///
    /// The renderer can only start rendering at a logical line boundary (not a wrapped
    /// sub-line), so if the ideal max visual start lands mid-line, we advance to the
    /// next logical line to ensure the final logical lines can still be reached.
    pub(super) fn compute_wrap_max_scroll_offset(
        wrap_map: &WrapMap,
        visible_rows: usize,
        max_line: usize,
    ) -> usize {
        let visible_rows = visible_rows.max(1);
        let total_visual = wrap_map.total_visual_lines();
        if total_visual <= visible_rows {
            return 0;
        }
        let max_visual_start = total_visual - visible_rows;
        let (line, sub_line) = wrap_map.visual_to_logical(max_visual_start);
        let candidate = if sub_line > 0 {
            line.saturating_add(1)
        } else {
            line
        };
        candidate.min(max_line)
    }

    fn compute_fallback_wrap_scroll_offset(
        &self,
        cursor_line: usize,
        current_offset: usize,
        visible_rows: usize,
        scrolloff: usize,
    ) -> usize {
        let visible_rows = visible_rows.max(1);
        let scrolloff = scrolloff.min(visible_rows.saturating_sub(1) / 2);

        let wrap_width = if let Some(map) = self.viewport.wrap_map.as_ref() {
            map.wrap_width()
        } else if self.render_cache.last_text_width > 0 {
            self.render_cache.last_text_width
        } else {
            // No reliable wrap width in headless/test mode — fall back to logical scrolling.
            return Self::compute_logical_scroll_offset(
                cursor_line,
                current_offset,
                visible_rows,
                scrolloff,
            );
        }
        .max(1);
        let tab_width = self.indent_options().tab_width;

        let line_count = self.buffer().line_count();
        let max_line = line_count.saturating_sub(1);

        // If cursor is logically above the viewport, scroll to it.
        if cursor_line < current_offset {
            return cursor_line;
        }

        // Compute cursor sub-line within its logical line.
        let line_text = self
            .buffer()
            .line_text(cursor_line)
            .unwrap_or_default()
            .to_string();
        let cursor_char_col = grapheme_to_char_col(&line_text, self.buffer().cursor().col());
        let cursor_display_col =
            crate::display::char_col_to_display_col(&line_text, cursor_char_col.0, tab_width);
        let rope = self.buffer().rope();
        let projected = self.projected_decorations();
        let cursor_inline_widths = projected.inline_decorations_for_line(cursor_line, rope);
        let cursor_display_col = cursor_display_col
            + projected.inline_width_before(cursor_line, cursor_char_col.0, rope);
        let cursor_subline = Self::cursor_subline_in_wrapped_line(
            &line_text,
            cursor_display_col,
            wrap_width,
            tab_width,
            &cursor_inline_widths,
        );

        // Fast path: if cursor is logically far below the viewport, just position it near bottom.
        let logical_view_end = current_offset + visible_rows.saturating_sub(1);
        if cursor_line > logical_view_end {
            let rows_above_cursor = visible_rows.saturating_sub(scrolloff + 1);
            return Self::top_offset_for_wrapped_cursor(
                self,
                cursor_line,
                cursor_subline,
                rows_above_cursor,
                wrap_width,
                tab_width,
                true,
            )
            .min(max_line);
        }

        // Cursor is logically within the viewport — check if it is visually within.
        let mut rows_from_top = 0usize;
        for line in current_offset..cursor_line {
            let text = self
                .buffer()
                .line_text(line)
                .unwrap_or_default()
                .to_string();
            rows_from_top += crate::wrap::visual_line_count(&text, wrap_width, tab_width);
            if rows_from_top > visible_rows + scrolloff + 5 {
                break;
            }
        }
        rows_from_top += cursor_subline;

        if rows_from_top < scrolloff {
            // Scroll up so cursor lands at scrolloff from top.
            Self::top_offset_for_wrapped_cursor(
                self,
                cursor_line,
                cursor_subline,
                scrolloff,
                wrap_width,
                tab_width,
                false,
            )
            .min(max_line)
        } else if rows_from_top + scrolloff >= visible_rows {
            // Scroll down so cursor lands at (visible_rows - scrolloff - 1).
            let rows_above_cursor = visible_rows.saturating_sub(scrolloff + 1);
            Self::top_offset_for_wrapped_cursor(
                self,
                cursor_line,
                cursor_subline,
                rows_above_cursor,
                wrap_width,
                tab_width,
                true,
            )
            .min(max_line)
        } else {
            current_offset
        }
    }

    fn cursor_subline_in_wrapped_line(
        line_text: &str,
        cursor_display_col: usize,
        wrap_width: usize,
        tab_width: usize,
        inline_widths: &[(usize, usize)],
    ) -> usize {
        // Delegate to the shared column-level walk so a cursor sitting inside
        // a split tab or decoration is attributed to the row that actually
        // renders it (OV-00275). The old wrap-point re-walk operated at raw
        // char granularity and could be off by one sub-line there.
        crate::wrap::visual_position_for_flat_col(
            line_text,
            cursor_display_col,
            wrap_width,
            tab_width,
            inline_widths,
        )
        .0
    }

    fn top_offset_for_wrapped_cursor(
        &self,
        cursor_line: usize,
        cursor_subline: usize,
        rows_above_cursor: usize,
        wrap_width: usize,
        tab_width: usize,
        advance_if_mid_line: bool,
    ) -> usize {
        // If the target visual start would land within the cursor's own wrapped line,
        // we can't start mid-line, so start at the cursor's logical line.
        if rows_above_cursor <= cursor_subline {
            return cursor_line;
        }

        let mut remaining = rows_above_cursor.saturating_sub(cursor_subline);
        let mut line = cursor_line;

        while line > 0 {
            line -= 1;
            let text = self
                .buffer()
                .line_text(line)
                .unwrap_or_default()
                .to_string();
            let count = crate::wrap::visual_line_count(&text, wrap_width, tab_width);

            if remaining == 0 {
                return line;
            }

            if remaining < count {
                return if advance_if_mid_line && remaining > 0 {
                    line.saturating_add(1)
                } else {
                    line
                };
            }

            remaining = remaining.saturating_sub(count);
            if remaining == 0 {
                return line;
            }
        }

        0
    }

    /// Calculates half-page scroll amount (Ctrl-D / Ctrl-U).
    ///
    /// Uses `options.scroll` when set. Otherwise it's half the *focused window's*
    /// height — not the whole buffer area — so that in a split, Ctrl-D/U scroll
    /// half of the focused pane rather than half of the entire editor.
    pub fn half_page_scroll(&self) -> usize {
        if let Some(scroll) = self.options.scroll {
            return scroll;
        }
        let height = self
            .window_manager
            .as_ref()
            .and_then(|wm| wm.focused_window())
            .map(|window| window.height() as usize)
            .unwrap_or(self.viewport.viewport_height);
        (height / 2).max(1)
    }
}
