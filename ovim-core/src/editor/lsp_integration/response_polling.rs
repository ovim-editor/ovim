//! Polling the response slots of in-flight LSP requests and applying their
//! results (hover, action slots, completion, inlay hints).

use super::lsp_modules;
use super::*;
use std::sync::Arc;
use std::time::Duration;

impl Editor {
    /// Invalidate hover cache when buffer is modified
    pub fn invalidate_hover_cache(&mut self) {
        if self.lsp.state.hover_cache.is_some() {
            self.lsp.state.hover_cache = None;
        }
    }

    /// Returns true if there's a pending LSP response being waited for
    pub fn has_pending_lsp_response(&self) -> bool {
        self.lsp.slots.any_pending()
    }

    pub fn has_pending_completion_response(&self) -> bool {
        self.lsp.slots.completion.is_pending()
    }

    pub fn has_pending_inlay_hint_response(&self) -> bool {
        self.lsp.slots.inlay_hints.is_pending()
    }

    /// Polls pending LSP responses (non-blocking)
    /// Returns true if a response was processed and UI should redraw
    ///
    /// Returns true if a hover response is pending (spawned but not yet received).
    pub fn has_pending_hover(&self) -> bool {
        self.lsp.slots.hover.is_pending()
    }

    /// Each response type is polled independently so that e.g. a hover request
    /// doesn't block or clobber a goto-definition request.
    ///
    /// Note: diagnostics and inlay hints are NOT polled here — they have
    /// dedicated sections earlier in the tick (sync_lsp_and_refresh_diagnostics
    /// and the inlay hints block) to ensure correct ordering with document sync.
    pub fn poll_pending_lsp_responses(&mut self) -> bool {
        let mut changed = false;

        // --- Navigation ---
        changed |= self.poll_hover_slot();
        changed |= self.poll_signature_help_slot();
        changed |= self.poll_folding_slot();
        changed |= self.poll_goto_slots();

        // --- Completion ---
        changed |= self.poll_pending_completion_response();
        changed |= self.poll_completion_resolve_slot();

        // --- User-triggered actions ---
        changed |= self.poll_action_slots();

        changed
    }

    /// Poll the hover response slot.
    fn poll_hover_slot(&mut self) -> bool {
        let timeout = std::time::Duration::from_secs(10);
        let Some(result) = self.lsp.slots.hover.poll_with_timeout(timeout) else {
            return false;
        };

        match result {
            Ok(hover_result) if !self.request_origin_is_current(&hover_result.origin) => {
                crate::lsp_debug!("LSP-HOVER", "Dropping hover response: editor moved on");
                false
            }
            Ok(hover_result) => {
                if let Some(hover_text) = hover_result.hover_text {
                    crate::lsp_debug!("LSP-HOVER", "Received hover response");

                    let cursor = self.buffer().cursor();
                    let buffer_version = self.buffer().version();
                    let cursor_line = cursor.line();
                    let cursor_col = cursor.col().0;
                    let file_path = self.buffer().file_path().unwrap_or("").to_string();

                    self.lsp.state.hover_cache = Some(crate::editor::lsp_state::HoverCache::new(
                        file_path,
                        cursor_line,
                        cursor_col,
                        buffer_version,
                        hover_text.clone(),
                    ));

                    self.lsp.state.hover_info = Some(hover_text);
                    self.lsp.state.hover_scroll = 0;
                    self.lsp.state.hover_h_scroll = 0;
                    self.lsp.state.hover_position = Some((cursor_line, cursor_col));
                    self.lsp.state.hover_content_type =
                        crate::editor::lsp_state::HoverContentType::LspHover;
                    self.mode = crate::mode::Mode::HoverPreview;
                    self.mark_dirty();
                    self.set_lsp_status(String::new());
                    true
                } else if self.has_diagnostics_on_line(self.buffer().cursor().line()) {
                    // The server has nothing to say about the symbol, but the
                    // line has a diagnostic — that message is almost always
                    // what the user pressed hover to read. Show it instead of
                    // a dead-end "No hover info available" status (which also
                    // lingered and suppressed the message-line echo).
                    crate::lsp_debug!("LSP-HOVER", "No hover info; showing diagnostic");
                    self.show_diagnostic_at_cursor();
                    true
                } else {
                    crate::lsp_debug!("LSP-HOVER", "No hover info available");
                    self.set_lsp_status("No hover info available".to_string());
                    false
                }
            }
            Err(e) => {
                crate::lsp_debug!("LSP-HOVER", "Hover request failed: {:?}", e);
                self.set_lsp_status(format!("Hover failed: {}", e));
                false
            }
        }
    }

    /// True when a buffer-mutating LSP result computed against
    /// (`file_path`, `buffer_version`) can still be applied safely: same
    /// file, not a single edit since the request fired. Results carry
    /// positions into that exact snapshot — splicing them into a buffer the
    /// user kept editing garbles the text (OV-00327).
    fn lsp_mutation_target_is_current(&self, file_path: &str, buffer_version: usize) -> bool {
        self.buffer().file_path() == Some(file_path) && self.buffer().version() == buffer_version
    }

    /// Poll all action slots (Step 5 — format, references, symbols, code actions,
    /// rename, organize imports, call/type hierarchy, semantic tokens).
    pub(super) fn poll_action_slots(&mut self) -> bool {
        let mut changed = false;
        let timeout = Duration::from_secs(15);

        // Format
        if let Some(result) = self.lsp.slots.format.poll_with_timeout(timeout) {
            match result {
                Ok(r) if !r.edits.is_empty() => {
                    if self.lsp_mutation_target_is_current(&r.file_path, r.buffer_version) {
                        self.apply_lsp_edits(r.edits);
                        self.set_lsp_status("Document formatted".to_string());
                        changed = true;
                    } else {
                        self.set_lsp_status(
                            "Format result discarded: buffer changed (rerun format)".to_string(),
                        );
                    }
                }
                Ok(_) => {
                    self.set_lsp_status("No formatting changes".to_string());
                }
                Err(e) => {
                    self.set_lsp_status(format!("Format request failed: {}", e));
                }
            }
        }

        // Find references
        if let Some(result) = self.lsp.slots.references.poll_with_timeout(timeout) {
            match result {
                Ok(r) if !r.locations.is_empty() => {
                    let count = r.locations.len();
                    self.lsp.state.available_references = r.locations.clone();
                    self.lsp.state.active_lsp_result_type =
                        Some(crate::editor::LspResultType::References);
                    let items = self.locations_to_picker_items(&r.locations);
                    self.open_location_picker(items, "References");
                    self.set_lsp_status(format!("Found {} references", count));
                    changed = true;
                }
                Ok(_) => {
                    self.set_lsp_status("No references found".to_string());
                }
                Err(e) => {
                    self.set_lsp_status(format!("References request failed: {}", e));
                }
            }
        }

        // Workspace symbols (live picker: results replace the list in place)
        if let Some(result) = self.lsp.slots.workspace_symbols.poll_with_timeout(timeout) {
            match result {
                Ok(r) => {
                    let items = self.workspace_symbol_items(&r.symbols);
                    let count = items.len();
                    self.lsp.state.available_workspace_symbols = r.symbols;
                    self.lsp.state.active_lsp_result_type =
                        Some(crate::editor::LspResultType::WorkspaceSymbols);
                    if let Some(picker) = self.picker_mut().filter(|p| p.is_symbol_search()) {
                        picker.set_results(items);
                        self.mark_picker_selection_changed();
                        self.set_lsp_status(format!("{count} symbols"));
                        changed = true;
                    }
                }
                Err(e) => {
                    self.set_lsp_status(format!("Workspace symbols request failed: {}", e));
                }
            }
        }

        // Code actions
        if let Some(result) = self.lsp.slots.code_actions.poll_with_timeout(timeout) {
            match result {
                Ok(r)
                    if !r.actions.is_empty()
                        && !self.lsp_mutation_target_is_current(&r.file_path, r.buffer_version) =>
                {
                    // The action edits embed positions into the request-time
                    // snapshot; the buffer has moved on (OV-00327).
                    self.set_lsp_status(
                        "Code actions discarded: buffer changed (rerun)".to_string(),
                    );
                }
                Ok(r) if !r.actions.is_empty() => {
                    let titles: Vec<String> = r
                        .actions
                        .iter()
                        .map(|a| lsp_modules::actions::code_action_title(&a.action))
                        .collect();
                    self.lsp.state.available_code_actions = r.actions;
                    let base_dir =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let picker = crate::editor::picker::Picker::new_custom(base_dir, titles);
                    self.set_picker(picker);
                    self.set_mode(crate::mode::Mode::Picker);
                    self.mark_picker_selection_changed();
                    changed = true;
                }
                Ok(_) => {
                    self.set_lsp_status("No code actions available".to_string());
                }
                Err(e) => {
                    self.set_lsp_status(format!("Code actions request failed: {}", e));
                }
            }
        }

        // Rename
        if let Some(result) = self.lsp.slots.rename.poll_with_timeout(timeout) {
            match result {
                Ok(r) => {
                    if let Some(workspace_edit) = r.edit {
                        if !self.lsp_mutation_target_is_current(&r.file_path, r.buffer_version) {
                            // Positions in the edit are stale (OV-00327).
                            self.set_lsp_status(
                                "Rename discarded: buffer changed (rerun rename)".to_string(),
                            );
                        } else {
                            match self.apply_workspace_edit(workspace_edit) {
                                Ok(true) => {
                                    self.set_lsp_status(format!("Renamed to '{}'", r.new_name));
                                    changed = true;
                                }
                                Ok(false) => {
                                    self.set_lsp_status("Rename failed to apply".to_string());
                                }
                                Err(e) => {
                                    self.set_lsp_status(format!("Failed to apply rename: {}", e));
                                }
                            }
                        }
                    } else {
                        self.set_lsp_status("Rename not available at this location".to_string());
                    }
                }
                Err(e) => {
                    self.set_lsp_status(format!("Rename request failed: {}", e));
                }
            }
        }

        // Organize imports
        if let Some(result) = self.lsp.slots.organize_imports.poll_with_timeout(timeout) {
            match result {
                Ok(r) => {
                    if let Some(action) = r.action {
                        if !self.lsp_mutation_target_is_current(&r.file_path, r.buffer_version) {
                            // Positions in the action edit are stale (OV-00327).
                            self.set_lsp_status(
                                "Organize imports discarded: buffer changed (rerun)".to_string(),
                            );
                        } else {
                            self.lsp.state.available_code_actions = vec![action];
                            self.apply_code_action(0);
                            self.set_lsp_status("Imports organized".to_string());
                            changed = true;
                        }
                    } else {
                        self.set_lsp_status("No organize imports action available".to_string());
                    }
                }
                Err(e) => {
                    self.set_lsp_status(format!("Organize imports failed: {}", e));
                }
            }
        }

        // Call hierarchy, type hierarchy, and drilling into either
        changed |= self.poll_hierarchy_slots(timeout);

        // Semantic tokens
        if let Some(result) = self.lsp.slots.semantic_tokens.poll_with_timeout(timeout) {
            match result {
                Ok(r) => {
                    if let Some(tokens) = r.tokens {
                        if let Some(legend) = r.legend {
                            self.buffer_mut().decode_semantic_tokens(&tokens, &legend);
                            self.set_lsp_status("Semantic tokens applied".to_string());
                        } else {
                            self.set_lsp_status(
                                "Semantic tokens received (no legend available)".to_string(),
                            );
                        }
                        changed = true;
                    } else {
                        self.set_lsp_status("No semantic tokens available".to_string());
                    }
                }
                Err(e) => {
                    self.set_lsp_status(format!("Semantic tokens request failed: {}", e));
                }
            }
        }

        changed
    }

    /// Poll completion responses (non-blocking)
    /// Returns true if a response was processed and UI should redraw
    pub fn poll_pending_completion_response(&mut self) -> bool {
        // Generous: a language server that is still loading (JDK index, first
        // analysis) can take several seconds to answer its first request, and
        // aborting it here left the menu closed until the user asked again.
        let timeout = Duration::from_secs(15);
        let Some(result) = self.lsp.slots.completion.poll_with_timeout(timeout) else {
            return false;
        };
        // The request is over whatever the answer: never leave the
        // "Requesting completions..." status behind (an empty result used to
        // leave it on screen indefinitely).
        if self.lsp_status() == lsp_modules::completion::REQUESTING_STATUS {
            self.set_lsp_status(String::new());
        }

        match result {
            Ok(result) => {
                if self.mode() != crate::mode::Mode::Insert {
                    self.hide_completion_menu();
                    return false;
                }

                // Drop responses that arrived after the user switched files.
                // Without this, completions from file A leak into file B's menu.
                let matches_file = self
                    .buffer()
                    .file_path()
                    .is_some_and(|path| path == result.file_path);
                if !matches_file {
                    self.hide_completion_menu();
                    return false;
                }

                // A response for a buffer that has been edited further is only
                // usable when the user merely kept typing the word at the
                // request position (the common case while typing fast: the
                // answer to `gE` arrives after `gEm`). Anything else - Esc and
                // back into insert mode, cursor moved, text elsewhere edited -
                // means the context has shifted.
                if result.buffer_version != self.buffer().version()
                    && self.completion_typed_since(&result.anchor).is_none()
                {
                    self.hide_completion_menu();
                    return false;
                }
                let cursor_col = self.buffer().cursor_char_col().0;
                if self.buffer().cursor().line() != result.anchor.line
                    || cursor_col < result.anchor.col
                {
                    self.hide_completion_menu();
                    return false;
                }

                if let (Some(synced), Some(flushed_version)) =
                    (result.synced_content, result.synced_lsp_version)
                {
                    self.mark_document_flushed(
                        &result.file_path,
                        Arc::from(synced),
                        flushed_version,
                    );
                }

                let (trigger_col, trigger_prefix) = self.derive_completion_prefix(&result.items);
                let items_version = result.buffer_version;
                let incomplete = result.is_incomplete;
                let anchor = result.anchor;
                let menu = self.completion_menu_mut();
                // An answer for the word the open menu is already completing
                // refreshes it, keeping the user's pick.
                if menu.has_session()
                    && menu.trigger_col() == trigger_col
                    && menu.anchor().is_some_and(|open| open.line == anchor.line)
                {
                    menu.refresh(result.items.clone(), trigger_col, trigger_prefix);
                } else {
                    menu.show(result.items.clone(), trigger_col, trigger_prefix);
                }
                menu.set_incomplete(incomplete);
                menu.set_anchor(anchor);
                // The items' textEdit ranges target this buffer version; if
                // the user typed on since, accepting rebases them through
                // the anchor (OV-00327).
                menu.set_items_buffer_version(items_version);
                self.lsp.state.completion_sources = result
                    .items
                    .iter()
                    .zip(&result.sources)
                    .rev()
                    .map(|(item, source)| {
                        (crate::editor::completion::item_key(item), source.clone())
                    })
                    .collect();
                self.lsp.state.available_completions = result.items;
                self.mark_dirty();
                true
            }
            Err(e) => {
                self.hide_completion_menu();
                self.set_lsp_status(format!("Completion failed: {}", e));
                self.mark_dirty();
                true
            }
        }
    }

    /// Poll inlay hint responses (non-blocking)
    /// Returns true if a response was processed and UI should redraw
    pub fn poll_pending_inlay_hint_response(&mut self) -> bool {
        let timeout = Duration::from_secs(5);
        let Some(result) = self.lsp.slots.inlay_hints.poll_with_timeout(timeout) else {
            return false;
        };

        match result {
            Ok(result) => {
                // File-scoped hints: only check that the file matches.
                // Scroll position is irrelevant since hints cover the full file.
                let matches_file = self
                    .buffer()
                    .file_path()
                    .is_some_and(|path| path == result.request_key.file_path);
                if !matches_file {
                    self.invalidate_inlay_hint_debounce();
                    return false;
                }

                if result.request_key.lsp_version < self.lsp.state.current_file_lsp_sent_version {
                    self.invalidate_inlay_hint_debounce();
                    return false;
                }

                // OV-00258: drop stale-buffer hints rather than placing them
                // mis-aligned. The LSP returned `Position {line, character}`
                // pairs computed against `result.buffer_version`. If the
                // buffer has advanced since (the user kept typing), those
                // positions index into a different rope's content — a
                // `line_to_char(line) + char_idx` lookup against the current
                // rope can land in the middle of a completely different
                // identifier or even a comment line that has no hints at all
                // (the user reported `r:string|undefined` sliced into
                // `awEmbedParam` and `s:boolean` injected into a `// SSR`
                // comment).
                //
                // Why drop instead of project:
                // - Option (2): tag decorations with `source_version =
                //   result.buffer_version` and project forward via
                //   `edit_log.edits_since(...)`. Requires either rope
                //   snapshots at request boundaries or backward edit-log
                //   replay to derive historical line→char mappings. Larger
                //   lift; not justified while inlay hints are off-by-default
                //   per OV-00259.
                // - Option (3): project the LSP `Position`s through the
                //   edit log before converting to char offsets. Same
                //   complexity; same gating.
                //
                // Dropping is correct (zero mis-aligned placements) and
                // cheap; the trailing `invalidate_inlay_hint_debounce()`
                // ensures the next tick fires a fresh request against the
                // current buffer. When OV-00257 is closed and inlay hints
                // are re-enabled by default, revisit (2)/(3) as a quality
                // bump.
                if result.buffer_version != self.buffer().version() {
                    self.invalidate_inlay_hint_debounce();
                    return false;
                }

                if let (Some(synced), Some(flushed_version)) =
                    (result.synced_content, result.synced_lsp_version)
                {
                    self.mark_document_flushed(
                        &result.request_key.file_path,
                        Arc::from(synced),
                        flushed_version,
                    );
                }

                self.lsp.state.current_file_lsp_version = result.request_key.lsp_version;
                self.lsp.state.current_file_lsp_sent_version = result.request_key.lsp_version;
                self.lsp.state.inlay_hints = result.hints;
                // Build decorations from the new hints, anchored to the
                // *current* buffer version: the stored `char_offset` is
                // computed against the current rope, so `edits_since(current)`
                // is empty at placement and the projected accessors yield the
                // stored offset as-is. Later edits fill the edit log and the
                // projection replays them forward. See `project_decoration`.
                let rope = self.buffer().rope().clone();
                let hint_source_version = self.buffer().version() as u64;
                let hint_decs = crate::editor::decoration::decorations_from_inlay_hints(
                    &self.lsp.state.inlay_hints,
                    &rope,
                    // Canonical "text the user sees on this line" — strips the
                    // line terminator (`\n` / `\r\n` / bare `\r`), so UTF-16
                    // → char-index conversion never miscounts a terminator and
                    // anchors a hint onto the next line. (OV-00265)
                    |line_idx| {
                        self.buffer()
                            .line_text(line_idx)
                            .map(|c| c.into_owned())
                            .unwrap_or_default()
                    },
                    hint_source_version,
                );
                self.decorations.replace_source(
                    crate::editor::decoration::DecorationSource::InlayHint,
                    hint_decs,
                    &rope,
                );

                self.mark_dirty();
                true
            }
            Err(_) => {
                self.invalidate_inlay_hint_debounce();
                false
            }
        }
    }

    /// Mark inlay hints as stale so the next tick re-requests them.
    /// Called when a poll result is dropped (wrong file, stale version)
    /// or when the buffer version changes.
    fn invalidate_inlay_hint_debounce(&mut self) {
        self.lsp.slots.inlay_hints.invalidate();
    }
}
