//! Applying goto-definition, implementation and type-definition results:
//! jumping to the location, opening it in a new tab, or fetching a virtual
//! document.

use super::*;

impl Editor {
    /// Poll goto-definition, goto-implementation, and goto-type-definition
    /// slots (all use `Slot<GotoLocationResult>`).
    pub(super) fn poll_goto_slots(&mut self) -> bool {
        let mut changed = false;

        // Helper: process a GotoLocationResult from any goto slot.
        // We poll each slot with a 10-second timeout to match the old behaviour.
        let timeout = std::time::Duration::from_secs(10);

        if let Some(result) = self.lsp.slots.goto_definition.poll_with_timeout(timeout) {
            changed |= self.handle_goto_slot_result(result, "Definition", "LSP-DEFINITION");
        }

        if let Some(result) = self
            .lsp
            .slots
            .goto_implementation
            .poll_with_timeout(timeout)
        {
            changed |= self.handle_goto_slot_result(result, "Implementation", "LSP-IMPLEMENTATION");
        }

        if let Some(result) = self
            .lsp
            .slots
            .goto_type_definition
            .poll_with_timeout(timeout)
        {
            changed |= self.handle_goto_slot_result(result, "Type", "LSP-TYPE");
        }

        if let Some(result) = self.lsp.slots.virtual_document.poll_with_timeout(timeout) {
            changed |= self.open_virtual_document_result(result);
        }

        changed
    }

    /// A definition in a document with no `file:` location: fetch its text
    /// from the server (`workspace/textDocumentContent`) and show it in a
    /// read-only buffer (OV-00470).
    fn request_virtual_document(&mut self, location: lsp_types::Location) -> bool {
        let uri_text = location.uri.as_str().to_string();
        let language_id = self
            .buffer()
            .file_path()
            .and_then(|path| self.language_id_for_path(path));
        let (Some(lsp), Some(language_id)) = (self.lsp.state.lsp_manager.clone(), language_id)
        else {
            self.set_lsp_status(format!("Cannot open {uri_text}: no language server"));
            return false;
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let uri = location.uri.clone();
        let range = location.range;
        let origin = self.request_origin();
        let task = tokio::spawn(async move {
            let result = lsp.text_document_content(&uri, &language_id).await;
            let _ = tx.send(
                result.map(|text| crate::editor::lsp_slot::VirtualDocumentResult {
                    uri,
                    text,
                    range,
                    origin,
                }),
            );
        });
        self.lsp.slots.virtual_document.fire(task, rx);
        self.set_lsp_status(format!("Fetching {uri_text}..."));
        false
    }

    pub(super) fn open_virtual_document_result(
        &mut self,
        result: anyhow::Result<crate::editor::lsp_slot::VirtualDocumentResult>,
    ) -> bool {
        let document = match result {
            Ok(document) => document,
            Err(error) => {
                self.set_lsp_status(format!("Cannot open document: {error}"));
                return false;
            }
        };
        if !self.request_origin_is_current(&document.origin) {
            self.set_lsp_status("Definition discarded: the editor moved on".to_string());
            return false;
        }
        let title = document
            .uri
            .as_str()
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or("document")
            .to_string();
        self.open_scratch_buffer(&title, &document.text);
        self.buffer_mut().set_modifiable(false);
        let line = document.range.start.line as usize;
        let col = self.utf16_to_grapheme_col(line, document.range.start.character);
        self.buffer_mut()
            .cursor_mut()
            .set_position(line, crate::unicode::GraphemeCol(col));
        self.buffer_mut().validate_cursor_position();
        self.center_cursor_in_viewport();
        self.set_lsp_status(format!("{title}: {}", document.uri.as_str()));
        true
    }

    /// Apply the result from a goto slot — shared logic for definition,
    /// implementation, and type-definition.
    fn handle_goto_slot_result(
        &mut self,
        result: anyhow::Result<crate::editor::lsp_slot::GotoLocationResult>,
        label: &str,
        log_tag: &str,
    ) -> bool {
        match result {
            Ok(goto) if !self.request_origin_is_current(&goto.origin) => {
                crate::lsp_debug!(log_tag, "Dropping {} response: editor moved on", label);
                self.set_lsp_status(format!("{label} discarded: the editor moved on"));
                false
            }
            Ok(goto) => {
                // Reuse the existing handle_location_result_raw logic
                self.handle_goto_location(goto.location, label, log_tag, goto.new_tab)
            }
            Err(e) => {
                crate::lsp_debug!(log_tag, "{} request failed: {:?}", label, e);
                self.set_lsp_status(format!("{} failed: {}", label, e));
                false
            }
        }
    }

    /// Navigate to a location returned by a goto LSP request.
    pub(super) fn handle_goto_location(
        &mut self,
        location: Option<lsp_types::Location>,
        label: &str,
        log_tag: &str,
        new_tab: bool,
    ) -> bool {
        match location {
            Some(location) => {
                crate::lsp_debug!(log_tag, "Received {} response", label.to_lowercase());

                let Some(path) = crate::lsp::uri_to_file_path(&location.uri) else {
                    if location
                        .uri
                        .scheme()
                        .is_some_and(|scheme| scheme.as_str() != "file")
                    {
                        return self.request_virtual_document(location);
                    }
                    self.set_lsp_status("Invalid file path in LSP response".to_string());
                    return false;
                };

                let target_line = location.range.start.line as usize;
                let target_character = location.range.start.character;

                self.push_tag();

                if new_tab {
                    let origin = self.tab_page_manager.current_tab().id();
                    // A file that is already open shows up as it is, unsaved
                    // changes included: loading it again would duplicate it.
                    if let Some(index) = self.find_buffer_by_path(&path.to_string_lossy()) {
                        self.new_tab_for_existing_buffer(index);
                        self.tab_page_manager.current_tab_mut().definition_origin = Some(origin);
                        if let Some(path) = self.buffer().file_path() {
                            self.registers.set_current_file(path.to_string());
                        }
                    } else {
                        match crate::buffer::Buffer::load_file(&path) {
                            Ok(mut buffer) => {
                                self.new_tab_for_definition();
                                let modeline =
                                    crate::modeline::Modeline::parse(&buffer.rope().to_string());
                                self.initialize_buffer_indent_options(&mut buffer);
                                self.request_buffer_git_status(&buffer);
                                super::buffer_manager::mark_library_source_read_only(&mut buffer);
                                self.buffers[self.current_buffer_index] = buffer;
                                if let Some(modeline) = modeline.as_ref() {
                                    self.apply_modeline(modeline);
                                }
                                // The replacement buffer has a fresh id; repoint
                                // the tab at it
                                self.sync_current_tab_buffer();
                                self.tab_page_manager.current_tab_mut().definition_origin =
                                    Some(origin);
                                if let Some(path) = self.buffer().file_path() {
                                    self.registers.set_current_file(path.to_string());
                                }
                            }
                            Err(_) => {
                                self.set_lsp_status("Failed to open file".to_string());
                                return false;
                            }
                        }
                    }
                } else if self.buffer().file_path() != Some(path.to_string_lossy().as_ref())
                    && self.open_file(path.to_string_lossy().as_ref()).is_err()
                {
                    self.set_lsp_status("Failed to open file".to_string());
                    return false;
                }

                let target_col = self.utf16_to_grapheme_col(target_line, target_character);
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(target_line, crate::unicode::GraphemeCol(target_col));
                self.buffer_mut().validate_cursor_position();
                self.center_cursor_in_viewport();
                let actual_col = self.buffer().cursor().col();

                let suffix = if new_tab { " (new tab)" } else { "" };
                self.set_lsp_status(format!(
                    "{}{}: {}:{}:{}",
                    label,
                    suffix,
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    target_line + 1,
                    actual_col.0 + 1
                ));
                self.mark_dirty();
                true
            }
            None => {
                crate::lsp_debug!(log_tag, "No {} found", label.to_lowercase());
                self.set_lsp_status(format!("No {} found", label.to_lowercase()));
                false
            }
        }
    }

    /// Legacy handler — delegates to `handle_goto_location`.
    /// Kept only for test compatibility; will be removed once all callers migrate.
    #[cfg(test)]
    pub(super) fn handle_location_result(
        &mut self,
        result: anyhow::Result<Option<lsp_types::Location>>,
        label: &str,
        log_tag: &str,
        new_tab: bool,
    ) -> bool {
        match result {
            Ok(loc) => self.handle_goto_location(loc, label, log_tag, new_tab),
            Err(e) => {
                crate::lsp_debug!(log_tag, "{} request failed: {:?}", label, e);
                self.set_lsp_status(format!("{} failed: {}", label, e));
                false
            }
        }
    }
}
