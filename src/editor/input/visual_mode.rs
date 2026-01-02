//! Visual mode handler
//!
//! Handles all input events in Visual, VisualLine, and VisualBlock modes including:
//! - Visual mode motions (h/j/k/l, w/b/e, etc.)
//! - Visual mode operators (d, c, y, >, <, ~, u, U)
//! - Visual mode text objects (iw, aw, i", a{, etc.)
//! - Visual block operations (I, A, c, r)
//! - Visual mode commands (o to swap cursor, gv to reselect)
//! - Visual mode search (/ and ?)

use crate::editor::{Change, Editor, Motions, Operators, Range, TextObjects};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::helpers;
use super::numbers;

/// Handles input in Visual mode (Visual, VisualLine, VisualBlock)
pub fn handle_visual_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // Handle pending command for visual block replace and g prefix
    if let Some(pending) = editor.pending_command() {
        editor.clear_pending_command();
        match (pending, key_event.code) {
            ('g', KeyCode::Char('a'))
                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                // g Ctrl-A: Sequential increment in visual selection
                numbers::sequential_modify_numbers(editor, 1)?;
                helpers::exit_visual_mode_to_normal(editor);
                return Ok(());
            }
            ('g', KeyCode::Char('x'))
                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                // g Ctrl-X: Sequential decrement in visual selection
                numbers::sequential_modify_numbers(editor, -1)?;
                helpers::exit_visual_mode_to_normal(editor);
                return Ok(());
            }
            ('r', KeyCode::Char(ch)) => {
                // r{char} in visual block - replace all characters in selection with ch
                if editor.mode() == Mode::VisualBlock {
                    if let Some(((start_line, start_col), (end_line, end_col))) =
                        editor.visual_selection()
                    {
                        let cursor = editor.buffer().cursor();
                        let cursor_before = (cursor.line(), cursor.col());

                        for line_idx in start_line..=end_line {
                            if let Some(line) = editor.buffer().line(line_idx) {
                                let line_text = line.trim_end_matches('\n');
                                let chars: Vec<char> = line_text.chars().collect();

                                let line_start = start_col.min(chars.len());
                                let line_end = (end_col + 1).min(chars.len());

                                if line_start < line_end {
                                    // Delete the range
                                    let deleted = editor
                                        .buffer_mut()
                                        .delete_range(line_idx, line_start, line_idx, line_end);

                                    // Replace with the same number of replacement characters
                                    let replace_count = deleted.chars().count();
                                    let replacement = ch.to_string().repeat(replace_count);
                                    editor.buffer_mut().insert_text_at(
                                        line_idx,
                                        line_start,
                                        &replacement,
                                    );

                                    // Track change
                                    let delete_change = Change::delete(
                                        Range::new(
                                            (line_idx, line_start),
                                            (line_idx, line_end),
                                        ),
                                        deleted,
                                        cursor_before,
                                    );
                                    let insert_change = Change::insert(
                                        (line_idx, line_start),
                                        replacement,
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
                    editor.clear_visual_start();
                    editor.set_mode(Mode::Normal);
                    return Ok(());
                }
            }
            ('i', KeyCode::Char('w')) => {
                // viw - visual inner word
                if let Some(range) = TextObjects::inner_word(editor.buffer()) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('a', KeyCode::Char('w')) => {
                // vaw - visual around word
                if let Some(range) = TextObjects::around_word(editor.buffer()) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('i', KeyCode::Char('p')) => {
                // vip - visual inner paragraph
                if let Some(range) = TextObjects::inner_paragraph(editor.buffer()) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('a', KeyCode::Char('p')) => {
                // vap - visual around paragraph
                if let Some(range) = TextObjects::around_paragraph(editor.buffer()) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('i', KeyCode::Char('"')) | ('i', KeyCode::Char('\'')) | ('i', KeyCode::Char('`')) => {
                // vi" vi' vi` - visual inner quoted string
                let quote = match key_event.code {
                    KeyCode::Char(c) => c,
                    _ => return Ok(()),
                };
                if let Some(range) = TextObjects::quoted_string(editor.buffer(), quote, false) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('a', KeyCode::Char('"')) | ('a', KeyCode::Char('\'')) | ('a', KeyCode::Char('`')) => {
                // va" va' va` - visual around quoted string
                let quote = match key_event.code {
                    KeyCode::Char(c) => c,
                    _ => return Ok(()),
                };
                if let Some(range) = TextObjects::quoted_string(editor.buffer(), quote, true) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('i', KeyCode::Char('(')) | ('i', KeyCode::Char(')')) | ('i', KeyCode::Char('b')) => {
                // vi( vi) vib - visual inner parentheses
                if let Some(range) = TextObjects::paired_delimiters(editor.buffer(), '(', ')', false) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('a', KeyCode::Char('(')) | ('a', KeyCode::Char(')')) | ('a', KeyCode::Char('b')) => {
                // va( va) vab - visual around parentheses
                if let Some(range) = TextObjects::paired_delimiters(editor.buffer(), '(', ')', true) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('i', KeyCode::Char('[')) | ('i', KeyCode::Char(']')) => {
                // vi[ vi] - visual inner brackets
                if let Some(range) = TextObjects::paired_delimiters(editor.buffer(), '[', ']', false) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('a', KeyCode::Char('[')) | ('a', KeyCode::Char(']')) => {
                // va[ va] - visual around brackets
                if let Some(range) = TextObjects::paired_delimiters(editor.buffer(), '[', ']', true) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('i', KeyCode::Char('{')) | ('i', KeyCode::Char('}')) | ('i', KeyCode::Char('B')) => {
                // vi{ vi} viB - visual inner braces
                if let Some(range) = TextObjects::paired_delimiters(editor.buffer(), '{', '}', false) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            ('a', KeyCode::Char('{')) | ('a', KeyCode::Char('}')) | ('a', KeyCode::Char('B')) => {
                // va{ va} vaB - visual around braces
                if let Some(range) = TextObjects::paired_delimiters(editor.buffer(), '{', '}', true) {
                    editor.set_visual_start(range.start_line, range.start_col);
                    editor.buffer_mut().cursor_mut().set_position(range.end_line, range.end_col);
                }
                return Ok(());
            }
            _ => {
                // Unknown pending command, ignore
            }
        }
    }

    match key_event.code {
        KeyCode::Esc => {
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Text object prefixes in visual mode
        KeyCode::Char('i') | KeyCode::Char('a') => {
            // Set pending command to handle text objects (iw, aw, ip, ap, i{, a{, etc.)
            editor.set_pending_command(match key_event.code {
                KeyCode::Char(c) => c,
                _ => unreachable!(),
            });
        }
        // Motion keys work in visual mode too
        KeyCode::Char('h') | KeyCode::Left => {
            helpers::move_left(editor);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            helpers::move_down(editor);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            helpers::move_up(editor);
        }
        KeyCode::Char('l') | KeyCode::Right => {
            helpers::move_right(editor);
        }
        KeyCode::Char('w') => {
            let count = editor.effective_count();
            Motions::word_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        KeyCode::Char('b') => {
            let count = editor.effective_count();
            Motions::word_backward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        KeyCode::Char('e') => {
            let count = editor.effective_count();
            Motions::word_end_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        KeyCode::Char('0') => {
            // If there's already a count, treat this as a digit (e.g., "50j")
            // Otherwise, treat it as a motion to column 0
            if editor.count().is_some() {
                editor.append_count(0);
            } else {
                editor.buffer_mut().cursor_mut().set_col(0);
            }
        }
        KeyCode::Char('$') => {
            if editor.mode() == Mode::VisualBlock {
                // In visual block mode, $ should extend to the end of the longest line in the selection
                if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                    let mut max_len = 0;
                    for line_idx in start_line..=end_line {
                        if let Some(line) = editor.buffer().line(line_idx) {
                            let line_len = line.trim_end_matches('\n').chars().count();
                            max_len = max_len.max(line_len);
                        }
                    }
                    let col = if max_len > 0 { max_len - 1 } else { 0 };
                    let cursor = editor.buffer_mut().cursor_mut();
                    cursor.set_col(col);
                    cursor.update_desired_col(usize::MAX);
                }
            } else {
                // Normal visual mode: move to end of current line
                let line_idx = editor.buffer().cursor().line();
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_len = line.trim_end_matches('\n').chars().count();
                    let col = if line_len > 0 { line_len - 1 } else { 0 };
                    let cursor = editor.buffer_mut().cursor_mut();
                    cursor.set_col(col);
                    // Set desired_col to usize::MAX to indicate "always end of line"
                    cursor.update_desired_col(usize::MAX);
                }
            }
        }
        KeyCode::Char('G') => {
            // G - go to last line (or line specified by count)
            let target_line = if let Some(count) = editor.count() {
                count.saturating_sub(1)
            } else {
                let line_count = editor.buffer().line_count();
                let mut last_line = line_count.saturating_sub(1);
                // Check if last line is empty (just a newline)
                // If so, go to the previous line (Neovim behavior)
                if let Some(line) = editor.buffer().line(last_line) {
                    if line == "\n" || line.is_empty() {
                        last_line = last_line.saturating_sub(1);
                    }
                }
                last_line
            };
            editor.buffer_mut().cursor_mut().set_line(target_line);
            editor.buffer_mut().cursor_mut().set_col(0);
            editor.clear_count();
        }
        KeyCode::Char('g') => {
            // Handle gg motion in visual mode
            if editor.pending_command() == Some('g') {
                // gg - go to first line
                editor.buffer_mut().cursor_mut().set_line(0);
                editor.buffer_mut().cursor_mut().set_col(0);
                editor.clear_pending_command();
            } else {
                editor.set_pending_command('g');
            }
        }
        // Search forward in visual mode
        KeyCode::Char('/') => {
            editor.clear_search_buffer();
            editor.set_search_forward(true);
            editor.save_search_start_position();
            editor.set_mode(Mode::Search);
        }
        // Search backward in visual mode
        KeyCode::Char('?') => {
            editor.clear_search_buffer();
            editor.set_search_forward(false);
            editor.save_search_start_position();
            editor.set_mode(Mode::Search);
        }
        // Search next in visual mode
        KeyCode::Char('n') => {
            editor.search_next();
        }
        // Search previous in visual mode
        KeyCode::Char('N') => {
            editor.search_prev();
        }
        // Delete selection
        KeyCode::Char('d') | KeyCode::Char('x') => {
            helpers::delete_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Yank selection
        KeyCode::Char('y') => {
            helpers::yank_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Change selection
        KeyCode::Char('c') => {
            // For visual block mode, need to track the block for multi-line insert
            let visual_block_state = if editor.mode() == Mode::VisualBlock {
                editor
                    .visual_selection()
                    .map(|((start_line, start_col), (end_line, _))| {
                        (start_line, end_line, start_col)
                    })
            } else {
                None
            };

            helpers::delete_visual_selection(editor)?;

            if let Some((start_line, end_line, start_col)) = visual_block_state {
                // Set visual block insert state for multi-line replication
                // For 'c', move cursor to start_line (move_to_end = false)
                let cursor_before = (start_line, start_col);
                editor.set_visual_block_insert_state(Some((
                    start_line, end_line, start_col, false, false,
                )));
                editor.start_change_building(cursor_before);
            }

            helpers::save_and_clear_visual(editor);
            editor.set_mode(Mode::Insert);
        }
        // Join lines
        KeyCode::Char('J') => {
            if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                // Calculate expected cursor position after join
                // The cursor should be at the last space inserted (before the last line)
                let mut cursor_col = 0;
                for line_idx in start_line..end_line {
                    // Note: end_line not included
                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        cursor_col += line_text.chars().count();
                        if line_idx < end_line - 1 {
                            cursor_col += 1; // Space after this line
                        }
                    }
                }

                // Join all lines in the selection
                let count = (end_line - start_line) + 1;
                editor.buffer_mut().cursor_mut().set_position(start_line, 0);
                helpers::join_lines(editor, count)?;

                // Position cursor at the last inserted space
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(start_line, cursor_col);
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Move to other end of selection
        KeyCode::Char('o') => {
            if let Some(visual_start) = editor.visual_start() {
                let cursor = editor.buffer().cursor();
                let cursor_pos = (cursor.line(), cursor.col());

                if editor.mode() == Mode::VisualBlock {
                    // For visual block mode, flip to diagonally opposite corner
                    // Swap line from one with column from the other
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(visual_start.0, cursor_pos.1);
                    editor.set_visual_start(cursor_pos.0, visual_start.1);
                } else {
                    // For other visual modes, swap positions normally
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(visual_start.0, visual_start.1);
                    editor.set_visual_start(cursor_pos.0, cursor_pos.1);
                }
            }
        }
        // Flip horizontally (uppercase O) - swap columns only
        KeyCode::Char('O') => {
            if let Some(visual_start) = editor.visual_start() {
                let cursor = editor.buffer().cursor();
                let cursor_pos = (cursor.line(), cursor.col());

                if editor.mode() == Mode::VisualBlock {
                    // For visual block mode, flip horizontally (swap columns only, keep line)
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(cursor_pos.0, visual_start.1);
                    editor.set_visual_start(visual_start.0, cursor_pos.1);
                } else {
                    // For other visual modes, same as 'o'
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(visual_start.0, visual_start.1);
                    editor.set_visual_start(cursor_pos.0, cursor_pos.1);
                }
            }
        }
        // Switch to other visual modes
        KeyCode::Char('v') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            // Switching to VisualBlock - preserve both line and column of anchor
            editor.set_mode(Mode::VisualBlock);
        }
        KeyCode::Char('v') => {
            // Switching to Visual mode
            if editor.mode() == Mode::VisualLine {
                // When switching from VisualLine to Visual, preserve anchor line
                // The column is already 0 from VisualLine, which is fine
                // (we don't track the original column before entering VisualLine)
            }
            // For VisualBlock to Visual, preserve anchor as-is
            editor.set_mode(Mode::Visual);
        }
        KeyCode::Char('V') => {
            // Switching to VisualLine mode
            if let Some((anchor_line, _)) = editor.visual_start() {
                // Preserve anchor line, but set column to 0 for line-wise selection
                editor.set_visual_start(anchor_line, 0);
            } else {
                // Fallback: use cursor position (shouldn't happen in visual mode)
                let cursor = editor.buffer().cursor();
                editor.set_visual_start(cursor.line(), 0);
            }
            editor.set_mode(Mode::VisualLine);
        }
        // Visual block insert/append
        KeyCode::Char('I') => {
            if editor.mode() == Mode::VisualBlock {
                // Insert at beginning of block on each line
                if let Some(((start_line, start_col), (end_line, _))) =
                    editor.visual_selection()
                {
                    let cursor_before = (start_line, start_col);
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, start_col);
                    // Track visual block insert state: (start_line, end_line, col, is_append, move_to_end)
                    // For 'I', move cursor to end_line (move_to_end = true)
                    editor.set_visual_block_insert_state(Some((
                        start_line, end_line, start_col, false, true,
                    )));
                    editor.clear_visual_start();
                    editor.start_change_building(cursor_before);
                    editor.set_mode(Mode::Insert);
                }
            } else {
                // Regular visual mode - just enter insert at start of selection
                if let Some(((start_line, start_col), _)) = editor.visual_selection() {
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, start_col);
                    editor.clear_visual_start();
                    editor.set_mode(Mode::Insert);
                }
            }
        }
        KeyCode::Char('A') => {
            if editor.mode() == Mode::VisualBlock {
                // Append at end of block on each line
                if let Some(((start_line, _), (end_line, end_col))) = editor.visual_selection()
                {
                    // Get actual end column - clamp to line length to avoid overflow
                    let line_len = editor
                        .buffer()
                        .line(start_line)
                        .map(|l| l.trim_end_matches('\n').chars().count())
                        .unwrap_or(0);
                    let actual_end_col = end_col.min(line_len.saturating_sub(1));
                    let append_col = actual_end_col.saturating_add(1);

                    let cursor_before = (start_line, append_col);
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, append_col);
                    // Track visual block append state: (start_line, end_line, col, is_append, move_to_end)
                    // For 'A', move cursor to end_line (move_to_end = true)
                    editor.set_visual_block_insert_state(Some((
                        start_line, end_line, append_col, true, true,
                    )));
                    editor.clear_visual_start();
                    editor.start_change_building(cursor_before);
                    editor.set_mode(Mode::Insert);
                }
            } else {
                // Regular visual mode - just enter insert at end of selection
                if let Some((_, (end_line, end_col))) = editor.visual_selection() {
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(end_line, end_col + 1);
                    editor.clear_visual_start();
                    editor.set_mode(Mode::Insert);
                }
            }
        }
        // Replace in visual block mode
        KeyCode::Char('r') => {
            if editor.mode() == Mode::VisualBlock {
                // r{char} in visual block - wait for next char to replace selection
                editor.set_pending_command('r');
            } else {
                // Regular visual mode - not supported in standard vim, just delete and enter insert
                helpers::delete_visual_selection(editor)?;
                editor.clear_visual_start();
                editor.set_mode(Mode::Insert);
            }
        }
        // Case operations in visual mode
        KeyCode::Char('~') => {
            if editor.mode() == Mode::VisualBlock {
                // Toggle case for visual block selection
                if let Some(((start_line, start_col), (end_line, end_col))) =
                    editor.visual_selection()
                {
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());

                    for line_idx in start_line..=end_line {
                        if let Some(line) = editor.buffer().line(line_idx) {
                            let line_text = line.trim_end_matches('\n');
                            let chars: Vec<char> = line_text.chars().collect();

                            let line_start = start_col.min(chars.len());
                            let line_end = (end_col + 1).min(chars.len());

                            if line_start < line_end {
                                // Delete the range
                                let deleted = editor
                                    .buffer_mut()
                                    .delete_range(line_idx, line_start, line_idx, line_end);

                                // Toggle case
                                let toggled: String = deleted
                                    .chars()
                                    .map(|ch| {
                                        if ch.is_uppercase() {
                                            ch.to_lowercase().to_string()
                                        } else {
                                            ch.to_uppercase().to_string()
                                        }
                                    })
                                    .collect();

                                // Insert the toggled text
                                editor
                                    .buffer_mut()
                                    .insert_text_at(line_idx, line_start, &toggled);

                                // Track change
                                let delete_change = Change::delete(
                                    Range::new((line_idx, line_start), (line_idx, line_end)),
                                    deleted,
                                    cursor_before,
                                );
                                let insert_change = Change::insert(
                                    (line_idx, line_start),
                                    toggled,
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
                helpers::exit_visual_mode_to_normal(editor);
            } else {
                // Regular visual mode - toggle case of selection
                if let Some(((start_line, start_col), (end_line, end_col))) =
                    editor.visual_selection()
                {
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());

                    // Handle simple case: same line
                    if start_line == end_line {
                        if let Some(line) = editor.buffer().line(start_line) {
                            let line_text = line.trim_end_matches('\n');
                            let chars: Vec<char> = line_text.chars().collect();
                            let line_end = (end_col + 1).min(chars.len());

                            if start_col < line_end {
                                let deleted = editor
                                    .buffer_mut()
                                    .delete_range(start_line, start_col, start_line, line_end);
                                let toggled: String = deleted
                                    .chars()
                                    .map(|ch| {
                                        if ch.is_uppercase() {
                                            ch.to_lowercase().to_string()
                                        } else {
                                            ch.to_uppercase().to_string()
                                        }
                                    })
                                    .collect();
                                editor
                                    .buffer_mut()
                                    .insert_text_at(start_line, start_col, &toggled);

                                let delete_change = Change::delete(
                                    Range::new((start_line, start_col), (start_line, line_end)),
                                    deleted,
                                    cursor_before,
                                );
                                let insert_change = Change::insert(
                                    (start_line, start_col),
                                    toggled,
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
                    } else {
                        // Handle multi-line case: toggle case across multiple lines
                        for line_idx in start_line..=end_line {
                            if let Some(line) = editor.buffer().line(line_idx) {
                                let line_text = line.trim_end_matches('\n');
                                let chars: Vec<char> = line_text.chars().collect();

                                // Determine the range for this line
                                let line_start = if line_idx == start_line { start_col } else { 0 };
                                let line_end = if line_idx == end_line {
                                    (end_col + 1).min(chars.len())
                                } else {
                                    chars.len()
                                };

                                if line_start < line_end {
                                    // Delete the range
                                    let deleted = editor
                                        .buffer_mut()
                                        .delete_range(line_idx, line_start, line_idx, line_end);

                                    // Toggle case
                                    let toggled: String = deleted
                                        .chars()
                                        .map(|ch| {
                                            if ch.is_uppercase() {
                                                ch.to_lowercase().to_string()
                                            } else {
                                                ch.to_uppercase().to_string()
                                            }
                                        })
                                        .collect();

                                    // Insert the toggled text
                                    editor
                                        .buffer_mut()
                                        .insert_text_at(line_idx, line_start, &toggled);

                                    // Track change
                                    let delete_change = Change::delete(
                                        Range::new((line_idx, line_start), (line_idx, line_end)),
                                        deleted,
                                        cursor_before,
                                    );
                                    let insert_change = Change::insert(
                                        (line_idx, line_start),
                                        toggled,
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
                }
                helpers::exit_visual_mode_to_normal(editor);
            }
        }
        // Paste in visual mode (replace selection)
        KeyCode::Char('p') | KeyCode::Char('P') => {
            // Get the text to paste BEFORE deleting (since delete will overwrite register)
            let (paste_text, paste_type) = editor.get_from_register_with_type();

            // Delete the visual selection (saves to numbered register "1)
            helpers::delete_visual_selection(editor)?;

            // Restore the paste text to unnamed register
            editor.registers.set_with_type(None, paste_text, paste_type);

            // Move cursor back one position so paste_after puts text at the right place
            // After delete_visual_selection, cursor is at the start of the deleted text
            // We want to paste_after the character before that position
            let cursor_col = editor.buffer().cursor().col();
            if cursor_col > 0 {
                editor.buffer_mut().cursor_mut().set_col(cursor_col - 1);
            }

            // Paste from the unnamed register
            helpers::paste_after(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Uppercase in visual mode
        KeyCode::Char('U') => {
            helpers::uppercase_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Lowercase in visual mode
        KeyCode::Char('u') => {
            helpers::lowercase_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Indent/dedent in visual mode
        KeyCode::Char('>') => {
            if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                let cursor = editor.buffer().cursor();
                let cursor_before = (cursor.line(), cursor.col());
                let tab_width = editor.options.tab_width;
                let is_visual_block = editor.mode() == Mode::VisualBlock;
                let original_col = cursor_before.1;

                helpers::indent_lines_with_tracking(
                    editor,
                    start_line,
                    end_line + 1,
                    tab_width,
                    cursor_before,
                )?;

                // For visual block mode, move cursor to end line at adjusted column
                if is_visual_block {
                    let cursor = editor.buffer_mut().cursor_mut();
                    cursor.set_position(end_line, original_col + tab_width);
                }
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        KeyCode::Char('<') => {
            if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                let cursor = editor.buffer().cursor();
                let cursor_before = (cursor.line(), cursor.col());
                let tab_width = editor.options.tab_width;
                let is_visual_block = editor.mode() == Mode::VisualBlock;

                helpers::dedent_lines_with_tracking(
                    editor,
                    start_line,
                    end_line + 1,
                    tab_width,
                    cursor_before,
                )?;

                // For visual block mode, move cursor to start position (start_line, 0)
                if is_visual_block {
                    editor.buffer_mut().cursor_mut().set_position(start_line, 0);
                }
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        KeyCode::Char('=') => {
            if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                let tab_width = editor.options.tab_width;
                Operators::auto_indent_lines(
                    editor.buffer_mut(),
                    start_line,
                    end_line + 1,
                    tab_width,
                )?;
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Count prefix (for motions like 5j, 10w)
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let digit = c.to_digit(10).unwrap() as usize;
            // 0 is handled separately above as a motion
            if digit != 0 || editor.count().is_some() {
                editor.append_count(digit);
            }
        }
        _ => {}
    }
    Ok(())
}
