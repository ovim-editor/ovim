//! Helper functions for cursor movement and editing
//!
//! These functions are used by various input handlers.

use crate::editor::{ApplyPos, CursorPos, Editor, RegisterType};
use crate::indentation::{
    leading_char_count, leading_str, leading_width, visual_width, IndentOptions,
};
use crate::mode::Mode;
use crate::repeat_action::{CaseTransform, RepeatAction, VisualShape, VisualTransform};
use crate::unicode::{grapheme_count, grapheme_to_char_col, CharCol, GraphemeCol};
use anyhow::Result;

/// Calculate end position after inserting text at a given start position.
/// Both input and output are char-space (the iteration counts chars, not graphemes).
fn calculate_end_position(start: ApplyPos, text: &str) -> ApplyPos {
    let mut line = start.line;
    let mut col = start.col.0;
    for ch in text.chars() {
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    ApplyPos::new(line, CharCol(col))
}

fn inherited_indent(line: &str, extra_levels: usize, options: IndentOptions) -> String {
    let options = options.normalized();
    let current_width = leading_width(line, options.tab_width);
    let target_width = current_width + extra_levels * options.shift_width;
    if options.copy_indent {
        let mut indent = leading_str(line).to_string();
        indent.push_str(&options.gap_text(current_width, target_width));
        indent
    } else {
        options.encode_indent(target_width)
    }
}

// Helper methods for cursor movement and editing

pub fn move_left(editor: &mut Editor) {
    let count = editor.effective_count();
    let cursor = editor.buffer_mut().cursor_mut();
    if cursor.col().0 >= count {
        cursor.move_left(count);
    } else {
        cursor.set_col(GraphemeCol(0));
    }
    editor.clear_count();
}

pub fn move_right(editor: &mut Editor) {
    let count = editor.effective_count();
    let line_idx = editor.buffer().cursor().line();
    let mode = editor.mode();
    let line_len = editor.buffer().line_index(line_idx).grapheme_count();
    let cursor = editor.buffer_mut().cursor_mut();

    // In VisualBlock mode, allow cursor beyond line end for rectangular selection.
    // In Insert mode, allow cursor one past end for appending.
    let max_col = if mode == Mode::VisualBlock {
        usize::MAX
    } else if mode == Mode::Insert {
        line_len
    } else {
        line_len.saturating_sub(1)
    };

    // `l` never moves left: on the line break after `v$` it stays.
    let new_col = cursor
        .col()
        .0
        .saturating_add(count)
        .min(max_col)
        .max(cursor.col().0);
    cursor.set_col(GraphemeCol(new_col));
    editor.clear_count();
}

pub fn move_up(editor: &mut Editor) {
    let count = editor.effective_count();
    let line_before = editor.buffer().cursor().line();
    let folds = editor.buffer().fold_manager();
    if !folds.hidden_ranges().is_empty() {
        // Closed folds count as one line.
        let target = folds.step_up(line_before, count);
        editor.buffer_mut().cursor_mut().set_line(target);
    } else {
        editor.buffer_mut().cursor_mut().move_up(count);
    }
    clamp_cursor_with_goal_column(editor);
    editor.clear_count();
    if editor.buffer().cursor().line() == line_before {
        editor.signal_macro_abort();
    }
}

pub fn move_down(editor: &mut Editor) {
    let count = editor.effective_count();
    let max_line = editor.buffer().line_count().saturating_sub(1);

    let line_before = editor.buffer().cursor().line();
    let folds = editor.buffer().fold_manager();
    let new_line = if folds.hidden_ranges().is_empty() {
        (line_before + count).min(max_line)
    } else {
        // Closed folds count as one line.
        folds.step_down(line_before, count, max_line)
    };
    editor.buffer_mut().cursor_mut().set_line(new_line);
    clamp_cursor_with_goal_column(editor);
    editor.clear_count();
    if editor.buffer().cursor().line() == line_before {
        editor.signal_macro_abort();
    }
}

pub fn clamp_cursor_to_line(editor: &mut Editor) {
    let line_idx = editor.buffer().cursor().line();
    let line_len = editor.buffer().line_index(line_idx).grapheme_count();
    let cursor = editor.buffer_mut().cursor_mut();
    if cursor.col().0 >= line_len {
        cursor.set_col(GraphemeCol(line_len.saturating_sub(1)));
    }
}

pub fn clamp_cursor_with_goal_column(editor: &mut Editor) {
    let line_idx = editor.buffer().cursor().line();
    let mode = editor.mode();
    let line_len = editor.buffer().line_index(line_idx).grapheme_count();
    let max_col = line_len.saturating_sub(1);
    let cursor = editor.buffer_mut().cursor_mut();
    let desired = cursor.desired_col();

    // usize::MAX is a sentinel value meaning "always end of line".
    let to_eol = desired == usize::MAX;
    // In VisualBlock mode, preserve desired column even if beyond line end.
    let target_col = if to_eol && mode == Mode::Visual {
        // After `v$` the cursor sits on the line break of every line it visits.
        line_len
    } else if to_eol {
        max_col
    } else if mode == Mode::VisualBlock {
        desired
    } else {
        desired.min(max_col)
    };

    cursor.set_col_preserve_desired(GraphemeCol(target_col));
    if to_eol && mode == Mode::VisualBlock {
        editor.set_visual_block_dollar(true);
    }
}

pub fn insert_char(editor: &mut Editor, c: char) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let grapheme_col = cursor.col();
    let char_col = editor
        .buffer()
        .line_index(line_idx)
        .grapheme_to_char(grapheme_col);

    // Insert-mode recording captures the edit; the undo entry is pushed as a
    // single `Recorded` at finalize_change_building time.
    editor.record_session_edit(|buf| {
        buf.insert_text_at_positioning_cursor(line_idx, char_col, &c.to_string())
    });

    Ok(())
}

pub fn insert_newline(editor: &mut Editor) -> Result<()> {
    let options = editor.indent_options();
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let grapheme_col = cursor.col();
    // Snapshot line text and compute char col (drops borrow before mutation)
    let line_text = editor
        .buffer()
        .line_text(line_idx)
        .unwrap_or_default()
        .to_string();
    let char_col = editor
        .buffer()
        .line_index(line_idx)
        .grapheme_to_char(grapheme_col);
    let position = ApplyPos::new(line_idx, char_col);

    // Special case: when the buffer does not end with a newline and the cursor
    // is at EOF, a single '\n' would only add a trailing newline (still 1 Vim
    // line). Vim's <CR> at EOF creates a *new empty line*, which corresponds to
    // inserting two '\n' characters (end current line, then terminate the new
    // empty line). We insert the second '\n' but keep the cursor on the newly
    // created line.
    let at_eof = {
        let rope = editor.buffer().rope();
        let line_start = rope.line_to_char(line_idx);
        line_start + char_col.0 == rope.len_chars()
    };
    let ends_with_newline = editor
        .buffer()
        .rope()
        .chars()
        .last()
        .is_some_and(|c| c == '\n');
    let needs_double_newline = at_eof && !ends_with_newline;

    // Get indentation from text before cursor. Using the text before cursor
    // (rather than the full line) prevents duplication when the cursor sits at
    // or inside leading whitespace — the remainder already carries that
    // whitespace and copying it again would produce extra spaces.
    let text_before: String = line_text.chars().take(char_col.0).collect();
    let cursor_inside_indent = char_col.0 < leading_char_count(&line_text);

    let text_before_cursor: String = line_text.chars().take(char_col.0).collect();
    let opening_delimiter = crate::auto_indent::opening_delimiter_at_end(
        editor.buffer(),
        line_idx,
        &text_before_cursor,
    );
    let opens_block = opening_delimiter.is_some();

    let indent = if cursor_inside_indent {
        // The untouched suffix already contains the remainder of the current
        // indentation. Copy only the prefix before the cursor so splitting in
        // leading whitespace preserves the original visual indentation.
        text_before
    } else {
        inherited_indent(&line_text, usize::from(opens_block), options)
    };

    let matching_close = match opening_delimiter {
        Some('{') => Some('}'),
        Some('(') => Some(')'),
        Some('[') => Some(']'),
        _ => None,
    };
    let text_after_cursor: String = line_text.chars().skip(char_col.0).collect();
    let split_pair = !cursor_inside_indent
        && matching_close.is_some_and(|close| text_after_cursor.starts_with(close));
    let base_indent = split_pair.then(|| inherited_indent(&line_text, 0, options));

    // When Enter splits an adjacent delimiter pair, keep the insertion point
    // on an indented middle line and align the existing closer with the base.
    let text_to_insert = if let Some(base_indent) = &base_indent {
        format!("\n{indent}\n{base_indent}")
    } else {
        format!("\n{indent}")
    };
    let inserted = editor.record_session_edit(|buf| {
        buf.insert_text_at_positioning_cursor(position.line, position.col, &text_to_insert)
    });

    if inserted && split_pair {
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line_idx + 1, GraphemeCol(indent.chars().count()));
    }

    if needs_double_newline && inserted && !split_pair {
        let cur = editor.buffer().cursor();
        let cur_char_col = editor.buffer().cursor_char_col();
        let cursor_after_first = ApplyPos::new(cur.line(), cur_char_col);
        editor.record_session_edit(|buf| {
            buf.insert_text_at_positioning_cursor(
                cursor_after_first.line,
                cursor_after_first.col,
                "\n",
            )
        });
        // Move cursor back to the line before the trailing newline
        editor
            .buffer_mut()
            .set_cursor_char_col(cursor_after_first.line, cursor_after_first.col);
    }

    Ok(())
}

/// Insert-mode `<Del>`: the character under the cursor, or the line break at the
/// end of a line (joining the next line).
pub fn delete_char_at_cursor_insert(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let (line, col) = (cursor.line(), cursor.col());
    let index = editor.buffer().line_index(line);
    let (start, end) = if col.0 < index.grapheme_count() {
        (
            ApplyPos::new(line, index.grapheme_to_char(col)),
            ApplyPos::new(line, index.grapheme_to_char(GraphemeCol(col.0 + 1))),
        )
    } else if line + 1 < editor.buffer().line_count() {
        (
            ApplyPos::new(line, CharCol(index.len_chars())),
            ApplyPos::new(line + 1, CharCol::ZERO),
        )
    } else {
        return Ok(());
    };
    editor.record_session_edit(|buf| {
        buf.delete_range_positioning_cursor(start.line, start.col, end.line, end.col)
            .0
    });
    Ok(())
}

pub fn delete_char_before_cursor(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let grapheme_col = cursor.col();
    if grapheme_col.0 == 0 && line_idx == 0 {
        // At start of buffer, nothing to delete
        return Ok(());
    }

    let (start_pos, end_pos) = if grapheme_col.0 == 0 {
        // Delete newline at end of previous line
        // Use char count for the position (delete_range expects char indices)
        let prev_line_char_len = editor.buffer().line_index(line_idx - 1).len_chars();
        (
            ApplyPos::new(line_idx - 1, CharCol(prev_line_char_len)),
            ApplyPos::new(line_idx, CharCol::ZERO),
        )
    } else {
        // Delete character before cursor on same line.
        // Convert grapheme col to char col for rope operations.
        let line_text = editor.buffer().line_text(line_idx).unwrap_or_default();
        let char_col = editor
            .buffer()
            .line_index(line_idx)
            .grapheme_to_char(grapheme_col);
        let before: String = line_text.chars().take(char_col.0).collect();
        let options = editor.indent_options();

        // Soft-tab backspace is visual-column based and works for spaces,
        // hard tabs, and mixed prefixes. Re-encoding the prefix avoids
        // leaving a tab that overshoots the requested stop.
        if options.soft_tab_stop != 0
            && !before.is_empty()
            && before.chars().all(|c| matches!(c, ' ' | '\t'))
        {
            let current_width = visual_width(&before, options.tab_width);
            let target_width = options.previous_soft_tab_stop(current_width);
            let replacement = options.encode_indent(target_width);
            editor.record_session_edit(|buf| {
                let deleted = buf
                    .delete_range_positioning_cursor(line_idx, CharCol::ZERO, line_idx, char_col)
                    .0;
                let inserted = if replacement.is_empty() {
                    false
                } else {
                    buf.insert_text_at_positioning_cursor(line_idx, CharCol::ZERO, &replacement)
                };
                deleted || inserted
            });
            return Ok(());
        }

        // Normal single-grapheme delete.
        let prev_char_col = editor
            .buffer()
            .line_index(line_idx)
            .grapheme_to_char(GraphemeCol(grapheme_col.0 - 1));
        (
            ApplyPos::new(line_idx, prev_char_col),
            ApplyPos::new(line_idx, char_col),
        )
    };

    // Record backspace via buffer helper. The insert-session recording
    // captures the edit; dot-repeat replays from the recorded `Edit` list, so
    // no backwards-direction flag is needed.
    editor.record_session_edit(|buf| {
        buf.delete_range_positioning_cursor(
            start_pos.line,
            start_pos.col,
            end_pos.line,
            end_pos.col,
        )
        .0
    });

    Ok(())
}

pub fn delete_word_backward_insert(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let grapheme_col = cursor.col();
    if grapheme_col.0 == 0 && line_idx == 0 {
        // At start of buffer, nothing to delete
        return Ok(());
    }

    // If at start of line, delete the newline character
    if grapheme_col.0 == 0 {
        let prev_line_len = editor.buffer().line_index(line_idx - 1).len_chars();
        let start_pos = ApplyPos::new(line_idx - 1, CharCol(prev_line_len));
        let end_pos = ApplyPos::new(line_idx, CharCol::ZERO);
        editor.record_session_edit(|buf| {
            buf.delete_range_positioning_cursor(
                start_pos.line,
                start_pos.col,
                end_pos.line,
                end_pos.col,
            )
            .0
        });
        return Ok(());
    }

    // Scan the rope slice directly so Ctrl-W does not materialize a long line.
    let index = editor.buffer().line_index(line_idx);
    let char_col = index.grapheme_to_char(grapheme_col);
    let col = char_col.0;
    let mut before_cursor = editor
        .buffer()
        .rope()
        .line(line_idx)
        .chars_at(col)
        .reversed()
        .peekable();

    // Find the start of the word to delete
    let mut start_col = col;

    // Skip trailing whitespace (Vim deletes whitespace + preceding word)
    while before_cursor
        .peek()
        .is_some_and(|character| character.is_whitespace())
    {
        before_cursor.next();
        start_col -= 1;
    }

    // Then delete the preceding word or punctuation run
    if start_col > 0 {
        let is_word_char = |character: char| character.is_alphanumeric() || character == '_';
        if let Some(character) = before_cursor.next() {
            start_col -= 1;
            if is_word_char(character) {
                while before_cursor.next().is_some_and(is_word_char) {
                    start_col -= 1;
                }
            } else if !character.is_whitespace() {
                while before_cursor
                    .next()
                    .is_some_and(|candidate| !candidate.is_whitespace() && !is_word_char(candidate))
                {
                    start_col -= 1;
                }
            }
        }
    }

    // Delete the range (char-space). `delete_range_positioning_cursor`
    // positions the cursor at the start of the deleted range.
    if start_col < col {
        editor.record_session_edit(|buf| {
            buf.delete_range_positioning_cursor(
                line_idx,
                CharCol(start_col),
                line_idx,
                CharCol(col),
            )
            .0
        });
    }

    Ok(())
}

/// Insert-mode `<C-u>` (vim, with the default `backspace=indent,eol,start`): delete
/// the text typed in this insert on the cursor line; with none, everything before
/// the cursor on the line but its indent (a further `<C-u>` takes the indent, and
/// at column 0 the line break).
pub fn delete_to_line_start_insert(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    if cursor.col().0 == 0 {
        return delete_char_before_cursor(editor);
    }
    let char_col = editor.buffer().cursor_char_col();

    // Where this insert began, when that is on the cursor line before the cursor.
    let typed_from = editor.buffer().recording_origin().and_then(|origin| {
        let rope = editor.buffer().rope();
        let origin = origin.min(rope.len_chars());
        let origin_line = rope.char_to_line(origin);
        let origin_col = origin - rope.line_to_char(origin_line);
        (origin_line == line_idx && origin_col < char_col.0).then_some(origin_col)
    });
    let indent = editor
        .buffer()
        .line_text(line_idx)
        .map_or(0, |line| leading_char_count(&line));
    let from = typed_from.unwrap_or(if char_col.0 > indent { indent } else { 0 });

    // `delete_range_positioning_cursor` lands the cursor at the start of the range.
    editor.record_session_edit(|buf| {
        buf.delete_range_positioning_cursor(line_idx, CharCol(from), line_idx, char_col)
            .0
    });

    Ok(())
}

pub fn indent_line_insert(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let grapheme_col = cursor.col();
    let options = editor.indent_options();
    let (current_width, old_indent_chars) = editor
        .buffer()
        .line_text(line_idx)
        .map(|line| {
            (
                leading_width(&line, options.tab_width),
                leading_char_count(&line),
            )
        })
        .unwrap_or_default();
    let target_width = options.next_indent_stop(current_width);
    let new_indent_chars = options.encode_indent(target_width).chars().count();
    editor.record_session_edit(|buf| {
        let version = buf.version();
        buf.set_indent_width_at(line_idx, target_width, options);
        buf.version() != version
    });

    // Preserve the cursor's logical offset from the first non-blank.
    let new_col = grapheme_col
        .0
        .saturating_sub(old_indent_chars)
        .saturating_add(new_indent_chars);
    editor
        .buffer_mut()
        .cursor_mut()
        .set_col(GraphemeCol(new_col));

    Ok(())
}

pub fn dedent_line_insert(editor: &mut Editor) -> Result<()> {
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let grapheme_col = cursor.col();
    let options = editor.indent_options();

    // Get current line
    let line_text = match editor.buffer().line_text(line_idx) {
        Some(l) => l,
        None => return Ok(()),
    };

    let old_indent_chars = leading_char_count(&line_text);
    let current_width = leading_width(&line_text, options.tab_width);
    if current_width == 0 {
        return Ok(());
    }
    let target_width = options.previous_indent_stop(current_width);
    let new_indent_chars = options.encode_indent(target_width).chars().count();
    editor.record_session_edit(|buf| {
        let version = buf.version();
        buf.set_indent_width_at(line_idx, target_width, options);
        buf.version() != version
    });

    let new_col = grapheme_col
        .0
        .saturating_sub(old_indent_chars)
        .saturating_add(new_indent_chars);
    editor
        .buffer_mut()
        .cursor_mut()
        .set_col(GraphemeCol(new_col));

    Ok(())
}

/// Electric dedent for closing brackets typed in insert mode.
///
/// When the user types `}`, `)`, or `]` on a line whose content up to the
/// cursor (and beyond) is purely whitespace, remove one indent level before
/// the bracket is inserted. This lets `{`, `<CR>`, `}` produce aligned
/// braces without manual dedent, matching how `==` would reindent the line.
pub fn electric_dedent_close_bracket(editor: &mut Editor, c: char) -> Result<()> {
    if !matches!(c, '}' | ')' | ']') {
        return Ok(());
    }
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let grapheme_col = cursor.col();
    let Some(line) = editor.buffer().line_text(line_idx) else {
        return Ok(());
    };
    let line_text = line.to_string();
    let char_col = grapheme_to_char_col(&line_text, grapheme_col);

    // Only trigger when the line is blank-prefixed up to the cursor AND the
    // rest of the line is also whitespace — i.e. the bracket is being typed
    // on an otherwise-empty indented line (the common `{<CR>}` shape).
    let text_before: String = line_text.chars().take(char_col.0).collect();
    if text_before.is_empty() || !text_before.chars().all(|c| c.is_whitespace()) {
        return Ok(());
    }
    let text_after: String = line_text.chars().skip(char_col.0).collect();
    if !text_after.chars().all(|c| c.is_whitespace()) {
        return Ok(());
    }

    dedent_line_insert(editor)
}

pub fn insert_line_below(editor: &mut Editor) -> Result<bool> {
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let options = editor.indent_options();

    // Reconstruct indentation from visual width unless copyindent requests the
    // source line's exact whitespace representation.
    let line_text = editor.buffer().line_text(line_idx).unwrap_or_default();

    // Add one level when the line ends in a structural opening delimiter.
    // Comments and literals are filtered by the same lexer used by `=`.
    let opens_block =
        crate::auto_indent::opening_delimiter_at_end(editor.buffer(), line_idx, &line_text)
            .is_some();
    let indent = inherited_indent(&line_text, usize::from(opens_block), options);

    // Determine insert position (char-space) and text. `line_text` strips
    // the terminator by design, so use the raw vs content length asymmetry
    // to test for one — true when the rope stores `…\n` for this line.
    let has_terminator =
        editor.buffer().line_raw_len(line_idx) > editor.buffer().line_content_len(line_idx);
    let (insert_position, text_to_insert) = if has_terminator {
        // Line ends with newline, insert at start of next line
        (
            ApplyPos::new(line_idx + 1, CharCol::ZERO),
            format!("{}\n", indent),
        )
    } else {
        // Last line without newline, insert at end of current line
        let line_len = line_text.chars().count();
        (
            ApplyPos::new(line_idx, CharCol(line_len)),
            format!("\n{}\n", indent),
        )
    };

    // Insert the new line (record for undo). `insert_text_at_positioning_cursor`
    // lands the cursor at end of inserted text; we override below.
    if !editor.record_session_edit(|buf| {
        buf.insert_text_at_positioning_cursor(
            insert_position.line,
            insert_position.col,
            &text_to_insert,
        )
    }) {
        return Ok(false);
    }

    // Position cursor at end of indentation on new line
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(line_idx + 1, GraphemeCol(indent.chars().count()));
    change_starts_on_opened_line(editor, line_idx + 1);
    Ok(true)
}

/// For `o` / `O` the change starts on the opened line (vim's `'.` and `g;`),
/// not where the cursor was when the newline was inserted.
fn change_starts_on_opened_line(editor: &mut Editor, line: usize) {
    let buffer = editor.buffer_mut();
    if let Some(offset) = buffer.recording_origin() {
        buffer.set_recording_origin(offset, CursorPos::new(line, GraphemeCol::ZERO));
    }
}

pub fn insert_line_above(editor: &mut Editor) -> Result<bool> {
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let options = editor.indent_options();

    // Get indentation from current line in the configured representation.
    let line_text = editor.buffer().line_text(line_idx).unwrap_or_default();
    let indent = inherited_indent(&line_text, 0, options);

    // Insert indented line above current line (col 0 char == col 0 grapheme)
    let text_to_insert = format!("{}\n", indent);
    let insert_position = ApplyPos::new(line_idx, CharCol::ZERO);

    // Insert the new line (record for undo). `insert_text_at_positioning_cursor`
    // lands cursor at end of inserted text; we override below.
    if !editor.record_session_edit(|buf| {
        buf.insert_text_at_positioning_cursor(
            insert_position.line,
            insert_position.col,
            &text_to_insert,
        )
    }) {
        return Ok(false);
    }

    // Position cursor at end of indentation on the new line (which is still at line_idx
    // because we inserted above, pushing everything down)
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(line_idx, GraphemeCol(indent.chars().count()));
    change_starts_on_opened_line(editor, line_idx);
    Ok(true)
}

/// Expands register text for a `[count]p`/`[count]P`, honoring the register type
/// so the copies land as the register kind intends (see call sites for rationale).
fn expand_paste_by_count(text: String, reg_type: RegisterType, count: usize) -> String {
    if count <= 1 {
        // Line registers still normalize their trailing newline in the paste
        // branch, so a single paste needs no expansion here.
        return text;
    }
    match reg_type {
        RegisterType::Block => text
            .split('\n')
            .map(|row| row.repeat(count))
            .collect::<Vec<_>>()
            .join("\n"),
        RegisterType::Line => {
            let base = if text.ends_with('\n') {
                text
            } else {
                format!("{text}\n")
            };
            base.repeat(count)
        }
        RegisterType::Character => text.repeat(count),
    }
}

pub fn paste_after(editor: &mut Editor, count: usize) -> Result<()> {
    let register = editor.pending_register();
    let (text, reg_type) = editor.get_from_register_with_type();
    paste_text_after(editor, text, reg_type, register, count)
}

/// `p` of already-captured register contents. `register` is the register the
/// text came from (what `.` re-reads), not necessarily where it is now.
pub fn paste_text_after(
    editor: &mut Editor,
    text: String,
    reg_type: RegisterType,
    register: Option<char>,
    count: usize,
) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    editor
        .buffer_mut()
        .change_manager_mut()
        .last_repeat_register = register;

    // Multiply paste text by count, respecting the register type:
    // - Character: concatenate copies inline.
    // - Line: each copy must be its own line, so normalize a trailing newline
    //   FIRST (registers from `S`/single-line cuts lack one) then repeat, else
    //   the copies glue into one merged line.
    // - Block: `count` repeats each row horizontally, keeping the block height.
    let text = expand_paste_by_count(text, reg_type, count);

    let cursor = editor.buffer().cursor();
    let cursor_before = CursorPos::new(cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col().0;

    match reg_type {
        RegisterType::Block => {
            // Block paste - insert each line at the same column on consecutive lines
            // Record all inserts atomically (single undo for entire block paste)
            let block_lines: Vec<&str> = text.split('\n').collect();
            let paste_col = col + 1; // Paste after cursor

            let (last_paste_info, edits) = editor.buffer_mut().record(|buf| {
                let mut last_line = line_idx;
                let mut last_text_len: usize = 0;

                for (i, block_line) in block_lines.iter().enumerate() {
                    let target_line = line_idx + i;
                    if target_line >= buf.line_count() {
                        // Vim appends new space-padded lines for block rows
                        // past the last buffer line; breaking here silently
                        // dropped those rows (OV-00291).
                        let last = buf.line_count().saturating_sub(1);
                        let last_len = buf.line_text(last).map(|l| l.chars().count()).unwrap_or(0);
                        let padding = " ".repeat(paste_col);
                        buf.insert_text_at(
                            last,
                            CharCol(last_len),
                            &format!("\n{}{}", padding, block_line),
                        );
                        last_line = buf.line_count().saturating_sub(1);
                        last_text_len = block_line.chars().count();
                        continue;
                    }

                    if let Some(line_text) = buf.line_text(target_line) {
                        // paste_col is grapheme-space; convert per line so
                        // multi-char graphemes (emoji, combining marks) before
                        // the paste column don't skew the insert position
                        // (OV-00299).
                        let line_graphemes = crate::unicode::grapheme_count(&line_text);
                        if paste_col > line_graphemes {
                            let line_chars = line_text.chars().count();
                            let padding = " ".repeat(paste_col - line_graphemes);
                            let padded_text = format!("{}{}", padding, block_line);
                            buf.insert_text_at(target_line, CharCol(line_chars), &padded_text);
                        } else {
                            let char_col = crate::unicode::grapheme_to_char_col(
                                &line_text,
                                GraphemeCol(paste_col),
                            );
                            buf.insert_text_at(target_line, char_col, block_line);
                        }

                        last_line = target_line;
                        last_text_len = block_line.chars().count();
                    }
                }

                (last_line, last_text_len)
            });

            let (last_pasted_line, last_text_char_count) = last_paste_info;
            // Position cursor on last character of pasted text
            let new_col = if last_text_char_count > 0 {
                paste_col + last_text_char_count - 1
            } else {
                paste_col
            };
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(last_pasted_line, GraphemeCol(new_col));

            if !edits.is_empty() {
                let cursor_after = editor.cursor_position();
                editor.push_recorded_undo(edits, cursor_before, cursor_after);
                editor.set_repeat_action(RepeatAction::PasteAfter { count });
            }
        }
        RegisterType::Line => {
            // Normalize: ensure linewise text ends with newline
            let text = if !text.ends_with('\n') {
                format!("{}\n", text)
            } else {
                text
            };

            // Detect empty buffer (single empty line, e.g. after dd)
            let is_empty_buffer = editor.buffer().line_count() == 1
                && editor
                    .buffer()
                    .line_text(0)
                    .map(|l| l.is_empty())
                    .unwrap_or(true);

            if is_empty_buffer {
                // Insert at (0, 0), cursor on first non-blank of line 0
                let text_clone = text.clone();
                let ((), edits) = editor.buffer_mut().record(|buf| {
                    buf.insert_text_at(0, CharCol::ZERO, &text_clone);
                });

                let first_non_blank = editor
                    .buffer()
                    .line_text(0)
                    .map(|l| {
                        l.chars()
                            .take_while(|ch| ch.is_whitespace() && *ch != '\n')
                            .count()
                    })
                    .unwrap_or(0);
                // first_non_blank is a char index; convert to grapheme for cursor.
                editor
                    .buffer_mut()
                    .set_cursor_char_col(0, CharCol(first_non_blank));

                if !edits.is_empty() {
                    let cursor_after = editor.cursor_position();
                    editor.push_recorded_undo(edits, cursor_before, cursor_after);
                    editor.set_repeat_action(RepeatAction::PasteAfter { count });
                }
            } else {
                // Line paste - insert after current line
                let rope_line = editor.buffer().rope().line(line_idx);
                let raw_line_len = rope_line.len_chars();
                let has_trailing_newline =
                    raw_line_len > 0 && rope_line.char(raw_line_len - 1) == '\n';
                let line_content_len = editor.buffer().line_content_len(line_idx);

                let text_clone = text.clone();
                let ((), edits) = editor.buffer_mut().record(|buf| {
                    if has_trailing_newline {
                        // Columns address visible line content, so the point
                        // after a terminator is the start of the next line.
                        buf.insert_text_at(line_idx + 1, CharCol::ZERO, &text_clone);
                    } else {
                        // No trailing newline on current line — prepend \n
                        let insert_text = format!("\n{}", text_clone);
                        buf.insert_text_at(line_idx, CharCol(line_content_len), &insert_text);
                    }
                });

                // Vim: cursor on first non-blank of the new line
                let new_line = line_idx + 1;
                let first_non_blank = editor
                    .buffer()
                    .line_text(new_line)
                    .map(|l| {
                        l.chars()
                            .take_while(|ch| ch.is_whitespace() && *ch != '\n')
                            .count()
                    })
                    .unwrap_or(0);
                // first_non_blank is a char index; convert to grapheme for cursor.
                editor
                    .buffer_mut()
                    .set_cursor_char_col(new_line, CharCol(first_non_blank));

                if !edits.is_empty() {
                    let cursor_after = editor.cursor_position();
                    editor.push_recorded_undo(edits, cursor_before, cursor_after);
                    editor.set_repeat_action(RepeatAction::PasteAfter { count });
                }
            }
        }
        RegisterType::Character => {
            // Character paste - insert after cursor. The cursor col is
            // grapheme-space: convert "after this grapheme" to a char index
            // against the line so clusters before the cursor don't shift
            // the insert position (OV-00299).
            let line_text = editor
                .buffer()
                .line_text(line_idx)
                .unwrap_or_default()
                .to_string();
            let line_graphemes = crate::unicode::grapheme_count(&line_text);
            let paste_grapheme = (col + 1).min(line_graphemes);
            let paste_col =
                crate::unicode::grapheme_to_char_col(&line_text, GraphemeCol(paste_grapheme));

            let text_clone = text.clone();
            let ((), edits) = editor.buffer_mut().record(|buf| {
                buf.insert_text_at(line_idx, paste_col, &text_clone);
            });

            // Place cursor on the last grapheme of the pasted text: compute
            // the char-space end, then convert against the post-insert line.
            let end_pos = calculate_end_position(ApplyPos::new(line_idx, paste_col), &text);
            let end_line_text = editor
                .buffer()
                .line_text(end_pos.line)
                .unwrap_or_default()
                .to_string();
            let end_grapheme = crate::unicode::char_to_grapheme_col(&end_line_text, end_pos.col);
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(end_pos.line, GraphemeCol(end_grapheme.0.saturating_sub(1)));

            if !edits.is_empty() {
                let cursor_after = editor.cursor_position();
                editor.push_recorded_undo(edits, cursor_before, cursor_after);
                editor.set_repeat_action(RepeatAction::PasteAfter { count });
            }
        }
    }

    Ok(())
}

pub fn paste_before(editor: &mut Editor, count: usize) -> Result<()> {
    let register = editor.pending_register();
    let (text, reg_type) = editor.get_from_register_with_type();
    paste_text_before(editor, text, reg_type, register, count)
}

/// `P` of already-captured register contents (see `paste_text_after`).
pub fn paste_text_before(
    editor: &mut Editor,
    text: String,
    reg_type: RegisterType,
    register: Option<char>,
    count: usize,
) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    editor
        .buffer_mut()
        .change_manager_mut()
        .last_repeat_register = register;

    // Multiply paste text by count (see `expand_paste_by_count`).
    let text = expand_paste_by_count(text, reg_type, count);

    let cursor = editor.buffer().cursor();
    let cursor_before = CursorPos::new(cursor.line(), cursor.col());
    let line_idx = cursor.line();
    let col = cursor.col().0;

    match reg_type {
        RegisterType::Block => {
            // Block paste before - record all inserts atomically (single undo)
            let block_lines: Vec<&str> = text.split('\n').collect();
            let paste_col = col;

            let (last_paste_info, edits) = editor.buffer_mut().record(|buf| {
                let mut last_line = line_idx;
                let mut last_text_len: usize = 0;

                for (i, block_line) in block_lines.iter().enumerate() {
                    let target_line = line_idx + i;
                    if target_line >= buf.line_count() {
                        // Vim appends new space-padded lines for block rows
                        // past the last buffer line; breaking here silently
                        // dropped those rows (OV-00291).
                        let last = buf.line_count().saturating_sub(1);
                        let last_len = buf.line_text(last).map(|l| l.chars().count()).unwrap_or(0);
                        let padding = " ".repeat(paste_col);
                        buf.insert_text_at(
                            last,
                            CharCol(last_len),
                            &format!("\n{}{}", padding, block_line),
                        );
                        last_line = buf.line_count().saturating_sub(1);
                        last_text_len = block_line.chars().count();
                        continue;
                    }

                    if let Some(line_text) = buf.line_text(target_line) {
                        // paste_col is grapheme-space; convert per line
                        // (OV-00299), mirroring the paste-after branch.
                        let line_graphemes = crate::unicode::grapheme_count(&line_text);
                        if paste_col > line_graphemes {
                            let line_chars = line_text.chars().count();
                            let padding = " ".repeat(paste_col - line_graphemes);
                            let padded_text = format!("{}{}", padding, block_line);
                            buf.insert_text_at(target_line, CharCol(line_chars), &padded_text);
                        } else {
                            let char_col = crate::unicode::grapheme_to_char_col(
                                &line_text,
                                GraphemeCol(paste_col),
                            );
                            buf.insert_text_at(target_line, char_col, block_line);
                        }

                        last_line = target_line;
                        last_text_len = block_line.chars().count();
                    }
                }

                (last_line, last_text_len)
            });

            let (last_pasted_line, last_text_char_count) = last_paste_info;
            // Position cursor on last character of pasted text
            let new_col = if last_text_char_count > 0 {
                paste_col + last_text_char_count - 1
            } else {
                paste_col
            };
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(last_pasted_line, GraphemeCol(new_col));

            if !edits.is_empty() {
                let cursor_after = editor.cursor_position();
                editor.push_recorded_undo(edits, cursor_before, cursor_after);
                editor.set_repeat_action(RepeatAction::PasteBefore { count });
            }
        }
        RegisterType::Line => {
            // Line paste before starts at the current line. This avoids
            // representing the same point as a column beyond the previous
            // line's visible content when that line has a terminator.
            let ((), edits) = editor.buffer_mut().record(|buf| {
                buf.insert_text_at(line_idx, CharCol::ZERO, &text);
            });

            // Vim: cursor on first non-blank of the pasted line
            let pasted_line = line_idx; // Text was inserted before current line
            let first_non_blank = editor
                .buffer()
                .line_text(pasted_line)
                .map(|l| {
                    l.chars()
                        .take_while(|ch| ch.is_whitespace() && *ch != '\n')
                        .count()
                })
                .unwrap_or(0);
            // first_non_blank is a char index; convert to grapheme for cursor.
            editor
                .buffer_mut()
                .set_cursor_char_col(pasted_line, CharCol(first_non_blank));

            if !edits.is_empty() {
                let cursor_after = editor.cursor_position();
                editor.push_recorded_undo(edits, cursor_before, cursor_after);
                editor.set_repeat_action(RepeatAction::PasteBefore { count });
            }
        }
        RegisterType::Character => {
            // Character paste before cursor. The cursor col is grapheme-
            // space; convert to a char index against the line (OV-00299).
            let line_text = editor
                .buffer()
                .line_text(line_idx)
                .unwrap_or_default()
                .to_string();
            let paste_col = crate::unicode::grapheme_to_char_col(&line_text, GraphemeCol(col));

            let text_clone = text.clone();
            let ((), edits) = editor.buffer_mut().record(|buf| {
                buf.insert_text_at(line_idx, paste_col, &text_clone);
            });

            // Position cursor on the last grapheme of the pasted text
            // (match paste_after behavior).
            let end_pos = calculate_end_position(ApplyPos::new(line_idx, paste_col), &text);
            let end_line_text = editor
                .buffer()
                .line_text(end_pos.line)
                .unwrap_or_default()
                .to_string();
            let end_grapheme = crate::unicode::char_to_grapheme_col(&end_line_text, end_pos.col);
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(end_pos.line, GraphemeCol(end_grapheme.0.saturating_sub(1)));

            if !edits.is_empty() {
                let cursor_after = editor.cursor_position();
                editor.push_recorded_undo(edits, cursor_before, cursor_after);
                editor.set_repeat_action(RepeatAction::PasteBefore { count });
            }
        }
    }

    Ok(())
}

pub fn delete_visual_selection(editor: &mut Editor) -> Result<()> {
    let _ = delete_visual_selection_with_token(editor)?;
    Ok(())
}

pub fn delete_visual_selection_with_token(
    editor: &mut Editor,
) -> Result<Option<crate::change::ChangeToken>> {
    let mode = editor.mode();
    let cursor_before = editor.cursor_position();

    let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() else {
        return Ok(None);
    };

    let character_range = editor.visual_character_range();

    // Record all deletions in one shot. visual_selection cols are
    // grapheme-space; convert per line so multi-char graphemes are deleted
    // whole instead of being split scalar-by-scalar (OV-00299).
    let (deleted_info, edits) = editor.buffer_mut().record(|buf| {
        match mode {
            Mode::VisualLine => {
                let deleted =
                    buf.delete_range(start_line, CharCol::ZERO, end_line + 1, CharCol::ZERO);
                (deleted, RegisterType::Line)
            }
            Mode::VisualBlock => {
                let mut deleted_lines = Vec::new();
                // Delete from bottom to top to avoid offset shifting
                for line_idx in (start_line..=end_line).rev() {
                    if let Some(line_text) = buf.line_text(line_idx) {
                        let line_graphemes = crate::unicode::grapheme_count(&line_text);
                        if start_col < line_graphemes {
                            let start_char = crate::unicode::grapheme_to_char_col(
                                &line_text,
                                GraphemeCol(start_col),
                            );
                            let end_char = crate::unicode::grapheme_to_char_col(
                                &line_text,
                                GraphemeCol(end_col.saturating_add(1).min(line_graphemes)),
                            );
                            let deleted =
                                buf.delete_range(line_idx, start_char, line_idx, end_char);
                            deleted_lines.push(deleted);
                        } else {
                            deleted_lines.push(String::new());
                        }
                    }
                }
                deleted_lines.reverse();
                (deleted_lines.join("\n"), RegisterType::Block)
            }
            _ => {
                let range = character_range.expect("selection has valid endpoints");
                let deleted = buf.delete_range(
                    range.start_line,
                    range.start_col,
                    range.end_line,
                    range.end_col,
                );
                (deleted, RegisterType::Character)
            }
        }
    });

    if edits.is_empty() {
        return Ok(None);
    }

    let (deleted, register_type) = deleted_info;

    // Cursor positioning (same logic as before)
    match mode {
        Mode::VisualLine => {
            let new_line = start_line.min(editor.buffer().line_count().saturating_sub(1));
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(new_line, GraphemeCol(0));
        }
        Mode::VisualBlock => {
            let line_len = if let Some(line) = editor.buffer().line_text(start_line) {
                line.chars().count()
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
                .set_position(start_line, GraphemeCol(clamped_col));
        }
        _ => {
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(start_line, GraphemeCol(start_col));
        }
    }

    let cursor_after = editor.cursor_position();
    let undo_token = editor.push_recorded_undo(edits, cursor_before, cursor_after);

    // Set dot-repeat template as a semantic RepeatAction for all visual delete modes.
    match mode {
        Mode::VisualLine => {
            let line_count = end_line.saturating_sub(start_line) + 1;
            editor.set_repeat_action(RepeatAction::DeleteVisualLine { line_count });
        }
        Mode::VisualBlock => {
            let line_count = end_line.saturating_sub(start_line) + 1;
            let width = end_col.saturating_sub(start_col) + 1;
            editor.set_repeat_action(RepeatAction::DeleteVisualBlock { line_count, width });
        }
        _ => {
            let line_delta = end_line.saturating_sub(start_line);
            let offset_col = if line_delta == 0 {
                end_col.saturating_add(1).saturating_sub(start_col)
            } else {
                end_col.saturating_add(1)
            };
            editor.set_repeat_action(RepeatAction::DeleteVisualChar {
                line_delta,
                offset_col,
            });
        }
    }

    // Register handling
    match register_type {
        RegisterType::Line => editor.delete_to_register_with_type(deleted, RegisterType::Line),
        RegisterType::Block => editor.delete_to_register_with_type(deleted, RegisterType::Block),
        _ => editor.delete_to_register(deleted),
    }

    Ok(Some(undo_token))
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
                // Yank rectangular block. Selection cols are grapheme-space;
                // convert per line so a combining cluster before the block
                // doesn't shift the extracted columns (OV-00299).
                let mut yanked_lines = Vec::new();

                for line_idx in start_line..=end_line {
                    if let Some(line_text) = editor.buffer().line_text(line_idx) {
                        let line_graphemes = crate::unicode::grapheme_count(&line_text);
                        if start_col < line_graphemes {
                            let start_char_col = crate::unicode::grapheme_to_char_col(
                                &line_text,
                                GraphemeCol(start_col),
                            );
                            let end_char_col = crate::unicode::grapheme_to_char_col(
                                &line_text,
                                GraphemeCol(end_col.saturating_add(1).min(line_graphemes)),
                            );
                            let line_start = editor.buffer().rope().line_to_char(line_idx);
                            let yanked = editor
                                .buffer()
                                .rope()
                                .slice(line_start + start_char_col.0..line_start + end_char_col.0)
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
                let range = editor
                    .visual_character_range()
                    .expect("selection has valid endpoints");
                let yanked = crate::textobjects::TextObjects::yank_range(editor.buffer(), range)?;
                editor.yank_to_register_with_type(yanked, RegisterType::Character);
            }
        }
    }

    Ok(())
}

pub fn join_lines(editor: &mut Editor, count: usize) -> Result<()> {
    editor.record_operation(
        |buf| buf.join_lines(count),
        Some(RepeatAction::JoinLines {
            count,
            add_space: true,
        }),
    )
}

pub fn join_lines_no_space(editor: &mut Editor, count: usize) -> Result<()> {
    editor.record_operation(
        |buf| buf.join_lines_no_space(count),
        Some(RepeatAction::JoinLines {
            count,
            add_space: false,
        }),
    )
}

pub fn indent_lines_with_tracking(
    editor: &mut Editor,
    start_line: usize,
    end_line: usize,
    cursor_before: CursorPos,
) -> Result<()> {
    let options = editor.indent_options();
    let actual_end = end_line.min(editor.buffer().line_count());

    let ((), edits) = editor.buffer_mut().record(|buf| {
        buf.indent_lines_at(start_line, actual_end, options);
    });
    if !edits.is_empty() {
        // Position cursor on start line at first non-blank (Vim behavior)
        let first_nb = editor.buffer().first_non_blank_col(start_line);
        editor
            .buffer_mut()
            .set_cursor_char_col(start_line, first_nb);
        let cursor_after = editor.cursor_position();
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
        let line_count = actual_end - start_line;
        editor.set_repeat_action(RepeatAction::IndentLines {
            line_count,
            options,
        });
        editor.mark_buffer_modified();
    }
    Ok(())
}

pub fn dedent_lines_with_tracking(
    editor: &mut Editor,
    start_line: usize,
    end_line: usize,
    cursor_before: CursorPos,
) -> Result<()> {
    let options = editor.indent_options();
    let ((), edits) = editor.buffer_mut().record(|buf| {
        let actual_end = end_line.min(buf.line_count());
        buf.dedent_lines_at(start_line, actual_end, options);
    });
    if !edits.is_empty() {
        // Position cursor on start line at first non-blank (Vim behavior)
        let first_nb = editor.buffer().first_non_blank_col(start_line);
        editor
            .buffer_mut()
            .set_cursor_char_col(start_line, first_nb);
        let cursor_after = editor.cursor_position();
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
        let line_count = end_line.min(editor.buffer().line_count()) - start_line;
        editor.set_repeat_action(RepeatAction::DedentLines {
            line_count,
            options,
        });
        editor.mark_buffer_modified();
    }
    Ok(())
}

/// Clamps cursor to valid buffer bounds (line and column)
pub fn clamp_cursor_to_buffer(editor: &mut Editor) {
    // First, clamp line to valid range
    let line_count = editor.buffer().line_count();
    if line_count == 0 {
        // Empty buffer, set to 0,0
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(0, GraphemeCol(0));
        return;
    }

    let cursor_line = editor.buffer().cursor().line();
    let clamped_line = cursor_line.min(line_count.saturating_sub(1));

    if cursor_line != clamped_line {
        editor.buffer_mut().cursor_mut().set_line(clamped_line);
    }

    // Then, clamp column to valid range for the line (grapheme-aware)
    editor.buffer_mut().clamp_cursor_col();
}

/// Exit visual mode and save the selection for gv command
/// This should be called whenever exiting visual mode to ensure the selection is saved
pub fn exit_visual_mode_to_normal(editor: &mut Editor) {
    editor.save_last_visual_selection();
    editor.set_visual_block_dollar(false);
    editor.clear_visual_start();
    editor.clear_pending_register();
    editor.set_mode(Mode::Normal);
}

/// Save visual selection and clear visual state (without changing mode)
/// Use this when transitioning to insert mode or other modes after visual operations
pub fn save_and_clear_visual(editor: &mut Editor) {
    editor.save_last_visual_selection();
    editor.clear_visual_start();
}

/// Applies a Visual `u` / `U` / `~` / `r{char}` to the selection, leaves the
/// cursor at its start (as vim does) and makes it the dot-repeat.
fn transform_visual_selection(editor: &mut Editor, transform: VisualTransform) -> Result<()> {
    let mode = editor.mode();
    let cursor_before = editor.cursor_position();

    let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() else {
        return Ok(());
    };
    let line_count = end_line - start_line + 1;
    let shape = match mode {
        Mode::VisualLine => VisualShape::Line { line_count },
        Mode::VisualBlock => VisualShape::Block {
            line_count,
            width: end_col.saturating_sub(start_col).saturating_add(1),
        },
        _ if end_line == start_line => VisualShape::Char {
            line_delta: 0,
            offset_col: end_col + 1 - start_col,
        },
        _ => VisualShape::Char {
            line_delta: end_line - start_line,
            offset_col: end_col + 1,
        },
    };

    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start_line, GraphemeCol(start_col));
    let ((), edits) = editor
        .buffer_mut()
        .record(|buf| crate::repeat_action::transform_visual_shape(buf, shape, transform));
    if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
    }
    editor.set_repeat_action(RepeatAction::TransformVisual { shape, transform });

    Ok(())
}

/// Convert visual selection to uppercase
pub fn uppercase_visual_selection(editor: &mut Editor) -> Result<()> {
    transform_visual_selection(editor, VisualTransform::Case(CaseTransform::Upper))
}

/// Convert visual selection to lowercase
pub fn lowercase_visual_selection(editor: &mut Editor) -> Result<()> {
    transform_visual_selection(editor, VisualTransform::Case(CaseTransform::Lower))
}

/// Replace all characters in visual selection with a given character.
/// Preserves newlines (matches Vim behavior).
pub fn replace_visual_selection(editor: &mut Editor, ch: char) -> Result<()> {
    transform_visual_selection(editor, VisualTransform::Replace(ch))
}

/// Toggle case of visual selection (~)
pub fn toggle_case_visual_selection(editor: &mut Editor) -> Result<()> {
    transform_visual_selection(editor, VisualTransform::Case(CaseTransform::Toggle))
}

/// Extracts the word under the cursor
/// A "word" consists of alphanumeric characters and underscores
/// Returns None if cursor is not on a word character
fn extract_word_at_cursor(editor: &Editor) -> Option<String> {
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let col = cursor.col().0;
    let index = editor.buffer().line_index(line_idx);
    if col >= index.grapheme_count() {
        return None;
    }

    let is_word = |grapheme_col: usize| {
        index
            .grapheme_first_char(GraphemeCol(grapheme_col))
            .is_some_and(|character| character.is_alphanumeric() || character == '_')
    };
    if !is_word(col) {
        return None;
    }

    let mut start = col;
    while start > 0 && is_word(start - 1) {
        start -= 1;
    }
    let mut end = col + 1;
    while end < index.grapheme_count() && is_word(end) {
        end += 1;
    }
    Some(index.slice_chars(
        index.grapheme_to_char(GraphemeCol(start)).0..index.grapheme_to_char(GraphemeCol(end)).0,
    ))
}

/// Sets up and executes a search for the given text
/// Returns true if a match was found, false otherwise
fn setup_and_execute_search(editor: &mut Editor, text: &str, forward: bool) -> bool {
    // Escape regex special characters for literal search
    let escaped = regex::escape(text);

    // Create and execute the search
    editor.clear_search_buffer();
    for ch in escaped.chars() {
        editor.insert_search_char(ch);
    }
    editor.set_search_forward(forward);

    // Update the / register with the search pattern
    editor.registers.set_last_search(escaped.clone());

    // Create search and find first match
    let mut search = crate::editor::Search::new_with_options(
        escaped,
        forward,
        editor.options.ignorecase,
        editor.options.smartcase,
    );

    // For visual * and #, we want to find the NEXT occurrence, not the current one:
    // after the cursor (forward) or before the start of the selection, which is
    // itself a match (backward).
    let cursor = editor.buffer().cursor();
    let origin = match editor.visual_selection() {
        Some((start, _)) if !forward => start,
        _ => (cursor.line(), cursor.col().0),
    };

    if let Some((line, col)) = editor.step_search(&mut search, origin, 1) {
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line, GraphemeCol(col));
        editor.set_current_search(search);
        true
    } else {
        false
    }
}

/// Gets the text content of the current visual selection
/// Returns the selected text as a String, or None if no selection exists
/// Handles Visual, VisualLine, and VisualBlock modes appropriately
pub fn get_visual_selection_text(editor: &Editor) -> Option<String> {
    let mode = editor.mode();
    let ((start_line, start_col), (end_line, end_col)) = editor.visual_selection()?;

    match mode {
        Mode::Visual => {
            let range = editor.visual_character_range()?;
            crate::textobjects::TextObjects::yank_range(editor.buffer(), range).ok()
        }
        Mode::VisualLine => {
            // Line-wise selection (include entire lines)
            let mut text = String::new();
            for line_idx in start_line..=end_line {
                if let Some(line) = editor.buffer().line_text(line_idx) {
                    text.push_str(&line);
                    if line_idx < end_line {
                        text.push('\n');
                    }
                }
            }
            Some(text)
        }
        Mode::VisualBlock => {
            // Rectangular block selection
            let mut lines = Vec::new();
            for line_idx in start_line..=end_line {
                if let Some(line_text) = editor.buffer().line_text(line_idx) {
                    let line_len = grapheme_count(&line_text);
                    let line_start = start_col.min(line_len);
                    let line_end = end_col.saturating_add(1).min(line_len);

                    if line_start < line_end {
                        use unicode_segmentation::UnicodeSegmentation;
                        lines.push(
                            line_text
                                .graphemes(true)
                                .skip(line_start)
                                .take(line_end - line_start)
                                .collect(),
                        );
                    } else {
                        // Line is too short for block selection
                        lines.push(String::new());
                    }
                }
            }
            // For block mode, join lines with newlines
            Some(lines.join("\n"))
        }
        _ => None,
    }
}

/// Searches forward for the visually selected text
/// Escapes regex special characters for literal search
/// Returns true if match found, false otherwise
#[must_use = "ignoring the return value means you won't know if the search succeeded"]
pub fn search_visual_selection_forward(editor: &mut Editor) -> bool {
    let selection_text = match get_visual_selection_text(editor) {
        Some(text) if !text.is_empty() => text,
        _ => {
            // Fall back to word under cursor if selection is empty
            match extract_word_at_cursor(editor) {
                Some(word) => word,
                None => return false,
            }
        }
    };

    setup_and_execute_search(editor, &selection_text, true)
}

/// Searches backward for the visually selected text
/// Escapes regex special characters for literal search
/// Returns true if match found, false otherwise
#[must_use = "ignoring the return value means you won't know if the search succeeded"]
pub fn search_visual_selection_backward(editor: &mut Editor) -> bool {
    let selection_text = match get_visual_selection_text(editor) {
        Some(text) if !text.is_empty() => text,
        _ => {
            // Fall back to word under cursor if selection is empty
            match extract_word_at_cursor(editor) {
                Some(word) => word,
                None => return false,
            }
        }
    };

    setup_and_execute_search(editor, &selection_text, false)
}

// ===================================================================
// Yank operations (moved from Operators struct for consolidation)
// ===================================================================

/// Yanks (copies) entire line(s)
pub fn yank_line(buffer: &crate::buffer::Buffer, count: usize) -> anyhow::Result<String> {
    let cursor = buffer.cursor();
    let start_line = cursor.line();
    let end_line = (start_line + count).min(buffer.line_count());

    if start_line >= buffer.line_count() {
        return Ok(String::new());
    }

    let start_char = buffer.rope().line_to_char(start_line);
    let end_char = if end_line < buffer.line_count() {
        buffer.rope().line_to_char(end_line)
    } else {
        buffer.rope().len_chars()
    };

    let mut yanked = buffer.rope().slice(start_char..end_char).to_string();

    // Ensure line yanks always end with newline (for line-wise paste behavior)
    if !yanked.ends_with('\n') {
        yanked.push('\n');
    }

    Ok(yanked)
}

// ===================================================================
// Auto-indent (moved from Operators struct for consolidation)
// ===================================================================

/// Auto-indents lines based on bracket context (= operator)
/// Returns the number of lines auto-indented
pub fn auto_indent_lines(
    buffer: &mut crate::buffer::Buffer,
    start_line: usize,
    end_line: usize,
    options: IndentOptions,
) -> anyhow::Result<usize> {
    let plan = crate::auto_indent::plan(buffer, start_line, end_line, options);
    Ok(apply_auto_indent_plan(buffer, &plan))
}

/// Auto-indents lines with undo tracking.
///
/// This mirrors `auto_indent_lines` but records all edits so `u`
/// restores the entire reindent in one step.
pub fn auto_indent_lines_with_tracking(
    editor: &mut Editor,
    start_line: usize,
    end_line: usize,
    options: IndentOptions,
) -> anyhow::Result<usize> {
    let plan = crate::auto_indent::plan(editor.buffer(), start_line, end_line, options);
    if plan.is_empty() {
        return Ok(0);
    }

    let cursor_before = editor.cursor_position();
    let last_cursor_after = plan
        .last()
        .map(|line| CursorPos::new(line.line, GraphemeCol(line.cursor_col)))
        .unwrap_or(cursor_before);
    let (lines_indented, edits) = editor
        .buffer_mut()
        .record(|buf| apply_auto_indent_plan(buf, &plan));

    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(last_cursor_after.line, last_cursor_after.col);

    if !edits.is_empty() {
        editor.push_recorded_undo(edits, cursor_before, last_cursor_after);
    }

    Ok(lines_indented)
}

fn apply_auto_indent_plan(
    buffer: &mut crate::buffer::Buffer,
    plan: &[crate::auto_indent::PlannedIndent],
) -> usize {
    let mut changed = 0usize;
    for line in plan {
        let Some(replacement) = &line.replacement else {
            continue;
        };
        if line.leading_chars > 0 {
            buffer.delete_range(
                line.line,
                CharCol::ZERO,
                line.line,
                CharCol(line.leading_chars),
            );
        }
        if !replacement.is_empty() {
            buffer.insert_text_at(line.line, CharCol::ZERO, replacement);
        }
        changed += 1;
    }
    changed
}

/// Insert a tab character or equivalent spaces, respecting expandtab.
pub fn insert_tab(editor: &mut Editor) -> Result<()> {
    let options = editor.indent_options();
    let cursor = editor.buffer().cursor();
    let line_idx = cursor.line();
    let grapheme_col = cursor.col();
    let (char_col, display_col) = {
        let line_text = editor.buffer().line_text(line_idx).unwrap_or_default();
        let char_col = grapheme_to_char_col(&line_text, grapheme_col);
        let before: String = line_text.chars().take(char_col.0).collect();
        (char_col, visual_width(&before, options.tab_width))
    };
    let text = options.tab_text(display_col);
    editor.record_session_edit(|buf| {
        buf.insert_text_at_positioning_cursor(line_idx, char_col, &text)
    });
    Ok(())
}

/// What a Ctrl-chord means in a single-line prompt (rename, `:` command line,
/// `/` search), following Vim's cmdline editing keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptControl {
    /// `CTRL-C` / `CTRL-[`: abandon the prompt (same as Esc).
    Cancel,
    /// `CTRL-H`: backspace.
    Backspace,
    /// `CTRL-U`: delete everything before the cursor.
    DeleteToStart,
    /// `CTRL-W`: delete the word before the cursor.
    DeleteWord,
    /// `CTRL-B`: cursor to the start.
    Home,
    /// `CTRL-E`: cursor to the end.
    End,
    /// Any other Ctrl chord: never inserted as a literal letter.
    Ignore,
}

/// Classifies `key` when it is a Ctrl+character chord; `None` for every other
/// key so the caller falls through to its normal handling.
pub fn prompt_control(key: &crate::KeyEvent) -> Option<PromptControl> {
    use crate::{KeyCode, Modifiers};
    if !key.modifiers.contains(Modifiers::CONTROL) {
        return None;
    }
    let KeyCode::Char(character) = key.code else {
        return None;
    };
    Some(match character.to_ascii_lowercase() {
        'c' | '[' => PromptControl::Cancel,
        'h' => PromptControl::Backspace,
        'u' => PromptControl::DeleteToStart,
        'w' => PromptControl::DeleteWord,
        'b' => PromptControl::Home,
        'e' => PromptControl::End,
        _ => PromptControl::Ignore,
    })
}
