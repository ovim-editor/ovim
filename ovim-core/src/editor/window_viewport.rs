//! Window splits and viewport scrolling

use super::{Editor, Motions, SplitDirection, WindowManager};

impl Editor {
    // === Window Management ===

    /// Gets a reference to the window manager
    pub fn window_manager(&self) -> Option<&WindowManager> {
        self.window_manager.as_ref()
    }

    /// Gets a mutable reference to the window manager
    pub fn window_manager_mut(&mut self) -> Option<&mut WindowManager> {
        self.window_manager.as_mut()
    }

    /// Initializes the window manager with the current viewport dimensions
    /// Call this once viewport size is known (typically from UI layer)
    pub fn init_window_manager(&mut self, width: u16, height: u16) {
        if self.window_manager.is_none() {
            self.window_manager = Some(WindowManager::new(0, width, height));
        }
    }

    /// Splits the current window horizontally (creates window above/below)
    pub fn split_window_horizontal(&mut self) {
        // Initialize window manager if needed (fallback dimensions)
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        // Get current cursor position to copy to new window
        let cursor = *self.buffer().cursor();
        let scroll_offset = self.scroll_offset();
        let scroll_subrow = self.scroll_subrow();
        let wrap_reflow_cursor_row = self.visible_cursor_row_before_split(cursor);

        if let Some(wm) = &mut self.window_manager {
            wm.split_focused_with_cursor(
                SplitDirection::Horizontal,
                self.current_buffer_index,
                cursor,
                scroll_offset,
                scroll_subrow,
                wrap_reflow_cursor_row,
            );
        }
    }

    /// Splits the current window vertically (creates window left/right)
    pub fn split_window_vertical(&mut self) {
        // Initialize window manager if needed (fallback dimensions)
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        // Get current cursor position to copy to new window
        let cursor = *self.buffer().cursor();
        let scroll_offset = self.scroll_offset();
        let scroll_subrow = self.scroll_subrow();
        let wrap_reflow_cursor_row = self.visible_cursor_row_before_split(cursor);

        if let Some(wm) = &mut self.window_manager {
            wm.split_focused_with_cursor(
                SplitDirection::Vertical,
                self.current_buffer_index,
                cursor,
                scroll_offset,
                scroll_subrow,
                wrap_reflow_cursor_row,
            );
        }
    }

    /// Return the cursor's current screen row only when it is already visible.
    /// A split copies explicit scroll positions unchanged; it requests a
    /// reflow repair only for the normal cursor-following viewport state.
    fn visible_cursor_row_before_split(&mut self, cursor: crate::buffer::Cursor) -> Option<usize> {
        if !self.options.wrap {
            return None;
        }
        if let Some(width) = self.wrap_map().map(super::WrapMap::wrap_width) {
            self.ensure_wrap_map(width);
        }
        let (cursor_visual_row, _) = self.cursor_visual_position(cursor.line(), cursor.col())?;
        let map = self.wrap_map()?;
        let viewport_top = map.viewport_top_visual_row(self.scroll_offset(), self.scroll_subrow());
        // A cursor above an explicitly scrolled viewport has no screen-row
        // anchor to preserve. Saturating here would incorrectly treat it as
        // the first visible row and override that explicit scroll position.
        let screen_row = cursor_visual_row.checked_sub(viewport_top)?;
        (screen_row < self.focused_visible_rows()).then_some(screen_row)
    }

    /// Consume a split-time wrap anchor after this window has a map at its
    /// actual pane width. The split copied the logical scroll position (and
    /// its sub-row), which is meaningful at the old width but can place a
    /// deeply wrapped cursor outside a reflowed pane. Keep the cursor at its
    /// previous on-screen row, clamped to the new pane height.
    ///
    /// This is intentionally one-shot: ordinary resizing and explicit
    /// scrolling continue to use their existing viewport behavior.
    pub fn repair_pending_split_wrap_viewport(&mut self, window_idx: usize, visible_rows: usize) {
        let Some((anchor, cursor)) = self.window_manager.as_ref().and_then(|manager| {
            manager.get_window(window_idx).and_then(|window| {
                window
                    .pending_wrap_reflow_cursor_row()
                    .map(|anchor| (anchor, *window.cursor()))
            })
        }) else {
            return;
        };

        let char_col = self
            .buffer()
            .line_index(cursor.line())
            .grapheme_to_char(cursor.col())
            .0;
        let visible_rows = visible_rows.max(1);
        let Some((scroll_offset, scroll_subrow)) =
            self.window_manager.as_ref().and_then(|manager| {
                let window = manager.get_window(window_idx)?;
                let map = window.wrap_map()?;
                let position = map.line_layout(cursor.line())?.position_for_char(char_col);
                let cursor_row = map.logical_to_visual(cursor.line()) + position.row;
                let requested_top = cursor_row.saturating_sub(anchor.min(visible_rows - 1));
                let max_top = map.total_visual_lines().saturating_sub(visible_rows);
                Some(map.visual_to_logical(requested_top.min(max_top)))
            })
        else {
            // The map has not yet been built for this pane. Keep the pending
            // anchor for the real pane-width render instead of guessing.
            return;
        };

        let is_focused = self
            .window_manager
            .as_ref()
            .is_some_and(|manager| manager.focused_window_index() == window_idx);
        if let Some(window) = self
            .window_manager
            .as_mut()
            .and_then(|manager| manager.get_window_mut(window_idx))
        {
            window.set_scroll_position(scroll_offset, scroll_subrow);
            window.set_pending_wrap_reflow_cursor_row(None);
        }
        if is_focused {
            self.viewport.scroll_offset = scroll_offset;
            self.viewport.scroll_subrow = scroll_subrow;
        }
    }

    /// Saves the buffer's cursor to the currently focused window
    fn save_cursor_to_focused_window(&mut self) {
        let cursor = *self.buffer().cursor();
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window
                    .cursor_mut()
                    .set_position(cursor.line(), cursor.col());
            }
        }
    }

    /// Restores the focused window's cursor to the buffer
    fn restore_cursor_from_focused_window(&mut self) {
        let (line, col) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                let cursor = window.cursor();
                (cursor.line(), cursor.col())
            } else {
                return;
            }
        } else {
            return;
        };
        self.buffer_mut().cursor_mut().set_position(line, col);
    }

    /// Moves focus to the next window
    pub fn focus_next_window(&mut self) {
        self.save_cursor_to_focused_window();
        if let Some(wm) = &mut self.window_manager {
            wm.focus_next();
        }
        self.restore_cursor_from_focused_window();
    }

    /// Moves focus to the previous window
    pub fn focus_prev_window(&mut self) {
        self.save_cursor_to_focused_window();
        if let Some(wm) = &mut self.window_manager {
            wm.focus_prev();
        }
        self.restore_cursor_from_focused_window();
    }

    /// Focus a specific window selected by a native frontend hit-test.
    pub fn focus_window(&mut self, index: usize) -> bool {
        self.save_cursor_to_focused_window();
        let focused = self
            .window_manager
            .as_mut()
            .is_some_and(|manager| manager.focus_window(index));
        if focused {
            self.restore_cursor_from_focused_window();
        }
        focused
    }

    /// Moves focus to the window to the left
    pub fn focus_window_left(&mut self) {
        self.save_cursor_to_focused_window();
        if let Some(wm) = &mut self.window_manager {
            wm.focus_left();
        }
        self.restore_cursor_from_focused_window();
    }

    /// Moves focus to the window to the right
    pub fn focus_window_right(&mut self) {
        self.save_cursor_to_focused_window();
        if let Some(wm) = &mut self.window_manager {
            wm.focus_right();
        }
        self.restore_cursor_from_focused_window();
    }

    /// Moves focus to the window above
    pub fn focus_window_up(&mut self) {
        self.save_cursor_to_focused_window();
        if let Some(wm) = &mut self.window_manager {
            wm.focus_up();
        }
        self.restore_cursor_from_focused_window();
    }

    /// Moves focus to the window below
    pub fn focus_window_down(&mut self) {
        self.save_cursor_to_focused_window();
        if let Some(wm) = &mut self.window_manager {
            wm.focus_down();
        }
        self.restore_cursor_from_focused_window();
    }

    /// Gets the current number of windows
    pub fn window_count(&self) -> usize {
        self.window_manager
            .as_ref()
            .map(|wm| wm.window_count())
            .unwrap_or(1)
    }

    /// Closes the current window
    /// Returns false if it's the last window (can't close)
    pub fn close_current_window(&mut self) -> bool {
        if let Some(wm) = &mut self.window_manager {
            wm.close_focused().is_ok()
        } else {
            false
        }
    }

    /// Closes the current window, or quits if it's the last window
    pub fn close_or_quit_window(&mut self) {
        if let Some(wm) = &mut self.window_manager {
            if wm.close_focused().is_err() {
                // Last window - quit instead
                self.quit();
            }
        } else {
            // No window manager - just quit
            self.quit();
        }
    }

    /// Closes all windows except the current one (like Vim's :only)
    /// Returns true if windows were closed, false if already only one window or no window manager
    pub fn close_other_windows(&mut self) -> bool {
        if let Some(wm) = &mut self.window_manager {
            wm.close_other_windows().is_ok()
        } else {
            false
        }
    }

    // === Viewport Scrolling ===

    /// Scrolls the focused viewport by `delta_down` *visual* (wrapped) rows —
    /// positive scrolls content up (Ctrl-E), negative scrolls content down
    /// (Ctrl-Y) — updating the visual sub-row offset so the viewport can stop
    /// partway into a wrapped line. Keeps the cursor within the viewport (minus
    /// `scrolloff`) measured in visual rows.
    ///
    /// Returns `false` without changing anything when soft wrap is off or the
    /// wrap map is stale, so the caller falls back to logical-line scrolling.
    fn scroll_visual_rows(&mut self, delta_down: isize) -> bool {
        if !self.options.wrap {
            return false;
        }
        // A macro can edit and scroll before either frontend renders again.
        // Refresh the geometry before using its row counts for explicit scroll.
        if let Some(width) = self.wrap_map().map(|map| map.wrap_width()) {
            self.ensure_wrap_map(width);
        }
        let len_lines = self.buffer().rope().len_lines();
        if self.wrap_map().is_none_or(|m| m.line_count() < len_lines) {
            return false;
        }

        let visible = self.focused_visible_rows().max(1);
        let scrolloff = self.options.scrolloff.min(visible.saturating_sub(1) / 2);
        let cur_off = self.scroll_offset();
        let cur_sub = self.scroll_subrow();
        // Source position is resolved through cached geometry below.
        let (cur_line, cur_col) = {
            let c = self.buffer().cursor();
            (c.line(), c.col())
        };

        let (new_off, new_sub, new_cursor_line) = {
            let map = match self.wrap_map() {
                Some(m) => m,
                None => return false,
            };
            let total_visual = map.total_visual_lines();
            let max_visual_start = total_visual.saturating_sub(visible);
            let cur_top = map.logical_to_visual(cur_off) + cur_sub;
            let new_top = if delta_down >= 0 {
                (cur_top + delta_down as usize).min(max_visual_start)
            } else {
                cur_top.saturating_sub(delta_down.unsigned_abs())
            };
            let (no, ns) = map.visual_to_logical(new_top);

            // Keep the cursor inside the viewport (± scrolloff) in visual rows.
            let (cursor_visual, _) = self
                .cursor_visual_position(cur_line, cur_col)
                .unwrap_or((0, 0));
            let last_visual = total_visual.saturating_sub(1);
            let new_cursor_line = if delta_down >= 0 {
                let min_visual = (new_top + scrolloff).min(last_visual);
                if cursor_visual < min_visual {
                    map.visual_to_logical(min_visual).0
                } else {
                    cur_line
                }
            } else {
                let bottom_visual = new_top + visible.saturating_sub(1);
                let max_visual = bottom_visual.saturating_sub(scrolloff);
                if cursor_visual > max_visual {
                    map.visual_to_logical(max_visual).0
                } else {
                    cur_line
                }
            };
            (no, ns, new_cursor_line)
        };

        let new_cursor_line = new_cursor_line.min(len_lines.saturating_sub(1));
        self.viewport.scroll_offset = new_off;
        self.viewport.scroll_subrow = new_sub;
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_position(new_off, new_sub);
                if new_cursor_line != cur_line {
                    window.cursor_mut().set_position(new_cursor_line, cur_col);
                }
            }
        }
        if new_cursor_line != cur_line {
            self.buffer_mut()
                .cursor_mut()
                .set_position(new_cursor_line, cur_col);
        }

        // We set the scroll explicitly — including a sub-row offset that
        // logical cursor-keeping can't express — so suppress the post-command
        // `update_scroll_offset`, which would otherwise snap the sub-row away.
        self.viewport.preserve_after_input();
        true
    }

    /// Scrolls viewport down N lines (Ctrl-e).
    ///
    /// The focused window's source of truth for the cursor is the *buffer*
    /// cursor (the renderer reads `buffer().cursor()` for the focused pane), so
    /// we sync it into the window before scrolling and read it back after.
    /// `Window::scroll_down` keeps the cursor `scrolloff` lines from the top
    /// edge, so the subsequent `update_scroll_offset()` is a vertical no-op
    /// rather than fighting the scroll.
    pub fn scroll_viewport_down(&mut self, lines: usize) {
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        // Soft wrap: scroll by visual rows (can stop partway into a wrapped line).
        if self.scroll_visual_rows(lines as isize) {
            return;
        }

        let buffer_line_count = self.buffer().line_count();
        let scrolloff = self.options.scrolloff;
        let (line, col) = {
            let cursor = self.buffer().cursor();
            (cursor.line(), cursor.col())
        };

        let new_state = if let Some(wm) = &mut self.window_manager {
            wm.focused_window_mut().map(|window| {
                window.cursor_mut().set_position(line, col);
                window.scroll_down(lines, buffer_line_count, scrolloff);
                (
                    window.scroll_offset(),
                    window.cursor().line(),
                    window.cursor().col(),
                )
            })
        } else {
            None
        };

        if let Some((scroll_offset, line, col)) = new_state {
            self.viewport.scroll_offset = scroll_offset;
            self.buffer_mut().cursor_mut().set_position(line, col);
        }
    }

    /// Scrolls viewport up N lines (Ctrl-y).
    ///
    /// See [`scroll_viewport_down`](Self::scroll_viewport_down) for the
    /// cursor-sync contract.
    pub fn scroll_viewport_up(&mut self, lines: usize) {
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        // Soft wrap: scroll by visual rows (can stop partway into a wrapped line).
        if self.scroll_visual_rows(-(lines as isize)) {
            return;
        }

        let scrolloff = self.options.scrolloff;
        let (line, col) = {
            let cursor = self.buffer().cursor();
            (cursor.line(), cursor.col())
        };

        let new_state = if let Some(wm) = &mut self.window_manager {
            wm.focused_window_mut().map(|window| {
                window.cursor_mut().set_position(line, col);
                window.scroll_up(lines, scrolloff);
                (
                    window.scroll_offset(),
                    window.cursor().line(),
                    window.cursor().col(),
                )
            })
        } else {
            None
        };

        if let Some((scroll_offset, line, col)) = new_state {
            self.viewport.scroll_offset = scroll_offset;
            self.buffer_mut().cursor_mut().set_position(line, col);
        }
    }

    /// Scrolls window `index` by `delta_down` rows exactly as Ctrl-E (positive)
    /// or Ctrl-Y (negative) would, without dispatching keys and without leaving
    /// the window focused: the cursor of the scrolled window stays inside its
    /// viewport by the usual rules, and focus (plus the focused cursor) is put
    /// back afterwards. Returns `false` for an unknown window.
    pub fn scroll_window_viewport(&mut self, index: usize, delta_down: isize) -> bool {
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }
        let Some((original, count)) = self
            .window_manager
            .as_ref()
            .map(|wm| (wm.focused_window_index(), wm.window_count()))
        else {
            return false;
        };
        if index >= count {
            return false;
        }
        let original_cursor = *self.buffer().cursor();
        if index != original {
            self.focus_window(index);
        }
        if delta_down >= 0 {
            self.scroll_viewport_down(delta_down as usize);
        } else {
            self.scroll_viewport_up(delta_down.unsigned_abs());
        }
        // The explicit scroll must not make the *next* key skip its scroll update.
        self.viewport.take_preserve_after_input();
        if index != original {
            self.focus_window(original);
            *self.buffer_mut().cursor_mut() = original_cursor;
            let scroll = self
                .window_manager
                .as_ref()
                .and_then(|wm| wm.focused_window())
                .map(|window| (window.scroll_offset(), window.scroll_subrow()));
            if let Some((offset, subrow)) = scroll {
                self.viewport.scroll_offset = offset;
                self.viewport.scroll_subrow = subrow;
            }
        }
        true
    }

    /// Centers cursor in viewport (zz).
    pub fn center_cursor_in_viewport(&mut self) {
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }
        let visible = self.focused_visible_rows();
        self.scroll_cursor_to_rows_below_top(visible / 2);
    }

    /// Moves cursor line to top of viewport, respecting scrolloff (zt).
    pub fn move_cursor_line_to_top(&mut self) {
        self.move_cursor_line_to_top_with_offset(self.options.scrolloff);
    }

    /// Moves cursor line to viewport with an explicit top offset.
    /// `top_offset` is the number of rows from the viewport top where the cursor should land.
    pub fn move_cursor_line_to_top_with_offset(&mut self, top_offset: usize) {
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }
        let visible = self.focused_visible_rows();
        let rows_above = top_offset.min(visible.saturating_sub(1) / 2);
        self.scroll_cursor_to_rows_below_top(rows_above);
    }

    /// Keeps an explicitly positioned viewport stable through the shared
    /// post-input cursor-visibility pass.
    ///
    /// Modal overlays use this when they render a source buffer for reading:
    /// even an ignored key must not apply normal-mode `scrolloff` afterward.
    pub(crate) fn preserve_viewport_after_input(&mut self) {
        self.viewport.preserve_after_input();
    }

    /// Moves cursor line to bottom of viewport, respecting scrolloff (zb).
    pub fn move_cursor_line_to_bottom(&mut self) {
        self.move_cursor_line_to_bottom_with_offset(self.options.scrolloff);
    }

    /// Moves cursor line to bottom of viewport with an explicit bottom offset.
    /// `bottom_offset` is the number of rows from the viewport bottom where the cursor should land.
    pub fn move_cursor_line_to_bottom_with_offset(&mut self, bottom_offset: usize) {
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }
        let visible = self.focused_visible_rows();
        let bottom_offset = bottom_offset.min(visible.saturating_sub(1) / 2);
        let rows_above = visible.saturating_sub(1).saturating_sub(bottom_offset);
        self.scroll_cursor_to_rows_below_top(rows_above);
    }

    /// The focused window's height in rows (≥ 1), falling back to the viewport
    /// height when there's no window manager.
    fn focused_visible_rows(&self) -> usize {
        self.window_manager
            .as_ref()
            .and_then(|wm| wm.focused_window())
            .map(|w| (w.height() as usize).max(1))
            .unwrap_or_else(|| self.viewport.viewport_height.max(1))
    }

    /// Scrolls the focused window so the cursor's line sits `rows_above` visual
    /// rows below the viewport top, then preserves that scroll after input.
    ///
    /// This is the shared engine for `zt`/`zz`/`zb`. It is wrap-aware: when soft
    /// wrap is on and a usable wrap map exists, `rows_above` is measured in
    /// *visual* (wrapped) rows so the cursor lands where the user sees it, not
    /// where logical line counting would put it.
    fn scroll_cursor_to_rows_below_top(&mut self, rows_above: usize) {
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        let (line, col) = {
            let cursor = self.buffer().cursor();
            (cursor.line(), cursor.col())
        };
        let max_line = self.buffer().line_count().saturating_sub(1);
        let visible = self.focused_visible_rows();

        let new_offset = self.top_line_for_cursor(line, col, rows_above, visible, max_line);

        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.cursor_mut().set_position(line, col);
                window.set_scroll_offset(new_offset);
            }
        }

        // Skip automatic scroll update - we explicitly set the scroll position.
        self.viewport.preserve_after_input();
    }

    /// Computes the top logical line so the cursor's line sits `rows_above` rows
    /// below it. Counts visual (wrapped) rows when wrap is on with a usable map;
    /// otherwise counts logical lines.
    fn top_line_for_cursor(
        &self,
        cursor_line: usize,
        cursor_col: crate::unicode::GraphemeCol,
        rows_above: usize,
        visible: usize,
        max_line: usize,
    ) -> usize {
        if self.options.wrap {
            if let Some(map) = self.wrap_map() {
                // Only trust the map when it covers the whole buffer (stale maps
                // can lag a structural edit until the next render rebuild).
                if map.line_count() >= self.buffer().rope().len_lines() {
                    let (cursor_visual, _) = self
                        .cursor_visual_position(cursor_line, cursor_col)
                        .expect("wrap map is available");
                    let target = cursor_visual.saturating_sub(rows_above);
                    let (target_line, subrow) = map.visual_to_logical(target);
                    // This API positions whole logical lines. If the requested
                    // start falls inside an earlier line, advance past it.
                    let offset = if target_line == cursor_line || subrow == 0 {
                        target_line
                    } else {
                        target_line.saturating_add(1)
                    };
                    let max_scroll = Self::compute_wrap_max_scroll_offset(map, visible, max_line);
                    return offset.min(max_scroll);
                }
            }
        }

        // Logical fallback: count whole lines.
        let offset = cursor_line.saturating_sub(rows_above);
        offset.min(max_line.saturating_sub(visible.saturating_sub(1)))
    }

    /// Scrolls forward (down) one page (both viewport and cursor)
    pub fn scroll_page_down(&mut self) {
        // Extract window info first to avoid borrowing conflicts
        // Use defaults when no window manager (for tests/headless mode)
        let (viewport_start, viewport_height) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.scroll_offset(), window.height() as usize)
            } else {
                (0, 24) // Default viewport
            }
        } else {
            // Default viewport for headless/test mode - use small size so scrolling works
            // with test content that may only have ~10 lines
            (0, 10)
        };

        // Now we can mutably borrow buffer
        let new_viewport =
            Motions::scroll_page_down(self.buffer_mut(), viewport_start, viewport_height);

        // Finally update window scroll offset
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_viewport);
            }
        }
    }

    /// Scrolls backward (up) one page (both viewport and cursor)
    pub fn scroll_page_up(&mut self) {
        // Extract window info first to avoid borrowing conflicts
        // Use defaults when no window manager (for tests/headless mode)
        let (viewport_start, viewport_height) = if let Some(wm) = &self.window_manager {
            if let Some(window) = wm.focused_window() {
                (window.scroll_offset(), window.height() as usize)
            } else {
                (0, 24) // Default viewport
            }
        } else {
            // Default viewport for headless/test mode
            // Estimate viewport_start based on cursor position
            let cursor_line = self.buffer().cursor().line();
            let viewport_height = 10;
            let viewport_start = cursor_line.saturating_sub(viewport_height / 2);
            (viewport_start, viewport_height)
        };

        // Now we can mutably borrow buffer
        let new_viewport =
            Motions::scroll_page_up(self.buffer_mut(), viewport_start, viewport_height);

        // Finally update window scroll offset
        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_scroll_offset(new_viewport);
            }
        }
    }

    /// Scrolls horizontally to put cursor at left edge (zs command)
    pub fn scroll_cursor_to_left_edge(&mut self) {
        // Initialize window manager if needed (fallback dimensions)
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        // Convert char column to display column for proper horizontal scrolling
        let cursor_col = self.buffer().cursor().col();
        let cursor_line = self.buffer().cursor().line();
        let tab_width = self.indent_options().tab_width;
        let cursor_display_col = {
            let line_text = self.buffer().line_text(cursor_line).unwrap_or_default();
            let char_col = crate::unicode::grapheme_to_char_col(&line_text, cursor_col);
            crate::display::char_col_to_display_col(&line_text, char_col.0, tab_width)
        };

        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                window.set_horizontal_offset(cursor_display_col);
            }
        }

        self.mark_dirty();
    }

    /// Scrolls horizontally to put cursor at right edge (ze command)
    pub fn scroll_cursor_to_right_edge(&mut self) {
        // Initialize window manager if needed (fallback dimensions)
        if self.window_manager.is_none() {
            self.init_window_manager(80, 24);
        }

        // Convert char column to display column for proper horizontal scrolling
        let cursor_col = self.buffer().cursor().col();
        let cursor_line = self.buffer().cursor().line();
        let tab_width = self.indent_options().tab_width;
        let cursor_display_col = {
            let line_text = self.buffer().line_text(cursor_line).unwrap_or_default();
            let char_col = crate::unicode::grapheme_to_char_col(&line_text, cursor_col);
            crate::display::char_col_to_display_col(&line_text, char_col.0, tab_width)
        };

        if let Some(wm) = &mut self.window_manager {
            if let Some(window) = wm.focused_window_mut() {
                let width = window.width() as usize;
                let new_offset = cursor_display_col.saturating_sub(width.saturating_sub(1));
                window.set_horizontal_offset(new_offset);
            }
        }

        self.mark_dirty();
    }
}
