//! Section motions: ]], [[, ][, []

use super::Motions;
use crate::buffer::Buffer;
use crate::unicode::GraphemeCol;

impl Motions {
    /// Section navigation: jump to next section start (`{` at column 0)
    /// `]]` motion in Vim
    pub fn section_forward(buffer: &mut Buffer, count: usize) {
        let total_lines = buffer.line_count();
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            current_line += 1;
            while current_line < total_lines {
                if let Some(line) = buffer.line_text(current_line) {
                    if line.starts_with('{') {
                        break;
                    }
                }
                current_line += 1;
            }
            if current_line >= total_lines {
                current_line = total_lines.saturating_sub(1);
                break;
            }
        }

        buffer
            .cursor_mut()
            .set_position(current_line, GraphemeCol::ZERO);
    }

    /// Section navigation: jump to previous section start (`{` at column 0)
    /// `[[` motion in Vim
    pub fn section_backward(buffer: &mut Buffer, count: usize) {
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            if current_line == 0 {
                break;
            }
            current_line -= 1;
            while current_line > 0 {
                if let Some(line) = buffer.line_text(current_line) {
                    if line.starts_with('{') {
                        break;
                    }
                }
                current_line -= 1;
            }
            // Check if line 0 is a match
            if current_line == 0 {
                if let Some(line) = buffer.line_text(0) {
                    if !line.starts_with('{') {
                        // No match found, stay at line 0
                    }
                }
            }
        }

        buffer
            .cursor_mut()
            .set_position(current_line, GraphemeCol::ZERO);
    }

    /// Section navigation: jump to next section end (`}` at column 0)
    /// `][` motion in Vim
    pub fn section_end_forward(buffer: &mut Buffer, count: usize) {
        let total_lines = buffer.line_count();
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            current_line += 1;
            while current_line < total_lines {
                if let Some(line) = buffer.line_text(current_line) {
                    if line.starts_with('}') {
                        break;
                    }
                }
                current_line += 1;
            }
            if current_line >= total_lines {
                current_line = total_lines.saturating_sub(1);
                break;
            }
        }

        buffer
            .cursor_mut()
            .set_position(current_line, GraphemeCol::ZERO);
    }

    /// Section navigation: jump to previous section end (`}` at column 0)
    /// `[]` motion in Vim
    pub fn section_end_backward(buffer: &mut Buffer, count: usize) {
        let mut current_line = buffer.cursor().line();

        for _ in 0..count {
            if current_line == 0 {
                break;
            }
            current_line -= 1;
            while current_line > 0 {
                if let Some(line) = buffer.line_text(current_line) {
                    if line.starts_with('}') {
                        break;
                    }
                }
                current_line -= 1;
            }
        }

        buffer
            .cursor_mut()
            .set_position(current_line, GraphemeCol::ZERO);
    }
}
