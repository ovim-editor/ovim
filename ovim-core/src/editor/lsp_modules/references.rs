//! LSP references, symbols, and hierarchy
//!
//! This module handles:
//! - Find references
//! - Document symbols
//! - Workspace symbols
//! - Call hierarchy (incoming/outgoing)
//! - Type hierarchy (supertypes/subtypes)
//! - Navigation to LSP locations
//! - Location picker helper

use super::super::picker::PickerResult;
use super::super::Editor;
use crate::lsp::uri_to_file_path;
use anyhow::Result;
use lsp_types::Location;

impl Editor {
    pub(in crate::editor) async fn find_references_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("find-references").await?;

        self.set_lsp_status("Finding references...".to_string());

        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = ctx
                .lsp
                .references(&ctx.uri, ctx.line, ctx.character, &ctx.language_id, true)
                .await;
            let _ = tx.send(
                result.map(|locations| crate::editor::lsp_slot::ReferencesResult { locations }),
            );
        });

        self.lsp.slots.references.fire(task, rx);
        Ok(true)
    }

    pub(in crate::editor) async fn workspace_symbols_impl(
        &mut self,
        query: String,
    ) -> Result<bool> {
        let ctx = self.prepare_lsp_request("workspace-symbols").await?;

        self.set_lsp_status("Fetching workspace symbols...".to_string());

        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = ctx.lsp.workspace_symbols(&ctx.language_id, query).await;
            let _ = tx.send(
                result.map(|symbols| crate::editor::lsp_slot::WorkspaceSymbolsResult { symbols }),
            );
        });

        self.lsp.slots.workspace_symbols.fire(task, rx);
        Ok(true)
    }

    /// Navigate to an LSP location by index (from references, symbols, call hierarchy, etc.)
    pub fn navigate_to_lsp_location(&mut self, index: usize) {
        let result_type = match &self.lsp.state.active_lsp_result_type {
            Some(t) => t,
            None => {
                self.set_lsp_status("No LSP results available".to_string());
                return;
            }
        };

        let location = match result_type {
            crate::editor::LspResultType::References => {
                if index >= self.lsp.state.available_references.len() {
                    self.set_lsp_status("Invalid reference index".to_string());
                    return;
                }
                self.lsp.state.available_references[index].clone()
            }
            crate::editor::LspResultType::WorkspaceSymbols => {
                if index >= self.lsp.state.available_workspace_symbols.len() {
                    self.set_lsp_status("Invalid symbol index".to_string());
                    return;
                }
                self.lsp.state.available_workspace_symbols[index]
                    .location
                    .clone()
            }
            crate::editor::LspResultType::CallHierarchy
            | crate::editor::LspResultType::TypeHierarchy => {
                let hierarchy_items =
                    if matches!(result_type, crate::editor::LspResultType::CallHierarchy) {
                        &self.lsp.state.available_call_hierarchy
                    } else {
                        &self.lsp.state.available_type_hierarchy
                    };

                if index >= hierarchy_items.len() {
                    self.set_lsp_status("Invalid hierarchy index".to_string());
                    return;
                }
                hierarchy_items[index].1.clone()
            }
        };

        if let Some(path) = uri_to_file_path(&location.uri) {
            let target_line = location.range.start.line as usize;
            let target_character = location.range.start.character;

            self.push_tag();

            if self.buffer().file_path() != Some(path.to_string_lossy().as_ref())
                && self.open_file(path.to_string_lossy().as_ref()).is_err()
            {
                self.set_lsp_status("Failed to open file".to_string());
                return;
            }

            let target_col = self.utf16_to_grapheme_col(target_line, target_character);
            self.buffer_mut()
                .cursor_mut()
                .set_position(target_line, crate::unicode::GraphemeCol(target_col));
            self.buffer_mut().validate_cursor_position();
            self.center_cursor_in_viewport();
            let actual_col = self.buffer().cursor().col();
            self.set_lsp_status(format!(
                "Navigated to {}:{}:{}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                target_line + 1,
                actual_col.0 + 1
            ));
        } else {
            self.set_lsp_status("Invalid file path in LSP response".to_string());
        }
    }

    /// Opens a picker of locations (LSP results, recent files, buffers),
    /// titled `title` and shown relative to the project root like every other
    /// picker. Replaces a hierarchy browser.
    pub(in crate::editor) fn open_location_picker(
        &mut self,
        items: Vec<PickerResult>,
        title: &str,
    ) {
        self.lsp.state.hierarchy = None;
        self.open_location_picker_keeping_hierarchy(items, title);
    }

    /// Opens the location picker without touching hierarchy state.
    pub(in crate::editor) fn open_location_picker_keeping_hierarchy(
        &mut self,
        items: Vec<PickerResult>,
        title: &str,
    ) {
        let picker = crate::editor::picker::Picker::new_with_results(self.picker_dirs().0, items)
            .with_title(title);
        self.set_picker(picker);
        self.set_mode(crate::mode::Mode::Picker);
        self.mark_picker_selection_changed();
    }

    /// Convert LSP locations to picker items.
    pub(in crate::editor) fn locations_to_picker_items(
        &self,
        locations: &[Location],
    ) -> Vec<PickerResult> {
        let mut columns = super::super::lsp_columns::ColumnResolver::new(self);
        locations
            .iter()
            .filter_map(|loc| {
                let path = uri_to_file_path(&loc.uri)?;
                let line = loc.range.start.line as usize;
                let col = columns.grapheme_col(&path, line, loc.range.start.character);
                Some(PickerResult {
                    display: format!(
                        "{}:{}:{}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        line + 1,
                        col + 1
                    ),
                    location: path.to_string_lossy().to_string(),
                    line,
                    col,
                    match_positions: Vec::new(),
                    content: None,
                })
            })
            .collect()
    }
}
