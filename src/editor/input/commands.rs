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
    let replacement = parts[2];
    let flags = if parts.len() >= 4 { parts[3] } else { "" };

    // Parse flags
    let global = flags.contains('g');
    let ignore_case = flags.contains('i');

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

    // Perform substitution with change tracking
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
                regex.replace_all(line_text, replacement).to_string()
            } else {
                // Replace first occurrence
                regex.replace(line_text, replacement).to_string()
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

    // Update the : register with the command
    editor.registers_mut().set_last_command(command.to_string());

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

    // Handle commands without arguments
    match command {
        "q" | "quit" => {
            // Quit without checking for modifications
            if editor.is_modified() {
                // In a real editor, we'd show an error message
                // For now, just don't quit if modified
                return Ok(());
            }
            editor.quit();
        }
        "q!" | "quit!" => {
            // Force quit without saving
            editor.quit();
        }
        "qa" | "qall" => {
            // Quit all - for now same as quit since we only have one buffer
            // In the future, this would check all buffers for modifications
            if editor.is_modified() {
                // Don't quit if modified
                return Ok(());
            }
            editor.quit();
        }
        "qa!" | "qall!" => {
            // Force quit all without saving
            editor.quit();
        }
        "w" | "write" => {
            // Save to current file
            editor.buffer_mut().save()?;
            editor.mark_saved();
        }
        "wq" | "x" => {
            // Write and quit
            editor.buffer_mut().save()?;
            editor.mark_saved();
            editor.quit();
        }
        "wq!" => {
            // Force write and quit
            editor.buffer_mut().save()?;
            editor.mark_saved();
            editor.quit();
        }
        "noh" | "nohl" | "nohlsearch" => {
            // Clear search highlighting
            editor.clear_search_highlight();
        }
        "LspInfo" | "lspinfo" => {
            // Show LSP information
            let info = editor.get_lsp_info();
            editor.set_lsp_status(info);
        }
        _ if command.starts_with("LspRename ") || command.starts_with("lsprename ") => {
            // Extract new name from command
            let new_name = if let Some(name) = command.strip_prefix("LspRename ") {
                name.trim()
            } else if let Some(name) = command.strip_prefix("lsprename ") {
                name.trim()
            } else {
                ""
            };

            if new_name.is_empty() {
                editor.set_lsp_status("Usage: :LspRename <newname>".to_string());
            } else {
                editor.request_rename(new_name.to_string());
            }
            return Ok(());
        }
        "colorscheme" | "colo" => {
            // Show current color scheme and available schemes
            let current = editor.current_color_scheme_name();
            let schemes = editor.list_color_schemes().join(", ");
            let message = format!("Current: {}\nAvailable: {}", current, schemes);
            editor.set_lsp_status(message);
        }
        "ls" | "buffers" | "files" => {
            // List all buffers
            let buffer_list = editor.list_buffers();
            editor.set_lsp_status(buffer_list);
        }
        "bn" | "bnext" => {
            // Switch to next buffer
            editor.next_buffer();
        }
        "bp" | "bprev" | "bprevious" => {
            // Switch to previous buffer
            editor.prev_buffer();
        }
        "bd" | "bdelete" => {
            // Delete current buffer
            if !editor.delete_current_buffer() {
                // Last buffer can't be deleted
                editor.set_lsp_status("Cannot delete last buffer".to_string());
            }
        }
        "bf" | "bfirst" => {
            // Switch to first buffer
            editor.switch_to_buffer(0);
        }
        "bl" | "blast" => {
            // Switch to last buffer
            let last_idx = editor.buffer_count().saturating_sub(1);
            editor.switch_to_buffer(last_idx);
        }
        "wqa" | "wqall" | "xa" | "xall" => {
            // Write all and quit
            // For now just save current buffer and quit
            editor.buffer_mut().save()?;
            editor.mark_saved();
            editor.quit();
        }
        "only" => {
            // Close all windows except current
            // For now this is a no-op since window management is minimal
            // TODO: Implement when multi-window support is more robust
        }
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
