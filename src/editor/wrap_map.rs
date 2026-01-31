/// Maps logical lines to visual (wrapped) lines for soft wrap rendering.
///
/// Each logical line may span multiple visual rows when its content
/// exceeds the available width. This structure precomputes the mapping
/// so rendering and scrolling can work in visual-line space.
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
    pub fn new<F>(line_count: usize, wrap_width: usize, tab_width: usize, buffer_version: usize, line_len: F) -> Self
    where
        F: Fn(usize) -> usize,
    {
        let width = wrap_width.max(1);
        let mut visual_counts = Vec::with_capacity(line_count);
        let mut visual_offsets = Vec::with_capacity(line_count);
        let mut total = 0;

        for i in 0..line_count {
            visual_offsets.push(total);
            let len = line_len(i);
            let count = Self::compute_visual_lines(len, width);
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

    /// Computes how many visual lines a line of the given display width needs.
    fn compute_visual_lines(display_width: usize, wrap_width: usize) -> u16 {
        if display_width == 0 {
            1
        } else {
            ((display_width + wrap_width - 1) / wrap_width) as u16
        }
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
        let sub_line = visual_row.saturating_sub(self.visual_offsets.get(line).copied().unwrap_or(0));
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
    pub fn invalidate_line<F>(&mut self, line: usize, line_len: F)
    where
        F: Fn(usize) -> usize,
    {
        if line >= self.visual_counts.len() {
            return;
        }
        let old_count = self.visual_counts[line] as usize;
        let new_count = Self::compute_visual_lines(line_len(line), self.wrap_width) as usize;
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
    pub fn rebuild<F>(&mut self, line_count: usize, wrap_width: usize, tab_width: usize, buffer_version: usize, line_len: F)
    where
        F: Fn(usize) -> usize,
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
            let len = line_len(i);
            let count = Self::compute_visual_lines(len, width);
            self.visual_counts.push(count);
            total += count as usize;
        }
        self.total_visual_lines = total;
    }

    /// Maps a cursor position (line, col) to a visual position (visual_row, visual_col).
    pub fn cursor_to_visual(&self, line: usize, col: usize) -> (usize, usize) {
        let base_row = self.logical_to_visual(line);
        let sub_line = col / self.wrap_width;
        let visual_col = col % self.wrap_width;
        (base_row + sub_line, visual_col)
    }

    /// Counts total visual lines from `start_line` to `end_line` (exclusive).
    pub fn visual_lines_in_range(&self, start_line: usize, end_line: usize) -> usize {
        let start = self.visual_offsets.get(start_line).copied().unwrap_or(0);
        let end = if end_line >= self.visual_counts.len() {
            self.total_visual_lines
        } else {
            self.visual_offsets.get(end_line).copied().unwrap_or(self.total_visual_lines)
        };
        end.saturating_sub(start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_line_fits() {
        let map = WrapMap::new(1, 80, 4, 0, |_| 40);
        assert_eq!(map.visual_lines_for(0), 1);
        assert_eq!(map.total_visual_lines(), 1);
    }

    #[test]
    fn test_line_exactly_fits() {
        let map = WrapMap::new(1, 80, 4, 0, |_| 80);
        assert_eq!(map.visual_lines_for(0), 1);
    }

    #[test]
    fn test_line_wraps_once() {
        let map = WrapMap::new(1, 80, 4, 0, |_| 81);
        assert_eq!(map.visual_lines_for(0), 2);
        assert_eq!(map.total_visual_lines(), 2);
    }

    #[test]
    fn test_empty_line() {
        let map = WrapMap::new(1, 80, 4, 0, |_| 0);
        assert_eq!(map.visual_lines_for(0), 1);
    }

    #[test]
    fn test_multiple_lines() {
        // Line 0: 40 chars (1 visual), Line 1: 160 chars (2 visual), Line 2: 0 (1 visual)
        let widths = [40, 160, 0];
        let map = WrapMap::new(3, 80, 4, 0, |i| widths[i]);
        assert_eq!(map.visual_lines_for(0), 1);
        assert_eq!(map.visual_lines_for(1), 2);
        assert_eq!(map.visual_lines_for(2), 1);
        assert_eq!(map.total_visual_lines(), 4);
    }

    #[test]
    fn test_logical_to_visual() {
        let widths = [40, 160, 0];
        let map = WrapMap::new(3, 80, 4, 0, |i| widths[i]);
        assert_eq!(map.logical_to_visual(0), 0);
        assert_eq!(map.logical_to_visual(1), 1);
        assert_eq!(map.logical_to_visual(2), 3);
    }

    #[test]
    fn test_visual_to_logical() {
        let widths = [40, 160, 0];
        let map = WrapMap::new(3, 80, 4, 0, |i| widths[i]);
        assert_eq!(map.visual_to_logical(0), (0, 0));
        assert_eq!(map.visual_to_logical(1), (1, 0));
        assert_eq!(map.visual_to_logical(2), (1, 1));
        assert_eq!(map.visual_to_logical(3), (2, 0));
    }

    #[test]
    fn test_cursor_to_visual() {
        let map = WrapMap::new(2, 80, 4, 0, |i| if i == 0 { 200 } else { 40 });
        // Line 0: 200 chars -> 3 visual lines
        // Cursor at col 85 -> sub_line 1, visual_col 5
        let (row, col) = map.cursor_to_visual(0, 85);
        assert_eq!(row, 1);
        assert_eq!(col, 5);
    }

    #[test]
    fn test_roundtrip() {
        let widths = [80, 161, 50, 240, 0];
        let map = WrapMap::new(5, 80, 4, 0, |i| widths[i]);
        for line in 0..5 {
            let visual = map.logical_to_visual(line);
            let (got_line, got_sub) = map.visual_to_logical(visual);
            assert_eq!(got_line, line);
            assert_eq!(got_sub, 0);
        }
    }

    #[test]
    fn test_invalidate_line() {
        let widths = [40usize, 160, 0];
        let mut map = WrapMap::new(3, 80, 4, 0, |i| widths[i]);
        assert_eq!(map.total_visual_lines(), 4);

        // Line 1 changed from 160 to 40 chars (2 -> 1 visual)
        map.invalidate_line(1, |_| 40);
        assert_eq!(map.visual_lines_for(1), 1);
        assert_eq!(map.total_visual_lines(), 3);
        assert_eq!(map.logical_to_visual(2), 2);
    }

    #[test]
    fn test_visual_lines_in_range() {
        let widths = [40, 160, 0, 80];
        let map = WrapMap::new(4, 80, 4, 0, |i| widths[i]);
        // Lines: 1 + 2 + 1 + 1 = 5 total
        assert_eq!(map.visual_lines_in_range(0, 4), 5);
        assert_eq!(map.visual_lines_in_range(1, 3), 3); // 2 + 1
        assert_eq!(map.visual_lines_in_range(0, 1), 1);
    }
}
