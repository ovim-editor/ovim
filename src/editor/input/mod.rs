use crate::editor::{
    Change, Editor, FindDirection, FindType, Motions, Operator, Operators, Range, Search,
    TextObjects,
};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

/// Command handling submodule
mod commands;

/// Number operations (Ctrl-A, Ctrl-X, g Ctrl-A, g Ctrl-X)
mod numbers;

/// Case operations (toggle, upper, lower)
mod case;

/// Helper functions for cursor movement and editing
mod helpers;

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

        let result = match editor.mode() {
            Mode::Normal => Self::handle_normal_mode(editor, key_event),
            Mode::Insert => Self::handle_insert_mode(editor, key_event),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
                Self::handle_visual_mode(editor, key_event)
            }
            Mode::Command => commands::handle_command_mode(editor, key_event),
            Mode::Search => Self::handle_search_mode(editor, key_event),
            Mode::Replace => Self::handle_replace_mode(editor, key_event),
            Mode::Picker => Self::handle_picker_mode(editor, key_event),
            Mode::HoverWindow => Self::handle_hover_window_mode(editor, key_event),
            Mode::FileTree => Self::handle_filetree_mode(editor, key_event),
            Mode::SubstituteConfirm => Self::handle_substitute_confirm_mode(editor, key_event),
        };

        // Mark the editor as dirty after processing any key event
        // This ensures UI is redrawn on next render cycle
        editor.mark_dirty();

        // Update scroll offset to keep cursor visible with scrolloff margin
        editor.update_scroll_offset();

        result
    }

    /// Handles input in Normal mode
    fn handle_normal_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
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
        // or a motion prefix ('g' for gg)
        // to allow text objects like di{ and motions like cgg to be handled later
        let has_text_obj_prefix = matches!(editor.pending_command(), Some('i') | Some('a') | Some('g'));

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

                    helpers::indent_lines_with_tracking(
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

                    helpers::dedent_lines_with_tracking(
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
                    helpers::clamp_cursor_to_buffer(editor);
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
                (Operator::Change, KeyCode::Char('G')) => {
                    // cG - change from current line to end of file
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

                    // Position cursor and insert new line below
                    helpers::clamp_cursor_to_buffer(editor);
                    editor.clear_count();

                    let insert_cursor = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
                    editor.start_change_building(insert_cursor);
                    editor.set_mode(Mode::Insert);
                    helpers::insert_line_below(editor)?;
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
                (Operator::Change, KeyCode::Char('g')) => {
                    // cgg - change from current line to first line
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

                        helpers::indent_lines_with_tracking(
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

                        helpers::dedent_lines_with_tracking(
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
                    (Operator::Change, KeyCode::Char('g')) => {
                        // cgg - change from current line to first line (or specified line)
                        editor.clear_pending_operator();
                        editor.clear_pending_command();
                        let end_line = editor.buffer().cursor().line();
                        let cursor_before = (end_line, editor.buffer().cursor().col());
                        let start_line = if let Some(cnt) = editor.count() {
                            cnt.saturating_sub(1)
                        } else {
                            0
                        };

                        // Delete from start line to current line (inclusive)
                        let deleted = editor
                            .buffer_mut()
                            .delete_range(start_line, 0, end_line + 1, 0);
                        let range = Range::new((start_line, 0), (end_line + 1, 0));
                        let change = Change::delete(range, deleted.clone(), cursor_before);

                        editor.delete_to_register(deleted);
                        editor.add_change(change);

                        // Position cursor at start, then start change building and enter insert mode
                        editor.buffer_mut().cursor_mut().set_position(start_line, 0);
                        helpers::clamp_cursor_to_buffer(editor);
                        editor.clear_count();

                        let insert_cursor = (
                            editor.buffer().cursor().line(),
                            editor.buffer().cursor().col(),
                        );
                        editor.start_change_building(insert_cursor);
                        editor.set_mode(Mode::Insert);
                        return Ok(());
                    }
                    _ => {}
                }
            }

            // Don't clear pending operator if we have a text object prefix ('i' or 'a')
            // or a motion prefix ('g' for gg)
            // This allows text objects like 'dip', 'caw', etc. and motions like 'cgg', 'dgg' to work
            if !matches!(editor.pending_command(), Some('i') | Some('a') | Some('g')) {
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
                    helpers::clamp_cursor_to_buffer(editor);

                    // Fix: After dd, cursor should always be at column 0 of the current line
                    // This prevents edge cases where cursor ends up at end of line
                    let current_line = editor.buffer().cursor().line();
                    editor.buffer_mut().cursor_mut().set_position(current_line, 0);

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

                    // Fix: If we crossed a newline, stop at the end of the current line (before newline)
                    // dw should delete to end of line but NOT include the newline character
                    // end_col is exclusive, so line_text.chars().count() correctly points after last char
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
                    helpers::clamp_cursor_to_buffer(editor);

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
                            helpers::clamp_cursor_to_buffer(editor);
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
                    helpers::insert_line_above(editor)?;
                    return Ok(());
                }
                (Operator::Change, KeyCode::Char('w')) => {
                    // cw - change word
                    // Special case in Vim: cw behaves like ce (change to end of word),
                    // not like dw (delete to start of next word)
                    let start_cursor = editor.buffer().cursor().clone();
                    let cursor_before = (start_cursor.line(), start_cursor.col());
                    let start_line = start_cursor.line();
                    let start_col = start_cursor.col();

                    // Start change building BEFORE deletion so it's part of the composite change
                    editor.start_change_building(cursor_before);

                    // Use end-of-word motion for cw (not word_forward)
                    Motions::word_end_forward(editor.buffer_mut(), count);

                    let end_cursor = editor.buffer().cursor();
                    let end_line = end_cursor.line();
                    // Fix Bug 3: Clamp end_col to line length to prevent out-of-bounds
                    // For deletion range, we need to include the character at end_col
                    let line_len = if let Some(line) = editor.buffer().line(end_line) {
                        line.trim_end_matches('\n').chars().count()
                    } else {
                        0
                    };
                    let end_col = (end_cursor.col() + 1).min(line_len);

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
                    // Add deletion to the change builder (not directly to undo stack)
                    editor.add_change(change);

                    // Continue in insert mode - insertions will be added to the same builder
                    editor.clear_count();
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
                (Operator::Change, KeyCode::Char('l')) | (Operator::Change, KeyCode::Right) => {
                    // cl - change character(s) to the right
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let line_idx = cursor.line();
                    let start_col = cursor.col();

                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let line_len = line_text.chars().count();
                        let end_col = (start_col + count).min(line_len);

                        if start_col < end_col {
                            let start_pos = (line_idx, start_col);
                            let end_pos = (line_idx, end_col);

                            // Start change building BEFORE deletion
                            editor.start_change_building(cursor_before);

                            let deleted = editor
                                .buffer_mut()
                                .delete_range(line_idx, start_col, line_idx, end_col);
                            let range = Range::new(start_pos, end_pos);
                            let change = Change::delete(range, deleted.clone(), cursor_before);

                            editor.delete_to_register(deleted);
                            editor.add_change(change);

                            // Position cursor at deletion start
                            editor
                                .buffer_mut()
                                .cursor_mut()
                                .set_position(line_idx, start_col);

                            editor.clear_count();
                            editor.set_mode(Mode::Insert);
                            return Ok(());
                        }
                    }
                    editor.clear_count();
                    return Ok(());
                }
                // Case change operations
                (Operator::Lowercase, KeyCode::Char('u')) => {
                    // gugu - lowercase line
                    case::change_case_line(editor, count, case::CaseChange::Lowercase)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Uppercase, KeyCode::Char('U')) => {
                    // gUgU - uppercase line
                    case::change_case_line(editor, count, case::CaseChange::Uppercase)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::ToggleCase, KeyCode::Char('~')) => {
                    // g~g~ - toggle case line
                    case::change_case_line(editor, count, case::CaseChange::Toggle)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Lowercase, KeyCode::Char('w')) => {
                    // guw - lowercase word
                    case::change_case_motion(editor, count, case::CaseChange::Lowercase, |buf, cnt| {
                        Motions::word_forward(buf, cnt);
                    })?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Uppercase, KeyCode::Char('w')) => {
                    // gUw - uppercase word
                    case::change_case_motion(editor, count, case::CaseChange::Uppercase, |buf, cnt| {
                        Motions::word_forward(buf, cnt);
                    })?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::ToggleCase, KeyCode::Char('w')) => {
                    // g~w - toggle case word
                    case::change_case_motion(editor, count, case::CaseChange::Toggle, |buf, cnt| {
                        Motions::word_forward(buf, cnt);
                    })?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Lowercase, KeyCode::Char('$')) => {
                    // gu$ - lowercase to end of line
                    case::change_case_to_end_of_line(editor, case::CaseChange::Lowercase)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Uppercase, KeyCode::Char('$')) => {
                    // gU$ - uppercase to end of line
                    case::change_case_to_end_of_line(editor, case::CaseChange::Uppercase)?;
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::ToggleCase, KeyCode::Char('$')) => {
                    // g~$ - toggle case to end of line
                    case::change_case_to_end_of_line(editor, case::CaseChange::Toggle)?;
                    editor.clear_count();
                    return Ok(());
                }
                // Count digits after operator (e.g., d2w)
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
                    helpers::clamp_cursor_to_buffer(editor);
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
                    helpers::clamp_cursor_to_buffer(editor);
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
                (Operator::Change, KeyCode::Char('j')) => {
                    // cj - change current line and line below
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let start_line = cursor.line();
                    let end_line = (start_line + count + 1).min(editor.buffer().line_count());

                    let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line, 0);
                    let range = Range::new((start_line, 0), (end_line, 0));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.delete_to_register(deleted);
                    editor.add_change(change);

                    // Position cursor at start, insert blank line, and enter insert mode
                    editor.buffer_mut().cursor_mut().set_position(start_line, 0);
                    helpers::clamp_cursor_to_buffer(editor);
                    editor.clear_count();

                    let insert_cursor = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
                    editor.start_change_building(insert_cursor);
                    editor.set_mode(Mode::Insert);
                    helpers::insert_line_above(editor)?;
                    return Ok(());
                }
                (Operator::Change, KeyCode::Char('k')) => {
                    // ck - change current line and line above
                    let cursor = editor.buffer().cursor();
                    let cursor_before = (cursor.line(), cursor.col());
                    let end_line = cursor.line() + 1;
                    let start_line = cursor.line().saturating_sub(count);

                    let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line, 0);
                    let range = Range::new((start_line, 0), (end_line, 0));
                    let change = Change::delete(range, deleted.clone(), cursor_before);

                    editor.delete_to_register(deleted);
                    editor.add_change(change);

                    // Position cursor at start, insert blank line, and enter insert mode
                    editor.buffer_mut().cursor_mut().set_position(start_line, 0);
                    helpers::clamp_cursor_to_buffer(editor);
                    editor.clear_count();

                    let insert_cursor = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );
                    editor.start_change_building(insert_cursor);
                    editor.set_mode(Mode::Insert);
                    helpers::insert_line_above(editor)?;
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
                    helpers::clamp_cursor_to_buffer(editor);
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
                    helpers::clamp_cursor_to_buffer(editor);
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
                        helpers::clamp_cursor_to_buffer(editor);
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

                    helpers::indent_lines_with_tracking(
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

                    helpers::indent_lines_with_tracking(
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

                    helpers::indent_lines_with_tracking(
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

                    helpers::dedent_lines_with_tracking(
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

                    helpers::dedent_lines_with_tracking(
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

                    helpers::dedent_lines_with_tracking(
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
                    KeyCode::Char('i') => {
                        // ii or ai - indentation block
                        let tab_width = editor.options.tab_width;
                        if text_obj_type == 'i' {
                            TextObjects::inner_indent(editor.buffer(), tab_width)
                        } else {
                            TextObjects::around_indent(editor.buffer(), tab_width)
                        }
                    }
                    KeyCode::Char('f') => {
                        // if or af - function
                        if text_obj_type == 'i' {
                            TextObjects::inner_function(editor.buffer())
                        } else {
                            TextObjects::around_function(editor.buffer())
                        }
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
                            helpers::clamp_cursor_to_buffer(editor);
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

                            // Don't clamp cursor - we're entering insert mode where cursor can be past end of line
                            // The cursor is already correctly positioned by delete_range at the start of deletion

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
                        Operator::Fold => {
                            // Create a fold from start_line to end_line (inclusive)
                            let start_line = range.start_line.min(range.end_line);
                            let end_line = range.start_line.max(range.end_line);
                            editor
                                .buffer_mut()
                                .fold_manager_mut()
                                .create_fold(start_line, end_line);
                        }
                        // Indent/dedent/auto-indent don't make sense with text objects, just ignore
                        Operator::Indent | Operator::Dedent | Operator::AutoIndent => {}
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
                    helpers::join_lines_no_space(editor, count)?;
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
                    // gr prefix for LSP commands (grr, grn, gra, gri, grt)
                    // Use 'R' as pending to avoid conflict with regular 'r' command
                    editor.set_pending_command('R');
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
                ('g', KeyCode::Char('v')) => {
                    // gv - reselect last visual selection
                    editor.restore_last_visual_selection();
                    editor.clear_pending_command();
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
                ('g', KeyCode::Char(';')) => {
                    // g; - jump to last change position
                    if let Some(change) = editor.last_change() {
                        let pos = change.cursor_before();
                        editor
                            .buffer_mut()
                            .cursor_mut()
                            .set_position(pos.0, pos.1);
                    }
                    editor.clear_count();
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
                ('Z', KeyCode::Char('Z')) => {
                    // ZZ - save and quit
                    if editor.buffer().file_path().is_some() {
                        // Try to save, but don't fail if runtime unavailable (e.g., tests)
                        if tokio::runtime::Handle::try_current().is_ok() {
                            let _ = editor.buffer_mut().save();
                        }
                    }
                    editor.quit();
                    return Ok(());
                }
                ('Z', KeyCode::Char('Q')) => {
                    // ZQ - quit without saving
                    editor.quit();
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
                // LSP commands - gr prefix (pending='R')
                ('R', KeyCode::Char('r')) => {
                    // grr - LSP references
                    editor.request_find_references();
                    editor.clear_count();
                    return Ok(());
                }
                ('R', KeyCode::Char('n')) => {
                    // grn - LSP rename
                    // Enter command mode with LspRename prompt
                    editor.clear_command_line();
                    editor.set_mode(Mode::Command);
                    // Pre-fill command line with "LspRename "
                    for ch in "LspRename ".chars() {
                        editor.append_to_command_line(ch);
                    }
                    return Ok(());
                }
                ('R', KeyCode::Char('a')) => {
                    // gra - LSP code action
                    editor.request_code_actions();
                    editor.clear_count();
                    return Ok(());
                }
                ('R', KeyCode::Char('i')) => {
                    // gri - LSP implementation
                    editor.request_goto_implementation();
                    editor.clear_count();
                    return Ok(());
                }
                ('R', KeyCode::Char('t')) => {
                    // grt - LSP type definition
                    editor.request_goto_type();
                    editor.clear_count();
                    return Ok(());
                }
                // Section/bracket navigation - [ prefix
                ('[', KeyCode::Char('[')) => {
                    // [[ - go to previous section (function start)
                    let count = editor.effective_count();
                    Motions::section_backward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                ('[', KeyCode::Char(']')) => {
                    // [] - go to previous section end
                    let count = editor.effective_count();
                    Motions::section_end_backward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                ('[', KeyCode::Char('{')) => {
                    // [{ - go to unmatched { (backward)
                    let count = editor.effective_count();
                    Motions::unmatched_brace_backward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                ('[', KeyCode::Char('(')) => {
                    // [( - go to unmatched ( (backward)
                    let count = editor.effective_count();
                    Motions::unmatched_paren_backward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                ('[', KeyCode::Char('m')) => {
                    // [m - go to previous method start
                    let count = editor.effective_count();
                    Motions::method_backward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                ('[', KeyCode::Char('M')) => {
                    // [M - go to previous method end
                    let count = editor.effective_count();
                    Motions::method_end_backward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                ('[', KeyCode::Char('d')) => {
                    // [d - go to previous diagnostic
                    editor.goto_prev_diagnostic();
                    editor.clear_count();
                    editor.clear_pending_command();
                    return Ok(());
                }
                // Section/bracket navigation - ] prefix
                (']', KeyCode::Char(']')) => {
                    // ]] - go to next section (function start)
                    let count = editor.effective_count();
                    Motions::section_forward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                (']', KeyCode::Char('[')) => {
                    // ][ - go to next section end
                    let count = editor.effective_count();
                    Motions::section_end_forward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                (']', KeyCode::Char('}')) => {
                    // ]} - go to unmatched } (forward)
                    let count = editor.effective_count();
                    Motions::unmatched_brace_forward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                (']', KeyCode::Char(')')) => {
                    // ]) - go to unmatched ) (forward)
                    let count = editor.effective_count();
                    Motions::unmatched_paren_forward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                (']', KeyCode::Char('m')) => {
                    // ]m - go to next method start
                    let count = editor.effective_count();
                    Motions::method_forward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                (']', KeyCode::Char('M')) => {
                    // ]M - go to next method end
                    let count = editor.effective_count();
                    Motions::method_end_forward(editor.buffer_mut(), count);
                    editor.clear_count();
                    return Ok(());
                }
                (']', KeyCode::Char('d')) => {
                    // ]d - go to next diagnostic
                    editor.goto_next_diagnostic();
                    editor.clear_count();
                    editor.clear_pending_command();
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
                helpers::clamp_cursor_to_line(editor);
                editor.clear_count();
            }
            // Scroll up half page (Ctrl-U)
            KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let half_page = editor.half_page_scroll();
                let count = editor.count().unwrap_or(half_page);
                let cursor = editor.buffer_mut().cursor_mut();
                let new_line = cursor.line().saturating_sub(count);
                cursor.set_line(new_line);
                helpers::clamp_cursor_to_line(editor);
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
                numbers::increment_number(editor, count)?;
                editor.clear_count();
            }
            // Decrement number (Ctrl-X)
            KeyCode::Char('x') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let count = editor.effective_count();
                numbers::decrement_number(editor, count)?;
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
            // Enter Replace mode
            KeyCode::Char('R') => {
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Replace);
            }
            KeyCode::Char('o') => {
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                // Insert new line below and move to it
                helpers::insert_line_below(editor)?;
            }
            KeyCode::Char('O') => {
                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );
                editor.start_change_building(cursor_before);
                editor.set_mode(Mode::Insert);
                // Insert new line above and move to it
                helpers::insert_line_above(editor)?;
            }
            // Motion commands
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
                // BUG FIX: Save cursor position before entering search mode
                // so we can restore it on ESC
                editor.save_search_start_position();
                editor.set_mode(Mode::Search);
            }
            // Enter Search mode (backward)
            KeyCode::Char('?') => {
                editor.clear_search_buffer();
                editor.set_search_forward(false);
                // BUG FIX: Save cursor position before entering search mode
                // so we can restore it on ESC
                editor.save_search_start_position();
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
                    let mut search = Search::new_with_options(
                        pattern,
                        true,
                        editor.options.ignorecase,
                        editor.options.smartcase,
                    );
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
                    let mut search = Search::new_with_options(
                        pattern,
                        false,
                        editor.options.ignorecase,
                        editor.options.smartcase,
                    );
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
            KeyCode::Char('Z') => {
                // Set pending command to wait for second character (ZZ, ZQ)
                editor.set_pending_command('Z');
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
            // Section/bracket navigation ([[, ]], [{, ]}, [m, ]m, etc.)
            KeyCode::Char('[') => {
                editor.set_pending_command('[');
            }
            KeyCode::Char(']') => {
                editor.set_pending_command(']');
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
                        helpers::clamp_cursor_to_buffer(editor);
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
                        helpers::clamp_cursor_to_buffer(editor);
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
                        helpers::clamp_cursor_to_buffer(editor);
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
                helpers::paste_after(editor)?;
                editor.clear_count();
            }
            KeyCode::Char('P') => {
                // P - paste before cursor
                helpers::paste_before(editor)?;
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
                helpers::join_lines(editor, count)?;
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
                        helpers::clamp_cursor_to_buffer(editor);
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
                helpers::insert_line_above(editor)?;
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
                    case::toggle_case_at_cursor(editor)?;
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
    fn handle_insert_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Esc => {
                // If completion menu is visible, hide it first without exiting insert mode
                if editor.completion_menu().is_visible() {
                    editor.hide_completion_menu();
                } else {
                    // Save last insert position BEFORE moving cursor (this is where we can continue inserting)
                    let cursor = editor.buffer().cursor();
                    editor.last_insert_position = Some((cursor.line(), cursor.col()));

                    editor.finalize_change_building();
                    // Update the . register with the last inserted text
                    editor.update_last_inserted_register();
                    editor.mark_buffer_modified(); // Mark for LSP didChange notification

                    // If we were in visual block insert/append mode, replay the changes on all other lines
                    let should_move_to_end_line =
                        if let Some((start_line, end_line, col, is_append, move_to_end)) =
                            editor.visual_block_insert_state()
                        {
                            // Get the text that was inserted and the first line's change
                            if let Some(last_change) = editor.last_change() {
                                let inserted_text = last_change.get_inserted_text();
                                let mut all_changes = vec![last_change.clone()];

                                // Get cursor_before from first change
                                let cursor_before = last_change.cursor_before();

                                // Replay on lines start_line+1 through end_line
                                for line_idx in (start_line + 1)..=end_line {
                                    if is_append {
                                        // Append mode: insert at end of line
                                        if let Some(line) = editor.buffer().line(line_idx) {
                                            let line_text = line.trim_end_matches('\n');
                                            let line_len = line_text.chars().count();
                                            editor.buffer_mut().insert_text_at(
                                                line_idx,
                                                line_len,
                                                &inserted_text,
                                            );
                                            // Track this insertion as a change
                                            let change = Change::insert(
                                                (line_idx, line_len),
                                                inserted_text.clone(),
                                                cursor_before,
                                            );
                                            all_changes.push(change);
                                        }
                                    } else {
                                        // Insert mode: insert at column
                                        if let Some(line) = editor.buffer().line(line_idx) {
                                            let line_text = line.trim_end_matches('\n');
                                            let insert_col = col.min(line_text.chars().count());
                                            editor.buffer_mut().insert_text_at(
                                                line_idx,
                                                insert_col,
                                                &inserted_text,
                                            );
                                            // Track this insertion as a change
                                            let change = Change::insert(
                                                (line_idx, insert_col),
                                                inserted_text.clone(),
                                                cursor_before,
                                            );
                                            all_changes.push(change);
                                        }
                                    }
                                }

                                // If multiple lines were affected, wrap in composite for proper undo
                                if all_changes.len() > 1 {
                                    // Remove the last change (first line's change) from undo stack
                                    editor.pop_last_change();

                                    // Create composite for all insert changes
                                    let insert_composite = Change::composite(
                                        all_changes,
                                        cursor_before,
                                        cursor_before,
                                    );

                                    // Check if there's a delete composite on the stack (from visual block 'c')
                                    // If so, combine delete + insert into a super-composite
                                    if let Some(prev_change) = editor.pop_last_change() {
                                        // Check if previous change looks like a visual block delete
                                        // (it would be a composite or delete change)
                                        // Combine them into a super-composite
                                        let super_composite = Change::composite(
                                            vec![prev_change, insert_composite],
                                            cursor_before,
                                            cursor_before,
                                        );
                                        editor.add_change(super_composite);
                                    } else {
                                        // No previous change, just add the insert composite
                                        editor.add_change(insert_composite);
                                    }
                                }
                            }

                            // Clear the visual block insert state
                            editor.set_visual_block_insert_state(None);
                            Some((start_line, end_line, col, is_append, move_to_end))
                        } else {
                            None
                        };

                    editor.set_mode(Mode::Normal);

                    // Move cursor left when exiting insert mode (unless at column 0)

                    // If we were in visual block mode, move cursor to appropriate line
                    if let Some((start_line, end_line, _col, is_append, move_to_end)) =
                        should_move_to_end_line
                    {
                        // For visual block, calculate the correct final cursor position
                        let target_line = if move_to_end { end_line } else { start_line };

                        if is_append {
                            // For append mode, position cursor on the last character of target line
                            if let Some(line) = editor.buffer().line(target_line) {
                                let line_text = line.trim_end_matches('\n');
                                let line_len = line_text.chars().count();
                                let final_col = if line_len > 0 { line_len - 1 } else { 0 };
                                editor
                                    .buffer_mut()
                                    .cursor_mut()
                                    .set_position(target_line, final_col);
                            }
                        } else {
                            // For insert mode, use the same column as on the first line
                            let cursor = editor.buffer().cursor();
                            let current_col = cursor.col();
                            let inserted_col = if current_col > 0 { current_col - 1 } else { 0 };
                            editor
                                .buffer_mut()
                                .cursor_mut()
                                .set_position(target_line, inserted_col);
                        }
                    } else {
                        let cursor = editor.buffer_mut().cursor_mut();
                        if cursor.col() > 0 {
                            cursor.move_left(1);
                        }
                    }
                }
            }
            // Ctrl-W - Delete word backward
            KeyCode::Char('w') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                helpers::delete_word_backward_insert(editor)?;
            }
            // Ctrl-U - Delete to start of line
            KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                helpers::delete_to_line_start_insert(editor)?;
            }
            // Ctrl-T - Indent current line in insert mode
            KeyCode::Char('t') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                helpers::indent_line_insert(editor)?;
            }
            // Ctrl-D - Dedent current line in insert mode
            KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                helpers::dedent_line_insert(editor)?;
            }
            // Ctrl-Space - Request code completion
            KeyCode::Char(' ') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.request_completion();
            }
            // Ctrl-O - Request code completion (vim omni-completion)
            KeyCode::Char('o') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.request_completion();
            }
            // Ctrl-N - Next completion item
            KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                if editor.completion_menu().is_visible() {
                    editor.completion_next();
                }
            }
            // Ctrl-P - Previous completion item
            KeyCode::Char('p') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                if editor.completion_menu().is_visible() {
                    editor.completion_previous();
                }
            }
            // Tab - Accept completion if menu is visible, otherwise insert tab
            KeyCode::Tab if editor.completion_menu().is_visible() => {
                editor.accept_completion();
            }
            KeyCode::Char(c) => {
                helpers::insert_char(editor, c)?;
            }
            KeyCode::Enter => {
                // If completion menu is visible, accept the selected completion
                if editor.completion_menu().is_visible() {
                    editor.accept_completion();
                } else {
                    helpers::insert_newline(editor)?;
                }
            }
            KeyCode::Backspace => {
                helpers::delete_char_before_cursor(editor)?;
            }
            KeyCode::Left => {
                let cursor = editor.buffer_mut().cursor_mut();
                if cursor.col() > 0 {
                    cursor.move_left(1);
                }
            }
            KeyCode::Right => {
                helpers::move_right(editor);
            }
            KeyCode::Up => {
                helpers::move_up(editor);
            }
            KeyCode::Down => {
                helpers::move_down(editor);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles input in Visual mode
    fn handle_visual_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
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

    /// Handles input in Search mode
    fn handle_search_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Char(ch) => {
                // Add character to search buffer
                editor.append_to_search_buffer(ch);
                // Incremental search: update highlighting immediately
                editor.execute_search();
            }
            KeyCode::Backspace => {
                // Remove last character from search buffer
                editor.backspace_search_buffer();
                // Incremental search: update highlighting after backspace
                editor.execute_search();
            }
            KeyCode::Enter => {
                // Execute the search and accept it
                editor.execute_search();
                // Return to visual mode if visual_start is set, otherwise normal mode
                if editor.visual_start().is_some() {
                    editor.set_mode(Mode::Visual);
                } else {
                    editor.set_mode(Mode::Normal);
                }
            }
            KeyCode::Esc => {
                // Cancel search mode
                // BUG FIX: Restore cursor to position before search started
                editor.restore_search_start_position();
                editor.clear_search_buffer();
                // Return to visual mode if visual_start is set, otherwise normal mode
                if editor.visual_start().is_some() {
                    editor.set_mode(Mode::Visual);
                } else {
                    editor.set_mode(Mode::Normal);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles input in Replace mode
    fn handle_replace_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Esc => {
                // Save last insert position
                let cursor_line = editor.buffer().cursor().line();
                let cursor_col = editor.buffer().cursor().col();
                editor.last_insert_position = Some((cursor_line, cursor_col));

                editor.finalize_change_building();
                editor.update_last_inserted_register();
                editor.mark_buffer_modified();

                // Move cursor left one position (unless at column 0)
                if cursor_col > 0 {
                    editor.buffer_mut().cursor_mut().move_left(1);
                }

                editor.set_mode(Mode::Normal);
            }
            KeyCode::Char(c) => {
                // Replace character under cursor with the typed character
                let line_idx = editor.buffer().cursor().line();
                let col = editor.buffer().cursor().col();

                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_text = line.trim_end_matches('\n');
                    let chars: Vec<char> = line_text.chars().collect();

                    if col < chars.len() {
                        // Delete character under cursor
                        let deleted = editor.buffer_mut().delete_range(line_idx, col, line_idx, col + 1);

                        // Insert new character
                        let new_char = c.to_string();
                        editor.buffer_mut().insert_text_at(line_idx, col, &new_char);

                        // Track the change
                        let cursor_before = (line_idx, col);
                        let delete_change = Change::delete(
                            Range::new((line_idx, col), (line_idx, col + 1)),
                            deleted,
                            cursor_before,
                        );
                        let insert_change = Change::insert(
                            (line_idx, col),
                            new_char.clone(),
                            cursor_before,
                        );
                        let change = Change::composite(
                            vec![delete_change, insert_change],
                            cursor_before,
                            (line_idx, col + 1),
                        );
                        editor.add_change(change);

                        // Move cursor forward
                        editor.buffer_mut().cursor_mut().move_right(1);
                    } else {
                        // At end of line, just insert (like append)
                        helpers::insert_char(editor, c)?;
                    }
                }
            }
            KeyCode::Enter => {
                // In replace mode, Enter inserts a newline (breaking the line)
                helpers::insert_newline(editor)?;
            }
            KeyCode::Backspace => {
                // Backspace in replace mode moves cursor left without deleting
                let cursor = editor.buffer_mut().cursor_mut();
                if cursor.col() > 0 {
                    cursor.move_left(1);
                }
            }
            KeyCode::Left => {
                let cursor = editor.buffer_mut().cursor_mut();
                if cursor.col() > 0 {
                    cursor.move_left(1);
                }
            }
            KeyCode::Right => {
                helpers::move_right(editor);
            }
            KeyCode::Up => {
                helpers::move_up(editor);
            }
            KeyCode::Down => {
                helpers::move_down(editor);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles input in Picker mode
    fn handle_picker_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        // Ctrl-C - cancel picker (same as Escape)
        if key_event.code == KeyCode::Char('c')
            && key_event.modifiers.contains(KeyModifiers::CONTROL)
        {
            editor.close_picker();
            editor.set_mode(Mode::Normal);
            return Ok(());
        }

        match key_event.code {
            // Escape - cancel picker
            KeyCode::Esc => {
                editor.close_picker();
                editor.set_mode(Mode::Normal);
            }
            // Enter - select current item
            KeyCode::Enter => {
                if let Some(picker) = editor.picker() {
                    let picker_mode = picker.mode().clone();

                    if let Some(result) = picker.selected_result() {
                        if picker_mode == crate::editor::PickerMode::Custom {
                            // Custom mode - apply code action
                            let action_index = result.line; // We stored index in line field

                            // Close picker first
                            editor.close_picker();
                            editor.set_mode(Mode::Normal);

                            // Apply the selected code action
                            editor.apply_code_action(action_index);
                        } else if picker_mode == crate::editor::PickerMode::Completion {
                            // Completion mode - apply completion
                            let completion_index = result.line; // We stored index in line field

                            // Close picker first
                            editor.close_picker();
                            editor.set_mode(Mode::Normal);

                            // Apply the selected completion
                            editor.apply_completion(completion_index);
                        } else if picker_mode == crate::editor::PickerMode::LspLocations {
                            // LSP locations mode - navigate to location
                            let location_index = result.line; // We stored index in line field

                            // Close picker first
                            editor.close_picker();
                            editor.set_mode(Mode::Normal);

                            // Navigate to the LSP location
                            editor.navigate_to_lsp_location(location_index);
                        } else {
                            // Regular mode - load file and jump to location
                            let location = result.location.clone();
                            let line = result.line;
                            let col = result.col;

                            // Close picker first
                            editor.close_picker();
                            editor.set_mode(Mode::Normal);

                            // Load the file
                            if let Err(e) = editor.load_file(&location) {
                                editor.set_lsp_status(format!("Failed to load file {}: {}", location, e));
                                return Ok(());
                            }

                            // Jump to line/col
                            editor.buffer_mut().cursor_mut().set_position(line, col);
                        }
                    }
                } else {
                    // No picker, return to normal
                    editor.set_mode(Mode::Normal);
                }
            }
            // Backspace - remove character before cursor
            KeyCode::Backspace => {
                if let Some(picker) = editor.picker_mut() {
                    picker.backspace_query();
                }
                // Mark query changed for debouncing
                editor.mark_picker_query_changed();
            }
            // Delete - remove character at cursor
            KeyCode::Delete => {
                if let Some(picker) = editor.picker_mut() {
                    picker.delete_char();
                }
                // Mark query changed for debouncing
                editor.mark_picker_query_changed();
            }
            // Left arrow - move cursor left in query
            KeyCode::Left => {
                if let Some(picker) = editor.picker_mut() {
                    picker.move_cursor_left();
                }
            }
            // Right arrow - move cursor right in query
            KeyCode::Right => {
                if let Some(picker) = editor.picker_mut() {
                    picker.move_cursor_right();
                }
            }
            // Home - move cursor to beginning of query
            KeyCode::Home => {
                if let Some(picker) = editor.picker_mut() {
                    picker.move_cursor_home();
                }
            }
            // End - move cursor to end of query
            KeyCode::End => {
                if let Some(picker) = editor.picker_mut() {
                    picker.move_cursor_end();
                }
            }
            // Ctrl-N - move down in results
            KeyCode::Char('n') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let mut moved = false;
                if let Some(picker) = editor.picker_mut() {
                    picker.move_down();
                    moved = true;
                }
                if moved {
                    editor.mark_picker_selection_changed();
                }
            }
            // Ctrl-P - move up in results
            KeyCode::Char('p') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let mut moved = false;
                if let Some(picker) = editor.picker_mut() {
                    picker.move_up();
                    moved = true;
                }
                if moved {
                    editor.mark_picker_selection_changed();
                }
            }
            // Down arrow - move down in results
            KeyCode::Down => {
                let mut moved = false;
                if let Some(picker) = editor.picker_mut() {
                    picker.move_down();
                    moved = true;
                }
                if moved {
                    editor.mark_picker_selection_changed();
                }
            }
            // Up arrow - move up in results
            KeyCode::Up => {
                let mut moved = false;
                if let Some(picker) = editor.picker_mut() {
                    picker.move_up();
                    moved = true;
                }
                if moved {
                    editor.mark_picker_selection_changed();
                }
            }
            // Any other character - insert at cursor
            KeyCode::Char(ch) => {
                if let Some(picker) = editor.picker_mut() {
                    picker.insert_char(ch);
                }
                // Mark query changed for debouncing
                editor.mark_picker_query_changed();
            }
            _ => {}
        }

        Ok(())
    }

    /// Polls for the next event
    pub fn poll_event() -> Result<Option<Event>> {
        // Use a very short timeout to keep the event loop responsive
        // This allows status updates and rendering to happen frequently
        if event::poll(std::time::Duration::from_millis(16))? {
            // ~60 FPS
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }

    /// Handles input in HoverWindow mode
    fn handle_hover_window_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
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
    fn handle_filetree_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
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

    /// Handles input in SubstituteConfirm mode
    /// Keys: y (yes), n (no/skip), a (all), q (quit), l (last - substitute and quit)
    fn handle_substitute_confirm_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // Confirm this substitution
                editor.confirm_substitute();
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                // Skip this match
                editor.skip_substitute();
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                // Substitute all remaining matches
                editor.confirm_all_substitutes();
            }
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                // Quit without substituting remaining matches
                editor.end_substitute_confirm();
            }
            KeyCode::Char('l') | KeyCode::Char('L') => {
                // Substitute this match and quit
                editor.confirm_substitute_and_quit();
            }
            _ => {
                // Show prompt in status
                editor.set_lsp_status("replace with ... (y/n/a/q/l)".to_string());
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
