//! Code folding on the editor: `z` commands, automatic fold sources (LSP
//! `foldingRange`, indentation fallback), cursor rules around closed folds and
//! the `⋯ N lines` marker on fold headers.
//!
//! Folds are only computed once the user issues a fold command for a buffer
//! and are then kept fresh (debounced) as the text changes.

use super::decoration::{Decoration, DecorationPlacement, DecorationSource, DecorationStyle};
use super::Editor;
use crate::fold::{indent_fold_ranges, syntax_fold_ranges, FoldSource};
use crate::unicode::GraphemeCol;

/// How long the buffer must be quiet before folds are recomputed.
const FOLD_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

impl Editor {
    /// Requests folds for the current buffer: indentation folds immediately
    /// (so `zM` works at once), the language server's ranges shortly after.
    pub(crate) fn ensure_folds(&mut self) {
        if self.buffer().fold_manager().is_active() {
            return;
        }
        self.buffer_mut().fold_manager_mut().activate();
        self.compute_local_folds();
        self.lsp.intents.folding_ranges = true;
    }

    /// Replaces the automatic folds with the ones the editor can compute
    /// itself: the tree-sitter syntax tree when the buffer has one, otherwise
    /// indentation. Used until (and instead of) a language server's answer.
    pub(crate) fn compute_local_folds(&mut self) {
        let tab_width = self.indent_options().tab_width.max(1);
        let buffer = self.buffer();
        let line_count = buffer.line_count();
        let version = buffer.version();
        let syntax = buffer.syntax_tree().map(|tree| {
            syntax_fold_ranges(tree, &|row| {
                buffer.line_text(row).unwrap_or_default().to_string()
            })
        });
        let (ranges, source) = match syntax {
            Some(ranges) if !ranges.is_empty() => (ranges, FoldSource::Syntax),
            _ => {
                let lines: Vec<String> = (0..line_count)
                    .map(|line| buffer.line_text(line).unwrap_or_default().to_string())
                    .collect();
                (indent_fold_ranges(&lines, tab_width), FoldSource::Indent)
            }
        };
        self.buffer_mut()
            .fold_manager_mut()
            .set_auto_folds_from(&ranges, line_count, version, source);
    }

    /// Applies folds from the language server. Returns false for stale or
    /// empty answers (the indentation folds stay).
    pub fn apply_lsp_folding_ranges(
        &mut self,
        file_path: &str,
        buffer_version: usize,
        ranges: &[lsp_types::FoldingRange],
    ) -> bool {
        if self.buffer().file_path() != Some(file_path) || self.buffer().version() != buffer_version
        {
            return false;
        }
        if ranges.is_empty() {
            return false;
        }
        let pairs: Vec<(usize, usize)> = ranges
            .iter()
            .map(|range| (range.start_line as usize, range.end_line as usize))
            .collect();
        let line_count = self.buffer().line_count();
        self.buffer_mut().fold_manager_mut().set_auto_folds(
            &pairs,
            line_count,
            buffer_version,
            true,
        );
        self.refresh_fold_view();
        true
    }

    /// Recomputes the automatic folds after the text settled.
    pub(crate) async fn maintain_folds(&mut self) {
        if !self.buffer().fold_manager().is_active() {
            // The fold gutter needs the folds up front; without it they are
            // computed on the first fold command.
            if !self.fold_gutter_wants_folds() {
                return;
            }
            self.ensure_folds();
        }
        let buffer = self.buffer();
        let manager = buffer.fold_manager();
        let version = buffer.version();
        if manager.auto_version() == Some(version) {
            self.lsp.state.fold_tracking = None;
            return;
        }
        match self.lsp.state.fold_tracking {
            Some((tracked, since)) if tracked == version => {
                if since.elapsed() < FOLD_DEBOUNCE {
                    return;
                }
            }
            _ => {
                self.lsp.state.fold_tracking = Some((version, std::time::Instant::now()));
                return;
            }
        }
        self.lsp.state.fold_tracking = None;
        if !self.buffer().fold_manager().is_lsp_backed() {
            self.compute_local_folds();
            self.refresh_fold_view();
        }
        self.request_folding_ranges().await;
    }

    /// Whether the current buffer should get folds without a fold command
    /// because the fold gutter is enabled: real files of a sane size.
    fn fold_gutter_wants_folds(&self) -> bool {
        const MAX_LINES: usize = 50_000;
        let buffer = self.buffer();
        self.options.foldcolumn > 0
            && buffer
                .file_path()
                .is_some_and(|path| !super::buffer_manager::is_scratch_path(path))
            && buffer.line_count() <= MAX_LINES
    }

    pub(in crate::editor) async fn request_folding_ranges(&mut self) {
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };
        let Some(file_path) = self.buffer().file_path().map(|p| p.to_string()) else {
            return;
        };
        let Some(language_id) = self.language_id_for_path(&file_path) else {
            return;
        };
        let Some(uri) = crate::lsp::uri_from_file_path(&file_path) else {
            return;
        };
        let version = self.buffer().version();
        self.ensure_lsp_document_synced().await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = lsp.folding_range(&uri, &language_id).await;
            let _ = tx.send(
                result.map(|ranges| crate::editor::lsp_slot::FoldingRangesResult {
                    ranges,
                    file_path,
                    buffer_version: version,
                }),
            );
        });
        self.lsp.slots.folding_ranges.fire(task, rx);
    }

    pub(in crate::editor) fn poll_folding_slot(&mut self) -> bool {
        let Some(result) = self
            .lsp
            .slots
            .folding_ranges
            .poll_with_timeout(std::time::Duration::from_secs(30))
        else {
            return false;
        };
        match result {
            Ok(result) => {
                let applied = self.apply_lsp_folding_ranges(
                    &result.file_path,
                    result.buffer_version,
                    &result.ranges,
                );
                if applied {
                    self.mark_dirty();
                }
                applied
            }
            Err(_) => false,
        }
    }

    // ----- `z` commands ---------------------------------------------------------

    /// Handles `z{key}` fold commands. Returns false when `key` is not a fold
    /// command (so the caller can try the scroll commands).
    pub fn fold_command(&mut self, key: char) -> bool {
        if !matches!(
            key,
            'o' | 'c'
                | 'a'
                | 'O'
                | 'C'
                | 'A'
                | 'R'
                | 'M'
                | 'v'
                | 'n'
                | 'N'
                | 'i'
                | 'd'
                | 'D'
                | 'E'
                | 'j'
                | 'k'
                | 'r'
                | 'm'
                | 'x'
                | 'X'
        ) {
            return false;
        }
        self.ensure_folds();
        let line = self.buffer().cursor().line();
        let count = self.effective_count();
        let manager = self.buffer_mut().fold_manager_mut();
        match key {
            'o' => {
                manager.open_one(line);
            }
            'c' => {
                manager.close_one(line);
            }
            'a' => {
                manager.toggle_one(line);
            }
            'O' => {
                manager.open_recursive(line);
            }
            'C' => {
                manager.close_recursive(line);
            }
            'A' => {
                manager.toggle_recursive(line);
            }
            'R' => manager.open_all(),
            'M' => {
                manager.set_enabled(true);
                manager.close_all();
            }
            'v' => {
                manager.reveal(line);
            }
            'r' => manager.reduce_folding(count),
            'm' => {
                manager.set_enabled(true);
                manager.fold_more(count);
            }
            'x' => {
                manager.set_enabled(true);
                manager.reapply_foldlevel();
                manager.reveal(line);
            }
            'X' => {
                manager.set_enabled(true);
                manager.reapply_foldlevel();
            }
            'n' => manager.set_enabled(false),
            'N' => manager.set_enabled(true),
            'i' => {
                let enabled = manager.is_enabled();
                manager.set_enabled(!enabled);
            }
            'd' => manager.delete_fold_at(line),
            'D' => manager.delete_recursive(line),
            'E' => manager.delete_all(),
            'j' | 'k' => {
                let target = if key == 'j' {
                    manager.next_fold_start(line)
                } else {
                    manager.previous_fold_end(line)
                };
                if let Some(target) = target {
                    self.move_cursor_to_line_keeping_column(target);
                }
            }
            _ => {}
        }
        self.clear_count();
        self.after_fold_change();
        true
    }

    /// `[z` / `]z`: start / end of the open fold containing the cursor; when
    /// already there, of the fold around it. `count` repeats. Fails (stays)
    /// when there is no such fold.
    pub fn fold_edge_motion(&mut self, to_end: bool) {
        self.ensure_folds();
        let count = self.effective_count();
        let mut line = self.buffer().cursor().line();
        for _ in 0..count {
            match self.buffer().fold_manager().fold_edge_target(line, to_end) {
                Some(target) => line = target,
                None => break,
            }
        }
        if line != self.buffer().cursor().line() {
            // Vim lands in the first column.
            self.buffer_mut()
                .cursor_mut()
                .set_position(line, GraphemeCol(0));
            self.buffer_mut().validate_cursor_position();
        }
        self.clear_count();
        self.after_fold_change();
    }

    fn move_cursor_to_line_keeping_column(&mut self, line: usize) {
        let col = self.buffer().cursor().col();
        self.buffer_mut().cursor_mut().set_position(line, col);
        self.buffer_mut().validate_cursor_position();
    }

    /// Cursor / scroll / marker bookkeeping after any fold state change.
    fn after_fold_change(&mut self) {
        let line = self.buffer().cursor().line();
        self.settle_cursor_after_fold_change(line);
        self.refresh_fold_view();
        self.mark_dirty();
    }

    /// A cursor inside a just-closed fold moves to the fold's header.
    fn settle_cursor_after_fold_change(&mut self, line: usize) {
        if let Some((start, _)) = self.buffer().fold_manager().closed_fold_at(line) {
            if start != line {
                self.move_cursor_to_line_keeping_column(start);
            }
        }
    }

    // ----- per-key bookkeeping ---------------------------------------------------

    /// Runs after every key: keeps fold ranges aligned with line-count changes,
    /// keeps the cursor out of closed folds (Vim's rules) and refreshes the
    /// header markers.
    pub(crate) fn sync_folds_after_key(
        &mut self,
        prev_line: usize,
        prev_col: GraphemeCol,
        prev_version: usize,
    ) {
        if self.buffer().fold_manager().is_empty() {
            self.clear_fold_markers();
            return;
        }
        let line_count = self.buffer().line_count();
        self.buffer_mut()
            .fold_manager_mut()
            .adjust_for_line_count(line_count, prev_line);

        let cursor = self.buffer().cursor();
        let (line, col) = (cursor.line(), cursor.col());
        let insert_like = matches!(
            self.mode(),
            crate::mode::Mode::Insert | crate::mode::Mode::Replace
        );
        match self.buffer().fold_manager().closed_fold_at(line) {
            Some((start, end)) if line > start => {
                if insert_like {
                    // Typing into a hidden line opens the fold around it.
                    self.buffer_mut().fold_manager_mut().reveal(line);
                } else if prev_line == start && line == start + 1 {
                    // `j` from a closed header: on to the next visible line.
                    let target = end + 1;
                    if target < line_count {
                        self.move_cursor_to_line_keeping_column(target);
                    } else {
                        self.move_cursor_to_line_keeping_column(start);
                    }
                } else if line.abs_diff(prev_line) <= 1 {
                    self.move_cursor_to_line_keeping_column(start);
                } else {
                    // A jump (search, mark, `%`, `G`, ...) lands in the fold:
                    // open it so the target is visible.
                    self.buffer_mut().fold_manager_mut().reveal(line);
                }
            }
            Some((start, _)) if line == start && line == prev_line && col != prev_col => {
                // Horizontal movement on a closed header opens it (`l`, `$`,
                // ...), except while selecting: a Visual selection keeps the
                // fold closed and covers all of it (`v$d` in Vim).
                let selecting = matches!(
                    self.mode(),
                    crate::mode::Mode::Visual
                        | crate::mode::Mode::VisualLine
                        | crate::mode::Mode::VisualBlock
                );
                // A key that edited the text (`>>` moves the cursor to the
                // first non-blank) was no horizontal motion.
                let edited = self.buffer().version() != prev_version;
                if !insert_like && !selecting && !edited {
                    self.buffer_mut().fold_manager_mut().open_one(line);
                }
            }
            _ => {}
        }
        // The viewport must not start inside a hidden region.
        let top = self.scroll_offset();
        if let Some((start, _)) = self.buffer().fold_manager().closed_fold_at(top) {
            if start != top {
                self.set_scroll_offset_line(start);
            }
        }
        self.refresh_fold_view();
    }

    fn set_scroll_offset_line(&mut self, line: usize) {
        self.viewport.scroll_offset = line;
        self.viewport.scroll_subrow = 0;
        if let Some(window) = self
            .window_manager
            .as_mut()
            .and_then(|wm| wm.focused_window_mut())
        {
            window.set_scroll_position(line, 0);
        }
    }

    fn clear_fold_markers(&mut self) {
        self.lsp.state.fold_markers_key = None;
        if self.lsp.state.fold_markers.is_empty() {
            return;
        }
        let rope = self.buffer().rope().clone();
        self.decorations
            .replace_source(DecorationSource::Fold, Vec::new(), &rope);
        self.lsp.state.fold_markers.clear();
        self.mark_dirty();
    }

    /// Re-renders the `⋯ N lines` markers when the set of closed folds changed.
    pub(crate) fn refresh_fold_view(&mut self) {
        let buffer = self.buffer();
        let manager = buffer.fold_manager();
        let key = (buffer.id(), manager.generation());
        if self.lsp.state.fold_markers_key == Some(key) {
            return;
        }
        let markers: Vec<(usize, usize)> = manager.closed_fold_headers().collect();
        self.lsp.state.fold_markers_key = Some(key);
        if markers == self.lsp.state.fold_markers {
            return;
        }
        let rope = self.buffer().rope().clone();
        let version = self.buffer().version() as u64;
        let decorations: Vec<Decoration> = markers
            .iter()
            .filter(|(line, _)| *line < rope.len_lines())
            .map(|&(line, count)| {
                let text = format!("  ⋯ {count} lines");
                Decoration {
                    placement: DecorationPlacement::EndOfLine {
                        char_offset: rope.line_to_char(line),
                    },
                    source: DecorationSource::Fold,
                    display_width: crate::display::display_width(&text, 1),
                    text,
                    style: DecorationStyle::new(crate::color::Color::Gray).with_italic(),
                    priority: 5,
                    source_version: version,
                }
            })
            .collect();
        self.decorations
            .replace_source(DecorationSource::Fold, decorations, &rope);
        self.lsp.state.fold_markers = markers;
        self.mark_dirty();
    }

    /// The buffer line drawn on screen row `rel_row` (top of the viewport is
    /// 0) when lines are not soft-wrapped: closed folds take no rows.
    pub(crate) fn line_for_screen_row(&self, rel_row: usize) -> usize {
        let folds = self.buffer().fold_manager();
        folds.line_at_visible_index(folds.visible_index(self.scroll_offset()) + rel_row)
    }

    /// Line count a linewise command with `count` lines covers when closed
    /// folds count as one line each (`dd` on a closed fold deletes the whole
    /// fold, as in Vim).
    pub(crate) fn linewise_count_over_folds(&self, count: usize) -> usize {
        let manager = self.buffer().fold_manager();
        if manager.hidden_ranges().is_empty() {
            return count;
        }
        let start = self.buffer().cursor().line();
        let max_line = self.buffer().line_count().saturating_sub(1);
        let last_visible = manager.step_down(start, count.saturating_sub(1), max_line);
        let end = manager
            .closed_fold_at(last_visible)
            .map_or(last_visible, |(_, end)| end);
        end - start + 1
    }

    /// The closed fold whose header is the cursor line, as `(start, end)`.
    /// Vim treats characterwise commands there (`x`, `D`, `C`, `s`, `dl`,
    /// `cl`) as acting on the whole fold.
    pub(crate) fn closed_fold_at_cursor(&self) -> Option<(usize, usize)> {
        let line = self.buffer().cursor().line();
        self.buffer()
            .fold_manager()
            .closed_fold_at(line)
            .filter(|(start, _)| *start == line)
    }

    /// `count` for a `{op}j`-style command that must cover `count` visible
    /// lines below the cursor line plus everything a closed fold hides:
    /// returns the `count` that yields the same line span when applied as
    /// "cursor line and `count` lines below" (Vim's `dj` over closed folds).
    pub(crate) fn down_count_over_folds(&self, count: usize) -> usize {
        let manager = self.buffer().fold_manager();
        if manager.hidden_ranges().is_empty() {
            return count;
        }
        let start = self.buffer().cursor().line();
        let max_line = self.buffer().line_count().saturating_sub(1);
        let last_visible = manager.step_down(start, count, max_line);
        let end = manager
            .closed_fold_at(last_visible)
            .map_or(last_visible, |(_, end)| end);
        end - start
    }

    /// `o` / linewise `p` on a closed fold act below the fold's last line.
    pub(crate) fn cursor_to_closed_fold_end(&mut self) {
        if let Some((_, end)) = self.closed_fold_at_cursor() {
            self.buffer_mut()
                .cursor_mut()
                .set_position(end, GraphemeCol(0));
            self.buffer_mut().validate_cursor_position();
        }
    }

    /// Extends a selection over closed folds at either end (Vim does this
    /// when a Visual selection or a motion starts or ends inside one).
    /// `end_col_past_line` is the column just past the last character.
    pub(crate) fn extend_selection_over_folds(
        &self,
        start: (usize, usize),
        end: (usize, usize),
        end_col: impl Fn(usize) -> usize,
    ) -> ((usize, usize), (usize, usize)) {
        let manager = self.buffer().fold_manager();
        let start = match manager.closed_fold_at(start.0) {
            Some((fold_start, _)) => (fold_start, 0),
            None => start,
        };
        let end = match manager.closed_fold_at(end.0) {
            Some((_, fold_end)) => (fold_end, end_col(fold_end)),
            None => end,
        };
        (start, end)
    }

    /// Width of the fold gutter column for the current buffer (0 = hidden).
    pub fn fold_column_width(&self) -> usize {
        let options = &self.options;
        if options.foldcolumn == 0 {
            return 0;
        }
        if !options.foldcolumn_auto {
            return options.foldcolumn;
        }
        let manager = self.buffer().fold_manager();
        if manager.is_empty() || !manager.is_enabled() {
            return 0;
        }
        manager.deepest_nesting().min(options.foldcolumn)
    }

    /// The `width` fold gutter cells of `line`, left to right. The innermost
    /// `width` fold levels are shown (right-aligned), so `foldcolumn=1`
    /// marks the innermost fold: `-` on its header, `|` inside, `+` when it
    /// is closed.
    pub fn fold_gutter_cells(&self, line: usize, width: usize) -> Vec<crate::fold::FoldGutterMark> {
        use crate::fold::FoldGutterMark;
        let chain = self.buffer().fold_manager().gutter_chain(line);
        let shown = &chain[chain.len().saturating_sub(width)..];
        let mut cells = vec![FoldGutterMark::Blank; width - shown.len().min(width)];
        cells.extend_from_slice(shown);
        cells
    }

    /// Toggles the fold that starts at `line` (a click on its gutter mark).
    /// Returns false when no fold starts there.
    pub fn toggle_fold_at_gutter(&mut self, line: usize) -> bool {
        self.ensure_folds();
        let manager = self.buffer_mut().fold_manager_mut();
        // The closed fold that hides the line's body, else the outermost
        // fold that starts on it.
        let started = manager.fold_at(line).is_some();
        if !started {
            return false;
        }
        manager.toggle_fold_at(line);
        self.after_fold_change();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// About 47k lines and 9.5k functions: `zM` closes all of them.
    fn editor_with_many_closed_folds() -> Editor {
        let text: String = (0..9_500)
            .map(|n| format!("fn f{n}() {{\n    let x = {n};\n    x + 1\n}}\n\n"))
            .collect();
        let mut editor = Editor::with_content(&text);
        editor
            .buffer_mut()
            .set_file_path("/tmp/many_folds.rs".to_string());
        editor.buffer_mut().enable_syntax_highlighting();
        assert!(editor.fold_command('M'));
        assert!(editor.buffer().fold_manager().folds().len() > 9_000);
        editor
    }

    /// `sync_folds_after_key` runs after every key and used to rescan all
    /// folds twice per call (140-160 ms per `j` on a 48k-line Rust file).
    /// With nothing changed it must be a few lookups; the bound is orders of
    /// magnitude above that so debug builds and slow machines cannot trip it.
    #[test]
    fn per_key_fold_bookkeeping_does_not_scale_with_the_fold_count() {
        let mut editor = editor_with_many_closed_folds();
        let version = editor.buffer().version();
        let started = std::time::Instant::now();
        for _ in 0..100 {
            let cursor = editor.buffer().cursor();
            editor.sync_folds_after_key(cursor.line(), cursor.col(), version);
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "100 keys over 9.5k closed folds took {elapsed:?}"
        );
    }

    /// The markers are rebuilt only when a fold changed, and then match the
    /// closed folds.
    #[test]
    fn fold_markers_follow_the_closed_folds_and_skip_unchanged_state() {
        let mut editor = Editor::with_content("a\n  b\n  c\nd\n  e\n  f\n");
        assert!(editor.fold_command('M'));
        assert_eq!(editor.lsp.state.fold_markers, vec![(0, 2), (3, 2)]);
        let key = editor.lsp.state.fold_markers_key;
        assert!(key.is_some());
        // Nothing changed: the pass is a no-op and keeps its key.
        editor.refresh_fold_view();
        assert_eq!(editor.lsp.state.fold_markers_key, key);
        assert!(editor.fold_command('o'));
        assert_eq!(editor.lsp.state.fold_markers, vec![(3, 2)]);
        assert_ne!(editor.lsp.state.fold_markers_key, key);
    }
}
