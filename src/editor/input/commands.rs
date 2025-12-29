use crate::api::ApiResponse;
use crate::editor::{Change, Editor, Mode, Range};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

/// Handles input in Command mode
pub fn handle_command_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        KeyCode::Char(ch) => {
            // Add character to command line
            editor.append_to_command_line(ch);
        }
        KeyCode::Backspace => {
            // Remove last character from command line
            editor.backspace_command_line();
        }
        KeyCode::Up => {
            // Navigate to previous command in history
            editor.history_prev();
        }
        KeyCode::Down => {
            // Navigate to next command in history
            editor.history_next();
        }
        KeyCode::Enter => {
            // Add to history before executing
            editor.add_command_to_history();
            // Execute the command
            execute_command(editor)?;
            editor.clear_command_line();
            editor.set_mode(Mode::Normal);
        }
        KeyCode::Esc => {
            // Cancel command mode
            editor.clear_command_line();
            editor.set_mode(Mode::Normal);
        }
        _ => {}
    }
    Ok(())
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
        return Ok(()); // Invalid format
    }

    let parts: Vec<&str> = substitute_part.splitn(4, '/').collect();
    if parts.len() < 3 {
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
                        let replacement_text = regex.replace(mat.as_str(), replacement.as_str()).to_string();
                        matches.push((line_idx, mat.start(), mat.end(), replacement_text));
                    }
                } else {
                    // Only first match per line
                    if let Some(mat) = regex.find(line_text) {
                        let replacement_text = regex.replace(mat.as_str(), replacement.as_str()).to_string();
                        matches.push((line_idx, mat.start(), mat.end(), replacement_text));
                    }
                }
            }
        }

        if matches.is_empty() {
            editor.set_lsp_status("Pattern not found".to_string());
        } else {
            let count = matches.len();
            editor.set_lsp_status(format!("replace with {} ({} matches) (y/n/a/q/l)", replacement, count));
            editor.start_substitute_confirm(matches, regex);
        }

        return Ok(());
    }

    // Perform substitution with change tracking (non-confirm mode)
    let cursor_before = (
        editor.buffer().cursor().line(),
        editor.buffer().cursor().col(),
    );

    for line_idx in start_line..=end_line.min(editor.buffer().line_count().saturating_sub(1)) {
        if let Some(line) = editor.buffer().line(line_idx) {
            let line_text = line.trim_end_matches('\n');

            // Perform the substitution
            let new_text = if global {
                // Replace all occurrences
                regex.replace_all(line_text, replacement.as_str()).to_string()
            } else {
                // Replace first occurrence
                regex.replace(line_text, replacement.as_str()).to_string()
            };

            if new_text != line_text {
                // Delete old line content and insert new
                let line_len = line_text.chars().count();
                let deleted = editor
                    .buffer_mut()
                    .delete_range(line_idx, 0, line_idx, line_len);
                let delete_range = Range::new((line_idx, 0), (line_idx, line_len));
                let delete_change = Change::delete(delete_range, deleted, cursor_before);

                let insert_change = Change::insert((line_idx, 0), new_text, cursor_before);
                insert_change.apply(editor.buffer_mut());

                editor.add_change(delete_change);
                editor.add_change(insert_change);
            }
        }
    }

    Ok(())
}

/// Handles :global and :vglobal commands (:g/pattern/command, :v/pattern/command)
fn handle_global_command(editor: &mut Editor, command: &str) -> Result<()> {
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

    // Find the closing / for the pattern
    let pattern_end = if let Some(idx) = rest[1..].find('/') {
        idx + 1
    } else {
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

    // Find all matching lines
    let line_count = editor.buffer().line_count();
    let mut matching_lines = Vec::new();

    for line_idx in 0..line_count {
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
            // Delete all matching lines (in reverse order to avoid index shifts)
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );
            let mut all_deleted = Vec::new();

            for &line_idx in matching_lines.iter().rev() {
                if let Some(line) = editor.buffer().line(line_idx) {
                    all_deleted.push(line.to_string());

                    let _line_len = line.trim_end_matches('\n').chars().count();

                    // Calculate character range
                    let start_char = editor.buffer().rope().line_to_char(line_idx);
                    let end_char = if line_idx + 1 < editor.buffer().line_count() {
                        editor.buffer().rope().line_to_char(line_idx + 1)
                    } else {
                        editor.buffer().rope().len_chars()
                    };

                    // Delete the line
                    editor.buffer_mut().rope_mut().remove(start_char..end_char);
                }
            }

            // Store in register
            all_deleted.reverse(); // Restore original order
            let deleted_text = all_deleted.join("");
            editor.delete_to_register(deleted_text.clone());

            // Position cursor at first deleted line
            let new_cursor_line =
                matching_lines[0].min(editor.buffer().line_count().saturating_sub(1));
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(new_cursor_line, 0);

            // Record change
            let range = Range::new(
                (matching_lines[0], 0),
                (matching_lines[matching_lines.len() - 1] + 1, 0),
            );
            let change = Change::delete(range, deleted_text, cursor_before);
            editor.add_change(change);

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
            // Run substitute on matching lines
            for &line_idx in &matching_lines {
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_text = line.trim_end_matches('\n');

                    // Parse substitute pattern: s/pattern/replacement/flags
                    let parts: Vec<&str> = sub_command.splitn(4, '/').collect();
                    if parts.len() < 3 {
                        continue;
                    }

                    let sub_pattern = parts[1];
                    let replacement = parts[2];
                    let flags = if parts.len() >= 4 { parts[3] } else { "" };

                    let global = flags.contains('g');
                    let ignore_case = flags.contains('i');

                    // Compile regex
                    use regex::RegexBuilder;
                    let sub_regex = match RegexBuilder::new(sub_pattern)
                        .case_insensitive(ignore_case)
                        .build()
                    {
                        Ok(r) => r,
                        Err(_) => continue,
                    };

                    // Perform substitution
                    let new_text = if global {
                        sub_regex.replace_all(line_text, replacement).to_string()
                    } else {
                        sub_regex.replace(line_text, replacement).to_string()
                    };

                    if new_text != line_text {
                        let cursor_before = (
                            editor.buffer().cursor().line(),
                            editor.buffer().cursor().col(),
                        );
                        let line_len = line_text.chars().count();

                        let deleted = editor
                            .buffer_mut()
                            .delete_range(line_idx, 0, line_idx, line_len);
                        let delete_range = Range::new((line_idx, 0), (line_idx, line_len));
                        let delete_change = Change::delete(delete_range, deleted, cursor_before);

                        let insert_change = Change::insert((line_idx, 0), new_text, cursor_before);
                        insert_change.apply(editor.buffer_mut());

                        editor.add_change(delete_change);
                        editor.add_change(insert_change);
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
        if let Some(mark) = editor.marks.get_mark(mark_char) {
            return Some(mark.line);
        }
        return None;
    }

    // +N or -N (relative to current line)
    if endpoint.starts_with('+') {
        let offset: usize = endpoint[1..].parse().ok()?;
        return Some((cursor_line + offset).min(last_line));
    }
    if endpoint.starts_with('-') {
        let offset: usize = endpoint[1..].parse().ok()?;
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
    use std::process::{Command, Stdio};

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

                    // Record change for undo
                    let cursor_before = (
                        editor.buffer().cursor().line(),
                        editor.buffer().cursor().col(),
                    );

                    // Delete the original range
                    let start_char = editor.buffer().rope().line_to_char(start_line);
                    let end_char = if end_line + 1 < editor.buffer().line_count() {
                        editor.buffer().rope().line_to_char(end_line + 1)
                    } else {
                        editor.buffer().rope().len_chars()
                    };

                    let deleted_text = editor
                        .buffer()
                        .rope()
                        .slice(start_char..end_char)
                        .to_string();

                    editor.buffer_mut().rope_mut().remove(start_char..end_char);

                    // Insert the command output (with trailing newline if needed)
                    let insert_text = if output_text.is_empty() {
                        String::new()
                    } else if output_text.ends_with('\n') {
                        output_text.to_string()
                    } else {
                        format!("{}\n", output_text)
                    };

                    if !insert_text.is_empty() {
                        editor
                            .buffer_mut()
                            .rope_mut()
                            .insert(start_char, &insert_text);
                    }

                    // Position cursor at start of filtered range
                    editor
                        .buffer_mut()
                        .cursor_mut()
                        .set_position(start_line, 0);

                    // Record change for undo using composite of delete + insert
                    let range = Range::new((start_line, 0), (end_line + 1, 0));
                    let delete_change = Change::delete(range.clone(), deleted_text, cursor_before);
                    let insert_change =
                        Change::insert((start_line, 0), insert_text.clone(), cursor_before);
                    let cursor_after = (start_line, 0);
                    let change = Change::composite(
                        vec![delete_change, insert_change],
                        cursor_before,
                        cursor_after,
                    );
                    editor.add_change(change);

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
        // Simple command execution - just display output
        let output = Command::new(shell)
            .arg(shell_arg)
            .arg(shell_cmd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if output.status.success() {
                    let msg = if stdout.is_empty() {
                        "Command completed".to_string()
                    } else {
                        // Truncate long output for status line
                        let trimmed = stdout.trim();
                        if trimmed.len() > 200 || trimmed.lines().count() > 3 {
                            format!(
                                "{}... ({} lines)",
                                trimmed
                                    .lines()
                                    .take(3)
                                    .collect::<Vec<_>>()
                                    .join(" | ")
                                    .chars()
                                    .take(100)
                                    .collect::<String>(),
                                trimmed.lines().count()
                            )
                        } else {
                            trimmed.lines().collect::<Vec<_>>().join(" | ")
                        }
                    };
                    editor.set_lsp_status(msg);
                } else {
                    editor.set_lsp_status(format!("Command failed: {}", stderr.trim()));
                }
            }
            Err(e) => {
                editor.set_lsp_status(format!("Failed to run command: {}", e));
            }
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
    // Split on | that's not escaped and handle each command
    if command.contains('|') && !command.starts_with('s') && !command.starts_with("%s") {
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

/// Execute a single command (no chaining)
fn execute_command_single(editor: &mut Editor, command: &str) -> Result<()> {
    // Update the : register with the command
    editor.registers_mut().set_last_command(command.to_string());

    // First, try to delegate to the top-level commands module which has all the standard commands
    let response = crate::commands::execute_command(editor, command);
    match response {
        ApiResponse::Success(success_resp) => {
            // Command executed successfully
            if let Some(msg) = success_resp.message {
                editor.set_lsp_status(msg);
            }
            return Ok(());
        }
        ApiResponse::Error(err_resp) => {
            // Check if it's an "unknown command" error
            if err_resp.error.contains("Not an editor command") {
                // Fall through to custom input-specific command handling below
            } else {
                // It's a real error from a known command
                editor.set_lsp_status(err_resp.error);
                return Ok(());
            }
        }
        _ => {
            // Other response types from commands module - just ignore
            return Ok(());
        }
    }

    // If we reach here, it's an unknown command - try custom input-specific handling
    // First, try to parse range from command
    // Format: :[range]command
    let (range_str, cmd_part) =
        if let Some(first_alpha) = command.chars().position(|c| c.is_alphabetic() || c == '!') {
            (&command[..first_alpha], &command[first_alpha..])
        } else {
            // No command part, might be just a line number (goto)
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
            let cursor_before = (
                editor.buffer().cursor().line(),
                editor.buffer().cursor().col(),
            );

            // Calculate character range to delete
            let start_char = editor.buffer().rope().line_to_char(start_line);
            let end_char = if end_line + 1 < editor.buffer().line_count() {
                editor.buffer().rope().line_to_char(end_line + 1)
            } else {
                editor.buffer().rope().len_chars()
            };

            // Store deleted text
            let deleted_text = editor
                .buffer()
                .rope()
                .slice(start_char..end_char)
                .to_string();

            // Remove the lines
            editor.buffer_mut().rope_mut().remove(start_char..end_char);

            // Store in register (use delete, which updates " and numbered regs but not 0)
            editor.delete_to_register(deleted_text.clone());

            // Position cursor at start of deleted range
            let new_cursor_line = start_line.min(editor.buffer().line_count().saturating_sub(1));
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(new_cursor_line, 0);

            // Record change for undo
            let range = Range::new((start_line, 0), (end_line + 1, 0));
            let change = Change::delete(range, deleted_text, cursor_before);
            editor.add_change(change);

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

    // Handle commands with arguments
    if command.starts_with("e ") || command.starts_with("edit ") {
        // :e <filename> or :edit <filename>
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.len() >= 2 {
            let filename = parts[1..].join(" ");
            editor.load_file(&filename)?;
        }
        return Ok(());
    }

    if command.starts_with("w ") || command.starts_with("write ") {
        // :w <filename> or :write <filename> - save as
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.len() >= 2 {
            let filename = parts[1..].join(" ");
            editor.buffer_mut().save_as(&filename)?;
            editor.mark_saved();
        }
        return Ok(());
    }

    // Handle substitute command (:s, :%s, :'<,'>s)
    // Check if it's a substitute command (contains 's/' pattern)
    if command.ends_with("s/") || command.contains("s/") {
        handle_substitute_command(editor, &command)?;
        return Ok(());
    }

    // Handle :global and :vglobal commands (:g/pattern/command, :v/pattern/command)
    if command.starts_with("g/") || command.starts_with("g!/") || command.starts_with("v/") {
        handle_global_command(editor, command)?;
        return Ok(());
    }

    // Handle shell command (:! or :.! or :%!)
    if cmd_part.starts_with('!') {
        let shell_cmd = cmd_part[1..].trim();
        if !shell_cmd.is_empty() {
            handle_shell_command(editor, range_str, shell_cmd)?;
            return Ok(());
        }
    }

    // Only handle custom input-specific commands here.
    // Standard commands are now delegated to the top-level commands module.
    match command {
        _ => {
            // Check if it's a :r or :read command
            if let Some(filename) = command
                .strip_prefix("r ")
                .or_else(|| command.strip_prefix("read "))
            {
                let filename = filename.trim();
                if !filename.is_empty() {
                    // Read file contents
                    match std::fs::read_to_string(filename) {
                        Ok(contents) => {
                            // Insert contents at current cursor position
                            let cursor = editor.buffer().cursor();
                            let line = cursor.line() + 1; // Insert after current line
                            let col = 0;
                            editor.buffer_mut().insert_text_at(line, col, &contents);
                            editor.set_lsp_status(format!(
                                "Read {} lines from {}",
                                contents.lines().count(),
                                filename
                            ));
                        }
                        Err(e) => {
                            editor.set_lsp_status(format!("Error reading file: {}", e));
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
    }

    Ok(())
}
