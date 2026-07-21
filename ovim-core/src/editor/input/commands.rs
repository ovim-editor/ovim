use crate::command_result::CommandResult;
use crate::edit::Edit;
use crate::editor::path_completion::extract_path_from_command;
use crate::editor::{CursorPos, Editor, Mode};
use crate::unicode::{CharCol, GraphemeCol};
use crate::{KeyCode, KeyEvent};
use anyhow::Result;

/// Handles input in Command mode
pub fn handle_command_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Char(ch) => {
            editor.append_to_command_line(ch);
            update_path_completion(editor);
        }
        KeyCode::Backspace => {
            if editor.command_line().is_empty() {
                editor.path_completion_mut().hide();
                editor.set_mode(Mode::Normal);
            } else {
                editor.backspace_command_line();
                update_path_completion(editor);
            }
        }
        KeyCode::Delete => {
            editor.delete_command_line_char();
            update_path_completion(editor);
        }
        KeyCode::Tab => {
            handle_tab_completion(editor, false);
        }
        KeyCode::BackTab => {
            handle_tab_completion(editor, true);
        }
        KeyCode::Up => {
            if editor.path_completion().is_visible() {
                editor.path_completion_mut().select_previous();
                accept_selected_into_command_line(editor);
            } else {
                editor.history_prev();
            }
        }
        KeyCode::Down => {
            if editor.path_completion().is_visible() {
                editor.path_completion_mut().select_next();
                accept_selected_into_command_line(editor);
            } else {
                editor.history_next();
            }
        }
        KeyCode::Left => {
            editor.move_command_cursor_left();
        }
        KeyCode::Right => {
            editor.move_command_cursor_right();
        }
        KeyCode::Home => {
            editor.move_command_cursor_home();
        }
        KeyCode::End => {
            editor.move_command_cursor_end();
        }
        KeyCode::Enter => {
            if editor.path_completion().is_visible() {
                if editor.path_completion().selected_is_dir() {
                    // Directory: accept into command line, refresh completions, don't execute.
                    if let Some(new_path) = editor.path_completion().accept() {
                        let cmd = editor.command_line().to_string();
                        if let Some(path_portion) = extract_path_from_command(&cmd) {
                            let prefix_len = cmd.len() - path_portion.len();
                            let new_cmd = format!("{}{}", &cmd[..prefix_len], new_path);
                            editor.set_command_line(&new_cmd);
                            let cwd = std::env::current_dir().unwrap_or_default();
                            editor.path_completion_mut().update(&new_path, &cwd);
                        }
                    }
                } else {
                    // File: accept into command line, then execute.
                    if let Some(new_path) = editor.path_completion().accept() {
                        let cmd = editor.command_line().to_string();
                        if let Some(path_portion) = extract_path_from_command(&cmd) {
                            let prefix_len = cmd.len() - path_portion.len();
                            let new_cmd = format!("{}{}", &cmd[..prefix_len], new_path);
                            editor.set_command_line(&new_cmd);
                        }
                    }
                    editor.path_completion_mut().hide();
                    editor.add_command_to_history();
                    execute_command(editor)?;
                    editor.clear_command_line();
                    if editor.mode() == Mode::Command {
                        editor.set_mode(Mode::Normal);
                    }
                }
            } else {
                editor.add_command_to_history();
                execute_command(editor)?;
                editor.clear_command_line();
                if editor.mode() == Mode::Command {
                    editor.set_mode(Mode::Normal);
                }
            }
        }
        KeyCode::Esc => {
            editor.path_completion_mut().hide();
            editor.clear_command_line();
            editor.set_mode(Mode::Normal);
        }
        _ => {}
    }
    Ok(())
}

fn expand_tilde_in_path(path: &str) -> std::path::PathBuf {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(path.trim_start_matches("~/"));
        }
    }
    std::path::PathBuf::from(path)
}

/// Updates the path completion popup based on current command line content.
fn update_path_completion(editor: &mut Editor) {
    let cmd = editor.command_line().to_string();
    if let Some(path_portion) = extract_path_from_command(&cmd) {
        let path_portion = path_portion.to_string();
        let cwd = std::env::current_dir().unwrap_or_default();
        editor.path_completion_mut().update(&path_portion, &cwd);
    } else {
        editor.path_completion_mut().hide();
    }
}

/// Known command names for Tab completion.
const COMMAND_NAMES: &[&str] = &[
    "bd",
    "bdelete",
    "buffer",
    "buffers",
    "cd",
    "cclose",
    "cfirst",
    "clast",
    "close",
    "cn",
    "cnext",
    "copen",
    "colorscheme",
    "cp",
    "cprev",
    "cq",
    "cquit",
    "delmarks",
    "e",
    "edit",
    "f",
    "file",
    "help",
    "hi",
    "highlight",
    "history",
    "lcd",
    "ls",
    "LspInstall",
    "LspManager",
    "map",
    "marks",
    "messages",
    "nmap",
    "nohlsearch",
    "noh",
    "norm",
    "normal",
    "noremap",
    "nmapclear",
    "only",
    "pwd",
    "q",
    "qa",
    "quit",
    "quitall",
    "reg",
    "registers",
    "reload",
    "saveas",
    "se",
    "set",
    "session",
    "sort",
    "source",
    "sp",
    "split",
    "tabe",
    "tabedit",
    "tabclose",
    "tabmove",
    "tabnext",
    "tabprev",
    "unmap",
    "unlet",
    "vmap",
    "vsp",
    "vsplit",
    "w",
    "wa",
    "wq",
    "wqa",
    "write",
    "writeall",
    "x",
    "xa",
];

/// State for command name Tab completion cycling.
struct CmdCompletion {
    matches: Vec<&'static str>,
    index: usize,
    original_prefix: String,
}

thread_local! {
    static CMD_COMPLETION: std::cell::RefCell<Option<CmdCompletion>> = const { std::cell::RefCell::new(None) };
}

/// Handles Tab (forward=false) or BackTab (backward=true) for path completion.
fn handle_tab_completion(editor: &mut Editor, backward: bool) {
    let cmd = editor.command_line().to_string();
    let trimmed = cmd.trim_start();

    // If the command line has no space, we're completing a command name.
    if !trimmed.contains(' ') {
        handle_command_name_completion(editor, trimmed, backward);
        return;
    }

    // Otherwise, fall through to path completion.
    if !editor.path_completion().is_visible() {
        // Try to trigger completion from current command line.
        update_path_completion(editor);
        if !editor.path_completion().is_visible() {
            return;
        }
        // First Tab after triggering: accept entry[0] without cycling.
    } else if editor.path_completion().tab_accepted() {
        // Already visible and Tab was used before — cycle selection.
        if backward {
            editor.path_completion_mut().select_previous();
        } else {
            editor.path_completion_mut().select_next();
        }
    }
    // else: popup was visible from typing but Tab hasn't accepted yet —
    // accept the current selection (entry[0]) without cycling.

    editor.path_completion_mut().set_tab_accepted();

    // Accept the selected entry: replace path portion in command line.
    let accepted = editor.path_completion().accept();
    if let Some(new_path) = accepted {
        let cmd = editor.command_line().to_string();
        if let Some(path_portion) = extract_path_from_command(&cmd) {
            let prefix_len = cmd.len() - path_portion.len();
            let new_cmd = format!("{}{}", &cmd[..prefix_len], new_path);
            editor.set_command_line(&new_cmd);

            // If we just completed a directory, refresh entries for its contents.
            if new_path.ends_with('/') {
                let cwd = std::env::current_dir().unwrap_or_default();
                editor.path_completion_mut().update(&new_path, &cwd);
            }
        }
    }
}

/// Handle Tab completion for command names (first word of command line).
fn handle_command_name_completion(editor: &mut Editor, prefix: &str, backward: bool) {
    CMD_COMPLETION.with(|cell| {
        let mut state = cell.borrow_mut();

        // Check if we're continuing a previous completion cycle
        let continuing = state.as_ref().is_some_and(|s| {
            s.original_prefix == prefix || {
                // Also continue if the command line matches a previous completion result
                s.matches.contains(&prefix)
            }
        });

        if continuing {
            let s = state.as_mut().unwrap();
            if backward {
                s.index = if s.index == 0 {
                    s.matches.len() // wraps to original prefix
                } else {
                    s.index - 1
                };
            } else {
                s.index += 1;
                if s.index > s.matches.len() {
                    s.index = 0;
                }
            }
            // index == matches.len() means show original prefix
            if s.index == s.matches.len() {
                editor.set_command_line(&s.original_prefix);
            } else {
                editor.set_command_line(s.matches[s.index]);
            }
        } else {
            // Build new completion list
            let matches: Vec<&'static str> = COMMAND_NAMES
                .iter()
                .filter(|cmd| cmd.starts_with(prefix))
                .copied()
                .collect();

            if matches.is_empty() {
                *state = None;
                return;
            }

            if matches.len() == 1 {
                // Unique match — complete it directly
                editor.set_command_line(matches[0]);
                *state = None;
                return;
            }

            // Multiple matches — start cycling from first
            let idx = if backward { matches.len() - 1 } else { 0 };
            editor.set_command_line(matches[idx]);
            *state = Some(CmdCompletion {
                matches,
                index: idx,
                original_prefix: prefix.to_string(),
            });
        }
    });
}

/// Accepts the currently selected path completion entry and updates the command line text.
fn accept_selected_into_command_line(editor: &mut Editor) {
    if let Some(new_path) = editor.path_completion().accept() {
        let cmd = editor.command_line().to_string();
        if let Some(path_portion) = extract_path_from_command(&cmd) {
            let prefix_len = cmd.len() - path_portion.len();
            let new_cmd = format!("{}{}", &cmd[..prefix_len], new_path);
            editor.set_command_line(&new_cmd);
        }
    }
}

/// Finds the byte index where the command name begins — the end of the leading
/// `[range]` prefix. The prefix may include mark addresses (`'a`, `'<`, `'>`),
/// whose following letter must NOT be mistaken for the command name. Skipping
/// the char after each `'` keeps `:'a,'bd` parsing as range `'a,'b` + command
/// `d` rather than range `'` + command `a,'bd`. Returns None when the whole
/// string is a bare range with no command.
fn command_name_start(command: &str) -> Option<usize> {
    let mut chars = command.char_indices();
    while let Some((idx, ch)) = chars.next() {
        if ch == '\'' {
            // Mark address: consume the mark-name char that follows.
            chars.next();
            continue;
        }
        if ch.is_alphabetic() {
            return Some(idx);
        }
    }
    None
}

/// Extracts the destination address of a `:copy`/`:t` or `:move`/`:m` command
/// from the command portion, accepting both the spaced (`t 3`) and the unspaced
/// (`t3`, `t0`, `t.`) Vim forms. Returns `None` when `cmd_part` isn't this
/// command — a trailing letter (`tabnew`, `marks`) is rejected so it doesn't get
/// mistaken for a copy/move to a mark.
fn parse_copy_move_dest<'a>(cmd_part: &'a str, long: &str, short: &str) -> Option<&'a str> {
    for word in [long, short] {
        if let Some(rest) = cmd_part.strip_prefix(word) {
            if rest.is_empty() {
                return Some("");
            }
            let first = rest.chars().next().unwrap();
            if first.is_whitespace()
                || first.is_ascii_digit()
                || matches!(first, '.' | '$' | '\'' | '+' | '-' | '/' | '?')
            {
                return Some(rest.trim());
            }
        }
    }
    None
}

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
fn handle_substitute_command(editor: &mut Editor, range_str: &str, cmd_part: &str) -> Result<()> {
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
fn handle_global_command(
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
            let parts: Vec<&str> = sub_command.splitn(4, '/').collect();
            if parts.len() >= 3 {
                let sub_pattern = parts[1];
                let replacement = convert_vim_backrefs(parts[2]);
                let flags = if parts.len() >= 4 { parts[3] } else { "" };

                let global = flags.contains('g');
                let ignore_case = flags.contains('i');

                use regex::RegexBuilder;
                if let Ok(sub_regex) = RegexBuilder::new(sub_pattern)
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

/// Parses an Ex command range (e.g., "1,5", "%", ".", "'a,'b")
/// Returns (start_line, end_line) as 0-indexed, inclusive
pub fn parse_range(editor: &Editor, range_str: &str) -> Option<(usize, usize)> {
    parse_range_internal(editor, range_str).ok()
}

#[derive(Debug, Clone, Copy)]
enum ParseRangeError {
    MarkNotSet,
    InvalidRange,
}

fn parse_range_internal(
    editor: &Editor,
    range_str: &str,
) -> Result<(usize, usize), ParseRangeError> {
    let range_str = range_str.trim();

    if range_str.is_empty() {
        // No range - current line only
        let cursor_line = editor.buffer().cursor().line();
        return Ok((cursor_line, cursor_line));
    }

    // Handle % (all lines)
    if range_str == "%" {
        if editor.buffer().line_count() == 0 {
            return Err(ParseRangeError::InvalidRange);
        }
        return Ok((0, editor.buffer().line_count().saturating_sub(1)));
    }

    // Handle visual selection markers
    if range_str == "'<,'>" || range_str.contains("'<") {
        if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
            return Ok((start_line, end_line));
        }
        return Err(ParseRangeError::InvalidRange);
    }

    // Handle ranges with comma (e.g., "1,5", ".,$ ", "'a,'b")
    if let Some(comma_idx) = range_str.find(',') {
        let start_part = range_str[..comma_idx].trim();
        let end_part = range_str[comma_idx + 1..].trim();

        let start = parse_range_endpoint_internal(editor, start_part)?;
        let end = parse_range_endpoint_internal(editor, end_part)?;

        return Ok((start.min(end), start.max(end)));
    }

    // Single endpoint
    let line = parse_range_endpoint_internal(editor, range_str)?;
    Ok((line, line))
}

fn parse_range_with_status(
    editor: &mut Editor,
    range_str: &str,
    invalid_status: Option<&str>,
) -> Option<(usize, usize)> {
    match parse_range_internal(editor, range_str) {
        Ok(range) => Some(range),
        Err(ParseRangeError::MarkNotSet) => {
            editor.set_status_message("E20: Mark not set".to_string());
            None
        }
        Err(ParseRangeError::InvalidRange) => {
            if let Some(status) = invalid_status {
                editor.set_status_message(status.to_string());
            }
            None
        }
    }
}

fn parse_range_endpoint_with_status(
    editor: &mut Editor,
    endpoint: &str,
    invalid_status: Option<&str>,
) -> Option<usize> {
    match parse_range_endpoint_internal(editor, endpoint) {
        Ok(line) => Some(line),
        Err(ParseRangeError::MarkNotSet) => {
            editor.set_status_message("E20: Mark not set".to_string());
            None
        }
        Err(ParseRangeError::InvalidRange) => {
            if let Some(status) = invalid_status {
                editor.set_status_message(status.to_string());
            }
            None
        }
    }
}

fn parse_range_endpoint_internal(
    editor: &Editor,
    endpoint: &str,
) -> Result<usize, ParseRangeError> {
    let endpoint = endpoint.trim();
    let cursor_line = editor.buffer().cursor().line();
    let last_line = editor.buffer().line_count().saturating_sub(1);

    // . = current line
    if endpoint == "." {
        return Ok(cursor_line);
    }

    // $ = last line
    if endpoint == "$" {
        return Ok(last_line);
    }

    // 'x = mark
    if endpoint.starts_with('\'') && endpoint.len() == 2 {
        let mark_char = endpoint
            .chars()
            .nth(1)
            .ok_or(ParseRangeError::InvalidRange)?;
        if let Some(mark) = editor.nav.marks.get_mark(mark_char) {
            return Ok(mark.line);
        }
        return Err(ParseRangeError::MarkNotSet);
    }

    // +N or -N (relative to current line)
    if let Some(rest) = endpoint.strip_prefix('+') {
        let offset: usize = rest.parse().map_err(|_| ParseRangeError::InvalidRange)?;
        return Ok((cursor_line + offset).min(last_line));
    }
    if let Some(rest) = endpoint.strip_prefix('-') {
        let offset: usize = rest.parse().map_err(|_| ParseRangeError::InvalidRange)?;
        return Ok(cursor_line.saturating_sub(offset));
    }

    // Plain number (1-indexed in Vim, convert to 0-indexed)
    if let Ok(line_num) = endpoint.parse::<usize>() {
        if line_num == 0 {
            return Ok(0);
        }
        // Convert to 0-indexed and clamp to valid range
        return Ok((line_num.saturating_sub(1)).min(last_line));
    }

    Err(ParseRangeError::InvalidRange)
}

/// Handles shell command execution (:! or :.! or :%!)
/// - `:!cmd` - runs command and displays output
/// - `:.!cmd` - replaces current line with command output
/// - `:%!cmd` - pipes entire buffer through command
/// - `:range!cmd` - pipes specified range through command
fn handle_shell_command(editor: &mut Editor, range_str: &str, shell_cmd: &str) -> Result<()> {
    use super::shell_expansion::expand_shell_command;
    use std::process::{Command, Stdio};

    // Expand % and # in the shell command
    let current_file = editor.buffer().file_path().unwrap_or("").to_string();
    let alternate_file = editor.registers().get(Some('#'));
    let shell_cmd = expand_shell_command(shell_cmd, &current_file, &alternate_file);

    // Determine the shell to use
    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    let shell_arg = if cfg!(windows) { "/C" } else { "-c" };

    // Check if we're piping buffer content through the command
    let is_filter = !range_str.is_empty();

    if is_filter {
        // Parse the range
        let (start_line, end_line) =
            match parse_range_with_status(editor, range_str, Some("Invalid range")) {
                Some(range) => range,
                None => return Ok(()),
            };

        // Get the text from the range. Re-add the line terminators
        // `line_text` strips so the spawned filter sees the same input it
        // would have seen via `cat`.
        let mut input_text = String::new();
        for line_idx in start_line..=end_line {
            if let Some(line) = editor.buffer().line_text(line_idx) {
                input_text.push_str(&line);
                input_text.push('\n');
            }
        }

        // Run command with input piped
        let output = Command::new(shell)
            .arg(shell_arg)
            .arg(shell_cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(ref mut stdin) = child.stdin {
                    stdin.write_all(input_text.as_bytes())?;
                }
                child.wait_with_output()
            });

        match output {
            Ok(output) => {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let output_text = stdout;

                    // Insert the command output (with trailing newline if needed)
                    let insert_text = if output_text.is_empty() {
                        String::new()
                    } else if output_text.ends_with('\n') {
                        output_text.to_string()
                    } else {
                        format!("{}\n", output_text)
                    };

                    let cursor_before = editor.cursor_position();
                    let ((), edits) = editor.buffer_mut().record(|buf| {
                        buf.delete_range(start_line, CharCol::ZERO, end_line + 1, CharCol::ZERO);
                        if !insert_text.is_empty() {
                            buf.insert_text_at(start_line, CharCol::ZERO, &insert_text);
                        }
                        buf.cursor_mut().set_position(start_line, GraphemeCol::ZERO);
                    });
                    if !edits.is_empty() {
                        let cursor_after = CursorPos::new(start_line, GraphemeCol::ZERO);
                        editor.push_recorded_undo(edits, cursor_before, cursor_after);
                    }

                    let line_count = insert_text.lines().count();
                    editor.set_status_message(format!("{} lines filtered", line_count));
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    editor.set_status_message(format!("Command failed: {}", stderr.trim()));
                }
            }
            Err(e) => {
                editor.set_status_message(format!("Failed to run command: {}", e));
            }
        }
    } else {
        // Queue for the event loop to execute with full terminal access.
        // The TUI will leave alternate screen, run the command with inherited I/O,
        // show a "Press ENTER" prompt, then restore the editor.
        editor.build.last_shell_command = Some(shell_cmd.to_string());
        editor.build.pending_shell_command = Some(crate::editor::PendingShellCommand {
            command: shell_cmd.to_string(),
        });
    }

    Ok(())
}

/// Handles :r !cmd - read output from shell command and insert below cursor
/// - `:r !cmd` - insert output below current line
/// - `:0r !cmd` - insert at start of buffer
/// - `:'<,'>r !cmd` - insert after selection
fn handle_read_shell_command(editor: &mut Editor, range_str: &str, shell_cmd: &str) -> Result<()> {
    use super::shell_expansion::expand_shell_command;
    use std::process::{Command, Stdio};

    // Expand % and # in the shell command
    let current_file = editor.buffer().file_path().unwrap_or("").to_string();
    let alternate_file = editor.registers().get(Some('#'));
    let shell_cmd = expand_shell_command(shell_cmd, &current_file, &alternate_file);

    // Determine the shell to use
    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    let shell_arg = if cfg!(windows) { "/C" } else { "-c" };

    // Run the command
    let output = Command::new(shell)
        .arg(shell_arg)
        .arg(&shell_cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match output {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let output_text = stdout.to_string();

                if output_text.is_empty() {
                    editor.set_status_message("Command produced no output".to_string());
                    return Ok(());
                }

                // Determine insertion point
                let insert_line = if range_str.is_empty() {
                    // Insert after current line
                    editor.buffer().cursor().line() + 1
                } else if let Some((_, end_line)) =
                    parse_range_with_status(editor, range_str, Some("Invalid range"))
                {
                    // Insert after the range end line
                    end_line + 1
                } else {
                    return Ok(());
                };

                // Record change for undo
                let cursor_before = CursorPos::new(
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );

                // Calculate insertion point
                let insert_char = if insert_line < editor.buffer().line_count() {
                    editor.buffer().rope().line_to_char(insert_line)
                } else {
                    editor.buffer().rope().len_chars()
                };

                // Ensure text ends with newline
                let text = if output_text.ends_with('\n') {
                    output_text
                } else {
                    format!("{}\n", output_text)
                };

                // Add newline prefix if inserting at end of file
                let text = if insert_line >= editor.buffer().line_count() && insert_char > 0 {
                    format!("\n{}", text)
                } else {
                    text
                };

                // Insert the text
                editor.buffer_mut().rope_mut().insert(insert_char, &text);

                // Position cursor at start of inserted text
                let cursor_after = CursorPos::new(insert_line, GraphemeCol::ZERO);
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(insert_line, GraphemeCol::ZERO);
                editor.push_recorded_undo(
                    vec![Edit::Insert {
                        offset: insert_char,
                        text: text.clone(),
                    }],
                    cursor_before,
                    cursor_after,
                );

                let line_count = text.lines().count();
                editor.set_status_message(format!(
                    "{} line{} inserted",
                    line_count,
                    if line_count == 1 { "" } else { "s" }
                ));
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                editor.set_status_message(format!("Command failed: {}", stderr.trim()));
            }
        }
        Err(e) => {
            editor.set_status_message(format!("Failed to run command: {}", e));
        }
    }

    Ok(())
}

/// Handles :w !cmd - write buffer/range to command stdin
/// - `:w !cmd` - send entire buffer to command stdin
/// - `:'<,'>w !cmd` - send selection to command stdin
fn handle_write_to_command(editor: &mut Editor, range_str: &str, shell_cmd: &str) -> Result<()> {
    use super::shell_expansion::expand_shell_command;
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Expand % and # in the shell command
    let current_file = editor.buffer().file_path().unwrap_or("").to_string();
    let alternate_file = editor.registers().get(Some('#'));
    let shell_cmd = expand_shell_command(shell_cmd, &current_file, &alternate_file);

    // Determine the shell to use
    let shell = if cfg!(windows) { "cmd" } else { "sh" };
    let shell_arg = if cfg!(windows) { "/C" } else { "-c" };

    // Get the content to write
    let content = if range_str.is_empty() {
        // Write entire buffer
        editor.buffer().rope().to_string()
    } else if let Some((start_line, end_line)) =
        parse_range_with_status(editor, range_str, Some("Invalid range"))
    {
        // Write specified range
        let mut text = String::new();
        for line_idx in start_line..=end_line {
            if let Some(line) = editor.buffer().line_text(line_idx) {
                text.push_str(&line);
            }
        }
        text
    } else {
        return Ok(());
    };

    // Run the command with content piped to stdin
    let mut child = match Command::new(shell)
        .arg(shell_arg)
        .arg(&shell_cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            editor.set_status_message(format!("Failed to run command: {}", e));
            return Ok(());
        }
    };

    // Write content to stdin
    if let Some(ref mut stdin) = child.stdin {
        if let Err(e) = stdin.write_all(content.as_bytes()) {
            editor.set_status_message(format!("Failed to write to command: {}", e));
            return Ok(());
        }
    }

    // Wait for command to complete
    match child.wait_with_output() {
        Ok(output) => {
            if output.status.success() {
                let line_count = content.lines().count();
                let stdout = String::from_utf8_lossy(&output.stdout);
                let msg = if stdout.trim().is_empty() {
                    format!(
                        "{} line{} written",
                        line_count,
                        if line_count == 1 { "" } else { "s" }
                    )
                } else {
                    // Show command output if any
                    let trimmed = stdout.trim();
                    if trimmed.len() > 100 {
                        format!(
                            "{} lines written: {}...",
                            line_count,
                            crate::unicode::truncate_bytes(trimmed, 100)
                        )
                    } else {
                        format!("{} lines written: {}", line_count, trimmed)
                    }
                };
                editor.set_status_message(msg);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                editor.set_status_message(format!("Command failed: {}", stderr.trim()));
            }
        }
        Err(e) => {
            editor.set_status_message(format!("Failed to wait for command: {}", e));
        }
    }

    Ok(())
}

/// Executes a command string directly (used for API/Lua commands)
pub fn execute_command_string(editor: &mut Editor, command: &str) -> Result<()> {
    execute_command_impl(editor, command)
}

/// Executes a command string on behalf of the headless API / CLI, returning a
/// structured [`CommandResult`].
///
/// The headless `exec` path historically called [`crate::commands::execute_command`]
/// directly, which only knows the "standard" ex-commands (`:w`, `:q`, `:set`, …).
/// Substitute (`:s`), global (`:g`/`:v`), ranges, and `:d`/`:y` live in the
/// interactive command handler and were therefore unreachable headlessly — an
/// `exec ':%s/a/b/'` returned "Not an editor command" while the same keys typed
/// interactively worked. Routing through the same dispatcher the interactive
/// command line uses keeps the two paths in parity.
pub fn execute_command_string_api(editor: &mut Editor, command: &str) -> CommandResult {
    use crate::command_result::{err, ok, ok_silent};

    let command = command.trim();

    // Standard commands return a structured result we forward verbatim — this
    // preserves messages like line counts and errors like "No write since last
    // change". Only when the standard dispatcher reports the command as unknown
    // do we fall through to the richer interactive handler.
    let result = crate::commands::execute_command(editor, command);
    let is_unknown = matches!(
        &result,
        CommandResult::Error(e) if e.error.contains("Not an editor command")
    );
    if !is_unknown {
        return result;
    }

    // The standard dispatcher performs no mutation when it doesn't recognize a
    // command, so it is safe to re-run the full interactive handler (which
    // re-checks the standard dispatcher and then handles substitute / global /
    // range / etc.). That handler reports outcomes on the status line rather
    // than returning them, so clear the line first and read it back afterwards.
    editor.set_status_message(String::new());
    match execute_command_string(editor, command) {
        Ok(()) => {
            let status = editor.status_message().trim().to_string();
            if status.is_empty() {
                ok_silent()
            } else if is_vim_error_status(&status) {
                err(status)
            } else {
                ok(status)
            }
        }
        Err(e) => err(e.to_string()),
    }
}

/// Vim surfaces command errors on the status line using the `E<number>:`
/// convention (`E146`, `E20`, `E486`, …). Map those back to an API error even
/// though the editor itself treats them as ordinary status messages.
fn is_vim_error_status(status: &str) -> bool {
    matches!(
        status.strip_prefix('E'),
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit())
    )
}

/// Executes a command from the command line
fn execute_command(editor: &mut Editor) -> Result<()> {
    let command = editor.command_line().trim().to_string();
    execute_command_impl(editor, &command)
}

/// Internal command execution implementation
fn execute_command_impl(editor: &mut Editor, command: &str) -> Result<()> {
    let command = command.trim();

    // Handle command chaining with |
    // BUG FIX: Don't split on | for substitute, global, or vglobal commands
    // because | can appear in patterns like :s/foo|bar/baz/
    if command.contains('|') && !is_command_with_pattern(command) {
        // Simple split for non-substitute commands
        for part in command.split('|') {
            let part = part.trim();
            if !part.is_empty() {
                execute_command_single(editor, part)?;
            }
        }
        return Ok(());
    }

    execute_command_single(editor, command)
}

/// Helper: Check if command contains patterns that may include | characters.
/// Returns true for substitute (:s), global (:g), and vglobal (:v) commands.
///
/// We strip leading range characters (digits, commas, %, ., $, ', <, >) then
/// check if the command part starts with s/, g/, g!/, or v/.  This avoids
/// false positives on commands like `:e files/foo | set number`.
fn is_command_with_pattern(command: &str) -> bool {
    let trimmed = command.trim();
    // Skip past range prefix: digits, commas, %, ., $, ', <, >, +, -, spaces
    let cmd = trimmed.trim_start_matches(|c: char| c.is_ascii_digit() || ",%.$ '<>+-".contains(c));
    cmd.starts_with("s/")
        || cmd.starts_with("g/")
        || cmd.starts_with("g!/")
        || cmd.starts_with("v/")
}

/// Execute a single command (no chaining)
fn execute_command_single(editor: &mut Editor, command: &str) -> Result<()> {
    // Update the : register with the command
    editor.registers_mut().set_last_command(command.to_string());

    // Intercept plain :!cmd in TUI mode — queue for the event loop so it
    // runs with full terminal access (outside alternate screen).
    // Filter commands (:range!cmd) and :r/:w !cmd are NOT intercepted here;
    // they're handled below by the standard command flow.
    if let Some(shell_cmd) = command.strip_prefix('!') {
        use super::shell_expansion::expand_shell_command;
        let shell_cmd = shell_cmd.trim();
        let cmd = if shell_cmd.is_empty() {
            // Bare :! — repeat last
            match editor.build.last_shell_command.clone() {
                Some(last) => last,
                None => {
                    editor.set_status_message("No previous shell command".to_string());
                    return Ok(());
                }
            }
        } else {
            let current_file = editor.buffer().file_path().unwrap_or("").to_string();
            let alternate_file = editor.registers().get(Some('#'));
            expand_shell_command(shell_cmd, &current_file, &alternate_file)
        };
        editor.build.last_shell_command = Some(cmd.clone());
        editor.build.pending_shell_command =
            Some(crate::editor::PendingShellCommand { command: cmd });
        return Ok(());
    }

    // First, try to delegate to the top-level commands module which has all the standard commands
    let response = crate::commands::execute_command(editor, command);
    match response {
        CommandResult::Success(success_resp) => {
            // Command executed successfully
            if let Some(msg) = success_resp.message {
                // Multi-line messages go to hover popup, single-line to status bar
                let msg = msg.into_owned();
                if msg.contains('\n') {
                    editor.set_hover_info(msg);
                } else {
                    editor.set_status_message(msg);
                }
            }
            return Ok(());
        }
        CommandResult::Error(err_resp) => {
            // Check if it's an "unknown command" error
            if err_resp.error.contains("Not an editor command") {
                // Fall through to custom input-specific command handling below
            } else {
                // It's a real error from a known command
                editor.set_status_message(err_resp.error);
                return Ok(());
            }
        }
    }

    // If we reach here, it's an unknown command - try custom input-specific handling

    // Handle :w !cmd (write to command stdin) - must be checked before range parsing
    // Note: :w! is force write (handled above), :w !cmd (with space) writes to command stdin
    if let Some(write_cmd) = command
        .strip_prefix("w !")
        .or_else(|| command.strip_prefix("write !"))
    {
        return handle_write_to_command(editor, "", write_cmd.trim());
    }

    // Handle range + w !cmd (e.g., :'<,'>w !pbcopy)
    // This requires checking if the command ends with "w !..." pattern after a range
    if command.contains("w !") || command.contains("write !") {
        // Find where 'w !' or 'write !' starts
        if let Some(pos) = command.find("w !").or_else(|| command.find("write !")) {
            let range_str = &command[..pos];
            let shell_cmd = if command[pos..].starts_with("write !") {
                &command[pos + 7..]
            } else {
                &command[pos + 3..]
            };
            return handle_write_to_command(editor, range_str.trim(), shell_cmd.trim());
        }
    }

    // Handle :r !cmd (read from command) - must be checked before range parsing
    // Note: This is different from file reading :r filename
    if let Some(read_cmd) = command
        .strip_prefix("r !")
        .or_else(|| command.strip_prefix("read !"))
    {
        return handle_read_shell_command(editor, "", read_cmd.trim());
    }

    // Handle range + r !cmd (e.g., :0r !cmd)
    if command.contains("r !") || command.contains("read !") {
        if let Some(pos) = command.find("r !").or_else(|| command.find("read !")) {
            let range_str = &command[..pos];
            let shell_cmd = if command[pos..].starts_with("read !") {
                &command[pos + 6..]
            } else {
                &command[pos + 3..]
            };
            return handle_read_shell_command(editor, range_str.trim(), shell_cmd.trim());
        }
    }

    // First, try to parse range from command
    // Format: :[range]command
    //
    // BUG FIX: '!' is ambiguous:
    // - Shell: :[range]!cmd  (the '!' starts the command)
    // - Global invert: :g!/pat/cmd (the '!' is part of the command name)
    //
    // Treat '!' as the command separator only when it appears *before* the
    // first alphabetic command character (i.e. it's the command itself).
    let cmd_start = command_name_start(command);
    let bang_split = command.find('!').and_then(|exclaim_idx| {
        let is_shell_separator = match cmd_start {
            None => true,
            Some(alpha_idx) => exclaim_idx < alpha_idx,
        };
        if is_shell_separator {
            Some(exclaim_idx)
        } else {
            None
        }
    });

    let (range_str, cmd_part) = if let Some(exclaim_idx) = bang_split {
        (&command[..exclaim_idx], &command[exclaim_idx..])
    } else if let Some(cmd_start) = cmd_start {
        (&command[..cmd_start], &command[cmd_start..])
    } else {
        (command, "")
    };

    // Handle goto line (just a number or range without command)
    if cmd_part.is_empty() && !range_str.is_empty() {
        if let Some((start_line, _end_line)) = parse_range_with_status(editor, range_str, None) {
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(start_line, GraphemeCol::ZERO);
            return Ok(());
        }
    }

    // Handle ranged delete command (:d or :delete)
    if cmd_part == "d" || cmd_part == "delete" {
        if let Some((start_line, end_line)) = parse_range_with_status(editor, range_str, None) {
            let cursor_before = editor.cursor_position();
            let (deleted_text, edits) = editor.buffer_mut().record(|buf| {
                buf.delete_range(start_line, CharCol::ZERO, end_line + 1, CharCol::ZERO)
            });

            // Store in register (use delete, which updates " and numbered regs but not 0)
            editor.delete_to_register(deleted_text.clone());

            // Position cursor at start of deleted range
            let new_cursor_line = start_line.min(editor.buffer().line_count().saturating_sub(1));
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(new_cursor_line, GraphemeCol::ZERO);

            if !edits.is_empty() {
                let cursor_after = CursorPos::new(new_cursor_line, GraphemeCol::ZERO);
                editor.push_recorded_undo(edits, cursor_before, cursor_after);
            }

            return Ok(());
        }
    }

    // Handle ranged yank command (:y or :yank)
    if cmd_part == "y" || cmd_part == "yank" {
        if let Some((start_line, end_line)) = parse_range_with_status(editor, range_str, None) {
            // Re-add the terminator `line_text` strips so register content
            // is linewise, matching `Y`/`yy`.
            let mut yanked_text = String::new();
            for line_idx in start_line..=end_line {
                if let Some(line) = editor.buffer().line_text(line_idx) {
                    yanked_text.push_str(&line);
                    yanked_text.push('\n');
                }
            }

            // Store in register (use yank, which updates " and 0)
            editor.yank_to_register(yanked_text);

            return Ok(());
        }
    }

    // Handle :sort command (sorts lines in range)
    if cmd_part == "sort" || cmd_part.starts_with("sort ") {
        if let Some((start_line, end_line)) = parse_range_with_status(editor, range_str, None) {
            let reverse = cmd_part.contains('!') || cmd_part.contains(" r");
            let numeric = cmd_part.contains(" n");
            let unique = cmd_part.contains(" u");
            let ignore_case = cmd_part.contains(" i");

            // Collect lines (terminator-stripped — we'll re-add `\n` when
            // we serialize back to a single block below).
            let mut lines: Vec<String> = (start_line..=end_line)
                .filter_map(|idx| editor.buffer().line_text(idx).map(|l| l.into_owned()))
                .collect();

            // Bug 3 fix: Use stable sort (Vim's :sort is stable)
            // sort_by is stable, but sort() uses unstable sort internally
            if numeric {
                // Sort by leading number
                lines.sort_by(|a, b| {
                    let num_a: i64 = a
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let num_b: i64 = b
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    num_a.cmp(&num_b)
                });
            } else if ignore_case {
                lines.sort_by_key(|a| a.to_lowercase());
            } else {
                // Use stable sort for consistency with Vim
                lines.sort();
            }

            if reverse {
                lines.reverse();
            }

            if unique {
                lines.dedup();
            }

            // Replace the range with sorted lines
            let cursor_before = CursorPos::new(
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );

            // Get the char positions for the range
            let start_char = editor.buffer().rope().line_to_char(start_line);
            let end_char = if end_line + 1 < editor.buffer().line_count() {
                editor.buffer().rope().line_to_char(end_line + 1)
            } else {
                editor.buffer().rope().len_chars()
            };

            // Store original text for undo
            let original_text = editor
                .buffer()
                .rope()
                .slice(start_char..end_char)
                .to_string();

            // Remove old lines
            editor.buffer_mut().rope_mut().remove(start_char..end_char);

            // Insert sorted lines, re-adding the terminators we stripped.
            let mut new_text = String::new();
            for line in &lines {
                new_text.push_str(line);
                new_text.push('\n');
            }
            editor.buffer_mut().rope_mut().insert(start_char, &new_text);

            let mut edits = Vec::new();
            if !original_text.is_empty() {
                edits.push(Edit::Delete {
                    offset: start_char,
                    text: original_text,
                });
            }
            if !new_text.is_empty() {
                edits.push(Edit::Insert {
                    offset: start_char,
                    text: new_text.clone(),
                });
            }
            if !edits.is_empty() {
                editor.push_recorded_undo(edits, cursor_before, cursor_before);
            }

            let sorted_count = lines.len();
            editor.set_status_message(format!("{} lines sorted", sorted_count));
            return Ok(());
        }
    }

    // Handle :copy or :t command (copy lines to destination)
    // Format: :[range]copy {address} or :[range]t {address}
    // (helper `parse_copy_move_dest` accepts both the spaced and unspaced forms)
    if let Some(dest_str) = parse_copy_move_dest(cmd_part, "copy", "t") {
        if dest_str.is_empty() {
            editor.set_status_message("E488: Trailing characters".to_string());
            return Ok(());
        }

        if let Some((start_line, end_line)) = parse_range_with_status(editor, range_str, None) {
            // Parse destination address
            if let Some(dest_line) =
                parse_range_endpoint_with_status(editor, dest_str, Some("E14: Invalid address"))
            {
                // Collect lines to copy. `line_text` strips terminators by
                // design — re-add `\n` so the inserted block keeps its
                // line breaks.
                let mut text_to_insert = String::new();
                let mut lines_copied = 0usize;
                for idx in start_line..=end_line {
                    if let Some(line) = editor.buffer().line_text(idx) {
                        text_to_insert.push_str(&line);
                        text_to_insert.push('\n');
                        lines_copied += 1;
                    }
                }

                // Insert after destination line. Address 0 is Vim's "before the
                // first line" — insert at buffer index 0 rather than after line 1
                // (both "0" and "1" parse to 0-based line 0, so disambiguate here).
                let insert_line = if dest_str == "0" { 0 } else { dest_line + 1 };
                let cursor_before = CursorPos::new(
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );

                let insert_char = if insert_line < editor.buffer().line_count() {
                    editor.buffer().rope().line_to_char(insert_line)
                } else {
                    editor.buffer().rope().len_chars()
                };

                // Add newline if we're at end of file
                let text =
                    if insert_line >= editor.buffer().line_count() && !text_to_insert.is_empty() {
                        format!("\n{}", text_to_insert)
                    } else {
                        text_to_insert.clone()
                    };

                editor.buffer_mut().rope_mut().insert(insert_char, &text);

                // Move cursor to first copied line
                let cursor_after = CursorPos::new(insert_line, GraphemeCol::ZERO);
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(insert_line, GraphemeCol::ZERO);
                editor.push_recorded_undo(
                    vec![Edit::Insert {
                        offset: insert_char,
                        text: text.clone(),
                    }],
                    cursor_before,
                    cursor_after,
                );

                let count = lines_copied;
                editor.set_status_message(format!(
                    "{} line{} copied",
                    count,
                    if count == 1 { "" } else { "s" }
                ));
                return Ok(());
            }
        }
    }

    // Handle :move or :m command (move lines to destination)
    // Format: :[range]move {address} or :[range]m {address}
    if let Some(dest_str) = parse_copy_move_dest(cmd_part, "move", "m") {
        if dest_str.is_empty() {
            editor.set_status_message("E488: Trailing characters".to_string());
            return Ok(());
        }

        if let Some((start_line, end_line)) = parse_range_with_status(editor, range_str, None) {
            // Parse destination address
            if let Some(mut dest_line) =
                parse_range_endpoint_with_status(editor, dest_str, Some("E14: Invalid address"))
            {
                // Bug 2 fix: Check for invalid moves (moving to within the range)
                // Should be <= end_line to prevent moving into self
                if dest_line >= start_line && dest_line <= end_line {
                    editor.set_status_message("E134: Move lines into themselves".to_string());
                    return Ok(());
                }

                let cursor_before = CursorPos::new(
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );

                // Collect lines to move. `line_text` strips terminators by
                // design — re-add `\n` so the moved block keeps its line
                // breaks.
                let mut text_to_move = String::new();
                let mut line_count = 0usize;
                for idx in start_line..=end_line {
                    if let Some(line) = editor.buffer().line_text(idx) {
                        text_to_move.push_str(&line);
                        text_to_move.push('\n');
                        line_count += 1;
                    }
                }

                // Delete the source lines
                let start_char = editor.buffer().rope().line_to_char(start_line);
                let end_char = if end_line + 1 < editor.buffer().line_count() {
                    editor.buffer().rope().line_to_char(end_line + 1)
                } else {
                    editor.buffer().rope().len_chars()
                };
                editor.buffer_mut().rope_mut().remove(start_char..end_char);

                // Adjust destination if it was after the deleted lines
                if dest_line > end_line {
                    dest_line = dest_line.saturating_sub(line_count);
                }

                // Insert after destination line; address 0 means the top of the
                // file (Vim), so insert at index 0 instead of after line 1.
                let insert_line = if dest_str == "0" { 0 } else { dest_line + 1 };
                let insert_char = if insert_line < editor.buffer().line_count() {
                    editor.buffer().rope().line_to_char(insert_line)
                } else {
                    editor.buffer().rope().len_chars()
                };

                // Add newline if we're at end of file
                let text =
                    if insert_line >= editor.buffer().line_count() && !text_to_move.is_empty() {
                        format!("\n{}", text_to_move)
                    } else {
                        text_to_move.clone()
                    };

                editor.buffer_mut().rope_mut().insert(insert_char, &text);

                // Move cursor to first moved line
                let cursor_after = CursorPos::new(insert_line, GraphemeCol::ZERO);
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(insert_line, GraphemeCol::ZERO);
                let mut edits = Vec::new();
                if !text_to_move.is_empty() {
                    edits.push(Edit::Delete {
                        offset: start_char,
                        text: text_to_move,
                    });
                }
                if !text.is_empty() {
                    edits.push(Edit::Insert {
                        offset: insert_char,
                        text: text.clone(),
                    });
                }
                if !edits.is_empty() {
                    editor.push_recorded_undo(edits, cursor_before, cursor_after);
                }

                editor.set_status_message(format!(
                    "{} line{} moved",
                    line_count,
                    if line_count == 1 { "" } else { "s" }
                ));
                return Ok(());
            }
        }
    }

    // Handle commands with arguments
    if command.starts_with("w ") || command.starts_with("write ") {
        // :w <filename> or :write <filename> - save as
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.len() >= 2 {
            let old_path = editor.buffer().file_path().map(|s| s.to_string());
            let filename = parts[1..].join(" ");
            editor.buffer_mut().save_as(&filename)?;
            let new_path = editor.buffer().file_path().map(|s| s.to_string());
            editor.handle_file_path_transition_after_save(old_path, new_path);
            if editor.options.blame {
                editor.buffer_mut().load_git_blame();
            }
            editor.mark_saved();
            editor.mark_buffer_saved();
        }
        return Ok(());
    }

    // Handle :global and :vglobal commands (:g/pattern/command, :v/pattern/command),
    // including when prefixed by a range (:rangeg/...).
    if cmd_part.starts_with("g/") || cmd_part.starts_with("g!/") || cmd_part.starts_with("v/") {
        let range = if range_str.trim().is_empty() {
            // Vim's :global defaults to the entire buffer, not the current line.
            None
        } else {
            match parse_range_with_status(editor, range_str, Some("E14: Invalid address")) {
                Some(r) => Some(r),
                None => return Ok(()),
            }
        };

        handle_global_command(editor, cmd_part, range)?;
        return Ok(());
    }

    // Handle substitute command (:s, :%s, :'<,'>s)
    // Only treat as substitute when the command part starts with `s/`.
    if cmd_part.starts_with("s/") {
        handle_substitute_command(editor, range_str, cmd_part)?;
        return Ok(());
    }

    // Handle range-prefixed shell commands (:.!cmd, :%!cmd, :1,5!cmd)
    // Plain :!cmd is intercepted at the top of execute_command_single.
    if let Some(shell_cmd) = cmd_part.strip_prefix('!') {
        let shell_cmd = shell_cmd.trim();
        if !shell_cmd.is_empty() {
            handle_shell_command(editor, range_str, shell_cmd)?;
            return Ok(());
        }
    }

    // Only handle custom input-specific commands here.
    // Standard commands are now delegated to the top-level commands module.
    {
        // Check if it's a :r or :read command
        if let Some(target) = command
            .strip_prefix("r ")
            .or_else(|| command.strip_prefix("read "))
        {
            let target = target.trim();
            if !target.is_empty() {
                if let Some(shell_cmd) = target.strip_prefix('!') {
                    // :r !cmd - read output from shell command
                    handle_read_shell_command(editor, range_str, shell_cmd.trim())?;
                } else {
                    // :r filename - read file contents
                    let expanded_target = expand_tilde_in_path(target);
                    let display_target = expanded_target.to_string_lossy().to_string();
                    match std::fs::read_to_string(&expanded_target) {
                        Ok(contents) => {
                            // Insert contents at current cursor position
                            let cursor = editor.buffer().cursor();
                            let line = cursor.line() + 1; // Insert after current line
                            editor
                                .buffer_mut()
                                .insert_text_at(line, CharCol::ZERO, &contents);
                            editor.set_status_message(format!(
                                "Read {} lines from {}",
                                contents.lines().count(),
                                display_target
                            ));
                        }
                        Err(e) => {
                            editor.set_status_message(format!("Error reading file: {}", e));
                        }
                    }
                }
            }
            return Ok(());
        }

        // Check if it's a :b <n> or :buffer <n> command
        if let Some(buffer_num_str) = command
            .strip_prefix("b ")
            .or_else(|| command.strip_prefix("buffer "))
        {
            if let Ok(buffer_num) = buffer_num_str.trim().parse::<usize>() {
                if buffer_num > 0 {
                    // Convert from 1-indexed to 0-indexed
                    editor.switch_to_buffer(buffer_num - 1);
                }
            }
            return Ok(());
        }

        // Check if it's a :colorscheme <name> or :colo <name> command
        if let Some(scheme_name) = command
            .strip_prefix("colorscheme ")
            .or_else(|| command.strip_prefix("colo "))
        {
            match editor.set_color_scheme(scheme_name.trim()) {
                Ok(_) => {
                    let message = format!("Color scheme set to '{}'", scheme_name.trim());
                    editor.set_status_message(message);
                }
                Err(e) => {
                    let available = editor.list_color_schemes().join(", ");
                    let message = format!("{}. Available schemes: {}", e, available);
                    editor.set_status_message(message);
                }
            }
        } else if editor.status_message().is_empty() {
            // Truly unrecognized ex-command (no handler set a status). Report it
            // the way Vim does (E492) so the user gets feedback on typos instead
            // of silence, and so the headless API can surface it as an error.
            // Guarded on an empty status so a recognized command that failed and
            // already set its own error (e.g. E20 from `:copy 'z`) isn't clobbered.
            editor.set_status_message(format!("E492: Not an editor command: {}", command));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_range_with_status_reports_missing_mark() {
        let mut editor = Editor::with_content("a\nb\nc\n");
        let range = parse_range_with_status(&mut editor, "1,'a", None);

        assert_eq!(range, None);
        assert_eq!(editor.status_message(), "E20: Mark not set");
    }
}
