use crate::buffer::Buffer;
use anyhow::Result;

/// Represents the different operators in Vim
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
}

/// Handles operator commands (delete, change, yank)
pub struct Operators;

impl Operators {
    /// Deletes from current position to the end of line
    pub fn delete_to_end_of_line(buffer: &mut Buffer) -> Result<String> {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= buffer.line_count() {
            return Ok(String::new());
        }

        let line_start = buffer.rope().line_to_char(line_idx);
        let line = buffer.rope().line(line_idx);
        let line_end_char = line_start + line.len_chars();

        // Calculate position to delete from (current column)
        let delete_from = line_start + col;

        // Delete to end of line (but keep the newline)
        let line_text = line.to_string();
        let ends_with_newline = line_text.ends_with('\n');
        let delete_to = if ends_with_newline {
            line_end_char - 1
        } else {
            line_end_char
        };

        if delete_from >= delete_to {
            return Ok(String::new());
        }

        // Store deleted text
        let deleted = buffer.rope().slice(delete_from..delete_to).to_string();

        // Remove the text
        buffer.rope_mut().remove(delete_from..delete_to);

        // Adjust cursor if needed
        let new_col = col.saturating_sub(1);
        buffer.cursor_mut().set_col(new_col);

        Ok(deleted)
    }

    /// Deletes entire line(s)
    pub fn delete_line(buffer: &mut Buffer, count: usize) -> Result<String> {
        let cursor = buffer.cursor();
        let start_line = cursor.line();
        let end_line = (start_line + count).min(buffer.line_count());

        if start_line >= buffer.line_count() {
            return Ok(String::new());
        }

        let start_char = buffer.rope().line_to_char(start_line);
        let end_char = if end_line < buffer.line_count() {
            buffer.rope().line_to_char(end_line)
        } else {
            buffer.rope().len_chars()
        };

        // Store deleted text
        let deleted = buffer.rope().slice(start_char..end_char).to_string();

        // Remove the lines
        buffer.rope_mut().remove(start_char..end_char);

        // Position cursor at start of line
        let new_line = start_line.min(buffer.line_count().saturating_sub(1));
        buffer.cursor_mut().set_position(new_line, 0);

        Ok(deleted)
    }

    /// Deletes a word forward from cursor
    pub fn delete_word(buffer: &mut Buffer, count: usize) -> Result<String> {
        let start_cursor = buffer.cursor().clone();
        let start_line = start_cursor.line();
        let start_col = start_cursor.col();
        let start_char = buffer.rope().line_to_char(start_line) + start_col;

        // Move cursor forward by word count times
        crate::editor::Motions::word_forward(buffer, count);

        let end_cursor = buffer.cursor();
        let end_line = end_cursor.line();
        let end_col = end_cursor.col();
        let end_char = buffer.rope().line_to_char(end_line) + end_col;

        // Store deleted text
        let deleted = buffer.rope().slice(start_char..end_char).to_string();

        // Remove the text
        buffer.rope_mut().remove(start_char..end_char);

        // Reset cursor to start position
        buffer.cursor_mut().set_position(start_line, start_col);

        Ok(deleted)
    }

    /// Deletes a character under the cursor
    pub fn delete_char(buffer: &mut Buffer, count: usize) -> Result<String> {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= buffer.line_count() {
            return Ok(String::new());
        }

        let line = buffer.line(line_idx).unwrap_or_default();
        let line_text = line.trim_end_matches('\n');
        let chars_count = line_text.chars().count();

        if col >= chars_count {
            return Ok(String::new());
        }

        let line_start = buffer.rope().line_to_char(line_idx);
        let delete_start = line_start + col;
        let delete_end = (delete_start + count).min(line_start + chars_count);

        // Store deleted text
        let deleted = buffer.rope().slice(delete_start..delete_end).to_string();

        // Remove characters
        buffer.rope_mut().remove(delete_start..delete_end);

        // Adjust cursor if at end of line
        let new_line = buffer.line(line_idx).unwrap_or_default();
        let new_line_len = new_line.trim_end_matches('\n').chars().count();
        if col >= new_line_len && new_line_len > 0 {
            buffer.cursor_mut().set_col(new_line_len - 1);
        }

        Ok(deleted)
    }

    /// Yanks (copies) from current position to end of line
    pub fn yank_to_end_of_line(buffer: &Buffer) -> Result<String> {
        let cursor = buffer.cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if line_idx >= buffer.line_count() {
            return Ok(String::new());
        }

        let line_start = buffer.rope().line_to_char(line_idx);
        let line = buffer.rope().line(line_idx);
        let line_end_char = line_start + line.len_chars();

        let yank_from = line_start + col;
        let line_text = line.to_string();
        let ends_with_newline = line_text.ends_with('\n');
        let yank_to = if ends_with_newline {
            line_end_char - 1
        } else {
            line_end_char
        };

        if yank_from >= yank_to {
            return Ok(String::new());
        }

        Ok(buffer.rope().slice(yank_from..yank_to).to_string())
    }

    /// Yanks (copies) entire line(s)
    pub fn yank_line(buffer: &Buffer, count: usize) -> Result<String> {
        let cursor = buffer.cursor();
        let start_line = cursor.line();
        let end_line = (start_line + count).min(buffer.line_count());

        if start_line >= buffer.line_count() {
            return Ok(String::new());
        }

        let start_char = buffer.rope().line_to_char(start_line);
        let end_char = if end_line < buffer.line_count() {
            buffer.rope().line_to_char(end_line)
        } else {
            buffer.rope().len_chars()
        };

        let mut yanked = buffer.rope().slice(start_char..end_char).to_string();

        // Ensure line yanks always end with newline (for line-wise paste behavior)
        if !yanked.ends_with('\n') {
            yanked.push('\n');
        }

        Ok(yanked)
    }

    /// Yanks a word forward from cursor
    pub fn yank_word(buffer: &mut Buffer, count: usize) -> Result<String> {
        let start_cursor = buffer.cursor().clone();
        let start_line = start_cursor.line();
        let start_col = start_cursor.col();
        let start_char = buffer.rope().line_to_char(start_line) + start_col;

        // Move cursor forward by word
        crate::editor::Motions::word_forward(buffer, count);

        let end_cursor = buffer.cursor();
        let end_line = end_cursor.line();
        let end_col = end_cursor.col();
        let end_char = buffer.rope().line_to_char(end_line) + end_col;

        // Get yanked text
        let yanked = buffer.rope().slice(start_char..end_char).to_string();

        // Reset cursor to start position
        buffer.cursor_mut().set_position(start_line, start_col);

        Ok(yanked)
    }
}
