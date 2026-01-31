//! LSP code actions, formatting, and refactoring
//!
//! This module handles:
//! - Document formatting
//! - Code actions (quick fixes, refactors)
//! - Organize imports
//! - Rename symbol
//! - Semantic tokens

use super::super::Editor;
use super::workspace_edits::extract_text_edits;
use crate::lsp::uri_to_file_path;
use anyhow::Result;

impl Editor {
    pub(in crate::editor) async fn format_document_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("format").await?;

        self.set_lsp_status("Formatting document...".to_string());

        // Get tab settings from buffer
        let tab_size = 4; // TODO: get from config
        let insert_spaces = true; // TODO: get from config

        let result = ctx
            .lsp
            .format_document(&ctx.uri, &ctx.language_id, tab_size, insert_spaces)
            .await;

        match result {
            Ok(edits) if !edits.is_empty() => {
                self.apply_lsp_edits(edits);
                self.set_lsp_status("Document formatted".to_string());
                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No formatting changes".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Format request failed: {}", e));
                Err(e)
            }
        }
    }

    pub(in crate::editor) async fn code_actions_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("code actions").await?;

        self.set_lsp_status("Fetching code actions...".to_string());

        // Get diagnostics for the current line to provide context for code actions
        let diagnostics = ctx
            .lsp
            .get_diagnostics_for_line(&ctx.uri, ctx.line)
            .await;
        let result = if ctx.server_ids.len() > 1 {
            ctx.lsp
                .code_actions_multi(
                    &ctx.uri,
                    ctx.line,
                    ctx.character,
                    &ctx.server_ids,
                    diagnostics,
                )
                .await
        } else {
            ctx.lsp
                .code_actions(
                    &ctx.uri,
                    ctx.line,
                    ctx.character,
                    &ctx.language_id,
                    diagnostics,
                )
                .await
        };

        match result {
            Ok(actions) if !actions.is_empty() => {
                let titles: Vec<String> = actions
                    .iter()
                    .map(|a| match a {
                        lsp_types::CodeActionOrCommand::CodeAction(ca) => ca.title.clone(),
                        lsp_types::CodeActionOrCommand::Command(cmd) => cmd.title.clone(),
                    })
                    .collect();

                self.lsp_state.available_code_actions = actions;

                let base_dir =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let picker =
                    crate::editor::picker::Picker::new_custom(base_dir, titles);
                self.set_picker(picker);
                self.set_mode(crate::mode::Mode::Picker);
                self.mark_picker_selection_changed();

                Ok(true)
            }
            Ok(_) => {
                self.set_lsp_status("No code actions available".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Code actions request failed: {}", e));
                Err(e)
            }
        }
    }

    /// Apply a code action by index from available code actions
    pub fn apply_code_action(&mut self, action_index: usize) {
        if action_index >= self.lsp_state.available_code_actions.len() {
            self.set_lsp_status("Invalid code action index".to_string());
            return;
        }

        let action = self.lsp_state.available_code_actions[action_index].clone();

        match action {
            lsp_types::CodeActionOrCommand::CodeAction(code_action) => {
                let workspace_edit = match code_action.edit {
                    Some(edit) => edit,
                    None => {
                        self.set_lsp_status("Code action has no edit".to_string());
                        return;
                    }
                };

                // Apply workspace edits: handle `changes`
                if let Some(changes) = &workspace_edit.changes {
                    for (uri, edits) in changes {
                        if let Some(path) = uri_to_file_path(uri) {
                            let current_path =
                                self.buffer().file_path().map(|s| s.to_string());

                            if current_path.as_deref()
                                == Some(path.to_string_lossy().as_ref())
                            {
                                self.apply_lsp_edits(edits.clone());
                            }
                            // Silently skip edits for other files (not yet supported)
                        }
                    }
                }

                // Apply workspace edits: handle `document_changes`
                if let Some(document_changes) = &workspace_edit.document_changes {
                    match document_changes {
                        lsp_types::DocumentChanges::Edits(edits) => {
                            for text_doc_edit in edits {
                                if let Some(path) =
                                    uri_to_file_path(&text_doc_edit.text_document.uri)
                                {
                                    let current_path =
                                        self.buffer().file_path().map(|s| s.to_string());

                                    if current_path.as_deref()
                                        == Some(path.to_string_lossy().as_ref())
                                    {
                                        let text_edits =
                                            extract_text_edits(&text_doc_edit.edits);
                                        self.apply_lsp_edits(text_edits);
                                    }
                                }
                            }
                        }
                        lsp_types::DocumentChanges::Operations(ops) => {
                            for op in ops {
                                if let lsp_types::DocumentChangeOperation::Edit(
                                    text_doc_edit,
                                ) = op
                                {
                                    if let Some(path) = uri_to_file_path(
                                        &text_doc_edit.text_document.uri,
                                    ) {
                                        let current_path = self
                                            .buffer()
                                            .file_path()
                                            .map(|s| s.to_string());

                                        if current_path.as_deref()
                                            == Some(path.to_string_lossy().as_ref())
                                        {
                                            let text_edits = extract_text_edits(
                                                &text_doc_edit.edits,
                                            );
                                            self.apply_lsp_edits(text_edits);
                                        }
                                    }
                                }
                                // Silently skip resource operations
                            }
                        }
                    }
                }

                self.set_lsp_status("Code action applied".to_string());
            }
            lsp_types::CodeActionOrCommand::Command(command) => {
                let lsp = match &self.lsp_state.lsp_manager {
                    Some(lsp) => lsp.clone(),
                    None => {
                        self.set_lsp_status("LSP not available".to_string());
                        return;
                    }
                };

                let language_id = match self.buffer().file_path() {
                    Some(path) => {
                        match crate::syntax::LanguageRegistry::get_lsp_language_id(path) {
                            Some(id) => id,
                            None => {
                                self.set_lsp_status(
                                    "Language not supported for LSP".to_string(),
                                );
                                return;
                            }
                        }
                    }
                    None => {
                        self.set_lsp_status(
                            "No file open for command execution".to_string(),
                        );
                        return;
                    }
                };

                let command_str = command.command.clone();
                let command_args = command.arguments.clone();
                tokio::spawn(async move {
                    let _ = lsp
                        .execute_command(command_str, command_args, language_id)
                        .await;
                });

                self.set_lsp_status("Executing code action command...".to_string());
            }
        }

        self.lsp_state.available_code_actions.clear();
    }

    pub(in crate::editor) async fn organize_imports_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("organize imports").await?;

        self.set_lsp_status("Organizing imports...".to_string());

        // Request code actions for organize imports (at file start, no diagnostics needed)
        let diagnostics = Vec::new();
        let result = if ctx.server_ids.len() > 1 {
            ctx.lsp
                .code_actions_multi(&ctx.uri, 0, 0, &ctx.server_ids, diagnostics)
                .await
        } else {
            ctx.lsp
                .code_actions(&ctx.uri, 0, 0, &ctx.language_id, diagnostics)
                .await
        };

        match result {
            Ok(actions) => {
                let organize_action = actions.into_iter().find(|action| match action {
                    lsp_types::CodeActionOrCommand::CodeAction(code_action) => {
                        code_action.edit.as_ref().is_some_and(|edit| {
                            edit.changes.is_some() || edit.document_changes.is_some()
                        })
                    }
                    lsp_types::CodeActionOrCommand::Command(cmd) => {
                        cmd.command.contains("organizeImports")
                    }
                });

                if let Some(action) = organize_action {
                    self.lsp_state.available_code_actions = vec![action];
                    self.apply_code_action(0);
                    self.set_lsp_status("Imports organized".to_string());
                    Ok(true)
                } else {
                    self.set_lsp_status("No organize imports action available".to_string());
                    Ok(false)
                }
            }
            Err(e) => {
                self.set_lsp_status(format!("Organize imports failed: {}", e));
                Err(e)
            }
        }
    }

    pub(in crate::editor) async fn rename_impl(&mut self, new_name: String) -> Result<bool> {
        let ctx = self.prepare_lsp_request("rename").await?;

        self.set_lsp_status(format!("Renaming to '{}'...", new_name));

        let result = ctx
            .lsp
            .rename(&ctx.uri, ctx.line, ctx.character, &ctx.language_id, new_name)
            .await;

        match result {
            Ok(Some(workspace_edit)) => {
                let applied = self.apply_workspace_edit(workspace_edit).await?;
                if applied {
                    self.set_lsp_status("Rename completed".to_string());
                    Ok(true)
                } else {
                    self.set_lsp_status("Rename failed to apply".to_string());
                    Ok(false)
                }
            }
            Ok(None) => {
                self.set_lsp_status("Rename not available at this location".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Rename request failed: {}", e));
                Err(e)
            }
        }
    }

    pub(in crate::editor) async fn semantic_tokens_impl(&mut self) -> Result<bool> {
        let ctx = self.prepare_lsp_request("semantic tokens").await?;

        self.set_lsp_status("Fetching semantic tokens...".to_string());

        let result = ctx.lsp.semantic_tokens_full(&ctx.uri, &ctx.language_id).await;

        match result {
            Ok(Some(_tokens)) => {
                // TODO: Store and use semantic tokens for enhanced syntax highlighting
                self.set_lsp_status("Semantic tokens received".to_string());
                Ok(true)
            }
            Ok(None) => {
                self.set_lsp_status("No semantic tokens available".to_string());
                Ok(false)
            }
            Err(e) => {
                self.set_lsp_status(format!("Semantic tokens request failed: {}", e));
                Err(e)
            }
        }
    }
}
