use crate::buffer::Buffer;
use anyhow::Result;

/// Represents the different operators in Vim
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Dedent,
    AutoIndent,
    Lowercase,
    Uppercase,
    ToggleCase,
    ReplaceWithRegister,
    Fold,
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

        // Adjust cursor - clamp to new line bounds
        // After deleting to end of line, cursor should stay at same column or move to last char
        let line = buffer.rope().line(line_idx);
        let line_text = line.to_string().trim_end_matches('\n').to_string();
        let new_line_len = line_text.chars().count();

        let new_col = if new_line_len > 0 {
            col.min(new_line_len - 1)
        } else {
            0
        };
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

    /// Indents line(s) by adding spaces/tabs at the beginning
    /// Returns the number of lines indented
    pub fn indent_lines(
        buffer: &mut Buffer,
        start_line: usize,
        end_line: usize,
        tab_width: usize,
    ) -> Result<usize> {
        let indent_str = " ".repeat(tab_width);
        let mut lines_indented = 0;

        for line_idx in start_line..end_line.min(buffer.line_count()) {
            // Insert indent at the beginning of the line
            buffer.insert_text_at(line_idx, 0, &indent_str);
            lines_indented += 1;
        }

        Ok(lines_indented)
    }

    /// Dedents line(s) by removing spaces/tabs from the beginning
    /// Returns the number of lines dedented
    pub fn dedent_lines(
        buffer: &mut Buffer,
        start_line: usize,
        end_line: usize,
        tab_width: usize,
    ) -> Result<usize> {
        let mut lines_dedented = 0;

        for line_idx in start_line..end_line.min(buffer.line_count()) {
            if let Some(line) = buffer.line(line_idx) {
                let line_text = line.trim_end_matches('\n');

                // Count leading whitespace to remove (up to tab_width)
                let chars: Vec<char> = line_text.chars().collect();
                let mut spaces_to_remove = 0;

                for &ch in chars.iter().take(tab_width) {
                    if ch == ' ' {
                        spaces_to_remove += 1;
                    } else if ch == '\t' {
                        spaces_to_remove = tab_width;
                        break;
                    } else {
                        break;
                    }
                }

                if spaces_to_remove > 0 {
                    buffer.delete_range(line_idx, 0, line_idx, spaces_to_remove);
                    lines_dedented += 1;
                }
            }
        }

        Ok(lines_dedented)
    }

    /// Auto-indents lines based on bracket context (= operator)
    /// Returns the number of lines auto-indented
    pub fn auto_indent_lines(
        buffer: &mut Buffer,
        start_line: usize,
        end_line: usize,
        tab_width: usize,
    ) -> Result<usize> {
        let end_line = end_line.min(buffer.line_count());
        if start_line >= end_line {
            return Ok(0);
        }

        // Determine base indent from the line before start_line (or 0 if first line)
        let mut current_indent = if start_line > 0 {
            if let Some(prev_line) = buffer.line(start_line - 1) {
                let prev_text = prev_line.trim_end_matches('\n');
                Self::count_leading_spaces(prev_text, tab_width)
                    + if prev_text.trim_end().ends_with('{')
                        || prev_text.trim_end().ends_with('(')
                        || prev_text.trim_end().ends_with('[')
                    {
                        tab_width
                    } else {
                        0
                    }
            } else {
                0
            }
        } else {
            0
        };

        let mut lines_indented = 0;

        for line_idx in start_line..end_line {
            if let Some(line) = buffer.line(line_idx) {
                let line_text = line.trim_end_matches('\n');
                let trimmed = line_text.trim_start();

                // Decrease indent if line starts with closing bracket
                if trimmed.starts_with('}')
                    || trimmed.starts_with(')')
                    || trimmed.starts_with(']')
                {
                    current_indent = current_indent.saturating_sub(tab_width);
                }

                // Calculate current leading spaces
                let current_spaces = Self::count_leading_spaces(line_text, tab_width);

                // Apply new indentation if different
                if current_spaces != current_indent && !trimmed.is_empty() {
                    // Remove existing indent
                    let leading_len = line_text.len() - trimmed.len();
                    if leading_len > 0 {
                        buffer.delete_range(line_idx, 0, line_idx, leading_len);
                    }
                    // Add new indent
                    if current_indent > 0 {
                        let indent_str = " ".repeat(current_indent);
                        buffer.insert_text_at(line_idx, 0, &indent_str);
                    }
                    lines_indented += 1;
                }

                // Increase indent if line ends with opening bracket
                if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
                    current_indent += tab_width;
                }
            }
        }

        Ok(lines_indented)
    }

    /// Count leading spaces (tabs count as tab_width spaces)
    fn count_leading_spaces(line: &str, tab_width: usize) -> usize {
        let mut count = 0;
        for ch in line.chars() {
            match ch {
                ' ' => count += 1,
                '\t' => count += tab_width,
                _ => break,
            }
        }
        count
    }

    /// Joins the current line with the next line (J command)
    /// Adds a space between the lines unless the current line already ends with whitespace
    pub fn join_lines(buffer: &mut Buffer, count: usize) -> Result<()> {
        Self::join_lines_impl(buffer, count, true)
    }

    /// Joins lines without adding a space (gJ command)
    pub fn join_lines_no_space(buffer: &mut Buffer, count: usize) -> Result<()> {
        Self::join_lines_impl(buffer, count, false)
    }

    /// Internal implementation for joining lines
    fn join_lines_impl(buffer: &mut Buffer, count: usize, add_space: bool) -> Result<()> {
        let start_line = buffer.cursor().line();
        let cursor_col = buffer.cursor().col();

        // Join 'count' times (count = 1 means join current with next)
        let lines_to_join = count.max(1);

        for _ in 0..lines_to_join {
            if start_line >= buffer.line_count().saturating_sub(1) {
                // Already at the last line, nothing to join
                break;
            }

            // Get the current line and next line
            let current_line_text = match buffer.line(start_line) {
                Some(text) => text.trim_end_matches('\n').to_string(),
                None => break,
            };

            let next_line_text = match buffer.line(start_line + 1) {
                Some(text) => text.trim_end_matches('\n').to_string(),
                None => break,
            };

            // Determine if we need to add a space
            let separator = if add_space {
                // Add space unless current line ends with whitespace
                if current_line_text.ends_with(|c: char| c.is_whitespace()) {
                    ""
                } else {
                    " "
                }
            } else {
                ""
            };

            // Trim leading whitespace from next line
            let next_trimmed = next_line_text.trim_start();

            // Build the joined line
            let joined = if next_trimmed.is_empty() {
                // Next line is all whitespace, just use current line
                current_line_text.clone()
            } else {
                format!("{}{}{}", current_line_text, separator, next_trimmed)
            };

            // Delete both lines (from start_line to start_line+2)
            buffer.delete_range(start_line, 0, start_line + 2, 0);

            // Insert the joined line with newline
            buffer.insert_text_at(start_line, 0, &format!("{}\n", joined));

            // Keep cursor at the original line, but clamp column
            let new_col = cursor_col.min(joined.len().saturating_sub(1).max(0));
            buffer.cursor_mut().set_position(start_line, new_col);
        }

        Ok(())
    }
}
