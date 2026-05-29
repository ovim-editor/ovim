//! Screen/viewport motions: H, M, L, Ctrl-D/U/F/B/E/Y

use super::Motions;
use crate::buffer::Buffer;
use crate::unicode::{grapheme_count, GraphemeCol};

impl Motions {
    /// Moves cursor to the top of the visible screen (H command)
    /// viewport_start: first visible line
    /// offset: optional offset from top (0 = first line, 1 = second line, etc.)
    pub fn move_to_screen_top(buffer: &mut Buffer, viewport_start: usize, offset: usize) {
        let target_line = (viewport_start + offset).min(buffer.line_count().saturating_sub(1));

        // Move to first non-blank character on the line
        if let Some(line_text) = buffer.line_text(target_line) {
            let first_non_blank = line_text
                .chars()
                .position(|c| !c.is_whitespace())
                .unwrap_or(0);

            // first_non_blank is a char index; convert to grapheme for cursor.
            buffer.set_cursor_char_col(target_line, crate::unicode::CharCol(first_non_blank));
        }
    }

    /// Moves cursor to the middle of the visible screen (M command)
    /// viewport_start: first visible line
    /// viewport_height: number of visible lines
    pub fn move_to_screen_middle(
        buffer: &mut Buffer,
        viewport_start: usize,
        viewport_height: usize,
    ) {
        let middle_offset = viewport_height / 2;
        let target_line =
            (viewport_start + middle_offset).min(buffer.line_count().saturating_sub(1));

        // Move to first non-blank character on the line
        if let Some(line_text) = buffer.line_text(target_line) {
            let first_non_blank = line_text
                .chars()
                .position(|c| !c.is_whitespace())
                .unwrap_or(0);

            // first_non_blank is a char index; convert to grapheme for cursor.
            buffer.set_cursor_char_col(target_line, crate::unicode::CharCol(first_non_blank));
        }
    }

    /// Moves cursor to the bottom of the visible screen (L command)
    /// viewport_start: first visible line
    /// viewport_height: number of visible lines
    /// offset: optional offset from bottom (0 = last line, 1 = second to last, etc.)
    pub fn move_to_screen_bottom(
        buffer: &mut Buffer,
        viewport_start: usize,
        viewport_height: usize,
        offset: usize,
    ) {
        let last_visible = viewport_start + viewport_height.saturating_sub(1);
        let target_line = last_visible
            .saturating_sub(offset)
            .min(buffer.line_count().saturating_sub(1));

        // Move to first non-blank character on the line
        if let Some(line_text) = buffer.line_text(target_line) {
            let first_non_blank = line_text
                .chars()
                .position(|c| !c.is_whitespace())
                .unwrap_or(0);

            // first_non_blank is a char index; convert to grapheme for cursor.
            buffer.set_cursor_char_col(target_line, crate::unicode::CharCol(first_non_blank));
        }
    }

    /// Scrolls forward (down) one full page (Ctrl-F / Page Down)
    /// Returns new viewport_start and moves cursor
    pub fn scroll_page_down(
        buffer: &mut Buffer,
        viewport_start: usize,
        viewport_height: usize,
    ) -> usize {
        let max_scroll = buffer.line_count().saturating_sub(viewport_height);
        let new_viewport = (viewport_start + viewport_height.saturating_sub(2)).min(max_scroll);

        // Move cursor down by the same amount (keep relative position in viewport)
        let cursor_line = buffer.cursor().line();
        let cursor_offset = cursor_line.saturating_sub(viewport_start);
        let new_cursor_line =
            (new_viewport + cursor_offset).min(buffer.line_count().saturating_sub(1));

        // Keep cursor in same column if possible
        let col = buffer.cursor().col();
        buffer.cursor_mut().set_position(new_cursor_line, col);

        // Adjust column to be within line bounds
        if let Some(line) = buffer.line_text(new_cursor_line) {
            let line_len = grapheme_count(&line);
            if line_len > 0 {
                let clamped_col = col.min(GraphemeCol(line_len.saturating_sub(1)));
                buffer.cursor_mut().set_col(clamped_col);
            } else {
                buffer.cursor_mut().set_col(GraphemeCol::ZERO);
            }
        }

        new_viewport
    }

    /// Scrolls backward (up) one full page (Ctrl-B / Page Up)
    /// Returns new viewport_start and moves cursor
    pub fn scroll_page_up(
        buffer: &mut Buffer,
        viewport_start: usize,
        viewport_height: usize,
    ) -> usize {
        let scroll_amount = viewport_height.saturating_sub(2);
        let new_viewport = viewport_start.saturating_sub(scroll_amount);

        // Move cursor up by the same amount (keep relative position in viewport)
        let cursor_line = buffer.cursor().line();
        let cursor_offset = cursor_line.saturating_sub(viewport_start);
        let new_cursor_line = new_viewport + cursor_offset;

        // Keep cursor in same column if possible
        let col = buffer.cursor().col();
        buffer.cursor_mut().set_position(new_cursor_line, col);

        // Adjust column to be within line bounds
        if let Some(line) = buffer.line_text(new_cursor_line) {
            let line_len = grapheme_count(&line);
            if line_len > 0 {
                let clamped_col = col.min(GraphemeCol(line_len.saturating_sub(1)));
                buffer.cursor_mut().set_col(clamped_col);
            } else {
                buffer.cursor_mut().set_col(GraphemeCol::ZERO);
            }
        }

        new_viewport
    }
}
