use super::Editor;
use crate::mode::Mode;

impl Editor {
    /// Gets the visual selection start position
    pub fn visual_start(&self) -> Option<(usize, usize)> {
        self.visual.visual_start
    }

    /// Sets the visual selection start position
    pub fn set_visual_start(&mut self, line: usize, col: usize) {
        self.visual.visual_start = Some((line, col));
    }

    /// Clears the visual selection
    pub fn clear_visual_start(&mut self) {
        self.visual.visual_start = None;
    }

    /// Saves the current visual selection for gv command
    pub fn save_last_visual_selection(&mut self) {
        if let Some(selection) = self.visual_selection() {
            self.visual.last_visual_selection = Some((selection.0, selection.1, self.mode));
        }
    }

    /// Restores the last visual selection (gv command)
    pub fn restore_last_visual_selection(&mut self) {
        if let Some((start, end, mode)) = self.visual.last_visual_selection {
            // Set the mode first
            self.mode = mode;

            // Clamp start position to buffer bounds
            let line_count = self.buffer().line_count();
            let clamped_start_line = start.0.min(line_count.saturating_sub(1));
            let start_line_len = self
                .buffer()
                .line(clamped_start_line)
                .map(|l| l.trim_end_matches('\n').chars().count())
                .unwrap_or(0);
            let clamped_start_col = if mode == crate::mode::Mode::VisualLine {
                // For VisualLine, always use column 0
                0
            } else {
                start.1.min(start_line_len.saturating_sub(1).max(0))
            };

            // Clamp end position to buffer bounds
            let clamped_end_line = end.0.min(line_count.saturating_sub(1));
            let end_line_len = self
                .buffer()
                .line(clamped_end_line)
                .map(|l| l.trim_end_matches('\n').chars().count())
                .unwrap_or(0);
            let clamped_end_col = if mode == crate::mode::Mode::VisualLine {
                // For VisualLine, always use column 0
                0
            } else {
                end.1.min(end_line_len.saturating_sub(1).max(0))
            };

            // Set visual start
            self.visual.visual_start = Some((clamped_start_line, clamped_start_col));
            // Move cursor to end position
            self.buffer_mut()
                .cursor_mut()
                .set_position(clamped_end_line, clamped_end_col);
        }
    }

    /// Sets visual block insert/append state for replay on insert mode exit
    pub fn set_visual_block_insert_state(
        &mut self,
        state: Option<(usize, usize, usize, bool, bool)>,
    ) {
        self.visual.visual_block_insert_state = state;
    }

    /// Gets visual block insert/append state
    pub fn visual_block_insert_state(&self) -> Option<(usize, usize, usize, bool, bool)> {
        self.visual.visual_block_insert_state
    }

    /// Returns true when `$` was pressed in visual block mode (extend to EOL).
    pub fn visual_block_dollar(&self) -> bool {
        self.visual.visual_block_dollar
    }

    pub fn set_visual_block_dollar(&mut self, val: bool) {
        self.visual.visual_block_dollar = val;
    }

    /// Gets the visual selection range (start and end positions)
    /// Returns ((start_line, start_col), (end_line, end_col))
    /// Note: For VisualBlock, this returns the corners of the rectangle
    pub fn visual_selection(&self) -> Option<((usize, usize), (usize, usize))> {
        self.visual.visual_start.map(|start| {
            let cursor = self.buffer().cursor();
            let mut end = (cursor.line(), cursor.col());

            match self.mode {
                Mode::VisualLine => {
                    // Get the length of the end line (excluding newline)
                    if let Some(line_text) = self.buffer().line(end.0) {
                        let line_len = line_text.trim_end_matches('\n').chars().count();
                        end.1 = if line_len > 0 { line_len - 1 } else { 0 };
                    }

                    // Also ensure start is at beginning of its line
                    let mut start = start;
                    start.1 = 0;

                    // Normalize so start is always before end
                    if start.0 <= end.0 {
                        (start, end)
                    } else {
                        // If cursor moved above start line, swap and adjust
                        let mut new_start = end;
                        new_start.1 = 0;
                        let mut new_end = start;
                        if let Some(line_text) = self.buffer().line(new_end.0) {
                            let line_len = line_text.trim_end_matches('\n').chars().count();
                            new_end.1 = if line_len > 0 { line_len - 1 } else { 0 };
                        }
                        (new_start, new_end)
                    }
                }
                Mode::VisualBlock => {
                    // Block mode: return corners of rectangle
                    // Normalize so start_line <= end_line and start_col <= end_col

                    let (min_line, max_line) = if start.0 <= end.0 {
                        (start.0, end.0)
                    } else {
                        (end.0, start.0)
                    };

                    let (min_col, max_col) = if self.visual.visual_block_dollar {
                        // `$` was pressed: extend each line to its own EOL.
                        // Use usize::MAX - 1 as sentinel (avoids +1 overflow in callers).
                        (start.1.min(end.1), usize::MAX - 1)
                    } else if start.1 <= end.1 {
                        (start.1, end.1)
                    } else {
                        (end.1, start.1)
                    };

                    ((min_line, min_col), (max_line, max_col))
                }
                _ => {
                    // Normal visual mode behavior
                    // Normalize so start is always before end
                    if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
                        (start, end)
                    } else {
                        (end, start)
                    }
                }
            }
        })
    }
}
