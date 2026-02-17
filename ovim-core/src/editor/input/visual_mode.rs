//! Visual mode handler
//!
//! Handles all input events in Visual, VisualLine, and VisualBlock modes including:
//! - Visual mode motions (h/j/k/l, w/b/e, etc.)
//! - Visual mode operators (d, c, y, >, <, ~, u, U)
//! - Visual mode text objects (iw, aw, i", a{, etc.)
//! - Visual block operations (I, A, c, r)
//! - Visual mode commands (o to swap cursor, gv to reselect)
//! - Visual mode search (/ and ?)

use crate::editor::{Editor, Motions, RegisterType, TextObjectRange, TextObjects};
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

use super::char_motion;
use super::helpers;
use super::numbers;
use crate::editor::input_state::InputState;

/// Apply a text object range to the visual selection.
/// If `inclusive` is true, end_col is used directly; otherwise it's decremented by 1.
fn apply_text_object(editor: &mut Editor, range: Option<TextObjectRange>, inclusive: bool) {
    if let Some(range) = range {
        editor.set_visual_start(range.start_line, range.start_col);
        let end_col = if inclusive {
            range.end_col
        } else {
            range.end_col.saturating_sub(1)
        };
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(range.end_line, end_col);
    }
}

/// Handles input in Visual mode (Visual, VisualLine, VisualBlock)
pub fn handle_visual_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    // =====================================================================
    // INPUT STATE CHECK (must happen before mode-specific handling)
    // =====================================================================
    // If we're awaiting a character (for f/t/F/T motions), handle that first
    // before processing any visual mode specific keys. This prevents conflicts
    // where the target character (like 'e' in 'fe') would be interpreted as
    // a motion command instead of the search target.
    if let InputState::AwaitingChar { motion, operator } = editor.input_state().clone() {
        return char_motion::handle_char_motion(editor, key_event, motion, operator);
    }

    // Handle pending command for visual block replace and g prefix
    if let Some(pending) = editor.pending_command() {
        editor.clear_pending_command();
        match (pending, key_event.code) {
            ('g', KeyCode::Char('g')) => {
                // gg - go to first line (or line specified by count)
                let target_line = if let Some(count) = editor.count() {
                    count.saturating_sub(1)
                } else {
                    0
                };

                let is_visual_block = editor.mode() == Mode::VisualBlock;
                let current_col = editor.buffer().cursor().col();
                let cursor = editor.buffer_mut().cursor_mut();
                cursor.set_line(target_line);

                if !is_visual_block {
                    cursor.set_col(0);
                    cursor.update_desired_col(0);
                } else {
                    cursor.set_col(current_col);
                    cursor.update_desired_col(current_col);
                }

                helpers::clamp_cursor_to_line(editor);
                editor.clear_count();
                return Ok(());
            }
            ('g', KeyCode::Char('a')) if key_event.modifiers.contains(Modifiers::CONTROL) => {
                // g Ctrl-A: Sequential increment in visual selection
                numbers::sequential_modify_numbers(editor, 1)?;
                helpers::exit_visual_mode_to_normal(editor);
                return Ok(());
            }
            ('g', KeyCode::Char('x')) if key_event.modifiers.contains(Modifiers::CONTROL) => {
                // g Ctrl-X: Sequential decrement in visual selection
                numbers::sequential_modify_numbers(editor, -1)?;
                helpers::exit_visual_mode_to_normal(editor);
                return Ok(());
            }
            ('g', KeyCode::Char('n')) => {
                // gn - extend selection to next search match
                if !editor.search_select_next() {
                    editor.set_lsp_status("Pattern not found".to_string());
                }
                editor.clear_count();
                return Ok(());
            }
            ('g', KeyCode::Char('N')) => {
                // gN - extend selection to previous search match
                if !editor.search_select_prev() {
                    editor.set_lsp_status("Pattern not found".to_string());
                }
                editor.clear_count();
                return Ok(());
            }
            ('r', KeyCode::Char(ch)) => {
                // r{char} in visual block - replace all characters in selection with ch
                if editor.mode() == Mode::VisualBlock {
                    helpers::replace_visual_selection(editor, ch)?;
                    helpers::exit_visual_mode_to_normal(editor);
                    return Ok(());
                }
            }
            ('i', KeyCode::Char('w')) => {
                apply_text_object(editor, TextObjects::inner_word(editor.buffer()), false);
                return Ok(());
            }
            ('a', KeyCode::Char('w')) => {
                apply_text_object(editor, TextObjects::around_word(editor.buffer()), false);
                return Ok(());
            }
            ('i', KeyCode::Char('p')) => {
                apply_text_object(editor, TextObjects::inner_paragraph(editor.buffer()), true);
                return Ok(());
            }
            ('a', KeyCode::Char('p')) => {
                apply_text_object(editor, TextObjects::around_paragraph(editor.buffer()), true);
                return Ok(());
            }
            ('i', KeyCode::Char('"')) | ('i', KeyCode::Char('\'')) | ('i', KeyCode::Char('`')) => {
                let quote = match key_event.code {
                    KeyCode::Char(c) => c,
                    _ => return Ok(()),
                };
                apply_text_object(
                    editor,
                    TextObjects::quoted_string(editor.buffer(), quote, false),
                    false,
                );
                return Ok(());
            }
            ('a', KeyCode::Char('"')) | ('a', KeyCode::Char('\'')) | ('a', KeyCode::Char('`')) => {
                let quote = match key_event.code {
                    KeyCode::Char(c) => c,
                    _ => return Ok(()),
                };
                apply_text_object(
                    editor,
                    TextObjects::quoted_string(editor.buffer(), quote, true),
                    false,
                );
                return Ok(());
            }
            ('i', KeyCode::Char('(')) | ('i', KeyCode::Char(')')) | ('i', KeyCode::Char('b')) => {
                apply_text_object(
                    editor,
                    TextObjects::paired_delimiters(editor.buffer(), '(', ')', false),
                    false,
                );
                return Ok(());
            }
            ('a', KeyCode::Char('(')) | ('a', KeyCode::Char(')')) | ('a', KeyCode::Char('b')) => {
                apply_text_object(
                    editor,
                    TextObjects::paired_delimiters(editor.buffer(), '(', ')', true),
                    false,
                );
                return Ok(());
            }
            ('i', KeyCode::Char('[')) | ('i', KeyCode::Char(']')) => {
                apply_text_object(
                    editor,
                    TextObjects::paired_delimiters(editor.buffer(), '[', ']', false),
                    false,
                );
                return Ok(());
            }
            ('a', KeyCode::Char('[')) | ('a', KeyCode::Char(']')) => {
                apply_text_object(
                    editor,
                    TextObjects::paired_delimiters(editor.buffer(), '[', ']', true),
                    false,
                );
                return Ok(());
            }
            ('i', KeyCode::Char('{')) | ('i', KeyCode::Char('}')) | ('i', KeyCode::Char('B')) => {
                apply_text_object(
                    editor,
                    TextObjects::paired_delimiters(editor.buffer(), '{', '}', false),
                    false,
                );
                return Ok(());
            }
            ('a', KeyCode::Char('{')) | ('a', KeyCode::Char('}')) | ('a', KeyCode::Char('B')) => {
                apply_text_object(
                    editor,
                    TextObjects::paired_delimiters(editor.buffer(), '{', '}', true),
                    false,
                );
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
        KeyCode::Char(' ') => {
            editor.start_ai_prompt_from_visual()?;
        }
        // Half-page scroll down (Ctrl-D) — must come before 'd' delete handler
        KeyCode::Char('d') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            let half_page = editor.half_page_scroll();
            let count = editor.count().unwrap_or(half_page);
            let max_line = editor.buffer().line_count().saturating_sub(1);

            let cursor = editor.buffer_mut().cursor_mut();
            let new_line = (cursor.line() + count).min(max_line);
            cursor.set_line(new_line);
            helpers::clamp_cursor_to_line(editor);
            editor.clear_count();
        }
        // Half-page scroll up (Ctrl-U) — must come before 'u' lowercase handler
        KeyCode::Char('u') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            let half_page = editor.half_page_scroll();
            let count = editor.count().unwrap_or(half_page);
            let cursor = editor.buffer_mut().cursor_mut();
            let new_line = cursor.line().saturating_sub(count);
            cursor.set_line(new_line);
            helpers::clamp_cursor_to_line(editor);
            editor.clear_count();
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
            editor.set_visual_block_dollar(false);
            helpers::move_left(editor);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            helpers::move_down(editor);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            helpers::move_up(editor);
        }
        KeyCode::Char('l') | KeyCode::Right => {
            editor.set_visual_block_dollar(false);
            helpers::move_right(editor);
        }
        KeyCode::Char('w') => {
            editor.set_visual_block_dollar(false);
            let count = editor.effective_count();
            Motions::word_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        KeyCode::Char('b') => {
            editor.set_visual_block_dollar(false);
            let count = editor.effective_count();
            Motions::word_backward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        KeyCode::Char('e') => {
            editor.set_visual_block_dollar(false);
            let count = editor.effective_count();
            Motions::word_end_forward(editor.buffer_mut(), count);
            editor.clear_count();
        }
        KeyCode::Char('0') => {
            editor.set_visual_block_dollar(false);
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
                // Set "extend to end-of-line" flag so each line in the block
                // is deleted/yanked to its own end, not to a fixed column.
                editor.set_visual_block_dollar(true);
                let line_idx = editor.buffer().cursor().line();
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_len = line.trim_end_matches('\n').chars().count();
                    let col = if line_len > 0 { line_len - 1 } else { 0 };
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
                editor.buffer().line_count().saturating_sub(1)
            };
            let is_visual_block = editor.mode() == Mode::VisualBlock;
            let current_col = editor.buffer().cursor().col();
            let cursor = editor.buffer_mut().cursor_mut();
            cursor.set_line(target_line);

            if !is_visual_block {
                cursor.set_col(0);
                cursor.update_desired_col(0);
            } else {
                cursor.set_col(current_col);
                cursor.update_desired_col(current_col);
            }

            helpers::clamp_cursor_to_line(editor);
            editor.clear_count();
        }
        KeyCode::Char('g') => {
            // g - first key of gg (go to first line with optional count)
            editor.set_pending_command('g');
        }
        // Find character forward (f)
        KeyCode::Char('f') => {
            use crate::editor::input_state::{CharMotion, InputState};
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::Find,
                operator: None,
            });
        }
        // Find character backward (F)
        KeyCode::Char('F') => {
            use crate::editor::input_state::{CharMotion, InputState};
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::FindBack,
                operator: None,
            });
        }
        // Till character forward (t)
        KeyCode::Char('t') => {
            use crate::editor::input_state::{CharMotion, InputState};
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::Till,
                operator: None,
            });
        }
        // Till character backward (T)
        KeyCode::Char('T') => {
            use crate::editor::input_state::{CharMotion, InputState};
            editor.set_input_state(InputState::AwaitingChar {
                motion: CharMotion::TillBack,
                operator: None,
            });
        }
        // Repeat the last find motion in visual modes (`;`/`,`).
        KeyCode::Char(';') => {
            editor.repeat_last_find(false);
        }
        KeyCode::Char(',') => {
            editor.repeat_last_find(true);
        }
        // Jump to matching bracket (%)
        KeyCode::Char('%') => {
            Motions::jump_to_matching_bracket(editor.buffer_mut());
            editor.clear_count();
        }
        // Search forward in visual mode
        KeyCode::Char('/') => {
            // Save visual search state for extending selection after search
            if let Some((anchor_line, anchor_col)) = editor.visual_start() {
                let mode = editor.mode();
                editor.set_visual_search_state((anchor_line, anchor_col), mode);
            }
            editor.clear_search_buffer();
            editor.set_search_forward(true);
            editor.save_search_start_position();
            editor.set_mode(Mode::Search);
        }
        // Search backward in visual mode
        KeyCode::Char('?') => {
            // Save visual search state for extending selection after search
            if let Some((anchor_line, anchor_col)) = editor.visual_start() {
                let mode = editor.mode();
                editor.set_visual_search_state((anchor_line, anchor_col), mode);
            }
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
        // Search forward for selected text (* in visual mode)
        KeyCode::Char('*') => {
            if !helpers::search_visual_selection_forward(editor) {
                editor.set_lsp_status("Pattern not found".to_string());
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Search backward for selected text (# in visual mode)
        KeyCode::Char('#') => {
            if !helpers::search_visual_selection_backward(editor) {
                editor.set_lsp_status("Pattern not found".to_string());
            }
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Delete selection
        KeyCode::Char('d') | KeyCode::Char('x') => {
            helpers::delete_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Yank selection
        KeyCode::Char('y') => {
            // Move cursor to start of selection before yanking (Vim behavior)
            if let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection()
            {
                let mode = editor.mode();
                helpers::yank_visual_selection(editor)?;
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(start_line, start_col);
                // Flash the yanked region
                if mode == Mode::VisualLine {
                    editor.set_yank_flash_lines(start_line, end_line);
                } else {
                    editor.set_yank_flash_range(start_line, start_col, end_line, end_col);
                }
            }
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
        KeyCode::Char('v') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            if editor.mode() == Mode::VisualBlock {
                helpers::exit_visual_mode_to_normal(editor);
            } else {
                editor.set_mode(Mode::VisualBlock);
            }
        }
        KeyCode::Char('v') => {
            if editor.mode() == Mode::Visual {
                helpers::exit_visual_mode_to_normal(editor);
            } else {
                // Switching to Visual mode from VisualLine or VisualBlock
                editor.set_mode(Mode::Visual);
            }
        }
        KeyCode::Char('V') => {
            if editor.mode() == Mode::VisualLine {
                helpers::exit_visual_mode_to_normal(editor);
            } else {
                // Switching to VisualLine mode
                if let Some((anchor_line, _)) = editor.visual_start() {
                    editor.set_visual_start(anchor_line, 0);
                } else {
                    let cursor = editor.buffer().cursor();
                    editor.set_visual_start(cursor.line(), 0);
                }
                editor.set_mode(Mode::VisualLine);
            }
        }
        // Visual block insert/append
        KeyCode::Char('I') => {
            if editor.mode() == Mode::VisualBlock {
                // Insert at beginning of block on each line
                if let Some(((start_line, start_col), (end_line, _))) = editor.visual_selection() {
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
                if let Some(((start_line, _), (end_line, end_col))) = editor.visual_selection() {
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
            helpers::toggle_case_visual_selection(editor)?;
            helpers::exit_visual_mode_to_normal(editor);
        }
        // Paste in visual mode (replace selection)
        KeyCode::Char('p') | KeyCode::Char('P') => {
            let is_visual_line = editor.mode() == Mode::VisualLine;

            // 1. Save paste text + type from register
            let (paste_text, paste_type) = editor.get_from_register_with_type();

            // 2. Delete the visual selection (saves to numbered registers + unnamed)
            helpers::delete_visual_selection(editor)?;

            // 3. Save deleted text from unnamed register
            let (deleted_text, deleted_type) = editor.get_from_register_with_type();

            // 4. Write paste text to unnamed register
            editor.registers.set_with_type(None, paste_text, paste_type);

            // 5. Branch on paste type
            if is_visual_line || paste_type == RegisterType::Line {
                // Linewise: use paste_before to insert at current line
                helpers::paste_before(editor, 1)?;
            } else {
                // Character: adjust cursor and paste_after
                let cursor_col = editor.buffer().cursor().col();
                if cursor_col > 0 {
                    editor.buffer_mut().cursor_mut().set_col(cursor_col - 1);
                }
                helpers::paste_after(editor, 1)?;
            }

            // 6. Set unnamed register to deleted text (so next p pastes the replaced text)
            editor
                .registers
                .set_with_type(None, deleted_text, deleted_type);

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
                let expand_tab = editor.options.expand_tab;
                let cursor_before = editor.cursor_position();
                let ((), edits) = editor.buffer_mut().record(|buf| {
                    let _ = helpers::auto_indent_lines(
                        buf,
                        start_line,
                        end_line + 1,
                        tab_width,
                        expand_tab,
                    );
                });
                if !edits.is_empty() {
                    let cursor_after = editor.cursor_position();
                    // push_recorded_undo() calls mark_buffer_modified() internally
                    editor.push_recorded_undo(edits, cursor_before, cursor_after);
                }
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
