//! Paragraph motions: {, }

use super::Motions;
use crate::buffer::Buffer;
use crate::unicode::GraphemeCol;

impl Motions {
    /// Move forward to start of next paragraph ({ and } motions)
    pub fn paragraph_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::paragraph_forward_once(buffer);
        }
    }

    fn paragraph_forward_once(buffer: &mut Buffer) {
        let cursor = buffer.cursor();
        let mut line_idx = cursor.line();
        let total_lines = buffer.line_count();

        // First skip any blank lines at/after cursor
        while line_idx < total_lines {
            if let Some(line) = buffer.line(line_idx) {
                if !line.trim().is_empty() {
                    break;
                }
            }
            line_idx += 1;
        }

        // Then skip non-blank lines to find the next blank line boundary
        while line_idx < total_lines {
            if let Some(line) = buffer.line(line_idx) {
                if line.trim().is_empty() {
                    break;
                }
            }
            line_idx += 1;
        }

        // Clamp to buffer bounds
        line_idx = line_idx.min(total_lines.saturating_sub(1));
        buffer.cursor_mut().set_position(line_idx, GraphemeCol::ZERO);
    }

    /// Move backward to start of previous paragraph
    pub fn paragraph_backward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::paragraph_backward_once(buffer);
        }
    }

    fn paragraph_backward_once(buffer: &mut Buffer) {
        let cursor = buffer.cursor();
        let mut line_idx = cursor.line();

        if line_idx == 0 {
            return;
        }

        line_idx = line_idx.saturating_sub(1);

        // Fix: Skip blank lines backward - check line 0 explicitly
        while line_idx > 0 {
            if let Some(line) = buffer.line(line_idx) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    break;
                }
            }
            line_idx = line_idx.saturating_sub(1);
        }
        // Check line 0 after loop (loop condition skips it)
        if line_idx == 0 {
            if let Some(line) = buffer.line(0) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    // Line 0 is non-blank, continue to next phase
                } else {
                    // Line 0 is blank, stop here
                    buffer.cursor_mut().set_position(0, GraphemeCol::ZERO);
                    return;
                }
            }
        }

        // Fix: Skip non-blank lines backward until we find a blank line
        while line_idx > 0 {
            if let Some(line) = buffer.line(line_idx) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    break; // Stop at the blank line
                }
            }
            line_idx = line_idx.saturating_sub(1);
        }
        // Check line 0 after loop - if we're here, check if it's blank
        if line_idx == 0 {
            if let Some(line) = buffer.line(0) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    // Line 0 is non-blank, we've gone as far back as we can
                    // The paragraph starts at line 0
                }
                // If line 0 is blank, line_idx is already 0
            }
        }

        buffer.cursor_mut().set_position(line_idx, GraphemeCol::ZERO);
    }
}
