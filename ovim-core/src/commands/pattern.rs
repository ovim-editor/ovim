//! Pattern commands: `:s[ubstitute]`, `:g[lobal]`, `:v[global]`.
//!
//! Patterns are Rust regexes (as they always were here), not vim regexes.

use super::edit::cursor_to_first_non_blank;
use super::marked_lines::MarkedLines;
use super::parse::parse;
use super::Ex;
use crate::command_result::{err, ok, ok_silent, CommandResult};
use crate::edit::Edit;
use crate::editor::{CursorPos, Editor, RegisterType};
use crate::search_pattern::{self, CaseOptions};
use crate::unicode::{CharCol, GraphemeCol};

/// Converts Vim-style backreferences (\1, \2, \0, &) to Rust regex syntax.
///
/// Backrefs are emitted in the braced `${N}` form so a following word character
/// doesn't get absorbed into the group name — Rust regex reads `$1foo` as a
/// reference to a group literally named "1foo" (which doesn't exist, so it
/// expands to empty). A literal `$` in the Vim replacement is escaped to `$$`
/// so Rust regex emits it verbatim instead of treating it as a capture ref.
/// `\r` becomes a line break and `\t` a tab, as in Vim.
fn convert_vim_backrefs(replacement: &str) -> String {
    let mut result = String::with_capacity(replacement.len() * 2);
    let mut chars = replacement.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek() {
                Some(&digit @ '0'..='9') => {
                    chars.next();
                    result.push_str("${");
                    result.push(digit);
                    result.push('}');
                }
                Some('\\') => {
                    chars.next();
                    result.push('\\');
                }
                Some('r') => {
                    // Vim: \r in the replacement is a line break. (\n would
                    // be a NUL byte in Vim; we leave it alone rather than
                    // emulate that trap.)
                    chars.next();
                    result.push('\n');
                }
                Some('t') => {
                    chars.next();
                    result.push('\t');
                }
                _ => result.push(ch),
            }
        } else if ch == '&' {
            result.push_str("${0}");
        } else if ch == '$' {
            result.push_str("$$");
        } else {
            result.push(ch);
        }
    }

    result
}

/// Splits `{delim}pattern{delim}replacement{delim}flags` into its fields.
/// `\{delim}` is a literal delimiter; other escapes are kept for the regex
/// and replacement engines. The replacement and flags may be omitted (vim:
/// `:s/pat` deletes the match). Returns `None` for an invalid delimiter.
fn split_substitute_parts(body: &str) -> Option<(String, String, String)> {
    let mut chars = body.chars();
    let delimiter = chars.next()?;
    if delimiter.is_alphanumeric() || matches!(delimiter, '\\' | '"' | '|' | ' ') {
        return None;
    }

    let mut fields: Vec<String> = vec![String::new()];
    let mut escaped = false;
    for ch in chars {
        let fields_so_far = fields.len();
        let field = fields.last_mut().expect("at least one field");
        if escaped {
            if ch != delimiter {
                field.push('\\');
            }
            field.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == delimiter && fields_so_far < 3 {
            fields.push(String::new());
        } else {
            field.push(ch);
        }
    }
    if escaped {
        fields.last_mut().expect("at least one field").push('\\');
    }
    let mut fields = fields.into_iter();
    let pattern = fields.next().unwrap_or_default();
    let replacement = fields.next().unwrap_or_default();
    let flags = fields.next().unwrap_or_default();
    Some((pattern, replacement, flags))
}

/// A parsed `:s` body ready to run on lines.
struct Substitution {
    regex: regex::Regex,
    replacement: String,
    /// The replacement as typed, for the confirm prompt.
    typed_replacement: String,
    global: bool,
    pattern: String,
}

impl Substitution {
    fn apply(&self, line: &str) -> String {
        if self.global {
            self.regex.replace_all(line, self.replacement.as_str())
        } else {
            self.regex.replace(line, self.replacement.as_str())
        }
        .into_owned()
    }
}

fn parse_substitution(
    editor: &mut Editor,
    body: &str,
) -> Result<(Substitution, String), CommandResult> {
    if body.is_empty() {
        return Err(err("E33: No previous substitute regular expression"));
    }
    let Some((raw_pattern, raw_replacement, flags)) = split_substitute_parts(body) else {
        return Err(err(
            "E146: Regular expressions can't be delimited by letters",
        ));
    };
    // An empty pattern reuses the last search (`:%s//bar/`).
    let pattern = if raw_pattern.is_empty() {
        let last = editor.registers().get_last_search().to_string();
        if last.is_empty() {
            return Err(err("E35: No previous regular expression"));
        }
        last
    } else {
        raw_pattern
    };
    // `i` / `I` override 'ignorecase' and 'smartcase' for this command.
    let forced = if flags.contains('I') {
        Some(false)
    } else {
        flags.contains('i').then_some(true)
    };
    let regex = search_pattern::compile(&pattern, CaseOptions::of(&editor.options), forced)
        .map_err(|_| err(format!("Invalid regex pattern: {pattern}")))?;
    editor.set_last_search_pattern(&pattern);
    Ok((
        Substitution {
            regex,
            replacement: convert_vim_backrefs(&raw_replacement),
            typed_replacement: raw_replacement,
            global: flags.contains('g'),
            pattern,
        },
        flags,
    ))
}

/// What [`substitute_lines`] changed.
struct Substituted {
    /// How many lines the substitution changed.
    lines: usize,
    /// The last changed line (0-based), after any line splits.
    last_line: Option<usize>,
}

/// Substitute on 0-based `lines` as one undo step, bottom-up: a replacement
/// containing `\r` splits its line and would shift the lines below it
/// (OV-00244).
fn substitute_lines(
    editor: &mut Editor,
    substitution: &Substitution,
    lines: &[usize],
) -> Substituted {
    let cursor_before = editor.cursor_position();
    let ((last_line, changed), edits) = editor.buffer_mut().record(|buffer| {
        let mut last_changed = None;
        let mut changed = 0;
        for &line in lines.iter().rev() {
            let Some(text) = buffer.line_text(line) else {
                continue;
            };
            let replaced = substitution.apply(&text);
            if replaced != text {
                let length = text.chars().count();
                buffer.delete_range(line, CharCol::ZERO, line, CharCol(length));
                buffer.insert_text_at(line, CharCol::ZERO, &replaced);
                // Lines split by `\r` push the last changed line down.
                let added = replaced.matches('\n').count();
                last_changed = Some(last_changed.map_or(line + added, |last: usize| last + added));
                changed += 1;
            }
        }
        (last_changed, changed)
    });
    if !edits.is_empty() {
        if let Some(line) = last_line {
            cursor_to_first_non_blank(editor, line);
        }
        let cursor_after = editor.cursor_position();
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
    }
    Substituted {
        lines: changed,
        last_line,
    }
}

/// `:s///n`: report how many matches (one per line without `g`) the
/// substitution would replace, leaving the buffer and cursor alone.
fn count_matches(
    editor: &Editor,
    substitution: &Substitution,
    lines: std::ops::RangeInclusive<usize>,
    quiet: bool,
) -> CommandResult {
    let (mut matches, mut matching_lines) = (0, 0);
    for line in lines {
        let Some(text) = editor.buffer().line_text(line) else {
            continue;
        };
        let found = if substitution.global {
            substitution.regex.find_iter(&text).count()
        } else {
            usize::from(substitution.regex.is_match(&text))
        };
        if found > 0 {
            matches += found;
            matching_lines += 1;
        }
    }
    if matches == 0 {
        return if quiet {
            ok_silent()
        } else {
            err(format!("E486: Pattern not found: {}", substitution.pattern))
        };
    }
    ok(format!(
        "{matches} match{} on {matching_lines} line{}",
        if matches == 1 { "" } else { "es" },
        if matching_lines == 1 { "" } else { "s" }
    ))
}

/// `:[range]s[ubstitute]/{pattern}/{string}/[flags]` with flags `g`, `i`,
/// `I`, `c` (confirm each), `e` (no error when nothing matches) and `n`
/// (only count the matches).
pub(super) fn substitute(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let (substitution, flags) = match parse_substitution(editor, ex.args) {
        Ok(parsed) => parsed,
        Err(error) => return error,
    };
    let (start, end) = ex.range.expect("line-range command").indexes();
    let end = end.min(editor.buffer().line_count().saturating_sub(1));

    // Counting changes nothing, so it also works in a read-only buffer.
    if flags.contains('n') {
        return count_matches(editor, &substitution, start..=end, flags.contains('e'));
    }
    if !editor.buffer().is_modifiable() {
        return super::edit::unmodifiable();
    }
    if flags.contains('c') {
        let mut matches = Vec::new();
        for line in start..=end {
            let Some(text) = editor.buffer().line_text(line) else {
                continue;
            };
            let found: Vec<_> = if substitution.global {
                substitution.regex.find_iter(&text).collect()
            } else {
                substitution.regex.find(&text).into_iter().collect()
            };
            for found in found {
                let replacement = substitution
                    .regex
                    .replace(found.as_str(), substitution.replacement.as_str())
                    .into_owned();
                let start_char = text[..found.start()].chars().count();
                let end_char = text[..found.end()].chars().count();
                matches.push((line, start_char, end_char, replacement));
            }
        }
        if matches.is_empty() {
            return err(format!("E486: Pattern not found: {}", substitution.pattern));
        }
        // The typed replacement keeps the prompt on one line (`\r`).
        let message = format!(
            "replace with {} ({} matches) (y/n/a/q/l)",
            substitution.typed_replacement,
            matches.len()
        );
        editor.start_substitute_confirm(matches, substitution.regex);
        return ok(message);
    }

    let lines: Vec<usize> = (start..=end).collect();
    if substitute_lines(editor, &substitution, &lines).lines == 0 && !flags.contains('e') {
        // `:cdo` / `:cfdo` rely on this error to stop at a failing entry.
        return err(format!("E486: Pattern not found: {}", substitution.pattern));
    }
    ok_silent()
}

/// Read the `/pattern/` of `:g` and the command after it.
fn split_global(args: &str) -> Option<(String, &str)> {
    let delimiter = args.chars().next()?;
    if delimiter.is_alphanumeric() || matches!(delimiter, '\\' | '"' | '|' | ' ') {
        return None;
    }
    let body = &args[delimiter.len_utf8()..];
    let mut pattern = String::new();
    let mut escaped = false;
    for (index, c) in body.char_indices() {
        if escaped {
            if c != delimiter {
                pattern.push('\\');
            }
            pattern.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delimiter {
            return Some((pattern, body[index + c.len_utf8()..].trim()));
        } else {
            pattern.push(c);
        }
    }
    Some((pattern, ""))
}

/// `:[range]g[lobal][!]/{pattern}/[cmd]` and `:[range]v[global]/{pattern}/[cmd]`
/// (default range: the whole buffer, default command: `:p`). `:d`, `:y`, `:s`
/// and `:p` run on all the lines at once; anything else, and any of those
/// followed by `|`, runs line by line with the cursor on each line, as one
/// undo step. The marked lines move with their text: see [`MarkedLines`].
pub(super) fn global(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let invert = ex.bang || ex.command.names[0].starts_with('v');
    let Some((pattern, command)) = split_global(ex.args) else {
        return err("E476: Invalid command");
    };
    let pattern = if pattern.is_empty() {
        let last = editor.registers().get_last_search().to_string();
        if last.is_empty() {
            return err("E35: No previous regular expression");
        }
        last
    } else {
        pattern
    };
    let regex = match search_pattern::compile(&pattern, CaseOptions::of(&editor.options), None) {
        Ok(regex) => regex,
        Err(_) => return err(format!("Invalid regex pattern: {pattern}")),
    };
    editor.set_last_search_pattern(&pattern);
    let range = ex.range.expect("line-range command");
    let (start, end) = range.indexes();
    let end = end.min(editor.buffer().line_count().saturating_sub(1));
    let lines: Vec<usize> = (start..=end)
        .filter(|&line| {
            editor
                .buffer()
                .line_text(line)
                .is_some_and(|text| regex.is_match(&text) != invert)
        })
        .collect();
    if lines.is_empty() {
        return ok(if invert {
            format!("Pattern found in every line: {pattern}")
        } else {
            format!("Pattern not found: {pattern}")
        });
    }
    let command = if command.is_empty() { "p" } else { command };
    let sub = match parse(command) {
        Ok(sub) => sub,
        Err(error) => return err(error.message(command)),
    };
    if matches!(sub.command.names[0], "g[lobal]" | "v[global]") {
        return err("E147: Cannot do :global recursive");
    }
    let whole = sub.range.is_none();
    match sub.command.names[0] {
        "d[elete]" if whole => global_delete(editor, &lines),
        "y[ank]" if whole => {
            let text = lines
                .iter()
                .filter_map(|&line| editor.buffer().line_text(line))
                .map(|text| format!("{text}\n"))
                .collect();
            editor.yank_to_register_with_type(text, RegisterType::Line);
            ok(format!("Yanked {} line(s)", lines.len()))
        }
        "p[rint]" if whole => {
            let mut output: Vec<String> = lines
                .iter()
                .take(10)
                .filter_map(|&line| {
                    editor
                        .buffer()
                        .line_text(line)
                        .map(|text| format!("{}: {text}", line + 1))
                })
                .collect();
            if lines.len() > 10 {
                output.push(format!("... and {} more lines", lines.len() - 10));
            }
            cursor_to_first_non_blank(editor, *lines.last().expect("non-empty"));
            ok(output.join("\n"))
        }
        "s[ubstitute]" if whole => {
            let (substitution, _) = match parse_substitution(editor, sub.args) {
                Ok(parsed) => parsed,
                Err(error) => return error,
            };
            let substituted = substitute_lines(editor, &substitution, &lines);
            ok(format!("Substituted on {} line(s)", substituted.lines))
        }
        _ => super::edit::one_undo_step(editor, |editor| global_each_line(editor, &lines, command)),
    }
}

fn global_delete(editor: &mut Editor, lines: &[usize]) -> CommandResult {
    if !editor.buffer().is_modifiable() {
        return super::edit::unmodifiable();
    }
    let cursor_before = editor.cursor_position();
    let (mut deleted, edits) = editor.buffer_mut().record(|buffer| {
        let mut deleted = Vec::new();
        for &line in lines.iter().rev() {
            let text = buffer.delete_range(line, CharCol::ZERO, line + 1, CharCol::ZERO);
            if !text.is_empty() {
                deleted.push(text);
            }
        }
        deleted
    });
    deleted.reverse();
    editor.delete_to_register_with_type(deleted.concat(), RegisterType::Line);
    // vim: the cursor ends where the last deleted line was.
    let last = lines[lines.len() - 1] + 1 - lines.len();
    cursor_to_first_non_blank(editor, last);
    if !edits.is_empty() {
        let cursor_after = CursorPos::new(
            editor.buffer().cursor().line(),
            editor.buffer().cursor().col(),
        );
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
    }
    ok(format!("Deleted {} line(s)", lines.len()))
}

/// How many chars `edits` added to the text.
fn net_change(edits: &[&Edit]) -> isize {
    edits
        .iter()
        .map(|edit| match edit {
            Edit::Insert { text, .. } => text.chars().count() as isize,
            Edit::Delete { text, .. } => -(text.chars().count() as isize),
        })
        .sum()
}

/// Run `command` with the cursor on each of the 0-based `lines`, top-down.
/// Vim marks the lines and visits them wherever the commands before moved
/// them, skipping the ones they deleted or joined away. An error stops the
/// rest, as in vim.
fn global_each_line(editor: &mut Editor, lines: &[usize], command: &str) -> CommandResult {
    let buffer = editor.buffer().id();
    let mut marked = MarkedLines::new(editor.buffer().rope(), lines);
    let mut result = ok_silent();
    while let Some(line) = marked.next(editor.buffer().rope()) {
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(line, GraphemeCol::ZERO);
        let version = editor.buffer().version();
        let chars_before = editor.buffer().rope().len_chars();
        let outcome = super::run_line(editor, command);
        if let CommandResult::Error(_) = outcome {
            return outcome;
        }
        if editor.buffer().id() != buffer {
            break;
        }
        let chars_after = editor.buffer().rope().len_chars();
        match editor.buffer().edit_log().edits_since(version as u64) {
            Some(edits) if net_change(&edits) == chars_after as isize - chars_before as isize => {
                marked.follow(&edits)
            }
            // The log lost some of the edits (it keeps a limited number, and
            // is cleared by bulk changes): assume they all came before the
            // lines still to visit.
            _ => marked.follow_unknown(chars_after as isize - chars_before as isize),
        }
        if let CommandResult::Success(success) = &outcome {
            if success.message.is_some() {
                result = outcome;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_vim_replacement_tokens_without_ambiguous_capture_names() {
        assert_eq!(
            convert_vim_backrefs(r"\1-\0-&-$-\r-\t-\\"),
            "${1}-${0}-${0}-$$-\n-\t-\\"
        );
    }

    #[test]
    fn substitute_parser_takes_any_delimiter_and_optional_fields() {
        assert_eq!(
            split_substitute_parts(r"/a\/b/c\/d/g"),
            Some(("a/b".to_owned(), "c/d".to_owned(), "g".to_owned()))
        );
        assert_eq!(
            split_substitute_parts("#a#b#"),
            Some(("a".to_owned(), "b".to_owned(), String::new()))
        );
        assert_eq!(
            split_substitute_parts("/pattern"),
            Some(("pattern".to_owned(), String::new(), String::new()))
        );
        assert_eq!(split_substitute_parts("abc"), None);
    }
}
