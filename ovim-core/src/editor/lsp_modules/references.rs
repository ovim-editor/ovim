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
use crate::lsp::uri_from_file_path;
use crate::lsp::uri_to_file_path;
use anyhow::Result;
use lsp_types::Location;

impl Editor {
    pub(in crate::editor) async fn find_references_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("find-references").await?;

        self.set_lsp_status("Finding references...".to_string());

        let result = ctx
            .lsp
            .references(&ctx.uri, ctx.line, ctx.character, &ctx.language_id, true)
            .await;

        match result {
            Ok(locations) if !locations.is_empty() => {
                self.lsp.state.available_references = locations.clone();
                self.lsp.state.active_lsp_result_type =
                    Some(crate::editor::LspResultType::References);

                let items = self.locations_to_picker_items(&locations);
                self.open_location_picker(items, "References");
                self.set_lsp_status(format!("Found {} references", locations.len()));
                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No references found".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("References request failed: {}", e));
                Err(e)
            }
        }
    }

    pub(in crate::editor) async fn document_symbols_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("document-symbols").await?;

        self.set_lsp_status("Fetching document symbols...".to_string());

        let result = ctx.lsp.document_symbols(&ctx.uri, &ctx.language_id).await;

        match result {
            Ok(symbols) if !symbols.is_empty() => {
                self.lsp.state.available_document_symbols = symbols.clone();
                self.lsp.state.active_lsp_result_type =
                    Some(crate::editor::LspResultType::DocumentSymbols);

                let file_path = ctx.file_path.clone();
                let items: Vec<PickerResult> = symbols
                    .iter()
                    .map(|sym| {
                        let line = sym.range.start.line as usize;
                        let col = self.utf16_to_grapheme_col(line, sym.range.start.character);
                        PickerResult {
                            display: format!("{}:{}:{} {}", file_path, line + 1, col + 1, sym.name),
                            location: file_path.to_string(),
                            line,
                            col,
                            match_positions: Vec::new(),
                            content: None,
                        }
                    })
                    .collect();

                self.open_location_picker(items, "Document Symbols");
                self.set_lsp_status(format!("Found {} symbols", symbols.len()));
                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No symbols found".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Document symbols request failed: {}", e));
                Err(e)
            }
        }
    }

    pub(in crate::editor) async fn workspace_symbols_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("workspace-symbols").await?;

        self.set_lsp_status("Fetching workspace symbols...".to_string());

        // TODO: Support query parameter for filtering
        let query = String::new();
        let result = ctx.lsp.workspace_symbols(&ctx.language_id, query).await;

        match result {
            Ok(symbols) if !symbols.is_empty() => {
                self.lsp.state.available_workspace_symbols = symbols.clone();
                self.lsp.state.active_lsp_result_type =
                    Some(crate::editor::LspResultType::WorkspaceSymbols);

                let items: Vec<PickerResult> = symbols
                    .iter()
                    .filter_map(|sym| {
                        let path = uri_to_file_path(&sym.location.uri)?;
                        let line = sym.location.range.start.line as usize;
                        let col = self.utf16_to_grapheme_col(line, sym.location.range.start.character);
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
                    .collect();

                self.open_location_picker(items, "Workspace Symbols");
                self.set_lsp_status(format!("Found {} symbols", symbols.len()));
                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No workspace symbols found".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Workspace symbols request failed: {}", e));
                Err(e)
            }
        }
    }

    pub(in crate::editor) async fn call_hierarchy_incoming_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("call-hierarchy").await?;

        self.set_lsp_status("Fetching incoming calls...".to_string());

        let items = ctx
            .lsp
            .prepare_call_hierarchy(ctx.uri, ctx.line, ctx.character, &ctx.language_id)
            .await;

        match items {
            Ok(Some(items)) if !items.is_empty() => {
                let incoming = ctx
                    .lsp
                    .incoming_calls(items[0].clone(), &ctx.language_id)
                    .await;

                match incoming {
                    Ok(Some(calls)) if !calls.is_empty() => {
                        let locations: Vec<Location> = calls
                            .iter()
                            .map(|call| Location {
                                uri: call.from.uri.clone(),
                                range: call.from.selection_range,
                            })
                            .collect();

                        self.store_call_hierarchy(&locations);
                        let picker_items = self.locations_to_picker_items(&locations);
                        self.open_location_picker(picker_items, "Incoming Calls");
                        self.set_lsp_status(format!("Found {} incoming calls", locations.len()));
                        Ok(true)
                    }
                    Ok(_) => {
                        self.set_lsp_status("No incoming calls found".to_string());
                        Ok(false)
                    }
                    Err(e) => {
                        self.set_lsp_status(format!("Incoming calls request failed: {}", e));
                        Err(e)
                    }
                }
            }
            Ok(_) => {
                self.set_lsp_status("Call hierarchy not available at cursor position".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Call hierarchy prepare failed: {}", e));
                Err(e)
            }
        }
    }

    pub(in crate::editor) async fn call_hierarchy_outgoing_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("call-hierarchy").await?;

        self.set_lsp_status("Fetching outgoing calls...".to_string());

        let items = ctx
            .lsp
            .prepare_call_hierarchy(ctx.uri, ctx.line, ctx.character, &ctx.language_id)
            .await;

        match items {
            Ok(Some(items)) if !items.is_empty() => {
                let outgoing = ctx
                    .lsp
                    .outgoing_calls(items[0].clone(), &ctx.language_id)
                    .await;

                match outgoing {
                    Ok(Some(calls)) if !calls.is_empty() => {
                        let locations: Vec<Location> = calls
                            .iter()
                            .map(|call| Location {
                                uri: call.to.uri.clone(),
                                range: call.to.selection_range,
                            })
                            .collect();

                        self.store_call_hierarchy(&locations);
                        let picker_items = self.locations_to_picker_items(&locations);
                        self.open_location_picker(picker_items, "Outgoing Calls");
                        self.set_lsp_status(format!("Found {} outgoing calls", locations.len()));
                        Ok(true)
                    }
                    Ok(_) => {
                        self.set_lsp_status("No outgoing calls found".to_string());
                        Ok(false)
                    }
                    Err(e) => {
                        self.set_lsp_status(format!("Outgoing calls request failed: {}", e));
                        Err(e)
                    }
                }
            }
            Ok(_) => {
                self.set_lsp_status("Call hierarchy not available at cursor position".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Call hierarchy prepare failed: {}", e));
                Err(e)
            }
        }
    }

    pub(in crate::editor) async fn type_hierarchy_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("type-hierarchy").await?;

        self.set_lsp_status("Fetching type hierarchy...".to_string());

        let prepare_result = ctx
            .lsp
            .prepare_type_hierarchy(ctx.uri.clone(), ctx.line, ctx.character, &ctx.language_id)
            .await;

        let items = match prepare_result {
            Ok(Some(items)) => items,
            Ok(None) => {
                self.set_lsp_status("No type hierarchy available at cursor".to_string());
                return Ok(false);
            }
            Err(e) => {
                self.set_lsp_status(format!("Type hierarchy request failed: {}", e));
                return Err(e);
            }
        };

        let item = &items[0];
        let mut all_types = Vec::new();
        let mut all_types_data = Vec::new();

        if let Ok(Some(supertypes)) = ctx.lsp.supertypes(item.clone(), &ctx.language_id).await {
            for supertype in supertypes {
                let location = Location {
                    uri: supertype.uri.clone(),
                    range: supertype.selection_range,
                };
                all_types.push(location.clone());
                all_types_data.push((format!("↑ {}", supertype.name), location));
            }
        }

        if let Ok(Some(subtypes)) = ctx.lsp.subtypes(item.clone(), &ctx.language_id).await {
            for subtype in subtypes {
                let location = Location {
                    uri: subtype.uri.clone(),
                    range: subtype.selection_range,
                };
                all_types.push(location.clone());
                all_types_data.push((format!("↓ {}", subtype.name), location));
            }
        }

        if !all_types.is_empty() {
            self.lsp.state.available_type_hierarchy = all_types_data;
            self.lsp.state.active_lsp_result_type =
                Some(crate::editor::LspResultType::TypeHierarchy);

            let picker_items = self.locations_to_picker_items(&all_types);
            self.open_location_picker(picker_items, "Type Hierarchy");
            self.set_lsp_status(format!("Found {} types", all_types.len()));
            Ok(true)
        } else {
            self.set_lsp_status("No type hierarchy found".to_string());
            Ok(false)
        }
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
            crate::editor::LspResultType::DocumentSymbols => {
                if index >= self.lsp.state.available_document_symbols.len() {
                    self.set_lsp_status("Invalid symbol index".to_string());
                    return;
                }
                let symbol = &self.lsp.state.available_document_symbols[index];
                let Some(file_path) = self.buffer().file_path() else {
                    self.set_lsp_status("Document symbols require a saved file".to_string());
                    return;
                };
                let Some(uri) = uri_from_file_path(file_path) else {
                    self.set_lsp_status("Invalid file path".to_string());
                    return;
                };
                Location {
                    uri,
                    range: symbol.selection_range,
                }
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

    /// Helper method to open a location picker with LSP results
    pub(in crate::editor) fn open_location_picker(
        &mut self,
        items: Vec<PickerResult>,
        _title: &str,
    ) {
        let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        let picker = crate::editor::picker::Picker::new_with_results(base_dir, items);
        self.set_picker(picker);
        self.set_mode(crate::mode::Mode::Picker);
        self.mark_picker_selection_changed();
    }

    /// Convert LSP locations to picker items.
    fn locations_to_picker_items(&self, locations: &[Location]) -> Vec<PickerResult> {
        locations
            .iter()
            .filter_map(|loc| {
                let path = uri_to_file_path(&loc.uri)?;
                let line = loc.range.start.line as usize;
                let col = self.utf16_to_grapheme_col(line, loc.range.start.character);
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

    /// Store call hierarchy locations for navigation.
    fn store_call_hierarchy(&mut self, locations: &[Location]) {
        self.lsp.state.available_call_hierarchy = locations
            .iter()
            .map(|loc| {
                let path = uri_to_file_path(&loc.uri)
                    .map(|p| {
                        p.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string()
                    })
                    .unwrap_or_default();
                (path, loc.clone())
            })
            .collect();
        self.lsp.state.active_lsp_result_type = Some(crate::editor::LspResultType::CallHierarchy);
    }
}
