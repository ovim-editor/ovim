//! LSP request methods for LspManager
//!
//! This module contains all the LSP request methods that query language servers
//! for information (goto definition, hover, completion, etc.)

use super::{server::ServerState, utils::marked_string_to_text, LspManager, LspServerInfo};
use anyhow::Result;
use lsp_types::{Diagnostic, Uri};
use std::future::Future;

/// Parse an LSP response, logging any parse failures instead of silently swallowing them.
fn parse_lsp_response<T: serde::de::DeserializeOwned>(
    result: serde_json::Value,
    method: &str,
) -> Option<T> {
    match serde_json::from_value::<T>(result) {
        Ok(v) => Some(v),
        Err(e) => {
            crate::lsp_warn!("LSP-PARSE", "{}: failed to parse response: {}", method, e);
            None
        }
    }
}

/// Why a completion request is being made (LSP `CompletionTriggerKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionTrigger {
    /// Explicit request (Ctrl-Space) or identifier typing.
    Invoked,
    /// A server-advertised trigger character was typed.
    Character(char),
    /// The previous list was `isIncomplete` and the user kept typing.
    Incomplete,
}

/// Completion answer with the list-level flags the menu needs.
#[derive(Debug, Clone, Default)]
pub struct CompletionOutcome {
    pub items: Vec<lsp_types::CompletionItem>,
    /// `CompletionList.isIncomplete`: typing further must ask again.
    pub is_incomplete: bool,
    /// The server each of `items` came from (same length once filled in by a
    /// request), so a later `completionItem/resolve` asks the right one.
    pub sources: Vec<String>,
}

/// Applies `CompletionList.itemDefaults` (LSP 3.17) to the raw response:
/// commit characters, edit range, insert text format/mode and data are copied
/// into every item that does not override them, and `textEditText` becomes the
/// `textEdit.newText`. The rest of the editor then only sees fully-specified
/// items. (`lsp-types` predates `itemDefaults`, hence raw JSON.)
pub fn apply_completion_item_defaults(mut response: serde_json::Value) -> serde_json::Value {
    use serde_json::{json, Value};

    let Some(list) = response.as_object_mut() else {
        return response;
    };
    let Some(defaults) = list.remove("itemDefaults") else {
        return response;
    };
    let Some(items) = list.get_mut("items").and_then(Value::as_array_mut) else {
        return response;
    };
    let Some(defaults) = defaults.as_object() else {
        return response;
    };
    for item in items {
        let Some(item) = item.as_object_mut() else {
            continue;
        };
        for key in [
            "commitCharacters",
            "insertTextFormat",
            "insertTextMode",
            "data",
        ] {
            if !item.contains_key(key) {
                if let Some(value) = defaults.get(key) {
                    item.insert(key.to_string(), value.clone());
                }
            }
        }
        if !item.contains_key("textEdit") {
            if let Some(edit_range) = defaults.get("editRange") {
                let new_text = item
                    .get("textEditText")
                    .or_else(|| item.get("insertText"))
                    .or_else(|| item.get("label"))
                    .cloned()
                    .unwrap_or_else(|| json!(""));
                let mut edit = serde_json::Map::new();
                edit.insert("newText".to_string(), new_text);
                match edit_range.as_object() {
                    // {insert, replace} vs a plain Range {start, end}
                    Some(range) if range.contains_key("insert") => {
                        edit.insert("insert".to_string(), range["insert"].clone());
                        edit.insert("replace".to_string(), range["replace"].clone());
                    }
                    _ => {
                        edit.insert("range".to_string(), edit_range.clone());
                    }
                }
                item.insert("textEdit".to_string(), Value::Object(edit));
            }
        }
        item.remove("textEditText");
    }
    response
}

/// Flattens a completion response into its items and the list-level flag.
pub fn completion_outcome(response: lsp_types::CompletionResponse) -> CompletionOutcome {
    match response {
        lsp_types::CompletionResponse::Array(items) => CompletionOutcome {
            items,
            is_incomplete: false,
            sources: Vec::new(),
        },
        lsp_types::CompletionResponse::List(list) => CompletionOutcome {
            items: list.items,
            is_incomplete: list.is_incomplete,
            sources: Vec::new(),
        },
    }
}

impl LspManager {
    pub async fn goto_definition(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::Location>> {
        self.goto_for_document(uri, line, character, language_id, "textDocument/definition")
            .await
    }

    /// Requests go-to-declaration for a position in a document
    pub async fn goto_declaration(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::Location>> {
        use lsp_types::{
            GotoDefinitionResponse, Position, TextDocumentIdentifier, TextDocumentPositionParams,
        };

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports goto declaration
        if !server.supports_goto_declaration().await {
            return Ok(None); // Gracefully return None if not supported
        }

        let params = lsp_types::GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/declaration", serde_json::to_value(params)?)
            .await?;

        let response: Option<GotoDefinitionResponse> =
            parse_lsp_response(result, "textDocument/declaration");

        // Convert response to single location (take first if multiple)
        Ok(response.and_then(|resp| match resp {
            GotoDefinitionResponse::Scalar(location) => Some(location),
            GotoDefinitionResponse::Array(locations) => locations.into_iter().next(),
            GotoDefinitionResponse::Link(links) => {
                links.into_iter().next().map(|link| lsp_types::Location {
                    uri: link.target_uri,
                    range: link.target_selection_range,
                })
            }
        }))
    }

    /// Requests go-to-implementation for a position in a document
    pub async fn implementation(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::Location>> {
        self.goto_for_document(
            uri,
            line,
            character,
            language_id,
            "textDocument/implementation",
        )
        .await
    }

    /// Requests go-to-type-definition for a position in a document
    pub async fn type_definition(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::Location>> {
        self.goto_for_document(
            uri,
            line,
            character,
            language_id,
            "textDocument/typeDefinition",
        )
        .await
    }

    /// Requests hover information for a position in a document
    pub async fn hover(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<String>> {
        use lsp_types::{
            HoverParams, Position, TextDocumentIdentifier, TextDocumentPositionParams,
        };

        lsp_info!(
            "LSP-HOVER",
            "hover() called | URI: {} | line: {}, char: {} | language: {}",
            uri.as_str(),
            line,
            character,
            language_id
        );

        let server = self.server_for_document(uri, language_id)?;

        lsp_info!("LSP-HOVER", "Server found for language: {}", language_id);

        // Check if server supports hover
        if !server.supports_hover().await {
            lsp_info!(
                "LSP-HOVER",
                "Server does not support hover - returning None"
            );
            return Ok(None); // Gracefully return None if not supported
        }

        lsp_info!("LSP-HOVER", "Server supports hover, sending request");

        // Cancel any pending hover requests before sending new one
        // This prevents stale hover information from appearing when cursor moves rapidly
        // Only the latest hover request matters - previous ones are obsolete
        if let Err(e) = server.cancel_requests_by_method("textDocument/hover").await {
            lsp_warn!(
                "LSP-HOVER",
                "Failed to cancel previous hover requests: {}",
                e
            );
            // Continue anyway - cancellation failure is not critical
        }

        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("textDocument/hover", serde_json::to_value(params)?)
            .await?;

        lsp_info!("LSP-HOVER", "Received hover result: {:?}", result);

        // Handle null response (valid LSP response meaning no hover info)
        let response: Option<lsp_types::Hover> = if result.is_null() {
            lsp_info!("LSP-HOVER", "Result is null");
            None
        } else {
            // Move result instead of cloning (from_value consumes it)
            match serde_json::from_value(result) {
                Ok(hover) => {
                    lsp_info!("LSP-HOVER", "Successfully parsed hover response");
                    Some(hover)
                }
                Err(e) => {
                    lsp_warn!("LSP-HOVER", "Failed to parse hover response: {}", e);
                    None
                }
            }
        };

        // Extract text from hover response with optimized array handling
        let hover_text = response.and_then(|hover| match hover.contents {
            lsp_types::HoverContents::Scalar(content) => Some(marked_string_to_text(content)),
            lsp_types::HoverContents::Array(mut contents) => {
                if contents.is_empty() {
                    None
                } else if contents.len() == 1 {
                    // Single item: no need to allocate a Vec and join
                    Some(marked_string_to_text(contents.remove(0)))
                } else {
                    // Multiple items: allocate and join
                    let texts: Vec<String> =
                        contents.into_iter().map(marked_string_to_text).collect();
                    Some(texts.join("\n\n"))
                }
            }
            lsp_types::HoverContents::Markup(content) => Some(content.value),
        });

        lsp_info!("LSP-HOVER", "Extracted hover text: {:?}", hover_text);
        Ok(hover_text)
    }

    /// Requests code completion for a position in a document
    pub async fn completion(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
        trigger: CompletionTrigger,
    ) -> Result<CompletionOutcome> {
        let server_id = self.server_id_for_document(uri, language_id)?;
        let server = self
            .server_handle(&server_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;

        // Cancel any pending completion requests before sending new one
        // Completion is high-frequency and only latest matters (user is still typing)
        if server.supports_completion().await {
            if let Err(e) = server
                .cancel_requests_by_method("textDocument/completion")
                .await
            {
                lsp_warn!(
                    "LSP-COMPLETION",
                    "Failed to cancel previous completion requests: {}",
                    e
                );
                // Continue anyway - cancellation failure is not critical
            }
        }

        let mut outcome =
            Self::completion_on_server(&server, uri, line, character, trigger).await?;
        outcome.sources = vec![server_id; outcome.items.len()];
        Ok(outcome)
    }

    /// Resolves the lazily-computed fields (documentation, detail, ...) of a
    /// completion item with `completionItem/resolve`. Tries each of
    /// `server_ids` that advertises `resolveProvider` until one answers; the
    /// original item is returned when none does.
    pub async fn resolve_completion_item(
        &self,
        server_ids: &[String],
        item: lsp_types::CompletionItem,
    ) -> Result<lsp_types::CompletionItem> {
        for server_id in server_ids {
            let Some(server) = self
                .servers
                .get(server_id.as_str())
                .map(|entry| entry.value().clone())
            else {
                continue;
            };
            if !server.supports_completion_resolve().await {
                continue;
            }
            // One server failing to resolve (or not knowing the item) must
            // not stop the next one from trying.
            let result = match server
                .request("completionItem/resolve", serde_json::to_value(&item)?)
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    lsp_debug!("LSP-COMPLETION", "{server_id} could not resolve: {error}");
                    continue;
                }
            };
            let resolved: Option<lsp_types::CompletionItem> =
                parse_lsp_response(result, "completionItem/resolve");
            if let Some(resolved) = resolved {
                return Ok(resolved);
            }
        }
        Ok(item)
    }

    /// Requests document formatting
    pub async fn format_document(
        &self,
        uri: &Uri,
        language_id: &str,
        tab_size: u32,
        insert_spaces: bool,
    ) -> Result<Vec<lsp_types::TextEdit>> {
        use lsp_types::{DocumentFormattingParams, FormattingOptions, TextDocumentIdentifier};

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports formatting
        if !server.supports_formatting().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = DocumentFormattingParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            options: FormattingOptions {
                tab_size,
                insert_spaces,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("textDocument/formatting", serde_json::to_value(params)?)
            .await?;

        let edits: Option<Vec<lsp_types::TextEdit>> =
            parse_lsp_response(result, "textDocument/formatting");

        Ok(edits.unwrap_or_default())
    }

    /// Requests range formatting (format only a selection)
    #[allow(clippy::too_many_arguments)]
    pub async fn format_range(
        &self,
        uri: &Uri,
        language_id: &str,
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
        tab_size: u32,
        insert_spaces: bool,
    ) -> Result<Vec<lsp_types::TextEdit>> {
        use lsp_types::{
            DocumentRangeFormattingParams, FormattingOptions, Position, Range,
            TextDocumentIdentifier,
        };

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports range formatting
        if !server.supports_range_formatting().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = DocumentRangeFormattingParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range: Range {
                start: Position {
                    line: start_line,
                    character: start_character,
                },
                end: Position {
                    line: end_line,
                    character: end_character,
                },
            },
            options: FormattingOptions {
                tab_size,
                insert_spaces,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request(
                "textDocument/rangeFormatting",
                serde_json::to_value(params)?,
            )
            .await?;

        let edits: Option<Vec<lsp_types::TextEdit>> =
            parse_lsp_response(result, "textDocument/rangeFormatting");

        Ok(edits.unwrap_or_default())
    }

    /// Requests code actions for a position in a document
    pub async fn code_actions(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Vec<lsp_types::CodeActionOrCommand>> {
        use lsp_types::{
            CodeActionContext, CodeActionParams, CodeActionTriggerKind, Position, Range,
            TextDocumentIdentifier,
        };

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports code actions
        if !server.supports_code_actions().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        // Create a range at the cursor position (zero-width range)
        let range = Range {
            start: Position { line, character },
            end: Position { line, character },
        };

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range,
            context: CodeActionContext {
                diagnostics,
                only: None,
                trigger_kind: Some(CodeActionTriggerKind::INVOKED),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/codeAction", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::CodeActionOrCommand>> =
            parse_lsp_response(result, "textDocument/codeAction");

        Ok(response.unwrap_or_default())
    }

    /// Resolves a lazily-populated code action on a specific language server.
    pub async fn resolve_code_action(
        &self,
        language_id: &str,
        action: lsp_types::CodeAction,
    ) -> Result<lsp_types::CodeAction> {
        let server = self
            .server_handle(language_id)
            .ok_or_else(|| anyhow::anyhow!("No server for language: {}", language_id))?;
        Self::resolve_code_action_on_server(&server, action).await
    }

    /// Resolves a lazily-populated code action on a specific server ID.
    pub async fn resolve_code_action_on_server_id(
        &self,
        server_id: &str,
        action: lsp_types::CodeAction,
    ) -> Result<lsp_types::CodeAction> {
        let server = self
            .server_handle(server_id)
            .ok_or_else(|| anyhow::anyhow!("No server with id: {}", server_id))?;
        Self::resolve_code_action_on_server(&server, action).await
    }

    async fn resolve_code_action_on_server(
        server: &super::LanguageServer,
        action: lsp_types::CodeAction,
    ) -> Result<lsp_types::CodeAction> {
        let fallback = action.clone();
        let result = server
            .request("codeAction/resolve", serde_json::to_value(action)?)
            .await?;
        let resolved: Option<lsp_types::CodeAction> =
            parse_lsp_response(result, "codeAction/resolve");
        Ok(resolved.unwrap_or(fallback))
    }

    /// Requests find references for a symbol at a position
    pub async fn references(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
        include_declaration: bool,
    ) -> Result<Vec<lsp_types::Location>> {
        use lsp_types::{
            Position, ReferenceContext, ReferenceParams, TextDocumentIdentifier,
            TextDocumentPositionParams,
        };

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports references
        if !server.supports_references().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            context: ReferenceContext {
                include_declaration,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/references", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::Location>> =
            parse_lsp_response(result, "textDocument/references");

        Ok(response.unwrap_or_default())
    }

    /// Prepares for rename by checking if the symbol can be renamed
    /// Returns the range of the symbol to rename and optionally a placeholder
    pub async fn prepare_rename(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::PrepareRenameResponse>> {
        use lsp_types::{Position, TextDocumentIdentifier, TextDocumentPositionParams};

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports prepare rename
        if !server.supports_prepare_rename().await {
            return Ok(None); // Return None if not supported
        }

        let params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            position: Position { line, character },
        };

        let result = server
            .request("textDocument/prepareRename", serde_json::to_value(params)?)
            .await?;

        let response: Option<lsp_types::PrepareRenameResponse> =
            parse_lsp_response(result, "textDocument/prepareRename");

        Ok(response)
    }

    /// Requests rename for a symbol at a position
    pub async fn rename(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
        new_name: String,
    ) -> Result<Option<lsp_types::WorkspaceEdit>> {
        use lsp_types::{
            Position, RenameParams, TextDocumentIdentifier, TextDocumentPositionParams,
        };

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports rename
        if !server.supports_rename().await {
            return Ok(None); // Return None if not supported
        }

        let params = RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            new_name,
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("textDocument/rename", serde_json::to_value(params)?)
            .await?;

        let response: Option<lsp_types::WorkspaceEdit> =
            parse_lsp_response(result, "textDocument/rename");

        Ok(response)
    }

    /// Requests signature help for a position in a document
    pub async fn signature_help(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::SignatureHelp>> {
        use lsp_types::{
            Position, SignatureHelpParams, TextDocumentIdentifier, TextDocumentPositionParams,
        };

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports signature help
        if !server.supports_signature_help().await {
            return Ok(None); // Return None if not supported
        }

        // Cancel any pending signature help requests before sending new one
        // Signature help is triggered during typing, only latest position matters
        if let Err(e) = server
            .cancel_requests_by_method("textDocument/signatureHelp")
            .await
        {
            lsp_warn!(
                "LSP-SIGNATURE",
                "Failed to cancel previous signature help requests: {}",
                e
            );
            // Continue anyway - cancellation failure is not critical
        }

        let params = SignatureHelpParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            context: None,
        };

        let result = server
            .request("textDocument/signatureHelp", serde_json::to_value(params)?)
            .await?;

        let response: Option<lsp_types::SignatureHelp> =
            parse_lsp_response(result, "textDocument/signatureHelp");

        Ok(response)
    }

    /// Requests selection ranges for smart selection expansion
    pub async fn selection_range(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<lsp_types::SelectionRange>> {
        use lsp_types::{Position, SelectionRangeParams, TextDocumentIdentifier};

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports selection range
        if !server.supports_selection_range().await {
            return Ok(None); // Return None if not supported
        }

        let params = SelectionRangeParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            positions: vec![Position { line, character }],
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/selectionRange", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::SelectionRange>> =
            parse_lsp_response(result, "textDocument/selectionRange");

        // Return the first (and only) selection range
        Ok(response.and_then(|ranges| ranges.into_iter().next()))
    }

    /// Requests document symbols (outline)
    pub async fn document_symbols(
        &self,
        uri: &Uri,
        language_id: &str,
    ) -> Result<Vec<lsp_types::DocumentSymbol>> {
        use lsp_types::{DocumentSymbolParams, TextDocumentIdentifier};

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports document symbols
        if !server.supports_document_symbol().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/documentSymbol", serde_json::to_value(params)?)
            .await?;

        // Response can be either DocumentSymbol[] or SymbolInformation[]
        // Try DocumentSymbol first (hierarchical)
        if let Ok(symbols) =
            serde_json::from_value::<Vec<lsp_types::DocumentSymbol>>(result.clone())
        {
            return Ok(symbols);
        }

        // Fall back to SymbolInformation (flat) - convert to DocumentSymbol
        if let Ok(symbols) = serde_json::from_value::<Vec<lsp_types::SymbolInformation>>(result) {
            // Convert SymbolInformation to DocumentSymbol (without children)
            let doc_symbols = symbols
                .into_iter()
                .map(|sym| {
                    #[allow(deprecated)]
                    let symbol = lsp_types::DocumentSymbol {
                        name: sym.name,
                        detail: None,
                        kind: sym.kind,
                        tags: sym.tags,
                        deprecated: None, // Deprecated in favor of tags
                        range: sym.location.range,
                        selection_range: sym.location.range,
                        children: None,
                    };
                    symbol
                })
                .collect();
            return Ok(doc_symbols);
        }

        // OV-00153: Log when both parse attempts fail
        crate::lsp_warn!(
            "LSP-PARSE",
            "textDocument/documentSymbol: failed to parse as DocumentSymbol[] or SymbolInformation[]"
        );
        Ok(Vec::new())
    }

    /// Requests document highlights for symbol at position
    /// Returns ranges that should be highlighted (read, write, or text occurrences)
    pub async fn document_highlight(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Vec<lsp_types::DocumentHighlight>> {
        use lsp_types::{
            DocumentHighlightParams, Position, TextDocumentIdentifier, TextDocumentPositionParams,
        };

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports document highlight
        if !server.supports_document_highlight().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = DocumentHighlightParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request(
                "textDocument/documentHighlight",
                serde_json::to_value(params)?,
            )
            .await?;

        let response: Option<Vec<lsp_types::DocumentHighlight>> =
            parse_lsp_response(result, "textDocument/documentHighlight");

        Ok(response.unwrap_or_default())
    }

    /// Requests workspace-wide symbol search from the primary server of every
    /// project root of the language. Fails only when no server answered.
    pub async fn workspace_symbols(
        &self,
        language_id: &str,
        query: String,
    ) -> Result<Vec<lsp_types::SymbolInformation>> {
        let servers: Vec<_> = self
            .servers_for_language(language_id)
            .into_iter()
            .filter(|server_id| !server_id.contains(':'))
            .filter_map(|server_id| self.server_handle(&server_id))
            .collect();
        if servers.is_empty() {
            return Err(anyhow::anyhow!("No server for language: {}", language_id));
        }

        let mut symbols = Vec::new();
        let mut last_error = None;
        let mut answered = false;
        for server in &servers {
            match Self::workspace_symbols_on_server(server, query.clone()).await {
                Ok(found) => {
                    answered = true;
                    symbols.extend(found);
                }
                Err(error) => last_error = Some(error),
            }
        }
        match last_error {
            Some(error) if !answered => Err(error),
            _ => Ok(symbols),
        }
    }

    async fn workspace_symbols_on_server(
        server: &super::LanguageServer,
        query: String,
    ) -> Result<Vec<lsp_types::SymbolInformation>> {
        use lsp_types::WorkspaceSymbolParams;

        // Check if server supports workspace symbols
        if !server.supports_workspace_symbol().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = WorkspaceSymbolParams {
            query,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("workspace/symbol", serde_json::to_value(params)?)
            .await?;

        // Response can be either SymbolInformation[] or WorkspaceSymbol[]
        // Try SymbolInformation first (simpler format)
        if let Ok(symbols) =
            serde_json::from_value::<Vec<lsp_types::SymbolInformation>>(result.clone())
        {
            return Ok(symbols);
        }

        // Try WorkspaceSymbol (newer format with optional data field)
        if let Ok(symbols) = serde_json::from_value::<Vec<lsp_types::WorkspaceSymbol>>(result) {
            // Convert WorkspaceSymbol to SymbolInformation
            let symbol_infos = symbols
                .into_iter()
                .filter_map(|sym| {
                    // WorkspaceSymbol has OneOf<Location, WorkspaceLocation>
                    // We only support full Location for now
                    match sym.location {
                        lsp_types::OneOf::Left(location) => {
                            #[allow(deprecated)]
                            let info = lsp_types::SymbolInformation {
                                name: sym.name,
                                kind: sym.kind,
                                tags: sym.tags,
                                deprecated: None, // Deprecated in favor of tags
                                location,
                                container_name: sym.container_name,
                            };
                            Some(info)
                        }
                        lsp_types::OneOf::Right(_workspace_location) => {
                            // Skip workspace locations (URIs without ranges) for now
                            // These need to be resolved separately
                            None
                        }
                    }
                })
                .collect();
            return Ok(symbol_infos);
        }

        // OV-00153: Log when both parse attempts fail
        crate::lsp_warn!(
            "LSP-PARSE",
            "workspace/symbol: failed to parse as SymbolInformation[] or WorkspaceSymbol[]"
        );
        Ok(Vec::new())
    }

    /// Requests folding ranges for a document
    /// Returns ranges that can be folded (functions, blocks, comments, etc.)
    pub async fn folding_range(
        &self,
        uri: &Uri,
        language_id: &str,
    ) -> Result<Vec<lsp_types::FoldingRange>> {
        use lsp_types::{FoldingRangeParams, TextDocumentIdentifier};

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports folding range
        if !server.supports_folding_range().await {
            return Ok(Vec::new()); // Return empty list if not supported
        }

        let params = FoldingRangeParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/foldingRange", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::FoldingRange>> =
            parse_lsp_response(result, "textDocument/foldingRange");

        Ok(response.unwrap_or_default())
    }

    /// Fetches the text of a document that has no `file:` location (LSP 3.18
    /// `workspace/textDocumentContent`), e.g. a class file or a library source
    /// a server addresses by its own URI scheme.
    pub async fn text_document_content(&self, uri: &Uri, language_id: &str) -> Result<String> {
        let server = self.server_for_document(uri, language_id)?;
        let result = server
            .request(
                "workspace/textDocumentContent",
                serde_json::json!({ "uri": uri.as_str() }),
            )
            .await?;
        result
            .get("text")
            .and_then(|text| text.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("server returned no text for {}", uri.as_str()))
    }

    /// Prepares call hierarchy for a position in a document
    /// Returns call hierarchy items at the cursor position (typically one item)
    pub async fn prepare_call_hierarchy(
        &self,
        uri: Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyItem>>> {
        use lsp_types::{
            CallHierarchyPrepareParams, Position, TextDocumentIdentifier,
            TextDocumentPositionParams,
        };

        let server = self.server_for_document(&uri, language_id)?;

        // Check if server supports call hierarchy
        if !server.supports_call_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = CallHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request(
                "textDocument/prepareCallHierarchy",
                serde_json::to_value(params)?,
            )
            .await?;

        let response: Option<Vec<lsp_types::CallHierarchyItem>> =
            parse_lsp_response(result, "textDocument/prepareCallHierarchy");

        Ok(response)
    }

    /// Requests incoming calls for a call hierarchy item
    /// Returns methods/functions that call the given item
    pub async fn incoming_calls(
        &self,
        item: lsp_types::CallHierarchyItem,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyIncomingCall>>> {
        use lsp_types::CallHierarchyIncomingCallsParams;

        let server = self.server_for_document(&item.uri, language_id)?;

        // Check if server supports call hierarchy
        if !server.supports_call_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = CallHierarchyIncomingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("callHierarchy/incomingCalls", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::CallHierarchyIncomingCall>> =
            parse_lsp_response(result, "callHierarchy/incomingCalls");

        Ok(response)
    }

    /// Requests outgoing calls for a call hierarchy item
    /// Returns methods/functions that the given item calls
    pub async fn outgoing_calls(
        &self,
        item: lsp_types::CallHierarchyItem,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::CallHierarchyOutgoingCall>>> {
        use lsp_types::CallHierarchyOutgoingCallsParams;

        let server = self.server_for_document(&item.uri, language_id)?;

        // Check if server supports call hierarchy
        if !server.supports_call_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = CallHierarchyOutgoingCallsParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("callHierarchy/outgoingCalls", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::CallHierarchyOutgoingCall>> =
            parse_lsp_response(result, "callHierarchy/outgoingCalls");

        Ok(response)
    }

    /// Prepares type hierarchy for a position in a document
    /// Returns type hierarchy items at the cursor position (typically one item - the class/interface at cursor)
    pub async fn prepare_type_hierarchy(
        &self,
        uri: Uri,
        line: u32,
        character: u32,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::TypeHierarchyItem>>> {
        use lsp_types::{
            Position, TextDocumentIdentifier, TextDocumentPositionParams,
            TypeHierarchyPrepareParams,
        };

        let server = self.server_for_document(&uri, language_id)?;

        // Check if server supports type hierarchy
        if !server.supports_type_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = TypeHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request(
                "textDocument/prepareTypeHierarchy",
                serde_json::to_value(params)?,
            )
            .await?;

        let response: Option<Vec<lsp_types::TypeHierarchyItem>> =
            parse_lsp_response(result, "textDocument/prepareTypeHierarchy");

        Ok(response)
    }

    /// Requests supertypes (parent classes and interfaces) for a type hierarchy item
    /// Returns parent classes and implemented interfaces
    pub async fn supertypes(
        &self,
        item: lsp_types::TypeHierarchyItem,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::TypeHierarchyItem>>> {
        use lsp_types::TypeHierarchySupertypesParams;

        let server = self.server_for_document(&item.uri, language_id)?;

        // Check if server supports type hierarchy
        if !server.supports_type_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = TypeHierarchySupertypesParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("typeHierarchy/supertypes", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::TypeHierarchyItem>> =
            parse_lsp_response(result, "typeHierarchy/supertypes");

        Ok(response)
    }

    /// Requests subtypes (subclasses and implementations) for a type hierarchy item
    /// Returns child classes and interface implementations
    pub async fn subtypes(
        &self,
        item: lsp_types::TypeHierarchyItem,
        language_id: &str,
    ) -> Result<Option<Vec<lsp_types::TypeHierarchyItem>>> {
        use lsp_types::TypeHierarchySubtypesParams;

        let server = self.server_for_document(&item.uri, language_id)?;

        // Check if server supports type hierarchy
        if !server.supports_type_hierarchy().await {
            return Ok(None); // Return None if not supported
        }

        let params = TypeHierarchySubtypesParams {
            item,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("typeHierarchy/subtypes", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::TypeHierarchyItem>> =
            parse_lsp_response(result, "typeHierarchy/subtypes");

        Ok(response)
    }

    /// Executes a command on the servers of the project root that owns `uri`
    /// (e.g., "Organize Imports"). Returns the command result if successful
    pub async fn execute_command(
        &self,
        command: String,
        arguments: Option<Vec<serde_json::Value>>,
        uri: &Uri,
        language_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        use lsp_types::ExecuteCommandParams;

        let mut server_ids = self.servers_for_document_uri(language_id, uri);
        if server_ids.is_empty() {
            server_ids = self.servers_for_language(language_id);
        }
        if server_ids.is_empty() {
            return Err(anyhow::anyhow!("No server for language: {}", language_id));
        }

        let mut saw_capable_server = false;
        let mut last_error: Option<String> = None;

        for server_id in server_ids {
            let server = match self
                .servers
                .get(server_id.as_str())
                .map(|e| e.value().clone())
            {
                Some(server) => server,
                None => continue,
            };

            if !server.supports_execute_command().await {
                continue;
            }
            saw_capable_server = true;

            let params = ExecuteCommandParams {
                command: command.clone(),
                arguments: arguments.clone().unwrap_or_default(),
                work_done_progress_params: Default::default(),
            };

            match server
                .request("workspace/executeCommand", serde_json::to_value(params)?)
                .await
            {
                Ok(result) => return Ok(Some(result)),
                Err(e) => {
                    crate::lsp_debug!(
                        "LSP-EXEC-CMD",
                        "Server {} failed executeCommand '{}': {}",
                        server_id,
                        command,
                        e
                    );
                    last_error = Some(e.to_string());
                }
            }
        }

        if !saw_capable_server {
            Err(anyhow::anyhow!(
                "No server for language '{}' supports workspace/executeCommand",
                language_id
            ))
        } else {
            Err(anyhow::anyhow!(
                "Failed to execute command '{}' on all servers for language '{}': {}",
                command,
                language_id,
                last_error.unwrap_or_else(|| "unknown error".to_string())
            ))
        }
    }

    /// Executes a command on a specific server ID.
    /// Returns the command result if successful.
    pub async fn execute_command_on_server_id(
        &self,
        command: String,
        arguments: Option<Vec<serde_json::Value>>,
        server_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        use lsp_types::ExecuteCommandParams;

        let server = self
            .servers
            .get(server_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow::anyhow!("No server with id: {}", server_id))?;

        if !server.supports_execute_command().await {
            return Err(anyhow::anyhow!(
                "Server '{}' does not support workspace/executeCommand",
                server_id
            ));
        }

        let params = ExecuteCommandParams {
            command: command.clone(),
            arguments: arguments.unwrap_or_default(),
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("workspace/executeCommand", serde_json::to_value(params)?)
            .await?;

        Ok(Some(result))
    }

    /// Requests code lenses for a document from every capable server that
    /// owns it. Lenses the server left unresolved (no `command`) are resolved
    /// with `codeLens/resolve` (best effort, capped). Each lens is paired with
    /// the id of the server that produced it, so a later command runs there.
    pub async fn code_lenses(
        &self,
        uri: &Uri,
        file: &std::path::Path,
        language_id: &str,
    ) -> Result<Vec<(String, lsp_types::CodeLens)>> {
        use lsp_types::{CodeLens, CodeLensParams, TextDocumentIdentifier};

        let mut all = Vec::new();
        for server_id in self.servers_for_document(language_id, file) {
            let Some(server) = self
                .servers
                .get(server_id.as_str())
                .map(|entry| entry.value().clone())
            else {
                continue;
            };
            if !server.supports_code_lens().await {
                continue;
            }
            let params = CodeLensParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let result = server
                .request("textDocument/codeLens", serde_json::to_value(params)?)
                .await?;
            let lenses: Vec<CodeLens> =
                parse_lsp_response::<Option<Vec<CodeLens>>>(result, "textDocument/codeLens")
                    .flatten()
                    .unwrap_or_default();
            for (index, lens) in lenses.into_iter().enumerate() {
                let lens = if lens.command.is_none() && index < 100 {
                    match server
                        .request("codeLens/resolve", serde_json::to_value(&lens)?)
                        .await
                        .ok()
                        .and_then(|v| serde_json::from_value::<CodeLens>(v).ok())
                    {
                        Some(resolved) => resolved,
                        None => lens,
                    }
                } else {
                    lens
                };
                all.push((server_id.clone(), lens));
            }
        }
        Ok(all)
    }

    /// Requests inlay hints for a document range
    pub async fn inlay_hints(
        &self,
        uri: &Uri,
        range: lsp_types::Range,
        language_id: &str,
    ) -> Result<Vec<lsp_types::InlayHint>> {
        use lsp_types::{InlayHintParams, TextDocumentIdentifier, WorkDoneProgressParams};

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports inlay hints
        if !server.supports_inlay_hints().await {
            return Ok(Vec::new());
        }

        let params = InlayHintParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range,
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let result = server
            .request("textDocument/inlayHint", serde_json::to_value(params)?)
            .await?;

        let hints: Vec<lsp_types::InlayHint> =
            parse_lsp_response(result, "textDocument/inlayHint").unwrap_or_default();

        Ok(hints)
    }

    /// Gets LSP status information for all active servers
    /// Returns a list of server info with language, command, state, and pending requests
    pub async fn get_lsp_status(&self) -> Vec<LspServerInfo> {
        let mut result = Vec::new();

        for entry in self.servers.iter() {
            let language = entry.key().clone();
            let server = entry.value();
            let state = server.get_state().await;
            let pending_count = server.pending_requests_count().await;
            let has_capabilities = server.has_capabilities().await;
            let command = server.get_command().await;

            result.push(LspServerInfo {
                language,
                command,
                state: match &state {
                    ServerState::Spawning => "spawning".to_string(),
                    ServerState::Initializing { .. } => "initializing".to_string(),
                    ServerState::Ready { .. } => "ready".to_string(),
                    ServerState::Failed { error, .. } => format!("failed: {}", error),
                    ServerState::ShuttingDown => "shutting_down".to_string(),
                    ServerState::Terminated => "terminated".to_string(),
                },
                pending_requests: pending_count,
                has_capabilities,
            });
        }

        result
    }

    /// Gets the list of active language server names
    pub async fn get_active_servers(&self) -> Vec<String> {
        self.servers
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Requests semantic tokens for the entire document
    pub async fn semantic_tokens_full(
        &self,
        uri: &Uri,
        language_id: &str,
    ) -> Result<Option<lsp_types::SemanticTokens>> {
        use lsp_types::{SemanticTokensParams, TextDocumentIdentifier};

        lsp_info!(
            "LSP-SEMANTIC-TOKENS",
            "semantic_tokens_full() | URI: {} | language: {}",
            uri.as_str(),
            language_id
        );

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports semantic tokens
        if !server.supports_semantic_tokens().await {
            lsp_info!(
                "LSP-SEMANTIC-TOKENS",
                "Server does not support semantic tokens - returning None"
            );
            return Ok(None);
        }

        let params = SemanticTokensParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request(
                "textDocument/semanticTokens/full",
                serde_json::to_value(params)?,
            )
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        Ok(parse_lsp_response(
            result,
            "textDocument/semanticTokens/full",
        ))
    }

    /// Requests semantic tokens for a range within the document
    pub async fn semantic_tokens_range(
        &self,
        uri: &Uri,
        range: lsp_types::Range,
        language_id: &str,
    ) -> Result<Option<lsp_types::SemanticTokens>> {
        use lsp_types::{SemanticTokensRangeParams, TextDocumentIdentifier};

        lsp_info!(
            "LSP-SEMANTIC-TOKENS",
            "semantic_tokens_range() | URI: {} | range: {:?} | language: {}",
            uri.as_str(),
            range,
            language_id
        );

        let server = self.server_for_document(uri, language_id)?;

        // Check if server supports semantic tokens
        if !server.supports_semantic_tokens().await {
            lsp_info!(
                "LSP-SEMANTIC-TOKENS",
                "Server does not support semantic tokens - returning None"
            );
            return Ok(None);
        }

        let params = SemanticTokensRangeParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request(
                "textDocument/semanticTokens/range",
                serde_json::to_value(params)?,
            )
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        Ok(parse_lsp_response(
            result,
            "textDocument/semanticTokens/range",
        ))
    }

    /// Gets the semantic tokens legend from a server's capabilities
    /// The legend maps token type and modifier indices to their names
    pub async fn get_semantic_tokens_legend(
        &self,
        uri: &Uri,
        language_id: &str,
    ) -> Result<Option<lsp_types::SemanticTokensLegend>> {
        let server = self.server_for_document(uri, language_id)?;

        let caps = server.capabilities().await;
        if let Some(caps) = caps {
            if let Some(provider) = &caps.semantic_tokens_provider {
                let legend = match provider {
                    lsp_types::SemanticTokensServerCapabilities::SemanticTokensOptions(opts) => {
                        opts.legend.clone()
                    }
                    lsp_types::SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(opts) => {
                        opts.semantic_tokens_options.legend.clone()
                    }
                };
                return Ok(Some(legend));
            }
        }
        Ok(None)
    }

    // =========================================================================
    // Multi-server fan-out methods
    // =========================================================================

    /// Fan out a request to multiple servers concurrently with a 3s per-server timeout.
    /// Returns collected results from all servers that responded successfully.
    async fn fan_out<T, Fut, F>(&self, server_ids: &[String], per_server: F) -> Vec<T>
    where
        T: Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        F: Fn(super::LanguageServer) -> Fut,
    {
        self.fan_out_with_ids(server_ids, per_server)
            .await
            .into_iter()
            .map(|(_, result)| result)
            .collect()
    }

    /// [`Self::fan_out`] keeping the id of the server each result came from.
    async fn fan_out_with_ids<T, Fut, F>(
        &self,
        server_ids: &[String],
        per_server: F,
    ) -> Vec<(String, T)>
    where
        T: Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        F: Fn(super::LanguageServer) -> Fut,
    {
        use std::time::Duration;

        let mut futures = Vec::new();
        for sid in server_ids {
            let server = self.servers.get(sid.as_str()).map(|e| e.value().clone());
            if let Some(server) = server {
                let sid = sid.clone();
                let fut = per_server(server);
                futures.push(tokio::spawn(async move {
                    match tokio::time::timeout(Duration::from_secs(3), fut).await {
                        Ok(Ok(val)) => Some((sid, val)),
                        Ok(Err(e)) => {
                            lsp_debug!("LSP-MULTI", "Server {} failed: {}", sid, e);
                            None
                        }
                        Err(_) => {
                            lsp_debug!("LSP-MULTI", "Server {} timed out", sid);
                            None
                        }
                    }
                }));
            }
        }

        let mut results = Vec::new();
        for f in futures {
            if let Ok(Some(val)) = f.await {
                results.push(val);
            }
        }
        results
    }

    /// Hover from multiple servers, concatenating results with separator.
    pub async fn hover_multi(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        server_ids: &[String],
    ) -> Result<Option<String>> {
        let results: Vec<Option<String>> = self
            .fan_out(server_ids, |server| {
                let uri = uri.clone();
                async move { Self::hover_on_server(&server, &uri, line, character).await }
            })
            .await;

        let texts: Vec<String> = results.into_iter().flatten().collect();
        Ok(if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n\n---\n\n"))
        })
    }

    /// Internal: hover on a single server (used by hover_multi)
    async fn hover_on_server(
        server: &super::LanguageServer,
        uri: &Uri,
        line: u32,
        character: u32,
    ) -> Result<Option<String>> {
        use lsp_types::{
            HoverParams, Position, TextDocumentIdentifier, TextDocumentPositionParams,
        };

        if !server.supports_hover().await {
            return Ok(None);
        }

        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let result = server
            .request("textDocument/hover", serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }

        let response: Option<lsp_types::Hover> = parse_lsp_response(result, "textDocument/hover");
        Ok(response.and_then(|hover| match hover.contents {
            lsp_types::HoverContents::Scalar(content) => Some(marked_string_to_text(content)),
            lsp_types::HoverContents::Array(mut contents) => {
                if contents.is_empty() {
                    None
                } else if contents.len() == 1 {
                    Some(marked_string_to_text(contents.remove(0)))
                } else {
                    let texts: Vec<String> =
                        contents.into_iter().map(marked_string_to_text).collect();
                    Some(texts.join("\n\n"))
                }
            }
            lsp_types::HoverContents::Markup(content) => Some(content.value),
        }))
    }

    /// Completion from multiple servers, concatenating result lists.
    pub async fn completion_multi(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        server_ids: &[String],
        trigger: CompletionTrigger,
    ) -> Result<CompletionOutcome> {
        let results: Vec<(String, CompletionOutcome)> = self
            .fan_out_with_ids(server_ids, |server| {
                let uri = uri.clone();
                async move {
                    Self::completion_on_server(&server, &uri, line, character, trigger).await
                }
            })
            .await;

        let mut merged = CompletionOutcome::default();
        for (server_id, outcome) in results {
            merged.is_incomplete |= outcome.is_incomplete;
            merged
                .sources
                .extend(std::iter::repeat_n(server_id, outcome.items.len()));
            merged.items.extend(outcome.items);
        }
        Ok(merged)
    }

    /// Internal: completion on a single server
    async fn completion_on_server(
        server: &super::LanguageServer,
        uri: &Uri,
        line: u32,
        character: u32,
        trigger: CompletionTrigger,
    ) -> Result<CompletionOutcome> {
        use lsp_types::{
            CompletionContext, CompletionParams, CompletionTriggerKind, Position,
            TextDocumentIdentifier, TextDocumentPositionParams,
        };

        if !server.supports_completion().await {
            return Ok(CompletionOutcome::default());
        }

        let (trigger_kind, trigger_character) = match trigger {
            CompletionTrigger::Invoked => (CompletionTriggerKind::INVOKED, None),
            CompletionTrigger::Character(c) => (
                CompletionTriggerKind::TRIGGER_CHARACTER,
                Some(c.to_string()),
            ),
            CompletionTrigger::Incomplete => (
                CompletionTriggerKind::TRIGGER_FOR_INCOMPLETE_COMPLETIONS,
                None,
            ),
        };
        let params = CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: Some(CompletionContext {
                trigger_kind,
                trigger_character,
            }),
        };

        let result = server
            .request("textDocument/completion", serde_json::to_value(params)?)
            .await?;

        let response: Option<lsp_types::CompletionResponse> = parse_lsp_response(
            apply_completion_item_defaults(result),
            "textDocument/completion",
        );
        let mut outcome = response.map(completion_outcome).unwrap_or_default();
        let all_commit_characters = server.completion_all_commit_characters().await;
        if !all_commit_characters.is_empty() {
            for item in &mut outcome.items {
                if item.commit_characters.is_none() {
                    item.commit_characters = Some(all_commit_characters.clone());
                }
            }
        }
        Ok(outcome)
    }

    /// Goto definition from multiple servers, returning first non-empty result.
    pub async fn goto_definition_multi(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        server_ids: &[String],
    ) -> Result<Option<lsp_types::Location>> {
        let results: Vec<Option<lsp_types::Location>> = self
            .fan_out(server_ids, |server| {
                let uri = uri.clone();
                async move { Self::goto_definition_on_server(&server, &uri, line, character).await }
            })
            .await;

        Ok(results.into_iter().flatten().next())
    }

    /// Internal: goto definition on a single server
    async fn goto_definition_on_server(
        server: &super::LanguageServer,
        uri: &Uri,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::Location>> {
        Self::goto_on_server(server, uri, line, character, "textDocument/definition").await
    }

    /// Route all navigation requests through the document's owning root, even
    /// when there is just one server (its ID need not equal the language).
    async fn goto_for_document(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        language_id: &str,
        method: &'static str,
    ) -> Result<Option<lsp_types::Location>> {
        let ids = self.servers_for_document_uri(language_id, uri);
        anyhow::ensure!(!ids.is_empty(), "No server for this {language_id} document");
        if ids.len() == 1 {
            let server = self
                .servers
                .get(&ids[0])
                .map(|entry| entry.value().clone())
                .ok_or_else(|| anyhow::anyhow!("Language server is no longer running"))?;
            return Self::goto_on_server(&server, uri, line, character, method).await;
        }
        let results = self
            .fan_out(&ids, |server| {
                let uri = uri.clone();
                async move { Self::goto_on_server(&server, &uri, line, character, method).await }
            })
            .await;
        Ok(results.into_iter().flatten().next())
    }

    async fn goto_on_server(
        server: &super::LanguageServer,
        uri: &Uri,
        line: u32,
        character: u32,
        method: &str,
    ) -> Result<Option<lsp_types::Location>> {
        use lsp_types::{
            GotoDefinitionParams, GotoDefinitionResponse, Position, TextDocumentIdentifier,
            TextDocumentPositionParams,
        };

        let supported = match method {
            "textDocument/implementation" => server.supports_goto_implementation().await,
            "textDocument/typeDefinition" => server.supports_goto_type_definition().await,
            _ => server.supports_goto_definition().await,
        };
        if !supported {
            return Ok(None);
        }

        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request(method, serde_json::to_value(params)?)
            .await?;

        if result.is_null() {
            return Ok(None);
        }
        let response: Option<GotoDefinitionResponse> = parse_lsp_response(result, method);
        Ok(response.and_then(|resp| match resp {
            GotoDefinitionResponse::Scalar(location) => Some(location),
            GotoDefinitionResponse::Array(locations) => locations.into_iter().next(),
            GotoDefinitionResponse::Link(links) => {
                links.into_iter().next().map(|link| lsp_types::Location {
                    uri: link.target_uri,
                    range: link.target_selection_range,
                })
            }
        }))
    }

    /// Code actions from multiple servers, concatenating result lists.
    pub async fn code_actions_multi(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        server_ids: &[String],
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Vec<lsp_types::CodeActionOrCommand>> {
        let with_sources = self
            .code_actions_multi_with_sources(uri, line, character, server_ids, diagnostics)
            .await?;
        Ok(with_sources.into_iter().map(|(_, action)| action).collect())
    }

    /// Code actions from multiple servers with the source server_id retained.
    pub async fn code_actions_multi_with_sources(
        &self,
        uri: &Uri,
        line: u32,
        character: u32,
        server_ids: &[String],
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Vec<(String, lsp_types::CodeActionOrCommand)>> {
        use std::time::Duration;

        let mut futures = Vec::new();
        for sid in server_ids {
            let server = self
                .servers
                .get(sid.as_str())
                .map(|entry| entry.value().clone());
            if let Some(server) = server {
                let sid = sid.clone();
                let uri = uri.clone();
                let diags = diagnostics.clone();
                futures.push(tokio::spawn(async move {
                    let request =
                        Self::code_actions_on_server(&server, &uri, line, character, diags);
                    match tokio::time::timeout(Duration::from_secs(3), request).await {
                        Ok(Ok(actions)) => Some(
                            actions
                                .into_iter()
                                .map(|action| (sid.clone(), action))
                                .collect::<Vec<_>>(),
                        ),
                        Ok(Err(e)) => {
                            lsp_debug!("LSP-MULTI", "Server {} failed: {}", sid, e);
                            None
                        }
                        Err(_) => {
                            lsp_debug!("LSP-MULTI", "Server {} timed out", sid);
                            None
                        }
                    }
                }));
            }
        }

        let mut results = Vec::new();
        for task in futures {
            if let Ok(Some(mut actions)) = task.await {
                results.append(&mut actions);
            }
        }
        Ok(results)
    }

    /// Internal: code actions on a single server
    async fn code_actions_on_server(
        server: &super::LanguageServer,
        uri: &Uri,
        line: u32,
        character: u32,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Vec<lsp_types::CodeActionOrCommand>> {
        use lsp_types::{
            CodeActionContext, CodeActionParams, CodeActionTriggerKind, Position, Range,
            TextDocumentIdentifier,
        };

        if !server.supports_code_actions().await {
            return Ok(Vec::new());
        }

        let range = Range {
            start: Position { line, character },
            end: Position { line, character },
        };

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range,
            context: CodeActionContext {
                diagnostics,
                only: None,
                trigger_kind: Some(CodeActionTriggerKind::INVOKED),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = server
            .request("textDocument/codeAction", serde_json::to_value(params)?)
            .await?;

        let response: Option<Vec<lsp_types::CodeActionOrCommand>> =
            parse_lsp_response(result, "textDocument/codeAction");
        Ok(response.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::LspManager;

    fn uri() -> lsp_types::Uri {
        "file:///project/main.rs".parse().unwrap()
    }

    #[tokio::test]
    async fn execute_command_errors_when_no_server_for_language() {
        let uri = uri();
        let manager = LspManager::new();
        let err = manager
            .execute_command("dummy.command".to_string(), None, &uri, "rust")
            .await
            .expect_err("expected missing-server error");

        assert!(err.to_string().contains("No server for language: rust"));
    }

    #[tokio::test]
    async fn execute_command_errors_when_indexed_servers_are_not_available() {
        let uri = uri();
        let manager = LspManager::new();
        manager
            .language_server_index
            .insert("rust".to_string(), vec!["rust".to_string()]);

        let err = manager
            .execute_command("dummy.command".to_string(), None, &uri, "rust")
            .await
            .expect_err("expected execute-command capability error");

        assert!(err
            .to_string()
            .contains("No server for language 'rust' supports workspace/executeCommand"));
    }
}

#[cfg(all(test, unix))]
mod navigation_routing_tests {
    use super::*;

    #[tokio::test]
    async fn navigation_uses_the_document_root_server_even_when_it_is_the_only_one() {
        // Exercise the actual JSON-RPC transport and LocationLink conversion.
        let script = r#"
import json, sys
while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line == b'\r\n': break
        key, value = line.decode().split(':', 1)
        headers[key.lower()] = value.strip()
    request = json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    if 'id' not in request: continue
    method = request['method']
    if method == 'initialize':
        result = {'capabilities': {'definitionProvider': True, 'implementationProvider': True, 'typeDefinitionProvider': True}}
    elif method == 'shutdown': result = None
    else:
        assert request['params']['position'] == {'line': 2, 'character': 7}
        result = [{'targetUri': 'file:///workspace/nested/target.rs', 'targetRange': {'start': {'line': 0, 'character': 0}, 'end': {'line': 9, 'character': 0}}, 'targetSelectionRange': {'start': {'line': 3, 'character': 4}, 'end': {'line': 3, 'character': 8}}}]
    body = json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n' % len(body)).encode() + body)
    sys.stdout.buffer.flush()
"#;
        let mut server = super::super::LanguageServer::spawn(
            "rust",
            "python3",
            vec!["-u".into(), "-c".into(), script.into()],
        )
        .await
        .unwrap();
        server
            .initialize("file:///workspace/nested".parse().unwrap())
            .await
            .unwrap();
        let manager = LspManager::new();
        let id = "rust@nested".to_string();
        manager.servers.insert(id.clone(), server.clone());
        manager
            .language_server_index
            .insert("rust".into(), vec![id.clone()]);
        manager.server_roots.insert(id, "/workspace/nested".into());
        let uri = "file:///workspace/nested/source.rs".parse().unwrap();
        for method in [
            "textDocument/definition",
            "textDocument/implementation",
            "textDocument/typeDefinition",
        ] {
            let result = manager
                .goto_for_document(&uri, 2, 7, "rust", method)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(result.range.start, lsp_types::Position::new(3, 4));
            assert_eq!(result.uri.as_str(), "file:///workspace/nested/target.rs");
        }
        assert!(manager
            .goto_definition(
                &"file:///elsewhere/source.rs".parse().unwrap(),
                2,
                7,
                "rust"
            )
            .await
            .is_err());
        server.shutdown().await.unwrap();
    }
}
