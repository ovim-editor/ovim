/// Maps logical lines to visual (wrapped) lines for soft wrap rendering.
///
/// Each logical line may span multiple visual rows when its content
/// exceeds the available width. This structure precomputes the mapping
/// so rendering and scrolling can work in visual-line space.
///
/// Uses [`crate::wrap::visual_line_count`] as the single source of truth
/// for wrap computation, ensuring agreement with the renderer's
/// `split_line_into_rows`.
#[derive(Debug, Clone)]
pub struct WrapMap {
    /// Number of visual lines each logical line occupies (minimum 1)
    visual_counts: Vec<u16>,
    /// Cumulative visual line offset for each logical line (prefix sum)
    /// visual_offsets[i] = total visual lines before line i
    visual_offsets: Vec<usize>,
    /// Total visual lines across all logical lines
    total_visual_lines: usize,
    /// The wrap width used to compute this map
    wrap_width: usize,
    /// Tab width for column calculations
    tab_width: usize,
    /// Buffer version when this map was built (for invalidation)
    buffer_version: usize,
}

impl WrapMap {
    /// Creates a new WrapMap by computing visual line counts for all lines.
    ///
    /// `line_text` returns the text of a given line index (without trailing newline).
    pub fn new<F>(
        line_count: usize,
        wrap_width: usize,
        tab_width: usize,
        buffer_version: usize,
        line_text: F,
    ) -> Self
    where
        F: Fn(usize) -> String,
    {
        let width = wrap_width.max(1);
        let mut visual_counts = Vec::with_capacity(line_count);
        let mut visual_offsets = Vec::with_capacity(line_count);
        let mut total = 0;

        for i in 0..line_count {
            visual_offsets.push(total);
            let text = line_text(i);
            let count = crate::wrap::visual_line_count(&text, width, tab_width) as u16;
            visual_counts.push(count);
            total += count as usize;
        }

        Self {
            visual_counts,
            visual_offsets,
            total_visual_lines: total,
            wrap_width: width,
            tab_width,
            buffer_version,
        }
    }

    /// Returns the buffer version this map was built for
    pub fn buffer_version(&self) -> usize {
        self.buffer_version
    }

    /// Updates the stored buffer version without rebuilding.
    pub fn set_buffer_version(&mut self, version: usize) {
        self.buffer_version = version;
    }

    /// Returns the number of visual lines for a given logical line.
    pub fn visual_lines_for(&self, line: usize) -> u16 {
        self.visual_counts.get(line).copied().unwrap_or(1)
    }

    /// Returns the first visual row index for a given logical line.
    /// For out-of-bounds lines, returns total_visual_lines (one past last row).
    pub fn logical_to_visual(&self, line: usize) -> usize {
        self.visual_offsets
            .get(line)
            .copied()
            .unwrap_or(self.total_visual_lines)
    }

    /// Converts a visual row index to (logical_line, sub_line) within that line.
    pub fn visual_to_logical(&self, visual_row: usize) -> (usize, usize) {
        // Binary search for the logical line containing this visual row
        let line = match self.visual_offsets.binary_search(&visual_row) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let sub_line =
            visual_row.saturating_sub(self.visual_offsets.get(line).copied().unwrap_or(0));
        (line, sub_line)
    }

    /// Total number of visual lines across all logical lines.
    pub fn total_visual_lines(&self) -> usize {
        self.total_visual_lines
    }

    /// The wrap width this map was computed for.
    pub fn wrap_width(&self) -> usize {
        self.wrap_width
    }

    /// Number of logical lines in this map.
    pub fn line_count(&self) -> usize {
        self.visual_counts.len()
    }

    /// Recompute a single line after its content changed.
    ///
    /// `line_text` returns the text of a given line index (without trailing newline).
    pub fn invalidate_line<F>(&mut self, line: usize, line_text: F)
    where
        F: Fn(usize) -> String,
    {
        if line >= self.visual_counts.len() {
            return;
        }
        let old_count = self.visual_counts[line] as usize;
        let text = line_text(line);
        let new_count =
            crate::wrap::visual_line_count(&text, self.wrap_width, self.tab_width) as usize;
        if old_count == new_count {
            return;
        }
        let diff = new_count as isize - old_count as isize;
        self.visual_counts[line] = new_count as u16;
        self.total_visual_lines = (self.total_visual_lines as isize + diff) as usize;
        // Rebuild offsets from changed line onwards
        for i in (line + 1)..self.visual_offsets.len() {
            self.visual_offsets[i] = (self.visual_offsets[i] as isize + diff) as usize;
        }
    }

    /// Rebuild the entire map (e.g., after resize or wrap toggle).
    ///
    /// `line_text` returns the text of a given line index (without trailing newline).
    pub fn rebuild<F>(
        &mut self,
        line_count: usize,
        wrap_width: usize,
        tab_width: usize,
        buffer_version: usize,
        line_text: F,
    ) where
        F: Fn(usize) -> String,
    {
        let width = wrap_width.max(1);
        self.wrap_width = width;
        self.tab_width = tab_width;
        self.buffer_version = buffer_version;
        self.visual_counts.clear();
        self.visual_counts.reserve(line_count);
        self.visual_offsets.clear();
        self.visual_offsets.reserve(line_count);
        let mut total = 0;

        for i in 0..line_count {
            self.visual_offsets.push(total);
            let text = line_text(i);
            let count = crate::wrap::visual_line_count(&text, width, tab_width) as u16;
            self.visual_counts.push(count);
            total += count as usize;
        }
        self.total_visual_lines = total;
    }

    /// Maps a cursor position (line, display_col) to a visual position (visual_row, visual_col).
    ///
    /// Requires the line text to properly compute wrap points for wide chars.
    pub fn cursor_to_visual(&self, line: usize, col: usize, line_text: &str) -> (usize, usize) {
        let base_row = self.logical_to_visual(line);
        let wrap_points =
            crate::wrap::compute_wrap_points(line_text, self.wrap_width, self.tab_width);

        if wrap_points.is_empty() {
            return (base_row, col);
        }

        // Walk characters, tracking display columns per wrap segment.
        // `col` is a display column; wrap_points are char indices.
        // Invariant: at each wrap point, col >= segment_start_display
        // (otherwise we would have broken at a previous wrap point).
        let mut segment_start_display = 0;
        let mut current_display = 0;
        let mut sub_line = 0;
        let mut wp_idx = 0;

        for (char_idx, ch) in line_text.chars().enumerate() {
            if wp_idx < wrap_points.len() && char_idx == wrap_points[wp_idx] {
                if col < current_display {
                    // col is in the segment that just ended
                    break;
                }
                segment_start_display = current_display;
                sub_line += 1;
                wp_idx += 1;
            }

            let ch_width = if ch == '\t' {
                self.tab_width - (current_display % self.tab_width)
            } else {
                crate::display::char_display_width(ch)
            };
            current_display += ch_width;
        }

        let visual_col = col - segment_start_display;
        (base_row + sub_line, visual_col)
    }

    /// Simpler cursor_to_visual that works like the old API when line text isn't available.
    /// Uses simple division — less accurate for lines with wide chars at wrap boundaries.
    pub fn cursor_to_visual_simple(&self, line: usize, col: usize) -> (usize, usize) {
        let base_row = self.logical_to_visual(line);
        let sub_line = col / self.wrap_width;
        let visual_col = col % self.wrap_width;
        (base_row + sub_line, visual_col)
    }

    /// Returns the absolute display-column range for a wrapped sub-line.
    ///
    /// The start and end are in global display columns on the source line, where
    /// `sub_line` is the index of the wrapped visual segment.
    /// Returns `None` if `sub_line` is out of range.
    pub fn sub_line_display_range(
        &self,
        line_text: &str,
        sub_line: usize,
    ) -> Option<(usize, usize)> {
        let wrap_points =
            crate::wrap::compute_wrap_points(line_text, self.wrap_width, self.tab_width);
        if wrap_points.is_empty() {
            if sub_line == 0 {
                let end = crate::display::display_width(line_text, self.tab_width.max(1));
                return Some((0, end));
            }
            return None;
        }

        let mut starts = Vec::with_capacity(wrap_points.len() + 1);
        starts.push(0);
        let mut wp_idx = 0;
        let mut current_display = 0;
        let tab_width = self.tab_width.max(1);

        for (char_idx, ch) in line_text.chars().enumerate() {
            if wp_idx < wrap_points.len() && char_idx == wrap_points[wp_idx] {
                starts.push(current_display);
                wp_idx += 1;
            }
            let ch_width = if ch == '\t' {
                tab_width - (current_display % tab_width)
            } else {
                crate::display::char_display_width(ch)
            };
            current_display += ch_width;
        }

        if sub_line >= starts.len() {
            return None;
        }

        let start = starts[sub_line];
        let end = if sub_line + 1 < starts.len() {
            starts[sub_line + 1]
        } else {
            current_display
        };

        Some((start, end))
    }

    /// Counts total visual lines from `start_line` to `end_line` (exclusive).
    pub fn visual_lines_in_range(&self, start_line: usize, end_line: usize) -> usize {
        let start = self.visual_offsets.get(start_line).copied().unwrap_or(0);
        let end = if end_line >= self.visual_counts.len() {
            self.total_visual_lines
        } else {
            self.visual_offsets
                .get(end_line)
                .copied()
                .unwrap_or(self.total_visual_lines)
        };
        end.saturating_sub(start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_text<'a>(lines: &'a [&'a str]) -> impl Fn(usize) -> String + 'a {
        move |i| {
            if i < lines.len() {
                lines[i].to_string()
            } else {
                String::new()
            }
        }
    }

    #[test]
    fn test_single_line_fits() {
        let map = WrapMap::new(1, 80, 4, 0, make_text(&["a".repeat(40).as_str()]));
        assert_eq!(map.visual_lines_for(0), 1);
        assert_eq!(map.total_visual_lines(), 1);
    }

    #[test]
    fn test_line_exactly_fits() {
        let text = "a".repeat(80);
        let map = WrapMap::new(1, 80, 4, 0, make_text(&[text.as_str()]));
        assert_eq!(map.visual_lines_for(0), 1);
    }

    #[test]
    fn test_line_wraps_once() {
        let text = "a".repeat(81);
        let map = WrapMap::new(1, 80, 4, 0, make_text(&[text.as_str()]));
        assert_eq!(map.visual_lines_for(0), 2);
        assert_eq!(map.total_visual_lines(), 2);
    }

    #[test]
    fn test_empty_line() {
        let map = WrapMap::new(1, 80, 4, 0, make_text(&[""]));
        assert_eq!(map.visual_lines_for(0), 1);
    }

    #[test]
    fn test_multiple_lines() {
        let l0 = "a".repeat(40);
        let l1 = "a".repeat(160);
        let map = WrapMap::new(3, 80, 4, 0, make_text(&[l0.as_str(), l1.as_str(), ""]));
        assert_eq!(map.visual_lines_for(0), 1);
        assert_eq!(map.visual_lines_for(1), 2);
        assert_eq!(map.visual_lines_for(2), 1);
        assert_eq!(map.total_visual_lines(), 4);
    }

    #[test]
    fn test_logical_to_visual() {
        let l0 = "a".repeat(40);
        let l1 = "a".repeat(160);
        let map = WrapMap::new(3, 80, 4, 0, make_text(&[l0.as_str(), l1.as_str(), ""]));
        assert_eq!(map.logical_to_visual(0), 0);
        assert_eq!(map.logical_to_visual(1), 1);
        assert_eq!(map.logical_to_visual(2), 3);
    }

    #[test]
    fn test_visual_to_logical() {
        let l0 = "a".repeat(40);
        let l1 = "a".repeat(160);
        let map = WrapMap::new(3, 80, 4, 0, make_text(&[l0.as_str(), l1.as_str(), ""]));
        assert_eq!(map.visual_to_logical(0), (0, 0));
        assert_eq!(map.visual_to_logical(1), (1, 0));
        assert_eq!(map.visual_to_logical(2), (1, 1));
        assert_eq!(map.visual_to_logical(3), (2, 0));
    }

    #[test]
    fn test_cursor_to_visual_simple() {
        let l0 = "a".repeat(200);
        let l1 = "a".repeat(40);
        let map = WrapMap::new(2, 80, 4, 0, make_text(&[l0.as_str(), l1.as_str()]));
        // Line 0: 200 ASCII chars -> 3 visual lines
        // Cursor at col 85 -> sub_line 1, visual_col 5
        let (row, col) = map.cursor_to_visual_simple(0, 85);
        assert_eq!(row, 1);
        assert_eq!(col, 5);
    }

    #[test]
    fn test_roundtrip() {
        let lines: Vec<String> = vec![
            "a".repeat(80),
            "a".repeat(161),
            "a".repeat(50),
            "a".repeat(240),
            String::new(),
        ];
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let map = WrapMap::new(5, 80, 4, 0, make_text(&refs));
        for line in 0..5 {
            let visual = map.logical_to_visual(line);
            let (got_line, got_sub) = map.visual_to_logical(visual);
            assert_eq!(got_line, line);
            assert_eq!(got_sub, 0);
        }
    }

    #[test]
    fn test_invalidate_line() {
        let l0 = "a".repeat(40);
        let l1 = "a".repeat(160);
        let mut map = WrapMap::new(3, 80, 4, 0, make_text(&[l0.as_str(), l1.as_str(), ""]));
        assert_eq!(map.total_visual_lines(), 4);

        // Line 1 changed from 160 to 40 chars (2 -> 1 visual)
        let new_l1 = "a".repeat(40);
        map.invalidate_line(1, |_| new_l1.clone());
        assert_eq!(map.visual_lines_for(1), 1);
        assert_eq!(map.total_visual_lines(), 3);
        assert_eq!(map.logical_to_visual(2), 2);
    }

    #[test]
    fn test_visual_lines_in_range() {
        let l0 = "a".repeat(40);
        let l1 = "a".repeat(160);
        let l3 = "a".repeat(80);
        let map = WrapMap::new(
            4,
            80,
            4,
            0,
            make_text(&[l0.as_str(), l1.as_str(), "", l3.as_str()]),
        );
        // Lines: 1 + 2 + 1 + 1 = 5 total
        assert_eq!(map.visual_lines_in_range(0, 4), 5);
        assert_eq!(map.visual_lines_in_range(1, 3), 3); // 2 + 1
        assert_eq!(map.visual_lines_in_range(0, 1), 1);
    }

    #[test]
    fn test_wide_chars_increase_row_count() {
        // This is the key test: wide chars at wrap boundaries cause more rows
        // than a naïve div_ceil calculation would predict.
        // Width 3: "世世世" = 6 display cols, but each 世 (width 2) gets its own row
        // because 2+2 = 4 > 3
        let map = WrapMap::new(1, 3, 4, 0, make_text(&["世世世"]));
        assert_eq!(map.visual_lines_for(0), 3); // not 2!
        assert_eq!(map.total_visual_lines(), 3);
    }

    // ---- cursor_to_visual tests ----

    #[test]
    fn test_cursor_to_visual_no_wrap() {
        let map = WrapMap::new(1, 80, 4, 0, make_text(&["hello"]));
        // No wrapping, col maps directly
        assert_eq!(map.cursor_to_visual(0, 0, "hello"), (0, 0));
        assert_eq!(map.cursor_to_visual(0, 3, "hello"), (0, 3));
    }

    #[test]
    fn test_cursor_to_visual_ascii_wrap() {
        // 10 chars, wrap at 5 → wrap_point at char 5
        // Row 0: "abcde" (display cols 0..5), Row 1: "fghij" (display cols 5..10)
        let text = "abcdefghij";
        let map = WrapMap::new(1, 5, 4, 0, make_text(&[text]));
        assert_eq!(map.visual_lines_for(0), 2);

        // Col 0-4 → sub_line 0
        assert_eq!(map.cursor_to_visual(0, 0, text), (0, 0));
        assert_eq!(map.cursor_to_visual(0, 4, text), (0, 4));
        // Col 5+ → sub_line 1
        assert_eq!(map.cursor_to_visual(0, 5, text), (0 + 1, 0));
        assert_eq!(map.cursor_to_visual(0, 9, text), (0 + 1, 4));
    }

    #[test]
    fn test_cursor_to_visual_multiple_wraps() {
        // 15 chars, wrap at 5 → 3 visual rows
        let text = "aaaaabbbbbccccc";
        let map = WrapMap::new(1, 5, 4, 0, make_text(&[text]));
        assert_eq!(map.visual_lines_for(0), 3);

        assert_eq!(map.cursor_to_visual(0, 0, text), (0, 0));
        assert_eq!(map.cursor_to_visual(0, 4, text), (0, 4));
        assert_eq!(map.cursor_to_visual(0, 5, text), (1, 0));
        assert_eq!(map.cursor_to_visual(0, 9, text), (1, 4));
        assert_eq!(map.cursor_to_visual(0, 10, text), (2, 0));
        assert_eq!(map.cursor_to_visual(0, 14, text), (2, 4));
    }

    #[test]
    fn test_cursor_to_visual_wide_chars() {
        // "世世世" with wrap_width=3
        // 世 = width 2, so each gets its own row (2+2=4 > 3)
        // wrap_points at char 1 and char 2
        // Row 0: 世 (display cols 0..2), Row 1: 世 (display cols 2..4), Row 2: 世
        let text = "世世世";
        let map = WrapMap::new(1, 3, 4, 0, make_text(&[text]));
        assert_eq!(map.visual_lines_for(0), 3);

        assert_eq!(map.cursor_to_visual(0, 0, text), (0, 0)); // first 世
        assert_eq!(map.cursor_to_visual(0, 2, text), (1, 0)); // second 世
        assert_eq!(map.cursor_to_visual(0, 4, text), (2, 0)); // third 世
    }

    #[test]
    fn test_cursor_to_visual_mixed_ascii_wide() {
        // "ab世cd" wrap_width=4
        // a(1) b(1) → 2, 世(2) → 4, fits! c(1) → 5 > 4, wraps
        // Row 0: "ab世" (cols 0-3), Row 1: "cd" (cols 4-5)
        let text = "ab世cd";
        let map = WrapMap::new(1, 4, 4, 0, make_text(&[text]));
        assert_eq!(map.visual_lines_for(0), 2);

        assert_eq!(map.cursor_to_visual(0, 0, text), (0, 0)); // a
        assert_eq!(map.cursor_to_visual(0, 1, text), (0, 1)); // b
        assert_eq!(map.cursor_to_visual(0, 2, text), (0, 2)); // 世 (starts at display col 2)
        assert_eq!(map.cursor_to_visual(0, 4, text), (1, 0)); // c (wrapped)
        assert_eq!(map.cursor_to_visual(0, 5, text), (1, 1)); // d
    }

    #[test]
    fn test_cursor_to_visual_with_tabs() {
        // "a\tb" with tab_width=4, wrap_width=6
        // a = 1 col, \t = 3 cols (4 - 1%4 = 3), b = 1 col → total 5, fits
        let text = "a\tb";
        let map = WrapMap::new(1, 6, 4, 0, make_text(&[text]));
        assert_eq!(map.visual_lines_for(0), 1);
        assert_eq!(map.cursor_to_visual(0, 0, text), (0, 0));
        assert_eq!(map.cursor_to_visual(0, 4, text), (0, 4)); // b at display col 4
    }

    #[test]
    fn test_cursor_to_visual_col_at_wrap_boundary() {
        // 10 chars, wrap at 5
        // Col 5 is the first col of the second row
        let text = "abcdefghij";
        let map = WrapMap::new(1, 5, 4, 0, make_text(&[text]));
        // Col exactly at boundary goes to next row
        assert_eq!(map.cursor_to_visual(0, 5, text), (1, 0));
    }

    #[test]
    fn test_cursor_to_visual_second_line() {
        // Two lines: first wraps, second doesn't
        let l0 = "a".repeat(10);
        let l1 = "bbb";
        let map = WrapMap::new(2, 5, 4, 0, make_text(&[&l0, l1]));
        // Line 0: 2 visual rows (base_row 0)
        // Line 1: 1 visual row (base_row 2)
        assert_eq!(map.cursor_to_visual(1, 0, l1), (2, 0));
        assert_eq!(map.cursor_to_visual(1, 2, l1), (2, 2));
    }

    #[test]
    fn test_sub_line_display_range_ascii_wrap() {
        let map = WrapMap::new(1, 5, 4, 0, make_text(&["abcdefghij"]));
        assert_eq!(map.sub_line_display_range("abcdefghij", 0), Some((0, 5)));
        assert_eq!(map.sub_line_display_range("abcdefghij", 1), Some((5, 10)));
        assert_eq!(map.sub_line_display_range("abcdefghij", 2), None);
    }

    #[test]
    fn test_sub_line_display_range_wide_chars() {
        // "世世世" with width 3 -> each wide char (2 cols) is wrapped separately.
        let map = WrapMap::new(1, 3, 4, 0, make_text(&["世世世"]));
        assert_eq!(map.sub_line_display_range("世世世", 0), Some((0, 2)));
        assert_eq!(map.sub_line_display_range("世世世", 1), Some((2, 4)));
        assert_eq!(map.sub_line_display_range("世世世", 2), Some((4, 6)));
    }

    #[test]
    fn test_sub_line_display_range_tabs() {
        // Wrap width 4, tab width 4: "\ta" -> [tab(4), "a"] -> wrap between entries
        let map = WrapMap::new(1, 4, 4, 0, make_text(&["\ta"]));
        assert_eq!(map.sub_line_display_range("\ta", 0), Some((0, 4)));
        assert_eq!(map.sub_line_display_range("\ta", 1), Some((4, 5)));
        assert_eq!(map.sub_line_display_range("", 0), Some((0, 0)));
        assert_eq!(map.sub_line_display_range("", 1), None);
    }
}
