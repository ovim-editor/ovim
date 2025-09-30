use crate::editor::{Editor, Motions, Operator, Operators, TextObjects};
use crate::mode::Mode;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

/// Handles input events for the editor
pub struct InputHandler;

impl InputHandler {
    /// Processes a keyboard event
    pub fn handle_key_event(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        // Record the event if we're recording a macro
        // (but don't record the 'q' that stops recording)
        let should_record = editor.is_recording_macro()
            && !(key_event.code == KeyCode::Char('q') && editor.mode() == Mode::Normal);

        if should_record {
            editor.record_macro_event(key_event);
        }

        match editor.mode() {
            Mode::Normal => Self::handle_normal_mode(editor, key_event),
            Mode::Insert => Self::handle_insert_mode(editor, key_event),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
                Self::handle_visual_mode(editor, key_event)
            }
            Mode::Command => Self::handle_command_mode(editor, key_event),
            Mode::Search => Self::handle_search_mode(editor, key_event),
            Mode::Replace => Self::handle_replace_mode(editor, key_event),
        }
    }

    /// Handles input in Normal mode
    fn handle_normal_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        // Handle pending operator + motion (like 'dw', 'dd', 'yy')
        if let Some(operator) = editor.pending_operator() {
            editor.clear_pending_operator();
            let count = editor.effective_count();

            match (operator, key_event.code) {
                // Delete operations
                (Operator::Delete, KeyCode::Char('d')) => {
                    // dd - delete line
                    editor.save_undo_state();
                    let deleted = Operators::delete_line(editor.buffer_mut(), count)?;
                    editor.registers_mut().delete(deleted);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Delete, KeyCode::Char('w')) => {
                    // dw - delete word
                    editor.save_undo_state();
                    let deleted = Operators::delete_word(editor.buffer_mut(), count)?;
                    editor.registers_mut().delete(deleted);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Delete, KeyCode::Char('$')) => {
                    // d$ - delete to end of line
                    editor.save_undo_state();
                    let deleted = Operators::delete_to_end_of_line(editor.buffer_mut())?;
                    editor.registers_mut().delete(deleted);
                    editor.clear_count();
                    return Ok(());
                }
                // Yank operations
                (Operator::Yank, KeyCode::Char('y')) => {
                    // yy - yank line
                    let yanked = Operators::yank_line(editor.buffer(), count)?;
                    editor.registers_mut().yank(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Yank, KeyCode::Char('w')) => {
                    // yw - yank word
                    let yanked = Operators::yank_word(editor.buffer_mut(), count)?;
                    editor.registers_mut().yank(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                (Operator::Yank, KeyCode::Char('$')) => {
                    // y$ - yank to end of line
                    let yanked = Operators::yank_to_end_of_line(editor.buffer())?;
                    editor.registers_mut().yank(yanked);
                    editor.clear_count();
                    return Ok(());
                }
                // Change operations (delete + insert mode)
                (Operator::Change, KeyCode::Char('c')) => {
                    // cc - change line
                    editor.save_undo_state();
                    let deleted = Operators::delete_line(editor.buffer_mut(), count)?;
                    editor.registers_mut().delete(deleted);
                    editor.clear_count();
                    editor.set_mode(Mode::Insert);
                    Self::insert_line_above(editor)?;
                    return Ok(());
                }
                (Operator::Change, KeyCode::Char('w')) => {
                    // cw - change word
                    editor.save_undo_state();
                    let deleted = Operators::delete_word(editor.buffer_mut(), count)?;
                    editor.registers_mut().delete(deleted);
                    editor.clear_count();
                    editor.set_mode(Mode::Insert);
                    return Ok(());
                }
                (Operator::Change, KeyCode::Char('$')) => {
                    // c$ - change to end of line
                    editor.save_undo_state();
                    let deleted = Operators::delete_to_end_of_line(editor.buffer_mut())?;
                    editor.registers_mut().delete(deleted);
                    editor.clear_count();
                    editor.set_mode(Mode::Insert);
                    return Ok(());
                }
                // Text objects with 'i' (inner)
                (_, KeyCode::Char('i')) => {
                    // Restore operator and set pending command to 'i'
                    editor.set_pending_operator(operator);
                    editor.set_pending_command('i');
                    return Ok(());
                }
                // Text objects with 'a' (around)
                (_, KeyCode::Char('a')) => {
                    // Restore operator and set pending command to 'a'
                    editor.set_pending_operator(operator);
                    editor.set_pending_command('a');
                    return Ok(());
                }
                _ => {
                    // Unknown operator+motion combo
                    editor.clear_count();
                }
            }
        }

        // Handle text objects after 'i' or 'a' with an operator
        if let Some(text_obj_type) = editor.pending_command() {
            if (text_obj_type == 'i' || text_obj_type == 'a') && editor.pending_operator().is_some() {
                let operator = editor.pending_operator().unwrap();
                editor.clear_pending_command();
                editor.clear_pending_operator();
                editor.clear_count();

                let result = match key_event.code {
                    KeyCode::Char('w') => {
                        // iw or aw - word
                        if text_obj_type == 'i' {
                            TextObjects::inner_word(editor.buffer())
                        } else {
                            TextObjects::around_word(editor.buffer())
                        }
                    }
                    KeyCode::Char('"') | KeyCode::Char('\'') | KeyCode::Char('`') => {
                        // i" or a" - quoted string
                        let quote = match key_event.code {
                            KeyCode::Char(c) => c,
                            _ => unreachable!(),
                        };
                        TextObjects::quoted_string(editor.buffer(), quote, text_obj_type == 'a')
                    }
                    KeyCode::Char('(') | KeyCode::Char(')') | KeyCode::Char('b') => {
                        // i( or a( or ib or ab - parentheses
                        TextObjects::paired_delimiters(editor.buffer(), '(', ')', text_obj_type == 'a')
                    }
                    KeyCode::Char('[') | KeyCode::Char(']') => {
                        // i[ or a[ - brackets
                        TextObjects::paired_delimiters(editor.buffer(), '[', ']', text_obj_type == 'a')
                    }
                    KeyCode::Char('{') | KeyCode::Char('}') | KeyCode::Char('B') => {
                        // i{ or a{ or iB or aB - braces
                        TextObjects::paired_delimiters(editor.buffer(), '{', '}', text_obj_type == 'a')
                    }
                    _ => {
                        // Unknown text object
                        return Ok(());
                    }
                };

                if let Some(range) = result {
                    match operator {
                        Operator::Delete => {
                            editor.save_undo_state();
                            let deleted = TextObjects::delete_range(editor.buffer_mut(), range)?;
                            editor.registers_mut().delete(deleted);
                        }
                        Operator::Yank => {
                            let yanked = TextObjects::yank_range(editor.buffer(), range)?;
                            editor.registers_mut().yank(yanked);
                        }
                        Operator::Change => {
                            editor.save_undo_state();
                            let deleted = TextObjects::delete_range(editor.buffer_mut(), range)?;
                            editor.registers_mut().delete(deleted);
                            editor.set_mode(Mode::Insert);
                        }
                    }
                }

                return Ok(());
            }
        }

        // Handle pending command (like 'g' waiting for second character)
        if let Some(pending) = editor.pending_command() {
            editor.clear_pending_command();
            match (pending, key_event.code) {
                ('g', KeyCode::Char('g')) => {
                    // gg - go to first line
                    let target_line = editor.count().unwrap_or(1).saturating_sub(1);
                    editor.buffer_mut().cursor_mut().set_position(target_line, 0);
                    editor.clear_count();
                    return Ok(());
                }
                ('m', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
                    // m{a-z} - set mark
                    editor.set_mark(ch);
                    return Ok(());
                }
                ('\'', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
                    // '{a-z} - jump to mark line
                    editor.add_jump(); // Add current position to jump list before jumping
                    editor.jump_to_mark_line(ch);
                    return Ok(());
                }
                ('`', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
                    // `{a-z} - jump to mark exact position
                    editor.add_jump(); // Add current position to jump list before jumping
                    editor.jump_to_mark(ch);
                    return Ok(());
                }
                ('q', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
                    // q{a-z} - start recording macro to register
                    editor.start_macro_recording(ch);
                    return Ok(());
                }
                ('@', KeyCode::Char(ch)) if ch.is_ascii_lowercase() => {
                    // @{a-z} - play back macro from register
                    if let Some(events) = editor.get_macro(ch) {
                        // Clone the events to avoid borrow checker issues
                        let events = events.clone();
                        for event in events {
                            Self::handle_key_event(editor, event)?;
                        }
                    }
                    return Ok(());
                }
                _ => {
                    // Unknown command sequence, clear and continue
                    editor.clear_count();
                }
            }
        }

        match key_event.code {
            // Quit
            KeyCode::Char('q') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.quit();
            }
            // Jump forward (Ctrl-I) - must come before 'i'
            KeyCode::Char('i') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.jump_forward();
            }
            // Jump back (Ctrl-O) - must come before 'o'
            KeyCode::Char('o') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.jump_back();
            }
            // Enter Insert mode
            KeyCode::Char('i') => {
                editor.save_undo_state();
                editor.set_mode(Mode::Insert);
            }
            KeyCode::Char('a') => {
                editor.save_undo_state();
                editor.set_mode(Mode::Insert);
                // Move cursor right (insert after)
                let cursor = editor.buffer_mut().cursor_mut();
                cursor.move_right(1);
            }
            KeyCode::Char('I') => {
                editor.save_undo_state();
                editor.set_mode(Mode::Insert);
                // Move to start of line
                let cursor = editor.buffer_mut().cursor_mut();
                cursor.set_col(0);
            }
            KeyCode::Char('A') => {
                editor.save_undo_state();
                editor.set_mode(Mode::Insert);
                // Move to end of line
                let line_idx = editor.buffer().cursor().line();
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_len = line.trim_end_matches('\n').chars().count();
                    editor.buffer_mut().cursor_mut().set_col(line_len);
                }
            }
            KeyCode::Char('o') => {
                editor.save_undo_state();
                editor.set_mode(Mode::Insert);
                // Insert new line below and move to it
                Self::insert_line_below(editor)?;
            }
            KeyCode::Char('O') => {
                editor.save_undo_state();
                editor.set_mode(Mode::Insert);
                // Insert new line above and move to it
                Self::insert_line_above(editor)?;
            }
            // Motion commands
            KeyCode::Char('h') | KeyCode::Left => {
                Self::move_left(editor);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                Self::move_down(editor);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                Self::move_up(editor);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                Self::move_right(editor);
            }
            // Line motions
            KeyCode::Char('0') => {
                editor.buffer_mut().cursor_mut().set_col(0);
                editor.clear_count();
            }
            KeyCode::Char('$') => {
                let line_idx = editor.buffer().cursor().line();
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_len = line.trim_end_matches('\n').chars().count();
                    let col = if line_len > 0 { line_len - 1 } else { 0 };
                    editor.buffer_mut().cursor_mut().set_col(col);
                }
                editor.clear_count();
            }
            // Count prefix
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let digit = c.to_digit(10).unwrap() as usize;
                // 0 is a motion, not a count prefix
                if digit != 0 || editor.count().is_some() {
                    editor.append_count(digit);
                }
            }
            // Enter Command mode
            KeyCode::Char(':') => {
                editor.clear_command_line();
                editor.set_mode(Mode::Command);
            }
            // Enter Search mode (forward)
            KeyCode::Char('/') => {
                editor.clear_search_buffer();
                editor.set_search_forward(true);
                editor.set_mode(Mode::Search);
            }
            // Enter Search mode (backward)
            KeyCode::Char('?') => {
                editor.clear_search_buffer();
                editor.set_search_forward(false);
                editor.set_mode(Mode::Search);
            }
            // Search next
            KeyCode::Char('n') => {
                editor.search_next();
            }
            // Search previous
            KeyCode::Char('N') => {
                editor.search_prev();
            }
            // Set mark (m followed by letter)
            KeyCode::Char('m') => {
                editor.set_pending_command('m');
            }
            // Jump to mark line (' followed by letter)
            KeyCode::Char('\'') => {
                editor.set_pending_command('\'');
            }
            // Jump to mark exact position (` followed by letter)
            KeyCode::Char('`') => {
                editor.set_pending_command('`');
            }
            // Start/stop macro recording (q followed by register, or q to stop)
            KeyCode::Char('q') => {
                if editor.is_recording_macro() {
                    // Stop recording
                    editor.stop_macro_recording();
                } else {
                    // Start recording - set pending command to wait for register
                    editor.set_pending_command('q');
                }
            }
            // Play back macro (@ followed by register)
            KeyCode::Char('@') => {
                editor.set_pending_command('@');
            }
            // Enter Visual mode
            KeyCode::Char('v') => {
                let cursor = editor.buffer().cursor();
                editor.set_visual_start(cursor.line(), cursor.col());
                editor.set_mode(Mode::Visual);
            }
            KeyCode::Char('V') => {
                let cursor = editor.buffer().cursor();
                editor.set_visual_start(cursor.line(), 0);
                editor.set_mode(Mode::VisualLine);
            }
            // Word motions
            KeyCode::Char('w') => {
                let count = editor.effective_count();
                Motions::word_forward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('W') => {
                let count = editor.effective_count();
                Motions::word_forward_big(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('b') => {
                let count = editor.effective_count();
                Motions::word_backward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('B') => {
                let count = editor.effective_count();
                Motions::word_backward_big(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('e') => {
                let count = editor.effective_count();
                Motions::word_end_forward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('E') => {
                let count = editor.effective_count();
                Motions::word_end_forward_big(editor.buffer_mut(), count);
                editor.clear_count();
            }
            // File motions
            KeyCode::Char('g') => {
                // Set pending command to wait for second 'g'
                editor.set_pending_command('g');
            }
            KeyCode::Char('G') => {
                // G - go to last line (or line specified by count)
                let target_line = if let Some(count) = editor.count() {
                    count.saturating_sub(1)
                } else {
                    editor.buffer().line_count().saturating_sub(1)
                };
                editor.buffer_mut().cursor_mut().set_position(target_line, 0);
                editor.clear_count();
            }
            // Operators
            KeyCode::Char('d') => {
                editor.set_pending_operator(Operator::Delete);
            }
            KeyCode::Char('y') => {
                editor.set_pending_operator(Operator::Yank);
            }
            KeyCode::Char('c') => {
                editor.set_pending_operator(Operator::Change);
            }
            // Simple delete commands
            KeyCode::Char('x') => {
                // x - delete character under cursor
                editor.save_undo_state();
                let count = editor.effective_count();
                let deleted = Operators::delete_char(editor.buffer_mut(), count)?;
                editor.registers_mut().delete(deleted);
                editor.clear_count();
            }
            KeyCode::Char('D') => {
                // D - delete to end of line
                editor.save_undo_state();
                let deleted = Operators::delete_to_end_of_line(editor.buffer_mut())?;
                editor.registers_mut().delete(deleted);
                editor.clear_count();
            }
            KeyCode::Char('C') => {
                // C - change to end of line
                editor.save_undo_state();
                let deleted = Operators::delete_to_end_of_line(editor.buffer_mut())?;
                editor.registers_mut().delete(deleted);
                editor.clear_count();
                editor.set_mode(Mode::Insert);
            }
            // Paste
            KeyCode::Char('p') => {
                // p - paste after cursor
                editor.save_undo_state();
                Self::paste_after(editor)?;
                editor.clear_count();
            }
            KeyCode::Char('P') => {
                // P - paste before cursor
                editor.save_undo_state();
                Self::paste_before(editor)?;
                editor.clear_count();
            }
            // Undo/Redo
            KeyCode::Char('u') => {
                // u - undo
                editor.undo();
                editor.clear_count();
            }
            KeyCode::Char('r') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+R - redo
                editor.redo();
                editor.clear_count();
            }
            _ => {
                // Clear count on unrecognized key
                editor.clear_count();
            }
        }
        Ok(())
    }

    /// Handles input in Insert mode
    fn handle_insert_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Esc => {
                editor.set_mode(Mode::Normal);
                // Move cursor left when exiting insert mode (unless at column 0)
                let cursor = editor.buffer_mut().cursor_mut();
                if cursor.col() > 0 {
                    cursor.move_left(1);
                }
            }
            KeyCode::Char(c) => {
                Self::insert_char(editor, c)?;
            }
            KeyCode::Enter => {
                Self::insert_newline(editor)?;
            }
            KeyCode::Backspace => {
                Self::delete_char_before_cursor(editor)?;
            }
            KeyCode::Left => {
                let cursor = editor.buffer_mut().cursor_mut();
                if cursor.col() > 0 {
                    cursor.move_left(1);
                }
            }
            KeyCode::Right => {
                Self::move_right(editor);
            }
            KeyCode::Up => {
                Self::move_up(editor);
            }
            KeyCode::Down => {
                Self::move_down(editor);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles input in Visual mode
    fn handle_visual_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Esc => {
                editor.clear_visual_start();
                editor.set_mode(Mode::Normal);
            }
            // Motion keys work in visual mode too
            KeyCode::Char('h') | KeyCode::Left => {
                Self::move_left(editor);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                Self::move_down(editor);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                Self::move_up(editor);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                Self::move_right(editor);
            }
            KeyCode::Char('w') => {
                let count = editor.effective_count();
                Motions::word_forward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('b') => {
                let count = editor.effective_count();
                Motions::word_backward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('e') => {
                let count = editor.effective_count();
                Motions::word_end_forward(editor.buffer_mut(), count);
                editor.clear_count();
            }
            KeyCode::Char('0') => {
                editor.buffer_mut().cursor_mut().set_col(0);
            }
            KeyCode::Char('$') => {
                let line_idx = editor.buffer().cursor().line();
                if let Some(line) = editor.buffer().line(line_idx) {
                    let line_len = line.trim_end_matches('\n').chars().count();
                    let col = if line_len > 0 { line_len - 1 } else { 0 };
                    editor.buffer_mut().cursor_mut().set_col(col);
                }
            }
            // Delete selection
            KeyCode::Char('d') | KeyCode::Char('x') => {
                Self::delete_visual_selection(editor)?;
                editor.clear_visual_start();
                editor.set_mode(Mode::Normal);
            }
            // Yank selection
            KeyCode::Char('y') => {
                Self::yank_visual_selection(editor)?;
                editor.clear_visual_start();
                editor.set_mode(Mode::Normal);
            }
            // Change selection
            KeyCode::Char('c') => {
                Self::delete_visual_selection(editor)?;
                editor.clear_visual_start();
                editor.set_mode(Mode::Insert);
            }
            // Switch to other visual modes
            KeyCode::Char('v') => {
                editor.set_mode(Mode::Visual);
            }
            KeyCode::Char('V') => {
                let cursor = editor.buffer().cursor();
                editor.set_visual_start(cursor.line(), 0);
                editor.set_mode(Mode::VisualLine);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles input in Command mode
    fn handle_command_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Char(ch) => {
                // Add character to command line
                editor.append_to_command_line(ch);
            }
            KeyCode::Backspace => {
                // Remove last character from command line
                editor.backspace_command_line();
            }
            KeyCode::Enter => {
                // Execute the command
                Self::execute_command(editor)?;
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

    /// Executes a command from the command line
    fn execute_command(editor: &mut Editor) -> Result<()> {
        let command = editor.command_line().trim();

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
            }
            return Ok(());
        }

        // Handle commands without arguments
        match command {
            "q" | "quit" => {
                // Quit without checking for modifications
                if editor.buffer().is_modified() {
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
            "w" | "write" => {
                // Save to current file
                editor.buffer_mut().save()?;
            }
            "wq" | "x" => {
                // Write and quit
                editor.buffer_mut().save()?;
                editor.quit();
            }
            "wq!" => {
                // Force write and quit
                editor.buffer_mut().save()?;
                editor.quit();
            }
            _ => {
                // Unknown command - for now just ignore
            }
        }

        Ok(())
    }

    /// Handles input in Search mode
    fn handle_search_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
        match key_event.code {
            KeyCode::Char(ch) => {
                // Add character to search buffer
                editor.append_to_search_buffer(ch);
            }
            KeyCode::Backspace => {
                // Remove last character from search buffer
                editor.backspace_search_buffer();
            }
            KeyCode::Enter => {
                // Execute the search
                editor.execute_search();
                editor.set_mode(Mode::Normal);
            }
            KeyCode::Esc => {
                // Cancel search mode
                editor.clear_search_buffer();
                editor.set_mode(Mode::Normal);
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles input in Replace mode
    fn handle_replace_mode(_editor: &mut Editor, _key_event: KeyEvent) -> Result<()> {
        // Placeholder for replace mode
        Ok(())
    }

    /// Polls for the next event
    pub fn poll_event() -> Result<Option<Event>> {
        if event::poll(std::time::Duration::from_millis(100))? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }

    // Helper methods for cursor movement and editing

    fn move_left(editor: &mut Editor) {
        let count = editor.effective_count();
        let cursor = editor.buffer_mut().cursor_mut();
        if cursor.col() >= count {
            cursor.move_left(count);
        } else {
            cursor.set_col(0);
        }
        editor.clear_count();
    }

    fn move_right(editor: &mut Editor) {
        let count = editor.effective_count();
        let line_idx = editor.buffer().cursor().line();
        if let Some(line) = editor.buffer().line(line_idx) {
            let line_len = line.trim_end_matches('\n').chars().count();
            let cursor = editor.buffer_mut().cursor_mut();
            let new_col = (cursor.col() + count).min(line_len.saturating_sub(1).max(0));
            cursor.set_col(new_col);
        }
        editor.clear_count();
    }

    fn move_up(editor: &mut Editor) {
        let count = editor.effective_count();
        let cursor = editor.buffer_mut().cursor_mut();
        cursor.move_up(count);
        Self::clamp_cursor_to_line(editor);
        editor.clear_count();
    }

    fn move_down(editor: &mut Editor) {
        let count = editor.effective_count();
        let max_line = editor.buffer().line_count().saturating_sub(1);
        let cursor = editor.buffer_mut().cursor_mut();
        let new_line = (cursor.line() + count).min(max_line);
        cursor.set_line(new_line);
        Self::clamp_cursor_to_line(editor);
        editor.clear_count();
    }

    fn clamp_cursor_to_line(editor: &mut Editor) {
        let line_idx = editor.buffer().cursor().line();
        if let Some(line) = editor.buffer().line(line_idx) {
            let line_len = line.trim_end_matches('\n').chars().count();
            let cursor = editor.buffer_mut().cursor_mut();
            if cursor.col() >= line_len {
                let new_col = if line_len > 0 { line_len - 1 } else { 0 };
                cursor.set_col(new_col);
            }
        }
    }

    fn insert_char(editor: &mut Editor, c: char) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        // Calculate byte position
        let line_start = editor.buffer().rope().line_to_char(line_idx);
        let insert_pos = line_start + col;

        // Insert the character
        editor.buffer_mut().rope_mut().insert_char(insert_pos, c);

        // Move cursor right
        editor.buffer_mut().cursor_mut().move_right(1);

        Ok(())
    }

    fn insert_newline(editor: &mut Editor) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        // Calculate byte position
        let line_start = editor.buffer().rope().line_to_char(line_idx);
        let insert_pos = line_start + col;

        // Insert newline
        editor.buffer_mut().rope_mut().insert_char(insert_pos, '\n');

        // Move cursor to next line, column 0
        editor.buffer_mut().cursor_mut().set_position(line_idx + 1, 0);

        Ok(())
    }

    fn delete_char_before_cursor(editor: &mut Editor) -> Result<()> {
        let cursor = editor.buffer().cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        if col == 0 && line_idx == 0 {
            // At start of buffer, nothing to delete
            return Ok(());
        }

        let line_start = editor.buffer().rope().line_to_char(line_idx);

        if col == 0 {
            // Delete newline at end of previous line
            let delete_pos = line_start - 1;
            editor.buffer_mut().rope_mut().remove(delete_pos..delete_pos + 1);

            // Move cursor to end of previous line
            if let Some(prev_line) = editor.buffer().line(line_idx - 1) {
                let prev_line_len = prev_line.trim_end_matches('\n').chars().count();
                editor.buffer_mut().cursor_mut().set_position(line_idx - 1, prev_line_len);
            }
        } else {
            // Delete character before cursor
            let delete_pos = line_start + col - 1;
            editor.buffer_mut().rope_mut().remove(delete_pos..delete_pos + 1);
            editor.buffer_mut().cursor_mut().move_left(1);
        }

        Ok(())
    }

    fn insert_line_below(editor: &mut Editor) -> Result<()> {
        let line_idx = editor.buffer().cursor().line();
        let line_start = editor.buffer().rope().line_to_char(line_idx);
        let line_len = editor.buffer().rope().line(line_idx).len_chars();
        let insert_pos = line_start + line_len;

        // Check if line already ends with newline
        let line_text = editor.buffer().line(line_idx).unwrap_or_default();
        if !line_text.ends_with('\n') {
            editor.buffer_mut().rope_mut().insert_char(insert_pos, '\n');
        }
        editor.buffer_mut().rope_mut().insert_char(insert_pos + 1, '\n');

        editor.buffer_mut().cursor_mut().set_position(line_idx + 1, 0);
        Ok(())
    }

    fn insert_line_above(editor: &mut Editor) -> Result<()> {
        let line_idx = editor.buffer().cursor().line();
        let line_start = editor.buffer().rope().line_to_char(line_idx);

        editor.buffer_mut().rope_mut().insert_char(line_start, '\n');
        // Cursor stays at same line index since we inserted above
        editor.buffer_mut().cursor_mut().set_col(0);
        Ok(())
    }

    fn paste_after(editor: &mut Editor) -> Result<()> {
        let text = editor.registers().get_default().to_string();
        if text.is_empty() {
            return Ok(());
        }

        let cursor = editor.buffer().cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        // Check if text contains newline (line paste vs character paste)
        if text.contains('\n') {
            // Line paste - insert after current line
            let line_start = editor.buffer().rope().line_to_char(line_idx);
            let line_len = editor.buffer().rope().line(line_idx).len_chars();
            let insert_pos = line_start + line_len;

            editor.buffer_mut().rope_mut().insert(insert_pos, &text);

            // Move cursor to start of pasted text (next line)
            editor.buffer_mut().cursor_mut().set_position(line_idx + 1, 0);
        } else {
            // Character paste - insert after cursor
            let line_start = editor.buffer().rope().line_to_char(line_idx);
            let insert_pos = line_start + col + 1;

            editor.buffer_mut().rope_mut().insert(insert_pos, &text);

            // Move cursor to last character of pasted text
            editor.buffer_mut().cursor_mut().set_col(col + text.chars().count());
        }

        Ok(())
    }

    fn paste_before(editor: &mut Editor) -> Result<()> {
        let text = editor.registers().get_default().to_string();
        if text.is_empty() {
            return Ok(());
        }

        let cursor = editor.buffer().cursor();
        let line_idx = cursor.line();
        let col = cursor.col();

        // Check if text contains newline (line paste vs character paste)
        if text.contains('\n') {
            // Line paste - insert before current line
            let line_start = editor.buffer().rope().line_to_char(line_idx);

            editor.buffer_mut().rope_mut().insert(line_start, &text);

            // Move cursor to start of pasted text
            editor.buffer_mut().cursor_mut().set_col(0);
        } else {
            // Character paste - insert at cursor
            let line_start = editor.buffer().rope().line_to_char(line_idx);
            let insert_pos = line_start + col;

            editor.buffer_mut().rope_mut().insert(insert_pos, &text);

            // Move cursor to last character of pasted text
            editor.buffer_mut().cursor_mut().set_col(col + text.chars().count().saturating_sub(1));
        }

        Ok(())
    }

    fn delete_visual_selection(editor: &mut Editor) -> Result<()> {
        let mode = editor.mode();

        if let Some(((start_line, start_col), (end_line, end_col))) = editor.visual_selection() {
            match mode {
                Mode::VisualLine => {
                    // Delete entire lines
                    let start_char = editor.buffer().rope().line_to_char(start_line);
                    let end_char = if end_line + 1 < editor.buffer().line_count() {
                        editor.buffer().rope().line_to_char(end_line + 1)
                    } else {
                        editor.buffer().rope().len_chars()
                    };

                    let deleted = editor.buffer().rope().slice(start_char..end_char).to_string();
                    editor.buffer_mut().rope_mut().remove(start_char..end_char);
                    editor.registers_mut().delete(deleted);

                    // Position cursor at start of selection
                    let new_line = start_line.min(editor.buffer().line_count().saturating_sub(1));
                    editor.buffer_mut().cursor_mut().set_position(new_line, 0);
                }
                _ => {
                    // Character-wise visual mode
                    let start_char = editor.buffer().rope().line_to_char(start_line) + start_col;
                    let end_char = editor.buffer().rope().line_to_char(end_line) + end_col + 1;

                    let deleted = editor.buffer().rope().slice(start_char..end_char).to_string();
                    editor.buffer_mut().rope_mut().remove(start_char..end_char);
                    editor.registers_mut().delete(deleted);

                    // Position cursor at start of selection
                    editor.buffer_mut().cursor_mut().set_position(start_line, start_col);
                }
            }
        }

        Ok(())
    }

    fn yank_visual_selection(editor: &mut Editor) -> Result<()> {
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

                    let yanked = editor.buffer().rope().slice(start_char..end_char).to_string();
                    editor.registers_mut().yank(yanked);
                }
                _ => {
                    // Character-wise visual mode
                    let start_char = editor.buffer().rope().line_to_char(start_line) + start_col;
                    let end_char = editor.buffer().rope().line_to_char(end_line) + end_col + 1;

                    let yanked = editor.buffer().rope().slice(start_char..end_char).to_string();
                    editor.registers_mut().yank(yanked);
                }
            }
        }

        Ok(())
    }
}
