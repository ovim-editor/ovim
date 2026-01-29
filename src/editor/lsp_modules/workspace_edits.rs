//! LSP workspace edit application
//!
//! This module handles applying text edits and workspace edits from LSP responses.
//! Used by rename, code actions, formatting, and organize imports.

use super::super::Editor;
use crate::lsp::uri_to_file_path;
use anyhow::Result;

/// Extract `TextEdit` values from a slice of `OneOf<TextEdit, AnnotatedTextEdit>`.
pub(in crate::editor) fn extract_text_edits(
    edits: &[lsp_types::OneOf<lsp_types::TextEdit, lsp_types::AnnotatedTextEdit>],
) -> Vec<lsp_types::TextEdit> {
    edits
        .iter()
        .map(|e| match e {
            lsp_types::OneOf::Left(edit) => edit.clone(),
            lsp_types::OneOf::Right(annot_edit) => annot_edit.text_edit.clone(),
        })
        .collect()
}

impl Editor {
    /// Apply LSP text edits to the current buffer
    pub(in crate::editor) fn apply_lsp_edits(&mut self, edits: Vec<lsp_types::TextEdit>) {
        // Sort edits in reverse order (bottom to top) to maintain correct positions
        let mut sorted_edits = edits;
        sorted_edits.sort_by(|a, b| {
            b.range
                .start
                .line
                .cmp(&a.range.start.line)
                .then(b.range.start.character.cmp(&a.range.start.character))
        });

        for edit in sorted_edits {
            let start_line = edit.range.start.line as usize;
            let end_line = edit.range.end.line as usize;
            let start_col = self.utf16_to_col(start_line, edit.range.start.character);
            let end_col = self.utf16_to_col(end_line, edit.range.end.character);

            self.buffer_mut()
                .delete_range(start_line, start_col, end_line, end_col);
            self.buffer_mut()
                .insert_text_at(start_line, start_col, &edit.new_text);
        }
    }

    /// Apply a workspace edit (used for rename, organize imports, etc.)
    pub async fn apply_workspace_edit(
        &mut self,
        edit: lsp_types::WorkspaceEdit,
    ) -> Result<bool> {
        let mut all_applied = true;
        let mut modified_files = Vec::new();

        // Handle `changes` (deprecated but still widely used)
        if let Some(changes) = edit.changes {
            for (uri, text_edits) in changes {
                if let Some(buffer_index) = self.find_or_load_buffer_index_by_uri(&uri) {
                    if let Some(path) = uri_to_file_path(&uri) {
                        let file_name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown");
                        if !modified_files.contains(&file_name.to_string()) {
                            modified_files.push(file_name.to_string());
                        }
                    }

                    if !self.apply_lsp_edits_to_buffer_index(buffer_index, text_edits) {
                        all_applied = false;
                    }
                } else {
                    all_applied = false;
                }
            }
        }

        // Handle `document_changes` (newer, more powerful format)
        if let Some(document_changes) = edit.document_changes {
            match document_changes {
                lsp_types::DocumentChanges::Edits(edits) => {
                    for text_doc_edit in edits {
                        let uri = &text_doc_edit.text_document.uri;

                        if let Some(buffer_index) =
                            self.find_or_load_buffer_index_by_uri(uri)
                        {
                            Self::track_modified_file(uri, &mut modified_files);
                            let text_edits = extract_text_edits(&text_doc_edit.edits);
                            if !self
                                .apply_lsp_edits_to_buffer_index(buffer_index, text_edits)
                            {
                                all_applied = false;
                            }
                        } else {
                            all_applied = false;
                        }
                    }
                }
                lsp_types::DocumentChanges::Operations(ops) => {
                    for op in ops {
                        match op {
                            lsp_types::DocumentChangeOperation::Edit(text_doc_edit) => {
                                let uri = &text_doc_edit.text_document.uri;

                                if let Some(buffer_index) =
                                    self.find_or_load_buffer_index_by_uri(uri)
                                {
                                    Self::track_modified_file(uri, &mut modified_files);
                                    let text_edits =
                                        extract_text_edits(&text_doc_edit.edits);
                                    if !self.apply_lsp_edits_to_buffer_index(
                                        buffer_index,
                                        text_edits,
                                    ) {
                                        all_applied = false;
                                    }
                                } else {
                                    all_applied = false;
                                }
                            }
                            lsp_types::DocumentChangeOperation::Op(resource_op) => {
                                if !Self::apply_resource_op(resource_op) {
                                    all_applied = false;
                                }
                            }
                        }
                    }
                }
            }
        }

        if !modified_files.is_empty() {
            let summary = if modified_files.len() == 1 {
                format!("Modified {}", modified_files[0])
            } else {
                format!("Modified {} files", modified_files.len())
            };
            self.set_lsp_status(summary);
        }

        Ok(all_applied)
    }

    /// Track a modified file by URI into the list.
    fn track_modified_file(uri: &lsp_types::Uri, modified_files: &mut Vec<String>) {
        if let Some(path) = uri_to_file_path(uri) {
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            if !modified_files.contains(&file_name) {
                modified_files.push(file_name);
            }
        }
    }

    /// Apply a resource operation (create, rename, delete).
    fn apply_resource_op(resource_op: lsp_types::ResourceOp) -> bool {
        match resource_op {
            lsp_types::ResourceOp::Create(create_file) => {
                let Some(file_path) = uri_to_file_path(&create_file.uri) else {
                    return false;
                };

                let should_create = create_file
                    .options
                    .as_ref()
                    .map(|opts| {
                        if file_path.exists() {
                            opts.overwrite.unwrap_or(false)
                        } else {
                            true
                        }
                    })
                    .unwrap_or(!file_path.exists());

                !should_create || std::fs::write(&file_path, "").is_ok()
            }
            lsp_types::ResourceOp::Rename(rename_file) => {
                let Some(old_path) = uri_to_file_path(&rename_file.old_uri) else {
                    return false;
                };
                let Some(new_path) = uri_to_file_path(&rename_file.new_uri) else {
                    return false;
                };

                if let Some(parent) = new_path.parent() {
                    if !parent.exists() && std::fs::create_dir_all(parent).is_err() {
                        return false;
                    }
                }

                std::fs::rename(&old_path, &new_path).is_ok()
            }
            lsp_types::ResourceOp::Delete(delete_file) => {
                let Some(file_path) = uri_to_file_path(&delete_file.uri) else {
                    return false;
                };

                !file_path.exists() || std::fs::remove_file(&file_path).is_ok()
            }
        }
    }
}
