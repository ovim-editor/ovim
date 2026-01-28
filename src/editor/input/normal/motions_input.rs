//! Motion commands in normal mode.
//!
//! Simple cursor movement commands:
//! h, j, k, l, w, W, b, B, e, E, 0, $, ^, _, +, -, G, %, {, }, (, ), ;, ,, n, N, *, #, K

use crate::editor::input::helpers;
use crate::editor::{Editor, FindDirection, FindType, Motions, Search};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Try to handle a motion command.
///
/// Returns `Ok(true)` if the key was handled, `Ok(false)` otherwise.
pub fn try_handle(editor: &mut Editor, key_event: KeyEvent) -> Result<bool> {
    // Handle Ctrl key combinations first (must be checked before regular keys)
    if key_event.modifiers.contains(KeyModifiers::CONTROL) {
        return try_handle_ctrl_motion(editor, key_event);
    }

    // Handle regular motions
    match key_event.code {
        // Basic motions
        KeyCode::Char('h') | KeyCode::Left => {
            helpers::move_left(editor);
            Ok(true)
        }
        KeyCode::Char('j') | KeyCode::Down => {
            helpers::move_down(editor);
            Ok(true)
        }
        KeyCode::Char('k') | KeyCode::Up => {
            helpers::move_up(editor);
            Ok(true)
        }
        KeyCode::Char('l') | KeyCode::Right => {
            helpers::move_right(editor);
            Ok(true)
        }

        // K - show hover information (LSP)
        KeyCode::Char('K') => {
            editor.request_hover();
            editor.clear_count();
            Ok(true)
        }

        // Line motions
        KeyCode::Char('0') => {
            // 0 is either a motion or count digit
            if editor.count().is_some() {
                editor.append_count(0);
            } else {
                editor.buffer_mut().cursor_mut().set_col(0);
                editor.clear_count();
            }
            Ok(true)
        }
        KeyCode::Char('$') => {
            let line_idx = editor.buffer().cursor().line();
            if let Some(line) = editor.buffer().line(line_idx) {
                let line_len = line.trim_end_matches('\n').chars().count();
                let col = if line_len > 0 { line_len - 1 } else { 0 };
                let cursor = editor.buffer_mut().cursor_mut();
                cursor.set_col(col);
                cursor.update_desired_col(usize::MAX);
            }
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('^') => {
            Motions::first_non_blank(editor.buffer_mut());
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('_') => {
            Motions::first_non_blank_underscore(editor.buffer_mut());
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('+') => {
            let count = editor.effective_count();
            Motions::plus_motion(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('-') => {
            let count = editor.effective_count();
            Motions::minus_motion(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }

        // Count prefix (digits 1-9)
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let digit = c.to_digit(10).unwrap() as usize;
            if digit != 0 || editor.count().is_some() {
                editor.append_count(digit);
            }
            Ok(true)
        }

        // Word motions
        KeyCode::Char('w') => {
            let count = editor.effective_count();
            Motions::word_forward(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('W') => {
            let count = editor.effective_count();
            Motions::word_forward_big(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('b') => {
            let count = editor.effective_count();
            Motions::word_backward(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('B') => {
            let count = editor.effective_count();
            Motions::word_backward_big(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('e') => {
            let count = editor.effective_count();
            Motions::word_end_forward(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('E') => {
            let count = editor.effective_count();
            Motions::word_end_forward_big(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }

        // File motions
        KeyCode::Char('G') => {
            let target_line = if let Some(count) = editor.count() {
                count.saturating_sub(1)
            } else {
                editor.buffer().line_count().saturating_sub(1)
            };
            editor.add_jump();
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(target_line, 0);
            editor.add_jump();
            editor.clear_count();
            Ok(true)
        }

        // Jump to matching bracket
        KeyCode::Char('%') => {
            Motions::jump_to_matching_bracket(editor.buffer_mut());
            editor.clear_count();
            Ok(true)
        }

        // Paragraph motions
        KeyCode::Char('}') => {
            let count = editor.effective_count();
            Motions::paragraph_forward(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('{') => {
            let count = editor.effective_count();
            Motions::paragraph_backward(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }

        // Sentence motions
        KeyCode::Char(')') => {
            let count = editor.effective_count();
            Motions::sentence_forward(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('(') => {
            let count = editor.effective_count();
            Motions::sentence_backward(editor.buffer_mut(), count);
            editor.clear_count();
            Ok(true)
        }

        // Search next/previous
        KeyCode::Char('n') => {
            editor.search_next();
            Ok(true)
        }
        KeyCode::Char('N') => {
            editor.search_prev();
            Ok(true)
        }

        // Search word under cursor
        KeyCode::Char('*') => {
            search_word_forward(editor);
            Ok(true)
        }
        KeyCode::Char('#') => {
            search_word_backward(editor);
            Ok(true)
        }

        // Repeat find motion
        KeyCode::Char(';') => {
            repeat_find_motion(editor, false);
            Ok(true)
        }
        KeyCode::Char(',') => {
            repeat_find_motion(editor, true);
            Ok(true)
        }

        _ => Ok(false),
    }
}

/// Handle Ctrl+key combinations for motions and scrolling.
fn try_handle_ctrl_motion(editor: &mut Editor, key_event: KeyEvent) -> Result<bool> {
    match key_event.code {
        // Scroll commands
        KeyCode::Char('d') => {
            let half_page = editor.half_page_scroll();
            let count = editor.count().unwrap_or(half_page);
            let max_line = editor.buffer().line_count().saturating_sub(1);

            let cursor = editor.buffer_mut().cursor_mut();
            let new_line = (cursor.line() + count).min(max_line);
            cursor.set_line(new_line);
            helpers::clamp_cursor_to_line(editor);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('u') => {
            let half_page = editor.half_page_scroll();
            let count = editor.count().unwrap_or(half_page);
            let cursor = editor.buffer_mut().cursor_mut();
            let new_line = cursor.line().saturating_sub(count);
            cursor.set_line(new_line);
            helpers::clamp_cursor_to_line(editor);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('e') => {
            let count = editor.effective_count();
            editor.scroll_viewport_down(count);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('y') => {
            let count = editor.effective_count();
            editor.scroll_viewport_up(count);
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('f') => {
            editor.scroll_page_down();
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('b') => {
            editor.scroll_page_up();
            editor.clear_count();
            Ok(true)
        }

        // Jump commands
        KeyCode::Char('i') => {
            editor.jump_forward();
            Ok(true)
        }
        KeyCode::Char('o') => {
            editor.jump_back();
            Ok(true)
        }
        KeyCode::Char('t') => {
            editor.tag_pop();
            Ok(true)
        }

        // Quit
        KeyCode::Char('q') => {
            editor.quit();
            Ok(true)
        }

        // Window commands prefix
        KeyCode::Char('w') => {
            editor.set_pending_command('W');
            Ok(true)
        }

        // Number increment/decrement
        KeyCode::Char('a') => {
            let count = editor.effective_count();
            super::super::numbers::increment_number(editor, count)?;
            editor.clear_count();
            Ok(true)
        }
        KeyCode::Char('x') => {
            let count = editor.effective_count();
            super::super::numbers::decrement_number(editor, count)?;
            editor.clear_count();
            Ok(true)
        }

        _ => Ok(false),
    }
}

/// Search for word under cursor (forward)
fn search_word_forward(editor: &mut Editor) {
    if let Some((word, _, _)) = editor.buffer().word_under_cursor() {
        let pattern = format!(r"\b{}\b", regex::escape(&word));
        let mut search = Search::new_with_options(
            pattern,
            true,
            editor.options.ignorecase,
            editor.options.smartcase,
        );
        let cursor = editor.buffer().cursor();

        if let Some((line, col, _)) =
            search.find_next(editor.buffer(), cursor.line(), cursor.col() + 1)
        {
            editor.buffer_mut().cursor_mut().set_position(line, col);
        }
        editor.set_current_search(search);
    }
}

/// Search for word under cursor (backward)
fn search_word_backward(editor: &mut Editor) {
    if let Some((word, _, _)) = editor.buffer().word_under_cursor() {
        let pattern = format!(r"\b{}\b", regex::escape(&word));
        let mut search = Search::new_with_options(
            pattern,
            false,
            editor.options.ignorecase,
            editor.options.smartcase,
        );
        let cursor = editor.buffer().cursor();

        let search_col = if cursor.col() > 0 {
            cursor.col() - 1
        } else {
            0
        };
        if let Some((line, col, _)) = search.find_next(editor.buffer(), cursor.line(), search_col) {
            editor.buffer_mut().cursor_mut().set_position(line, col);
        }
        editor.set_current_search(search);
    }
}

/// Repeat the last f/F/t/T motion
fn repeat_find_motion(editor: &mut Editor, reverse: bool) {
    if let Some((ch, find_type, direction)) = editor.get_last_find() {
        let count = editor.effective_count();

        let direction = if reverse {
            match direction {
                FindDirection::Forward => FindDirection::Backward,
                FindDirection::Backward => FindDirection::Forward,
            }
        } else {
            direction
        };

        match (find_type, direction) {
            (FindType::Find, FindDirection::Forward) => {
                Motions::find_char_forward(editor.buffer_mut(), ch, count);
            }
            (FindType::Find, FindDirection::Backward) => {
                Motions::find_char_backward(editor.buffer_mut(), ch, count);
            }
            (FindType::Till, FindDirection::Forward) => {
                if !reverse {
                    // For ; after t, skip past current target
                    let col = editor.buffer().cursor().col();
                    editor.buffer_mut().cursor_mut().set_col(col + 1);
                    if !Motions::till_char_forward(editor.buffer_mut(), ch, count) {
                        editor.buffer_mut().cursor_mut().set_col(col);
                    }
                } else {
                    Motions::till_char_forward(editor.buffer_mut(), ch, count);
                }
            }
            (FindType::Till, FindDirection::Backward) => {
                if !reverse {
                    // For ; after T, skip past current target
                    let col = editor.buffer().cursor().col();
                    if col > 0 {
                        editor.buffer_mut().cursor_mut().set_col(col - 1);
                        if !Motions::till_char_backward(editor.buffer_mut(), ch, count) {
                            editor.buffer_mut().cursor_mut().set_col(col);
                        }
                    }
                } else {
                    Motions::till_char_backward(editor.buffer_mut(), ch, count);
                }
            }
        }
    }
    editor.clear_count();
}
