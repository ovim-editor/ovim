//! Operator + motion handling in normal mode.
//!
//! Handles pending operators combined with motions:
//! - `dd`, `dw`, `dW`, `de`, `dE`, `db`, `dB`, `d$`, `dh`, `dl`, `d0`, `d^`
//!   `dj`, `dk`, `d{`, `d}`, `d%`, `dG`, `dgg`, `df`, `dt`, `dF`, `dT`
//! - `yy`, `yw`, `yW`, `ye`, `yE`, `yb`, `yB`, `y$`, `yh`, `y0`, `y^`
//!   `yj`, `yk`, `y{`, `y}`, `yG`, `ygg`, `yf`, `yt`, `yF`, `yT`
//! - `cc`, `cw`, `cW`, `ce`, `cE`, `cb`, `cB`, `c$`, `ch`, `cl`, `c0`, `c^`
//!   `cj`, `ck`, `c{`, `c}`, `cG`, `cgg`, `cf`, `ct`, `cF`, `cT`
//! - `>>`, `>j`, `>k`, `>G`, `>gg`
//! - `<<`, `<j`, `<k`, `<G`, `<gg`
//! - `zf{motion}`
//! - `gu*`, `gU*`, `g~*`

use crate::editor::input::helpers;
use crate::editor::{
    Change, CharMotion, Editor, InputState, Motions, Operator, PendingChangeRepeat,
    PendingSemanticChange, Range, RegisterType,
};
use crate::mode::Mode;
use crate::repeat_action::RepeatAction;
use crate::{KeyCode, KeyEvent};
use anyhow::Result;

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
    // All linewise operators support gg: dgg, ygg, cgg, >gg, <gg, zfgg
    // dgn, ygn, cgn ARE also supported (gn is a search motion)
    if key_event.code == KeyCode::Char('g')
        && editor.pending_command() != Some('g')
        && matches!(
            operator,
            Operator::Indent
                | Operator::Dedent
                | Operator::AutoIndent
                | Operator::Fold
                | Operator::Change
                | Operator::Delete
                | Operator::Yank
        )
    {
        editor.set_pending_command('g');
        return Ok(true);
    }

    // Handle gg motion with operators (dgg, ygg, cgg, >gg, <gg, zfgg)
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
        (Operator::Delete, KeyCode::Char('b')) => {
            handle_db(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('e')) => {
            handle_de(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('B')) => {
            handle_d_big_b(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('E')) => {
            handle_d_big_e(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('h')) | (Operator::Delete, KeyCode::Left) => {
            handle_dh(editor, count)?;
            true
        }
        (Operator::Delete, KeyCode::Char('0')) => {
            handle_d0(editor)?;
            true
        }
        (Operator::Delete, KeyCode::Char('^')) => {
            handle_d_caret(editor)?;
            true
        }
        (Operator::Delete, KeyCode::Char('W')) => {
            handle_d_big_w(editor, count)?;
            true
        }

        // =====================================================================
        // Yank operations
        // =====================================================================
        (Operator::Yank, KeyCode::Char('y')) => {
            let start_line = editor.buffer().cursor().line();
            let end_line = (start_line + count).min(editor.buffer().line_count()) - 1;
            let yanked = helpers::yank_line(editor.buffer(), count)?;
            editor.yank_to_register_with_type(yanked, RegisterType::Line);
            editor.set_yank_flash_lines(start_line, end_line);
            editor.clear_count();
            true
        }
        (Operator::Yank, KeyCode::Char('w')) => {
            let start_line = editor.buffer().cursor().line();
            let start_col = editor.buffer().cursor().col();
            let yanked = helpers::yank_word(editor.buffer_mut(), count)?;
            let end_col = start_col + yanked.chars().count().saturating_sub(1);
            editor.yank_to_register(yanked);
            editor.set_yank_flash_range(start_line, start_col, start_line, end_col);
            editor.clear_count();
            true
        }
        (Operator::Yank, KeyCode::Char('$')) => {
            let line = editor.buffer().cursor().line();
            let start_col = editor.buffer().cursor().col();
            let yanked = helpers::yank_to_end_of_line(editor.buffer())?;
            let end_col = start_col + yanked.chars().count().saturating_sub(1);
            editor.yank_to_register(yanked);
            editor.set_yank_flash_range(line, start_col, line, end_col);
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
        (Operator::Yank, KeyCode::Char('b')) => {
            handle_yb(editor, count)?;
            true
        }
        (Operator::Yank, KeyCode::Char('e')) => {
            handle_ye(editor, count)?;
            true
        }
        (Operator::Yank, KeyCode::Char('B')) => {
            handle_y_big_b(editor, count)?;
            true
        }
        (Operator::Yank, KeyCode::Char('E')) => {
            handle_y_big_e(editor, count)?;
            true
        }
        (Operator::Yank, KeyCode::Char('h')) | (Operator::Yank, KeyCode::Left) => {
            handle_yh(editor, count)?;
            true
        }
        (Operator::Yank, KeyCode::Char('0')) => {
            handle_y0(editor)?;
            true
        }
        (Operator::Yank, KeyCode::Char('^')) => {
            handle_y_caret(editor)?;
            true
        }
        (Operator::Yank, KeyCode::Char('W')) => {
            handle_y_big_w(editor, count)?;
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
        (Operator::Change, KeyCode::Char('b')) => {
            handle_cb(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('e')) => {
            handle_ce(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('B')) => {
            handle_c_big_b(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('E')) => {
            handle_c_big_e(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('h')) | (Operator::Change, KeyCode::Left) => {
            handle_ch(editor, count)?;
            true
        }
        (Operator::Change, KeyCode::Char('0')) => {
            handle_c0(editor)?;
            true
        }
        (Operator::Change, KeyCode::Char('^')) => {
            handle_c_caret(editor)?;
            true
        }
        (Operator::Change, KeyCode::Char('W')) => {
            handle_c_big_w(editor, count)?;
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
        (Operator::Lowercase, KeyCode::Char('e')) => {
            case::change_case_motion(editor, count, case::CaseChange::Lowercase, |buf, cnt| {
                Motions::word_end_forward(buf, cnt);
            })?;
            editor.clear_count();
            true
        }
        (Operator::Uppercase, KeyCode::Char('e')) => {
            case::change_case_motion(editor, count, case::CaseChange::Uppercase, |buf, cnt| {
                Motions::word_end_forward(buf, cnt);
            })?;
            editor.clear_count();
            true
        }
        (Operator::ToggleCase, KeyCode::Char('e')) => {
            case::change_case_motion(editor, count, case::CaseChange::Toggle, |buf, cnt| {
                Motions::word_end_forward(buf, cnt);
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
            let end_line = (start_line + count).min(editor.buffer().line_count().saturating_sub(1));
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
            helpers::indent_lines_with_tracking(
                editor,
                start_line,
                end_line,
                tab_width,
                cursor_before,
            )?;
            editor.clear_count();
            true
        }
        (Operator::Indent, KeyCode::Char('j')) | (Operator::Indent, KeyCode::Down) => {
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
            true
        }
        (Operator::Indent, KeyCode::Char('k')) | (Operator::Indent, KeyCode::Up) => {
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
            true
        }

        // =====================================================================
        // Auto-indent operations
        // =====================================================================
        (Operator::AutoIndent, KeyCode::Char('=')) => {
            let cursor = editor.buffer().cursor();
            let start_line = cursor.line();
            let end_line = start_line + count;
            let tab_width = editor.options.tab_width;
            helpers::auto_indent_lines_with_tracking(editor, start_line, end_line, tab_width, editor.options.expand_tab)?;
            editor.clear_count();
            true
        }
        (Operator::AutoIndent, KeyCode::Char('j')) | (Operator::AutoIndent, KeyCode::Down) => {
            let cursor = editor.buffer().cursor();
            let start_line = cursor.line();
            let end_line = start_line + count + 1;
            let tab_width = editor.options.tab_width;
            helpers::auto_indent_lines_with_tracking(editor, start_line, end_line, tab_width, editor.options.expand_tab)?;
            editor.clear_count();
            true
        }
        (Operator::AutoIndent, KeyCode::Char('k')) | (Operator::AutoIndent, KeyCode::Up) => {
            let cursor = editor.buffer().cursor();
            let current_line = cursor.line();
            let start_line = current_line.saturating_sub(count);
            let end_line = current_line + 1;
            let tab_width = editor.options.tab_width;
            helpers::auto_indent_lines_with_tracking(editor, start_line, end_line, tab_width, editor.options.expand_tab)?;
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
            helpers::dedent_lines_with_tracking(
                editor,
                start_line,
                end_line,
                tab_width,
                cursor_before,
            )?;
            editor.clear_count();
            true
        }
        (Operator::Dedent, KeyCode::Char('j')) | (Operator::Dedent, KeyCode::Down) => {
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
            true
        }
        (Operator::Dedent, KeyCode::Char('k')) | (Operator::Dedent, KeyCode::Up) => {
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
        KeyCode::Char('f')
            if matches!(
                operator,
                Operator::Delete | Operator::Change | Operator::Yank
            ) =>
        {
            CharMotion::Find
        }
        KeyCode::Char('t')
            if matches!(
                operator,
                Operator::Delete | Operator::Change | Operator::Yank
            ) =>
        {
            CharMotion::Till
        }
        KeyCode::Char('F')
            if matches!(
                operator,
                Operator::Delete | Operator::Change | Operator::Yank
            ) =>
        {
            CharMotion::FindBack
        }
        KeyCode::Char('T')
            if matches!(
                operator,
                Operator::Delete | Operator::Change | Operator::Yank
            ) =>
        {
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
    let cursor_line = cursor.line();
    let max_line = editor.buffer().line_count().saturating_sub(1);
    let target_line = if editor.count().is_some() {
        count.saturating_sub(1).min(max_line)
    } else {
        max_line
    };

    // Normalize so start_line <= end_line
    let start_line = cursor_line.min(target_line);
    let end_line = cursor_line.max(target_line);

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
        Operator::AutoIndent => {
            let tab_width = editor.options.tab_width;
            helpers::auto_indent_lines_with_tracking(editor, start_line, end_line + 1, tab_width, editor.options.expand_tab)?;
        }
        Operator::Delete => {
            let deleted = editor.record_operation(
                |buf| buf.delete_to_last_line(target_line),
                Some(RepeatAction::DeleteToLastLine { target_line }),
            );
            if !deleted.is_empty() {
                editor.delete_to_register_with_type(deleted, RegisterType::Line);
            }
        }
        Operator::Yank => {
            // Yank from start_line to end_line (inclusive, line-wise)
            let mut yanked = String::new();
            for line_idx in start_line..=end_line {
                if let Some(line) = editor.buffer().line(line_idx) {
                    yanked.push_str(&line);
                }
            }
            editor.yank_to_register_with_type(yanked, RegisterType::Line);
            editor.set_yank_flash_lines(start_line, end_line);
            // Cursor stays at original position for yank
        }
        Operator::Fold => {
            editor
                .buffer_mut()
                .fold_manager_mut()
                .create_fold(start_line, end_line);
        }
        Operator::Change => {
            // Get indent from the cursor's line (top of the range we're changing)
            let indent = editor
                .buffer()
                .line(start_line)
                .map(|l| {
                    l.chars()
                        .take_while(|c| c.is_whitespace() && *c != '\n')
                        .collect::<String>()
                })
                .unwrap_or_default();

            let (deleted, edits) = editor.buffer_mut().record(|buf| {
                let cur = buf.cursor().line();
                let tgt = target_line;
                let del_start = cur.min(tgt);
                let del_end = (cur.max(tgt) + 1).min(buf.line_count());
                let deleted = buf.delete_range(del_start, 0, del_end, 0);
                // Insert a blank line at where the deletion started
                let insert_at = del_start.min(buf.line_count());
                buf.insert_text_at(insert_at, 0, &format!("{}\n", indent));
                buf.cursor_mut().set_position(insert_at, indent.len());
                deleted
            });
            let delete_token = if !edits.is_empty() {
                let cursor_after = editor.cursor_position();
                Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
            } else {
                None
            };
            editor.delete_to_register_with_type(deleted, RegisterType::Line);
            editor.mark_buffer_modified();

            editor.set_pending_change_repeat(PendingChangeRepeat {
                delete_action: RepeatAction::DeleteToLastLine { target_line },
                linewise: true,
                delete_token,
            });
            editor.start_change_building(editor.cursor_position());
            editor.set_mode(Mode::Insert);
        }
        _ => {}
    }

    editor.clear_count();
    Ok(true)
}

/// Handle gg motion with operator (dgg, ygg, cgg, >gg, <gg, zfgg)
fn handle_gg_motion(editor: &mut Editor, operator: Operator, count: usize) -> Result<bool> {
    editor.clear_pending_operator();
    editor.clear_pending_command();

    let cursor_line = editor.buffer().cursor().line();
    let cursor_before = (cursor_line, editor.buffer().cursor().col());
    let max_line = editor.buffer().line_count().saturating_sub(1);
    let target_line = if editor.count().is_some() {
        count.saturating_sub(1).min(max_line)
    } else {
        0
    };

    // Normalize so start_line <= end_line
    let start_line = cursor_line.min(target_line);
    let end_line = cursor_line.max(target_line);

    match operator {
        Operator::Delete => {
            let deleted = editor.record_operation(
                |buf| buf.delete_to_first_line(target_line),
                Some(RepeatAction::DeleteToFirstLine { target_line }),
            );
            if !deleted.is_empty() {
                editor.delete_to_register_with_type(deleted, RegisterType::Line);
            }
        }
        Operator::Yank => {
            // Yank from start_line to end_line (inclusive, line-wise)
            let mut yanked = String::new();
            for line_idx in start_line..=end_line {
                if let Some(line) = editor.buffer().line(line_idx) {
                    yanked.push_str(&line);
                }
            }
            editor.yank_to_register_with_type(yanked, RegisterType::Line);
            editor.set_yank_flash_lines(start_line, end_line);
            // Cursor stays at original position for yank
        }
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
        Operator::AutoIndent => {
            let tab_width = editor.options.tab_width;
            helpers::auto_indent_lines_with_tracking(editor, start_line, end_line + 1, tab_width, editor.options.expand_tab)?;
        }
        Operator::Fold => {
            editor
                .buffer_mut()
                .fold_manager_mut()
                .create_fold(start_line, end_line);
        }
        Operator::Change => {
            let indent = editor
                .buffer()
                .line(start_line)
                .map(|l| {
                    l.chars()
                        .take_while(|c| c.is_whitespace() && *c != '\n')
                        .collect::<String>()
                })
                .unwrap_or_default();

            let (deleted, edits) = editor.buffer_mut().record(|buf| {
                let cur = buf.cursor().line();
                let tgt = target_line;
                let del_start = cur.min(tgt);
                let del_end = (cur.max(tgt) + 1).min(buf.line_count());
                let deleted = buf.delete_range(del_start, 0, del_end, 0);
                // Insert a blank line at where the deletion started
                let insert_at = del_start.min(buf.line_count());
                buf.insert_text_at(insert_at, 0, &format!("{}\n", indent));
                buf.cursor_mut().set_position(insert_at, indent.len());
                deleted
            });
            let delete_token = if !edits.is_empty() {
                let cursor_after = editor.cursor_position();
                Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
            } else {
                None
            };
            editor.delete_to_register_with_type(deleted, RegisterType::Line);
            editor.mark_buffer_modified();

            editor.set_pending_change_repeat(PendingChangeRepeat {
                delete_action: RepeatAction::DeleteToFirstLine { target_line },
                linewise: true,
                delete_token,
            });
            editor.start_change_building(editor.cursor_position());
            editor.set_mode(Mode::Insert);
        }
        _ => {}
    }

    editor.clear_count();
    Ok(true)
}

// =====================================================================
// Individual operator handlers
// =====================================================================

fn handle_dd(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_lines(count),
        Some(RepeatAction::DeleteLines { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register_with_type(deleted, RegisterType::Line);
    }
    editor.clear_count();
    Ok(())
}

fn handle_dl(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_chars_forward(count),
        Some(RepeatAction::DeleteCharForward { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

fn handle_dw(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_word_forward(count),
        Some(RepeatAction::DeleteWordForward { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

fn handle_d_dollar(editor: &mut Editor) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_to_end_of_line(),
        Some(RepeatAction::DeleteToEndOfLine),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

fn handle_dj(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_line_down(count),
        Some(RepeatAction::DeleteLineDown { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register_with_type(deleted, RegisterType::Line);
    }
    editor.clear_count();
    Ok(())
}

fn handle_dk(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_line_up(count),
        Some(RepeatAction::DeleteLineUp { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register_with_type(deleted, RegisterType::Line);
    }
    editor.clear_count();
    Ok(())
}

fn handle_d_paragraph_forward(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_paragraph_forward(count),
        Some(RepeatAction::DeleteParagraphForward { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register_with_type(deleted, RegisterType::Line);
    }
    editor.clear_count();
    Ok(())
}

fn handle_d_paragraph_backward(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_paragraph_backward(count),
        Some(RepeatAction::DeleteParagraphBackward { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register_with_type(deleted, RegisterType::Line);
    }
    editor.clear_count();
    Ok(())
}

fn handle_d_percent(editor: &mut Editor) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_to_matching_bracket(),
        Some(RepeatAction::DeleteToMatchingBracket),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
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
    editor.set_yank_flash_lines(start_line, end_line.saturating_sub(1));
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
    editor.set_yank_flash_lines(start_line, end_line.saturating_sub(1));
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

    editor.yank_to_register_with_type(yanked, RegisterType::Line);
    editor.set_yank_flash_lines(start_line, end_line);
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start_line, start_col);
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

    editor.yank_to_register_with_type(yanked, RegisterType::Line);
    editor.set_yank_flash_lines(start_line, end_line);
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(end_line, end_col);
    editor.clear_count();
    Ok(())
}

fn handle_cc(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();
    let start_line = editor.buffer().cursor().line();
    let end_line = (start_line + count).min(editor.buffer().line_count());

    // Capture indentation BEFORE deleting
    let indent = editor
        .buffer()
        .line(start_line)
        .map(|l| {
            l.chars()
                .take_while(|c| c.is_whitespace() && *c != '\n')
                .collect::<String>()
        })
        .unwrap_or_default();

    // Phase 1: Delete lines + open blank line with indent (recorded for undo)
    let (deleted, edits) = editor.buffer_mut().record(|buf| {
        let deleted = buf.delete_range(start_line, 0, end_line, 0);
        let insert_at = start_line.min(buf.line_count());
        buf.insert_text_at(insert_at, 0, &format!("{}\n", indent));
        buf.cursor_mut().set_position(insert_at, indent.len());
        deleted
    });
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    editor.delete_to_register_with_type(deleted, RegisterType::Line);
    editor.mark_buffer_modified();

    // Phase 2: Set up for insert mode
    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteLines { count },
        linewise: true,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_cw(editor: &mut Editor, count: usize) -> Result<()> {
    let start_cursor = *editor.buffer().cursor();
    let cursor_before = (start_cursor.line(), start_cursor.col());
    let start_line = start_cursor.line();
    let start_col = start_cursor.col();

    // cw behaves like ce
    Motions::word_end_forward_prefer_current(editor.buffer_mut(), count);

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

    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start_line, start_col);
    editor.delete_to_register(deleted.clone());

    editor.set_pending_semantic_change(PendingSemanticChange {
        object_type: None,
        is_word_change: true,
        is_search_match_change: false,
        search_pattern: None,
        search_forward: None,
        old_text: deleted,
        old_range: change_range,
        cursor_before,
    });

    editor.start_change_building((start_line, start_col));
    editor.mark_buffer_modified();
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_c_dollar(editor: &mut Editor) -> Result<()> {
    let cursor_before = editor.cursor_position();
    let line_idx = cursor_before.0;
    let col = cursor_before.1;

    let (deleted, edits) = editor.buffer_mut().record(|buf| {
        let line_len = buf
            .line(line_idx)
            .map(|l| l.trim_end_matches('\n').chars().count())
            .unwrap_or(0);
        if col < line_len {
            let deleted = buf.delete_range(line_idx, col, line_idx, line_len);
            buf.cursor_mut().set_position(line_idx, col);
            deleted
        } else {
            String::new()
        }
    });
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        let token = editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after);
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
        Some(token)
    } else {
        None
    };

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteToEndOfLine,
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
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

            let version_before = editor.buffer().version();
            let deleted = editor
                .buffer_mut()
                .delete_range(line_idx, start_col, line_idx, end_col);
            if editor.buffer().version() == version_before {
                editor.clear_count();
                return Ok(());
            }
            let range = Range::new(start_pos, end_pos);
            let change = Change::delete(range, deleted.clone(), cursor_before);

            editor.delete_to_register(deleted);
            editor.add_change(change);
            editor.mark_buffer_modified();
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
    Ok(())
}

fn handle_cj(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();
    let start_line = editor.buffer().cursor().line();
    let end_line = (start_line + count + 1).min(editor.buffer().line_count());

    let indent = editor
        .buffer()
        .line(start_line)
        .map(|l| {
            l.chars()
                .take_while(|c| c.is_whitespace() && *c != '\n')
                .collect::<String>()
        })
        .unwrap_or_default();

    let (deleted, edits) = editor.buffer_mut().record(|buf| {
        let deleted = buf.delete_range(start_line, 0, end_line, 0);
        let insert_at = start_line.min(buf.line_count());
        buf.insert_text_at(insert_at, 0, &format!("{}\n", indent));
        buf.cursor_mut().set_position(insert_at, indent.len());
        deleted
    });
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    editor.delete_to_register_with_type(deleted, RegisterType::Line);
    editor.mark_buffer_modified();

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteLineDown { count },
        linewise: true,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_ck(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();
    let end_line = editor.buffer().cursor().line() + 1;
    let start_line = editor.buffer().cursor().line().saturating_sub(count);

    let indent = editor
        .buffer()
        .line(start_line)
        .map(|l| {
            l.chars()
                .take_while(|c| c.is_whitespace() && *c != '\n')
                .collect::<String>()
        })
        .unwrap_or_default();

    let (deleted, edits) = editor.buffer_mut().record(|buf| {
        let deleted = buf.delete_range(start_line, 0, end_line, 0);
        let insert_at = start_line.min(buf.line_count());
        buf.insert_text_at(insert_at, 0, &format!("{}\n", indent));
        buf.cursor_mut().set_position(insert_at, indent.len());
        deleted
    });
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    editor.delete_to_register_with_type(deleted, RegisterType::Line);
    editor.mark_buffer_modified();

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteLineUp { count },
        linewise: true,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_c_paragraph_forward(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor
        .buffer_mut()
        .record(|buf| buf.delete_paragraph_forward(count));
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    editor.delete_to_register_with_type(deleted, RegisterType::Line);
    editor.mark_buffer_modified();

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteParagraphForward { count },
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_c_paragraph_backward(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor
        .buffer_mut()
        .record(|buf| buf.delete_paragraph_backward(count));
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    editor.delete_to_register_with_type(deleted, RegisterType::Line);
    editor.mark_buffer_modified();

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteParagraphBackward { count },
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
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

// =====================================================================
// Delete handlers for new motions (db, de, dB, dE, dh, d0, d^, dW)
// =====================================================================

fn handle_db(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_word_backward(count),
        Some(RepeatAction::DeleteWordBackward { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

fn handle_de(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_word_end(count),
        Some(RepeatAction::DeleteWordEnd { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

fn handle_d_big_b(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_word_backward_big(count),
        Some(RepeatAction::DeleteWordBackwardBig { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

fn handle_d_big_e(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_word_end_big(count),
        Some(RepeatAction::DeleteWordEndBig { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

fn handle_dh(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_char_left(count),
        Some(RepeatAction::DeleteCharLeft { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

fn handle_d0(editor: &mut Editor) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_to_start_of_line(),
        Some(RepeatAction::DeleteToStartOfLine),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

fn handle_d_caret(editor: &mut Editor) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_to_first_non_blank(),
        Some(RepeatAction::DeleteToFirstNonBlank),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

fn handle_d_big_w(editor: &mut Editor, count: usize) -> Result<()> {
    let deleted = editor.record_operation(
        |buf| buf.delete_word_forward_big(count),
        Some(RepeatAction::DeleteWordForwardBig { count }),
    );
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
    }
    editor.clear_count();
    Ok(())
}

// =====================================================================
// Yank handlers for new motions (yb, ye, yB, yE, yh, y0, y^, yW)
// =====================================================================

fn handle_yb(editor: &mut Editor, count: usize) -> Result<()> {
    let start_line = editor.buffer().cursor().line();
    let start_col = editor.buffer().cursor().col();

    Motions::word_backward(editor.buffer_mut(), count);

    let end_line = editor.buffer().cursor().line();
    let end_col = editor.buffer().cursor().col();

    let yanked = yank_range(editor, end_line, end_col, start_line, start_col);
    editor.yank_to_register(yanked);
    editor.set_yank_flash_range(end_line, end_col, start_line, start_col.saturating_sub(1));
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(end_line, end_col);
    editor.clear_count();
    Ok(())
}

fn handle_ye(editor: &mut Editor, count: usize) -> Result<()> {
    let start_line = editor.buffer().cursor().line();
    let start_col = editor.buffer().cursor().col();

    Motions::word_end_forward(editor.buffer_mut(), count);

    let end_line = editor.buffer().cursor().line();
    let end_col = editor.buffer().cursor().col();

    // Inclusive: include the char motion lands on
    let yanked = yank_range(editor, start_line, start_col, end_line, end_col + 1);
    editor.yank_to_register(yanked);
    editor.set_yank_flash_range(start_line, start_col, end_line, end_col);
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start_line, start_col);
    editor.clear_count();
    Ok(())
}

fn handle_y_big_b(editor: &mut Editor, count: usize) -> Result<()> {
    let start_line = editor.buffer().cursor().line();
    let start_col = editor.buffer().cursor().col();

    Motions::word_backward_big(editor.buffer_mut(), count);

    let end_line = editor.buffer().cursor().line();
    let end_col = editor.buffer().cursor().col();

    let yanked = yank_range(editor, end_line, end_col, start_line, start_col);
    editor.yank_to_register(yanked);
    editor.set_yank_flash_range(end_line, end_col, start_line, start_col.saturating_sub(1));
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(end_line, end_col);
    editor.clear_count();
    Ok(())
}

fn handle_y_big_e(editor: &mut Editor, count: usize) -> Result<()> {
    let start_line = editor.buffer().cursor().line();
    let start_col = editor.buffer().cursor().col();

    Motions::word_end_forward_big(editor.buffer_mut(), count);

    let end_line = editor.buffer().cursor().line();
    let end_col = editor.buffer().cursor().col();

    let yanked = yank_range(editor, start_line, start_col, end_line, end_col + 1);
    editor.yank_to_register(yanked);
    editor.set_yank_flash_range(start_line, start_col, end_line, end_col);
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start_line, start_col);
    editor.clear_count();
    Ok(())
}

fn handle_yh(editor: &mut Editor, count: usize) -> Result<()> {
    let line_idx = editor.buffer().cursor().line();
    let col = editor.buffer().cursor().col();
    if col == 0 {
        editor.clear_count();
        return Ok(());
    }
    let start_col = col.saturating_sub(count);
    let yanked = yank_range(editor, line_idx, start_col, line_idx, col);
    editor.yank_to_register(yanked);
    editor.set_yank_flash_range(line_idx, start_col, line_idx, col.saturating_sub(1));
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(line_idx, start_col);
    editor.clear_count();
    Ok(())
}

fn handle_y0(editor: &mut Editor) -> Result<()> {
    let line_idx = editor.buffer().cursor().line();
    let col = editor.buffer().cursor().col();
    if col == 0 {
        editor.clear_count();
        return Ok(());
    }
    let yanked = yank_range(editor, line_idx, 0, line_idx, col);
    editor.yank_to_register(yanked);
    editor.set_yank_flash_range(line_idx, 0, line_idx, col.saturating_sub(1));
    editor.buffer_mut().cursor_mut().set_position(line_idx, 0);
    editor.clear_count();
    Ok(())
}

fn handle_y_caret(editor: &mut Editor) -> Result<()> {
    let line_idx = editor.buffer().cursor().line();
    let col = editor.buffer().cursor().col();
    let fnb = editor.buffer().first_non_blank_col(line_idx);
    if fnb == col {
        editor.clear_count();
        return Ok(());
    }
    let (start, end) = if fnb < col { (fnb, col) } else { (col, fnb) };
    let yanked = yank_range(editor, line_idx, start, line_idx, end);
    editor.yank_to_register(yanked);
    editor.set_yank_flash_range(line_idx, start, line_idx, end.saturating_sub(1));
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(line_idx, start);
    editor.clear_count();
    Ok(())
}

fn handle_y_big_w(editor: &mut Editor, count: usize) -> Result<()> {
    let start_line = editor.buffer().cursor().line();
    let start_col = editor.buffer().cursor().col();

    Motions::word_forward_big(editor.buffer_mut(), count);

    let end_line = editor.buffer().cursor().line();
    let end_col = editor.buffer().cursor().col();

    let yanked = yank_range(editor, start_line, start_col, end_line, end_col);
    editor.yank_to_register(yanked);
    let flash_end_col = if end_col > 0 { end_col - 1 } else { 0 };
    editor.set_yank_flash_range(start_line, start_col, end_line, flash_end_col);
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start_line, start_col);
    editor.clear_count();
    Ok(())
}

/// Helper to yank a range of text without modifying the buffer.
fn yank_range(
    editor: &Editor,
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
) -> String {
    let buf = editor.buffer();
    let mut result = String::new();
    for line_idx in start_line..=end_line {
        if let Some(line) = buf.line(line_idx) {
            let chars: Vec<char> = line.chars().collect();
            let from = if line_idx == start_line { start_col } else { 0 };
            let to = if line_idx == end_line {
                end_col.min(chars.len())
            } else {
                chars.len()
            };
            if from < to {
                result.extend(&chars[from..to]);
            }
        }
    }
    result
}

// =====================================================================
// Change handlers for new motions (cb, ce, cB, cE, ch, c0, c^, cW)
// =====================================================================

fn handle_cb(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor
        .buffer_mut()
        .record(|buf| buf.delete_word_backward(count));
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
    }

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteWordBackward { count },
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_ce(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor.buffer_mut().record(|buf| buf.delete_word_end(count));
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
    }

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteWordEnd { count },
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_c_big_b(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor
        .buffer_mut()
        .record(|buf| buf.delete_word_backward_big(count));
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
    }

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteWordBackwardBig { count },
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_c_big_e(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor
        .buffer_mut()
        .record(|buf| buf.delete_word_end_big(count));
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
    }

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteWordEndBig { count },
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_ch(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor
        .buffer_mut()
        .record(|buf| buf.delete_char_left(count));
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
    }

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteCharLeft { count },
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_c0(editor: &mut Editor) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor
        .buffer_mut()
        .record(|buf| buf.delete_to_start_of_line());
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
    }

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteToStartOfLine,
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_c_caret(editor: &mut Editor) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor
        .buffer_mut()
        .record(|buf| buf.delete_to_first_non_blank());
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
    }

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteToFirstNonBlank,
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}

fn handle_c_big_w(editor: &mut Editor, count: usize) -> Result<()> {
    let cursor_before = editor.cursor_position();

    let (deleted, edits) = editor
        .buffer_mut()
        .record(|buf| buf.delete_word_forward_big(count));
    let delete_token = if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        Some(editor.push_recorded_undo_returning_token(edits, cursor_before, cursor_after))
    } else {
        None
    };
    if !deleted.is_empty() {
        editor.delete_to_register(deleted);
        editor.mark_buffer_modified();
    }

    editor.set_pending_change_repeat(PendingChangeRepeat {
        delete_action: RepeatAction::DeleteWordForwardBig { count },
        linewise: false,
        delete_token,
    });
    editor.start_change_building(editor.cursor_position());
    editor.clear_count();
    editor.set_mode(Mode::Insert);
    Ok(())
}
