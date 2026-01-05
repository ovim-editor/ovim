//! Operator + motion handling in normal mode.
//!
//! Handles pending operators combined with motions:
//! - `dd`, `dw`, `d$`, `dj`, `dk`, `d{`, `d}`, `d%`, `dG`, `dgg`
//! - `yy`, `yw`, `y$`, `yj`, `yk`, `y{`, `y}`
//! - `cc`, `cw`, `c$`, `cj`, `ck`, `c{`, `c}`, `cG`, `cgg`
//! - `>>`, `>j`, `>k`, `>G`, `>gg`
//! - `<<`, `<j`, `<k`, `<G`, `<gg`
//! - `zf{motion}`
//! - `gu*`, `gU*`, `g~*`

use crate::editor::input::helpers;
use crate::editor::{
    Change, CharMotion, Editor, InputState, Motions, Operator, Operators, PendingSemanticChange,
    Range, RegisterType,
};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use super::super::case;

/// Try to handle a pending operator with motion.
///
/// Returns `Ok(true)` if the key was handled, `Ok(false)` otherwise.
pub fn try_handle(editor: &mut Editor, key_event: KeyEvent) -> Result<bool> {
    // Skip if we have a text object prefix ('i' or 'a')
    let has_text_obj_prefix = matches!(editor.pending_command(), Some('i') | Some('a'));
    if has_text_obj_prefix {
        return Ok(false);
    }

    let operator = match editor.pending_operator() {
        Some(op) => op,
        None => return Ok(false),
    };

    let count = editor.effective_count();

    // K is not a motion, so operator+K should just cancel the operator
    if key_event.code == KeyCode::Char('K') {
        editor.clear_pending_operator();
        editor.clear_count();
        return Ok(true);
    }

    // Handle character motions with operators (df, dt, cf, ct, yf, yt, etc.)
    if let Some(handled) = try_handle_char_motion_with_operator(editor, operator, key_event)? {
        return Ok(handled);
    }

    // Handle 'g' prefix for gg motion and gn/gN motions
    // NOTE: dgg and ygg are NOT supported in the original - only cgg, >gg, <gg, zfgg
    // But dgn, ygn, cgn ARE supported (gn is a search motion)
    if key_event.code == KeyCode::Char('g')
        && editor.pending_command() != Some('g')
        && matches!(
            operator,
            Operator::Indent | Operator::Dedent | Operator::Fold | Operator::Change | Operator::Delete | Operator::Yank
        )
    {
        editor.set_pending_command('g');
        return Ok(true);
    }

    // Handle gg motion with operators (cgg, >gg, <gg, zfgg)
    // NOTE: dgg and ygg are NOT supported - the 'd' or 'y' is cancelled by the first 'g'
    if editor.pending_command() == Some('g') && key_event.code == KeyCode::Char('g') {
        return handle_gg_motion(editor, operator, count);
    }

    // Handle gn and gN motions (search next/prev) - delegate to pending_commands
    if editor.pending_command() == Some('g')
        && matches!(key_event.code, KeyCode::Char('n') | KeyCode::Char('N'))
    {
        // Don't clear the operator - let pending_commands handle it
        return Ok(false);
    }

    // Handle G motion with operators
    if key_event.code == KeyCode::Char('G') {
        return handle_g_motion(editor, operator, count);
    }

    // Clear pending operator for the main match (will be restored if needed)
    editor.clear_pending_operator();

    let handled = match (operator, key_event.code) {
        // =====================================================================
        // Delete operations
        // =====================================================================
        (Operator::Delete, KeyCode::Char('d')) => {
            handle_dd(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('l')) | (Operator::Delete, KeyCode::Right) => {
            handle_dl(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('w')) => {
            handle_dw(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('$')) => {
            handle_d_dollar(editor)?;
            true
        }
        (Operator::Delete, KeyCode::Char('j')) => {
            handle_dj(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('k')) => {
            handle_dk(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('}')) => {
            handle_d_paragraph_forward(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('{')) => {
            handle_d_paragraph_backward(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('%')) => {
            handle_d_percent(editor)?;
            true
        }

        // =====================================================================
        // Yank operations
        // =====================================================================
        (Operator::Yank, KeyCode::Char('y')) => {
            let yanked = Operators::yank_line(editor.buffer(), count)?;
            editor.yank_to_register_with_type(yanked, RegisterType::Line);
            editor.clear_count();
            true
        }
        (Operator::Yank, KeyCode::Char('w')) => {
            let yanked = Operators::yank_word(editor.buffer_mut(), count)?;
            editor.yank_to_register(yanked);
            editor.clear_count();
            true
        }
        (Operator::Yank, KeyCode::Char('$')) => {
            let yanked = Operators::yank_to_end_of_line(editor.buffer())?;
            editor.yank_to_register(yanked);
            editor.clear_count();
            true
        }
        (Operator::Yank, KeyCode::Char('j')) => {
            handle_yj(editor, count)?;
            true
        }
        (Operator::Yank, KeyCode::Char('k')) => {
            handle_yk(editor, count)?;
            true
        }
        (Operator::Yank, KeyCode::Char('}')) => {
            handle_y_paragraph_forward(editor, count)?;
            true
        }
        (Operator::Yank, KeyCode::Char('{')) => {
            handle_y_paragraph_backward(editor, count)?;
            true
        }

        // =====================================================================
        // Change operations
        // =====================================================================
        (Operator::Change, KeyCode::Char('c')) => {
            handle_cc(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('w')) => {
            handle_cw(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('$')) => {
            handle_c_dollar(editor)?;
            true
        }
        (Operator::Change, KeyCode::Char('l')) | (Operator::Change, KeyCode::Right) => {
            handle_cl(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('j')) => {
            handle_cj(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('k')) => {
            handle_ck(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('}')) => {
            handle_c_paragraph_forward(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('{')) => {
            handle_c_paragraph_backward(editor, count)?;
            true
        }

        // =====================================================================
        // Case change operations
        // =====================================================================
        (Operator::Lowercase, KeyCode::Char('u')) => {
            case::change_case_line(editor, count, case::CaseChange::Lowercase)?;
            editor.clear_count();
            true
        }
        (Operator::Uppercase, KeyCode::Char('U')) => {
            case::change_case_line(editor, count, case::CaseChange::Uppercase)?;
            editor.clear_count();
            true
        }
        (Operator::ToggleCase, KeyCode::Char('~')) => {
            case::change_case_line(editor, count, case::CaseChange::Toggle)?;
            editor.clear_count();
            true
        }
        (Operator::Lowercase, KeyCode::Char('w')) => {
            case::change_case_motion(editor, count, case::CaseChange::Lowercase, |buf, cnt| {
                Motions::word_forward(buf, cnt);
            })?;
            editor.clear_count();
            true
        }
        (Operator::Uppercase, KeyCode::Char('w')) => {
            case::change_case_motion(editor, count, case::CaseChange::Uppercase, |buf, cnt| {
                Motions::word_forward(buf, cnt);
            })?;
            editor.clear_count();
            true
        }
        (Operator::ToggleCase, KeyCode::Char('w')) => {
            case::change_case_motion(editor, count, case::CaseChange::Toggle, |buf, cnt| {
                Motions::word_forward(buf, cnt);
            })?;
            editor.clear_count();
            true
        }
        (Operator::Lowercase, KeyCode::Char('$')) => {
            case::change_case_to_end_of_line(editor, case::CaseChange::Lowercase)?;
            editor.clear_count();
            true
        }
        (Operator::Uppercase, KeyCode::Char('$')) => {
            case::change_case_to_end_of_line(editor, case::CaseChange::Uppercase)?;
            editor.clear_count();
            true
        }
        (Operator::ToggleCase, KeyCode::Char('$')) => {
            case::change_case_to_end_of_line(editor, case::CaseChange::Toggle)?;
            editor.clear_count();
            true
        }

        // =====================================================================
        // Fold operations
        // =====================================================================
        (Operator::Fold, KeyCode::Char('j')) => {
            let start_line = editor.buffer().cursor().line();
            let end_line =
                (start_line + count).min(editor.buffer().line_count().saturating_sub(1));
            editor
                .buffer_mut()
                .fold_manager_mut()
                .create_fold(start_line, end_line);
            editor.clear_count();
            true
        }
        (Operator::Fold, KeyCode::Char('k')) => {
            let end_line = editor.buffer().cursor().line() + 1;
            let start_line = editor.buffer().cursor().line().saturating_sub(count);
            editor
                .buffer_mut()
                .fold_manager_mut()
                .create_fold(start_line, end_line);
            editor.clear_count();
            true
        }
        (Operator::Fold, KeyCode::Char('}')) => {
            let start_line = editor.buffer().cursor().line();
            Motions::paragraph_forward(editor.buffer_mut(), count);
            let end_line = editor.buffer().cursor().line();
            editor
                .buffer_mut()
                .fold_manager_mut()
                .create_fold(start_line, end_line);
            editor.clear_count();
            true
        }
        (Operator::Fold, KeyCode::Char('{')) => {
            let end_line = editor.buffer().cursor().line();
            Motions::paragraph_backward(editor.buffer_mut(), count);
            let start_line = editor.buffer().cursor().line();
            editor
                .buffer_mut()
                .fold_manager_mut()
                .create_fold(start_line, end_line);
            editor.clear_count();
            true
        }
        (Operator::Fold, KeyCode::Char('%')) => {
            handle_zf_percent(editor)?;
            true
        }

        // =====================================================================
        // Indent operations
        // =====================================================================
        (Operator::Indent, KeyCode::Char('>')) => {
            let cursor = editor.buffer().cursor();
            let cursor_before = (cursor.line(), cursor.col());
            let start_line = cursor.line();
            let end_line = start_line + count;
            let tab_width = editor.options.tab_width;
            helpers::indent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
            editor.clear_count();
            true
        }
        (Operator::Indent, KeyCode::Char('j')) | (Operator::Indent, KeyCode::Down) => {
            let cursor = editor.buffer().cursor();
            let cursor_before = (cursor.line(), cursor.col());
            let start_line = cursor.line();
            let end_line = start_line + count + 1;
            let tab_width = editor.options.tab_width;
            helpers::indent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
            editor.clear_count();
            true
        }
        (Operator::Indent, KeyCode::Char('k')) | (Operator::Indent, KeyCode::Up) => {
            let cursor = editor.buffer().cursor();
            let cursor_before = (cursor.line(), cursor.col());
            let current_line = cursor.line();
            let start_line = current_line.saturating_sub(count);
            let end_line = current_line + 1;
            let tab_width = editor.options.tab_width;
            helpers::indent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
            editor.clear_count();
            true
        }

        // =====================================================================
        // Dedent operations
        // =====================================================================
        (Operator::Dedent, KeyCode::Char('<')) => {
            let cursor = editor.buffer().cursor();
            let cursor_before = (cursor.line(), cursor.col());
            let start_line = cursor.line();
            let end_line = start_line + count;
            let tab_width = editor.options.tab_width;
            helpers::dedent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
            editor.clear_count();
            true
        }
        (Operator::Dedent, KeyCode::Char('j')) | (Operator::Dedent, KeyCode::Down) => {
            let cursor = editor.buffer().cursor();
            let cursor_before = (cursor.line(), cursor.col());
            let start_line = cursor.line();
            let end_line = start_line + count + 1;
            let tab_width = editor.options.tab_width;
            helpers::dedent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
            editor.clear_count();
            true
        }
        (Operator::Dedent, KeyCode::Char('k')) | (Operator::Dedent, KeyCode::Up) => {
            let cursor = editor.buffer().cursor();
            let cursor_before = (cursor.line(), cursor.col());
            let current_line = cursor.line();
            let start_line = current_line.saturating_sub(count);
            let end_line = current_line + 1;
            let tab_width = editor.options.tab_width;
            helpers::dedent_lines_with_tracking(editor, start_line, end_line, tab_width, cursor_before)?;
            editor.clear_count();
            true
        }

        // =====================================================================
        // Count digits after operator (e.g., d2w)
        // =====================================================================
        (_, KeyCode::Char(c)) if c.is_ascii_digit() && c != '0' => {
            let digit = c.to_digit(10).unwrap() as usize;
            editor.append_count(digit);
            editor.set_pending_operator(operator); // Restore operator
            true
        }

        // =====================================================================
        // Text object prefixes
        // =====================================================================
        (_, KeyCode::Char('i')) => {
            editor.set_pending_operator(operator);
            editor.set_pending_command('i');
            true
        }
        (_, KeyCode::Char('a')) => {
            editor.set_pending_operator(operator);
            editor.set_pending_command('a');
            true
        }

        _ => {
            editor.clear_count();
            false
        }
    };

    Ok(handled)
}

/// Handle character motions with operators (df, dt, cf, ct, yf, yt, etc.)
fn try_handle_char_motion_with_operator(
    editor: &mut Editor,
    operator: Operator,
    key_event: KeyEvent,
) -> Result<Option<bool>> {
    let motion = match key_event.code {
        KeyCode::Char('f') if matches!(operator, Operator::Delete | Operator::Change | Operator::Yank) => {
            CharMotion::Find
        }
        KeyCode::Char('t') if matches!(operator, Operator::Delete | Operator::Change | Operator::Yank) => {
            CharMotion::Till
        }
        KeyCode::Char('F') if matches!(operator, Operator::Delete | Operator::Change | Operator::Yank) => {
            CharMotion::FindBack
        }
        KeyCode::Char('T') if matches!(operator, Operator::Delete | Operator::Change | Operator::Yank) => {
            CharMotion::TillBack
        }
        _ => return Ok(None),
    };

    editor.set_input_state(InputState::AwaitingChar {
        motion,
        operator: Some(operator),
    });
    Ok(Some(true))
}

/// Handle G motion with operator (dG, cG, yG, >G, <G, zfG)
fn handle_g_motion(editor: &mut Editor, operator: Operator, count: usize) -> Result<bool> {
    editor.clear_pending_operator();
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let start_line = cursor.line();
    let end_line = if editor.count().is_some() {
        count.saturating_sub(1)
    } else {
        editor.buffer().line_count().saturating_sub(1)
    };

    match operator {
        Operator::Indent => {
            let tab_width = editor.options.tab_width;
            helpers::indent_lines_with_tracking(
                editor,
                start_line,
                end_line + 1,
                tab_width,
                cursor_before,
            )?;
        }
        Operator::Dedent => {
            let tab_width = editor.options.tab_width;
            helpers::dedent_lines_with_tracking(
                editor,
                start_line,
                end_line + 1,
                tab_width,
                cursor_before,
            )?;
        }
        Operator::Delete => {
            let start_pos = (start_line, 0);
            let end_pos = (end_line + 1, 0);
            let deleted = editor
                .buffer_mut()
                .delete_range(start_line, 0, end_line + 1, 0);
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, deleted.clone(), cursor_before);
            editor.add_change(change);
            editor.delete_to_register_with_type(deleted, RegisterType::Line);
            helpers::clamp_cursor_to_buffer(editor);
        }
        Operator::Fold => {
            editor
                .buffer_mut()
                .fold_manager_mut()
                .create_fold(start_line, end_line);
        }
        Operator::Change => {
            let start_pos = (start_line, 0);
            let end_pos = (end_line + 1, 0);
            let deleted = editor
                .buffer_mut()
                .delete_range(start_line, 0, end_line + 1, 0);
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, deleted.clone(), cursor_before);
            editor.add_change(change);
            editor.delete_to_register(deleted);
            helpers::clamp_cursor_to_buffer(editor);

            let insert_cursor = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(insert_cursor);
            editor.set_mode(Mode::Insert);
            helpers::insert_line_below(editor)?;
        }
        _ => {}
    }

    editor.clear_count();
    Ok(true)
}

/// Handle gg motion with operator (cgg, >gg, <gg, zfgg)
/// NOTE: dgg and ygg are NOT supported in the original implementation
fn handle_gg_motion(editor: &mut Editor, operator: Operator, count: usize) -> Result<bool> {
    editor.clear_pending_operator();
    editor.clear_pending_command();

    let end_line = editor.buffer().cursor().line();
    let cursor_before = (end_line, editor.buffer().cursor().col());
    let start_line = if editor.count().is_some() {
        count.saturating_sub(1)
    } else {
        0
    };

    match operator {
        Operator::Indent => {
            let tab_width = editor.options.tab_width;
            helpers::indent_lines_with_tracking(
                editor,
                start_line,
                end_line + 1,
                tab_width,
                cursor_before,
            )?;
        }
        Operator::Dedent => {
            let tab_width = editor.options.tab_width;
            helpers::dedent_lines_with_tracking(
                editor,
                start_line,
                end_line + 1,
                tab_width,
                cursor_before,
            )?;
        }
        Operator::Fold => {
            editor
                .buffer_mut()
                .fold_manager_mut()
                .create_fold(start_line, end_line);
        }
        Operator::Change => {
            let deleted = editor
                .buffer_mut()
                .delete_range(start_line, 0, end_line + 1, 0);
            let range = Range::new((start_line, 0), (end_line + 1, 0));
            let change = Change::delete(range, deleted.clone(), cursor_before);

            editor.delete_to_register(deleted);
            editor.add_change(change);

            editor.buffer_mut().cursor_mut().set_position(start_line, 0);
            helpers::clamp_cursor_to_buffer(editor);

            let insert_cursor = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            editor.start_change_building(insert_cursor);
            editor.set_mode(Mode::Insert);
        }
        // dgg and ygg fall through - they're not supported
        // The first 'g' cancels the operator
        _ => {}
    }

    editor.clear_count();
    Ok(true)
}

// =====================================================================
// Individual operator handlers
// =====================================================================

fn handle_dd(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let start_line = cursor.line();
    let line_count = editor.buffer().line_count();
    let end_line = (start_line + count).min(line_count);

    let (delete_start_line, delete_start_col) = if end_line >= line_count && start_line > 0 {
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

    editor.delete_to_register_with_type(deleted, RegisterType::Line);
    editor.add_change(change);
    helpers::clamp_cursor_to_buffer(editor);

    let current_line = editor.buffer().cursor().line();
    editor.buffer_mut().cursor_mut().set_position(current_line, 0);
    editor.clear_count();

    Ok(())
}

fn handle_dl(editor: &mut Editor, count: usize) -> Result<()> {
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

            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, start_col, line_idx, end_col);
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, deleted.clone(), cursor_before);

            editor.delete_to_register(deleted);
            editor.add_change(change);
            editor.buffer_mut().cursor_mut().set_position(line_idx, start_col);
            helpers::clamp_cursor_to_buffer(editor);
        }
    }
    editor.clear_count();
    Ok(())
}

fn handle_dw(editor: &mut Editor, count: usize) -> Result<()> {
    let start_cursor = *editor.buffer().cursor();
    let cursor_before = (start_cursor.line(), start_cursor.col());
    let start_line = start_cursor.line();
    let start_col = start_cursor.col();

    Motions::word_forward(editor.buffer_mut(), count);

    let end_cursor = editor.buffer().cursor();
    let mut end_line = end_cursor.line();
    let mut end_col = end_cursor.col();

    // dw should stop at end of line, not cross newlines
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

    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
    editor.delete_to_register(deleted);
    editor.add_change(change);
    helpers::clamp_cursor_to_buffer(editor);
    editor.clear_count();

    Ok(())
}

fn handle_d_dollar(editor: &mut Editor) -> Result<()> {
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
            helpers::clamp_cursor_to_buffer(editor);
        }
    }
    editor.clear_count();
    Ok(())
}

fn handle_dj(editor: &mut Editor, count: usize) -> Result<()> {
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
    Ok(())
}

fn handle_dk(editor: &mut Editor, count: usize) -> Result<()> {
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
    Ok(())
}

fn handle_d_paragraph_forward(editor: &mut Editor, count: usize) -> Result<()> {
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

    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
    editor.delete_to_register(deleted);
    editor.add_change(change);
    helpers::clamp_cursor_to_buffer(editor);
    editor.clear_count();
    Ok(())
}

fn handle_d_paragraph_backward(editor: &mut Editor, count: usize) -> Result<()> {
    let start_cursor = editor.buffer().cursor();
    let cursor_before = (start_cursor.line(), start_cursor.col());
    let end_line = start_cursor.line();
    let end_col = start_cursor.col();

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
    Ok(())
}

fn handle_d_percent(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let start_line = cursor.line();
    let start_col = cursor.col();

    let rope = editor.buffer().rope();
    let text = rope.to_string();
    let chars: Vec<char> = text.chars().collect();

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
            editor.clear_count();
            return Ok(());
        }
    };

    let match_abs_pos = if is_opening {
        Motions::find_matching_bracket_forward(&chars, abs_start, current_char, matching_char)
    } else {
        Motions::find_matching_bracket_backward(&chars, abs_start, matching_char, current_char)
    };

    if let Some(abs_end) = match_abs_pos {
        let (delete_start, delete_end) = if abs_start < abs_end {
            (abs_start, abs_end + 1)
        } else {
            (abs_end, abs_start + 1)
        };

        let (start_line, start_col) = Motions::abs_pos_to_line_col(rope, delete_start);
        let (end_line, end_col) = Motions::abs_pos_to_line_col(rope, delete_end);

        let deleted = editor
            .buffer_mut()
            .delete_range(start_line, start_col, end_line, end_col);
        let range = Range::new((start_line, start_col), (end_line, end_col));
        let change = Change::delete(range, deleted.clone(), cursor_before);

        editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
        editor.delete_to_register(deleted);
        editor.add_change(change);
        helpers::clamp_cursor_to_buffer(editor);
    }

    editor.clear_count();
    Ok(())
}

fn handle_yj(editor: &mut Editor, count: usize) -> Result<()> {
    let start_line = editor.buffer().cursor().line();
    let end_line = (start_line + count + 1).min(editor.buffer().line_count());

    let mut yanked = String::new();
    for line_idx in start_line..end_line {
        if let Some(line) = editor.buffer().line(line_idx) {
            yanked.push_str(&line);
        }
    }
    editor.yank_to_register_with_type(yanked, RegisterType::Line);
    editor.clear_count();
    Ok(())
}

fn handle_yk(editor: &mut Editor, count: usize) -> Result<()> {
    let end_line = editor.buffer().cursor().line() + 1;
    let start_line = editor.buffer().cursor().line().saturating_sub(count);

    let mut yanked = String::new();
    for line_idx in start_line..end_line {
        if let Some(line) = editor.buffer().line(line_idx) {
            yanked.push_str(&line);
        }
    }
    editor.yank_to_register_with_type(yanked, RegisterType::Line);
    editor.clear_count();
    Ok(())
}

fn handle_y_paragraph_forward(editor: &mut Editor, count: usize) -> Result<()> {
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
    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
    editor.clear_count();
    Ok(())
}

fn handle_y_paragraph_backward(editor: &mut Editor, count: usize) -> Result<()> {
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
    editor.buffer_mut().cursor_mut().set_position(end_line, end_col);
    editor.clear_count();
    Ok(())
}

fn handle_cc(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let start_line = cursor.line();
    let end_line = (start_line + count).min(editor.buffer().line_count());

    editor.start_change_building(cursor_before);

    let start_pos = (start_line, 0);
    let end_pos = (end_line, 0);

    let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line, 0);
    let range = Range::new(start_pos, end_pos);
    let change = Change::delete(range, deleted.clone(), cursor_before);

    editor.delete_to_register(deleted);
    editor.add_change(change);
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    helpers::insert_line_above(editor)?;
    Ok(())
}

fn handle_cw(editor: &mut Editor, count: usize) -> Result<()> {
    let start_cursor = *editor.buffer().cursor();
    let cursor_before = (start_cursor.line(), start_cursor.col());
    let start_line = start_cursor.line();
    let start_col = start_cursor.col();

    // cw behaves like ce
    Motions::word_end_forward(editor.buffer_mut(), count);

    let end_cursor = editor.buffer().cursor();
    let end_line = end_cursor.line();
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
    let change_range = Range::new(start_pos, end_pos);

    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
    editor.delete_to_register(deleted.clone());

    editor.set_pending_semantic_change(PendingSemanticChange {
        object_type: None,
        is_word_change: true,
        old_text: deleted,
        old_range: change_range,
        cursor_before,
    });

    editor.start_change_building((start_line, start_col));
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_c_dollar(editor: &mut Editor) -> Result<()> {
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
    Ok(())
}

fn handle_cl(editor: &mut Editor, count: usize) -> Result<()> {
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

            editor.start_change_building(cursor_before);

            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, start_col, line_idx, end_col);
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, deleted.clone(), cursor_before);

            editor.delete_to_register(deleted);
            editor.add_change(change);
            editor.buffer_mut().cursor_mut().set_position(line_idx, start_col);

            editor.clear_count();
            editor.set_mode(Mode::Insert);
            return Ok(());
        }
    }
    editor.clear_count();
    Ok(())
}

fn handle_cj(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let start_line = cursor.line();
    let end_line = (start_line + count + 1).min(editor.buffer().line_count());

    let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line, 0);
    let range = Range::new((start_line, 0), (end_line, 0));
    let change = Change::delete(range, deleted.clone(), cursor_before);

    editor.delete_to_register(deleted);
    editor.add_change(change);

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
    Ok(())
}

fn handle_ck(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let cursor_before = (cursor.line(), cursor.col());
    let end_line = cursor.line() + 1;
    let start_line = cursor.line().saturating_sub(count);

    let deleted = editor.buffer_mut().delete_range(start_line, 0, end_line, 0);
    let range = Range::new((start_line, 0), (end_line, 0));
    let change = Change::delete(range, deleted.clone(), cursor_before);

    editor.delete_to_register(deleted);
    editor.add_change(change);

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
    Ok(())
}

fn handle_c_paragraph_forward(editor: &mut Editor, count: usize) -> Result<()> {
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

    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
    editor.delete_to_register(deleted);
    editor.add_change(change);
    editor.set_mode(Mode::Insert);
    editor.clear_count();
    Ok(())
}

fn handle_c_paragraph_backward(editor: &mut Editor, count: usize) -> Result<()> {
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
    Ok(())
}

fn handle_zf_percent(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let start_line = cursor.line();
    let start_col = cursor.col();

    let rope = editor.buffer().rope();
    let text = rope.to_string();
    let chars: Vec<char> = text.chars().collect();

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
            editor.clear_count();
            return Ok(());
        }
    };

    let match_abs_pos = if is_opening {
        Motions::find_matching_bracket_forward(&chars, abs_start, current_char, matching_char)
    } else {
        Motions::find_matching_bracket_backward(&chars, abs_start, matching_char, current_char)
    };

    if let Some(abs_end) = match_abs_pos {
        let (fold_start_line, _) = Motions::abs_pos_to_line_col(rope, abs_start.min(abs_end));
        let (fold_end_line, _) = Motions::abs_pos_to_line_col(rope, abs_start.max(abs_end));
        editor
            .buffer_mut()
            .fold_manager_mut()
            .create_fold(fold_start_line, fold_end_line);
    }

    editor.clear_count();
    Ok(())
}
