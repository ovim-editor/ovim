use anyhow::Result;

use crate::editor::{CursorPos, Editor};
use crate::unicode::{CharCol, GraphemeCol};

use super::range::parse_range_with_status;
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
            if let Some(&next) = chars.peek() {
                match next {
                    '0'..='9' => {
                        // \1 -> ${1}, \2 -> ${2}, etc.
                        result.push_str("${");
                        result.push(chars.next().unwrap());
                        result.push('}');
                    }
                    '\\' => {
                        // \\ -> \
                        result.push('\\');
                        chars.next();
                    }
                    'r' => {
                        // Vim: \r in the replacement is a line break. (\n would
                        // be a NUL byte in Vim; we leave it alone rather than
                        // emulate that trap.)
                        result.push('\n');
                        chars.next();
                    }
                    't' => {
                        // Vim: \t in the replacement is a tab.
                        result.push('\t');
                        chars.next();
                    }
                    _ => {
                        // Keep other escapes as-is
                        result.push(ch);
                    }
                }
            } else {
                result.push(ch);
            }
        } else if ch == '&' {
            // & means whole match in Vim, ${0} in Rust
            result.push_str("${0}");
        } else if ch == '$' {
            // A literal '$' in a Vim replacement — escape it for Rust regex.
            result.push_str("$$");
        } else {
            result.push(ch);
        }
    }

    result
}

/// Splits a substitute body `/pattern/replacement/flags` into its three fields,
/// honoring backslash-escaped delimiters (`\/`) inside the pattern/replacement.
/// The escaped delimiter is unescaped to a literal `/` (which is not a regex
/// metacharacter); all other backslash escapes are preserved for the regex /
/// replacement engines. Returns `None` if there is no replacement delimiter.
fn split_substitute_parts(body: &str) -> Option<(String, String, String)> {
    let mut chars = body.chars();
    if chars.next() != Some('/') {
        return None;
    }

    let mut fields: Vec<String> = vec![String::new()];
    let mut escaped = false;
    for ch in chars {
        if escaped {
            let cur = fields.last_mut().unwrap();
            if ch == '/' {
                cur.push('/');
            } else {
                cur.push('\\');
                cur.push(ch);
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '/' {
            fields.push(String::new());
        } else {
            fields.last_mut().unwrap().push(ch);
        }
    }
    if escaped {
        fields.last_mut().unwrap().push('\\');
    }

    if fields.len() < 2 {
        return None;
    }
    let pattern = fields[0].clone();
    let replacement = fields.get(1).cloned().unwrap_or_default();
    let flags = fields.get(2).cloned().unwrap_or_default();
    Some((pattern, replacement, flags))
}

/// Handles substitute command (:s/pattern/replacement/flags)
///
/// `range_str` is the pre-parsed range portion (e.g., "%", "'<,'>", "1,5", or "").
/// `cmd_part` is the command portion starting with "s/" (e.g., "s/foo/bar/g").
pub(super) fn handle_substitute_command(
    editor: &mut Editor,
    range_str: &str,
    cmd_part: &str,
) -> Result<()> {
    // cmd_part starts with "s/..." — strip the leading "s" to get "/pattern/replacement/flags"
    let substitute_part = &cmd_part[1..];

    // Parse substitute pattern: /pattern/replacement/flags
    if !substitute_part.starts_with('/') {
        editor.set_status_message("E146: Invalid substitute syntax".to_string());
        return Ok(()); // Invalid format
    }

    let Some((raw_pattern, raw_replacement, flags_str)) = split_substitute_parts(substitute_part)
    else {
        editor.set_status_message("E146: Invalid substitute syntax".to_string());
        return Ok(()); // Invalid format - need at least /pattern/replacement/
    };

    // Empty pattern reuses last search (Vim behavior: :%s//bar/ means "replace last search with bar")
    let pattern = if raw_pattern.is_empty() {
        let last = editor.registers().get_last_search().to_string();
        if last.is_empty() {
            editor.set_status_message("E35: No previous regular expression".to_string());
            return Ok(());
        }
        last
    } else {
        // Update last search register with this pattern
        editor.registers_mut().set_last_search(raw_pattern.clone());
        raw_pattern
    };
    // Convert Vim-style backreferences to Rust regex syntax
    let replacement = convert_vim_backrefs(&raw_replacement);
    let flags = flags_str.as_str();

    // Parse flags
    let global = flags.contains('g');
    let ignore_case = flags.contains('i');
    let confirm = flags.contains('c');

    // Determine the range using the new parser (returns inclusive range)
    let (start_line, end_line) =
        if let Some((start, end)) = parse_range_with_status(editor, range_str, None) {
            (start, end)
        } else {
            // Invalid range
            return Ok(());
        };

    // Compile the regex pattern
    use regex::RegexBuilder;
    let regex = match RegexBuilder::new(&pattern)
        .case_insensitive(ignore_case)
        .build()
    {
        Ok(r) => r,
        Err(_) => {
            editor.set_status_message(format!("Invalid regex pattern: {}", pattern));
            return Ok(());
        }
    };

    // If confirm flag is set, collect matches and enter SubstituteConfirm mode
    if confirm {
        let mut matches = Vec::new();

        for line_idx in start_line..=end_line.min(editor.buffer().line_count().saturating_sub(1)) {
            if let Some(line_text) = editor.buffer().line_text(line_idx) {
                // Find all matches in this line
                if global {
                    for mat in regex.find_iter(&line_text) {
                        let replacement_text = regex
                            .replace(mat.as_str(), replacement.as_str())
                            .to_string();
                        // Convert byte offsets to char indices for delete_range
                        let start_char = line_text[..mat.start()].chars().count();
                        let end_char = line_text[..mat.end()].chars().count();
                        matches.push((line_idx, start_char, end_char, replacement_text));
                    }
                } else {
                    // Only first match per line
                    if let Some(mat) = regex.find(&line_text) {
                        let replacement_text = regex
                            .replace(mat.as_str(), replacement.as_str())
                            .to_string();
                        // Convert byte offsets to char indices for delete_range
                        let start_char = line_text[..mat.start()].chars().count();
                        let end_char = line_text[..mat.end()].chars().count();
                        matches.push((line_idx, start_char, end_char, replacement_text));
                    }
                }
            }
        }

        if matches.is_empty() {
            editor.set_status_message("Pattern not found".to_string());
        } else {
            let count = matches.len();
            editor.set_status_message(format!(
                "replace with {} ({} matches) (y/n/a/q/l)",
                replacement, count
            ));
            editor.start_substitute_confirm(matches, regex);
        }

        return Ok(());
    }

    // Perform substitution atomically (single undo for all lines)
    let cursor_before = editor.cursor_position();

    let ((), edits) = editor.buffer_mut().record(|buf| {
        // Iterate bottom-up: a replacement containing `\r` (a line break) splits
        // its line and shifts every line below it, so top-down iteration would
        // process the freshly inserted continuation lines and skip the
        // originally targeted ones (OV-00244). Lines above the edit keep their
        // indices, so reverse order visits each original line exactly once.
        let last_line = end_line.min(buf.line_count().saturating_sub(1));
        for line_idx in (start_line..=last_line).rev() {
            if let Some(line_text) = buf.line_text(line_idx) {
                let new_text = if global {
                    regex
                        .replace_all(&line_text, replacement.as_str())
                        .to_string()
                } else {
                    regex.replace(&line_text, replacement.as_str()).to_string()
                };

                if new_text != line_text {
                    let line_len = line_text.chars().count();
                    buf.delete_range(line_idx, CharCol::ZERO, line_idx, CharCol(line_len));
                    buf.insert_text_at(line_idx, CharCol::ZERO, &new_text);
                }
            }
        }
    });

    if !edits.is_empty() {
        let cursor_after = editor.cursor_position();
        editor.push_recorded_undo(edits, cursor_before, cursor_after);
    }

    Ok(())
}

/// Handles :global and :vglobal commands (:g/pattern/command, :v/pattern/command)
pub(super) fn handle_global_command(
    editor: &mut Editor,
    command: &str,
    range: Option<(usize, usize)>,
) -> Result<()> {
    // Parse the command: g/pattern/command or v/pattern/command or g!/pattern/command
    // :g/pattern/command - execute command on lines matching pattern
    // :v/pattern/command or :g!/pattern/command - execute command on lines NOT matching pattern

    let invert = command.starts_with("v/") || command.starts_with("g!/");
    let cmd_start = if command.starts_with("g!/") { 2 } else { 1 };

    let rest = &command[cmd_start..];

    // Extract pattern and command
    if !rest.starts_with('/') {
        editor.set_status_message("Invalid global command format".to_string());
        return Ok(());
    }

    // Find the closing / for the pattern, honoring escaped delimiters (\/)
    let mut pattern_end: Option<usize> = None;
    let mut escaped = false;
    for (i, ch) in rest[1..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '/' {
            pattern_end = Some(i + 1);
            break;
        }
    }

    let Some(pattern_end) = pattern_end else {
        editor.set_status_message("Invalid global command: missing closing /".to_string());
        return Ok(());
    };

    let pattern = &rest[1..pattern_end];
    let sub_command = if pattern_end + 1 < rest.len() {
        rest[pattern_end + 1..].trim()
    } else {
        ""
    };

    // Default command is 'p' (print) if none specified
    let sub_command = if sub_command.is_empty() {
        "p"
    } else {
        sub_command
    };

    // Compile regex
    use regex::Regex;
    let regex = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => {
            editor.set_status_message(format!("Invalid regex pattern: {}", pattern));
            return Ok(());
        }
    };

    // Find all matching lines (optionally restricted by Ex range)
    let line_count = editor.buffer().line_count();
    if line_count == 0 {
        editor.set_status_message("No matching lines found".to_string());
        return Ok(());
    }

    let (scan_start, scan_end) = range.unwrap_or((0, line_count.saturating_sub(1)));
    let scan_start = scan_start.min(line_count.saturating_sub(1));
    let scan_end = scan_end.min(line_count.saturating_sub(1));

    let mut matching_lines = Vec::new();

    for line_idx in scan_start..=scan_end {
        if let Some(line_text) = editor.buffer().line_text(line_idx) {
            let matches = regex.is_match(&line_text);

            // Include line if: (matches && !invert) || (!matches && invert)
            if matches != invert {
                matching_lines.push(line_idx);
            }
        }
    }

    if matching_lines.is_empty() {
        editor.set_status_message("No matching lines found".to_string());
        return Ok(());
    }

    // Execute the command on matching lines
    match sub_command {
        "d" | "delete" => {
            let cursor_before = editor.cursor_position();
            let (mut all_deleted, edits) = editor.buffer_mut().record(|buf| {
                let mut deleted_chunks = Vec::new();
                for &line_idx in matching_lines.iter().rev() {
                    let deleted =
                        buf.delete_range(line_idx, CharCol::ZERO, line_idx + 1, CharCol::ZERO);
                    if deleted.is_empty() {
                        continue;
                    }
                    deleted_chunks.push(deleted);
                }
                deleted_chunks
            });

            // Store in register
            all_deleted.reverse(); // Restore original order
            let deleted_text = all_deleted.join("");
            editor.delete_to_register(deleted_text);

            // Position cursor at first deleted line
            let new_cursor_line =
                matching_lines[0].min(editor.buffer().line_count().saturating_sub(1));
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(new_cursor_line, GraphemeCol::ZERO);

            if !edits.is_empty() {
                let cursor_after = CursorPos::new(new_cursor_line, GraphemeCol::ZERO);
                editor.push_recorded_undo(edits, cursor_before, cursor_after);
            }

            editor.set_status_message(format!("Deleted {} line(s)", matching_lines.len()));
        }
        "p" | "print" => {
            // Print all matching lines to status
            let mut output = Vec::new();
            for &line_idx in &matching_lines {
                if let Some(line_text) = editor.buffer().line_text(line_idx) {
                    output.push(format!("{}: {}", line_idx + 1, line_text));
                }
            }

            let result = if output.len() > 10 {
                // Limit output to first 10 lines
                let mut limited = output.into_iter().take(10).collect::<Vec<_>>();
                limited.push(format!("... and {} more lines", matching_lines.len() - 10));
                limited.join("\n")
            } else {
                output.join("\n")
            };

            editor.set_status_message(result);
        }
        "y" | "yank" => {
            // Yank all matching lines. Each yanked line must include its
            // trailing `\n` so the register stores linewise content the
            // same way `:y` and `Y` do — so re-add the terminator that
            // `line_text` strips by design.
            let mut yanked_text = String::new();
            for &line_idx in &matching_lines {
                if let Some(line) = editor.buffer().line_text(line_idx) {
                    yanked_text.push_str(&line);
                    yanked_text.push('\n');
                }
            }
            editor.yank_to_register(yanked_text);

            editor.set_status_message(format!("Yanked {} line(s)", matching_lines.len()));
        }
        _ if sub_command.starts_with("s/") => {
            // Parse substitute pattern once, outside the loop
            if let Some((sub_pattern, raw_replacement, flags)) =
                split_substitute_parts(&sub_command[1..])
            {
                let replacement = convert_vim_backrefs(&raw_replacement);
                let global = flags.contains('g');
                let ignore_case = flags.contains('i');

                use regex::RegexBuilder;
                if let Ok(sub_regex) = RegexBuilder::new(&sub_pattern)
                    .case_insensitive(ignore_case)
                    .build()
                {
                    // Run substitute on matching lines atomically (single undo)
                    let cursor_before = editor.cursor_position();

                    let ((), edits) = editor.buffer_mut().record(|buf| {
                        // Bottom-up for the same reason as handle_substitute_command:
                        // a `\r` in the replacement splits the line and would shift
                        // the remaining matched line numbers (OV-00244).
                        for &line_idx in matching_lines.iter().rev() {
                            if let Some(line_text) = buf.line_text(line_idx) {
                                let new_text = if global {
                                    sub_regex
                                        .replace_all(&line_text, replacement.as_str())
                                        .to_string()
                                } else {
                                    sub_regex
                                        .replace(&line_text, replacement.as_str())
                                        .to_string()
                                };

                                if new_text != line_text {
                                    let line_len = line_text.chars().count();
                                    buf.delete_range(
                                        line_idx,
                                        CharCol::ZERO,
                                        line_idx,
                                        CharCol(line_len),
                                    );
                                    buf.insert_text_at(line_idx, CharCol::ZERO, &new_text);
                                }
                            }
                        }
                    });

                    if !edits.is_empty() {
                        let cursor_after = editor.cursor_position();
                        editor.push_recorded_undo(edits, cursor_before, cursor_after);
                    }
                }
            }

            editor.set_status_message(format!("Substituted on {} line(s)", matching_lines.len()));
        }
        _ => {
            editor.set_status_message(format!("Unsupported global command: {}", sub_command));
        }
    }

    Ok(())
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
    fn substitute_parser_preserves_escaped_delimiters() {
        assert_eq!(
            split_substitute_parts(r"/a\/b/c\/d/g"),
            Some(("a/b".to_owned(), "c/d".to_owned(), "g".to_owned()))
        );
        assert_eq!(split_substitute_parts("/pattern"), None);
    }
}
