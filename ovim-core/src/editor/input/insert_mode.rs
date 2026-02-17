//! Insert mode handler
//!
//! Handles all input events in Insert mode including:
//! - Character insertion
//! - Backspace/Delete handling
//! - Ctrl+W (delete word backward)
//! - Ctrl+U (delete to line start)
//! - Ctrl+N/Ctrl+P (completion navigation)
//! - Visual block insert state handling
//! - Tab/auto-indent

use crate::editor::{Change, Editor, Range};
use crate::mode::Mode;
use crate::repeat_action::RepeatAction;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

use super::helpers;

fn is_completion_trigger_char(c: char) -> bool {
    matches!(c, '.')
}

fn is_completion_ident_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// Cleans up whitespace-only lines before exiting insert mode.
///
/// Vim behavior: if the current line contains only whitespace when exiting insert mode,
/// remove the whitespace (e.g., o<Esc> should leave an empty line, not an indented one).
///
/// This must be called BEFORE finalize_change_building() so it's part of the undo group.
///
/// Returns true if cleanup was performed (which means cursor shouldn't move left).
fn cleanup_whitespace_only_line(editor: &mut Editor) -> bool {
    let current_line_idx = editor.buffer().cursor().line();
    if let Some(line) = editor.buffer().line(current_line_idx) {
        let line_without_newline = line.trim_end_matches('\n');
        // Check if line is non-empty but only whitespace
        if !line_without_newline.is_empty()
            && line_without_newline.chars().all(|c| c.is_whitespace())
        {
            // Delete the whitespace, leaving just the newline
            let whitespace_len = line_without_newline.chars().count();
            let cursor_before = (current_line_idx, whitespace_len);
            let deleted_text = line_without_newline.to_string();
            let range = Range::new((current_line_idx, 0), (current_line_idx, whitespace_len));

            // Create and apply the delete change (records for undo)
            let change = Change::delete(range, deleted_text, cursor_before);
            if !editor.apply_change_and_record(change) {
                return false;
            }

            // Move cursor to column 0 since we removed the whitespace
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(current_line_idx, 0);
            return true;
        }
    }
    false
}

/// Shared logic for exiting insert mode (Esc, Ctrl-[, Ctrl-C)
fn exit_insert_mode(editor: &mut Editor) {
    // Save last insert position BEFORE moving cursor (this is where we can continue inserting)
    let cursor = editor.buffer().cursor();
    editor.editing.last_insert_position = Some((cursor.line(), cursor.col()));

    // Cleanup whitespace-only lines before finalizing changes
    cleanup_whitespace_only_line(editor);

    editor.finalize_change_building();

    // Check for pending change repeat (cc, C, s, S, cj, ck, etc.)
    // Must be checked BEFORE PendingSemanticChange — they are mutually exclusive.
    if let Some(pending) = editor.take_pending_change_repeat() {
        // Extract typed text from the just-finalized composite
        let inserted_text = editor
            .last_change()
            .map(|c| c.get_inserted_text())
            .unwrap_or_default();

        // Pop insert composite (from ChangeBuilder)
        let insert_undo = editor.pop_last_change();
        // Pop delete undo only if the delete phase actually produced edits
        let delete_undo = pending
            .delete_token
            .and_then(|token| editor.pop_by_token(token));

        // Merge into single undo unit
        let cursor_before = delete_undo
            .as_ref()
            .map(|c| c.cursor_before())
            .unwrap_or_else(|| editor.cursor_position());
        let cursor_after = editor.cursor_position();

        let mut merged = vec![];
        if let Some(d) = delete_undo {
            merged.push(d);
        }
        if let Some(i) = insert_undo {
            merged.push(i);
        }
        if !merged.is_empty() {
            editor.add_change(Change::composite(merged, cursor_before, cursor_after));
        }

        // Set semantic repeat action
        editor.set_repeat_action(RepeatAction::Change {
            delete: Box::new(pending.delete_action),
            inserted_text,
            linewise: pending.linewise,
        });
    }
    // Check for pending semantic change operation (ci", cw, etc.)
    else if let Some(pending) = editor.take_pending_semantic_change() {
        // Get the inserted text from the last change
        let inserted_text = if let Some(last_change) = editor.last_change() {
            last_change.get_inserted_text()
        } else {
            String::new()
        };

        // Remove the composite change that was just added
        editor.pop_last_change();

        // Create the appropriate semantic change
        let cursor_after = (
            editor.buffer().cursor().line(),
            editor.buffer().cursor().col(),
        );

        let semantic_change = if pending.is_word_change {
            Change::change_word(
                inserted_text,
                pending.cursor_before,
                cursor_after,
                pending.old_text,
                pending.old_range,
            )
        } else if pending.is_search_match_change {
            Change::change_search_match(
                pending.search_pattern.unwrap_or_default(),
                pending.search_forward.unwrap_or(true),
                inserted_text,
                pending.cursor_before,
                cursor_after,
                pending.old_text,
                pending.old_range,
            )
        } else if let Some(obj_type) = pending.object_type {
            Change::change_text_object(
                obj_type,
                inserted_text,
                pending.cursor_before,
                cursor_after,
                pending.old_text,
                pending.old_range,
            )
        } else {
            // Shouldn't happen, but fall back to composite
            if let Some(last_change) = editor.last_change() {
                last_change.clone()
            } else {
                Change::composite(vec![], pending.cursor_before, cursor_after)
            }
        };

        editor.add_change(semantic_change);
    }

    // Update the . register with the last inserted text
    editor.update_last_inserted_register();

    // If we were in visual block insert/append mode, replay the changes on all other lines
    let should_move_to_end_line = if let Some((start_line, end_line, col, is_append, move_to_end)) =
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
                        let version_before = editor.buffer().version();
                        editor
                            .buffer_mut()
                            .insert_text_at(line_idx, line_len, &inserted_text);
                        if editor.buffer().version() != version_before {
                            // Track this insertion as a change
                            let change = Change::insert(
                                (line_idx, line_len),
                                inserted_text.clone(),
                                cursor_before,
                            );
                            all_changes.push(change);
                        }
                    }
                } else {
                    // Insert mode: insert at column
                    if let Some(line) = editor.buffer().line(line_idx) {
                        let line_text = line.trim_end_matches('\n');
                        let insert_col = col.min(line_text.chars().count());
                        let version_before = editor.buffer().version();
                        editor
                            .buffer_mut()
                            .insert_text_at(line_idx, insert_col, &inserted_text);
                        if editor.buffer().version() != version_before {
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
            }

            // If multiple lines were affected, wrap in composite for proper undo
            if all_changes.len() > 1 {
                // Remove the last change (first line's change) from undo stack
                editor.pop_last_change();

                // Create composite for all insert changes
                let insert_composite = Change::composite(all_changes, cursor_before, cursor_before);

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

    // Mark buffer modified for LSP didChange — placed after visual block replay
    // so the server sees ALL changes (first line + replayed lines).
    editor.mark_buffer_modified();

    editor.set_mode(Mode::Normal);

    // Move cursor left when exiting insert mode (unless at column 0)

    // If we were in visual block mode, move cursor to appropriate line
    if let Some((start_line, end_line, _col, is_append, move_to_end)) = should_move_to_end_line {
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

/// Handles input in Insert mode
pub fn handle_insert_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Esc => {
            if editor.completion_menu().is_visible() {
                editor.hide_completion_menu();
            }
            exit_insert_mode(editor);
        }
        // Ctrl-[ is equivalent to Esc
        KeyCode::Char('[') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.hide_completion_menu();
            }
            exit_insert_mode(editor);
        }
        // Ctrl-C exits insert mode (like Esc but without triggering InsertLeave)
        KeyCode::Char('c') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.hide_completion_menu();
            }
            exit_insert_mode(editor);
        }
        // Ctrl-W - Delete word backward
        KeyCode::Char('w') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::delete_word_backward_insert(editor)?;
        }
        // Ctrl-U - Delete to start of line
        KeyCode::Char('u') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::delete_to_line_start_insert(editor)?;
        }
        // Ctrl-T - Indent current line in insert mode
        KeyCode::Char('t') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::indent_line_insert(editor)?;
        }
        // Ctrl-D - Dedent current line in insert mode
        KeyCode::Char('d') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::dedent_line_insert(editor)?;
        }
        // Ctrl-H is equivalent to Backspace
        KeyCode::Char('h') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            helpers::delete_char_before_cursor(editor)?;
        }
        // Ctrl-Space - Request code completion
        KeyCode::Char(' ') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.request_completion();
        }
        // Ctrl-O - Request code completion (vim omni-completion)
        KeyCode::Char('o') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.request_completion();
        }
        // Ctrl-N - Next completion item
        KeyCode::Char('n') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.completion_next();
            }
        }
        // Ctrl-P - Previous completion item
        KeyCode::Char('p') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.completion_previous();
            }
        }
        // Ctrl-Y - Accept completion (Vim behavior)
        KeyCode::Char('y') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.completion_menu().is_visible() {
                editor.accept_completion();
            }
        }
        // Tab - Accept completion if menu is visible, otherwise insert tab
        KeyCode::Tab if editor.completion_menu().is_visible() => {
            editor.accept_completion();
        }
        KeyCode::Tab => {
            helpers::insert_tab(editor)?;
        }
        KeyCode::Char(c) => {
            helpers::insert_char(editor, c)?;
            // Basic autocomplete:
            // - Trigger on '.' (member access) immediately
            // - Trigger on '::' after typing the second ':' (Rust/C++ style paths)
            // - Trigger when typing an identifier prefix of length >= 2
            // - If menu is already visible, keep it updated while typing
            if editor.completion_menu().is_visible() {
                if is_completion_trigger_char(c) || is_completion_ident_char(c) {
                    let (_, prefix) = editor.completion_trigger_context();
                    editor.completion_menu_mut().filter(&prefix);
                    editor.request_completion();
                } else {
                    editor.hide_completion_menu();
                }
            } else if is_completion_trigger_char(c) {
                editor.request_completion();
            } else if c == ':' {
                let cursor = editor.buffer().cursor();
                if cursor.col() >= 2 {
                    let line_text = editor
                        .buffer()
                        .line(cursor.line())
                        .unwrap_or_default()
                        .trim_end_matches('\n')
                        .to_string();
                    if crate::unicode::grapheme_at_index(&line_text, cursor.col().saturating_sub(1))
                        == Some(":")
                        && crate::unicode::grapheme_at_index(
                            &line_text,
                            cursor.col().saturating_sub(2),
                        ) == Some(":")
                    {
                        editor.request_completion();
                    }
                }
            } else if is_completion_ident_char(c) {
                let (_, prefix) = editor.completion_trigger_context();
                if prefix.chars().count() >= 2 {
                    editor.request_completion();
                }
            }
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
            if editor.completion_menu().is_visible() {
                let (_, prefix) = editor.completion_trigger_context();
                if prefix.is_empty() {
                    // If we deleted back to the trigger column, keep showing member completions only
                    editor.request_completion();
                } else {
                    editor.completion_menu_mut().filter(&prefix);
                    editor.request_completion();
                }
            }
        }
        KeyCode::Left => {
            if editor.completion_menu().is_visible() {
                editor.hide_completion_menu();
            }
            let cursor = editor.buffer_mut().cursor_mut();
            if cursor.col() > 0 {
                cursor.move_left(1);
            }
        }
        KeyCode::Right => {
            if editor.completion_menu().is_visible() {
                editor.hide_completion_menu();
            }
            helpers::move_right(editor);
        }
        KeyCode::Up => {
            if editor.completion_menu().is_visible() {
                editor.completion_previous();
            } else {
                helpers::move_up(editor);
            }
        }
        KeyCode::Down => {
            if editor.completion_menu().is_visible() {
                editor.completion_next();
            } else {
                helpers::move_down(editor);
            }
        }
        _ => {}
    }
    Ok(())
}
