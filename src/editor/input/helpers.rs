//! Helper functions for cursor movement and editing
//!
//! These functions are used by various input handlers.

// TODO: Grapheme cluster support needed throughout this file
// Currently using chars().count() which splits multi-codepoint emojis (e.g., 👨‍👩‍👧‍👦)
// into separate characters. Should use a grapheme cluster library for proper Unicode handling.

use crate::editor::{Change, Editor, Operators, Range, RegisterType};
use crate::mode::Mode;
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
        let line_len = line.trim_end_matches('\n').chars().count();
        let cursor = editor.buffer_mut().cursor_mut();

        // In VisualBlock mode, allow cursor beyond line end for rectangular selection
        let new_col = if mode == Mode::VisualBlock {
            cursor.col() + count
        } else {
            (cursor.col() + count).min(line_len.saturating_sub(1).max(0))
        };

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
    clamp_cursor_with_goal_column(editor);
    editor.clear_count();
}

pub fn clamp_cursor_to_line(editor: &mut Editor) {
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

pub fn clamp_cursor_with_goal_column(editor: &mut Editor) {
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
    let position = (cursor.line(), cursor.col());

    // Create and apply the change
    let change = Change::insert(position, "\n".to_string(), cursor_before);
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

    // Skip trailing whitespace
    while start_col > 0
        && chars
            .get(start_col - 1)
            .is_some_and(|c| c.is_whitespace())
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
                        && chars.get(start_col - 1).is_some_and(|&c| is_word_char(c))
                    {
                        start_col -= 1;
                    }
                } else {
                    // Delete punctuation/special characters
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

pub fn paste_before(editor: &mut Editor) -> Result<()> {
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
    Operators::join_lines(editor.buffer_mut(), count)?;

    // Track the change for dot-repeat and undo
    let cursor_after = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
    let change = Change::join_lines(count, true, cursor_before, cursor_after, old_text, old_range);
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
    Operators::join_lines_no_space(editor.buffer_mut(), count)?;

    // Track the change for dot-repeat and undo
    let cursor_after = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
    let change = Change::join_lines(count, false, cursor_before, cursor_after, old_text, old_range);
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
    for line_idx in start_line..end_line.min(editor.buffer().line_count()) {
        let indent_str = " ".repeat(tab_width);
        let change = Change::insert((line_idx, 0), indent_str.clone(), cursor_before);
        change.apply(editor.buffer_mut());
        editor.add_change(change);
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

/// Convert visual selection to uppercase
pub fn uppercase_visual_selection(editor: &mut Editor) -> Result<()> {
    let mode = editor.mode();
    let cursor_before = (
        editor.buffer().cursor().line(),
        editor.buffer().cursor().col(),
    );

    if let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() {
        match mode {
            Mode::VisualLine => {
                // Uppercase entire lines
                for line_idx in start_line..=end_line {
                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let uppercased = line_text.to_uppercase();

                        // Delete and replace
                        editor.buffer_mut().delete_range(line_idx, 0, line_idx, line_text.chars().count());
                        editor.buffer_mut().insert_text_at(line_idx, 0, &uppercased);

                        let delete_change = Change::delete(
                            Range::new((line_idx, 0), (line_idx, line_text.chars().count())),
                            line_text.to_string(),
                            cursor_before,
                        );
                        let insert_change = Change::insert((line_idx, 0), uppercased, cursor_before);
                        let change = Change::composite(
                            vec![delete_change, insert_change],
                            cursor_before,
                            cursor_before,
                        );
                        editor.add_change(change);
                    }
                }
            }
            Mode::VisualBlock => {
                // Convert to uppercase for visual block selection
                for line_idx in start_line..=end_line {
                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let chars: Vec<char> = line_text.chars().collect();

                        let line_start = start_col.min(chars.len());
                        let line_end = (end_col + 1).min(chars.len());

                        if line_start < line_end {
                            let deleted = editor
                                .buffer_mut()
                                .delete_range(line_idx, line_start, line_idx, line_end);
                            let uppercased = deleted.to_uppercase();
                            editor.buffer_mut().insert_text_at(
                                line_idx,
                                line_start,
                                &uppercased,
                            );

                            let delete_change = Change::delete(
                                Range::new((line_idx, line_start), (line_idx, line_end)),
                                deleted,
                                cursor_before,
                            );
                            let insert_change = Change::insert(
                                (line_idx, line_start),
                                uppercased,
                                cursor_before,
                            );
                            let change = Change::composite(
                                vec![delete_change, insert_change],
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
                let deleted = editor.buffer_mut().delete_range(
                    start_line,
                    start_col,
                    end_line,
                    end_col + 1,
                );
                let uppercased = deleted.to_uppercase();
                editor.buffer_mut().insert_text_at(start_line, start_col, &uppercased);

                let delete_change = Change::delete(
                    Range::new((start_line, start_col), (end_line, end_col + 1)),
                    deleted,
                    cursor_before,
                );
                let insert_change = Change::insert((start_line, start_col), uppercased, cursor_before);
                let change = Change::composite(
                    vec![delete_change, insert_change],
                    cursor_before,
                    cursor_before,
                );
                editor.add_change(change);
            }
        }
    }

    Ok(())
}

/// Convert visual selection to lowercase
pub fn lowercase_visual_selection(editor: &mut Editor) -> Result<()> {
    let mode = editor.mode();
    let cursor_before = (
        editor.buffer().cursor().line(),
        editor.buffer().cursor().col(),
    );

    if let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() {
        match mode {
            Mode::VisualLine => {
                // Lowercase entire lines
                for line_idx in start_line..=end_line {
                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let lowercased = line_text.to_lowercase();

                        // Delete and replace
                        editor.buffer_mut().delete_range(line_idx, 0, line_idx, line_text.chars().count());
                        editor.buffer_mut().insert_text_at(line_idx, 0, &lowercased);

                        let delete_change = Change::delete(
                            Range::new((line_idx, 0), (line_idx, line_text.chars().count())),
                            line_text.to_string(),
                            cursor_before,
                        );
                        let insert_change = Change::insert((line_idx, 0), lowercased, cursor_before);
                        let change = Change::composite(
                            vec![delete_change, insert_change],
                            cursor_before,
                            cursor_before,
                        );
                        editor.add_change(change);
                    }
                }
            }
            Mode::VisualBlock => {
                // Convert to lowercase for visual block selection
                for line_idx in start_line..=end_line {
                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let chars: Vec<char> = line_text.chars().collect();

                        let line_start = start_col.min(chars.len());
                        let line_end = (end_col + 1).min(chars.len());

                        if line_start < line_end {
                            let deleted = editor
                                .buffer_mut()
                                .delete_range(line_idx, line_start, line_idx, line_end);
                            let lowercased = deleted.to_lowercase();
                            editor.buffer_mut().insert_text_at(
                                line_idx,
                                line_start,
                                &lowercased,
                            );

                            let delete_change = Change::delete(
                                Range::new((line_idx, line_start), (line_idx, line_end)),
                                deleted,
                                cursor_before,
                            );
                            let insert_change = Change::insert(
                                (line_idx, line_start),
                                lowercased,
                                cursor_before,
                            );
                            let change = Change::composite(
                                vec![delete_change, insert_change],
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
                let deleted = editor.buffer_mut().delete_range(
                    start_line,
                    start_col,
                    end_line,
                    end_col + 1,
                );
                let lowercased = deleted.to_lowercase();
                editor.buffer_mut().insert_text_at(start_line, start_col, &lowercased);

                let delete_change = Change::delete(
                    Range::new((start_line, start_col), (end_line, end_col + 1)),
                    deleted,
                    cursor_before,
                );
                let insert_change = Change::insert((start_line, start_col), lowercased, cursor_before);
                let change = Change::composite(
                    vec![delete_change, insert_change],
                    cursor_before,
                    cursor_before,
                );
                editor.add_change(change);
            }
        }
    }

    Ok(())
}

