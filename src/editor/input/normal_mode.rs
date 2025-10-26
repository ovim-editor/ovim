//! Normal mode input handling

use super::helpers;
use crate::buffer::Buffer;
use crate::editor::{
    Change, Editor, FindDirection, FindType, Motions, Operator, Operators, Range, RegisterType,
    Search, TextObjects,
};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handles input in Normal mode
pub(super) fn handle_normal_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        // Hover is now handled in HoverWindow mode, no need to clear it here

        // Handle pending leader key sequences (e.g., <Space>sf, <Space>sg, <Space>ca)
        if editor.pending_leader() {
            editor.set_pending_leader(false);

            match key_event.code {
                KeyCode::Char('s') => {
                    // Expect 'f' or 'g' next
                    editor.set_pending_command('s');
                    return Ok(());
                }
                KeyCode::Char('c') => {
                    // Expect 'a' or 'i'/'o' next for code actions or call hierarchy
                    editor.set_pending_command('c');
                    return Ok(());
                }
                KeyCode::Char('o') => {
                    // <Space>o - Document outline (symbols)
                    editor.request_document_symbols();
                    return Ok(());
                }
                KeyCode::Char('S') => {
                    // <Space>S - Workspace symbols
                    editor.request_workspace_symbols();
                    return Ok(());
                }
                KeyCode::Char('t') => {
                    // Expect 'h' next for type hierarchy
                    editor.set_pending_command('t');
                    return Ok(());
                }
                KeyCode::Char('i') => {
                    // <Space>i - Organize imports
                    editor.request_organize_imports();
                    return Ok(());
                }
                KeyCode::Char('e') => {
                    // <Space>e - Toggle file tree explorer
                    editor.toggle_file_tree();
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
                    let base_dir =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let picker = crate::editor::Picker::new_file_finder(base_dir);
                    editor.set_picker(picker);
                    editor.set_mode(Mode::Picker);
                    editor.mark_picker_selection_changed();
                    return Ok(());
                }
                KeyCode::Char('g') => {
                    // <Space>sg - Live grep
                    let base_dir =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let picker = crate::editor::Picker::new_live_grep(base_dir);
                    editor.set_picker(picker);
                    editor.set_mode(Mode::Picker);
                    // Note: live grep starts empty, so no preview to preload
                    return Ok(());
                }
                _ => {
                    // Invalid sequence
                    return Ok(());
                }
            }
        }

        // Handle 'c' prefix for leader sequences (code actions, call hierarchy)
        if let Some('c') = editor.pending_command() {
            editor.clear_pending_command();

            match key_event.code {
                KeyCode::Char('a') => {
                    // <Space>ca - Code actions
                    editor.request_code_actions();
                    return Ok(());
                }
                KeyCode::Char('i') => {
                    // <Space>ci - Call hierarchy incoming
                    editor.request_call_hierarchy_incoming();
                    return Ok(());
                }
                KeyCode::Char('o') => {
                    // <Space>co - Call hierarchy outgoing
                    editor.request_call_hierarchy_outgoing();
                    return Ok(());
                }
                _ => {
                    // Invalid sequence
                    return Ok(());
                }
            }
        }

        // Handle 't' prefix for leader sequences (type hierarchy)
        if let Some('t') = editor.pending_command() {
            editor.clear_pending_command();

            match key_event.code {
                KeyCode::Char('h') => {
                    // <Space>th - Type hierarchy
                    editor.request_type_hierarchy();
                    return Ok(());
                }
                _ => {
                    // Invalid sequence
                    return Ok(());
                }
            }
        }

        // Handle pending operator + motion (like 'dw', 'dd', 'yy')
        // Skip this block if we have a pending text object prefix ('i' or 'a')
        // to allow text objects like di{ to be handled later
        let has_text_obj_prefix = matches!(editor.pending_command(), Some('i') | Some('a'));

        if !has_text_obj_prefix && editor.pending_operator().is_some() {
            let operator = editor.pending_operator().unwrap();
            let count = editor.effective_count();

            // K is not a motion, so operator+K should just cancel the operator
            if key_event.code == KeyCode::Char('K') {
                editor.clear_pending_operator();
                editor.clear_count();
                // Don't process K further - just cancel the operator
                return Ok(());
            }

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
                    let tab_width = editor.options.tab_width;

                    Self::indent_lines_with_tracking(
                        editor,
                        start_line,
                        end_line + 1,
                        tab_width,
                        cursor_before,
                    )?;
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
                    let tab_width = editor.options.tab_width;

                    Self::dedent_lines_with_tracking(
                        editor,
                        start_line,
                        end_line + 1,
                        tab_width,
                        cursor_before,
                    )?;
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

                    let deleted = editor
                        .buffer_mut()
                        .delete_range(start_line, 0, end_line + 1, 0);

                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);
                    editor.add_change(change);
                    editor.delete_to_register(deleted);

                    // Clamp cursor to buffer bounds
                    Self::clamp_cursor_to_buffer(editor);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Fold, KeyCode::Char('G')) => {
                    // zfG - fold from current line to end of file (or specified line)
                    editor.clear_pending_operator();
                    let start_line = editor.buffer().cursor().line();
                    let end_line = if let Some(cnt) = editor.count() {
                        cnt.saturating_sub(1)
                    } else {
                        editor.buffer().line_count().saturating_sub(1)
                    };
                    editor
                        .buffer_mut()
                        .fold_manager_mut()
                        .create_fold(start_line, end_line);
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
                (Operator::Fold, KeyCode::Char('g')) => {
                    // zfgg - fold from current line to first line
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
                        let tab_width = editor.options.tab_width;

                        Self::indent_lines_with_tracking(
                            editor,
                            start_line,
                            end_line + 1,
                            tab_width,
                            cursor_before,
                        )?;
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
                        let tab_width = editor.options.tab_width;

                        Self::dedent_lines_with_tracking(
                            editor,
                            start_line,
                            end_line + 1,
                            tab_width,
                            cursor_before,
                        )?;
                        editor.clear_count();
                        return Ok(());
                    }
                    (Operator::Fold, KeyCode::Char('g')) => {
                        // zfgg - fold from current line to first line (or specified line)
                        editor.clear_pending_operator();
                        editor.clear_pending_command();
                        let end_line = editor.buffer().cursor().line();
                        let start_line = if let Some(cnt) = editor.count() {
                            cnt.saturating_sub(1)
                        } else {
                            0
                        };
                        editor
                            .buffer_mut()
                            .fold_manager_mut()
                            .create_fold(start_line, end_line);
                        editor.clear_count();
                        return Ok(());
                    }
                    _ => {}
                }
            }

            // Don't clear pending operator if we have a text object prefix ('i' or 'a')
            // This allows text objects like 'dip', 'caw', etc. to work
            if !matches!(editor.pending_command(), Some('i') | Some('a')) {
                editor.clear_pending_operator();
            }

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
                    let (delete_start_line, delete_start_col) =
                        if end_line >= line_count && start_line > 0 {
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
                        delete_start_line,
                        delete_start_col,
                        end_line,
                        0,
                    );
                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.delete_to_register(deleted);
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

                    let deleted = editor
                        .buffer_mut()
                        .delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    // Position cursor at deletion start
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, start_col);

                    editor.delete_to_register(deleted);
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

                            let deleted = editor
                                .buffer_mut()
                                .delete_range(line_idx, col, line_idx, line_len);
                            let range = Range::new(start_pos, end_pos);
                            let change = Change::delete(range, deleted.clone(), cursor_before);

                            editor.delete_to_register(deleted);
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
                    editor.yank_to_register(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Yank, KeyCode::Char('w')) => {
                    // yw - yank word
                    let yanked = Operators::yank_word(editor.buffer_mut(), count)?;
                    editor.yank_to_register(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Yank, KeyCode::Char('$')) => {
                    // y$ - yank to end of line
                    let yanked = Operators::yank_to_end_of_line(editor.buffer())?;
                    editor.yank_to_register(yanked);
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

                    editor.delete_to_register(deleted);
                    editor.add_change(change);
                    editor.clear_count();
                    let cursor_before = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
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

                    let deleted = editor
                        .buffer_mut()
                        .delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new(start_pos, end_pos);
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    // Position cursor at deletion start
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, start_col);

                    editor.delete_to_register(deleted);
                    editor.add_change(change);

                    // Don't clamp cursor for c$ - we want to insert at end of line
                    editor.clear_count();
                    let insert_cursor = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
                    editor.start_change_building(insert_cursor);
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

                            let deleted = editor
                                .buffer_mut()
                                .delete_range(line_idx, col, line_idx, line_len);
                            let range = Range::new(start_pos, end_pos);
                            let change = Change::delete(range, deleted.clone(), cursor_before);

                            editor.delete_to_register(deleted);
                            editor.add_change(change);

                            // Don't clamp cursor - we want to insert at end of line (col position)
                            editor.clear_count();
                            let insert_cursor = (
                                editor.buffer().cursor().line(),
                                editor.buffer().cursor().col(),
                            );
                            editor.start_change_building(insert_cursor);
                            editor.set_mode(Mode::Insert);
                            return Ok(());
                        }
                    }
                    editor.clear_count();
                    let cursor_before = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
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
                // Replace with register operations
                (Operator::ReplaceWithRegister, KeyCode::Char('i')) => {
                    // gri - replace character under cursor with register, then insert mode
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let line_idx = cursor.line();
                    let col = cursor.col();

                    let register_content = editor.get_from_register();

                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        if col < line_text.chars().count() {
                            // Delete one character
                            let deleted =
                                editor
                                    .buffer_mut()
                                    .delete_range(line_idx, col, line_idx, col + 1);
                            let delete_range = Range::new((line_idx, col), (line_idx, col + 1));
                            let delete_change =
                                Change::delete(delete_range, deleted, cursor_before);

                            // Insert register content
                            let insert_change =
                                Change::insert((line_idx, col), register_content, cursor_before);
                            insert_change.apply(editor.buffer_mut());

                            editor.add_change(delete_change);
                            editor.add_change(insert_change);

                            // Enter insert mode at the position
                            editor.buffer_mut().cursor_mut().set_position(line_idx, col);
                        }
                    }
                    let cursor_after = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
                    editor.start_change_building(cursor_after);
                    editor.set_mode(Mode::Insert);
                    editor.clear_count();
                    editor.clear_pending_operator();
                    return Ok(());
                }
                (Operator::ReplaceWithRegister, KeyCode::Char('a')) => {
                    // gra - replace character under cursor with register, then append
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let line_idx = cursor.line();
                    let col = cursor.col();

                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        if col < line_text.chars().count() {
                            let register_content = editor.get_from_register();

                            // Delete one character
                            let deleted =
                                editor
                                    .buffer_mut()
                                    .delete_range(line_idx, col, line_idx, col + 1);
                            let delete_range = Range::new((line_idx, col), (line_idx, col + 1));
                            let delete_change =
                                Change::delete(delete_range, deleted, cursor_before);

                            // Insert register content
                            let insert_change = Change::insert(
                                (line_idx, col),
                                register_content.clone(),
                                cursor_before,
                            );
                            insert_change.apply(editor.buffer_mut());

                            editor.add_change(delete_change);
                            editor.add_change(insert_change);

                            // Enter insert mode after the replaced content
                            let new_col = col + register_content.chars().count();
                            editor
                                .buffer_mut()
                                .cursor_mut()
                                .set_position(line_idx, new_col);
                        }
                    }
                    let cursor_after = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
                    editor.start_change_building(cursor_after);
                    editor.set_mode(Mode::Insert);
                    editor.clear_count();
                    editor.clear_pending_operator();
                    return Ok(());
                }
                (Operator::ReplaceWithRegister, KeyCode::Char('I')) => {
                    // grI - replace at column 0, then insert mode
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let line_idx = cursor.line();

                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        if !line_text.is_empty() {
                            let register_content = editor.get_from_register();

                            // Delete first character
                            let deleted =
                                editor.buffer_mut().delete_range(line_idx, 0, line_idx, 1);
                            let delete_range = Range::new((line_idx, 0), (line_idx, 1));
                            let delete_change =
                                Change::delete(delete_range, deleted, cursor_before);

                            // Insert register content at column 0
                            let insert_change =
                                Change::insert((line_idx, 0), register_content, cursor_before);
                            insert_change.apply(editor.buffer_mut());

                            editor.add_change(delete_change);
                            editor.add_change(insert_change);

                            // Enter insert mode at column 0
                            editor.buffer_mut().cursor_mut().set_position(line_idx, 0);
                        }
                    }
                    let cursor_after = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
                    editor.start_change_building(cursor_after);
                    editor.set_mode(Mode::Insert);
                    editor.clear_count();
                    editor.clear_pending_operator();
                    return Ok(());
                }
                (Operator::ReplaceWithRegister, KeyCode::Char('A')) => {
                    // grA - replace at end of line, then insert mode
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let line_idx = cursor.line();

                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let line_len = line_text.chars().count();
                        if line_len > 0 {
                            let register_content = editor.get_from_register();
                            let last_col = line_len - 1;

                            // Delete last character
                            let deleted = editor
                                .buffer_mut()
                                .delete_range(line_idx, last_col, line_idx, line_len);
                            let delete_range =
                                Range::new((line_idx, last_col), (line_idx, line_len));
                            let delete_change =
                                Change::delete(delete_range, deleted, cursor_before);

                            // Insert register content
                            let insert_change = Change::insert(
                                (line_idx, last_col),
                                register_content.clone(),
                                cursor_before,
                            );
                            insert_change.apply(editor.buffer_mut());

                            editor.add_change(delete_change);
                            editor.add_change(insert_change);

                            // Enter insert mode after the replaced content
                            let new_col = last_col + register_content.chars().count();
                            editor
                                .buffer_mut()
                                .cursor_mut()
                                .set_position(line_idx, new_col);
                        }
                    }
                    let cursor_after = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
                    editor.start_change_building(cursor_after);
                    editor.set_mode(Mode::Insert);
                    editor.clear_count();
                    editor.clear_pending_operator();
                    return Ok(());
                }
                (Operator::ReplaceWithRegister, KeyCode::Char('r')) => {
                    // grr - replace line with register
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let line_idx = cursor.line();

                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let line_len = line_text.chars().count();
                        let register_content = editor.get_from_register();

                        if line_len > 0 {
                            let start_pos = (line_idx, 0);
                            let end_pos = (line_idx, line_len);

                            let deleted = editor
                                .buffer_mut()
                                .delete_range(line_idx, 0, line_idx, line_len);
                            let delete_range = Range::new(start_pos, end_pos);
                            let delete_change =
                                Change::delete(delete_range, deleted, cursor_before);

                            let insert_change =
                                Change::insert((line_idx, 0), register_content, cursor_before);
                            insert_change.apply(editor.buffer_mut());

                            editor.add_change(delete_change);
                            editor.add_change(insert_change);

                            // Position cursor at start of line
                            editor.buffer_mut().cursor_mut().set_position(line_idx, 0);
                        }
                    }
                    editor.clear_count();
                    editor.clear_pending_operator();
                    return Ok(());
                }
                (Operator::ReplaceWithRegister, KeyCode::Char('w')) => {
                    // grw - replace word with register
                    let start_cursor = editor.buffer().cursor().clone();
                    let cursor_before = (start_cursor.line(), start_cursor.col());
                    let start_line = start_cursor.line();
                    let start_col = start_cursor.col();

                    // Move cursor forward by word
                    Motions::word_forward(editor.buffer_mut(), count);

                    let end_cursor = editor.buffer().cursor();
                    let mut end_line = end_cursor.line();
                    let mut end_col = end_cursor.col();

                    // If we crossed a newline, stop at the end of the current line
                    if end_line > start_line {
                        if let Some(line) = editor.buffer().line(start_line) {
                            let line_text = line.trim_end_matches('\n');
                            end_line = start_line;
                            end_col = line_text.chars().count();
                        }
                    }

                    let register_content = editor.get_from_register();
                    let start_pos = (start_line, start_col);
                    let end_pos = (end_line, end_col);

                    let deleted = editor
                        .buffer_mut()
                        .delete_range(start_line, start_col, end_line, end_col);
                    let delete_range = Range::new(start_pos, end_pos);
                    let delete_change = Change::delete(delete_range, deleted, cursor_before);

                    let insert_change =
                        Change::insert((start_line, start_col), register_content, cursor_before);
                    insert_change.apply(editor.buffer_mut());

                    editor.add_change(delete_change);
                    editor.add_change(insert_change);

                    // Position cursor at start of replacement
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, start_col);

                    editor.clear_count();
                    editor.clear_pending_operator();
                    return Ok(());
                }
                (Operator::ReplaceWithRegister, KeyCode::Char('$')) => {
                    // gr$ - replace to end of line with register
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let line_idx = cursor.line();
                    let col = cursor.col();

                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let line_len = line_text.chars().count();

                        if col < line_len {
                            let register_content = editor.get_from_register();
                            let start_pos = (line_idx, col);
                            let end_pos = (line_idx, line_len);

                            let deleted = editor
                                .buffer_mut()
                                .delete_range(line_idx, col, line_idx, line_len);
                            let delete_range = Range::new(start_pos, end_pos);
                            let delete_change =
                                Change::delete(delete_range, deleted, cursor_before);

                            let insert_change =
                                Change::insert((line_idx, col), register_content, cursor_before);
                            insert_change.apply(editor.buffer_mut());

                            editor.add_change(delete_change);
                            editor.add_change(insert_change);
                        }
                    }
                    editor.clear_count();
                    editor.clear_pending_operator();
                    return Ok(());
                }
                // Count digits after operator (e.g., gr2w, d2w)
                (_, KeyCode::Char(c)) if c.is_ascii_digit() && c != '0' => {
                    let digit = c.to_digit(10).unwrap() as usize;
                    editor.append_count(digit);
                    editor.set_pending_operator(operator); // Restore operator
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

                    editor.delete_to_register(deleted);
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

                    editor.delete_to_register(deleted);
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
                    editor.yank_to_register(yanked);
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
                    editor.yank_to_register(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Fold, KeyCode::Char('j')) => {
                    // zfj - fold current line and line below
                    let start_line = editor.buffer().cursor().line();
                    let end_line =
                        (start_line + count).min(editor.buffer().line_count().saturating_sub(1));
                    editor
                        .buffer_mut()
                        .fold_manager_mut()
                        .create_fold(start_line, end_line);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Fold, KeyCode::Char('k')) => {
                    // zfk - fold current line and line above
                    let end_line = editor.buffer().cursor().line() + 1;
                    let start_line = editor.buffer().cursor().line().saturating_sub(count);
                    editor
                        .buffer_mut()
                        .fold_manager_mut()
                        .create_fold(start_line, end_line);
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

                    let deleted = editor
                        .buffer_mut()
                        .delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new((start_line, start_col), (end_line, end_col));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, start_col);
                    editor.delete_to_register(deleted);
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

                    let deleted = editor
                        .buffer_mut()
                        .delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new((start_line, start_col), (end_line, end_col));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.delete_to_register(deleted);
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
                        Motions::find_matching_bracket_forward(
                            &chars,
                            abs_start,
                            current_char,
                            matching_char,
                        )
                    } else {
                        Motions::find_matching_bracket_backward(
                            &chars,
                            abs_start,
                            matching_char,
                            current_char,
                        )
                    };

                    if let Some(abs_end) = match_abs_pos {
                        // Determine delete range (from min to max, inclusive)
                        let (delete_start, delete_end) = if abs_start < abs_end {
                            (abs_start, abs_end + 1)
                        } else {
                            (abs_end, abs_start + 1)
                        };

                        // Convert absolute positions to (line, col)
                        let (start_line, start_col) =
                            Motions::abs_pos_to_line_col(&rope, delete_start);
                        let (end_line, end_col) = Motions::abs_pos_to_line_col(&rope, delete_end);

                        // Delete the range
                        let deleted = editor
                            .buffer_mut()
                            .delete_range(start_line, start_col, end_line, end_col);
                        let range = Range::new((start_line, start_col), (end_line, end_col));
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor
                            .buffer_mut()
                            .cursor_mut()
                            .set_position(start_line, start_col);
                        editor.delete_to_register(deleted);
                        editor.add_change(change);
                        Self::clamp_cursor_to_buffer(editor);
                    }

                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Fold, KeyCode::Char('%')) => {
                    // zf% - fold to matching bracket
                    let cursor = editor.buffer().cursor();
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
                        Motions::find_matching_bracket_forward(
                            &chars,
                            abs_start,
                            current_char,
                            matching_char,
                        )
                    } else {
                        Motions::find_matching_bracket_backward(
                            &chars,
                            abs_start,
                            matching_char,
                            current_char,
                        )
                    };

                    if let Some(abs_end) = match_abs_pos {
                        // Convert absolute positions to (line, col)
                        let (fold_start_line, _) =
                            Motions::abs_pos_to_line_col(&rope, abs_start.min(abs_end));
                        let (fold_end_line, _) =
                            Motions::abs_pos_to_line_col(&rope, abs_start.max(abs_end));

                        // Create fold from start to end line
                        editor
                            .buffer_mut()
                            .fold_manager_mut()
                            .create_fold(fold_start_line, fold_end_line);
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

                    editor.yank_to_register(yanked);
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, start_col);
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
                                yanked.push_str(
                                    &chars[..=end_col.min(chars.len().saturating_sub(1))]
                                        .iter()
                                        .collect::<String>(),
                                );
                            } else {
                                yanked.push_str(&line);
                            }
                        }
                    }

                    editor.yank_to_register(yanked);
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(end_line, end_col);
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

                    let deleted = editor
                        .buffer_mut()
                        .delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new((start_line, start_col), (end_line, end_col));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, start_col);
                    editor.delete_to_register(deleted);
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

                    let deleted = editor
                        .buffer_mut()
                        .delete_range(start_line, start_col, end_line, end_col);
                    let range = Range::new((start_line, start_col), (end_line, end_col));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.delete_to_register(deleted);
                    editor.add_change(change);
                    editor.set_mode(Mode::Insert);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Fold, KeyCode::Char('}')) => {
                    // zf} - fold to next paragraph
                    let start_line = editor.buffer().cursor().line();
                    Motions::paragraph_forward(editor.buffer_mut(), count);
                    let end_line = editor.buffer().cursor().line();
                    editor
                        .buffer_mut()
                        .fold_manager_mut()
                        .create_fold(start_line, end_line);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Fold, KeyCode::Char('{')) => {
                    // zf{ - fold to previous paragraph
                    let end_line = editor.buffer().cursor().line();
                    Motions::paragraph_backward(editor.buffer_mut(), count);
                    let start_line = editor.buffer().cursor().line();
                    editor
                        .buffer_mut()
                        .fold_manager_mut()
                        .create_fold(start_line, end_line);
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
                    let tab_width = editor.options.tab_width;

                    Self::indent_lines_with_tracking(
                        editor,
                        start_line,
                        end_line,
                        tab_width,
                        cursor_before,
                    )?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Indent, KeyCode::Char('j')) | (Operator::Indent, KeyCode::Down) => {
                    // >j - indent current and next line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = start_line + count + 1;
                    let tab_width = editor.options.tab_width;

                    Self::indent_lines_with_tracking(
                        editor,
                        start_line,
                        end_line,
                        tab_width,
                        cursor_before,
                    )?;
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
                    let tab_width = editor.options.tab_width;

                    Self::indent_lines_with_tracking(
                        editor,
                        start_line,
                        end_line,
                        tab_width,
                        cursor_before,
                    )?;
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
                    let tab_width = editor.options.tab_width;

                    Self::dedent_lines_with_tracking(
                        editor,
                        start_line,
                        end_line,
                        tab_width,
                        cursor_before,
                    )?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Dedent, KeyCode::Char('j')) | (Operator::Dedent, KeyCode::Down) => {
                    // <j - dedent current and next line
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = start_line + count + 1;
                    let tab_width = editor.options.tab_width;

                    Self::dedent_lines_with_tracking(
                        editor,
                        start_line,
                        end_line,
                        tab_width,
                        cursor_before,
                    )?;
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
                    let tab_width = editor.options.tab_width;

                    Self::dedent_lines_with_tracking(
                        editor,
                        start_line,
                        end_line,
                        tab_width,
                        cursor_before,
                    )?;
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
            if (text_obj_type == 'i' || text_obj_type == 'a') && editor.pending_operator().is_some()
            {
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
                    KeyCode::Char('p') => {
                        // ip or ap - paragraph
                        if text_obj_type == 'i' {
                            TextObjects::inner_paragraph(editor.buffer())
                        } else {
                            TextObjects::around_paragraph(editor.buffer())
                        }
                    }
                    KeyCode::Char('s') => {
                        // is or as - sentence
                        if text_obj_type == 'i' {
                            TextObjects::inner_sentence(editor.buffer())
                        } else {
                            TextObjects::around_sentence(editor.buffer())
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
                        TextObjects::paired_delimiters(
                            editor.buffer(),
                            '(',
                            ')',
                            text_obj_type == 'a',
                        )
                    }
                    KeyCode::Char('[') | KeyCode::Char(']') => {
                        // i[ or a[ - brackets
                        TextObjects::paired_delimiters(
                            editor.buffer(),
                            '[',
                            ']',
                            text_obj_type == 'a',
                        )
                    }
                    KeyCode::Char('{') | KeyCode::Char('}') | KeyCode::Char('B') => {
                        // i{ or a{ or iB or aB - braces
                        TextObjects::paired_delimiters(
                            editor.buffer(),
                            '{',
                            '}',
                            text_obj_type == 'a',
                        )
                    }
                    KeyCode::Char('<') | KeyCode::Char('>') => {
                        // i< or a< or i> or a> - angle brackets
                        TextObjects::paired_delimiters(
                            editor.buffer(),
                            '<',
                            '>',
                            text_obj_type == 'a',
                        )
                    }
                    KeyCode::Char('t') => {
                        // it or at - HTML/XML tag
                        TextObjects::tag(editor.buffer(), text_obj_type == 'a')
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

                            // Create Change (range.end_col is already exclusive)
                            let change_range = Range::new(
                                (range.start_line, range.start_col),
                                (range.end_line, range.end_col),
                            );
                            let change =
                                Change::delete(change_range, deleted.clone(), cursor_before);

                            // Apply the change to actually delete the text
                            change.apply(editor.buffer_mut());

                            editor.delete_to_register(deleted);
                            editor.add_change(change);

                            // Clamp cursor to buffer bounds
                            Self::clamp_cursor_to_buffer(editor);
                        }
                        Operator::Yank => {
                            let yanked = TextObjects::yank_range(editor.buffer(), range)?;
                            editor.yank_to_register(yanked);
                        }
                        Operator::Change => {
                            let cursor = editor.buffer().cursor();
                            let cursor_before = (cursor.line(), cursor.col());

                            // Get the text to be deleted first
                            let deleted = TextObjects::yank_range(editor.buffer(), range)?;

                            // Create Change (range.end_col is already exclusive)
                            let change_range = Range::new(
                                (range.start_line, range.start_col),
                                (range.end_line, range.end_col),
                            );
                            let change =
                                Change::delete(change_range, deleted.clone(), cursor_before);

                            // Apply the change to actually delete the text
                            change.apply(editor.buffer_mut());

                            editor.delete_to_register(deleted);
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
                            let cursor_before = (
                                editor.buffer().cursor().line(),
                                editor.buffer().cursor().col(),
                            );

                            // Get the text in the range
                            let text = TextObjects::yank_range(editor.buffer(), range)?;

                            // Transform to lowercase
                            let transformed = text.to_lowercase();

                            if transformed != text {
                                // Delete the old text (range.end_col is already exclusive)
                                let deleted = editor.buffer_mut().delete_range(
                                    range.start_line,
                                    range.start_col,
                                    range.end_line,
                                    range.end_col,
                                );
                                let delete_range = Range::new(
                                    (range.start_line, range.start_col),
                                    (range.end_line, range.end_col),
                                );
                                let delete_change =
                                    Change::delete(delete_range, deleted, cursor_before);

                                // Insert the transformed text
                                let insert_change = Change::insert(
                                    (range.start_line, range.start_col),
                                    transformed,
                                    cursor_before,
                                );
                                insert_change.apply(editor.buffer_mut());

                                editor.add_change(delete_change);
                                editor.add_change(insert_change);
                            }
                        }
                        Operator::Uppercase => {
                            let cursor_before = (
                                editor.buffer().cursor().line(),
                                editor.buffer().cursor().col(),
                            );

                            // Get the text in the range
                            let text = TextObjects::yank_range(editor.buffer(), range)?;

                            // Transform to uppercase
                            let transformed = text.to_uppercase();

                            if transformed != text {
                                // Delete the old text (range.end_col is already exclusive)
                                let deleted = editor.buffer_mut().delete_range(
                                    range.start_line,
                                    range.start_col,
                                    range.end_line,
                                    range.end_col,
                                );
                                let delete_range = Range::new(
                                    (range.start_line, range.start_col),
                                    (range.end_line, range.end_col),
                                );
                                let delete_change =
                                    Change::delete(delete_range, deleted, cursor_before);

                                // Insert the transformed text
                                let insert_change = Change::insert(
                                    (range.start_line, range.start_col),
                                    transformed,
                                    cursor_before,
                                );
                                insert_change.apply(editor.buffer_mut());

                                editor.add_change(delete_change);
                                editor.add_change(insert_change);
                            }
                        }
                        Operator::ToggleCase => {
                            let cursor_before = (
                                editor.buffer().cursor().line(),
                                editor.buffer().cursor().col(),
                            );

                            // Get the text in the range
                            let text = TextObjects::yank_range(editor.buffer(), range)?;

                            // Toggle case
                            let transformed: String = text
                                .chars()
                                .map(|ch| {
                                    if ch.is_lowercase() {
                                        ch.to_uppercase().to_string()
                                    } else {
                                        ch.to_lowercase().to_string()
                                    }
                                })
                                .collect();

                            if transformed != text {
                                // Delete the old text (range.end_col is already exclusive)
                                let deleted = editor.buffer_mut().delete_range(
                                    range.start_line,
                                    range.start_col,
                                    range.end_line,
                                    range.end_col,
                                );
                                let delete_range = Range::new(
                                    (range.start_line, range.start_col),
                                    (range.end_line, range.end_col),
                                );
                                let delete_change =
                                    Change::delete(delete_range, deleted, cursor_before);

                                // Insert the transformed text
                                let insert_change = Change::insert(
                                    (range.start_line, range.start_col),
                                    transformed,
                                    cursor_before,
                                );
                                insert_change.apply(editor.buffer_mut());

                                editor.add_change(delete_change);
                                editor.add_change(insert_change);
                            }
                        }
                        Operator::ReplaceWithRegister => {
                            let cursor_before = (
                                editor.buffer().cursor().line(),
                                editor.buffer().cursor().col(),
                            );

                            // Get the register content
                            let register_content = editor.get_from_register();

                            // Get the text in the range (to delete)
                            let deleted = TextObjects::yank_range(editor.buffer(), range)?;

                            // Delete the old text (range.end_col is already exclusive)
                            editor.buffer_mut().delete_range(
                                range.start_line,
                                range.start_col,
                                range.end_line,
                                range.end_col,
                            );
                            let delete_range = Range::new(
                                (range.start_line, range.start_col),
                                (range.end_line, range.end_col),
                            );
                            let delete_change =
                                Change::delete(delete_range, deleted, cursor_before);

                            // Insert the register content
                            let insert_change = Change::insert(
                                (range.start_line, range.start_col),
                                register_content,
                                cursor_before,
                            );
                            insert_change.apply(editor.buffer_mut());

                            editor.add_change(delete_change);
                            editor.add_change(insert_change);

                            // Position cursor at start of replaced text
                            editor
                                .buffer_mut()
                                .cursor_mut()
                                .set_position(range.start_line, range.start_col);
                        }
                        Operator::Fold => {
                            // Create a fold from start_line to end_line (inclusive)
                            let start_line = range.start_line.min(range.end_line);
                            let end_line = range.start_line.max(range.end_line);
                            editor
                                .buffer_mut()
                                .fold_manager_mut()
                                .create_fold(start_line, end_line);
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
                ('r', KeyCode::Char(ch)) => {
                    // r{char} - replace character(s) under cursor
                    let count = editor.effective_count();
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let line_idx = cursor.line();
                    let col = cursor.col();

                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let chars_count = line_text.chars().count();

                        if col < chars_count {
                            let replace_count = count.min(chars_count - col);
                            let end_col = col + replace_count;

                            // Delete the characters
                            let deleted = editor
                                .buffer_mut()
                                .delete_range(line_idx, col, line_idx, end_col);

                            // Insert the replacement character(s)
                            let replacement = ch.to_string().repeat(replace_count);
                            editor
                                .buffer_mut()
                                .insert_text_at(line_idx, col, &replacement);

                            // Create composite change for undo/redo
                            let start_pos = (line_idx, col);
                            let end_pos = (line_idx, end_col);
                            let range = Range::new(start_pos, end_pos);

                            let delete_change = Change::delete(range, deleted, cursor_before);
                            let insert_change =
                                Change::insert((line_idx, col), replacement, cursor_before);
                            let change = Change::composite(
                                vec![delete_change, insert_change],
                                cursor_before,
                                cursor_before,
                            );

                            editor.add_change(change);

                            // Don't move cursor after replace (Vim behavior)
                        }
                    }
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('g')) => {
                    // gg - go to first line
                    let target_line = editor.count().unwrap_or(1).saturating_sub(1);
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(target_line, 0);
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('d')) => {
                    // gd - go to definition (LSP)
                    editor.request_goto_definition();
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('D')) => {
                    // gD - go to implementation (LSP)
                    editor.request_goto_implementation();
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('y')) => {
                    // gy - go to type definition (LSP)
                    editor.request_goto_type();
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('R')) => {
                    // gR - find references (LSP)
                    editor.request_find_references();
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('c')) => {
                    // gc - show code actions (LSP)
                    editor.request_code_actions();
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('q')) => {
                    // gq - format document (LSP)
                    editor.request_format_document();
                    editor.clear_count();
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
                ('g', KeyCode::Char('r')) => {
                    // gr{motion} - replace with register content
                    editor.set_pending_operator(Operator::ReplaceWithRegister);
                    return Ok(());
                }
                ('g', KeyCode::Char('i')) => {
                    // gi - go to last insert position and enter insert mode
                    if let Some((line, col)) = editor.last_insert_position {
                        editor.buffer_mut().cursor_mut().set_position(line, col);
                    }
                    // Enter insert mode regardless of whether position was saved
                    let cursor_before = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
                    editor.start_change_building(cursor_before);
                    editor.set_mode(Mode::Insert);
                    return Ok(());
                }
                ('g', KeyCode::Char('I')) => {
                    // gI - insert at column 0 (before any indentation)
                    editor.buffer_mut().cursor_mut().set_col(0);
                    let cursor_before = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
                    editor.start_change_building(cursor_before);
                    editor.set_mode(Mode::Insert);
                    return Ok(());
                }
                ('z', KeyCode::Char('o')) => {
                    // zo - open fold at cursor
                    let line = editor.buffer().cursor().line();
                    editor.buffer_mut().open_fold(line);
                    return Ok(());
                }
                ('z', KeyCode::Char('c')) => {
                    // zc - close fold at cursor
                    let line = editor.buffer().cursor().line();
                    editor.buffer_mut().close_fold(line);
                    return Ok(());
                }
                ('z', KeyCode::Char('a')) => {
                    // za - toggle fold at cursor
                    let line = editor.buffer().cursor().line();
                    editor.buffer_mut().toggle_fold(line);
                    return Ok(());
                }
                ('z', KeyCode::Char('R')) => {
                    // zR - open all folds
                    editor.buffer_mut().fold_manager_mut().open_all();
                    return Ok(());
                }
                ('z', KeyCode::Char('M')) => {
                    // zM - close all folds
                    editor.buffer_mut().fold_manager_mut().close_all();
                    return Ok(());
                }
                ('z', KeyCode::Char('d')) => {
                    // zd - delete fold at cursor
                    let line = editor.buffer().cursor().line();
                    editor.buffer_mut().fold_manager_mut().delete_fold_at(line);
                    return Ok(());
                }
                ('z', KeyCode::Char('E')) => {
                    // zE - eliminate all folds
                    editor.buffer_mut().clear_folds();
                    return Ok(());
                }
                ('z', KeyCode::Char('f')) => {
                    // zf{motion} - create fold
                    editor.set_pending_operator(Operator::Fold);
                    return Ok(());
                }
                ('g', KeyCode::Char('t')) => {
                    // gt - go to next tab (or tab number if count specified)
                    if let Some(count) = editor.count() {
                        // {count}gt - go to specific tab (1-indexed)
                        editor.goto_tab(count.saturating_sub(1));
                    } else {
                        // gt - next tab
                        editor.next_tab();
                    }
                    editor.clear_count();
                    return Ok(());
                }
                ('g', KeyCode::Char('T')) => {
                    // gT - go to previous tab
                    editor.previous_tab();
                    editor.clear_count();
                    return Ok(());
                }
                ('"', KeyCode::Char(ch)) if ch.is_ascii_alphanumeric() || ch == '"' => {
                    // "{register} - select register for next yank/delete/paste
                    editor.set_pending_register(ch);
                    return Ok(());
                }
                ('m', KeyCode::Char(ch)) if ch.is_ascii_lowercase() || ch.is_ascii_uppercase() => {
                    // m{a-z} or m{A-Z} - set local or global mark
                    editor.set_mark(ch);
                    return Ok(());
                }
                ('\'', KeyCode::Char(ch)) if ch.is_ascii_lowercase() || ch.is_ascii_uppercase() => {
                    // '{a-z} or '{A-Z} - jump to mark line
                    editor.add_jump(); // Add current position to jump list before jumping
                    editor.jump_to_mark_line(ch);
                    return Ok(());
                }
                ('`', KeyCode::Char(ch)) if ch.is_ascii_lowercase() || ch.is_ascii_uppercase() => {
                    // `{a-z} or `{A-Z} - jump to mark exact position
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
                ('W', KeyCode::Char('w')) => {
                    // Ctrl-W w - cycle to next window
                    editor.focus_next_window();
                    editor.clear_pending_command();
                    return Ok(());
                }
                ('W', KeyCode::Char('p')) => {
                    // Ctrl-W p - cycle to previous window
                    editor.focus_prev_window();
                    editor.clear_pending_command();
                    return Ok(());
                }
                ('W', KeyCode::Char('s')) => {
                    // Ctrl-W s - split window horizontally
                    editor.split_window_horizontal();
                    editor.clear_pending_command();
                    return Ok(());
                }
                ('W', KeyCode::Char('v')) => {
                    // Ctrl-W v - split window vertically
                    editor.split_window_vertical();
                    editor.clear_pending_command();
                    return Ok(());
                }
                ('z', KeyCode::Char('z')) => {
                    // zz - center cursor in viewport
                    editor.center_cursor_in_viewport();
                    editor.clear_count();
                    return Ok(());
                }
                ('z', KeyCode::Char('t')) => {
                    // zt - move cursor line to top of viewport
                    editor.move_cursor_line_to_top();
                    editor.clear_count();
                    return Ok(());
                }
                ('z', KeyCode::Char('b')) => {
                    // zb - move cursor line to bottom of viewport
                    editor.move_cursor_line_to_bottom();
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
            // Jump back (Ctrl-T) - tag stack equivalent
            KeyCode::Char('t') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.jump_back();
            }
            // Window commands (Ctrl-W) - set pending command
            KeyCode::Char('w') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.set_pending_command('W'); // Use capital W for Ctrl-W prefix
                return Ok(());
            }
            // Scroll down half page (Ctrl-D)
            KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let half_page = editor.half_page_scroll();
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
                let half_page = editor.half_page_scroll();
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
            // Scrolling commands
            // Ctrl-E: Scroll viewport down one line (cursor follows if needed)
            KeyCode::Char('e') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let count = editor.effective_count();
                editor.scroll_viewport_down(count);
                editor.clear_count();
            }
            // Ctrl-Y: Scroll viewport up one line (cursor follows if needed)
            KeyCode::Char('y') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let count = editor.effective_count();
                editor.scroll_viewport_up(count);
                editor.clear_count();
            }
            // Ctrl-D: Scroll down half page (both viewport and cursor)
            KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.scroll_half_page_down();
                editor.clear_count();
            }
            // Ctrl-U: Scroll up half page (both viewport and cursor)
            KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.scroll_half_page_up();
                editor.clear_count();
            }
            // Ctrl-F: Scroll forward (down) one page (both viewport and cursor)
            KeyCode::Char('f') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.scroll_page_down();
                editor.clear_count();
            }
            // Ctrl-B: Scroll backward (up) one page (both viewport and cursor)
            KeyCode::Char('b') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.scroll_page_up();
                editor.clear_count();
            }
            // Enter Insert mode
            KeyCode::Char('i') => {
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
            }
            KeyCode::Char('a') => {
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                // Move cursor right (insert after)
                let cursor = editor.buffer_mut().cursor_mut();
                cursor.move_right(1);
            }
            KeyCode::Char('I') => {
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                // Move to first non-blank character
                Motions::first_non_blank(editor.buffer_mut());
            }
            KeyCode::Char('A') => {
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
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
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                // Insert new line below and move to it
                Self::insert_line_below(editor)?;
            }
            KeyCode::Char('O') => {
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
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
                editor.clear_count();
                return Ok(());
            }
            // Line motions
            KeyCode::Char('0') => {
                // If there's already a count, treat this as a digit (e.g., "50j")
                // Otherwise, treat it as a motion to column 0
                if editor.count().is_some() {
                    editor.append_count(0);
                } else {
                    editor.buffer_mut().cursor_mut().set_col(0);
                    editor.clear_count();
                }
            }
            KeyCode::Char('$') => {
                let line_idx = editor.buffer().cursor().line();
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_len = line.trim_end_matches('\n').chars().count();
                    let col = if line_len > 0 { line_len - 1 } else { 0 };
                    let cursor = editor.buffer_mut().cursor_mut();
                    cursor.set_col(col);
                    // Set desired_col to usize::MAX to indicate "always end of line"
                    cursor.update_desired_col(usize::MAX);
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
                    if let Some((line, col, _)) =
                        search.find_next(editor.buffer(), cursor.line(), cursor.col() + 1)
                    {
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
                    let search_col = if cursor.col() > 0 {
                        cursor.col() - 1
                    } else {
                        0
                    };
                    if let Some((line, col, _)) =
                        search.find_next(editor.buffer(), cursor.line(), search_col)
                    {
                        editor.buffer_mut().cursor_mut().set_position(line, col);
                    }
                    editor.set_current_search(search);
                }
            }
            // Register selection (" followed by register name)
            KeyCode::Char('"') => {
                editor.set_pending_command('"');
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
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(target_line, 0);
                editor.clear_count();
            }
            // Fold commands
            KeyCode::Char('z') => {
                // Set pending command to wait for second character (zo, zc, etc.)
                editor.set_pending_command('z');
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

                        let deleted = editor
                            .buffer_mut()
                            .delete_range(line_idx, col, line_idx, end_col);
                        let range = Range::new(start_pos, end_pos);
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.delete_to_register(deleted);
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
                        let _chars_count = line_text.chars().count();

                        // Calculate start column (delete count chars before cursor)
                        let start_col = col.saturating_sub(count);
                        let start_pos = (line_idx, start_col);
                        let end_pos = (line_idx, col);

                        let deleted = editor
                            .buffer_mut()
                            .delete_range(line_idx, start_col, line_idx, col);
                        let range = Range::new(start_pos, end_pos);
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.delete_to_register(deleted);
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

                        let deleted = editor
                            .buffer_mut()
                            .delete_range(line_idx, col, line_idx, line_len);
                        let range = Range::new(start_pos, end_pos);
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.delete_to_register(deleted);
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

                        let deleted = editor
                            .buffer_mut()
                            .delete_range(line_idx, col, line_idx, line_len);
                        let range = Range::new(start_pos, end_pos);
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.delete_to_register(deleted);
                        editor.add_change(change);

                        // Don't clamp cursor - we want to insert at end of line
                        editor.clear_count();
                        let insert_cursor = (
                            editor.buffer().cursor().line(),
                            editor.buffer().cursor().col(),
                        );
                        editor.start_change_building(insert_cursor); // C enters insert mode, start building
                        editor.set_mode(Mode::Insert);
                        return Ok(());
                    }
                }
                editor.clear_count();
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
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
            KeyCode::Char('Y') => {
                // Y - yank line (same as yy)
                let count = editor.effective_count();
                let yanked = Operators::yank_line(editor.buffer(), count)?;
                editor.yank_to_register(yanked);
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

                        let deleted = editor
                            .buffer_mut()
                            .delete_range(line_idx, col, line_idx, end_col);
                        let range = Range::new(start_pos, end_pos);
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.delete_to_register(deleted);
                        editor.add_change(change);

                        // Clamp cursor to buffer bounds
                        Self::clamp_cursor_to_buffer(editor);
                    }
                }
                editor.clear_count();
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
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

                editor.delete_to_register(deleted);
                editor.add_change(change);
                editor.clear_count();
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                Self::insert_line_above(editor)?;
            }
            // Replace
            KeyCode::Char('r') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+R - redo (handle this before regular 'r' to avoid unreachable pattern)
                editor.redo();
                editor.clear_count();
            }
            KeyCode::Char('r') => {
                // r{char} - replace character under cursor (wait for next key)
                editor.set_pending_command('r');
            }
            // Toggle case
            KeyCode::Char('~') => {
                // ~ - toggle case of character under cursor (with count)
                let count = editor.effective_count();
                for _ in 0..count {
                    Self::toggle_case_at_cursor(editor)?;
                }
                editor.clear_count();
            }
            // Undo/Redo
            KeyCode::Char('u') => {
                // u - undo
                editor.undo();
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
}
