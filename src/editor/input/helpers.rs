//! Helper functions for cursor movement and editing
//!
//! These functions are used by various input handlers.

use crate::editor::{Change, Editor, Range, RegisterType};
use crate::mode::Mode;
use crate::unicode::grapheme_count;
use anyhow::Result;

// Helper methods for cursor movement and editing

pub fn move_left(editor: &mut Editor) {
    let count = editor.effective_count();
    let cursor = editor.buffer_mut().cursor_mut();
    if cursor.col() >= count {
        cursor.move_left(count);
    } else {
        cursor.set_col(0);
    }
    editor.clear_count();
}

pub fn move_right(editor: &mut Editor) {
    let count = editor.effective_count();
    let line_idx = editor.buffer().cursor().line();
    let mode = editor.mode();
    if let Some(line) = editor.buffer().line(line_idx) {
        let line_len = grapheme_count(line.trim_end_matches('\n'));
        let cursor = editor.buffer_mut().cursor_mut();

        // In VisualBlock mode, allow cursor beyond line end for rectangular selection
        // In Insert mode, allow cursor one past end (for appending)
        let max_col = if mode == Mode::VisualBlock {
            usize::MAX // No limit in visual block
        } else if mode == Mode::Insert {
            line_len // Can be at position after last char
        } else {
            line_len.saturating_sub(1) // Normal mode: on last char
        };

        let new_col = (cursor.col() + count).min(max_col);
        cursor.set_col(new_col);
    }
    editor.clear_count();
}

pub fn move_up(editor: &mut Editor) {
    let count = editor.effective_count();
    let cursor = editor.buffer_mut().cursor_mut();
    cursor.move_up(count);
    clamp_cursor_with_goal_column(editor);
    editor.clear_count();
}

pub fn move_down(editor: &mut Editor) {
    let count = editor.effective_count();
    let max_line = editor.buffer().line_count().saturating_sub(1);

    let cursor = editor.buffer_mut().cursor_mut();
    let new_line = (cursor.line() + count).min(max_line);
    cursor.set_line(new_line);
    clamp_cursor_with_goal_column(editor);
    editor.clear_count();
}

pub fn clamp_cursor_to_line(editor: &mut Editor) {
    let line_idx = editor.buffer().cursor().line();
    if let Some(line) = editor.buffer().line(line_idx) {
        let line_len = grapheme_count(line.trim_end_matches('\n'));
        let cursor = editor.buffer_mut().cursor_mut();
        if cursor.col() >= line_len {
            let new_col = if line_len > 0 { line_len - 1 } else { 0 };
            cursor.set_col(new_col);
        }
    }
}

pub fn clamp_cursor_with_goal_column(editor: &mut Editor) {
    let line_idx = editor.buffer().cursor().line();
    let mode = editor.mode();
    if let Some(line) = editor.buffer().line(line_idx) {
        let line_len = grapheme_count(line.trim_end_matches('\n'));
        let max_col = if line_len > 0 { line_len - 1 } else { 0 };
        let cursor = editor.buffer_mut().cursor_mut();
        let desired = cursor.desired_col();

        // In VisualBlock mode, preserve desired column even if beyond line end
        let target_col = if mode == Mode::VisualBlock {
            desired
        } else if desired == usize::MAX {
            // usize::MAX is a sentinel value meaning "always end of line"
            max_col
        } else {
            desired.min(max_col)
        };

        cursor.set_col_preserve_desired(target_col);
    }
}

pub fn insert_char(editor: &mut Editor, c: char) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let position = (cursor.line(), cursor.col());

    // Create and apply the change
    let change = Change::insert(position, c.to_string(), cursor_before);
    change.apply(editor.buffer_mut());
    editor.add_change(change);

    Ok(())
}

pub fn insert_newline(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let position = (line_idx, cursor.col());

    // Get indentation from current line
    let line_text = editor.buffer().line(line_idx).unwrap_or_default();
    let indent: String = line_text
        .chars()
        .take_while(|c| c.is_whitespace() && *c != '\n')
        .collect();

    // Insert newline + indentation
    let text_to_insert = format!("\n{}", indent);
    let change = Change::insert(position, text_to_insert, cursor_before);
    change.apply(editor.buffer_mut());
    editor.add_change(change);

    Ok(())
}

pub fn delete_char_before_cursor(editor: &mut Editor) -> Result<()> {
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

pub fn delete_word_backward_insert(editor: &mut Editor) -> Result<()> {
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

    // Skip trailing whitespace (Vim deletes whitespace + preceding word)
    while start_col > 0 && chars.get(start_col - 1).is_some_and(|c| c.is_whitespace()) {
        start_col -= 1;
    }

    // Then delete the preceding word or punctuation run
    if start_col > 0 {
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

        if let Some(&ch) = chars.get(start_col - 1) {
            if is_word_char(ch) {
                while start_col > 0 && chars.get(start_col - 1).is_some_and(|&c| is_word_char(c)) {
                    start_col -= 1;
                }
            } else {
                while start_col > 0
                    && chars
                        .get(start_col - 1)
                        .is_some_and(|&c| !is_word_char(c) && !c.is_whitespace())
                {
                    start_col -= 1;
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
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line_idx, start_col);
    }

    Ok(())
}

pub fn delete_to_line_start_insert(editor: &mut Editor) -> Result<()> {
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

pub fn indent_line_insert(editor: &mut Editor) -> Result<()> {
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

pub fn dedent_line_insert(editor: &mut Editor) -> Result<()> {
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
    let mut chars_to_remove = 0;

    for &ch in chars.iter().take(tab_width) {
        if ch == ' ' {
            chars_to_remove += 1;
        } else if ch == '\t' {
            chars_to_remove += 1;
            break;
        } else {
            break;
        }
    }

    // If no leading whitespace, do nothing
    if chars_to_remove == 0 {
        return Ok(());
    }

    // Delete the leading whitespace
    let deleted = editor
        .buffer_mut()
        .delete_range(line_idx, 0, line_idx, chars_to_remove);

    // Update cursor position - move column left by chars_to_remove
    let new_col = col.saturating_sub(chars_to_remove);
    editor.buffer_mut().cursor_mut().set_col(new_col);

    // Create change for undo
    let range = Range::new((line_idx, 0), (line_idx, chars_to_remove));
    let change = Change::delete(range, deleted, cursor_before);
    editor.add_change(change);

    Ok(())
}

pub fn insert_line_below(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();

    // Get indentation from current line
    let line_text = editor.buffer().line(line_idx).unwrap_or_default();
    let indent: String = line_text
        .chars()
        .take_while(|c| c.is_whitespace() && *c != '\n')
        .collect();

    // Determine insert position and text
    let (insert_position, text_to_insert) = if line_text.ends_with('\n') {
        // Line ends with newline, insert at start of next line
        ((line_idx + 1, 0), format!("{}\n", indent))
    } else {
        // Last line without newline, insert at end of current line
        let line_len = line_text.chars().count();
        ((line_idx, line_len), format!("\n{}\n", indent))
    };

    // Create and apply the change (this records it for undo)
    let change = Change::insert(insert_position, text_to_insert, cursor_before);
    change.apply(editor.buffer_mut());
    editor.add_change(change);

    // Position cursor at end of indentation on new line
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(line_idx + 1, indent.len());
    Ok(())
}

pub fn insert_line_above(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let line_idx = cursor.line();

    // Get indentation from current line
    let line_text = editor.buffer().line(line_idx).unwrap_or_default();
    let indent: String = line_text
        .chars()
        .take_while(|c| c.is_whitespace() && *c != '\n')
        .collect();

    // Insert indented line above current line
    let text_to_insert = format!("{}\n", indent);
    let insert_position = (line_idx, 0);

    // Create and apply the change (this records it for undo)
    let change = Change::insert(insert_position, text_to_insert, cursor_before);
    change.apply(editor.buffer_mut());
    editor.add_change(change);

    // Position cursor at end of indentation on the new line (which is still at line_idx
    // because we inserted above, pushing everything down)
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(line_idx, indent.len());
    Ok(())
}

pub fn paste_after(editor: &mut Editor) -> Result<()> {
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
                    if line_content.is_empty() && target_line == editor.buffer().line_count() - 1 {
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

            // Vim: cursor on first non-blank of the new line
            let new_line = line_idx + 1;
            let first_non_blank = editor
                .buffer()
                .line(new_line)
                .map(|l| {
                    l.chars()
                        .take_while(|ch| ch.is_whitespace() && *ch != '\n')
                        .count()
                })
                .unwrap_or(0);
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(new_line, first_non_blank);
        }
        RegisterType::Character => {
            // Character paste - insert after cursor
            let position = (line_idx, col + 1);
            let change = Change::insert(position, text, cursor_before);
            change.apply(editor.buffer_mut());
            editor.add_change(change);
            // change.apply() sets cursor one-past-end via calculate_end_position.
            // Vim places cursor on the last character of pasted text.
            // If cursor col > 0, move back by 1 to land on last char.
            let cur = editor.buffer().cursor();
            let cur_col = cur.col();
            let cur_line = cur.line();
            if cur_col > 0 {
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(cur_line, cur_col - 1);
            }
            // If col == 0 (text ended with \n), cursor is already at correct position
        }
    }

    Ok(())
}

pub fn paste_before(editor: &mut Editor) -> Result<()> {
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
            let block_lines: Vec<&str> = text.split('\n').collect();
            let paste_col = col;
            let mut last_pasted_line = line_idx;
            let mut last_pasted_text = "";

            for (i, block_line) in block_lines.iter().enumerate() {
                let target_line = line_idx + i;
                if target_line >= editor.buffer().line_count() {
                    break;
                }

                if let Some(line_text) = editor.buffer().line(target_line) {
                    let line_content = line_text.trim_end_matches('\n');
                    if line_content.is_empty() && target_line == editor.buffer().line_count() - 1 {
                        break;
                    }

                    let line_len = line_content.chars().count();
                    let insert_col = if paste_col > line_len {
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

            let new_col = paste_col + last_pasted_text.chars().count();
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(last_pasted_line, new_col);
        }
        RegisterType::Line => {
            // Line paste before - insert at end of previous line (newline splits correctly)
            // For first line, insert at (0, 0) as there's no previous line
            let position = if line_idx > 0 {
                let prev_line_len = editor.buffer().rope().line(line_idx - 1).len_chars();
                (line_idx - 1, prev_line_len)
            } else {
                (0, 0)
            };
            let change = Change::insert(position, text, cursor_before);
            change.apply(editor.buffer_mut());
            editor.add_change(change);

            // Vim: cursor on first non-blank of the pasted line
            let pasted_line = line_idx; // Text was inserted before current line
            let first_non_blank = editor
                .buffer()
                .line(pasted_line)
                .map(|l| {
                    l.chars()
                        .take_while(|ch| ch.is_whitespace() && *ch != '\n')
                        .count()
                })
                .unwrap_or(0);
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(pasted_line, first_non_blank);
        }
        RegisterType::Character => {
            // Character paste before cursor
            let position = (line_idx, col);
            let change = Change::insert(position, text, cursor_before);
            change.apply(editor.buffer_mut());
            editor.add_change(change);
        }
    }

    Ok(())
}

pub fn delete_visual_selection(editor: &mut Editor) -> Result<()> {
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
                editor.delete_to_register_with_type(deleted, RegisterType::Line);

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
                editor.delete_to_register_with_type(deleted, RegisterType::Block);

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

                let deleted =
                    editor
                        .buffer_mut()
                        .delete_range(start_line, start_col, end_line, end_col + 1);

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

pub fn yank_visual_selection(editor: &mut Editor) -> Result<()> {
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

pub fn join_lines(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let start_line = cursor.line();

    // Capture the old text for undo (lines that will be affected)
    let lines_to_join = count.max(1);
    let end_line = (start_line + lines_to_join).min(editor.buffer().line_count());
    let mut old_text = String::new();
    for line_idx in start_line..end_line {
        if let Some(line) = editor.buffer().line(line_idx) {
            old_text.push_str(&line);
        }
    }
    let old_range = Range::new(cursor_before, (end_line.saturating_sub(1), 0));

    // Perform the join operation
    editor.buffer_mut().join_lines(count)?;

    // Track the change for dot-repeat and undo
    let cursor_after = (
        editor.buffer().cursor().line(),
        editor.buffer().cursor().col(),
    );
    let change = Change::join_lines(
        count,
        true,
        cursor_before,
        cursor_after,
        old_text,
        old_range,
    );
    editor.add_change(change);

    Ok(())
}

pub fn join_lines_no_space(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let start_line = cursor.line();

    // Capture the old text for undo (lines that will be affected)
    let lines_to_join = count.max(1);
    let end_line = (start_line + lines_to_join).min(editor.buffer().line_count());
    let mut old_text = String::new();
    for line_idx in start_line..end_line {
        if let Some(line) = editor.buffer().line(line_idx) {
            old_text.push_str(&line);
        }
    }
    let old_range = Range::new(cursor_before, (end_line.saturating_sub(1), 0));

    // Perform the join operation
    editor.buffer_mut().join_lines_no_space(count)?;

    // Track the change for dot-repeat and undo
    let cursor_after = (
        editor.buffer().cursor().line(),
        editor.buffer().cursor().col(),
    );
    let change = Change::join_lines(
        count,
        false,
        cursor_before,
        cursor_after,
        old_text,
        old_range,
    );
    editor.add_change(change);

    Ok(())
}

pub fn indent_lines_with_tracking(
    editor: &mut Editor,
    start_line: usize,
    end_line: usize,
    tab_width: usize,
    cursor_before: (usize, usize),
) -> Result<()> {
    let mut modified = false;
    for line_idx in start_line..end_line.min(editor.buffer().line_count()) {
        let indent_str = " ".repeat(tab_width);
        let change = Change::insert((line_idx, 0), indent_str.clone(), cursor_before);
        change.apply(editor.buffer_mut());
        editor.add_change(change);
        modified = true;
    }
    if modified {
        editor.mark_buffer_modified();
    }
    Ok(())
}

pub fn dedent_lines_with_tracking(
    editor: &mut Editor,
    start_line: usize,
    end_line: usize,
    tab_width: usize,
    cursor_before: (usize, usize),
) -> Result<()> {
    let mut modified = false;
    for line_idx in start_line..end_line.min(editor.buffer().line_count()) {
        if let Some(line) = editor.buffer().line(line_idx) {
            let line_text = line.trim_end_matches('\n');
            let chars: Vec<char> = line_text.chars().collect();
            let mut chars_to_remove = 0;

            for &ch in chars.iter().take(tab_width) {
                if ch == ' ' {
                    chars_to_remove += 1;
                } else if ch == '\t' {
                    // A tab counts as one character to delete, completing
                    // the removal of one indent level
                    chars_to_remove += 1;
                    break;
                } else {
                    break;
                }
            }

            if chars_to_remove > 0 {
                let deleted =
                    editor
                        .buffer_mut()
                        .delete_range(line_idx, 0, line_idx, chars_to_remove);
                let range = Range::new((line_idx, 0), (line_idx, chars_to_remove));
                let change = Change::delete(range, deleted, cursor_before);
                editor.add_change(change);
                modified = true;
            }
        }
    }
    if modified {
        editor.mark_buffer_modified();
    }
    Ok(())
}

/// Clamps cursor to valid buffer bounds (line and column)
pub fn clamp_cursor_to_buffer(editor: &mut Editor) {
    // First, clamp line to valid range
    let line_count = editor.buffer().line_count();
    if line_count == 0 {
        // Empty buffer, set to 0,0
        editor.buffer_mut().cursor_mut().set_position(0, 0);
        return;
    }

    let cursor_line = editor.buffer().cursor().line();
    let clamped_line = cursor_line.min(line_count.saturating_sub(1));

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

/// Exit visual mode and save the selection for gv command
/// This should be called whenever exiting visual mode to ensure the selection is saved
pub fn exit_visual_mode_to_normal(editor: &mut Editor) {
    editor.save_last_visual_selection();
    editor.clear_visual_start();
    editor.set_mode(Mode::Normal);
}

/// Save visual selection and clear visual state (without changing mode)
/// Use this when transitioning to insert mode or other modes after visual operations
pub fn save_and_clear_visual(editor: &mut Editor) {
    editor.save_last_visual_selection();
    editor.clear_visual_start();
}

/// Transform visual selection text using the given function (shared by uppercase/lowercase)
fn transform_visual_selection(editor: &mut Editor, transform: fn(&str) -> String) -> Result<()> {
    let mode = editor.mode();
    let cursor_before = (
        editor.buffer().cursor().line(),
        editor.buffer().cursor().col(),
    );

    let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() else {
        return Ok(());
    };

    match mode {
        Mode::VisualLine => {
            for line_idx in start_line..=end_line {
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_text = line.trim_end_matches('\n');
                    let transformed = transform(line_text);
                    let char_count = line_text.chars().count();

                    editor
                        .buffer_mut()
                        .delete_range(line_idx, 0, line_idx, char_count);
                    editor
                        .buffer_mut()
                        .insert_text_at(line_idx, 0, &transformed);

                    let change = Change::composite(
                        vec![
                            Change::delete(
                                Range::new((line_idx, 0), (line_idx, char_count)),
                                line_text.to_string(),
                                cursor_before,
                            ),
                            Change::insert((line_idx, 0), transformed, cursor_before),
                        ],
                        cursor_before,
                        cursor_before,
                    );
                    editor.add_change(change);
                }
            }
        }
        Mode::VisualBlock => {
            for line_idx in start_line..=end_line {
                if let Some(line) = editor.buffer().line(line_idx) {
                    let chars_len = line.trim_end_matches('\n').chars().count();
                    let line_start = start_col.min(chars_len);
                    let line_end = (end_col + 1).min(chars_len);

                    if line_start < line_end {
                        let deleted = editor
                            .buffer_mut()
                            .delete_range(line_idx, line_start, line_idx, line_end);
                        let transformed = transform(&deleted);
                        editor
                            .buffer_mut()
                            .insert_text_at(line_idx, line_start, &transformed);

                        let change = Change::composite(
                            vec![
                                Change::delete(
                                    Range::new((line_idx, line_start), (line_idx, line_end)),
                                    deleted,
                                    cursor_before,
                                ),
                                Change::insert((line_idx, line_start), transformed, cursor_before),
                            ],
                            cursor_before,
                            cursor_before,
                        );
                        editor.add_change(change);
                    }
                }
            }
        }
        _ => {
            // Character-wise visual mode
            let deleted =
                editor
                    .buffer_mut()
                    .delete_range(start_line, start_col, end_line, end_col + 1);
            let transformed = transform(&deleted);
            editor
                .buffer_mut()
                .insert_text_at(start_line, start_col, &transformed);

            let change = Change::composite(
                vec![
                    Change::delete(
                        Range::new((start_line, start_col), (end_line, end_col + 1)),
                        deleted,
                        cursor_before,
                    ),
                    Change::insert((start_line, start_col), transformed, cursor_before),
                ],
                cursor_before,
                cursor_before,
            );
            editor.add_change(change);
        }
    }

    Ok(())
}

/// Convert visual selection to uppercase
pub fn uppercase_visual_selection(editor: &mut Editor) -> Result<()> {
    transform_visual_selection(editor, |s| s.to_uppercase())
}

/// Convert visual selection to lowercase
pub fn lowercase_visual_selection(editor: &mut Editor) -> Result<()> {
    transform_visual_selection(editor, |s| s.to_lowercase())
}

/// Extracts the word under the cursor
/// A "word" consists of alphanumeric characters and underscores
/// Returns None if cursor is not on a word character
fn extract_word_at_cursor(editor: &Editor) -> Option<String> {
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let col = cursor.col();

    let line = editor.buffer().line(line_idx)?;
    let line_text = line.trim_end_matches('\n');
    let chars: Vec<char> = line_text.chars().collect();

    if col >= chars.len() {
        return None;
    }

    // Extract word under cursor
    let is_word_char = |c: char| c.is_alphanumeric() || c == '_';
    let start = chars[..=col]
        .iter()
        .rposition(|&c| !is_word_char(c))
        .map(|i| i + 1)
        .unwrap_or(0);
    let end = chars[col..]
        .iter()
        .position(|&c| !is_word_char(c))
        .map(|i| col + i)
        .unwrap_or(chars.len());

    if start < end {
        Some(chars[start..end].iter().collect())
    } else {
        None
    }
}

/// Sets up and executes a search for the given text
/// Returns true if a match was found, false otherwise
fn setup_and_execute_search(editor: &mut Editor, text: &str, forward: bool) -> bool {
    // Escape regex special characters for literal search
    let escaped = regex::escape(text);

    // Create and execute the search
    editor.clear_search_buffer();
    for ch in escaped.chars() {
        editor.append_to_search_buffer(ch);
    }
    editor.set_search_forward(forward);

    // Update the / register with the search pattern
    editor.registers.set_last_search(escaped.clone());

    // Create search and find first match
    let mut search = crate::editor::Search::new_with_options(
        escaped,
        forward,
        editor.options.ignorecase,
        editor.options.smartcase,
    );

    // For visual * and #, we want to find the NEXT occurrence, not the current one
    // So start searching from the next column position (forward) or current position (backward)
    let cursor = editor.buffer().cursor();
    let search_col = if forward {
        cursor.col() + 1
    } else {
        cursor.col()
    };

    if let Some((line, col, _)) = search.find_next(editor.buffer(), cursor.line(), search_col) {
        editor.buffer_mut().cursor_mut().set_position(line, col);
        editor.set_current_search(search);
        true
    } else {
        false
    }
}

/// Gets the text content of the current visual selection
/// Returns the selected text as a String, or None if no selection exists
/// Handles Visual, VisualLine, and VisualBlock modes appropriately
pub fn get_visual_selection_text(editor: &Editor) -> Option<String> {
    let mode = editor.mode();
    let ((start_line, start_col), (end_line, end_col)) = editor.visual_selection()?;

    match mode {
        Mode::Visual => {
            // Character-wise selection
            let start_char = editor.buffer().rope().line_to_char(start_line) + start_col;
            let end_char = editor.buffer().rope().line_to_char(end_line) + end_col + 1;
            Some(
                editor
                    .buffer()
                    .rope()
                    .slice(start_char..end_char)
                    .to_string(),
            )
        }
        Mode::VisualLine => {
            // Line-wise selection (include entire lines)
            let mut text = String::new();
            for line_idx in start_line..=end_line {
                if let Some(line) = editor.buffer().line(line_idx) {
                    text.push_str(line.trim_end_matches('\n'));
                    if line_idx < end_line {
                        text.push('\n');
                    }
                }
            }
            Some(text)
        }
        Mode::VisualBlock => {
            // Rectangular block selection
            let mut lines = Vec::new();
            for line_idx in start_line..=end_line {
                if let Some(line_text) = editor.buffer().line(line_idx) {
                    let chars: Vec<char> = line_text.trim_end_matches('\n').chars().collect();
                    let line_start = start_col.min(chars.len());
                    let line_end = (end_col + 1).min(chars.len());

                    if line_start < line_end {
                        let block_text: String = chars[line_start..line_end].iter().collect();
                        lines.push(block_text);
                    } else {
                        // Line is too short for block selection
                        lines.push(String::new());
                    }
                }
            }
            // For block mode, join lines with newlines
            Some(lines.join("\n"))
        }
        _ => None,
    }
}

/// Searches forward for the visually selected text
/// Escapes regex special characters for literal search
/// Returns true if match found, false otherwise
#[must_use = "ignoring the return value means you won't know if the search succeeded"]
pub fn search_visual_selection_forward(editor: &mut Editor) -> bool {
    let selection_text = match get_visual_selection_text(editor) {
        Some(text) if !text.is_empty() => text,
        _ => {
            // Fall back to word under cursor if selection is empty
            match extract_word_at_cursor(editor) {
                Some(word) => word,
                None => return false,
            }
        }
    };

    setup_and_execute_search(editor, &selection_text, true)
}

/// Searches backward for the visually selected text
/// Escapes regex special characters for literal search
/// Returns true if match found, false otherwise
#[must_use = "ignoring the return value means you won't know if the search succeeded"]
pub fn search_visual_selection_backward(editor: &mut Editor) -> bool {
    let selection_text = match get_visual_selection_text(editor) {
        Some(text) if !text.is_empty() => text,
        _ => {
            // Fall back to word under cursor if selection is empty
            match extract_word_at_cursor(editor) {
                Some(word) => word,
                None => return false,
            }
        }
    };

    setup_and_execute_search(editor, &selection_text, false)
}

// ===================================================================
// Yank operations (moved from Operators struct for consolidation)
// ===================================================================

/// Yanks (copies) from current position to end of line
pub fn yank_to_end_of_line(buffer: &crate::buffer::Buffer) -> anyhow::Result<String> {
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
pub fn yank_line(buffer: &crate::buffer::Buffer, count: usize) -> anyhow::Result<String> {
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
pub fn yank_word(buffer: &mut crate::buffer::Buffer, count: usize) -> anyhow::Result<String> {
    let start_cursor = *buffer.cursor();
    let start_line = start_cursor.line();
    let start_col = start_cursor.col();
    let start_char = buffer.rope().line_to_char(start_line) + start_col;

    // Move cursor forward by word
    crate::editor::Motions::word_forward(buffer, count);

    let end_cursor = buffer.cursor();
    let mut end_line = end_cursor.line();
    let mut end_col = end_cursor.col();

    // When the motion didn't move (last word on last line), yank to end of line
    if end_line == start_line && end_col == start_col {
        if let Some(line) = buffer.line(end_line) {
            let line_len = line.trim_end_matches('\n').chars().count();
            if end_line + 1 >= buffer.line_count() {
                end_col = line_len;
            }
        }
    }

    let end_char = buffer.rope().line_to_char(end_line) + end_col;

    // Get yanked text
    let yanked = buffer.rope().slice(start_char..end_char).to_string();

    // Reset cursor to start position
    buffer.cursor_mut().set_position(start_line, start_col);

    Ok(yanked)
}

// ===================================================================
// Auto-indent (moved from Operators struct for consolidation)
// ===================================================================

/// Auto-indents lines based on bracket context (= operator)
/// Returns the number of lines auto-indented
pub fn auto_indent_lines(
    buffer: &mut crate::buffer::Buffer,
    start_line: usize,
    end_line: usize,
    tab_width: usize,
) -> anyhow::Result<usize> {
    let end_line = end_line.min(buffer.line_count());
    if start_line >= end_line {
        return Ok(0);
    }

    // Determine base indent from the line before start_line (or 0 if first line)
    let mut current_indent = if start_line > 0 {
        if let Some(prev_line) = buffer.line(start_line - 1) {
            let prev_text = prev_line.trim_end_matches('\n');
            count_leading_spaces(prev_text, tab_width)
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
            if trimmed.starts_with('}') || trimmed.starts_with(')') || trimmed.starts_with(']') {
                current_indent = current_indent.saturating_sub(tab_width);
            }

            // Calculate current leading spaces
            let current_spaces = count_leading_spaces(line_text, tab_width);

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
