//! Register access, yank/delete into registers and paste events.

use super::{CursorPos, Editor, RegisterManager, RegisterType};
use crate::mode::Mode;
use anyhow::Result;

impl Editor {
    /// Gets a reference to the registers
    pub fn registers(&self) -> &RegisterManager {
        &self.registers
    }

    /// Gets a mutable reference to the registers
    pub fn registers_mut(&mut self) -> &mut RegisterManager {
        &mut self.registers
    }

    /// Gets the pending register for next operation
    pub fn pending_register(&self) -> Option<char> {
        self.input.pending_register
    }

    /// Sets the pending register for next operation
    pub fn set_pending_register(&mut self, reg: char) {
        self.input.pending_register = Some(reg);
    }

    /// Clears the pending register
    pub fn clear_pending_register(&mut self) {
        self.input.pending_register = None;
    }

    /// Yanks text to the appropriate register (pending_register or default)
    pub fn yank_to_register(&mut self, text: String) {
        self.yank_to_register_with_type(text, RegisterType::Character);
    }

    /// Yanks text to the appropriate register with explicit type
    pub fn yank_to_register_with_type(&mut self, text: String, reg_type: RegisterType) {
        if let Some(reg) = self.input.pending_register {
            self.input.pending_register = None;
            match reg {
                '_' => return, // black hole register: discard
                r if RegisterManager::is_read_only(r) => {
                    // Read-only registers: silently use default behavior
                    self.registers.yank_with_type(text.clone(), reg_type);
                    if !self.options.clipboard.is_empty() {
                        self.registers.set_clipboard(text);
                    }
                    return;
                }
                '+' | '*' => {
                    self.registers.set_with_type(Some(reg), text, reg_type);
                    return;
                }
                r if r.is_ascii_digit() => {
                    // Preserve numbered-register delete routing. Numbered
                    // registers are managed as a rotating history rather than
                    // ordinary named-register slots.
                    self.registers.delete_with_type(text.clone(), reg_type);
                }
                _ => {
                    self.registers
                        .set_with_type(Some(reg), text.clone(), reg_type);
                    // The unnamed register follows the named one (all of it
                    // after an append) but the yank register `0` is left alone.
                    self.unnamed_follows_register(reg);
                }
            }
        } else {
            self.registers.yank_with_type(text.clone(), reg_type);
        }
        // Sync to system clipboard when clipboard option is set and no explicit register was used
        if !self.options.clipboard.is_empty() {
            self.registers.set_clipboard(text);
        }
    }

    /// Points the unnamed register at what `register` now holds.
    fn unnamed_follows_register(&mut self, register: char) {
        let (text, reg_type) = self.registers.get_with_type(Some(register));
        self.registers.set_with_type(None, text, reg_type);
    }

    /// Deletes text and stores in the appropriate register (pending_register or default)
    pub fn delete_to_register(&mut self, text: String) {
        self.delete_to_register_with_type(text, RegisterType::Character);
    }

    /// Deletes text and stores in the appropriate register with explicit type
    pub fn delete_to_register_with_type(&mut self, text: String, reg_type: RegisterType) {
        self.buffer_mut().change_manager_mut().last_repeat_register = self.input.pending_register;
        if let Some(reg) = self.input.pending_register {
            self.input.pending_register = None;
            match reg {
                '_' => return, // black hole register: discard
                r if RegisterManager::is_read_only(r) => {
                    // Read-only registers: silently use default behavior
                    self.registers.delete_with_type(text.clone(), reg_type);
                    if !self.options.clipboard.is_empty() {
                        self.registers.set_clipboard(text);
                    }
                    return;
                }
                '+' | '*' => {
                    self.registers.set_with_type(Some(reg), text, reg_type);
                    return;
                }
                _ => {
                    self.registers
                        .set_with_type(Some(reg), text.clone(), reg_type);
                    // Explicit-register deletes also update unnamed, but do not
                    // rotate numbered or small-delete registers.
                    self.unnamed_follows_register(reg);
                }
            }
        } else {
            self.registers.delete_with_type(text.clone(), reg_type);
        }
        // Sync to system clipboard when clipboard option is set and no explicit register was used
        if !self.options.clipboard.is_empty() {
            self.registers.set_clipboard(text);
        }
    }

    /// Gets text from the appropriate register (pending_register or default)
    pub fn get_from_register(&mut self) -> String {
        let text = if let Some(reg) = self.input.pending_register {
            match reg {
                '_' => String::new(), // black hole register: always empty
                '+' | '*' => self.registers.get_clipboard(),
                _ => self.registers.get(Some(reg)),
            }
        } else if !self.options.clipboard.is_empty() {
            // When clipboard option is set, read from system clipboard
            self.registers.get_clipboard()
        } else {
            self.registers.get_default().to_string()
        };
        self.input.pending_register = None;
        text
    }

    /// Gets text and type from the appropriate register (pending_register or default)
    pub fn get_from_register_with_type(&mut self) -> (String, RegisterType) {
        let (text, reg_type) = if let Some(reg) = self.input.pending_register {
            match reg {
                '_' => (String::new(), RegisterType::Character), // black hole: always empty
                '+' | '*' => {
                    let clipboard_text = self.registers.get_clipboard();
                    (clipboard_text, RegisterType::Character)
                }
                _ => self.registers.get_with_type(Some(reg)),
            }
        } else if !self.options.clipboard.is_empty() {
            // When clipboard option is set, read from system clipboard
            // Use Character type since system clipboard doesn't carry type info
            let clipboard_text = self.registers.get_clipboard();
            // Check if the unnamed register has the same text - if so, use its type
            let (default_text, default_type) = self.registers.get_default_with_type();
            if default_text == clipboard_text {
                (clipboard_text, default_type)
            } else {
                // Clipboard has different content (from external paste), treat as character
                (clipboard_text, RegisterType::Character)
            }
        } else {
            let (t, rt) = self.registers.get_default_with_type();
            (t.to_string(), rt)
        };
        self.input.pending_register = None;
        (text, reg_type)
    }

    /// Handles a bracketed paste event (for all supported modes, including chat input).
    pub fn handle_paste_event(&mut self, text: &str) -> Result<()> {
        if self.is_pseudocode_buffer() && !matches!(self.mode(), Mode::Command | Mode::Search) {
            self.set_status_message("Pseudocode is a reading view; press Enter to edit source");
            return Ok(());
        }
        if text.is_empty() {
            return Ok(());
        }
        if self.has_codex_auth_dialog() {
            return Ok(());
        }
        if self.mode() == Mode::AiChat && self.ai_chat_has_exa_setup_dialog() {
            self.insert_exa_setup_text(text);
            return Ok(());
        }
        if self.mode() == Mode::AiChat && self.try_attach_dropped_chat_images(text)? {
            return Ok(());
        }

        // Strip CR variants so pasted CRLF/CR content doesn't leave `^M`
        // artifacts in the rope (OV-00250). Cheap when there are no CRs.
        let normalized = crate::buffer::normalize_for_buffer(text);
        let text = normalized.as_ref();

        match self.mode() {
            Mode::Insert => {
                // Insert pasted text at cursor position. The ambient insert
                // session captures the edit; the undo entry is pushed as a
                // single `Recorded` at `finalize_change_building`.
                let cursor = self.buffer().cursor();
                // Convert grapheme col to char col for buffer operations.
                let char_col = self.buffer().cursor_char_col();
                let line = cursor.line();
                let pasted = text.to_string();
                self.record_session_edit(|buf| {
                    buf.insert_text_at_positioning_cursor(line, char_col, &pasted)
                });
            }
            Mode::AiChat => {
                if let Some(chat) = self.ai_state.chat.as_mut() {
                    if matches!(chat.focus, crate::ai::chat_types::ChatFocus::TextInput) {
                        chat.input.insert_str(chat.input_cursor, text);
                        chat.input_cursor += text.len();
                    }
                }
            }
            Mode::Normal => {
                // Set unnamed register and paste after cursor as a recorded
                // undo entry. `.`-repeat runs through `RepeatAction::PasteAfter`
                // so the re-paste re-anchors to the current cursor (same
                // semantics the old `Change::InsertText::repeat` provided).
                self.registers.set(None, text.to_string());
                self.buffer_mut().change_manager_mut().last_repeat_register = None;
                let cursor = self.buffer().cursor();
                let cursor_before = CursorPos::new(cursor.line(), cursor.col());
                // Convert grapheme col to char col for buffer operations.
                let char_col = self.buffer().cursor_char_col();
                let line = cursor.line();
                let paste_col = char_col + 1;
                let pasted = text.to_string();
                let (_mutated, edits) = self
                    .buffer_mut()
                    .record(|buf| buf.insert_text_at_positioning_cursor(line, paste_col, &pasted));
                if !edits.is_empty() {
                    let cursor_after = self.cursor_position();
                    self.push_recorded_undo(edits, cursor_before, cursor_after);
                    self.set_repeat_action(crate::repeat_action::RepeatAction::PasteAfter {
                        count: 1,
                    });
                }
            }
            Mode::Command => {
                // Insert text into command buffer
                self.insert_into_command_line(text);
            }
            Mode::Search => {
                // Insert text into search buffer
                self.insert_into_search_buffer(text);
            }
            Mode::Picker => {
                if let Some(picker) = self.picker_mut() {
                    picker.insert_text(text);
                }
                self.mark_picker_query_changed();
            }
            _ => {
                // Visual modes: treat like normal mode paste
                self.registers.set(None, text.to_string());
            }
        }
        Ok(())
    }
}
