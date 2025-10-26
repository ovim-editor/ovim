//! Helper functions for input handling

use crate::buffer::Buffer;
use crate::editor::{
    Change, Editor, FindDirection, FindType, Motions, Operator, Operators, Range, RegisterType,
    Search, TextObjects,
};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

/// Type of case change operation
pub(super) enum CaseChange {
    Lowercase,
    Uppercase,
    Toggle,
}

        // Use a very short timeout to keep the event loop responsive
        // This allows status updates and rendering to happen frequently
        if event::poll(std::time::Duration::from_millis(16))? {
            // ~60 FPS
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }

    // Helper methods for cursor movement and editing

    pub(super) fn move_left(editor: &mut Editor) {
        let count = editor.effective_count();
        let cursor = editor.buffer_mut().cursor_mut();
        if cursor.col() >= count {
            cursor.move_left(count);
        } else {
            cursor.set_col(0);
        }
        editor.clear_count();
    }

    pub(super) fn move_right(editor: &mut Editor) {
        let count = editor.effective_count();
        let line_idx = editor.buffer().cursor().line();
        let mode = editor.mode();
        if let Some(line) = editor.buffer().line(line_idx) {
            let line_len = line.trim_end_matches('\n').chars().count();
            let cursor = editor.buffer_mut().cursor_mut();
            let old_col = cursor.col();

            // In VisualBlock mode, allow cursor beyond line end for rectangular selection
            let new_col = if mode == Mode::VisualBlock {
                cursor.col() + count
            } else {
                (cursor.col() + count).min(line_len.saturating_sub(1).max(0))
            };

            eprintln!(
                "[DEBUG move_right] line={}, line_len={}, old_col={}, new_col={}, mode={:?}",
                line_idx, line_len, old_col, new_col, mode
            );
            cursor.set_col(new_col);
        }
        editor.clear_count();
    }

    pub(super) fn move_up(editor: &mut Editor) {
        let count = editor.effective_count();
        let cursor = editor.buffer_mut().cursor_mut();
        cursor.move_up(count);
        Self::clamp_cursor_with_goal_column(editor);
        editor.clear_count();
    }

    pub(super) fn move_down(editor: &mut Editor) {
        let count = editor.effective_count();
        let line_count = editor.buffer().line_count();
        let mut max_line = line_count.saturating_sub(1);

        // Check if last line is empty (just a newline)
        // If so, don't allow moving to it (Neovim behavior)
        if max_line < line_count {
            if let Some(last_line) = editor.buffer().line(max_line) {
                if last_line == "\n" || last_line.is_empty() {
                    max_line = max_line.saturating_sub(1);
                }
            }
        }

        let cursor = editor.buffer_mut().cursor_mut();
        let new_line = (cursor.line() + count).min(max_line);
        cursor.set_line(new_line);
        Self::clamp_cursor_with_goal_column(editor);
        editor.clear_count();
    }

    pub(super) fn clamp_cursor_to_line(editor: &mut Editor) {
        let line_idx = editor.buffer().cursor().line();
        if let Some(line) = editor.buffer().line(line_idx) {
            let line_len = line.trim_end_matches('\n').chars().count();
            let cursor = editor.buffer_mut().cursor_mut();
            if cursor.col() >= line_len {
                let new_col = if line_len > 0 { line_len - 1 } else { 0 };
                cursor.set_col(new_col);
            }
        }
    }

    pub(super) fn clamp_cursor_with_goal_column(editor: &mut Editor) {
        let line_idx = editor.buffer().cursor().line();
        let mode = editor.mode();
        if let Some(line) = editor.buffer().line(line_idx) {
            let line_len = line.trim_end_matches('\n').chars().count();
            let max_col = if line_len > 0 { line_len - 1 } else { 0 };
            let cursor = editor.buffer_mut().cursor_mut();
            let desired = cursor.desired_col();
            let _old_col = cursor.col();

            // In VisualBlock mode, preserve desired column even if beyond line end
            let target_col = if mode == Mode::VisualBlock {
                desired
            } else if desired == usize::MAX {
                // usize::MAX is a sentinel value meaning "always end of line"
                max_col
            } else {
                desired.min(max_col)
            };

            // eprintln!("[DEBUG clamp_cursor_with_goal_column] line={}, line_len={}, max_col={}, desired={}, old_col={}, target_col={}, mode={:?}",
            //   line_idx, line_len, max_col, desired, old_col, target_col, mode);
            cursor.set_col_preserve_desired(target_col);
        }
    }

    pub(super) fn insert_char(editor: &mut Editor, c: char) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let position = (cursor.line(), cursor.col());

        // Create and apply the change
        let change = Change::insert(position, c.to_string(), cursor_before);
        change.apply(editor.buffer_mut());
        editor.add_change(change);

        Ok(())
    }

    pub(super) fn insert_newline(editor: &mut Editor) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let position = (cursor.line(), cursor.col());

        // Create and apply the change
        let change = Change::insert(position, "\n".to_string(), cursor_before);
        change.apply(editor.buffer_mut());
        editor.add_change(change);

        Ok(())
    }

    pub(super) fn delete_char_before_cursor(editor: &mut Editor) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        if col == 0 && line_idx == 0 {
            // At start of buffer, nothing to delete
            return Ok(());
        }

        let (start_pos, end_pos, deleted_text) = if col == 0 {
            // Delete newline at end of previous line
            let prev_line_len = editor
                .buffer()
                .line(line_idx - 1)
                .map(|s| s.trim_end_matches('\n').chars().count())
                .unwrap_or(0);
            (
                (line_idx - 1, prev_line_len),
                (line_idx, 0),
                "\n".to_string(),
            )
        } else {
            // Delete character before cursor on same line
            // Get the actual character to delete
            let line_start = editor.buffer().rope().line_to_char(line_idx);
            let delete_pos = line_start + col - 1;
            let deleted_char = editor.buffer().rope().get_char(delete_pos).unwrap_or(' ');
            (
                (line_idx, col - 1),
                (line_idx, col),
                deleted_char.to_string(),
            )
        };

        let range = Range::new(start_pos, end_pos);
        let change = Change::delete(range, deleted_text, cursor_before);
        change.apply(editor.buffer_mut());
        editor.add_change(change);

        Ok(())
    }

    pub(super) fn delete_word_backward_insert(editor: &mut Editor) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        if col == 0 && line_idx == 0 {
            // At start of buffer, nothing to delete
            return Ok(());
        }

        // If at start of line, delete the newline character
        if col == 0 {
            let prev_line_len = editor
                .buffer()
                .line(line_idx - 1)
                .map(|s| s.trim_end_matches('\n').chars().count())
                .unwrap_or(0);
            let start_pos = (line_idx - 1, prev_line_len);
            let end_pos = (line_idx, 0);
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, "\n".to_string(), cursor_before);
            change.apply(editor.buffer_mut());
            editor.add_change(change);
            return Ok(());
        }

        // Get the line text
        let line = editor.buffer().line(line_idx).unwrap_or_default();
        let line_text = line.trim_end_matches('\n');
        let chars: Vec<char> = line_text.chars().collect();

        // Find the start of the word to delete
        let mut start_col = col;

        // Skip trailing whitespace
        while start_col > 0
            && chars
                .get(start_col - 1)
                .map_or(false, |c| c.is_whitespace())
        {
            start_col -= 1;
        }

        // If we only found whitespace, we're done
        if start_col == col {
            // No whitespace found, delete the word
            // Determine if we're in a word (alphanumeric/underscore) or punctuation
            if start_col > 0 {
                let char_at_cursor = chars.get(start_col - 1);
                let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

                if let Some(&ch) = char_at_cursor {
                    if is_word_char(ch) {
                        // Delete word characters
                        while start_col > 0
                            && chars.get(start_col - 1).map_or(false, |&c| is_word_char(c))
                        {
                            start_col -= 1;
                        }
                    } else {
                        // Delete punctuation/special characters
                        while start_col > 0
                            && chars
                                .get(start_col - 1)
                                .map_or(false, |&c| !is_word_char(c) && !c.is_whitespace())
                        {
                            start_col -= 1;
                        }
                    }
                }
            }
        }

        // Delete the range
        if start_col < col {
            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, start_col, line_idx, col);
            let range = Range::new((line_idx, start_col), (line_idx, col));
            let change = Change::delete(range, deleted, cursor_before);
            editor.add_change(change);
        }

        Ok(())
    }

    pub(super) fn delete_to_line_start_insert(editor: &mut Editor) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        // If already at start of line, do nothing
        if col == 0 {
            return Ok(());
        }

        // Delete from start of line to cursor
        let deleted = editor.buffer_mut().delete_range(line_idx, 0, line_idx, col);
        let range = Range::new((line_idx, 0), (line_idx, col));
        let change = Change::delete(range, deleted, cursor_before);
        editor.add_change(change);

        Ok(())
    }

    pub(super) fn indent_line_insert(editor: &mut Editor) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        // Get tab width from config or use default
        let tab_width = editor.options.tab_width;

        // Create indent string
        let indent_str = " ".repeat(tab_width);

        // Insert indent at beginning of line
        editor.buffer_mut().insert_text_at(line_idx, 0, &indent_str);

        // Update cursor position - move column right by tab_width
        let new_col = col + tab_width;
        editor.buffer_mut().cursor_mut().set_col(new_col);

        // Create change for undo
        let change = Change::insert((line_idx, 0), indent_str, cursor_before);
        editor.add_change(change);

        Ok(())
    }

    pub(super) fn dedent_line_insert(editor: &mut Editor) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        // Get tab width from config or use default
        let tab_width = editor.options.tab_width;

        // Get current line
        let line = match editor.buffer().line(line_idx) {
            Some(l) => l,
            None => return Ok(()),
        };
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

        // If no leading whitespace, do nothing
        if spaces_to_remove == 0 {
            return Ok(());
        }

        // Delete the leading whitespace
        let deleted = editor
            .buffer_mut()
            .delete_range(line_idx, 0, line_idx, spaces_to_remove);

        // Update cursor position - move column left by spaces_to_remove
        let new_col = col.saturating_sub(spaces_to_remove);
        editor.buffer_mut().cursor_mut().set_col(new_col);

        // Create change for undo
        let range = Range::new((line_idx, 0), (line_idx, spaces_to_remove));
        let change = Change::delete(range, deleted, cursor_before);
        editor.add_change(change);

        Ok(())
    }

    pub(super) fn insert_line_below(editor: &mut Editor) -> Result<()> {
        let line_idx = editor.buffer().cursor().line();
        let line_start = editor.buffer().rope().line_to_char(line_idx);
        let line_len = editor.buffer().rope().line(line_idx).len_chars();
        let insert_pos = line_start + line_len;

        // Get indentation from current line
        let line_text = editor.buffer().line(line_idx).unwrap_or_default();
        let indent = line_text
            .chars()
            .take_while(|c| c.is_whitespace() && *c != '\n')
            .collect::<String>();

        // Check if line already ends with newline
        let added_newline = if !line_text.ends_with('\n') {
            editor.buffer_mut().rope_mut().insert_char(insert_pos, '\n');
            true
        } else {
            false
        };

        // Insert newline with indentation
        // If we added a newline, insert_pos moved by 1, so insert at insert_pos + 1
        // If line already had newline, insert_pos is at start of next line, so insert there
        let text_to_insert = format!("{}\n", indent);
        let final_insert_pos = if added_newline {
            insert_pos + 1
        } else {
            insert_pos
        };
        editor
            .buffer_mut()
            .rope_mut()
            .insert(final_insert_pos, &text_to_insert);

        // Position cursor at end of indentation on new line
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line_idx + 1, indent.len());
        Ok(())
    }

    pub(super) fn insert_line_above(editor: &mut Editor) -> Result<()> {
        let line_idx = editor.buffer().cursor().line();
        let line_start = editor.buffer().rope().line_to_char(line_idx);

        // Get indentation from current line
        let line_text = editor.buffer().line(line_idx).unwrap_or_default();
        let indent = line_text
            .chars()
            .take_while(|c| c.is_whitespace() && *c != '\n')
            .collect::<String>();

        // Insert indented line above
        let text_to_insert = format!("{}\n", indent);
        editor
            .buffer_mut()
            .rope_mut()
            .insert(line_start, &text_to_insert);

        // Cursor stays at same line index, positioned at end of indentation
        editor.buffer_mut().cursor_mut().set_col(indent.len());
        Ok(())
    }

    pub(super) fn paste_after(editor: &mut Editor) -> Result<()> {
        let (text, reg_type) = editor.get_from_register_with_type();
        if text.is_empty() {
            return Ok(());
        }

        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        match reg_type {
            RegisterType::Block => {
                // Block paste - insert each line at the same column on consecutive lines
                let block_lines: Vec<&str> = text.split('\n').collect();
                let paste_col = col + 1; // Paste after cursor
                let mut last_pasted_line = line_idx;
                let mut last_pasted_text = "";

                for (i, block_line) in block_lines.iter().enumerate() {
                    let target_line = line_idx + i;
                    if target_line >= editor.buffer().line_count() {
                        break; // Don't create new lines for block paste
                    }

                    // Get current line and check if it's the final empty line (from trailing newline)
                    if let Some(line_text) = editor.buffer().line(target_line) {
                        let line_content = line_text.trim_end_matches('\n');

                        // Skip the final empty line (implicit from trailing newline)
                        if line_content.is_empty()
                            && target_line == editor.buffer().line_count() - 1
                        {
                            break;
                        }

                        let line_len = line_content.chars().count();

                        // Calculate insertion position
                        let insert_col = if paste_col > line_len {
                            // Pad the line with spaces if needed
                            let padding = " ".repeat(paste_col - line_len);
                            let padded_text = format!("{}{}", padding, block_line);
                            let change =
                                Change::insert((target_line, line_len), padded_text, cursor_before);
                            change.apply(editor.buffer_mut());
                            editor.add_change(change);
                            last_pasted_line = target_line;
                            last_pasted_text = block_line;
                            continue;
                        } else {
                            paste_col
                        };

                        // Insert the block line at the column
                        let change = Change::insert(
                            (target_line, insert_col),
                            block_line.to_string(),
                            cursor_before,
                        );
                        change.apply(editor.buffer_mut());
                        editor.add_change(change);
                        last_pasted_line = target_line;
                        last_pasted_text = block_line;
                    }
                }

                // Position cursor at the end of the last pasted block line
                let new_col = paste_col + last_pasted_text.chars().count();
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(last_pasted_line, new_col);
            }
            RegisterType::Line => {
                // Line paste - insert after current line
                let line_len = editor.buffer().rope().line(line_idx).len_chars();
                let position = (line_idx, line_len);
                let change = Change::insert(position, text, cursor_before);
                change.apply(editor.buffer_mut());
                editor.add_change(change);
            }
            RegisterType::Character => {
                // Character paste - insert after cursor
                let position = (line_idx, col + 1);
                let change = Change::insert(position, text, cursor_before);
                change.apply(editor.buffer_mut());
                editor.add_change(change);
            }
        }

        Ok(())
    }

    pub(super) fn paste_before(editor: &mut Editor) -> Result<()> {
        let text = editor.get_from_register();
        if text.is_empty() {
            return Ok(());
        }

        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        // Check if text contains newline (line paste vs character paste)
        let position = if text.contains('\n') {
            // Line paste - insert at end of previous line (or start if on first line)
            if line_idx > 0 {
                let prev_line_len = editor.buffer().rope().line(line_idx - 1).len_chars();
                (line_idx - 1, prev_line_len)
            } else {
                (0, 0)
            }
        } else {
            // Character paste - insert at cursor
            (line_idx, col)
        };

        // Create and apply the change
        let change = Change::insert(position, text, cursor_before);
        change.apply(editor.buffer_mut());
        editor.add_change(change);

        Ok(())
    }

    pub(super) fn delete_visual_selection(editor: &mut Editor) -> Result<()> {
        let mode = editor.mode();
        let cursor_before = (
            editor.buffer().cursor().line(),
            editor.buffer().cursor().col(),
        );

        if let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() {
            match mode {
                Mode::VisualLine => {
                    // Delete entire lines
                    let start_pos = (start_line, 0);
                    let end_pos = (end_line + 1, 0);

                    let deleted = editor
                        .buffer_mut()
                        .delete_range(start_line, 0, end_line + 1, 0);

                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);
                    editor.add_change(change);
                    editor.delete_to_register(deleted);

                    // Position cursor at start of selection
                    let new_line = start_line.min(editor.buffer().line_count().saturating_sub(1));
                    editor.buffer_mut().cursor_mut().set_position(new_line, 0);
                }
                Mode::VisualBlock => {
                    // Delete rectangular block
                    let mut deleted_lines = Vec::new();
                    let mut changes = Vec::new();

                    // Delete from bottom to top to avoid line number shifting
                    for line_idx in (start_line..=end_line).rev() {
                        if let Some(line_text) = editor.buffer().line(line_idx) {
                            let line_len = line_text.trim_end_matches('\n').chars().count();
                            // Only delete if the line is long enough
                            if start_col < line_len {
                                let actual_end_col = (end_col + 1).min(line_len);
                                let deleted = editor.buffer_mut().delete_range(
                                    line_idx,
                                    start_col,
                                    line_idx,
                                    actual_end_col,
                                );

                                // Create individual Change for each line deletion
                                let range =
                                    Range::new((line_idx, start_col), (line_idx, actual_end_col));
                                let change = Change::delete(range, deleted.clone(), cursor_before);
                                changes.push(change);
                                deleted_lines.push(deleted);
                            } else {
                                deleted_lines.push(String::new());
                            }
                        }
                    }

                    // Reverse to get original order
                    deleted_lines.reverse();
                    changes.reverse();
                    let deleted = deleted_lines.join("\n");

                    // Record as composite change for proper undo
                    let composite = Change::composite(changes, cursor_before, cursor_before);
                    editor.add_change(composite);
                    editor.delete_to_register(deleted);

                    // Position cursor at start of block, clamped to line length
                    let line_len = if let Some(line) = editor.buffer().line(start_line) {
                        line.trim_end_matches('\n').chars().count()
                    } else {
                        0
                    };
                    let clamped_col = if line_len > 0 {
                        start_col.min(line_len - 1)
                    } else {
                        0
                    };
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, clamped_col);
                }
                _ => {
                    // Character-wise visual mode
                    let start_pos = (start_line, start_col);
                    let end_pos = (end_line, end_col + 1);

                    let deleted = editor.buffer_mut().delete_range(
                        start_line,
                        start_col,
                        end_line,
                        end_col + 1,
                    );

                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);
                    editor.add_change(change);
                    editor.delete_to_register(deleted);

                    // Position cursor at start of selection
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, start_col);
                }
            }
        }

        Ok(())
    }

    pub(super) fn yank_visual_selection(editor: &mut Editor) -> Result<()> {
        let mode = editor.mode();

        if let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() {
            match mode {
                Mode::VisualLine => {
                    // Yank entire lines
                    let start_char = editor.buffer().rope().line_to_char(start_line);
                    let end_char = if end_line + 1 < editor.buffer().line_count() {
                        editor.buffer().rope().line_to_char(end_line + 1)
                    } else {
                        editor.buffer().rope().len_chars()
                    };

                    let yanked = editor
                        .buffer()
                        .rope()
                        .slice(start_char..end_char)
                        .to_string();
                    editor.yank_to_register_with_type(yanked, RegisterType::Line);
                }
                Mode::VisualBlock => {
                    // Yank rectangular block
                    let mut yanked_lines = Vec::new();

                    for line_idx in start_line..=end_line {
                        if let Some(line_text) = editor.buffer().line(line_idx) {
                            let line_len = line_text.trim_end_matches('\n').chars().count();
                            if start_col < line_len {
                                let actual_end_col = (end_col + 1).min(line_len);
                                let start_char =
                                    editor.buffer().rope().line_to_char(line_idx) + start_col;
                                let end_char =
                                    editor.buffer().rope().line_to_char(line_idx) + actual_end_col;
                                let yanked = editor
                                    .buffer()
                                    .rope()
                                    .slice(start_char..end_char)
                                    .to_string();
                                yanked_lines.push(yanked);
                            } else {
                                yanked_lines.push(String::new());
                            }
                        }
                    }

                    let yanked = yanked_lines.join("\n");
                    editor.yank_to_register_with_type(yanked, RegisterType::Block);
                }
                _ => {
                    // Character-wise visual mode
                    let start_char = editor.buffer().rope().line_to_char(start_line) + start_col;
                    let end_char = editor.buffer().rope().line_to_char(end_line) + end_col + 1;

                    let yanked = editor
                        .buffer()
                        .rope()
                        .slice(start_char..end_char)
                        .to_string();
                    editor.yank_to_register_with_type(yanked, RegisterType::Character);
                }
            }
        }

        Ok(())
    }

    pub(super) fn join_lines(editor: &mut Editor, count: usize) -> Result<()> {
        Operators::join_lines(editor.buffer_mut(), count)
    }

    pub(super) fn join_lines_no_space(editor: &mut Editor, count: usize) -> Result<()> {
        Operators::join_lines_no_space(editor.buffer_mut(), count)
    }

    pub(super) fn indent_lines_with_tracking(
        editor: &mut Editor,
        start_line: usize,
        end_line: usize,
        tab_width: usize,
        cursor_before: (usize, usize),
    ) -> Result<()> {
        for line_idx in start_line..end_line.min(editor.buffer().line_count()) {
            let indent_str = " ".repeat(tab_width);
            let change = Change::insert((line_idx, 0), indent_str.clone(), cursor_before);
            change.apply(editor.buffer_mut());
            editor.add_change(change);
        }
        Ok(())
    }

    pub(super) fn dedent_lines_with_tracking(
        editor: &mut Editor,
        start_line: usize,
        end_line: usize,
        tab_width: usize,
        cursor_before: (usize, usize),
    ) -> Result<()> {
        for line_idx in start_line..end_line.min(editor.buffer().line_count()) {
            if let Some(line) = editor.buffer().line(line_idx) {
                let line_text = line.trim_end_matches('\n');
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
                    let deleted =
                        editor
                            .buffer_mut()
                            .delete_range(line_idx, 0, line_idx, spaces_to_remove);
                    let range = Range::new((line_idx, 0), (line_idx, spaces_to_remove));
                    let change = Change::delete(range, deleted, cursor_before);
                    editor.add_change(change);
                }
            }
        }
        Ok(())
    }

    pub(super) fn toggle_case_at_cursor(editor: &mut Editor) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        if let Some(line) = editor.buffer().line(line_idx) {
            let line_text = line.trim_end_matches('\n');
            let chars: Vec<char> = line_text.chars().collect();

            if col < chars.len() {
                let ch = chars[col];
                let toggled = if ch.is_lowercase() {
                    ch.to_uppercase().to_string()
                } else {
                    ch.to_lowercase().to_string()
                };

                // Delete the character
                let start_pos = (line_idx, col);
                let end_pos = (line_idx, col + 1);
                let deleted = editor
                    .buffer_mut()
                    .delete_range(line_idx, col, line_idx, col + 1);
                let range = Range::new(start_pos, end_pos);
                let delete_change = Change::delete(range, deleted, cursor_before);

                // Insert the toggled character
                let insert_change = Change::insert((line_idx, col), toggled.clone(), cursor_before);
                insert_change.apply(editor.buffer_mut());

                editor.add_change(delete_change);
                editor.add_change(insert_change);

                // Move cursor right (Vim behavior)
                let new_col = col + toggled.chars().count();
                if new_col < chars.len() {
                    editor.buffer_mut().cursor_mut().set_col(new_col);
                }
            }
        }

        Ok(())
    }

    /// Changes case of entire line(s)
    pub(super) fn change_case_line(editor: &mut Editor, count: usize, case_change: CaseChange) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let start_line = cursor.line();
        let end_line = (start_line + count).min(editor.buffer().line_count());

        for line_idx in start_line..end_line {
            if let Some(line) = editor.buffer().line(line_idx) {
                let line_text = line.trim_end_matches('\n');
                let transformed = Self::apply_case_change(line_text, &case_change);

                if transformed != line_text {
                    let line_len = line_text.chars().count();
                    let deleted = editor
                        .buffer_mut()
                        .delete_range(line_idx, 0, line_idx, line_len);
                    let delete_range = Range::new((line_idx, 0), (line_idx, line_len));
                    let delete_change = Change::delete(delete_range, deleted, cursor_before);

                    let insert_change = Change::insert((line_idx, 0), transformed, cursor_before);
                    insert_change.apply(editor.buffer_mut());

                    editor.add_change(delete_change);
                    editor.add_change(insert_change);
                }
            }
        }

        Ok(())
    }

    /// Changes case using a motion
    pub(super) fn change_case_motion<F>(
        editor: &mut Editor,
        count: usize,
        case_change: CaseChange,
        motion: F,
    ) -> Result<()>
    where
        F: FnOnce(&mut Buffer, usize),
    {
        let start_cursor = editor.buffer().cursor().clone();
        let cursor_before = (start_cursor.line(), start_cursor.col());
        let start_line = start_cursor.line();
        let start_col = start_cursor.col();

        // Apply the motion to find the end position
        motion(editor.buffer_mut(), count);

        let end_cursor = editor.buffer().cursor();
        let end_line = end_cursor.line();
        let end_col = end_cursor.col();

        // Get the text in the range
        let start_char = editor.buffer().rope().line_to_char(start_line) + start_col;
        let end_char = editor.buffer().rope().line_to_char(end_line) + end_col;
        let text = editor
            .buffer()
            .rope()
            .slice(start_char..end_char)
            .to_string();

        // Transform the case
        let transformed = Self::apply_case_change(&text, &case_change);

        if transformed != text {
            // Delete the old text
            let deleted = editor
                .buffer_mut()
                .delete_range(start_line, start_col, end_line, end_col);
            let delete_range = Range::new((start_line, start_col), (end_line, end_col));
            let delete_change = Change::delete(delete_range, deleted, cursor_before);

            // Insert the transformed text
            let insert_change = Change::insert((start_line, start_col), transformed, cursor_before);
            insert_change.apply(editor.buffer_mut());

            editor.add_change(delete_change);
            editor.add_change(insert_change);
        }

        // Reset cursor to start position
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(start_line, start_col);

        Ok(())
    }

    /// Changes case from cursor to end of line
    pub(super) fn change_case_to_end_of_line(editor: &mut Editor, case_change: CaseChange) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        if let Some(line) = editor.buffer().line(line_idx) {
            let line_text = line.trim_end_matches('\n');
            let line_len = line_text.chars().count();

            if col < line_len {
                let text_to_end: String = line_text.chars().skip(col).collect();
                let transformed = Self::apply_case_change(&text_to_end, &case_change);

                if transformed != text_to_end {
                    let deleted = editor
                        .buffer_mut()
                        .delete_range(line_idx, col, line_idx, line_len);
                    let delete_range = Range::new((line_idx, col), (line_idx, line_len));
                    let delete_change = Change::delete(delete_range, deleted, cursor_before);

                    let insert_change = Change::insert((line_idx, col), transformed, cursor_before);
                    insert_change.apply(editor.buffer_mut());

                    editor.add_change(delete_change);
                    editor.add_change(insert_change);
                }
            }
        }

        Ok(())
    }

    /// Applies case change transformation to a string
    pub(super) fn apply_case_change(text: &str, case_change: &CaseChange) -> String {
        match case_change {
            CaseChange::Lowercase => text.to_lowercase(),
            CaseChange::Uppercase => text.to_uppercase(),
            CaseChange::Toggle => text
                .chars()
                .map(|ch| {
                    if ch.is_lowercase() {
                        ch.to_uppercase().to_string()
                    } else {
                        ch.to_lowercase().to_string()
                    }
                })
                .collect(),
        }
    }

    /// Increments the number under/after the cursor
    pub(super) fn increment_number(editor: &mut Editor, count: usize) -> Result<()> {
        Self::modify_number(editor, count as i64)
    }

    /// Decrements the number under/after the cursor
    pub(super) fn decrement_number(editor: &mut Editor, count: usize) -> Result<()> {
        Self::modify_number(editor, -(count as i64))
    }

    /// Sequential modify numbers in visual selection (g Ctrl-A / g Ctrl-X)
    /// delta: 1 for increment, -1 for decrement
    pub(super) fn sequential_modify_numbers(editor: &mut Editor, delta: i64) -> Result<()> {
        // Get visual selection range
        let selection = editor.visual_selection();
        if selection.is_none() {
            return Ok(());
        }

        let ((start_line, _), (end_line, _)) = selection.unwrap();
        let cursor_before = (start_line, editor.buffer().cursor().col());

        // Track all changes for composite undo
        let mut changes = Vec::new();

        // For each line in selection, find and modify number
        for line_idx in start_line..=end_line {
            let line_offset = (line_idx - start_line) as i64;
            let total_delta = delta * line_offset;

            if let Some(line) = editor.buffer().line(line_idx) {
                let line_text = line.trim_end_matches('\n');

                // Find number on this line (start from beginning)
                if let Some((start_col, end_col, number_str)) =
                    Self::find_number_at_or_after(line_text, 0)
                {
                    // Parse the number
                    if let Ok((value, base, prefix_len)) = Self::parse_number(&number_str) {
                        // Apply the sequential delta
                        let new_value = value.wrapping_add(total_delta);

                        // Format the new number
                        let mut new_number_str = Self::format_number(new_value, base, prefix_len);

                        // Preserve explicit '+' sign if original had it
                        let has_plus_sign = number_str.starts_with('+');
                        if has_plus_sign && new_value >= 0 && !new_number_str.starts_with('+') {
                            new_number_str = format!("+{}", new_number_str);
                        }

                        // Store the old text and range for undo
                        let old_text = number_str.clone();
                        let old_range = Range::new((line_idx, start_col), (line_idx, end_col));

                        // Delete and insert
                        let _deleted = editor
                            .buffer_mut()
                            .delete_range(line_idx, start_col, line_idx, end_col);
                        editor
                            .buffer_mut()
                            .insert_text_at(line_idx, start_col, &new_number_str);

                        // Create a NumberOperation for this line
                        let line_cursor_after = (line_idx, start_col + new_number_str.len() - 1);
                        let number_op = Change::number_operation(
                            total_delta,
                            cursor_before,
                            line_cursor_after,
                            old_text,
                            old_range,
                        );
                        changes.push(number_op);
                    }
                }
            }
        }

        // Position cursor back at start of selection
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(start_line, cursor_before.1);

        // Create a composite change for all the sequential modifications
        if !changes.is_empty() {
            let cursor_after = (start_line, cursor_before.1);
            let composite = Change::composite(changes, cursor_before, cursor_after);
            editor.add_change(composite);
        }

        Ok(())
    }

    /// Modifies (increments or decrements) the number under/after the cursor
    pub(super) fn modify_number(editor: &mut Editor, delta: i64) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        if let Some(line) = editor.buffer().line(line_idx) {
            let line_text = line.trim_end_matches('\n');

            // Find number at or after cursor position
            if let Some((start_col, end_col, number_str)) =
                Self::find_number_at_or_after(line_text, col)
            {
                // Check if number has explicit '+' sign
                let has_plus_sign = number_str.starts_with('+');

                // Parse the number with base detection
                let (value, base, prefix_len) = Self::parse_number(&number_str)?;

                // Apply the delta
                let new_value = value.wrapping_add(delta);

                // Format the new number with the same base
                let mut new_number_str = Self::format_number(new_value, base, prefix_len);

                // Preserve explicit '+' sign for positive numbers
                if has_plus_sign && new_value >= 0 && !new_number_str.starts_with('+') {
                    new_number_str = format!("+{}", new_number_str);
                }

                // Replace the number in the buffer
                // Store old text before deleting for undo
                let old_text = number_str.clone();
                let old_range = Range::new((line_idx, start_col), (line_idx, end_col));

                let _deleted = editor
                    .buffer_mut()
                    .delete_range(line_idx, start_col, line_idx, end_col);
                editor
                    .buffer_mut()
                    .insert_text_at(line_idx, start_col, &new_number_str);

                // Position cursor on the last digit of the modified number
                let new_end_col = start_col + new_number_str.len() - 1;
                editor.buffer_mut().cursor_mut().set_col(new_end_col);
                let cursor_after = (line_idx, new_end_col);

                // Create a NumberOperation change for proper dot-repeat behavior
                let number_op = Change::number_operation(
                    delta,
                    cursor_before,
                    cursor_after,
                    old_text,
                    old_range,
                );
                editor.add_change(number_op);
            }
        }

        Ok(())
    }

    /// Finds a number at or after the given column position
    /// Returns (start_col, end_col, number_string)
    pub(super) fn find_number_at_or_after(line: &str, col: usize) -> Option<(usize, usize, String)> {
        let chars: Vec<char> = line.chars().collect();

        if chars.is_empty() {
            return None;
        }

        // First, check if we're currently inside a number by searching backward
        let cursor_col = col.min(chars.len().saturating_sub(1));

        // If we're on a digit, search backward to find the start of the number
        if cursor_col < chars.len() && chars[cursor_col].is_ascii_digit() {
            let mut start_col = cursor_col;

            // Search backward to find the start of the number
            while start_col > 0 {
                let prev_ch = chars[start_col - 1];
                if prev_ch.is_ascii_digit() {
                    start_col -= 1;
                } else if prev_ch == '-' || prev_ch == '+' {
                    // Check if this sign is part of the number
                    if start_col > 1
                        && !chars[start_col - 2].is_whitespace()
                        && chars[start_col - 2] != '('
                        && chars[start_col - 2] != '['
                    {
                        // Not a sign, just adjacent character
                        break;
                    }
                    start_col -= 1;
                    break;
                } else if start_col >= 2 && prev_ch == 'x' && chars[start_col - 2] == '0' {
                    // Hex prefix
                    start_col -= 2;
                    break;
                } else if start_col >= 2
                    && (prev_ch == 'b' || prev_ch == 'o')
                    && chars[start_col - 2] == '0'
                {
                    // Binary or octal prefix
                    start_col -= 2;
                    break;
                } else {
                    break;
                }
            }

            // Now find the end of the number
            let mut end_col = cursor_col + 1;
            while end_col < chars.len() && chars[end_col].is_ascii_digit() {
                end_col += 1;
            }

            let number_str: String = chars[start_col..end_col].iter().collect();
            return Some((start_col, end_col, number_str));
        }

        // Not on a digit, so search backward first, then forward
        // This matches Vim behavior: search backward on current line, then forward

        // Try searching backward from cursor
        if cursor_col > 0 {
            let mut back_col = cursor_col;
            while back_col > 0 {
                back_col -= 1;
                if chars[back_col].is_ascii_digit() {
                    // Found a digit backward, now find the start and end of this number
                    let mut start_col = back_col;
                    while start_col > 0 {
                        let prev_ch = chars[start_col - 1];
                        if prev_ch.is_ascii_digit() {
                            start_col -= 1;
                        } else if prev_ch == '-' || prev_ch == '+' {
                            if start_col > 1
                                && !chars[start_col - 2].is_whitespace()
                                && chars[start_col - 2] != '('
                                && chars[start_col - 2] != '['
                            {
                                break;
                            }
                            start_col -= 1;
                            break;
                        } else if start_col >= 2 && prev_ch == 'x' && chars[start_col - 2] == '0' {
                            start_col -= 2;
                            break;
                        } else if start_col >= 2
                            && (prev_ch == 'b' || prev_ch == 'o')
                            && chars[start_col - 2] == '0'
                        {
                            start_col -= 2;
                            break;
                        } else {
                            break;
                        }
                    }

                    let mut end_col = back_col + 1;
                    while end_col < chars.len() && chars[end_col].is_ascii_digit() {
                        end_col += 1;
                    }

                    let number_str: String = chars[start_col..end_col].iter().collect();
                    return Some((start_col, end_col, number_str));
                }
            }
        }

        // No number found backward, search forward from cursor position
        let mut search_col = col;

        // Skip non-digit/non-hex characters to find start of number
        while search_col < chars.len() {
            let ch = chars[search_col];
            // Check if this could be the start of a number (including sign)
            if ch.is_ascii_digit()
                || ch == '-'
                || ch == '+'
                || (search_col + 1 < chars.len()
                    && ch == '0'
                    && (chars[search_col + 1] == 'x'
                        || chars[search_col + 1] == 'X'
                        || chars[search_col + 1] == 'b'
                        || chars[search_col + 1] == 'B'
                        || chars[search_col + 1] == 'o'
                        || chars[search_col + 1] == 'O'))
            {
                break;
            }
            search_col += 1;
        }

        if search_col >= chars.len() {
            return None;
        }

        let mut start_col = search_col;

        // Check if we're on a sign, and if so, verify there's a digit after it
        if chars[start_col] == '-' || chars[start_col] == '+' {
            if start_col + 1 < chars.len() && chars[start_col + 1].is_ascii_digit() {
                // Keep the sign as part of the number
            } else {
                // Not a number, just a sign
                start_col += 1;
                if start_col >= chars.len() {
                    return None;
                }
            }
        }
        let mut end_col = start_col;

        // Check for hex (0x), binary (0b), or octal (0o) prefix
        if chars[end_col] == '0' && end_col + 1 < chars.len() {
            let next = chars[end_col + 1];
            if next == 'x'
                || next == 'X'
                || next == 'b'
                || next == 'B'
                || next == 'o'
                || next == 'O'
            {
                end_col += 2;

                // Collect hex/binary/octal digits
                let is_hex = next == 'x' || next == 'X';
                let is_binary = next == 'b' || next == 'B';

                while end_col < chars.len() {
                    let ch = chars[end_col];
                    if is_hex && ch.is_ascii_hexdigit() {
                        end_col += 1;
                    } else if is_binary && (ch == '0' || ch == '1') {
                        end_col += 1;
                    } else if !is_hex && !is_binary && ch.is_ascii_digit() {
                        end_col += 1;
                    } else {
                        break;
                    }
                }

                if end_col > start_col + 2 {
                    let number_str: String = chars[start_col..end_col].iter().collect();
                    return Some((start_col, end_col, number_str));
                }
            }
        }

        // Regular decimal number (may have sign)
        end_col = start_col;

        // Skip optional sign
        if end_col < chars.len() && (chars[end_col] == '-' || chars[end_col] == '+') {
            end_col += 1;
        }

        // Collect digits
        while end_col < chars.len() && chars[end_col].is_ascii_digit() {
            end_col += 1;
        }

        if end_col > start_col {
            let number_str: String = chars[start_col..end_col].iter().collect();
            Some((start_col, end_col, number_str))
        } else {
            None
        }
    }

    /// Parses a number string, detecting the base from prefix
    /// Returns (value, base, prefix_length)
    pub(super) fn parse_number(s: &str) -> Result<(i64, u32, usize)> {
        if s.len() >= 3 {
            let prefix = &s[0..2];
            let digits = &s[2..];

            match prefix {
                "0x" | "0X" => {
                    let value = i64::from_str_radix(digits, 16).unwrap_or(0);
                    return Ok((value, 16, 2));
                }
                "0b" | "0B" => {
                    let value = i64::from_str_radix(digits, 2).unwrap_or(0);
                    return Ok((value, 2, 2));
                }
                "0o" | "0O" => {
                    let value = i64::from_str_radix(digits, 8).unwrap_or(0);
                    return Ok((value, 8, 2));
                }
                _ => {}
            }
        }

        // Regular decimal
        let value = s.parse::<i64>().unwrap_or(0);
        Ok((value, 10, 0))
    }

    /// Formats a number with the given base
    pub(super) fn format_number(value: i64, base: u32, prefix_len: usize) -> String {
        match base {
            16 => {
                if prefix_len > 0 {
                    format!("0x{:x}", value)
                } else {
                    format!("{:x}", value)
                }
            }
            2 => {
                if prefix_len > 0 {
                    format!("0b{:b}", value)
                } else {
                    format!("{:b}", value)
                }
            }
            8 => {
                if prefix_len > 0 {
                    format!("0o{:o}", value)
                } else {
                    format!("{:o}", value)
                }
            }
            _ => format!("{}", value),
        }
    }

    /// Clamps cursor to valid buffer bounds (line and column)
    pub(super) fn clamp_cursor_to_buffer(editor: &mut Editor) {
        // First, clamp line to valid range
        let line_count = editor.buffer().line_count();
        if line_count == 0 {
            // Empty buffer, set to 0,0
            editor.buffer_mut().cursor_mut().set_position(0, 0);
            return;
        }

        let cursor_line = editor.buffer().cursor().line();
        let mut clamped_line = cursor_line.min(line_count.saturating_sub(1));

        // If the last line is empty (just a newline), don't allow cursor on it
        // This matches Neovim behavior
        if clamped_line == line_count.saturating_sub(1) {
            if let Some(last_line) = editor.buffer().line(clamped_line) {
                if last_line == "\n" || last_line.is_empty() {
                    // Last line is empty, move cursor to previous line
                    if clamped_line > 0 {
                        clamped_line = clamped_line.saturating_sub(1);
                    }
                }
            }
        }

        if cursor_line != clamped_line {
            editor.buffer_mut().cursor_mut().set_line(clamped_line);
        }

        // Then, clamp column to valid range for the line
        let current_line = editor.buffer().cursor().line();
        if let Some(line) = editor.buffer().line(current_line) {
            let line_text = line.trim_end_matches('\n');
            let line_len = line_text.chars().count();
            let cursor_col = editor.buffer().cursor().col();

            if line_len == 0 {
                // Empty line, set to column 0
                if cursor_col != 0 {
                    editor.buffer_mut().cursor_mut().set_col(0);
                }
            } else if cursor_col >= line_len {
                // Past end of line, clamp to last character
                editor.buffer_mut().cursor_mut().set_col(line_len - 1);
            }
        }
    }

    /// Handles input in HoverWindow mode
    pub(super) fn handle_hover_window_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            // Esc or q - close hover window
            KeyCode::Esc | KeyCode::Char('q') => {
                editor.clear_hover();
                editor.set_mode(Mode::Normal);
            }
            // j or Down - scroll down
            KeyCode::Char('j') | KeyCode::Down => {
                editor.scroll_hover_down(1);
            }
            // k or Up - scroll up
            KeyCode::Char('k') | KeyCode::Up => {
                editor.scroll_hover_up(1);
            }
            // Ctrl-D - scroll down half page
            KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.scroll_hover_down(10);
            }
            // Ctrl-U - scroll up half page
            KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.scroll_hover_up(10);
            }
            // Ctrl-F or PageDown - scroll down full page
            KeyCode::Char('f') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.scroll_hover_down(20);
            }
            KeyCode::PageDown => {
                editor.scroll_hover_down(20);
            }
            // Ctrl-B or PageUp - scroll up full page
            KeyCode::Char('b') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.scroll_hover_up(20);
            }
            KeyCode::PageUp => {
                editor.scroll_hover_up(20);
            }
            // g - go to top
            KeyCode::Char('g') => {
                editor.scroll_hover_up(usize::MAX); // Scroll to top
            }
            // G - go to bottom
            KeyCode::Char('G') => {
                editor.scroll_hover_down(usize::MAX); // Scroll to bottom
            }
            _ => {
                // Ignore other keys
            }
        }
        Ok(())
    }

    /// Handles input in FileTree mode
    pub(super) fn handle_filetree_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            // Esc or q - close file tree
            KeyCode::Esc | KeyCode::Char('q') => {
                editor.toggle_file_tree();
            }
            // j or Down - move selection down
            KeyCode::Char('j') | KeyCode::Down => {
                editor.file_tree_mut().select_next();
            }
            // k or Up - move selection up
            KeyCode::Char('k') | KeyCode::Up => {
                editor.file_tree_mut().select_previous();
            }
            // Enter or o - open file or toggle directory
            KeyCode::Enter | KeyCode::Char('o') => {
                editor.open_file_from_tree();
            }
            // x or h - collapse directory
            KeyCode::Char('x') | KeyCode::Char('h') => {
                // Only collapse if it's an expanded directory
                if let Some(node) = editor.file_tree().selected_node() {
                    if node.is_dir() && node.is_expanded() {
                        editor.file_tree_mut().toggle_selected();
                    }
                }
            }
            // l - expand directory or open file
            KeyCode::Char('l') => {
                editor.open_file_from_tree();
            }
            // r - refresh file tree
            KeyCode::Char('r') => {
                editor.file_tree_mut().refresh();
            }
            // Tab - switch focus to buffer
            KeyCode::Tab => {
                editor.set_mode(Mode::Normal);
            }
            _ => {
                // Ignore other keys
            }
        }
        Ok(())
    }

    /// Wrapper to call commands module's execute_command_string
    pub fn execute_command_string(editor: &mut Editor, command: &str) -> Result<()> {
        commands::execute_command_string(editor, command)
    }

    /// Wrapper to call commands module's handle_command_mode
    pub fn handle_command_mode_wrapper(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        commands::handle_command_mode(editor, key_event)
    }

    /// Wrapper to call commands module's parse_range
    pub fn parse_range_wrapper(editor: &Editor, range_str: &str) -> Option<(usize, usize)> {
        commands::parse_range(editor, range_str)
    }
}
