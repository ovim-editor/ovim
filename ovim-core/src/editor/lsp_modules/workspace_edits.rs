//! LSP workspace edit application
//!
//! This module handles applying text edits and workspace edits from LSP responses.
//! Used by rename, code actions, formatting, and organize imports.

use super::super::{Change, Editor};
use crate::lsp::uri_to_file_path;
use anyhow::Result;
use std::path::PathBuf;

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

        let cursor_before = self.cursor_position();
        let ((), recorded_edits) = self.buffer_mut().record(|buf| {
            for edit in sorted_edits {
                let start_line = edit.range.start.line as usize;
                let end_line = edit.range.end.line as usize;
                let start_col =
                    Self::utf16_to_col_for_buffer(buf, start_line, edit.range.start.character);
                let end_col =
                    Self::utf16_to_col_for_buffer(buf, end_line, edit.range.end.character);

                if start_line != end_line || start_col != end_col {
                    buf.delete_range(start_line, start_col, end_line, end_col);
                }
                if !edit.new_text.is_empty() {
                    // LSP servers running on Windows (or returning text from
                    // CRLF source files) ship `\r\n` in TextEdit.newText.
                    // The rope is LF-only by convention — normalize at the
                    // seam (OV-00251).
                    let new_text = crate::buffer::normalize_for_buffer(&edit.new_text);
                    buf.insert_text_at(start_line, start_col, new_text.as_ref());
                }
            }
        });

        if !recorded_edits.is_empty() {
            let cursor_after = self.cursor_position();
            self.push_recorded_undo(recorded_edits, cursor_before, cursor_after);
        }

        // LSP-applied edits are still edits: ensure we sync back to the server so
        // diagnostics and other LSP features refresh.
        self.invalidate_hover_cache();
        self.mark_buffer_modified_force_send();
        self.request_diagnostics_refresh();
    }

    /// Apply a workspace edit (used for rename, organize imports, etc.)
    pub fn apply_workspace_edit(&mut self, edit: lsp_types::WorkspaceEdit) -> Result<bool> {
        let mut all_applied = true;
        let mut modified_files = Vec::new();

        // LSP spec: when `document_changes` is present, `changes` is ignored.
        // `document_changes` is the newer, more powerful format that supports
        // versioned edits and resource operations.
        if let Some(document_changes) = edit.document_changes {
            match document_changes {
                lsp_types::DocumentChanges::Edits(edits) => {
                    for text_doc_edit in edits {
                        let uri = &text_doc_edit.text_document.uri;

                        if let Some(buffer_index) = self.find_or_load_buffer_index_by_uri(uri) {
                            Self::track_modified_file(uri, &mut modified_files);
                            let text_edits = extract_text_edits(&text_doc_edit.edits);
                            if !self.apply_lsp_edits_to_buffer_index(buffer_index, text_edits) {
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
                            lsp_types::DocumentChangeOperation::Op(resource_op) => {
                                let cursor_before = self.cursor_position();
                                let (applied, undo_change) =
                                    Self::apply_resource_op(resource_op, cursor_before);
                                if !applied {
                                    all_applied = false;
                                } else if let Some(change) = undo_change {
                                    self.push_resource_undo_change(change);
                                }
                            }
                        }
                    }
                }
            }
        } else if let Some(changes) = edit.changes {
            // Fallback: deprecated `changes` field (still widely used by older servers)
            for (uri, text_edits) in changes {
                if let Some(buffer_index) = self.find_or_load_buffer_index_by_uri(&uri) {
                    Self::track_modified_file(&uri, &mut modified_files);
                    if !self.apply_lsp_edits_to_buffer_index(buffer_index, text_edits) {
                        all_applied = false;
                    }
                } else {
                    all_applied = false;
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

    fn snapshot_paths(paths: &[PathBuf]) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        paths
            .iter()
            .map(|path| (path.clone(), Change::snapshot_file(path)))
            .collect()
    }

    fn build_resource_undo_change(
        before: Vec<(PathBuf, Option<Vec<u8>>)>,
        after: Vec<(PathBuf, Option<Vec<u8>>)>,
        cursor: crate::change::CursorPos,
    ) -> Option<Change> {
        let mut snapshots = Vec::new();
        for ((path, before_bytes), (_, after_bytes)) in before.into_iter().zip(after.into_iter()) {
            if before_bytes != after_bytes {
                snapshots.push(Change::resource_snapshot(path, before_bytes, after_bytes));
            }
        }

        if snapshots.is_empty() {
            None
        } else {
            Some(Change::resource_op(snapshots, cursor, cursor))
        }
    }

    fn push_resource_undo_change(&mut self, change: Change) {
        self.buffer_mut()
            .change_manager_mut()
            .push_undo_change_preserving_repeat(change);
    }

    /// Apply a resource operation (create, rename, delete).
    fn apply_resource_op(
        resource_op: lsp_types::ResourceOp,
        cursor: crate::change::CursorPos,
    ) -> (bool, Option<Change>) {
        match resource_op {
            lsp_types::ResourceOp::Create(create_file) => {
                let Some(file_path) = uri_to_file_path(&create_file.uri) else {
                    return (false, None);
                };
                let paths = vec![file_path.clone()];
                let before = Self::snapshot_paths(&paths);

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

                let applied = !should_create || std::fs::write(&file_path, "").is_ok();
                if !applied {
                    return (false, None);
                }
                let after = Self::snapshot_paths(&paths);
                (
                    true,
                    Self::build_resource_undo_change(before, after, cursor),
                )
            }
            lsp_types::ResourceOp::Rename(rename_file) => {
                let Some(old_path) = uri_to_file_path(&rename_file.old_uri) else {
                    return (false, None);
                };
                let Some(new_path) = uri_to_file_path(&rename_file.new_uri) else {
                    return (false, None);
                };
                let paths = vec![old_path.clone(), new_path.clone()];
                let before = Self::snapshot_paths(&paths);

                if let Some(parent) = new_path.parent() {
                    if !parent.exists() && std::fs::create_dir_all(parent).is_err() {
                        return (false, None);
                    }
                }

                if std::fs::rename(&old_path, &new_path).is_err() {
                    return (false, None);
                }
                let after = Self::snapshot_paths(&paths);
                (
                    true,
                    Self::build_resource_undo_change(before, after, cursor),
                )
            }
            lsp_types::ResourceOp::Delete(delete_file) => {
                let Some(file_path) = uri_to_file_path(&delete_file.uri) else {
                    return (false, None);
                };
                let paths = vec![file_path.clone()];
                let before = Self::snapshot_paths(&paths);

                let applied = !file_path.exists() || std::fs::remove_file(&file_path).is_ok();
                if !applied {
                    return (false, None);
                }
                let after = Self::snapshot_paths(&paths);
                (
                    true,
                    Self::build_resource_undo_change(before, after, cursor),
                )
            }
        }
    }
}
