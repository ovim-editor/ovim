use std::time::Instant;

const FLASH_DURATION_MS: u128 = 150;

/// Represents a region to briefly highlight after a yank operation.
#[derive(Debug, Clone)]
pub struct YankFlash {
    pub start_line: usize,
    pub end_line: usize,
    /// If None, the entire line(s) are flashed (linewise).
    /// If Some, only the column range is flashed (character-wise).
    pub col_range: Option<(usize, usize)>,
    created_at: Instant,
}

impl YankFlash {
    pub fn lines(start_line: usize, end_line: usize) -> Self {
        Self {
            start_line,
            end_line,
            col_range: None,
            created_at: Instant::now(),
        }
    }

    pub fn range(start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> Self {
        Self {
            start_line,
            end_line,
            col_range: Some((start_col, end_col)),
            created_at: Instant::now(),
        }
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed().as_millis() >= FLASH_DURATION_MS
    }

    /// Returns true if the given line index is within the flash region.
    pub fn contains_line(&self, line_idx: usize) -> bool {
        line_idx >= self.start_line && line_idx <= self.end_line
    }

    /// Returns the column range to highlight for a given line, or None if not flashed.
    /// For linewise flash, returns None (meaning highlight the whole line).
    /// For character-wise flash on a single line, returns the col range.
    pub fn col_range_for_line(&self, _line_idx: usize) -> Option<(usize, usize)> {
        self.col_range
    }
}
