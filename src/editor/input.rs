use crate::buffer::Buffer;
use crate::editor::{Change, Editor, FindDirection, FindType, Motions, Operator, Operators, Range, Search, TextObjects};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

/// Type of case change operation
enum CaseChange {
    Lowercase,
    Uppercase,
    Toggle,
}

/// Handles input events for the editor
pub struct InputHandler;

impl InputHandler {
    /// Processes a keyboard event
    pub fn handle_key_event(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        // Record the event if we're recording a macro
        // (but don't record the 'q' that stops recording)
        let should_record_macro = editor.is_recording_macro()
            && !(key_event.code == KeyCode::Char('q') && editor.mode() == Mode::Normal);

        if should_record_macro {
            editor.record_macro_event(key_event);
        }

        match editor.mode() {
            Mode::Normal => Self::handle_normal_mode(editor, key_event),
            Mode::Insert => Self::handle_insert_mode(editor, key_event),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
                Self::handle_visual_mode(editor, key_event)
            }
            Mode::Command => Self::handle_command_mode(editor, key_event),
            Mode::Search => Self::handle_search_mode(editor, key_event),
            Mode::Replace => Self::handle_replace_mode(editor, key_event),
            Mode::Picker => Self::handle_picker_mode(editor, key_event),
        }
    }

    /// Handles input in Normal mode
    fn handle_normal_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        // Clear hover info on any key except 'K' (which requests hover)
        if !matches!(key_event.code, KeyCode::Char('K')) {
            editor.clear_hover();
        }

        // Handle pending leader key sequences (e.g., <Space>sf, <Space>sg)
        if editor.pending_leader() {
            editor.set_pending_leader(false);

            match key_event.code {
                KeyCode::Char('s') => {
                    // Expect 'f' or 'g' next
                    editor.set_pending_command('s');
                    return Ok(());
                }
                _ => {
                    // Cancel leader sequence
                    return Ok(());
                }
            }
        }

        // Handle second key in leader sequences
        if let Some('s') = editor.pending_command() {
            editor.clear_pending_command();

            match key_event.code {
                KeyCode::Char('f') => {
                    // <Space>sf - Find files
                    let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let picker = crate::editor::Picker::new_file_finder(base_dir);
                    editor.set_picker(picker);
                    editor.set_mode(Mode::Picker);
                    return Ok(());
                }
                KeyCode::Char('g') => {
                    // <Space>sg - Live grep
                    let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let picker = crate::editor::Picker::new_live_grep(base_dir);
                    editor.set_picker(picker);
                    editor.set_mode(Mode::Picker);
                    return Ok(());
                }
                _ => {
                    // Invalid sequence
                    return Ok(());
                }
            }
        }

        // Handle pending operator + motion (like 'dw', 'dd', 'yy')
        if let Some(operator) = editor.pending_operator() {
            let count = editor.effective_count();

            // Handle indent/dedent with motions - these are always line-wise
            match (operator, key_event.code) {
                (Operator::Indent, KeyCode::Char('G')) => {
                    editor.clear_pending_operator();
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = if let Some(cnt) = editor.count() {
                        cnt.saturating_sub(1)
                    } else {
                        editor.buffer().line_count().saturating_sub(1)
                    };
                    let tab_width = 4;

                    Self::indent_lines_with_tracking(editor, start_line, end_line + 1, tab_width, cursor_before)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Dedent, KeyCode::Char('G')) => {
                    editor.clear_pending_operator();
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = if let Some(cnt) = editor.count() {
                        cnt.saturating_sub(1)
                    } else {
                        editor.buffer().line_count().saturating_sub(1)
                    };
                    let tab_width = 4;

                    Self::dedent_lines_with_tracking(editor, start_line, end_line + 1, tab_width, cursor_before)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Delete, KeyCode::Char('G')) => {
                    // dG - delete from current line to end of file
                    editor.clear_pending_operator();
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = if let Some(cnt) = editor.count() {
                        cnt.saturating_sub(1)
                    } else {
                        editor.buffer().line_count().saturating_sub(1)
                    };

                    // Delete from current line to end line (inclusive)
                    let start_pos = (start_line, 0);
                    let end_pos = (end_line + 1, 0);

                    let deleted = editor.buffer_mut().delete_range(
                        start_line, 0,
                        end_line + 1, 0
                    );

                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);
                    editor.add_change(change);
                    editor.registers_mut().delete(deleted);

                    // Clamp cursor to buffer bounds
                    Self::clamp_cursor_to_buffer(editor);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Indent, KeyCode::Char('g')) => {
                    // >gg - indent from current line to first line
                    editor.set_pending_command('g');
                    return Ok(());
                }
                (Operator::Dedent, KeyCode::Char('g')) => {
                    // <gg - dedent from current line to first line
                    editor.set_pending_command('g');
                    return Ok(());
                }
                _ => {}
            }

            // Handle gg motion for indent/dedent
            if let Some('g') = editor.pending_command() {
                match (operator, key_event.code) {
                    (Operator::Indent, KeyCode::Char('g')) => {
                        editor.clear_pending_operator();
                        editor.clear_pending_command();
                        let cursor = editor.buffer().cursor();
                        let cursor_before = (cursor.line(), cursor.col());
                        let end_line = cursor.line();
                        let start_line = if let Some(cnt) = editor.count() {
                            cnt.saturating_sub(1)
                        } else {
                            0
                        };
                        let tab_width = 4;

                        Self::indent_lines_with_tracking(editor, start_line, end_line + 1, tab_width, cursor_before)?;
                        editor.clear_count();
                        return Ok(());
                    }
                    (Operator::Dedent, KeyCode::Char('g')) => {
                        editor.clear_pending_operator();
                        editor.clear_pending_command();
                        let cursor = editor.buffer().cursor();
                        let cursor_before = (cursor.line(), cursor.col());
                        let end_line = cursor.line();
                        let start_line = if let Some(cnt) = editor.count() {
                            cnt.saturating_sub(1)
                        } else {
                            0
                        };
                        let tab_width = 4;

                        Self::dedent_lines_with_tracking(editor, start_line, end_line + 1, tab_width, cursor_before)?;
                        editor.clear_count();
                        return Ok(());
                    }
                    _ => {}
                }
            }

            editor.clear_pending_operator();

            match (operator, key_event.code) {
                // Delete operations
                (Operator::Delete, KeyCode::Char('d')) => {
                    // dd - delete line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let line_count = editor.buffer().line_count();
                    let end_line = (start_line + count).min(line_count);

                    // Special handling for deleting to/past end of file
                    // When deleting the last line(s), also delete the newline from the previous line
                    // This moves the cursor up to the new last line
                    let (delete_start_line, delete_start_col) = if end_line >= line_count && start_line > 0 {
                        // Deleting to end of file, and there's a previous line
                        // Delete from end of previous line (including its newline)
                        if let Some(prev_line) = editor.buffer().line(start_line - 1) {
                            let prev_line_text = prev_line.trim_end_matches('\n');
                            let prev_line_len = prev_line_text.chars().count();
                            (start_line - 1, prev_line_len)
                        } else {
                            (start_line, 0)
                        }
                    } else {
                        (start_line, 0)
                    };

                    let start_pos = (delete_start_line, delete_start_col);
                    let end_pos = (end_line, 0);

                    let deleted = editor.buffer_mut().delete_range(
                        delete_start_line, delete_start_col,
                        end_line, 0
                    );
                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.registers_mut().delete(deleted);
                    editor.add_change(change);

                    // Clamp cursor to buffer bounds (handles end of file)
                    Self::clamp_cursor_to_buffer(editor);

                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Delete, KeyCode::Char('w')) => {
                    // dw - delete word (stops at newlines)
                    let start_cursor = editor.buffer().cursor().clone();
                    let cursor_before = (start_cursor.line(), start_cursor.col());
                    let start_line = start_cursor.line();
                    let start_col = start_cursor.col();

                    // Move cursor forward by word count times
                    Motions::word_forward(editor.buffer_mut(), count);

                    let end_cursor = editor.buffer().cursor();
                    let mut end_line = end_cursor.line();
                    let mut end_col = end_cursor.col();

                    // If we crossed a newline, stop at the end of the current line (before newline)
                    if end_line > start_line {
                        if let Some(line) = editor.buffer().line(start_line) {
                            let line_text = line.trim_end_matches('\n');
                            end_line = start_line;
                            end_col = line_text.chars().count();
                        }
                    }

                    let start_pos = (start_line, start_col);
                    let end_pos = (end_line, end_col);

                    let deleted = editor.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    // Position cursor at deletion start
                    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);

                    editor.registers_mut().delete(deleted);
                    editor.add_change(change);

                    // Clamp cursor to buffer bounds
                    Self::clamp_cursor_to_buffer(editor);

                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Delete, KeyCode::Char('$')) => {
                    // d$ - delete to end of line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let line_idx = cursor.line();
                    let col = cursor.col();

                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let line_len = line_text.chars().count();

                        if col < line_len {
                            let start_pos = (line_idx, col);
                            let end_pos = (line_idx, line_len);

                            let deleted = editor.buffer_mut().delete_range(line_idx, col, line_idx, line_len);
                            let range = Range::new(start_pos, end_pos);
                            let change = Change::delete(range, deleted.clone(), cursor_before);

                            editor.registers_mut().delete(deleted);
                            editor.add_change(change);

                            // Clamp cursor to buffer bounds
                            Self::clamp_cursor_to_buffer(editor);
                        }
                    }
                    editor.clear_count();
                    return Ok(());
                }
                // Yank operations
                (Operator::Yank, KeyCode::Char('y')) => {
                    // yy - yank line
                    let yanked = Operators::yank_line(editor.buffer(), count)?;
                    editor.registers_mut().yank(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Yank, KeyCode::Char('w')) => {
                    // yw - yank word
                    let yanked = Operators::yank_word(editor.buffer_mut(), count)?;
                    editor.registers_mut().yank(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Yank, KeyCode::Char('$')) => {
                    // y$ - yank to end of line
                    let yanked = Operators::yank_to_end_of_line(editor.buffer())?;
                    editor.registers_mut().yank(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                // Change operations (delete + insert mode)
                (Operator::Change, KeyCode::Char('c')) => {
                    // cc - change line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = (start_line + count).min(editor.buffer().line_count());

                    let start_pos = (start_line, 0);
                    let end_pos = (end_line, 0);

                    let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line, 0);
                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.registers_mut().delete(deleted);
                    editor.add_change(change);
                    editor.clear_count();
                    let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                    editor.start_change_building(cursor_before);
                    editor.set_mode(Mode::Insert);
                    Self::insert_line_above(editor)?;
                    return Ok(());
                }
                (Operator::Change, KeyCode::Char('w')) => {
                    // cw - change word
                    let start_cursor = editor.buffer().cursor().clone();
                    let cursor_before = (start_cursor.line(), start_cursor.col());
                    let start_line = start_cursor.line();
                    let start_col = start_cursor.col();

                    // Move cursor forward by word count times
                    Motions::word_forward(editor.buffer_mut(), count);

                    let end_cursor = editor.buffer().cursor();
                    let end_line = end_cursor.line();
                    let end_col = end_cursor.col();

                    let start_pos = (start_line, start_col);
                    let end_pos = (end_line, end_col);

                    let deleted = editor.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    // Position cursor at deletion start
                    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);

                    editor.registers_mut().delete(deleted);
                    editor.add_change(change);

                    // Clamp cursor to buffer bounds
                    Self::clamp_cursor_to_buffer(editor);

                    editor.clear_count();
                    let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                    editor.start_change_building(cursor_before);
                    editor.set_mode(Mode::Insert);
                    return Ok(());
                }
                (Operator::Change, KeyCode::Char('$')) => {
                    // c$ - change to end of line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let line_idx = cursor.line();
                    let col = cursor.col();

                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let line_len = line_text.chars().count();

                        if col < line_len {
                            let start_pos = (line_idx, col);
                            let end_pos = (line_idx, line_len);

                            let deleted = editor.buffer_mut().delete_range(line_idx, col, line_idx, line_len);
                            let range = Range::new(start_pos, end_pos);
                            let change = Change::delete(range, deleted.clone(), cursor_before);

                            editor.registers_mut().delete(deleted);
                            editor.add_change(change);

                            // Clamp cursor to buffer bounds
                            Self::clamp_cursor_to_buffer(editor);
                        }
                    }
                    editor.clear_count();
                    let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                    editor.start_change_building(cursor_before);
                    editor.set_mode(Mode::Insert);
                    return Ok(());
                }
                // Case change operations
                (Operator::Lowercase, KeyCode::Char('u')) => {
                    // gugu - lowercase line
                    Self::change_case_line(editor, count, CaseChange::Lowercase)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Uppercase, KeyCode::Char('U')) => {
                    // gUgU - uppercase line
                    Self::change_case_line(editor, count, CaseChange::Uppercase)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::ToggleCase, KeyCode::Char('~')) => {
                    // g~g~ - toggle case line
                    Self::change_case_line(editor, count, CaseChange::Toggle)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Lowercase, KeyCode::Char('w')) => {
                    // guw - lowercase word
                    Self::change_case_motion(editor, count, CaseChange::Lowercase, |buf, cnt| {
                        Motions::word_forward(buf, cnt);
                    })?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Uppercase, KeyCode::Char('w')) => {
                    // gUw - uppercase word
                    Self::change_case_motion(editor, count, CaseChange::Uppercase, |buf, cnt| {
                        Motions::word_forward(buf, cnt);
                    })?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::ToggleCase, KeyCode::Char('w')) => {
                    // g~w - toggle case word
                    Self::change_case_motion(editor, count, CaseChange::Toggle, |buf, cnt| {
                        Motions::word_forward(buf, cnt);
                    })?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Lowercase, KeyCode::Char('$')) => {
                    // gu$ - lowercase to end of line
                    Self::change_case_to_end_of_line(editor, CaseChange::Lowercase)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Uppercase, KeyCode::Char('$')) => {
                    // gU$ - uppercase to end of line
                    Self::change_case_to_end_of_line(editor, CaseChange::Uppercase)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::ToggleCase, KeyCode::Char('$')) => {
                    // g~$ - toggle case to end of line
                    Self::change_case_to_end_of_line(editor, CaseChange::Toggle)?;
                    editor.clear_count();
                    return Ok(());
                }
                // Text objects with 'i' (inner)
                (_, KeyCode::Char('i')) => {
                    // Restore operator and set pending command to 'i'
                    editor.set_pending_operator(operator);
                    editor.set_pending_command('i');
                    return Ok(());
                }
                // Text objects with 'a' (around)
                (_, KeyCode::Char('a')) => {
                    // Restore operator and set pending command to 'a'
                    editor.set_pending_operator(operator);
                    editor.set_pending_command('a');
                    return Ok(());
                }
                // Handle operators with motion keys (j, k)
                (Operator::Delete, KeyCode::Char('j')) => {
                    // dj - delete current line and line below
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = (start_line + count + 1).min(editor.buffer().line_count());

                    let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line, 0);
                    let range = Range::new((start_line, 0), (end_line, 0));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.registers_mut().delete(deleted);
                    editor.add_change(change);
                    Self::clamp_cursor_to_buffer(editor);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Delete, KeyCode::Char('k')) => {
                    // dk - delete current line and line above
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let end_line = cursor.line() + 1;
                    let start_line = cursor.line().saturating_sub(count);

                    let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line, 0);
                    let range = Range::new((start_line, 0), (end_line, 0));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.registers_mut().delete(deleted);
                    editor.add_change(change);
                    Self::clamp_cursor_to_buffer(editor);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Yank, KeyCode::Char('j')) => {
                    // yj - yank current line and line below
                    let start_line = editor.buffer().cursor().line();
                    let end_line = (start_line + count + 1).min(editor.buffer().line_count());

                    let mut yanked = String::new();
                    for line_idx in start_line..end_line {
                        if let Some(line) = editor.buffer().line(line_idx) {
                            yanked.push_str(&line);
                        }
                    }
                    editor.registers_mut().yank(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Yank, KeyCode::Char('k')) => {
                    // yk - yank current line and line above
                    let end_line = editor.buffer().cursor().line() + 1;
                    let start_line = editor.buffer().cursor().line().saturating_sub(count);

                    let mut yanked = String::new();
                    for line_idx in start_line..end_line {
                        if let Some(line) = editor.buffer().line(line_idx) {
                            yanked.push_str(&line);
                        }
                    }
                    editor.registers_mut().yank(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                // Paragraph motions with operators
                (Operator::Delete, KeyCode::Char('}')) => {
                    // d} - delete to next paragraph
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let start_col = cursor.col();

                    // Move to next paragraph
                    Motions::paragraph_forward(editor.buffer_mut(), count);
                    let end_line = editor.buffer().cursor().line();
                    let end_col = 0;

                    let deleted = editor.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new((start_line, start_col), (end_line, end_col));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
                    editor.registers_mut().delete(deleted);
                    editor.add_change(change);
                    Self::clamp_cursor_to_buffer(editor);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Delete, KeyCode::Char('{')) => {
                    // d{ - delete to previous paragraph
                    let start_cursor = editor.buffer().cursor();
                    let cursor_before = (start_cursor.line(), start_cursor.col());
                    let end_line = start_cursor.line();
                    let end_col = start_cursor.col();

                    // Move to previous paragraph
                    Motions::paragraph_backward(editor.buffer_mut(), count);
                    let start_line = editor.buffer().cursor().line();
                    let start_col = 0;

                    let deleted = editor.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new((start_line, start_col), (end_line, end_col));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.registers_mut().delete(deleted);
                    editor.add_change(change);
                    Self::clamp_cursor_to_buffer(editor);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Delete, KeyCode::Char('%')) => {
                    // d% - delete to matching bracket
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let start_col = cursor.col();

                    // Find matching bracket position
                    let rope = editor.buffer().rope();
                    let text = rope.to_string();
                    let chars: Vec<char> = text.chars().collect();

                    // Convert line+col to absolute position
                    let mut abs_start = 0;
                    for i in 0..start_line {
                        if i < rope.len_lines() {
                            abs_start += rope.line(i).len_chars();
                        }
                    }
                    abs_start += start_col;

                    if abs_start >= chars.len() {
                        editor.clear_count();
                        return Ok(());
                    }

                    let current_char = chars[abs_start];

                    // Determine if we're on a bracket
                    let (is_opening, matching_char) = match current_char {
                        '(' => (true, ')'),
                        ')' => (false, '('),
                        '[' => (true, ']'),
                        ']' => (false, '['),
                        '{' => (true, '}'),
                        '}' => (false, '{'),
                        '<' => (true, '>'),
                        '>' => (false, '<'),
                        _ => {
                            // Not on a bracket, do nothing
                            editor.clear_count();
                            return Ok(());
                        }
                    };

                    // Find matching bracket
                    let match_abs_pos = if is_opening {
                        Motions::find_matching_bracket_forward(&chars, abs_start, current_char, matching_char)
                    } else {
                        Motions::find_matching_bracket_backward(&chars, abs_start, matching_char, current_char)
                    };

                    if let Some(abs_end) = match_abs_pos {
                        // Determine delete range (from min to max, inclusive)
                        let (delete_start, delete_end) = if abs_start < abs_end {
                            (abs_start, abs_end + 1)
                        } else {
                            (abs_end, abs_start + 1)
                        };

                        // Convert absolute positions to (line, col)
                        let (start_line, start_col) = Motions::abs_pos_to_line_col(&rope, delete_start);
                        let (end_line, end_col) = Motions::abs_pos_to_line_col(&rope, delete_end);

                        // Delete the range
                        let deleted = editor.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
                        let range = Range::new((start_line, start_col), (end_line, end_col));
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
                        editor.registers_mut().delete(deleted);
                        editor.add_change(change);
                        Self::clamp_cursor_to_buffer(editor);
                    }

                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Yank, KeyCode::Char('}')) => {
                    // y} - yank to next paragraph
                    let start_line = editor.buffer().cursor().line();
                    let start_col = editor.buffer().cursor().col();

                    Motions::paragraph_forward(editor.buffer_mut(), count);
                    let end_line = editor.buffer().cursor().line();

                    let mut yanked = String::new();
                    if start_line == end_line {
                        if let Some(line) = editor.buffer().line(start_line) {
                            let chars: Vec<char> = line.chars().collect();
                            yanked = chars[start_col..].iter().collect();
                        }
                    } else {
                        for line_idx in start_line..=end_line {
                            if let Some(line) = editor.buffer().line(line_idx) {
                                if line_idx == start_line {
                                    let chars: Vec<char> = line.chars().collect();
                                    yanked.push_str(&chars[start_col..].iter().collect::<String>());
                                } else {
                                    yanked.push_str(&line);
                                }
                            }
                        }
                    }

                    editor.registers_mut().yank(yanked);
                    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Yank, KeyCode::Char('{')) => {
                    // y{ - yank to previous paragraph
                    let end_line = editor.buffer().cursor().line();
                    let end_col = editor.buffer().cursor().col();

                    Motions::paragraph_backward(editor.buffer_mut(), count);
                    let start_line = editor.buffer().cursor().line();

                    let mut yanked = String::new();
                    for line_idx in start_line..=end_line {
                        if let Some(line) = editor.buffer().line(line_idx) {
                            if line_idx == end_line {
                                let chars: Vec<char> = line.chars().collect();
                                yanked.push_str(&chars[..=end_col.min(chars.len().saturating_sub(1))].iter().collect::<String>());
                            } else {
                                yanked.push_str(&line);
                            }
                        }
                    }

                    editor.registers_mut().yank(yanked);
                    editor.buffer_mut().cursor_mut().set_position(end_line, end_col);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Change, KeyCode::Char('}')) => {
                    // c} - change to next paragraph
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let start_col = cursor.col();

                    Motions::paragraph_forward(editor.buffer_mut(), count);
                    let end_line = editor.buffer().cursor().line();
                    let end_col = 0;

                    let deleted = editor.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new((start_line, start_col), (end_line, end_col));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
                    editor.registers_mut().delete(deleted);
                    editor.add_change(change);
                    editor.set_mode(Mode::Insert);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Change, KeyCode::Char('{')) => {
                    // c{ - change to previous paragraph
                    let end_line = editor.buffer().cursor().line();
                    let end_col = editor.buffer().cursor().col();
                    let cursor_before = (end_line, end_col);

                    Motions::paragraph_backward(editor.buffer_mut(), count);
                    let start_line = editor.buffer().cursor().line();
                    let start_col = 0;

                    let deleted = editor.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new((start_line, start_col), (end_line, end_col));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.registers_mut().delete(deleted);
                    editor.add_change(change);
                    editor.set_mode(Mode::Insert);
                    editor.clear_count();
                    return Ok(());
                }
                // Indent operations
                (Operator::Indent, KeyCode::Char('>')) => {
                    // >> - indent line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = start_line + count;
                    let tab_width = 4; // TODO: get from settings

                    Self::indent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Indent, KeyCode::Char('j')) | (Operator::Indent, KeyCode::Down) => {
                    // >j - indent current and next line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = start_line + count + 1;
                    let tab_width = 4;

                    Self::indent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Indent, KeyCode::Char('k')) | (Operator::Indent, KeyCode::Up) => {
                    // >k - indent current and previous line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let current_line = cursor.line();
                    let start_line = current_line.saturating_sub(count);
                    let end_line = current_line + 1;
                    let tab_width = 4;

                    Self::indent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
                    editor.clear_count();
                    return Ok(());
                }
                // Dedent operations
                (Operator::Dedent, KeyCode::Char('<')) => {
                    // << - dedent line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = start_line + count;
                    let tab_width = 4; // TODO: get from settings

                    Self::dedent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Dedent, KeyCode::Char('j')) | (Operator::Dedent, KeyCode::Down) => {
                    // <j - dedent current and next line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = start_line + count + 1;
                    let tab_width = 4;

                    Self::dedent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Dedent, KeyCode::Char('k')) | (Operator::Dedent, KeyCode::Up) => {
                    // <k - dedent current and previous line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let current_line = cursor.line();
                    let start_line = current_line.saturating_sub(count);
                    let end_line = current_line + 1;
                    let tab_width = 4;

                    Self::dedent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
                    editor.clear_count();
                    return Ok(());
                }
                _ => {
                    // Unknown operator+motion combo
                    editor.clear_count();
                }
            }
        }

        // Handle text objects after 'i' or 'a' with an operator
        if let Some(text_obj_type) = editor.pending_command() {
            if (text_obj_type == 'i' || text_obj_type == 'a') && editor.pending_operator().is_some() {
                let operator = editor.pending_operator().unwrap();
                editor.clear_pending_command();
                editor.clear_pending_operator();
                editor.clear_count();

                let result = match key_event.code {
                    KeyCode::Char('w') => {
                        // iw or aw - word
                        if text_obj_type == 'i' {
                            TextObjects::inner_word(editor.buffer())
                        } else {
                            TextObjects::around_word(editor.buffer())
                        }
                    }
                    KeyCode::Char('"') | KeyCode::Char('\'') | KeyCode::Char('`') => {
                        // i" or a" - quoted string
                        let quote = match key_event.code {
                            KeyCode::Char(c) => c,
                            _ => unreachable!(),
                        };
                        TextObjects::quoted_string(editor.buffer(), quote, text_obj_type == 'a')
                    }
                    KeyCode::Char('(') | KeyCode::Char(')') | KeyCode::Char('b') => {
                        // i( or a( or ib or ab - parentheses
                        TextObjects::paired_delimiters(editor.buffer(), '(', ')', text_obj_type == 'a')
                    }
                    KeyCode::Char('[') | KeyCode::Char(']') => {
                        // i[ or a[ - brackets
                        TextObjects::paired_delimiters(editor.buffer(), '[', ']', text_obj_type == 'a')
                    }
                    KeyCode::Char('{') | KeyCode::Char('}') | KeyCode::Char('B') => {
                        // i{ or a{ or iB or aB - braces
                        TextObjects::paired_delimiters(editor.buffer(), '{', '}', text_obj_type == 'a')
                    }
                    KeyCode::Char('<') | KeyCode::Char('>') => {
                        // i< or a< or i> or a> - angle brackets
                        TextObjects::paired_delimiters(editor.buffer(), '<', '>', text_obj_type == 'a')
                    }
                    _ => {
                        // Unknown text object
                        return Ok(());
                    }
                };

                if let Some(range) = result {
                    match operator {
                        Operator::Delete => {
                            let cursor = editor.buffer().cursor();
                            let cursor_before = (cursor.line(), cursor.col());

                            // Get the text to be deleted first
                            let deleted = TextObjects::yank_range(editor.buffer(), range)?;

                            // Create Change with exclusive end position
                            let change_range = Range::new(
                                (range.start_line, range.start_col),
                                (range.end_line, range.end_col + 1)
                            );
                            let change = Change::delete(change_range, deleted.clone(), cursor_before);

                            // Apply the change to actually delete the text
                            change.apply(editor.buffer_mut());

                            editor.registers_mut().delete(deleted);
                            editor.add_change(change);

                            // Clamp cursor to buffer bounds
                            Self::clamp_cursor_to_buffer(editor);
                        }
                        Operator::Yank => {
                            let yanked = TextObjects::yank_range(editor.buffer(), range)?;
                            editor.registers_mut().yank(yanked);
                        }
                        Operator::Change => {
                            let cursor = editor.buffer().cursor();
                            let cursor_before = (cursor.line(), cursor.col());

                            // Get the text to be deleted first
                            let deleted = TextObjects::yank_range(editor.buffer(), range)?;

                            // Create Change with exclusive end position
                            let change_range = Range::new(
                                (range.start_line, range.start_col),
                                (range.end_line, range.end_col + 1)
                            );
                            let change = Change::delete(change_range, deleted.clone(), cursor_before);

                            // Apply the change to actually delete the text
                            change.apply(editor.buffer_mut());

                            editor.registers_mut().delete(deleted);
                            editor.add_change(change);

                            // Clamp cursor to buffer bounds
                            Self::clamp_cursor_to_buffer(editor);

                            // Start building composite change for insert mode
                            let new_cursor = editor.buffer().cursor();
                            let new_cursor_pos = (new_cursor.line(), new_cursor.col());
                            editor.start_change_building(new_cursor_pos);

                            editor.set_mode(Mode::Insert);
                        }
                        Operator::Lowercase => {
                            let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());

                            // Get the text in the range
                            let text = TextObjects::yank_range(editor.buffer(), range)?;

                            // Transform to lowercase
                            let transformed = text.to_lowercase();

                            if transformed != text {
                                // Delete the old text
                                let deleted = editor.buffer_mut().delete_range(
                                    range.start_line, range.start_col,
                                    range.end_line, range.end_col + 1
                                );
                                let delete_range = Range::new(
                                    (range.start_line, range.start_col),
                                    (range.end_line, range.end_col + 1)
                                );
                                let delete_change = Change::delete(delete_range, deleted, cursor_before);

                                // Insert the transformed text
                                let insert_change = Change::insert(
                                    (range.start_line, range.start_col),
                                    transformed,
                                    cursor_before
                                );
                                insert_change.apply(editor.buffer_mut());

                                editor.add_change(delete_change);
                                editor.add_change(insert_change);
                            }
                        }
                        Operator::Uppercase => {
                            let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());

                            // Get the text in the range
                            let text = TextObjects::yank_range(editor.buffer(), range)?;

                            // Transform to uppercase
                            let transformed = text.to_uppercase();

                            if transformed != text {
                                // Delete the old text
                                let deleted = editor.buffer_mut().delete_range(
                                    range.start_line, range.start_col,
                                    range.end_line, range.end_col + 1
                                );
                                let delete_range = Range::new(
                                    (range.start_line, range.start_col),
                                    (range.end_line, range.end_col + 1)
                                );
                                let delete_change = Change::delete(delete_range, deleted, cursor_before);

                                // Insert the transformed text
                                let insert_change = Change::insert(
                                    (range.start_line, range.start_col),
                                    transformed,
                                    cursor_before
                                );
                                insert_change.apply(editor.buffer_mut());

                                editor.add_change(delete_change);
                                editor.add_change(insert_change);
                            }
                        }
                        Operator::ToggleCase => {
                            let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());

                            // Get the text in the range
                            let text = TextObjects::yank_range(editor.buffer(), range)?;

                            // Toggle case
                            let transformed: String = text.chars().map(|ch| {
                                if ch.is_lowercase() {
                                    ch.to_uppercase().to_string()
                                } else {
                                    ch.to_lowercase().to_string()
                                }
                            }).collect();

                            if transformed != text {
                                // Delete the old text
                                let deleted = editor.buffer_mut().delete_range(
                                    range.start_line, range.start_col,
                                    range.end_line, range.end_col + 1
                                );
                                let delete_range = Range::new(
                                    (range.start_line, range.start_col),
                                    (range.end_line, range.end_col + 1)
                                );
                                let delete_change = Change::delete(delete_range, deleted, cursor_before);

                                // Insert the transformed text
                                let insert_change = Change::insert(
                                    (range.start_line, range.start_col),
                                    transformed,
                                    cursor_before
                                );
                                insert_change.apply(editor.buffer_mut());

                                editor.add_change(delete_change);
                                editor.add_change(insert_change);
                            }
                        }
                        // Indent/dedent don't make sense with text objects, just ignore
                        Operator::Indent | Operator::Dedent => {}
                    }
                }

                return Ok(());
            }
        }

        // Handle pending command (like 'g' waiting for second character)
        if let Some(pending) = editor.pending_command() {
            editor.clear_pending_command();
            match (pending, key_event.code) {
                ('g', KeyCode::Char('g')) => {
                    // gg - go to first line
                    let target_line = editor.count().unwrap_or(1).saturating_sub(1);
                    editor.buffer_mut().cursor_mut().set_position(target_line, 0);
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('d')) => {
                    // gd - go to definition (LSP)
                    editor.request_goto_definition();
                    return Ok(());
                }
                ('g', KeyCode::Char('J')) => {
                    // gJ - join lines without space
                    let count = editor.effective_count();
                    Self::join_lines_no_space(editor, count)?;
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('e')) => {
                    // ge - backward to end of word
                    let count = editor.effective_count();
                    Motions::word_end_backward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('E')) => {
                    // gE - backward to end of WORD
                    let count = editor.effective_count();
                    Motions::word_end_backward_big(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('_')) => {
                    // g_ - move to last non-blank character
                    Motions::last_non_blank(editor.buffer_mut());
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('u')) => {
                    // gu{motion} - lowercase text
                    editor.set_pending_operator(Operator::Lowercase);
                    return Ok(());
                }
                ('g', KeyCode::Char('U')) => {
                    // gU{motion} - uppercase text
                    editor.set_pending_operator(Operator::Uppercase);
                    return Ok(());
                }
                ('g', KeyCode::Char('~')) => {
                    // g~{motion} - toggle case
                    editor.set_pending_operator(Operator::ToggleCase);
                    return Ok(());
                }
                ('m', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
                    // m{a-z} - set mark
                    editor.set_mark(ch);
                    return Ok(());
                }
                ('\'', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
                    // '{a-z} - jump to mark line
                    editor.add_jump(); // Add current position to jump list before jumping
                    editor.jump_to_mark_line(ch);
                    return Ok(());
                }
                ('`', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
                    // `{a-z} - jump to mark exact position
                    editor.add_jump(); // Add current position to jump list before jumping
                    editor.jump_to_mark(ch);
                    return Ok(());
                }
                ('q', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
                    // q{a-z} - start recording macro to register
                    editor.start_macro_recording(ch);
                    return Ok(());
                }
                ('@', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
                    // @{a-z} - play back macro from register
                    if let Some(events) = editor.get_macro(ch) {
                        // Clone the events to avoid borrow checker issues
                        let events = events.clone();
                        for event in events {
                            Self::handle_key_event(editor, event)?;
                        }
                    }
                    return Ok(());
                }
                ('f', KeyCode::Char(ch)) => {
                    // f{char} - find next occurrence of char on line
                    let count = editor.effective_count();
                    if Motions::find_char_forward(editor.buffer_mut(), ch, count) {
                        editor.set_last_find(ch, FindType::Find, FindDirection::Forward);
                    }
                    editor.clear_count();
                    return Ok(());
                }
                ('F', KeyCode::Char(ch)) => {
                    // F{char} - find previous occurrence of char on line
                    let count = editor.effective_count();
                    if Motions::find_char_backward(editor.buffer_mut(), ch, count) {
                        editor.set_last_find(ch, FindType::Find, FindDirection::Backward);
                    }
                    editor.clear_count();
                    return Ok(());
                }
                ('t', KeyCode::Char(ch)) => {
                    // t{char} - till next occurrence (cursor before char)
                    let count = editor.effective_count();
                    if Motions::till_char_forward(editor.buffer_mut(), ch, count) {
                        editor.set_last_find(ch, FindType::Till, FindDirection::Forward);
                    }
                    editor.clear_count();
                    return Ok(());
                }
                ('T', KeyCode::Char(ch)) => {
                    // T{char} - till previous occurrence (cursor after char)
                    let count = editor.effective_count();
                    if Motions::till_char_backward(editor.buffer_mut(), ch, count) {
                        editor.set_last_find(ch, FindType::Till, FindDirection::Backward);
                    }
                    editor.clear_count();
                    return Ok(());
                }
                _ => {
                    // Unknown command sequence, clear and continue
                    editor.clear_count();
                }
            }
        }

        match key_event.code {
            // Quit
            KeyCode::Char('q') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.quit();
            }
            // Jump forward (Ctrl-I) - must come before 'i'
            KeyCode::Char('i') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.jump_forward();
            }
            // Jump back (Ctrl-O) - must come before 'o'
            KeyCode::Char('o') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.jump_back();
            }
            // Scroll down half page (Ctrl-D)
            KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let half_page = 10; // TODO: calculate based on viewport height
                let count = editor.count().unwrap_or(half_page);
                let line_count = editor.buffer().line_count();
                let mut max_line = line_count.saturating_sub(1);

                // Check if last line is empty (just a newline)
                // If so, don't allow moving to it (Neovim behavior)
                if let Some(last_line) = editor.buffer().line(max_line) {
                    if last_line == "\n" || last_line.is_empty() {
                        max_line = max_line.saturating_sub(1);
                    }
                }

                let cursor = editor.buffer_mut().cursor_mut();
                let new_line = (cursor.line() + count).min(max_line);
                cursor.set_line(new_line);
                Self::clamp_cursor_to_line(editor);
                editor.clear_count();
            }
            // Scroll up half page (Ctrl-U)
            KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let half_page = 10; // TODO: calculate based on viewport height
                let count = editor.count().unwrap_or(half_page);
                let cursor = editor.buffer_mut().cursor_mut();
                let new_line = cursor.line().saturating_sub(count);
                cursor.set_line(new_line);
                Self::clamp_cursor_to_line(editor);
                editor.clear_count();
            }
            // Enter Visual Block mode (Ctrl-V)
            KeyCode::Char('v') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let cursor = editor.buffer().cursor();
                editor.set_visual_start(cursor.line(), cursor.col());
                editor.set_mode(Mode::VisualBlock);
            }
            // Increment number (Ctrl-A)
            KeyCode::Char('a') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let count = editor.effective_count();
                Self::increment_number(editor, count)?;
                editor.clear_count();
            }
            // Decrement number (Ctrl-X)
            KeyCode::Char('x') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let count = editor.effective_count();
                Self::decrement_number(editor, count)?;
                editor.clear_count();
            }
            // Enter Insert mode
            KeyCode::Char('i') => {
                let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
            }
            KeyCode::Char('a') => {
                let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                // Move cursor right (insert after)
                let cursor = editor.buffer_mut().cursor_mut();
                cursor.move_right(1);
            }
            KeyCode::Char('I') => {
                let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                // Move to start of line
                let cursor = editor.buffer_mut().cursor_mut();
                cursor.set_col(0);
            }
            KeyCode::Char('A') => {
                let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                // Move to end of line
                let line_idx = editor.buffer().cursor().line();
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_len = line.trim_end_matches('\n').chars().count();
                    editor.buffer_mut().cursor_mut().set_col(line_len);
                }
            }
            KeyCode::Char('o') => {
                let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                // Insert new line below and move to it
                Self::insert_line_below(editor)?;
            }
            KeyCode::Char('O') => {
                let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                // Insert new line above and move to it
                Self::insert_line_above(editor)?;
            }
            // Motion commands
            KeyCode::Char('h') | KeyCode::Left => {
                Self::move_left(editor);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                Self::move_down(editor);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                Self::move_up(editor);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                Self::move_right(editor);
            }
            KeyCode::Char('K') => {
                // K - show hover information (LSP)
                editor.request_hover();
            }
            // Line motions
            KeyCode::Char('0') => {
                editor.buffer_mut().cursor_mut().set_col(0);
                editor.clear_count();
            }
            KeyCode::Char('$') => {
                let line_idx = editor.buffer().cursor().line();
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_len = line.trim_end_matches('\n').chars().count();
                    let col = if line_len > 0 { line_len - 1 } else { 0 };
                    editor.buffer_mut().cursor_mut().set_col(col);
                }
                editor.clear_count();
            }
            KeyCode::Char('^') => {
                // ^ - move to first non-blank character
                Motions::first_non_blank(editor.buffer_mut());
                editor.clear_count();
            }
            KeyCode::Char('_') => {
                // _ - move to first non-blank character (same as ^)
                Motions::first_non_blank_underscore(editor.buffer_mut());
                editor.clear_count();
            }
            KeyCode::Char('+') => {
                // + - move to first non-blank of next line
                let count = editor.effective_count();
                Motions::plus_motion(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('-') => {
                // - - move to first non-blank of previous line
                let count = editor.effective_count();
                Motions::minus_motion(editor.buffer_mut(), count);
                editor.clear_count();
            }
            // Count prefix
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let digit = c.to_digit(10).unwrap() as usize;
                // 0 is a motion, not a count prefix
                if digit != 0 || editor.count().is_some() {
                    editor.append_count(digit);
                }
            }
            // Enter Command mode
            KeyCode::Char(':') => {
                editor.clear_command_line();
                editor.set_mode(Mode::Command);
            }
            // Enter Search mode (forward)
            KeyCode::Char('/') => {
                editor.clear_search_buffer();
                editor.set_search_forward(true);
                editor.set_mode(Mode::Search);
            }
            // Enter Search mode (backward)
            KeyCode::Char('?') => {
                editor.clear_search_buffer();
                editor.set_search_forward(false);
                editor.set_mode(Mode::Search);
            }
            // Search next
            KeyCode::Char('n') => {
                editor.search_next();
            }
            // Search previous
            KeyCode::Char('N') => {
                editor.search_prev();
            }
            // Search for word under cursor (forward)
            KeyCode::Char('*') => {
                if let Some((word, _, _)) = editor.buffer().word_under_cursor() {
                    // Create search pattern with word boundaries
                    let pattern = format!(r"\b{}\b", regex::escape(&word));
                    let mut search = Search::new(pattern, true);
                    let cursor = editor.buffer().cursor();

                    // Find next occurrence (skip current one)
                    if let Some((line, col, _)) = search.find_next(editor.buffer(), cursor.line(), cursor.col() + 1) {
                        editor.buffer_mut().cursor_mut().set_position(line, col);
                    }
                    editor.set_current_search(search);
                }
            }
            // Search for word under cursor (backward)
            KeyCode::Char('#') => {
                if let Some((word, _, _)) = editor.buffer().word_under_cursor() {
                    // Create search pattern with word boundaries
                    let pattern = format!(r"\b{}\b", regex::escape(&word));
                    let mut search = Search::new(pattern, false);
                    let cursor = editor.buffer().cursor();

                    // Find previous occurrence
                    let search_col = if cursor.col() > 0 { cursor.col() - 1 } else { 0 };
                    if let Some((line, col, _)) = search.find_next(editor.buffer(), cursor.line(), search_col) {
                        editor.buffer_mut().cursor_mut().set_position(line, col);
                    }
                    editor.set_current_search(search);
                }
            }
            // Set mark (m followed by letter)
            KeyCode::Char('m') => {
                editor.set_pending_command('m');
            }
            // Jump to mark line (' followed by letter)
            KeyCode::Char('\'') => {
                editor.set_pending_command('\'');
            }
            // Jump to mark exact position (` followed by letter)
            KeyCode::Char('`') => {
                editor.set_pending_command('`');
            }
            // Start/stop macro recording (q followed by register, or q to stop)
            KeyCode::Char('q') => {
                if editor.is_recording_macro() {
                    // Stop recording
                    editor.stop_macro_recording();
                } else {
                    // Start recording - set pending command to wait for register
                    editor.set_pending_command('q');
                }
            }
            // Play back macro (@ followed by register)
            KeyCode::Char('@') => {
                editor.set_pending_command('@');
            }
            // Repeat last change
            KeyCode::Char('.') => {
                editor.repeat_last_change();
                editor.clear_count();
            }
            // Enter Visual mode
            KeyCode::Char('v') => {
                let cursor = editor.buffer().cursor();
                editor.set_visual_start(cursor.line(), cursor.col());
                editor.set_mode(Mode::Visual);
            }
            KeyCode::Char('V') => {
                let cursor = editor.buffer().cursor();
                editor.set_visual_start(cursor.line(), 0);
                editor.set_mode(Mode::VisualLine);
            }
            // Leader key (Space)
            KeyCode::Char(' ') => {
                editor.set_pending_leader(true);
            }
            // Word motions
            KeyCode::Char('w') => {
                let count = editor.effective_count();
                Motions::word_forward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('W') => {
                let count = editor.effective_count();
                Motions::word_forward_big(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('b') => {
                let count = editor.effective_count();
                Motions::word_backward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('B') => {
                let count = editor.effective_count();
                Motions::word_backward_big(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('e') => {
                let count = editor.effective_count();
                Motions::word_end_forward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('E') => {
                let count = editor.effective_count();
                Motions::word_end_forward_big(editor.buffer_mut(), count);
                editor.clear_count();
            }
            // File motions
            KeyCode::Char('g') => {
                // Set pending command to wait for second 'g'
                editor.set_pending_command('g');
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
                editor.buffer_mut().cursor_mut().set_position(target_line, 0);
                editor.clear_count();
            }
            // Find character motions
            KeyCode::Char('f') => {
                // f{char} - find next occurrence of char on line
                editor.set_pending_command('f');
            }
            KeyCode::Char('F') => {
                // F{char} - find previous occurrence of char on line
                editor.set_pending_command('F');
            }
            KeyCode::Char('t') => {
                // t{char} - till next occurrence (cursor before char)
                editor.set_pending_command('t');
            }
            KeyCode::Char('T') => {
                // T{char} - till previous occurrence (cursor after char)
                editor.set_pending_command('T');
            }
            KeyCode::Char(';') => {
                // ; - repeat last f/F/t/T motion
                if let Some((ch, find_type, direction)) = editor.get_last_find() {
                    let count = editor.effective_count();
                    match (find_type, direction) {
                        (FindType::Find, FindDirection::Forward) => {
                            Motions::find_char_forward(editor.buffer_mut(), ch, count);
                        }
                        (FindType::Find, FindDirection::Backward) => {
                            Motions::find_char_backward(editor.buffer_mut(), ch, count);
                        }
                        (FindType::Till, FindDirection::Forward) => {
                            Motions::till_char_forward(editor.buffer_mut(), ch, count);
                        }
                        (FindType::Till, FindDirection::Backward) => {
                            Motions::till_char_backward(editor.buffer_mut(), ch, count);
                        }
                    }
                }
                editor.clear_count();
            }
            KeyCode::Char(',') => {
                // , - repeat last f/F/t/T motion in opposite direction
                if let Some((ch, find_type, direction)) = editor.get_last_find() {
                    let count = editor.effective_count();
                    // Reverse the direction
                    let opposite_direction = match direction {
                        FindDirection::Forward => FindDirection::Backward,
                        FindDirection::Backward => FindDirection::Forward,
                    };
                    match (find_type, opposite_direction) {
                        (FindType::Find, FindDirection::Forward) => {
                            Motions::find_char_forward(editor.buffer_mut(), ch, count);
                        }
                        (FindType::Find, FindDirection::Backward) => {
                            Motions::find_char_backward(editor.buffer_mut(), ch, count);
                        }
                        (FindType::Till, FindDirection::Forward) => {
                            Motions::till_char_forward(editor.buffer_mut(), ch, count);
                        }
                        (FindType::Till, FindDirection::Backward) => {
                            Motions::till_char_backward(editor.buffer_mut(), ch, count);
                        }
                    }
                }
                editor.clear_count();
            }
            // Jump to matching bracket
            KeyCode::Char('%') => {
                Motions::jump_to_matching_bracket(editor.buffer_mut());
                editor.clear_count();
            }
            // Paragraph motions
            KeyCode::Char('}') => {
                let count = editor.effective_count();
                Motions::paragraph_forward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('{') => {
                let count = editor.effective_count();
                Motions::paragraph_backward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            // Sentence motions
            KeyCode::Char(')') => {
                let count = editor.effective_count();
                Motions::sentence_forward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('(') => {
                let count = editor.effective_count();
                Motions::sentence_backward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            // Operators
            KeyCode::Char('d') => {
                editor.set_pending_operator(Operator::Delete);
            }
            KeyCode::Char('y') => {
                editor.set_pending_operator(Operator::Yank);
            }
            KeyCode::Char('c') => {
                editor.set_pending_operator(Operator::Change);
            }
            KeyCode::Char('>') => {
                editor.set_pending_operator(Operator::Indent);
            }
            KeyCode::Char('<') => {
                editor.set_pending_operator(Operator::Dedent);
            }
            // Simple delete commands
            KeyCode::Char('x') => {
                // x - delete character under cursor
                let count = editor.effective_count();
                let cursor = editor.buffer().cursor();
                let cursor_before = (cursor.line(), cursor.col());
                let line_idx = cursor.line();
                let col = cursor.col();

                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_text = line.trim_end_matches('\n');
                    let chars_count = line_text.chars().count();

                    if col < chars_count {
                        let end_col = (col + count).min(chars_count);
                        let start_pos = (line_idx, col);
                        let end_pos = (line_idx, end_col);

                        let deleted = editor.buffer_mut().delete_range(line_idx, col, line_idx, end_col);
                        let range = Range::new(start_pos, end_pos);
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.registers_mut().delete(deleted);
                        editor.add_change(change);

                        // Clamp cursor to buffer bounds
                        Self::clamp_cursor_to_buffer(editor);
                    }
                }
                editor.clear_count();
            }
            KeyCode::Char('X') => {
                // X - delete character before cursor
                let count = editor.effective_count();
                let cursor = editor.buffer().cursor();
                let cursor_before = (cursor.line(), cursor.col());
                let line_idx = cursor.line();
                let col = cursor.col();

                if col > 0 {
                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let chars_count = line_text.chars().count();

                        // Calculate start column (delete count chars before cursor)
                        let start_col = col.saturating_sub(count);
                        let start_pos = (line_idx, start_col);
                        let end_pos = (line_idx, col);

                        let deleted = editor.buffer_mut().delete_range(line_idx, start_col, line_idx, col);
                        let range = Range::new(start_pos, end_pos);
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.registers_mut().delete(deleted);
                        editor.add_change(change);

                        // Move cursor to the start of deletion
                        editor.buffer_mut().cursor_mut().set_col(start_col);

                        // Clamp cursor to buffer bounds
                        Self::clamp_cursor_to_buffer(editor);
                    }
                }
                editor.clear_count();
            }
            KeyCode::Char('D') => {
                // D - delete to end of line
                let cursor = editor.buffer().cursor();
                let cursor_before = (cursor.line(), cursor.col());
                let line_idx = cursor.line();
                let col = cursor.col();

                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_text = line.trim_end_matches('\n');
                    let line_len = line_text.chars().count();

                    if col < line_len {
                        let start_pos = (line_idx, col);
                        let end_pos = (line_idx, line_len);

                        let deleted = editor.buffer_mut().delete_range(line_idx, col, line_idx, line_len);
                        let range = Range::new(start_pos, end_pos);
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.registers_mut().delete(deleted);
                        editor.add_change(change);

                        // Clamp cursor to buffer bounds
                        Self::clamp_cursor_to_buffer(editor);
                    }
                }
                editor.clear_count();
            }
            KeyCode::Char('C') => {
                // C - change to end of line
                let cursor = editor.buffer().cursor();
                let cursor_before = (cursor.line(), cursor.col());
                let line_idx = cursor.line();
                let col = cursor.col();

                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_text = line.trim_end_matches('\n');
                    let line_len = line_text.chars().count();

                    if col < line_len {
                        let start_pos = (line_idx, col);
                        let end_pos = (line_idx, line_len);

                        let deleted = editor.buffer_mut().delete_range(line_idx, col, line_idx, line_len);
                        let range = Range::new(start_pos, end_pos);
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.registers_mut().delete(deleted);
                        editor.add_change(change);

                        // Clamp cursor to buffer bounds
                        Self::clamp_cursor_to_buffer(editor);
                    }
                }
                editor.clear_count();
                let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                editor.start_change_building(cursor_before); // C enters insert mode, start building
                editor.set_mode(Mode::Insert);
            }
            // Paste
            KeyCode::Char('p') => {
                // p - paste after cursor
                Self::paste_after(editor)?;
                editor.clear_count();
            }
            KeyCode::Char('P') => {
                // P - paste before cursor
                Self::paste_before(editor)?;
                editor.clear_count();
            }
            // Join lines
            KeyCode::Char('J') => {
                // J - join current line with next line
                let count = editor.effective_count();
                Self::join_lines(editor, count)?;
                editor.clear_count();
            }
            // Substitute
            KeyCode::Char('s') => {
                // s - substitute character(s) under cursor
                let count = editor.effective_count();
                let cursor = editor.buffer().cursor();
                let cursor_before = (cursor.line(), cursor.col());
                let line_idx = cursor.line();
                let col = cursor.col();

                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_text = line.trim_end_matches('\n');
                    let chars_count = line_text.chars().count();

                    if col < chars_count {
                        let end_col = (col + count).min(chars_count);
                        let start_pos = (line_idx, col);
                        let end_pos = (line_idx, end_col);

                        let deleted = editor.buffer_mut().delete_range(line_idx, col, line_idx, end_col);
                        let range = Range::new(start_pos, end_pos);
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.registers_mut().delete(deleted);
                        editor.add_change(change);

                        // Clamp cursor to buffer bounds
                        Self::clamp_cursor_to_buffer(editor);
                    }
                }
                editor.clear_count();
                let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
            }
            KeyCode::Char('S') => {
                // S - substitute entire line
                let cursor = editor.buffer().cursor();
                let cursor_before = (cursor.line(), cursor.col());
                let start_line = cursor.line();
                let count = editor.effective_count();
                let end_line = (start_line + count).min(editor.buffer().line_count());

                let start_pos = (start_line, 0);
                let end_pos = (end_line, 0);

                let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line, 0);
                let range = Range::new(start_pos, end_pos);
                let change = Change::delete(range, deleted.clone(), cursor_before);

                editor.registers_mut().delete(deleted);
                editor.add_change(change);
                editor.clear_count();
                let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                Self::insert_line_above(editor)?;
            }
            // Toggle case
            KeyCode::Char('~') => {
                // ~ - toggle case of character under cursor
                Self::toggle_case_at_cursor(editor)?;
                editor.clear_count();
            }
            // Undo/Redo
            KeyCode::Char('u') => {
                // u - undo
                editor.undo();
                editor.clear_count();
            }
            KeyCode::Char('r') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+R - redo
                editor.redo();
                editor.clear_count();
            }
            _ => {
                // Clear count on unrecognized key
                editor.clear_count();
            }
        }
        Ok(())
    }

    /// Handles input in Insert mode
    fn handle_insert_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Esc => {
                editor.finalize_change_building();
                editor.mark_buffer_modified(); // Mark for LSP didChange notification
                editor.set_mode(Mode::Normal);
                // Move cursor left when exiting insert mode (unless at column 0)
                let cursor = editor.buffer_mut().cursor_mut();
                if cursor.col() > 0 {
                    cursor.move_left(1);
                }
            }
            // Ctrl-W - Delete word backward
            KeyCode::Char('w') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                Self::delete_word_backward_insert(editor)?;
            }
            // Ctrl-U - Delete to start of line
            KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                Self::delete_to_line_start_insert(editor)?;
            }
            KeyCode::Char(c) => {
                Self::insert_char(editor, c)?;
            }
            KeyCode::Enter => {
                Self::insert_newline(editor)?;
            }
            KeyCode::Backspace => {
                Self::delete_char_before_cursor(editor)?;
            }
            KeyCode::Left => {
                let cursor = editor.buffer_mut().cursor_mut();
                if cursor.col() > 0 {
                    cursor.move_left(1);
                }
            }
            KeyCode::Right => {
                Self::move_right(editor);
            }
            KeyCode::Up => {
                Self::move_up(editor);
            }
            KeyCode::Down => {
                Self::move_down(editor);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles input in Visual mode
    fn handle_visual_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Esc => {
                editor.clear_visual_start();
                editor.set_mode(Mode::Normal);
            }
            // Motion keys work in visual mode too
            KeyCode::Char('h') | KeyCode::Left => {
                Self::move_left(editor);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                Self::move_down(editor);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                Self::move_up(editor);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                Self::move_right(editor);
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
                editor.buffer_mut().cursor_mut().set_col(0);
            }
            KeyCode::Char('$') => {
                let line_idx = editor.buffer().cursor().line();
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_len = line.trim_end_matches('\n').chars().count();
                    let col = if line_len > 0 { line_len - 1 } else { 0 };
                    editor.buffer_mut().cursor_mut().set_col(col);
                }
            }
            // Delete selection
            KeyCode::Char('d') | KeyCode::Char('x') => {
                Self::delete_visual_selection(editor)?;
                editor.clear_visual_start();
                editor.set_mode(Mode::Normal);
            }
            // Yank selection
            KeyCode::Char('y') => {
                Self::yank_visual_selection(editor)?;
                editor.clear_visual_start();
                editor.set_mode(Mode::Normal);
            }
            // Change selection
            KeyCode::Char('c') => {
                Self::delete_visual_selection(editor)?;
                editor.clear_visual_start();
                editor.set_mode(Mode::Insert);
            }
            // Move to other end of selection
            KeyCode::Char('o') => {
                if let Some(visual_start) = editor.visual_start() {
                    let cursor = editor.buffer().cursor();
                    let cursor_pos = (cursor.line(), cursor.col());

                    // Swap cursor and visual_start
                    editor.buffer_mut().cursor_mut().set_position(visual_start.0, visual_start.1);
                    editor.set_visual_start(cursor_pos.0, cursor_pos.1);
                }
            }
            // Switch to other visual modes
            KeyCode::Char('v') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.set_mode(Mode::VisualBlock);
            }
            KeyCode::Char('v') => {
                editor.set_mode(Mode::Visual);
            }
            KeyCode::Char('V') => {
                let cursor = editor.buffer().cursor();
                editor.set_visual_start(cursor.line(), 0);
                editor.set_mode(Mode::VisualLine);
            }
            // Indent/dedent in visual mode
            KeyCode::Char('>') => {
                if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let tab_width = 4;

                    Self::indent_lines_with_tracking(editor, start_line, end_line + 1, tab_width, cursor_before)?;
                }
                editor.clear_visual_start();
                editor.set_mode(Mode::Normal);
            }
            KeyCode::Char('<') => {
                if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let tab_width = 4;

                    Self::dedent_lines_with_tracking(editor, start_line, end_line + 1, tab_width, cursor_before)?;
                }
                editor.clear_visual_start();
                editor.set_mode(Mode::Normal);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles input in Command mode
    fn handle_command_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Char(ch) => {
                // Add character to command line
                editor.append_to_command_line(ch);
            }
            KeyCode::Backspace => {
                // Remove last character from command line
                editor.backspace_command_line();
            }
            KeyCode::Enter => {
                // Execute the command
                Self::execute_command(editor)?;
                editor.clear_command_line();
                editor.set_mode(Mode::Normal);
            }
            KeyCode::Esc => {
                // Cancel command mode
                editor.clear_command_line();
                editor.set_mode(Mode::Normal);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles substitute command (:s/pattern/replacement/flags)
    fn handle_substitute_command(editor: &mut Editor, command: &str) -> Result<()> {
        // Parse the command to extract range, pattern, replacement, and flags
        // Supported formats:
        // :s/pattern/replacement/[flags]
        // :%s/pattern/replacement/[flags]
        // :'<,'>s/pattern/replacement/[flags]
        // :1,5s/pattern/replacement/[flags]

        let (range_str, substitute_part) = if let Some(s_idx) = command.rfind('s') {
            (&command[..s_idx], &command[s_idx+1..])
        } else {
            return Ok(()); // No 's' found
        };

        // Parse substitute pattern: /pattern/replacement/flags
        if !substitute_part.starts_with('/') {
            return Ok(()); // Invalid format
        }

        let parts: Vec<&str> = substitute_part.splitn(4, '/').collect();
        if parts.len() < 3 {
            return Ok(()); // Invalid format - need at least /pattern/replacement/
        }

        let pattern = parts[1];
        let replacement = parts[2];
        let flags = if parts.len() >= 4 { parts[3] } else { "" };

        // Parse flags
        let global = flags.contains('g');
        let _ignore_case = flags.contains('i');

        // Determine the range
        let (start_line, end_line) = if range_str.is_empty() {
            // :s - current line only
            let cursor_line = editor.buffer().cursor().line();
            (cursor_line, cursor_line + 1)
        } else if range_str == "%" {
            // :%s - all lines
            (0, editor.buffer().line_count())
        } else if range_str == "'<,'>" || range_str.contains("'<") {
            // :'<,'>s - visual selection
            if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
                (start_line, end_line + 1)
            } else {
                return Ok(());
            }
        } else {
            // Try to parse line range like 1,5 or 1,$
            return Ok(()); // For now, skip complex range parsing
        };

        // Perform substitution with change tracking
        let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());

        for line_idx in start_line..end_line.min(editor.buffer().line_count()) {
            if let Some(line) = editor.buffer().line(line_idx) {
                let line_text = line.trim_end_matches('\n');

                // Perform the substitution
                let new_text = if global {
                    // Replace all occurrences
                    line_text.replace(pattern, replacement)
                } else {
                    // Replace first occurrence
                    if let Some(pos) = line_text.find(pattern) {
                        let mut result = String::new();
                        result.push_str(&line_text[..pos]);
                        result.push_str(replacement);
                        result.push_str(&line_text[pos + pattern.len()..]);
                        result
                    } else {
                        line_text.to_string()
                    }
                };

                if new_text != line_text {
                    // Delete old line content and insert new
                    let line_len = line_text.chars().count();
                    let deleted = editor.buffer_mut().delete_range(line_idx, 0, line_idx, line_len);
                    let delete_range = Range::new((line_idx, 0), (line_idx, line_len));
                    let delete_change = Change::delete(delete_range, deleted, cursor_before);

                    let insert_change = Change::insert((line_idx, 0), new_text, cursor_before);
                    insert_change.apply(editor.buffer_mut());

                    editor.add_change(delete_change);
                    editor.add_change(insert_change);
                }
            }
        }

        Ok(())
    }

    /// Executes a command from the command line
    fn execute_command(editor: &mut Editor) -> Result<()> {
        let command = editor.command_line().trim().to_string();

        // Handle commands with arguments
        if command.starts_with("e ") || command.starts_with("edit ") {
            // :e <filename> or :edit <filename>
            let parts: Vec<&str> = command.split_whitespace().collect();
            if parts.len() >= 2 {
                let filename = parts[1..].join(" ");
                editor.load_file(&filename)?;
            }
            return Ok(());
        }

        if command.starts_with("w ") || command.starts_with("write ") {
            // :w <filename> or :write <filename> - save as
            let parts: Vec<&str> = command.split_whitespace().collect();
            if parts.len() >= 2 {
                let filename = parts[1..].join(" ");
                editor.buffer_mut().save_as(&filename)?;
                editor.mark_saved();
            }
            return Ok(());
        }

        // Handle substitute command (:s, :%s, :'<,'>s)
        // Check if it's a substitute command (contains 's/' pattern)
        if command.ends_with("s/") || command.contains("s/") {
            Self::handle_substitute_command(editor, &command)?;
            return Ok(());
        }

        // Handle commands without arguments
        match command.as_str() {
            "q" | "quit" => {
                // Quit without checking for modifications
                if editor.is_modified() {
                    // In a real editor, we'd show an error message
                    // For now, just don't quit if modified
                    return Ok(());
                }
                editor.quit();
            }
            "q!" | "quit!" => {
                // Force quit without saving
                editor.quit();
            }
            "w" | "write" => {
                // Save to current file
                editor.buffer_mut().save()?;
                editor.mark_saved();
            }
            "wq" | "x" => {
                // Write and quit
                editor.buffer_mut().save()?;
                editor.mark_saved();
                editor.quit();
            }
            "wq!" => {
                // Force write and quit
                editor.buffer_mut().save()?;
                editor.mark_saved();
                editor.quit();
            }
            "noh" | "nohl" | "nohlsearch" => {
                // Clear search highlighting
                editor.clear_search_highlight();
            }
            _ => {
                // Unknown command - for now just ignore
            }
        }

        Ok(())
    }

    /// Handles input in Search mode
    fn handle_search_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Char(ch) => {
                // Add character to search buffer
                editor.append_to_search_buffer(ch);
            }
            KeyCode::Backspace => {
                // Remove last character from search buffer
                editor.backspace_search_buffer();
            }
            KeyCode::Enter => {
                // Execute the search
                editor.execute_search();
                editor.set_mode(Mode::Normal);
            }
            KeyCode::Esc => {
                // Cancel search mode
                editor.clear_search_buffer();
                editor.set_mode(Mode::Normal);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles input in Replace mode
    fn handle_replace_mode(_editor: &mut Editor, _key_event: KeyEvent) -> Result<()> {
        // Placeholder for replace mode
        Ok(())
    }

    /// Handles input in Picker mode
    fn handle_picker_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            // Escape - cancel picker
            KeyCode::Esc => {
                editor.close_picker();
                editor.set_mode(Mode::Normal);
            }
            // Enter - select current item
            KeyCode::Enter => {
                if let Some(picker) = editor.picker() {
                    if let Some(result) = picker.selected_result() {
                        // Load file and jump to location
                        let location = result.location.clone();
                        let line = result.line;
                        let col = result.col;

                        // Close picker first
                        editor.close_picker();
                        editor.set_mode(Mode::Normal);

                        // Load the file
                        if let Err(e) = editor.load_file(&location) {
                            eprintln!("Failed to load file {}: {}", location, e);
                            return Ok(());
                        }

                        // Jump to line/col
                        editor.buffer_mut().cursor_mut().set_position(line, col);
                    }
                } else {
                    // No picker, return to normal
                    editor.set_mode(Mode::Normal);
                }
            }
            // Backspace - remove character from query
            KeyCode::Backspace => {
                if let Some(picker) = editor.picker_mut() {
                    picker.backspace_query();
                }
            }
            // Ctrl-N - move down
            KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(picker) = editor.picker_mut() {
                    picker.move_down();
                }
            }
            // Ctrl-P - move up
            KeyCode::Char('p') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(picker) = editor.picker_mut() {
                    picker.move_up();
                }
            }
            // Down arrow - move down
            KeyCode::Down => {
                if let Some(picker) = editor.picker_mut() {
                    picker.move_down();
                }
            }
            // Up arrow - move up
            KeyCode::Up => {
                if let Some(picker) = editor.picker_mut() {
                    picker.move_up();
                }
            }
            // Any other character - add to query
            KeyCode::Char(ch) => {
                if let Some(picker) = editor.picker_mut() {
                    picker.append_query(ch);
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Polls for the next event
    pub fn poll_event() -> Result<Option<Event>> {
        if event::poll(std::time::Duration::from_millis(100))? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }

    // Helper methods for cursor movement and editing

    fn move_left(editor: &mut Editor) {
        let count = editor.effective_count();
        let cursor = editor.buffer_mut().cursor_mut();
        if cursor.col() >= count {
            cursor.move_left(count);
        } else {
            cursor.set_col(0);
        }
        editor.clear_count();
    }

    fn move_right(editor: &mut Editor) {
        let count = editor.effective_count();
        let line_idx = editor.buffer().cursor().line();
        if let Some(line) = editor.buffer().line(line_idx) {
            let line_len = line.trim_end_matches('\n').chars().count();
            let cursor = editor.buffer_mut().cursor_mut();
            let new_col = (cursor.col() + count).min(line_len.saturating_sub(1).max(0));
            cursor.set_col(new_col);
        }
        editor.clear_count();
    }

    fn move_up(editor: &mut Editor) {
        let count = editor.effective_count();
        let cursor = editor.buffer_mut().cursor_mut();
        cursor.move_up(count);
        Self::clamp_cursor_to_line(editor);
        editor.clear_count();
    }

    fn move_down(editor: &mut Editor) {
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
        Self::clamp_cursor_to_line(editor);
        editor.clear_count();
    }

    fn clamp_cursor_to_line(editor: &mut Editor) {
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

    fn insert_char(editor: &mut Editor, c: char) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let position = (cursor.line(), cursor.col());

        // Create and apply the change
        let change = Change::insert(position, c.to_string(), cursor_before);
        change.apply(editor.buffer_mut());
        editor.add_change(change);

        Ok(())
    }

    fn insert_newline(editor: &mut Editor) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let position = (cursor.line(), cursor.col());

        // Create and apply the change
        let change = Change::insert(position, "\n".to_string(), cursor_before);
        change.apply(editor.buffer_mut());
        editor.add_change(change);

        Ok(())
    }

    fn delete_char_before_cursor(editor: &mut Editor) -> Result<()> {
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
            let prev_line_len = editor.buffer().line(line_idx - 1)
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
            let deleted_char = editor.buffer().rope()
                .get_char(delete_pos)
                .unwrap_or(' ');
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

    fn delete_word_backward_insert(editor: &mut Editor) -> Result<()> {
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
            let prev_line_len = editor.buffer().line(line_idx - 1)
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
        while start_col > 0 && chars.get(start_col - 1).map_or(false, |c| c.is_whitespace()) {
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
                        while start_col > 0 && chars.get(start_col - 1).map_or(false, |&c| is_word_char(c)) {
                            start_col -= 1;
                        }
                    } else {
                        // Delete punctuation/special characters
                        while start_col > 0 && chars.get(start_col - 1).map_or(false, |&c| !is_word_char(c) && !c.is_whitespace()) {
                            start_col -= 1;
                        }
                    }
                }
            }
        }

        // Delete the range
        if start_col < col {
            let deleted = editor.buffer_mut().delete_range(line_idx, start_col, line_idx, col);
            let range = Range::new((line_idx, start_col), (line_idx, col));
            let change = Change::delete(range, deleted, cursor_before);
            editor.add_change(change);
        }

        Ok(())
    }

    fn delete_to_line_start_insert(editor: &mut Editor) -> Result<()> {
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

    fn insert_line_below(editor: &mut Editor) -> Result<()> {
        let line_idx = editor.buffer().cursor().line();
        let line_start = editor.buffer().rope().line_to_char(line_idx);
        let line_len = editor.buffer().rope().line(line_idx).len_chars();
        let insert_pos = line_start + line_len;

        // Get indentation from current line
        let line_text = editor.buffer().line(line_idx).unwrap_or_default();
        let indent = line_text.chars()
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
        let final_insert_pos = if added_newline { insert_pos + 1 } else { insert_pos };
        editor.buffer_mut().rope_mut().insert(final_insert_pos, &text_to_insert);

        // Position cursor at end of indentation on new line
        editor.buffer_mut().cursor_mut().set_position(line_idx + 1, indent.len());
        Ok(())
    }

    fn insert_line_above(editor: &mut Editor) -> Result<()> {
        let line_idx = editor.buffer().cursor().line();
        let line_start = editor.buffer().rope().line_to_char(line_idx);

        // Get indentation from current line
        let line_text = editor.buffer().line(line_idx).unwrap_or_default();
        let indent = line_text.chars()
            .take_while(|c| c.is_whitespace() && *c != '\n')
            .collect::<String>();

        // Insert indented line above
        let text_to_insert = format!("{}\n", indent);
        editor.buffer_mut().rope_mut().insert(line_start, &text_to_insert);

        // Cursor stays at same line index, positioned at end of indentation
        editor.buffer_mut().cursor_mut().set_col(indent.len());
        Ok(())
    }

    fn paste_after(editor: &mut Editor) -> Result<()> {
        let text = editor.registers().get_default().to_string();
        if text.is_empty() {
            return Ok(());
        }

        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        // Check if text contains newline (line paste vs character paste)
        let position = if text.contains('\n') {
            // Line paste - insert after current line
            let line_len = editor.buffer().rope().line(line_idx).len_chars();
            // For line paste, we insert at the end of current line (after newline if exists)
            (line_idx, line_len)
        } else {
            // Character paste - insert after cursor (col + 1)
            (line_idx, col + 1)
        };

        // Create and apply the change
        let change = Change::insert(position, text, cursor_before);
        change.apply(editor.buffer_mut());
        editor.add_change(change);

        Ok(())
    }

    fn paste_before(editor: &mut Editor) -> Result<()> {
        let text = editor.registers().get_default().to_string();
        if text.is_empty() {
            return Ok(());
        }

        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        // Check if text contains newline (line paste vs character paste)
        let position = if text.contains('\n') {
            // Line paste - insert before current line (at start of line)
            (line_idx, 0)
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

    fn delete_visual_selection(editor: &mut Editor) -> Result<()> {
        let mode = editor.mode();
        let cursor_before = (editor.buffer().cursor().line(), editor.buffer().cursor().col());

        if let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() {
            match mode {
                Mode::VisualLine => {
                    // Delete entire lines
                    let start_pos = (start_line, 0);
                    let end_pos = (end_line + 1, 0);

                    let deleted = editor.buffer_mut().delete_range(
                        start_line, 0,
                        end_line + 1, 0
                    );

                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);
                    editor.add_change(change);
                    editor.registers_mut().delete(deleted);

                    // Position cursor at start of selection
                    let new_line = start_line.min(editor.buffer().line_count().saturating_sub(1));
                    editor.buffer_mut().cursor_mut().set_position(new_line, 0);
                }
                _ => {
                    // Character-wise visual mode
                    let start_pos = (start_line, start_col);
                    let end_pos = (end_line, end_col + 1);

                    let deleted = editor.buffer_mut().delete_range(
                        start_line, start_col,
                        end_line, end_col + 1
                    );

                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);
                    editor.add_change(change);
                    editor.registers_mut().delete(deleted);

                    // Position cursor at start of selection
                    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
                }
            }
        }

        Ok(())
    }

    fn yank_visual_selection(editor: &mut Editor) -> Result<()> {
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

                    let yanked = editor.buffer().rope().slice(start_char..end_char).to_string();
                    editor.registers_mut().yank(yanked);
                }
                _ => {
                    // Character-wise visual mode
                    let start_char = editor.buffer().rope().line_to_char(start_line) + start_col;
                    let end_char = editor.buffer().rope().line_to_char(end_line) + end_col + 1;

                    let yanked = editor.buffer().rope().slice(start_char..end_char).to_string();
                    editor.registers_mut().yank(yanked);
                }
            }
        }

        Ok(())
    }

    fn join_lines(editor: &mut Editor, count: usize) -> Result<()> {
        Self::join_lines_impl(editor, count, true)
    }

    fn join_lines_no_space(editor: &mut Editor, count: usize) -> Result<()> {
        Self::join_lines_impl(editor, count, false)
    }

    fn join_lines_impl(editor: &mut Editor, count: usize, add_space: bool) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let start_line = cursor.line();

        // Join count lines (minimum 1, which joins current with next)
        let lines_to_join = count.max(1);

        for _ in 0..lines_to_join {
            let line_idx = start_line;

            // Don't join if we're on the last line
            if line_idx >= editor.buffer().line_count().saturating_sub(1) {
                break;
            }

            if let Some(line) = editor.buffer().line(line_idx) {
                let line_text = line.trim_end_matches('\n');
                let line_len = line_text.chars().count();

                // Delete the newline at the end of the current line
                let start_pos = (line_idx, line_len);
                let end_pos = (line_idx + 1, 0);

                let deleted = editor.buffer_mut().delete_range(line_idx, line_len, line_idx + 1, 0);

                // Insert a space where the newline was (if requested)
                if add_space {
                    editor.buffer_mut().insert_text_at(line_idx, line_len, " ");
                }

                // Record the change
                let range = Range::new(start_pos, end_pos);
                let change = Change::delete(range, deleted, cursor_before);
                editor.add_change(change);
            }
        }

        // Position cursor at the join point
        if let Some(line) = editor.buffer().line(start_line) {
            let line_len = line.trim_end_matches('\n').chars().count();
            editor.buffer_mut().cursor_mut().set_position(start_line, line_len.saturating_sub(1).max(0));
        }

        Ok(())
    }

    fn indent_lines_with_tracking(editor: &mut Editor, start_line: usize, end_line: usize, tab_width: usize, cursor_before: (usize, usize)) -> Result<()> {
        for line_idx in start_line..end_line.min(editor.buffer().line_count()) {
            let indent_str = " ".repeat(tab_width);
            let change = Change::insert((line_idx, 0), indent_str.clone(), cursor_before);
            change.apply(editor.buffer_mut());
            editor.add_change(change);
        }
        Ok(())
    }

    fn dedent_lines_with_tracking(editor: &mut Editor, start_line: usize, end_line: usize, tab_width: usize, cursor_before: (usize, usize)) -> Result<()> {
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
                    let deleted = editor.buffer_mut().delete_range(line_idx, 0, line_idx, spaces_to_remove);
                    let range = Range::new((line_idx, 0), (line_idx, spaces_to_remove));
                    let change = Change::delete(range, deleted, cursor_before);
                    editor.add_change(change);
                }
            }
        }
        Ok(())
    }

    fn toggle_case_at_cursor(editor: &mut Editor) -> Result<()> {
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
                let deleted = editor.buffer_mut().delete_range(line_idx, col, line_idx, col + 1);
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
    fn change_case_line(editor: &mut Editor, count: usize, case_change: CaseChange) -> Result<()> {
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
                    let deleted = editor.buffer_mut().delete_range(line_idx, 0, line_idx, line_len);
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
    fn change_case_motion<F>(editor: &mut Editor, count: usize, case_change: CaseChange, motion: F) -> Result<()>
    where
        F: FnOnce(&mut Buffer, usize)
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
        let text = editor.buffer().rope().slice(start_char..end_char).to_string();

        // Transform the case
        let transformed = Self::apply_case_change(&text, &case_change);

        if transformed != text {
            // Delete the old text
            let deleted = editor.buffer_mut().delete_range(start_line, start_col, end_line, end_col);
            let delete_range = Range::new((start_line, start_col), (end_line, end_col));
            let delete_change = Change::delete(delete_range, deleted, cursor_before);

            // Insert the transformed text
            let insert_change = Change::insert((start_line, start_col), transformed, cursor_before);
            insert_change.apply(editor.buffer_mut());

            editor.add_change(delete_change);
            editor.add_change(insert_change);
        }

        // Reset cursor to start position
        editor.buffer_mut().cursor_mut().set_position(start_line, start_col);

        Ok(())
    }

    /// Changes case from cursor to end of line
    fn change_case_to_end_of_line(editor: &mut Editor, case_change: CaseChange) -> Result<()> {
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
                    let deleted = editor.buffer_mut().delete_range(line_idx, col, line_idx, line_len);
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
    fn apply_case_change(text: &str, case_change: &CaseChange) -> String {
        match case_change {
            CaseChange::Lowercase => text.to_lowercase(),
            CaseChange::Uppercase => text.to_uppercase(),
            CaseChange::Toggle => {
                text.chars().map(|ch| {
                    if ch.is_lowercase() {
                        ch.to_uppercase().to_string()
                    } else {
                        ch.to_lowercase().to_string()
                    }
                }).collect()
            }
        }
    }

    /// Increments the number under/after the cursor
    fn increment_number(editor: &mut Editor, count: usize) -> Result<()> {
        Self::modify_number(editor, count as i64)
    }

    /// Decrements the number under/after the cursor
    fn decrement_number(editor: &mut Editor, count: usize) -> Result<()> {
        Self::modify_number(editor, -(count as i64))
    }

    /// Modifies (increments or decrements) the number under/after the cursor
    fn modify_number(editor: &mut Editor, delta: i64) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let cursor_before = (cursor.line(), cursor.col());
        let line_idx = cursor.line();
        let col = cursor.col();

        if let Some(line) = editor.buffer().line(line_idx) {
            let line_text = line.trim_end_matches('\n');

            // Find number at or after cursor position
            if let Some((start_col, end_col, number_str)) = Self::find_number_at_or_after(line_text, col) {
                // Parse the number with base detection
                let (value, base, prefix_len) = Self::parse_number(&number_str)?;

                // Apply the delta
                let new_value = value.wrapping_add(delta);

                // Format the new number with the same base
                let new_number_str = Self::format_number(new_value, base, prefix_len);

                // Replace the number in the buffer
                let deleted = editor.buffer_mut().delete_range(line_idx, start_col, line_idx, end_col);
                let delete_range = Range::new((line_idx, start_col), (line_idx, end_col));
                let delete_change = Change::delete(delete_range, deleted, cursor_before);

                let insert_change = Change::insert((line_idx, start_col), new_number_str.clone(), cursor_before);
                insert_change.apply(editor.buffer_mut());

                editor.add_change(delete_change);
                editor.add_change(insert_change);

                // Position cursor on the first digit of the number
                editor.buffer_mut().cursor_mut().set_col(start_col);
            }
        }

        Ok(())
    }

    /// Finds a number at or after the given column position
    /// Returns (start_col, end_col, number_string)
    fn find_number_at_or_after(line: &str, col: usize) -> Option<(usize, usize, String)> {
        let chars: Vec<char> = line.chars().collect();

        // Start searching from cursor position
        let mut search_col = col;

        // Skip non-digit/non-hex characters to find start of number
        while search_col < chars.len() {
            let ch = chars[search_col];
            // Check if this could be the start of a number
            if ch.is_ascii_digit() || (search_col + 1 < chars.len() && ch == '0' &&
                (chars[search_col + 1] == 'x' || chars[search_col + 1] == 'X' ||
                 chars[search_col + 1] == 'b' || chars[search_col + 1] == 'B' ||
                 chars[search_col + 1] == 'o' || chars[search_col + 1] == 'O')) {
                break;
            }
            search_col += 1;
        }

        if search_col >= chars.len() {
            return None;
        }

        let start_col = search_col;
        let mut end_col = start_col;

        // Check for hex (0x), binary (0b), or octal (0o) prefix
        if chars[end_col] == '0' && end_col + 1 < chars.len() {
            let next = chars[end_col + 1];
            if next == 'x' || next == 'X' || next == 'b' || next == 'B' || next == 'o' || next == 'O' {
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

        // Regular decimal number
        end_col = start_col;
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
    fn parse_number(s: &str) -> Result<(i64, u32, usize)> {
        if s.len() >= 3 {
            let prefix = &s[0..2];
            let digits = &s[2..];

            match prefix {
                "0x" | "0X" => {
                    let value = i64::from_str_radix(digits, 16)
                        .unwrap_or(0);
                    return Ok((value, 16, 2));
                }
                "0b" | "0B" => {
                    let value = i64::from_str_radix(digits, 2)
                        .unwrap_or(0);
                    return Ok((value, 2, 2));
                }
                "0o" | "0O" => {
                    let value = i64::from_str_radix(digits, 8)
                        .unwrap_or(0);
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
    fn format_number(value: i64, base: u32, prefix_len: usize) -> String {
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
    fn clamp_cursor_to_buffer(editor: &mut Editor) {
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
}
