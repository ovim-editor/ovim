use crate::command_result::CommandResult;
use crate::edit::Edit;
use crate::editor::path_completion::extract_path_from_command;
use crate::editor::{Editor, Mode};
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

/// Handles Tab (forward=false) or BackTab (backward=true) for path completion.
fn handle_tab_completion(editor: &mut Editor, backward: bool) {
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

/// Converts Vim-style backreferences (\1, \2, \0, &) to Rust regex syntax ($1, $2, $0, $0)
fn convert_vim_backrefs(replacement: &str) -> String {
    let mut result = String::with_capacity(replacement.len() * 2);
    let mut chars = replacement.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                match next {
                    '0'..='9' => {
                        // \1 -> $1, \2 -> $2, etc.
                        result.push('$');
                        result.push(chars.next().unwrap());
                    }
                    '\\' => {
                        // \\ -> \
                        result.push('\\');
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
            // & means whole match in Vim, $0 in Rust
            result.push_str("$0");
        } else {
            result.push(ch);
        }
    }

    result
}

/// Handles substitute command (:s/pattern/replacement/flags)
fn handle_substitute_command(editor: &mut Editor, command: &str) -> Result<()> {
    // Parse the command to extract range, pattern, replacement, and flags
    // Supported formats:
    // :s/pattern/replacement/[flags]
    // :%s/pattern/replacement/[flags]
    // :'<,'>s/pattern/replacement/[flags]
    // :1,5s/pattern/replacement/[flags]

    let (range_str, substitute_part) = if let Some(s_idx) = command.rfind('s') {
        (&command[..s_idx], &command[s_idx + 1..])
    } else {
        return Ok(()); // No 's' found
    };

    // Parse substitute pattern: /pattern/replacement/flags
    if !substitute_part.starts_with('/') {
        editor.set_lsp_status("E146: Invalid substitute syntax".to_string());
        return Ok(()); // Invalid format
    }

    let parts: Vec<&str> = substitute_part.splitn(4, '/').collect();
    if parts.len() < 3 {
        editor.set_lsp_status("E146: Invalid substitute syntax".to_string());
        return Ok(()); // Invalid format - need at least /pattern/replacement/
    }

    let pattern = parts[1];
    // Convert Vim-style backreferences to Rust regex syntax
    let replacement = convert_vim_backrefs(parts[2]);
    let flags = if parts.len() >= 4 { parts[3] } else { "" };

    // Parse flags
    let global = flags.contains('g');
    let ignore_case = flags.contains('i');
    let confirm = flags.contains('c');

    // Determine the range using the new parser (returns inclusive range)
    let (start_line, end_line) = if let Some((start, end)) = parse_range(editor, range_str) {
        (start, end)
    } else {
        // Invalid range
        return Ok(());
    };

    // Compile the regex pattern
    use regex::RegexBuilder;
    let regex = match RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
    {
        Ok(r) => r,
        Err(_) => {
            editor.set_lsp_status(format!("Invalid regex pattern: {}", pattern));
            return Ok(());
        }
    };

    // If confirm flag is set, collect matches and enter SubstituteConfirm mode
    if confirm {
        let mut matches = Vec::new();

        for line_idx in start_line..=end_line.min(editor.buffer().line_count().saturating_sub(1)) {
            if let Some(line) = editor.buffer().line(line_idx) {
                let line_text = line.trim_end_matches('\n');

                // Find all matches in this line
                if global {
                    for mat in regex.find_iter(line_text) {
                        let replacement_text = regex
                            .replace(mat.as_str(), replacement.as_str())
                            .to_string();
                        matches.push((line_idx, mat.start(), mat.end(), replacement_text));
                    }
                } else {
                    // Only first match per line
                    if let Some(mat) = regex.find(line_text) {
                        let replacement_text = regex
                            .replace(mat.as_str(), replacement.as_str())
                            .to_string();
                        matches.push((line_idx, mat.start(), mat.end(), replacement_text));
                    }
                }
            }
        }

        if matches.is_empty() {
            editor.set_lsp_status("Pattern not found".to_string());
        } else {
            let count = matches.len();
            editor.set_lsp_status(format!(
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
        for line_idx in start_line..=end_line.min(buf.line_count().saturating_sub(1)) {
            if let Some(line) = buf.line(line_idx) {
                let line_text = line.trim_end_matches('\n');

                let new_text = if global {
                    regex
                        .replace_all(line_text, replacement.as_str())
                        .to_string()
                } else {
                    regex.replace(line_text, replacement.as_str()).to_string()
                };

                if new_text != line_text {
                    let line_len = line_text.chars().count();
                    buf.delete_range(line_idx, 0, line_idx, line_len);
                    buf.insert_text_at(line_idx, 0, &new_text);
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
        editor.set_lsp_status("Invalid global command format".to_string());
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
        editor.set_lsp_status("Invalid global command: missing closing /".to_string());
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
            editor.set_lsp_status(format!("Invalid regex pattern: {}", pattern));
            return Ok(());
        }
    };

    // Find all matching lines (optionally restricted by Ex range)
    let line_count = editor.buffer().line_count();
    if line_count == 0 {
        editor.set_lsp_status("No matching lines found".to_string());
        return Ok(());
    }

    let (scan_start, scan_end) = range.unwrap_or((0, line_count.saturating_sub(1)));
    let scan_start = scan_start.min(line_count.saturating_sub(1));
    let scan_end = scan_end.min(line_count.saturating_sub(1));

    let mut matching_lines = Vec::new();

    for line_idx in scan_start..=scan_end {
        if let Some(line) = editor.buffer().line(line_idx) {
            let line_text = line.trim_end_matches('\n');
            let matches = regex.is_match(line_text);

            // Include line if: (matches && !invert) || (!matches && invert)
            if matches != invert {
                matching_lines.push(line_idx);
            }
        }
    }

    if matching_lines.is_empty() {
        editor.set_lsp_status("No matching lines found".to_string());
        return Ok(());
    }

    // Execute the command on matching lines
    match sub_command {
        "d" | "delete" => {
            let cursor_before = editor.cursor_position();
            let (mut all_deleted, edits) = editor.buffer_mut().record(|buf| {
                let mut deleted_chunks = Vec::new();
                for &line_idx in matching_lines.iter().rev() {
                    let deleted = buf.delete_range(line_idx, 0, line_idx + 1, 0);
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
                .set_position(new_cursor_line, 0);

            if !edits.is_empty() {
                let cursor_after = (new_cursor_line, 0);
                editor.push_recorded_undo(edits, cursor_before, cursor_after);
            }

            editor.set_lsp_status(format!("Deleted {} line(s)", matching_lines.len()));
        }
        "p" | "print" => {
            // Print all matching lines to status
            let mut output = Vec::new();
            for &line_idx in &matching_lines {
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_text = line.trim_end_matches('\n');
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

            editor.set_lsp_status(result);
        }
        "y" | "yank" => {
            // Yank all matching lines
            let mut yanked_lines = Vec::new();
            for &line_idx in &matching_lines {
                if let Some(line) = editor.buffer().line(line_idx) {
                    yanked_lines.push(line.to_string());
                }
            }
            let yanked_text = yanked_lines.join("");
            editor.yank_to_register(yanked_text);

            editor.set_lsp_status(format!("Yanked {} line(s)", matching_lines.len()));
        }
        _ if sub_command.starts_with("s/") => {
            // Parse substitute pattern once, outside the loop
            let parts: Vec<&str> = sub_command.splitn(4, '/').collect();
            if parts.len() >= 3 {
                let sub_pattern = parts[1];
                let replacement = parts[2];
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
                        for &line_idx in &matching_lines {
                            if let Some(line) = buf.line(line_idx) {
                                let line_text = line.trim_end_matches('\n');

                                let new_text = if global {
                                    sub_regex.replace_all(line_text, replacement).to_string()
                                } else {
                                    sub_regex.replace(line_text, replacement).to_string()
                                };

                                if new_text != line_text {
                                    let line_len = line_text.chars().count();
                                    buf.delete_range(line_idx, 0, line_idx, line_len);
                                    buf.insert_text_at(line_idx, 0, &new_text);
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

            editor.set_lsp_status(format!("Substituted on {} line(s)", matching_lines.len()));
        }
        _ => {
            editor.set_lsp_status(format!("Unsupported global command: {}", sub_command));
        }
    }

    Ok(())
}

/// Parses an Ex command range (e.g., "1,5", "%", ".", "'a,'b")
/// Returns (start_line, end_line) as 0-indexed, inclusive
pub fn parse_range(editor: &Editor, range_str: &str) -> Option<(usize, usize)> {
    let range_str = range_str.trim();

    if range_str.is_empty() {
        // No range - current line only
        let cursor_line = editor.buffer().cursor().line();
        return Some((cursor_line, cursor_line));
    }

    // Handle % (all lines)
    if range_str == "%" {
        if editor.buffer().line_count() == 0 {
            return None;
        }
        return Some((0, editor.buffer().line_count().saturating_sub(1)));
    }

    // Handle visual selection markers
    if range_str == "'<,'>" || range_str.contains("'<") {
        if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
            return Some((start_line, end_line));
        }
        return None;
    }

    // Handle ranges with comma (e.g., "1,5", ".,$ ", "'a,'b")
    if let Some(comma_idx) = range_str.find(',') {
        let start_part = range_str[..comma_idx].trim();
        let end_part = range_str[comma_idx + 1..].trim();

        let start = parse_range_endpoint(editor, start_part)?;
        let end = parse_range_endpoint(editor, end_part)?;

        return Some((start.min(end), start.max(end)));
    }

    // Single endpoint
    let line = parse_range_endpoint(editor, range_str)?;
    Some((line, line))
}

/// Parses a single range endpoint (e.g., ".", "$", "5", "'a", "+3")
fn parse_range_endpoint(editor: &Editor, endpoint: &str) -> Option<usize> {
    let endpoint = endpoint.trim();
    let cursor_line = editor.buffer().cursor().line();
    let last_line = editor.buffer().line_count().saturating_sub(1);

    // . = current line
    if endpoint == "." {
        return Some(cursor_line);
    }

    // $ = last line
    if endpoint == "$" {
        return Some(last_line);
    }

    // 'x = mark
    if endpoint.starts_with('\'') && endpoint.len() == 2 {
        let mark_char = endpoint.chars().nth(1)?;
        if let Some(mark) = editor.nav.marks.get_mark(mark_char) {
            return Some(mark.line);
        }
        // TODO (Bug 1): Add "E20: Mark not set" error message
        // Currently returns None silently. To fix, need to refactor parse_range_endpoint
        // to return Result<usize, String> instead of Option<usize> so we can propagate
        // error messages, or change signature to take &mut Editor to set status directly.
        return None;
    }

    // +N or -N (relative to current line)
    if let Some(rest) = endpoint.strip_prefix('+') {
        let offset: usize = rest.parse().ok()?;
        return Some((cursor_line + offset).min(last_line));
    }
    if let Some(rest) = endpoint.strip_prefix('-') {
        let offset: usize = rest.parse().ok()?;
        return Some(cursor_line.saturating_sub(offset));
    }

    // Plain number (1-indexed in Vim, convert to 0-indexed)
    if let Ok(line_num) = endpoint.parse::<usize>() {
        if line_num == 0 {
            return Some(0);
        }
        // Convert to 0-indexed and clamp to valid range
        return Some((line_num.saturating_sub(1)).min(last_line));
    }

    None
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
        let (start_line, end_line) = match parse_range(editor, range_str) {
            Some(range) => range,
            None => {
                editor.set_lsp_status("Invalid range".to_string());
                return Ok(());
            }
        };

        // Get the text from the range
        let mut input_text = String::new();
        for line_idx in start_line..=end_line {
            if let Some(line) = editor.buffer().line(line_idx) {
                input_text.push_str(&line);
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
                    let output_text = stdout.trim_end_matches('\n');

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
                        buf.delete_range(start_line, 0, end_line + 1, 0);
                        if !insert_text.is_empty() {
                            buf.insert_text_at(start_line, 0, &insert_text);
                        }
                        buf.cursor_mut().set_position(start_line, 0);
                    });
                    if !edits.is_empty() {
                        let cursor_after = (start_line, 0);
                        editor.push_recorded_undo(edits, cursor_before, cursor_after);
                    }

                    let line_count = insert_text.lines().count();
                    editor.set_lsp_status(format!("{} lines filtered", line_count));
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    editor.set_lsp_status(format!("Command failed: {}", stderr.trim()));
                }
            }
            Err(e) => {
                editor.set_lsp_status(format!("Failed to run command: {}", e));
            }
        }
    } else {
        // Simple command execution - let shell handle I/O so pipelines work
        // (e.g., `:!echo % | pbcopy` needs echo's output to go to pbcopy, not us)
        let status = Command::new(shell)
            .arg(shell_arg)
            .arg(shell_cmd)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .stdin(Stdio::inherit())
            .status();

        match status {
            Ok(status) => {
                if status.success() {
                    editor.set_lsp_status("Command completed".to_string());
                } else {
                    editor.set_lsp_status(format!("Command exited with {}", status));
                }
            }
            Err(e) => {
                editor.set_lsp_status(format!("Failed to run command: {}", e));
            }
        }
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
                    editor.set_lsp_status("Command produced no output".to_string());
                    return Ok(());
                }

                // Determine insertion point
                let insert_line = if range_str.is_empty() {
                    // Insert after current line
                    editor.buffer().cursor().line() + 1
                } else if let Some((_, end_line)) = parse_range(editor, range_str) {
                    // Insert after the range end line
                    end_line + 1
                } else {
                    editor.set_lsp_status("Invalid range".to_string());
                    return Ok(());
                };

                // Record change for undo
                let cursor_before = (
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
                    format!("\n{}", text.trim_end_matches('\n'))
                } else {
                    text
                };

                // Insert the text
                editor.buffer_mut().rope_mut().insert(insert_char, &text);

                // Position cursor at start of inserted text
                let cursor_after = (insert_line, 0);
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(insert_line, 0);
                editor.push_recorded_undo(
                    vec![Edit::Insert {
                        offset: insert_char,
                        text: text.clone(),
                    }],
                    cursor_before,
                    cursor_after,
                );

                let line_count = text.lines().count();
                editor.set_lsp_status(format!(
                    "{} line{} inserted",
                    line_count,
                    if line_count == 1 { "" } else { "s" }
                ));
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                editor.set_lsp_status(format!("Command failed: {}", stderr.trim()));
            }
        }
        Err(e) => {
            editor.set_lsp_status(format!("Failed to run command: {}", e));
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
    } else if let Some((start_line, end_line)) = parse_range(editor, range_str) {
        // Write specified range
        let mut text = String::new();
        for line_idx in start_line..=end_line {
            if let Some(line) = editor.buffer().line(line_idx) {
                text.push_str(&line);
            }
        }
        text
    } else {
        editor.set_lsp_status("Invalid range".to_string());
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
            editor.set_lsp_status(format!("Failed to run command: {}", e));
            return Ok(());
        }
    };

    // Write content to stdin
    if let Some(ref mut stdin) = child.stdin {
        if let Err(e) = stdin.write_all(content.as_bytes()) {
            editor.set_lsp_status(format!("Failed to write to command: {}", e));
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
                        format!("{} lines written: {}...", line_count, &trimmed[..100])
                    } else {
                        format!("{} lines written: {}", line_count, trimmed)
                    }
                };
                editor.set_lsp_status(msg);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                editor.set_lsp_status(format!("Command failed: {}", stderr.trim()));
            }
        }
        Err(e) => {
            editor.set_lsp_status(format!("Failed to wait for command: {}", e));
        }
    }

    Ok(())
}

/// Executes a command string directly (used for API/Lua commands)
pub fn execute_command_string(editor: &mut Editor, command: &str) -> Result<()> {
    execute_command_impl(editor, command)
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

/// Helper: Check if command contains patterns that may include | characters
/// Returns true for substitute (:s), global (:g), and vglobal (:v) commands
fn is_command_with_pattern(command: &str) -> bool {
    // Match: :s/, :%s/, :'<,'>s/, :1,5s/, etc.
    if command.contains("s/") {
        return true;
    }
    // Match: :g/, :g!/, :v/ (including when prefixed by a range like % or .,$)
    //
    // This is intentionally permissive: returning true here simply prevents command-chaining
    // splitting on |, which can legally appear in regex patterns.
    if command.contains("g/") || command.contains("g!/") || command.contains("v/") {
        return true;
    }
    false
}

/// Execute a single command (no chaining)
fn execute_command_single(editor: &mut Editor, command: &str) -> Result<()> {
    // Update the : register with the command
    editor.registers_mut().set_last_command(command.to_string());

    // First, try to delegate to the top-level commands module which has all the standard commands
    let response = crate::commands::execute_command(editor, command);
    match response {
        CommandResult::Success(success_resp) => {
            // Command executed successfully
            if let Some(msg) = success_resp.message {
                // Multi-line messages go to hover popup, single-line to status bar
                if msg.contains('\n') {
                    editor.set_hover_info(msg);
                } else {
                    editor.set_lsp_status(msg);
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
                editor.set_lsp_status(err_resp.error);
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
    let first_alpha = command.chars().position(|c| c.is_alphabetic());
    let bang_split = command.find('!').and_then(|exclaim_idx| {
        let is_shell_separator = match first_alpha {
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
    } else if let Some(first_alpha) = first_alpha {
        (&command[..first_alpha], &command[first_alpha..])
    } else {
        (command, "")
    };

    // Handle goto line (just a number or range without command)
    if cmd_part.is_empty() && !range_str.is_empty() {
        if let Some((start_line, _end_line)) = parse_range(editor, range_str) {
            editor.buffer_mut().cursor_mut().set_position(start_line, 0);
            return Ok(());
        }
    }

    // Handle ranged delete command (:d or :delete)
    if cmd_part == "d" || cmd_part == "delete" {
        if let Some((start_line, end_line)) = parse_range(editor, range_str) {
            let cursor_before = editor.cursor_position();
            let (deleted_text, edits) = editor.buffer_mut().record(|buf| {
                buf.delete_range(start_line, 0, end_line + 1, 0)
            });

            // Store in register (use delete, which updates " and numbered regs but not 0)
            editor.delete_to_register(deleted_text.clone());

            // Position cursor at start of deleted range
            let new_cursor_line = start_line.min(editor.buffer().line_count().saturating_sub(1));
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(new_cursor_line, 0);

            if !edits.is_empty() {
                let cursor_after = (new_cursor_line, 0);
                editor.push_recorded_undo(edits, cursor_before, cursor_after);
            }

            return Ok(());
        }
    }

    // Handle ranged yank command (:y or :yank)
    if cmd_part == "y" || cmd_part == "yank" {
        if let Some((start_line, end_line)) = parse_range(editor, range_str) {
            let mut yanked_lines = Vec::new();
            for line_idx in start_line..=end_line {
                if let Some(line) = editor.buffer().line(line_idx) {
                    yanked_lines.push(line.to_string());
                }
            }
            let yanked_text = yanked_lines.join("");

            // Store in register (use yank, which updates " and 0)
            editor.yank_to_register(yanked_text);

            return Ok(());
        }
    }

    // Handle :sort command (sorts lines in range)
    if cmd_part == "sort" || cmd_part.starts_with("sort ") {
        if let Some((start_line, end_line)) = parse_range(editor, range_str) {
            let reverse = cmd_part.contains('!') || cmd_part.contains(" r");
            let numeric = cmd_part.contains(" n");
            let unique = cmd_part.contains(" u");
            let ignore_case = cmd_part.contains(" i");

            // Collect lines
            let mut lines: Vec<String> = (start_line..=end_line)
                .filter_map(|idx| editor.buffer().line(idx).map(|l| l.to_string()))
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
            let cursor_before = (
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

            // Insert sorted lines
            let new_text = lines.join("");
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
            editor.set_lsp_status(format!("{} lines sorted", sorted_count));
            return Ok(());
        }
    }

    // Handle :copy or :t command (copy lines to destination)
    // Format: :[range]copy {address} or :[range]t {address}
    if cmd_part.starts_with("copy ")
        || cmd_part.starts_with("t ")
        || cmd_part == "copy"
        || cmd_part == "t"
    {
        let dest_str = cmd_part
            .strip_prefix("copy ")
            .or_else(|| cmd_part.strip_prefix("t "))
            .unwrap_or("");
        let dest_str = dest_str.trim();

        if dest_str.is_empty() {
            editor.set_lsp_status("E488: Trailing characters".to_string());
            return Ok(());
        }

        if let Some((start_line, end_line)) = parse_range(editor, range_str) {
            // Parse destination address
            if let Some(dest_line) = parse_range_endpoint(editor, dest_str) {
                // Collect lines to copy
                let mut lines_to_copy: Vec<String> = Vec::new();
                for idx in start_line..=end_line {
                    if let Some(line) = editor.buffer().line(idx) {
                        lines_to_copy.push(line.to_string());
                    }
                }
                let text_to_insert = lines_to_copy.join("");

                // Insert after destination line
                let insert_line = dest_line + 1;
                let cursor_before = (
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
                        format!("\n{}", text_to_insert.trim_end_matches('\n'))
                    } else {
                        text_to_insert.clone()
                    };

                editor.buffer_mut().rope_mut().insert(insert_char, &text);

                // Move cursor to first copied line
                let cursor_after = (insert_line, 0);
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(insert_line, 0);
                editor.push_recorded_undo(
                    vec![Edit::Insert {
                        offset: insert_char,
                        text: text.clone(),
                    }],
                    cursor_before,
                    cursor_after,
                );

                let count = lines_to_copy.len();
                editor.set_lsp_status(format!(
                    "{} line{} copied",
                    count,
                    if count == 1 { "" } else { "s" }
                ));
                return Ok(());
            } else {
                editor.set_lsp_status("E14: Invalid address".to_string());
                return Ok(());
            }
        }
    }

    // Handle :move or :m command (move lines to destination)
    // Format: :[range]move {address} or :[range]m {address}
    if cmd_part.starts_with("move ")
        || cmd_part.starts_with("m ")
        || cmd_part == "move"
        || cmd_part == "m"
    {
        let dest_str = cmd_part
            .strip_prefix("move ")
            .or_else(|| cmd_part.strip_prefix("m "))
            .unwrap_or("");
        let dest_str = dest_str.trim();

        if dest_str.is_empty() {
            editor.set_lsp_status("E488: Trailing characters".to_string());
            return Ok(());
        }

        if let Some((start_line, end_line)) = parse_range(editor, range_str) {
            // Parse destination address
            if let Some(mut dest_line) = parse_range_endpoint(editor, dest_str) {
                // Bug 2 fix: Check for invalid moves (moving to within the range)
                // Should be <= end_line to prevent moving into self
                if dest_line >= start_line && dest_line <= end_line {
                    editor.set_lsp_status("E134: Move lines into themselves".to_string());
                    return Ok(());
                }

                let cursor_before = (
                    editor.buffer().cursor().line(),
                    editor.buffer().cursor().col(),
                );

                // Collect lines to move
                let mut lines_to_move: Vec<String> = Vec::new();
                for idx in start_line..=end_line {
                    if let Some(line) = editor.buffer().line(idx) {
                        lines_to_move.push(line.to_string());
                    }
                }
                let text_to_move = lines_to_move.join("");
                let line_count = lines_to_move.len();

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

                // Insert after destination line
                let insert_line = dest_line + 1;
                let insert_char = if insert_line < editor.buffer().line_count() {
                    editor.buffer().rope().line_to_char(insert_line)
                } else {
                    editor.buffer().rope().len_chars()
                };

                // Add newline if we're at end of file
                let text =
                    if insert_line >= editor.buffer().line_count() && !text_to_move.is_empty() {
                        format!("\n{}", text_to_move.trim_end_matches('\n'))
                    } else {
                        text_to_move.clone()
                    };

                editor.buffer_mut().rope_mut().insert(insert_char, &text);

                // Move cursor to first moved line
                let cursor_after = (insert_line, 0);
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(insert_line, 0);
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

                editor.set_lsp_status(format!(
                    "{} line{} moved",
                    line_count,
                    if line_count == 1 { "" } else { "s" }
                ));
                return Ok(());
            } else {
                editor.set_lsp_status("E14: Invalid address".to_string());
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
            match parse_range(editor, range_str) {
                Some(r) => Some(r),
                None => {
                    editor.set_lsp_status("E14: Invalid address".to_string());
                    return Ok(());
                }
            }
        };

        handle_global_command(editor, cmd_part, range)?;
        return Ok(());
    }

    // Handle substitute command (:s, :%s, :'<,'>s)
    // Only treat as substitute when the command part starts with `s/`.
    if cmd_part.starts_with("s/") {
        handle_substitute_command(editor, command)?;
        return Ok(());
    }

    // Handle shell command (:! or :.! or :%!)
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
                            let col = 0;
                            editor.buffer_mut().insert_text_at(line, col, &contents);
                            editor.set_lsp_status(format!(
                                "Read {} lines from {}",
                                contents.lines().count(),
                                display_target
                            ));
                        }
                        Err(e) => {
                            editor.set_lsp_status(format!("Error reading file: {}", e));
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
                    editor.set_lsp_status(message);
                }
                Err(e) => {
                    let available = editor.list_color_schemes().join(", ");
                    let message = format!("{}. Available schemes: {}", e, available);
                    editor.set_lsp_status(message);
                }
            }
        } else {
            // Unknown command - for now just ignore
        }
    }

    Ok(())
}
