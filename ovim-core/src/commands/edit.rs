//! Line-range editing commands: a bare range (go to line), `:d`, `:y`, `:j`,
//! `:sort`, `:t`/`:copy`, `:m`/`:move`, `:u`, `:red` and `:normal`.
//!
//! Cursor placement follows vim (checked with `nvim --clean --headless`):
//! `:d`, `:j` and `:sort` leave it on the first non-blank of the resulting
//! line, `:t` and `:m` on column 0 of the last copied or moved line, `:y`
//! does not move it.

use super::parse::{self, ParseError};
use super::range::{self, LineRange};
use super::Ex;
use crate::command_result::{err, ok, ok_silent, CommandResult};
use crate::editor::{Editor, InputHandler, RegisterType};
use crate::unicode::{CharCol, GraphemeCol};

/// Place the cursor on 0-based `line` at its first non-blank character.
pub(super) fn cursor_to_first_non_blank(editor: &mut Editor, line: usize) {
    let line = line.min(editor.buffer().line_count().saturating_sub(1));
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(line, GraphemeCol::ZERO);
    crate::editor::Motions::first_non_blank(editor.buffer_mut());
}

pub(super) fn unmodifiable() -> CommandResult {
    err("E21: Cannot make changes, 'modifiable' is off")
}

/// Text of 0-based lines `start..=end`, each with its line break.
pub(super) fn lines_text(editor: &Editor, start: usize, end: usize) -> String {
    let mut text = String::new();
    for index in start..=end {
        if let Some(line) = editor.buffer().line_text(index) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    text
}

/// Insert whole `lines` (each ending in `\n`) below 1-based line `after`
/// (0 = above the first line), keeping the buffer's own final line break.
pub(super) fn insert_lines_below(buffer: &mut crate::buffer::Buffer, after: usize, lines: &str) {
    if after < buffer.line_count() || after == 0 {
        buffer.insert_text_at(after, CharCol::ZERO, lines);
        return;
    }
    // Appending: the last line may lack a line break.
    let last = buffer.line_count() - 1;
    let end = CharCol(buffer.line_len(last));
    let text = lines.strip_suffix('\n').unwrap_or(lines);
    let ends_with_newline =
        buffer.rope().len_chars() > 0 && buffer.rope().char(buffer.rope().len_chars() - 1) == '\n';
    if ends_with_newline {
        buffer.insert_text_at(last + 1, CharCol::ZERO, lines);
    } else {
        buffer.insert_text_at(last, end, &format!("\n{text}"));
    }
}

/// A bare range: go to its last line (clamped like vim, which does not
/// report an error for `:999`).
pub(super) fn goto_line(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if let Some(range) = ex.range {
        let last = range::last_line(editor);
        let line = range.end.clamp(1, last.max(1));
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line - 1, GraphemeCol::ZERO);
    }
    ok_silent()
}

/// `[x] [count]` arguments of `:d` and `:y`: an optional register name and
/// an optional count that makes the range start at its last line.
fn register_and_count(ex: &Ex) -> Result<(Option<char>, LineRange), CommandResult> {
    let mut rest = ex.args.trim();
    let mut register = None;
    if let Some(first) = rest.chars().next() {
        if !first.is_ascii_digit() {
            register = Some(first);
            rest = rest[first.len_utf8()..].trim_start();
        }
    }
    let mut range = ex.range.expect("line-range command");
    if !rest.is_empty() {
        let count: usize = rest
            .parse()
            .map_err(|_| err(format!("E488: Trailing characters: {rest}")))?;
        if count == 0 {
            return Err(err("E939: Positive count required"));
        }
        range = LineRange {
            start: range.end,
            end: range.end + count - 1,
        };
    }
    Ok((register, range))
}

pub(super) fn delete(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let (register, range) = match register_and_count(ex) {
        Ok(parsed) => parsed,
        Err(error) => return error,
    };
    if !editor.buffer().is_modifiable() {
        return unmodifiable();
    }
    let last = range::last_line(editor);
    let (start, end) = range.indexes();
    let end = end.min(last.saturating_sub(1));
    let mut deleted = editor.record_operation(
        |buffer| {
            let deleted = buffer.delete_range(start, CharCol::ZERO, end + 1, CharCol::ZERO);
            let line = start.min(buffer.line_count().saturating_sub(1));
            buffer.cursor_mut().set_position(line, GraphemeCol::ZERO);
            crate::editor::Motions::first_non_blank(buffer);
            deleted
        },
        None,
    );
    if !deleted.ends_with('\n') {
        deleted.push('\n');
    }
    if let Some(register) = register {
        editor.set_pending_register(register);
    }
    editor.delete_to_register_with_type(deleted, RegisterType::Line);
    ok_silent()
}

pub(super) fn yank(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let (register, range) = match register_and_count(ex) {
        Ok(parsed) => parsed,
        Err(error) => return error,
    };
    let last = range::last_line(editor);
    let (start, end) = range.indexes();
    let text = lines_text(editor, start, end.min(last.saturating_sub(1)));
    if let Some(register) = register {
        editor.set_pending_register(register);
    }
    editor.yank_to_register_with_type(text, RegisterType::Line);
    ok_silent()
}

/// `:[range]j[oin][!] [count]`: a one-line range joins that line with the
/// next; `!` joins without adjusting white space (like `gJ`). A count joins
/// that many lines from the last line of the range. As in vim, two equal
/// addresses (`:2,2j`) and the last line have nothing to join.
pub(super) fn join(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if !editor.buffer().is_modifiable() {
        return unmodifiable();
    }
    let range = ex.range.expect("line-range command");
    let (mut start, mut end) = range.indexes();
    let mut addresses = ex.addresses;
    let count = ex.args.trim();
    if !count.is_empty() {
        match count.parse::<usize>() {
            Ok(0) => return err("E939: Positive count required"),
            Ok(count) => {
                start = end;
                end = end
                    .saturating_add(count - 1)
                    .min(range::last_line(editor).saturating_sub(1));
                addresses += 1;
            }
            Err(_) => return err(format!("E488: Trailing characters: {count}")),
        }
    }
    if start == end {
        if addresses >= 2 || end + 1 >= range::last_line(editor) {
            cursor_to_first_non_blank(editor, start);
            return ok_silent();
        }
        end += 1;
    }
    let lines = end - start + 1;
    let keep_spaces = ex.bang;
    let result = editor.record_operation(
        |buffer| {
            buffer.cursor_mut().set_position(start, GraphemeCol::ZERO);
            let result = if keep_spaces {
                buffer.join_lines_no_space(lines)
            } else {
                buffer.join_lines(lines)
            };
            crate::editor::Motions::first_non_blank(buffer);
            result
        },
        None,
    );
    match result {
        Ok(()) => ok_silent(),
        Err(error) => err(format!("Failed to join lines: {error}")),
    }
}

/// The first decimal number in `line` for `:sort n`.
fn first_number(line: &str) -> Option<i64> {
    let start = line.find(|c: char| c.is_ascii_digit())?;
    let negative = line[..start].ends_with('-');
    let digits: String = line[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let value: i64 = digits.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// `:[range]sor[t][!] [n][i][u]` (default range: the whole buffer).
pub(super) fn sort(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if !editor.buffer().is_modifiable() {
        return unmodifiable();
    }
    let flags = ex.args;
    if let Some(bad) = flags.chars().find(|c| !matches!(c, 'n' | 'i' | 'u' | ' ')) {
        return err(format!("E474: Invalid argument: {bad}"));
    }
    let numeric = flags.contains('n');
    let ignore_case = flags.contains('i');
    let unique = flags.contains('u');
    let range = ex.range.expect("line-range command");
    let (start, end) = range.indexes();
    let mut lines: Vec<String> = (start..=end)
        .filter_map(|index| {
            editor
                .buffer()
                .line_text(index)
                .map(|line| line.into_owned())
        })
        .collect();
    let key = |line: &String| {
        if ignore_case {
            line.to_lowercase()
        } else {
            line.clone()
        }
    };
    // Stable sorts: vim's :sort keeps the order of equal lines, and with `n`
    // lines without a number come first in their original order.
    if numeric {
        lines.sort_by_key(|line| first_number(line).map_or((0, 0), |n| (1, n)));
    } else {
        lines.sort_by_key(key);
    }
    if ex.bang {
        lines.reverse();
    }
    if unique {
        lines.dedup_by(|a, b| key(a) == key(b));
    }
    let sorted = lines.len();
    let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
    editor.record_operation(
        |buffer| {
            buffer.delete_range(start, CharCol::ZERO, end + 1, CharCol::ZERO);
            insert_lines_below(buffer, start, &text);
            buffer.cursor_mut().set_position(start, GraphemeCol::ZERO);
            crate::editor::Motions::first_non_blank(buffer);
        },
        None,
    );
    ok(format!("{sorted} lines sorted"))
}

/// Parse the destination of `:t` / `:m` (0 allowed, past the end is E16).
fn destination(editor: &Editor, args: &str) -> Result<usize, CommandResult> {
    let address = match parse::address(args) {
        Ok(Some(address)) => address,
        Ok(None) | Err(ParseError::InvalidAddress) => return Err(err("E16: Invalid range")),
        Err(ParseError::TrailingCharacters(rest)) => {
            return Err(err(format!("E488: Trailing characters: {rest}")))
        }
        Err(other) => return Err(err(other.message(args))),
    };
    let line = range::eval_address(editor, &address, range::cursor_line(editor)).map_err(err)?;
    if line > range::last_line(editor) {
        return Err(err("E16: Invalid range"));
    }
    Ok(line)
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// `:[range]t {address}` / `:[range]co[py] {address}`: copy below the address.
pub(super) fn copy(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if !editor.buffer().is_modifiable() {
        return unmodifiable();
    }
    let dest = match destination(editor, ex.args) {
        Ok(dest) => dest,
        Err(error) => return error,
    };
    let range = ex.range.expect("line-range command");
    let (start, end) = range.indexes();
    let text = lines_text(editor, start, end);
    let count = end - start + 1;
    editor.record_operation(
        |buffer| {
            insert_lines_below(buffer, dest, &text);
            buffer
                .cursor_mut()
                .set_position(dest + count - 1, GraphemeCol::ZERO);
        },
        None,
    );
    ok(format!("{count} line{} copied", plural(count)))
}

/// `:[range]m[ove] {address}`: move below the address.
pub(super) fn move_lines(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if !editor.buffer().is_modifiable() {
        return unmodifiable();
    }
    let dest = match destination(editor, ex.args) {
        Ok(dest) => dest,
        Err(error) => return error,
    };
    let range = ex.range.expect("line-range command");
    let (first, last) = (range.start.max(1), range.end);
    if dest >= first && dest < last {
        return err("E134: Cannot move a range of lines into itself");
    }
    let count = last - first + 1;
    let text = lines_text(editor, first - 1, last - 1);
    // Below the block the destination shifts up by the moved lines.
    let target = if dest > last { dest - count } else { dest };
    editor.record_operation(
        |buffer| {
            if dest != first - 1 && dest != last {
                buffer.delete_range(first - 1, CharCol::ZERO, last, CharCol::ZERO);
                insert_lines_below(buffer, target, &text);
            }
            buffer
                .cursor_mut()
                .set_position(target + count - 1, GraphemeCol::ZERO);
        },
        None,
    );
    ok(format!("{count} line{} moved", plural(count)))
}

/// `:[range]p[rint]`: show the lines; the cursor goes to the last one.
pub(super) fn print(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let (start, end) = ex.range.expect("line-range command").indexes();
    let text = lines_text(editor, start, end);
    cursor_to_first_non_blank(editor, end);
    ok(text.trim_end_matches('\n').to_string())
}

pub(super) fn undo(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.undo();
    ok_silent()
}

pub(super) fn redo(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    editor.redo();
    ok_silent()
}

/// `:[range]norm[al][!] {commands}`: type `commands` in Normal mode, once or
/// on every line of the range with the cursor at its start. `!` ignores
/// mappings. An unfinished command is ended as if <Esc> was typed.
pub(super) fn normal(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.args.is_empty() {
        return err("E471: Argument required");
    }
    one_undo_step(editor, |editor| run_normal(editor, ex))
}

/// Run `body` so that everything it changes in the current buffer is undone
/// by one `u` (vim's `:normal` over a range, `:g`).
pub(super) fn one_undo_step(
    editor: &mut Editor,
    body: impl FnOnce(&mut Editor) -> CommandResult,
) -> CommandResult {
    let buffer = editor.buffer().id();
    let mark = editor.buffer().change_manager().undo_mark();
    let result = body(editor);
    if editor.buffer().id() == buffer {
        editor.buffer_mut().change_manager_mut().group_since(mark);
    }
    result
}

fn run_normal(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let remap = !ex.bang;
    let lines = match ex.range {
        Some(range) if ex.explicit_range => range.start.max(1)..=range.end,
        _ => {
            return match InputHandler::type_normal_keys(editor, ex.args, remap) {
                Ok(()) => ok_silent(),
                Err(error) => err(error.to_string()),
            }
        }
    };
    for line in lines {
        if line > range::last_line(editor) {
            break;
        }
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line - 1, GraphemeCol::ZERO);
        if let Err(error) = InputHandler::type_normal_keys(editor, ex.args, remap) {
            return err(error.to_string());
        }
    }
    ok_silent()
}

#[cfg(test)]
mod tests {
    use super::first_number;

    #[test]
    fn sort_n_uses_the_first_decimal_number_anywhere_in_the_line() {
        assert_eq!(first_number("x 10"), Some(10));
        assert_eq!(first_number("a-3b"), Some(-3));
        assert_eq!(first_number("none"), None);
    }
}
