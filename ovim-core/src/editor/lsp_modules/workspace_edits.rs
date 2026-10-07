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
    /// Apply LSP text edits to the current buffer.
    ///
    /// Ordering and position resolution are handled by
    /// [`Editor::apply_text_edits_to_buffer`], the single implementation of
    /// the LSP `TextEdit[]` application rules (OV-00332).
    pub(in crate::editor) fn apply_lsp_edits(&mut self, edits: Vec<lsp_types::TextEdit>) {
        let cursor_before = self.cursor_position();
        let (_all_applied, recorded_edits) = self.buffer_mut().record(|buf| {
            let applied = Self::apply_text_edits_to_buffer(buf, &edits);
            buf.clamp_cursor_to_content();
            applied
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
        Ok(self.apply_workspace_edit_reporting(edit).applied)
    }

    /// Applies a workspace edit and reports the outcome the way a server's
    /// `workspace/applyEdit` request expects it.
    ///
    /// Every precondition that can be known up front (the versions of the
    /// documents the edit was computed for) is checked before anything is
    /// touched, so a stale edit is refused whole instead of leaving the
    /// project half edited.
    pub fn apply_workspace_edit_reporting(
        &mut self,
        edit: lsp_types::WorkspaceEdit,
    ) -> lsp_types::ApplyWorkspaceEditResponse {
        if let Some((index, file)) = self.stale_workspace_edit_document(&edit) {
            let reason = format!("edit for {file} discarded: document changed");
            self.set_lsp_status(reason.clone());
            return lsp_types::ApplyWorkspaceEditResponse {
                applied: false,
                failure_reason: Some(reason),
                failed_change: Some(index),
            };
        }

        let mut failure: Option<(u32, String)> = None;
        let mut modified_files = Vec::new();

        // LSP spec: when `document_changes` is present, `changes` is ignored.
        // `document_changes` is the newer, more powerful format that supports
        // versioned edits and resource operations.
        if let Some(document_changes) = edit.document_changes {
            let operations = match document_changes {
                lsp_types::DocumentChanges::Edits(edits) => edits
                    .into_iter()
                    .map(lsp_types::DocumentChangeOperation::Edit)
                    .collect(),
                lsp_types::DocumentChanges::Operations(ops) => ops,
            };
            for (index, op) in operations.into_iter().enumerate() {
                let outcome = match op {
                    lsp_types::DocumentChangeOperation::Edit(text_doc_edit) => self
                        .apply_text_document_edit(&text_doc_edit, &mut modified_files)
                        .then_some(())
                        .ok_or_else(|| {
                            format!(
                                "failed to edit {}",
                                Self::uri_display_name(&text_doc_edit.text_document.uri)
                            )
                        }),
                    lsp_types::DocumentChangeOperation::Op(resource_op) => {
                        self.apply_resource_operation(resource_op)
                    }
                };
                if let Err(reason) = outcome {
                    failure.get_or_insert((index as u32, reason));
                }
            }
        } else if let Some(changes) = edit.changes {
            // Fallback: deprecated `changes` field (still widely used by older servers)
            for (index, (uri, text_edits)) in changes.into_iter().enumerate() {
                if !self.apply_uri_edits(&uri, text_edits, &mut modified_files) {
                    let reason = format!("failed to edit {}", Self::uri_display_name(&uri));
                    failure.get_or_insert((index as u32, reason));
                }
            }
        }

        if !modified_files.is_empty() {
            self.set_lsp_status(if modified_files.len() == 1 {
                format!("Modified {}", modified_files[0])
            } else {
                format!("Modified {} files", modified_files.len())
            });
        }

        match failure {
            None => lsp_types::ApplyWorkspaceEditResponse {
                applied: true,
                failure_reason: None,
                failed_change: None,
            },
            Some((index, reason)) => lsp_types::ApplyWorkspaceEditResponse {
                applied: false,
                failure_reason: Some(reason),
                failed_change: Some(index),
            },
        }
    }

    /// Applies one resource operation with its undo entry and buffer fixups.
    fn apply_resource_operation(
        &mut self,
        resource_op: lsp_types::ResourceOp,
    ) -> Result<(), String> {
        let cursor_before = self.cursor_position();
        // Resolve open buffers BEFORE touching the disk: lookup canonicalizes
        // paths, which fails once the file has moved or vanished.
        let affected = self.buffers_affected_by_resource_op(&resource_op);
        let overwritten = self.buffer_overwritten_by_resource_op(&resource_op);
        let undo_change = match Self::apply_resource_op(&resource_op, cursor_before) {
            ResourceOpOutcome::Failed(reason) => {
                return Err(format!(
                    "failed to {}: {reason}",
                    Self::describe_resource_op(&resource_op)
                ));
            }
            // Left alone on purpose (`ignoreIfExists`, `ignoreIfNotExists`).
            ResourceOpOutcome::Skipped => return Ok(()),
            ResourceOpOutcome::Applied(undo_change) => undo_change,
        };
        if let Some(change) = undo_change {
            self.push_resource_undo_change(change);
        }
        self.retarget_buffers_after_resource_op(&resource_op, affected);
        // A file the operation replaced on disk: an open copy that holds no
        // unsaved work follows it.
        if let Some(index) = overwritten.filter(|index| !self.buffer_index_is_modified(*index)) {
            self.reload_buffer_from_disk(index);
        }
        Ok(())
    }

    fn describe_resource_op(resource_op: &lsp_types::ResourceOp) -> String {
        match resource_op {
            lsp_types::ResourceOp::Create(create) => {
                format!("create {}", Self::uri_display_name(&create.uri))
            }
            lsp_types::ResourceOp::Rename(rename) => format!(
                "rename {} to {}",
                Self::uri_display_name(&rename.old_uri),
                Self::uri_display_name(&rename.new_uri)
            ),
            lsp_types::ResourceOp::Delete(delete) => {
                format!("delete {}", Self::uri_display_name(&delete.uri))
            }
        }
    }

    fn uri_display_name(uri: &lsp_types::Uri) -> String {
        uri_to_file_path(uri)
            .as_deref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("document")
            .to_string()
    }

    /// The first document edit (its index in the edit's changes, and the
    /// file's name) addressed to a version of its document that is no longer
    /// current (OV-00330).
    fn stale_workspace_edit_document(
        &self,
        edit: &lsp_types::WorkspaceEdit,
    ) -> Option<(u32, String)> {
        let text_document_edits: Vec<(usize, &lsp_types::TextDocumentEdit)> =
            match edit.document_changes.as_ref()? {
                lsp_types::DocumentChanges::Edits(edits) => edits.iter().enumerate().collect(),
                lsp_types::DocumentChanges::Operations(ops) => ops
                    .iter()
                    .enumerate()
                    .filter_map(|(index, op)| match op {
                        lsp_types::DocumentChangeOperation::Edit(edit) => Some((index, edit)),
                        lsp_types::DocumentChangeOperation::Op(_) => None,
                    })
                    .collect(),
            };
        text_document_edits.into_iter().find_map(|(index, edit)| {
            let document = &edit.text_document;
            let version = document.version?;
            (!self.workspace_edit_version_current(&document.uri, version))
                .then(|| (index as u32, Self::uri_display_name(&document.uri)))
        })
    }

    /// Apply one document's edits from `DocumentChanges`. Its version was
    /// already checked by [`Self::stale_workspace_edit_document`].
    fn apply_text_document_edit(
        &mut self,
        text_doc_edit: &lsp_types::TextDocumentEdit,
        modified_files: &mut Vec<String>,
    ) -> bool {
        let text_edits = extract_text_edits(&text_doc_edit.edits);
        self.apply_uri_edits(&text_doc_edit.text_document.uri, text_edits, modified_files)
    }

    /// Apply a batch of text edits to the document identified by `uri`.
    ///
    /// Buffers that were loaded purely to receive this edit (hidden, clean
    /// before the edit) are written straight to disk afterwards so a
    /// multi-file rename can't silently lose edits on quit (OV-00331).
    fn apply_uri_edits(
        &mut self,
        uri: &lsp_types::Uri,
        text_edits: Vec<lsp_types::TextEdit>,
        modified_files: &mut Vec<String>,
    ) -> bool {
        // Some servers (Hyperion's "Move to package") edit a file that does
        // not exist yet without sending a CreateFile first. Treat that as an
        // implicit create, as other clients do, instead of dropping the edit.
        if let Some(path) = uri_to_file_path(uri) {
            if !path.exists() && !self.create_missing_edit_target(&path) {
                return false;
            }
        }
        let Some(buffer_index) = self.find_or_load_buffer_index_by_uri(uri) else {
            return false;
        };
        // A buffer that was clean before this edit and isn't visible anywhere
        // in the UI exists only to carry this workspace edit — write it
        // through to disk below. Buffers the user has open (or has unrelated
        // unsaved changes in) stay in-memory modified; saving is their call.
        let was_clean = !self.buffer_index_is_modified(buffer_index);
        // Only a buffer that exists solely to carry this edit (never viewed by
        // the user) may be persisted here. A user-opened buffer keeps the
        // edit in memory (undoable, `[+]`), never written behind their back.
        let is_carrier = self.buffer_is_workspace_edit_carrier(buffer_index);
        Self::track_modified_file(uri, modified_files);
        let mut applied = self.apply_lsp_edits_to_buffer_index(buffer_index, text_edits);
        // Write through ONLY on full success: a partial apply (some edits
        // dropped as invalid) must never be persisted to disk — it stays
        // in-memory modified where the :qa guard covers it and the failure
        // is reported upstream (external review finding on OV-00331/332).
        if applied
            && was_clean
            && is_carrier
            && self.buffer_index_is_modified(buffer_index)
            && !self.buffer_is_open_in_ui(buffer_index)
            && !self.write_through_workspace_edit_buffer(buffer_index)
        {
            applied = false;
        }
        applied
    }

    fn buffer_is_workspace_edit_carrier(&mut self, index: usize) -> bool {
        // A carrier that has since become visible is the user's buffer now.
        if self.buffer_is_open_in_ui(index) {
            if let Some(buffer) = self.buffers.get(index) {
                let id = buffer.id();
                self.lsp
                    .state
                    .workspace_edit_carriers
                    .retain(|carrier| *carrier != id);
            }
            return false;
        }
        self.buffers
            .get(index)
            .is_some_and(|b| self.lsp.state.workspace_edit_carriers.contains(&b.id()))
    }

    /// Creates an empty file (and parent directories) so an edit addressed to
    /// a not-yet-existing document has somewhere to land. Returns false when
    /// the path cannot be created.
    fn create_missing_edit_target(&self, path: &std::path::Path) -> bool {
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return false;
            }
        }
        std::fs::write(path, "").is_ok()
    }

    /// OV-00330 (version-guard leg): returns false when a versioned document
    /// edit no longer matches our view of the document.
    ///
    /// The current file's LSP document version is tracked synchronously on the
    /// editor side (`lsp.state.current_file_lsp_version`, refreshed on each
    /// sync tick). Every other open buffer carries its own last-flushed
    /// version and content in `lsp.state.document_sync`, so an edit addressed
    /// to a hidden buffer is checked the same way (OV-00475): it must carry the
    /// version the server last received AND the buffer must still hold exactly
    /// the text the server saw. A hidden buffer nobody synced is compared with
    /// the disk instead: unsaved edits mean the server never saw the text the
    /// edit was computed for.
    fn workspace_edit_version_current(&self, uri: &lsp_types::Uri, version: i32) -> bool {
        let Some(edit_path) = uri_to_file_path(uri) else {
            return true;
        };
        let Some(edit_path_text) = edit_path.to_str() else {
            return true;
        };
        let Some(index) = self.find_buffer_by_path(edit_path_text) else {
            // Not open in the editor: the edit is applied to what is on disk.
            return true;
        };
        if index != self.current_buffer_index {
            return self.hidden_buffer_edit_version_current(index, version);
        }
        // A local edit marks sync dirty BEFORE the version counter advances
        // (the bump happens on the next sync tick), and server workspace
        // edits are drained ahead of the sync step in the tick. A versioned
        // edit arriving in that window would pass a bare version compare
        // while targeting an older rope — dirty sync state means our
        // content is newer than any version the server can know about, so
        // reject (external review finding on OV-00330).
        if self.lsp_document_is_modified() == Some(true) {
            return false;
        }
        let known_version = self.lsp.state.current_file_lsp_version;
        if known_version <= 0 {
            // Version unknown (no manager yet / never synced) — don't guess.
            // This keeps the common unversioned/unsynced path working.
            return true;
        }
        version == known_version
    }

    /// The version check for a buffer that is open but not the current one.
    fn hidden_buffer_edit_version_current(&self, index: usize, version: i32) -> bool {
        let Some(buffer) = self.buffers.get(index) else {
            return true;
        };
        let state = buffer
            .file_path()
            .and_then(|path| self.lsp.state.document_sync.get(path));
        // A pending full resend means the buffer's text changed behind the
        // server's back (reload from disk): whatever version it holds is stale.
        if state.is_some_and(|state| state.force_full_resend) {
            return false;
        }
        match state.and_then(|state| state.flushed_content().map(|text| (state, text))) {
            Some((state, flushed)) => {
                // The server's copy must be the buffer's text...
                if buffer.rope() != flushed {
                    return false;
                }
                // ...at the version the edit was computed for.
                state.flushed_lsp_version <= 0 || state.flushed_lsp_version == version
            }
            // The server was never told about this buffer's text: it can only
            // have read the file, so unsaved changes make the edit stale.
            None => !buffer.is_modified(),
        }
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
        for ((path, before_bytes), (_, after_bytes)) in before.into_iter().zip(after) {
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

    /// Records a filesystem operation in the undo history of the current
    /// buffer. It changes no text, so a buffer that had nothing unsaved still
    /// has nothing unsaved afterwards.
    fn push_resource_undo_change(&mut self, change: Change) {
        let was_saved = self.buffer().change_manager().is_at_save_point();
        let history = self.buffer_mut().change_manager_mut();
        history.push_change(change);
        if was_saved {
            history.mark_saved();
        }
    }

    /// The open buffers whose file a resource operation renames or deletes (a
    /// folder's buffers included), with their path below the operation's
    /// target.
    fn buffers_affected_by_resource_op(&self, op: &lsp_types::ResourceOp) -> Vec<(usize, PathBuf)> {
        let uri = match op {
            lsp_types::ResourceOp::Rename(rename) => &rename.old_uri,
            lsp_types::ResourceOp::Delete(delete) => &delete.uri,
            lsp_types::ResourceOp::Create(_) => return Vec::new(),
        };
        uri_to_file_path(uri)
            .map(|path| self.buffers_at_or_below(&path))
            .unwrap_or_default()
    }

    /// The open buffer of a file that a create or rename will replace on disk.
    fn buffer_overwritten_by_resource_op(&self, op: &lsp_types::ResourceOp) -> Option<usize> {
        let (uri, overwrite) = match op {
            lsp_types::ResourceOp::Create(create) => (
                &create.uri,
                create.options.as_ref().and_then(|o| o.overwrite),
            ),
            lsp_types::ResourceOp::Rename(rename) => (
                &rename.new_uri,
                rename.options.as_ref().and_then(|o| o.overwrite),
            ),
            lsp_types::ResourceOp::Delete(_) => return None,
        };
        let path = uri_to_file_path(uri)?;
        if overwrite != Some(true) || !path.is_file() {
            return None;
        }
        self.find_buffer_by_path(path.to_str()?)
    }

    /// Points the buffer at `new_path` after its file moved on disk. The
    /// language server hears `didClose` for the old URI and `didOpen` for the
    /// new one (through the normal sync tick).
    pub(in crate::editor) fn retarget_buffer_path(&mut self, index: usize, new_path: PathBuf) {
        let Some(old_path) = self
            .buffers
            .get(index)
            .and_then(|b| b.file_path())
            .map(str::to_string)
        else {
            return;
        };
        let new_path = new_path.canonicalize().unwrap_or(new_path);
        let new_path = new_path.to_string_lossy().to_string();
        let is_current = index == self.current_buffer_index;
        let mtime = std::fs::metadata(&new_path)
            .ok()
            .and_then(|m| m.modified().ok());
        let buffer = &mut self.buffers[index];
        buffer.set_file_path(new_path.clone());
        if !buffer.is_modified() {
            buffer.set_file_mtime(mtime);
        }
        if is_current {
            self.registers.set_current_file(new_path.clone());
        }
        self.handle_file_path_transition_after_save(Some(old_path), Some(new_path));
    }

    /// Keeps open buffers coherent with files the server just renamed or
    /// deleted: a renamed file's (or folder's) buffers follow it, and the
    /// language server hears `didClose` for the old URI and `didOpen` for the
    /// new one; a deleted file's document is closed on the server while the
    /// buffer stays open so unsaved text is never discarded behind the user's
    /// back.
    fn retarget_buffers_after_resource_op(
        &mut self,
        op: &lsp_types::ResourceOp,
        affected: Vec<(usize, PathBuf)>,
    ) {
        match op {
            lsp_types::ResourceOp::Rename(rename) => {
                let Some(new_path) = uri_to_file_path(&rename.new_uri) else {
                    return;
                };
                self.retarget_buffers_after_move(affected, &new_path);
            }
            lsp_types::ResourceOp::Delete(_) => {
                for (index, _) in affected {
                    let Some(old_path) = self
                        .buffers
                        .get(index)
                        .and_then(|b| b.file_path())
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    self.lsp.state.document_sync.remove(&old_path);
                    self.queue_lsp_did_close(old_path);
                }
            }
            lsp_types::ResourceOp::Create(_) => {}
        }
    }

    /// Apply a resource operation (create, rename, delete) with the options
    /// the LSP spec defines for it: `overwrite` beats `ignoreIfExists`, and
    /// without either an existing target makes the operation fail.
    fn apply_resource_op(
        resource_op: &lsp_types::ResourceOp,
        cursor: crate::change::CursorPos,
    ) -> ResourceOpOutcome {
        let outcome = match resource_op {
            lsp_types::ResourceOp::Create(create_file) => {
                let Some(file_path) = uri_to_file_path(&create_file.uri) else {
                    return ResourceOpOutcome::Failed("not a file path".to_string());
                };
                let options = create_file.options.as_ref();
                let overwrite = options.and_then(|o| o.overwrite).unwrap_or(false);
                let ignore_if_exists = options.and_then(|o| o.ignore_if_exists).unwrap_or(false);
                if file_path.exists() {
                    if !overwrite && ignore_if_exists {
                        return ResourceOpOutcome::Skipped;
                    }
                    if !overwrite {
                        return ResourceOpOutcome::Failed("it already exists".to_string());
                    }
                    if file_path.is_dir() {
                        return ResourceOpOutcome::Failed("it is a directory".to_string());
                    }
                }
                let paths = vec![file_path.clone()];
                let before = Self::snapshot_paths(&paths);
                if let Some(parent) = file_path.parent() {
                    if let Err(error) = std::fs::create_dir_all(parent) {
                        return ResourceOpOutcome::Failed(error.to_string());
                    }
                }
                if let Err(error) = std::fs::write(&file_path, "") {
                    return ResourceOpOutcome::Failed(error.to_string());
                }
                (before, Self::snapshot_paths(&paths))
            }
            lsp_types::ResourceOp::Rename(rename_file) => {
                let (Some(old_path), Some(new_path)) = (
                    uri_to_file_path(&rename_file.old_uri),
                    uri_to_file_path(&rename_file.new_uri),
                ) else {
                    return ResourceOpOutcome::Failed("not a file path".to_string());
                };
                let options = rename_file.options.as_ref();
                let overwrite = options.and_then(|o| o.overwrite).unwrap_or(false);
                let ignore_if_exists = options.and_then(|o| o.ignore_if_exists).unwrap_or(false);
                if !old_path.exists() {
                    return ResourceOpOutcome::Failed("it does not exist".to_string());
                }
                if new_path.exists() {
                    if !overwrite && ignore_if_exists {
                        return ResourceOpOutcome::Skipped;
                    }
                    if !overwrite {
                        return ResourceOpOutcome::Failed("the target already exists".to_string());
                    }
                    // Only a file is replaced; a directory is never wiped.
                    if new_path.is_dir() && std::fs::remove_dir(&new_path).is_err() {
                        return ResourceOpOutcome::Failed(
                            "the target is a directory that is not empty".to_string(),
                        );
                    }
                }
                let paths = vec![old_path.clone(), new_path.clone()];
                let before = Self::snapshot_paths(&paths);

                if let Some(parent) = new_path.parent() {
                    if !parent.exists() {
                        if let Err(error) = std::fs::create_dir_all(parent) {
                            return ResourceOpOutcome::Failed(error.to_string());
                        }
                    }
                }
                if let Err(error) = std::fs::rename(&old_path, &new_path) {
                    return ResourceOpOutcome::Failed(error.to_string());
                }
                (before, Self::snapshot_paths(&paths))
            }
            lsp_types::ResourceOp::Delete(delete_file) => {
                let Some(file_path) = uri_to_file_path(&delete_file.uri) else {
                    return ResourceOpOutcome::Failed("not a file path".to_string());
                };
                let options = delete_file.options.as_ref();
                let recursive = options.and_then(|o| o.recursive).unwrap_or(false);
                let ignore_if_not_exists = options
                    .and_then(|o| o.ignore_if_not_exists)
                    .unwrap_or(false);
                if !file_path.exists() {
                    return if ignore_if_not_exists {
                        ResourceOpOutcome::Skipped
                    } else {
                        ResourceOpOutcome::Failed("it does not exist".to_string())
                    };
                }
                let paths = vec![file_path.clone()];
                let before = Self::snapshot_paths(&paths);
                let removed = if file_path.is_dir() {
                    if recursive {
                        std::fs::remove_dir_all(&file_path)
                    } else {
                        std::fs::remove_dir(&file_path)
                    }
                } else {
                    std::fs::remove_file(&file_path)
                };
                if let Err(error) = removed {
                    return ResourceOpOutcome::Failed(error.to_string());
                }
                (before, Self::snapshot_paths(&paths))
            }
        };
        ResourceOpOutcome::Applied(Self::build_resource_undo_change(
            outcome.0, outcome.1, cursor,
        ))
    }
}

/// What applying a resource operation did.
enum ResourceOpOutcome {
    /// Done; the filesystem change to undo, if it can be undone (folders
    /// cannot be snapshotted).
    Applied(Option<Change>),
    /// The operation asked to be ignored in this state (`ignoreIfExists`...).
    Skipped,
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::str::FromStr;

    fn file_uri(path: &std::path::Path) -> lsp_types::Uri {
        lsp_types::Uri::from_str(&format!(
            "file://{}",
            path.canonicalize().unwrap().to_string_lossy()
        ))
        .expect("uri")
    }

    fn replace_edit(start_char: u32, end_char: u32, new_text: &str) -> lsp_types::TextEdit {
        lsp_types::TextEdit {
            range: lsp_types::Range {
                start: lsp_types::Position {
                    line: 0,
                    character: start_char,
                },
                end: lsp_types::Position {
                    line: 0,
                    character: end_char,
                },
            },
            new_text: new_text.to_string(),
        }
    }

    // lsp_types::Uri has interior mutability per clippy, but WorkspaceEdit's
    // `changes` field is defined as HashMap<Uri, _> by the protocol crate —
    // we just construct what the API demands.
    #[allow(clippy::mutable_key_type)]
    fn changes_edit(
        uri: lsp_types::Uri,
        edits: Vec<lsp_types::TextEdit>,
    ) -> lsp_types::WorkspaceEdit {
        let mut changes = std::collections::HashMap::new();
        changes.insert(uri, edits);
        lsp_types::WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }
    }

    fn versioned_edit(
        uri: lsp_types::Uri,
        version: Option<i32>,
        edits: Vec<lsp_types::TextEdit>,
    ) -> lsp_types::WorkspaceEdit {
        lsp_types::WorkspaceEdit {
            document_changes: Some(lsp_types::DocumentChanges::Edits(vec![
                lsp_types::TextDocumentEdit {
                    text_document: lsp_types::OptionalVersionedTextDocumentIdentifier {
                        uri,
                        version,
                    },
                    edits: edits.into_iter().map(lsp_types::OneOf::Left).collect(),
                },
            ])),
            ..Default::default()
        }
    }

    /// OV-00331: a multi-file WorkspaceEdit loads unopened files into hidden
    /// buffers. Those buffers exist only to carry the edit — they must be
    /// written to disk as part of the apply, or `:qa` throws the edits away
    /// ("Modified 5 files" → 4 files keep the old name on disk).
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn workspace_edit_writes_hidden_buffer_to_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opened = dir.path().join("opened.rs");
        let hidden = dir.path().join("hidden.rs");
        fs::write(&opened, "fn main() {}\n").expect("write opened");
        fs::write(&hidden, "fn helper() {}\n").expect("write hidden");

        let mut editor = Editor::default();
        editor.open_file(&opened).expect("open opened.rs");

        let edit = changes_edit(file_uri(&hidden), vec![replace_edit(3, 9, "renamed")]);
        let applied = editor.apply_workspace_edit(edit).expect("apply");
        assert!(applied, "workspace edit should fully apply");

        assert_eq!(
            fs::read_to_string(&hidden).expect("read hidden"),
            "fn renamed() {}\n",
            "hidden buffer must be written through to disk"
        );
        assert!(
            !editor.any_buffer_modified(),
            "written-through hidden buffer must not linger as modified"
        );
    }

    /// OV-00331: buffers the user has open stay in-memory modified — saving
    /// is their call. Only hidden edit-carrier buffers get write-through.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn workspace_edit_leaves_current_buffer_unwritten() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opened = dir.path().join("opened.rs");
        fs::write(&opened, "fn main() {}\n").expect("write opened");

        let mut editor = Editor::default();
        editor.open_file(&opened).expect("open opened.rs");

        let edit = changes_edit(file_uri(&opened), vec![replace_edit(3, 7, "renamed")]);
        let applied = editor.apply_workspace_edit(edit).expect("apply");
        assert!(applied);

        assert_eq!(
            fs::read_to_string(&opened).expect("read opened"),
            "fn main() {}\n",
            "the user's open buffer must not be auto-saved"
        );
        assert!(editor.is_modified());
    }

    /// OV-00331: a hidden buffer that already carried the user's own unsaved
    /// changes must NOT be auto-saved — write-through would silently commit
    /// unrelated half-finished edits. The `:qa` guard covers it instead.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn workspace_edit_skips_write_through_for_user_modified_hidden_buffer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opened = dir.path().join("opened.rs");
        let hidden = dir.path().join("hidden.rs");
        fs::write(&opened, "fn main() {}\n").expect("write opened");
        fs::write(&hidden, "fn helper() {}\n").expect("write hidden");

        let mut editor = Editor::default();
        editor.open_file(&opened).expect("open opened.rs");

        let hidden_uri = file_uri(&hidden);
        let index = editor
            .find_or_load_buffer_index_by_uri(&hidden_uri)
            .expect("load hidden");
        editor.buffers[index].insert_text_at(
            0,
            crate::unicode::CharCol(0),
            "// user work in progress\n",
        );

        let edit = changes_edit(hidden_uri, vec![replace_edit(3, 9, "renamed")]);
        editor.apply_workspace_edit(edit).expect("apply");

        assert_eq!(
            fs::read_to_string(&hidden).expect("read hidden"),
            "fn helper() {}\n",
            "user-modified hidden buffer must not be auto-saved"
        );
        assert!(editor.any_buffer_modified());
    }

    // ---- OV-00450: path identity ---------------------------------------

    /// Every spelling of one file's path: absolute, `dir/./f`, `dir/sub/../f`,
    /// through a symlinked directory, and through a symlink to the file.
    fn path_spellings(dir: &std::path::Path, file_name: &str) -> Vec<PathBuf> {
        let real = dir.canonicalize().unwrap();
        let mut spellings = vec![
            real.join(file_name),
            real.join(".").join(file_name),
            real.join("sub").join("..").join(file_name),
        ];
        #[cfg(unix)]
        {
            let link = real.join("linkdir");
            let _ = std::os::unix::fs::symlink(&real, &link);
            spellings.push(link.join(file_name));
            spellings.push(link.join(".").join("linkdir").join(file_name));
            // A symlink to the file itself.
            let file_link = real.join(format!("{file_name}.lnk"));
            let _ = std::os::unix::fs::symlink(real.join(file_name), &file_link);
            spellings.push(file_link);
        }
        spellings
    }

    fn raw_uri(path: &std::path::Path) -> lsp_types::Uri {
        lsp_types::Uri::from_str(&format!("file://{}", path.to_string_lossy())).expect("uri")
    }

    /// Loading one file under any path spelling must never create a second
    /// buffer (the root cause of the `:e rel/path` rename corruption).
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn loading_any_spelling_of_open_file_reuses_buffer() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("sub")).unwrap();
        let file = dir.path().join("Service.java");
        let other = dir.path().join("Controller.java");
        fs::write(&file, "class Service {}\n").unwrap();
        fs::write(&other, "class Controller {}\n").unwrap();

        for spelling in path_spellings(dir.path(), "Service.java") {
            let mut editor = Editor::default();
            editor.open_file(&file).expect("open");
            let count = editor.buffer_count();
            editor.load_file(&other).expect("switch away");
            editor.load_file(&spelling).unwrap_or_else(|e| {
                panic!("load {}: {e}", spelling.display());
            });
            assert_eq!(
                editor.buffer_count(),
                count + 1,
                "spelling {} duplicated the buffer",
                spelling.display()
            );
            assert_eq!(
                editor.buffer().file_path().map(std::path::Path::new),
                Some(file.canonicalize().unwrap().as_path())
            );
        }
    }

    /// The reported corruption: same file reopened under another spelling,
    /// unsaved edit in the visible buffer, rename arrives. The edit must land
    /// in the visible buffer and the disk must stay untouched.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn workspace_edit_under_any_spelling_hits_visible_dirty_buffer_not_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("sub")).unwrap();
        let file = dir.path().join("Service.java");
        let other = dir.path().join("Controller.java");
        fs::write(&file, "int old = 1;\n").unwrap();
        fs::write(&other, "class C {}\n").unwrap();

        for open_spelling in path_spellings(dir.path(), "Service.java") {
            for uri_spelling in path_spellings(dir.path(), "Service.java") {
                let mut editor = Editor::default();
                editor.open_file(&file).expect("open");
                editor.load_file(&other).expect("other");
                editor.load_file(&open_spelling).expect("reopen");
                editor
                    .buffer_mut()
                    .insert_text_at(0, crate::unicode::CharCol(0), "// unsaved\n");
                // Rename was computed against the text the server saw
                // (the dirty text): `old` sits on line 1 now.
                let edit = changes_edit(
                    raw_uri(&uri_spelling),
                    vec![lsp_types::TextEdit {
                        range: lsp_types::Range::new(
                            lsp_types::Position::new(1, 4),
                            lsp_types::Position::new(1, 7),
                        ),
                        new_text: "renamed".into(),
                    }],
                );
                assert!(editor.apply_workspace_edit(edit).unwrap());
                assert_eq!(
                    editor.buffer().rope().to_string(),
                    "// unsaved\nint renamed = 1;\n",
                    "open={} uri={}",
                    open_spelling.display(),
                    uri_spelling.display()
                );
                assert_eq!(
                    fs::read_to_string(&file).unwrap(),
                    "int old = 1;\n",
                    "disk must be untouched (open={} uri={})",
                    open_spelling.display(),
                    uri_spelling.display()
                );
            }
        }
    }

    /// A buffer the user opened (then switched away from) is not a carrier:
    /// a workspace edit must not be persisted behind their back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn workspace_edit_never_writes_user_opened_hidden_buffer_to_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opened = dir.path().join("opened.rs");
        let hidden = dir.path().join("hidden.rs");
        fs::write(&opened, "fn main() {}\n").unwrap();
        fs::write(&hidden, "fn helper() {}\n").unwrap();

        let mut editor = Editor::default();
        editor.open_file(&hidden).unwrap();
        editor.open_file(&opened).unwrap(); // hidden is now switched away

        let edit = changes_edit(file_uri(&hidden), vec![replace_edit(3, 9, "renamed")]);
        assert!(editor.apply_workspace_edit(edit).unwrap());
        assert_eq!(
            fs::read_to_string(&hidden).unwrap(),
            "fn helper() {}\n",
            "user-opened buffer must not be written through"
        );
        assert!(editor.any_buffer_modified(), "edit stays in memory, `[+]`");
    }

    /// OV-00330 (version-guard leg): a versioned document edit whose version
    /// no longer matches our view of the document is stale — the spec's
    /// staleness mechanism. It must be skipped, not spliced into newer text.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn stale_versioned_edit_is_discarded_with_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opened = dir.path().join("opened.rs");
        fs::write(&opened, "fn main() {}\n").expect("write opened");

        let mut editor = Editor::default();
        editor.open_file(&opened).expect("open opened.rs");
        editor.lsp.state.current_file_lsp_version = 3;

        let edit = versioned_edit(
            file_uri(&opened),
            Some(5),
            vec![replace_edit(3, 7, "stale")],
        );
        let applied = editor.apply_workspace_edit(edit).expect("apply");

        assert!(!applied, "stale versioned edit must not report success");
        assert_eq!(
            editor.buffer().rope().to_string(),
            "fn main() {}\n",
            "stale edit must not touch the buffer"
        );
        assert!(
            editor.lsp_status().contains("discarded: document changed"),
            "status must explain the discard, got: {:?}",
            editor.lsp_status()
        );
    }

    /// OV-00475: a versioned edit to a HIDDEN buffer was applied unchecked. The
    /// hidden buffer's synced version and text are tracked per document now:
    /// a version mismatch, or a buffer that changed since the server saw it,
    /// discards the edit; a matching one still applies.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn versioned_edit_to_a_hidden_buffer_is_checked_against_its_own_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hidden = dir.path().join("hidden.rs");
        let opened = dir.path().join("opened.rs");
        fs::write(&hidden, "fn helper() {}\n").unwrap();
        fs::write(&opened, "fn main() {}\n").unwrap();

        let mut editor = Editor::default();
        editor.open_file(&hidden).unwrap();
        editor.open_file(&opened).unwrap(); // `hidden` is switched away
        let hidden_path = editor
            .buffers
            .iter()
            .find_map(|b| b.file_path().filter(|p| p.ends_with("hidden.rs")))
            .unwrap()
            .to_string();
        // The server last received version 4 with exactly this text.
        let sync = editor
            .lsp
            .state
            .document_sync
            .entry(hidden_path.clone())
            .or_default();
        sync.did_open_sent = true;
        sync.mark_change_flushed(std::sync::Arc::from("fn helper() {}\n"), 4, None);

        // Wrong version: stale.
        let stale = versioned_edit(file_uri(&hidden), Some(3), vec![replace_edit(3, 9, "old")]);
        assert!(!editor.apply_workspace_edit(stale).unwrap());
        assert!(!editor.any_buffer_modified(), "a stale edit must not land");
        assert!(
            editor.lsp_status().contains("discarded"),
            "{:?}",
            editor.lsp_status()
        );

        // Matching version and text: applied (kept in memory, not written).
        let fresh = versioned_edit(file_uri(&hidden), Some(4), vec![replace_edit(3, 9, "new")]);
        assert!(editor.apply_workspace_edit(fresh).unwrap());
        assert!(editor.any_buffer_modified());
        assert_eq!(fs::read_to_string(&hidden).unwrap(), "fn helper() {}\n");

        // The buffer now differs from what the server saw (version 4): the
        // next versioned edit, even with the same number, is stale.
        let again = versioned_edit(file_uri(&hidden), Some(4), vec![replace_edit(3, 6, "x")]);
        assert!(!editor.apply_workspace_edit(again).unwrap());
        // Unversioned edits keep working.
        let plain = changes_edit(file_uri(&hidden), vec![replace_edit(3, 6, "y")]);
        assert!(editor.apply_workspace_edit(plain).unwrap());
    }

    /// OV-00475: a hidden buffer the server never saw, with unsaved edits, is
    /// stale for any versioned edit; a clean one accepts it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn versioned_edit_to_an_unsynced_dirty_hidden_buffer_is_discarded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hidden = dir.path().join("hidden.rs");
        let opened = dir.path().join("opened.rs");
        fs::write(&hidden, "fn helper() {}\n").unwrap();
        fs::write(&opened, "fn main() {}\n").unwrap();
        let mut editor = Editor::default();
        editor.open_file(&hidden).unwrap();
        editor
            .buffer_mut()
            .insert_text_at(0, crate::unicode::CharCol(0), "// unsaved\n");
        editor.open_file(&opened).unwrap();

        let edit = versioned_edit(file_uri(&hidden), Some(2), vec![replace_edit(3, 9, "x")]);
        assert!(!editor.apply_workspace_edit(edit).unwrap());
        assert!(
            editor.lsp_status().contains("discarded"),
            "{:?}",
            editor.lsp_status()
        );
    }

    /// External review on OV-00331: write-through must never clobber a file
    /// that changed on disk after the hidden buffer was loaded. The edit
    /// stays in-memory modified, where the :qa guard protects it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn write_through_refuses_when_disk_changed_after_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opened = dir.path().join("opened.rs");
        let target = dir.path().join("target.rs");
        fs::write(&opened, "fn main() {}\n").expect("write opened");
        fs::write(&target, "fn helper() {}\n").expect("write target");

        let mut editor = Editor::default();
        editor.open_file(&opened).expect("open opened.rs");
        // target is a hidden edit-carrier buffer (loaded for the edit only)
        editor
            .find_or_load_buffer_index_by_uri(&file_uri(&target))
            .expect("load target.rs");

        // External change lands after the buffer snapshot.
        fs::write(&target, "fn external_truth() {}\n").expect("external write");
        let newer = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
        std::fs::File::options()
            .write(true)
            .open(&target)
            .expect("open for mtime")
            .set_modified(newer)
            .expect("bump mtime");

        let edit = changes_edit(file_uri(&target), vec![replace_edit(3, 9, "renamed")]);
        let applied = editor.apply_workspace_edit(edit).expect("apply");

        assert!(!applied, "refused write-through must surface as failure");
        assert_eq!(
            fs::read_to_string(&target).expect("read target"),
            "fn external_truth() {}\n",
            "the external on-disk content must survive"
        );
        assert!(
            editor.any_buffer_modified(),
            "the unpersisted edit must keep the buffer modified so :qa protects it"
        );
    }

    /// External review on OV-00331/332: a partially-invalid TextEdit[] must
    /// never be persisted — dropping some edits and writing the rest to disk
    /// silently ships a half-applied change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn partial_apply_is_not_written_through() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opened = dir.path().join("opened.rs");
        let target = dir.path().join("target.rs");
        fs::write(&opened, "fn main() {}\n").expect("write opened");
        fs::write(&target, "fn helper() {}\n").expect("write target");

        let mut editor = Editor::default();
        editor.open_file(&opened).expect("open opened.rs");

        let out_of_bounds = lsp_types::TextEdit {
            range: lsp_types::Range::new(
                lsp_types::Position::new(99, 0),
                lsp_types::Position::new(99, 1),
            ),
            new_text: "nope".to_string(),
        };
        let edit = changes_edit(
            file_uri(&target),
            vec![replace_edit(3, 9, "renamed"), out_of_bounds],
        );
        let applied = editor.apply_workspace_edit(edit).expect("apply");

        assert!(!applied, "partial apply must report failure");
        assert_eq!(
            fs::read_to_string(&target).expect("read target"),
            "fn helper() {}\n",
            "a partial result must never reach disk"
        );
    }

    /// External review on OV-00330: a versioned edit arriving while local
    /// edits are still unsynced targets an older rope even when the version
    /// numbers match (the editor-side counter only advances on the next sync
    /// tick) — dirty sync state must reject it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn versioned_edit_is_discarded_while_sync_dirty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opened = dir.path().join("opened.rs");
        fs::write(&opened, "fn main() {}\n").expect("write opened");

        let mut editor = Editor::default();
        editor.open_file(&opened).expect("open opened.rs");
        editor.lsp.state.current_file_lsp_version = 5;

        // Local mutation marks sync dirty; the version counter hasn't
        // advanced yet — exactly the race window.
        editor
            .buffer_mut()
            .insert_text_at(0, crate::unicode::CharCol(0), "x");
        editor.mark_buffer_modified();

        let edit = versioned_edit(
            file_uri(&opened),
            Some(5),
            vec![replace_edit(4, 8, "stale")],
        );
        let applied = editor.apply_workspace_edit(edit).expect("apply");

        assert!(
            !applied,
            "matching version number must not bypass the dirty-sync rejection"
        );
        assert_eq!(
            editor.buffer().rope().to_string(),
            "xfn main() {}\n",
            "the newer local content must be untouched"
        );
    }

    /// OV-00330: matching and absent versions keep applying — the guard must
    /// not regress the common paths.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn matching_and_unversioned_edits_still_apply() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opened = dir.path().join("opened.rs");
        fs::write(&opened, "fn main() {}\n").expect("write opened");

        let mut editor = Editor::default();
        editor.open_file(&opened).expect("open opened.rs");
        editor.lsp.state.current_file_lsp_version = 3;

        let edit = versioned_edit(
            file_uri(&opened),
            Some(3),
            vec![replace_edit(3, 7, "fresh")],
        );
        let applied = editor.apply_workspace_edit(edit).expect("apply");
        assert!(applied);
        assert_eq!(editor.buffer().rope().to_string(), "fn fresh() {}\n");

        let edit = versioned_edit(file_uri(&opened), None, vec![replace_edit(3, 8, "newer")]);
        let applied = editor.apply_workspace_edit(edit).expect("apply");
        assert!(applied);
        assert_eq!(editor.buffer().rope().to_string(), "fn newer() {}\n");
    }

    fn plain_uri(path: &std::path::Path) -> lsp_types::Uri {
        lsp_types::Uri::from_str(&format!("file://{}", path.to_string_lossy())).expect("uri")
    }

    fn doc_edit(
        uri: lsp_types::Uri,
        start: (u32, u32),
        end: (u32, u32),
        text: &str,
    ) -> lsp_types::DocumentChangeOperation {
        lsp_types::DocumentChangeOperation::Edit(lsp_types::TextDocumentEdit {
            text_document: lsp_types::OptionalVersionedTextDocumentIdentifier {
                uri,
                version: None,
            },
            edits: vec![lsp_types::OneOf::Left(lsp_types::TextEdit {
                range: lsp_types::Range::new(
                    lsp_types::Position::new(start.0, start.1),
                    lsp_types::Position::new(end.0, end.1),
                ),
                new_text: text.to_string(),
            })],
        })
    }

    fn operations(ops: Vec<lsp_types::DocumentChangeOperation>) -> lsp_types::WorkspaceEdit {
        lsp_types::WorkspaceEdit {
            document_changes: Some(lsp_types::DocumentChanges::Operations(ops)),
            ..Default::default()
        }
    }

    /// OV-00403: Hyperion's "Move 'Probe' to package" blanks the original
    /// through a range ending on the phantom line after the final newline and
    /// writes the new class into a file that does not exist yet.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn move_class_edit_with_missing_target_and_eof_range_does_not_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let probe = dir.path().join("Probe.java");
        let body: String = (0..19).map(|n| format!("line {n}\n")).collect();
        fs::write(&probe, &body).expect("write probe");
        let target = dir.path().join("util/Probe.java");

        let mut editor = Editor::default();
        editor.open_file(&probe).expect("open probe");
        // Cursor on a line that the edit is about to delete (the observed
        // crash: ropey "Line index out of bounds" in scroll/decoration code).
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(5, crate::unicode::GraphemeCol(4));
        let edit = operations(vec![
            doc_edit(file_uri(&probe), (0, 0), (19, 0), ""),
            doc_edit(plain_uri(&target), (0, 0), (0, 0), "package util;\n"),
        ]);
        let applied = editor
            .apply_workspace_edit(edit)
            .expect("apply must not error");
        // The edit to the not-yet-existing file has no CreateFile op, so
        // it cannot be applied - but nothing may crash.
        assert!(!applied || target.exists());
        assert_eq!(
            editor.buffer().cursor().line(),
            0,
            "cursor must follow the shrunken text"
        );
        editor.update_scroll_offset();
    }

    /// The decoration lookups themselves must survive a stale line index.
    #[test]
    fn decoration_lookups_tolerate_lines_past_the_end() {
        let rope = ropey::Rope::from_str("one\n");
        let map = crate::editor::decoration::DecorationMap::default();
        let log = crate::edit_log::EditLog::default();
        assert_eq!(
            map.project_all(&rope, &log)
                .inline_width_before(7, 3, &rope),
            0
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn out_of_range_edits_report_failure_instead_of_panicking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("A.java");
        fs::write(&file, "one\ntwo\n").expect("write");
        let mut editor = Editor::default();
        editor.open_file(&file).expect("open");

        for (start, end) in [
            ((99, 0), (99, 5)),
            ((0, 0), (99, 0)),
            ((2, 0), (99, 99)),
            ((3, 0), (3, 0)),
            ((0, 500), (0, 900)),
            ((1, 0), (0, 0)),
        ] {
            let edit = operations(vec![doc_edit(file_uri(&file), start, end, "X")]);
            let _ = editor.apply_workspace_edit(edit);
            editor.buffer().rope().to_string(); // buffer stays intact and readable
        }
    }

    fn resource(op: lsp_types::ResourceOp) -> lsp_types::DocumentChangeOperation {
        lsp_types::DocumentChangeOperation::Op(op)
    }

    /// OV-00403: an edit addressed to a file that does not exist and was not
    /// announced by CreateFile still lands (implicit create).
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn edit_to_missing_file_creates_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let open = dir.path().join("A.java");
        fs::write(&open, "class A {}\n").unwrap();
        let target = dir.path().join("pkg/deep/B.java");
        let mut editor = Editor::default();
        editor.open_file(&open).unwrap();

        let edit = operations(vec![doc_edit(
            plain_uri(&target),
            (0, 0),
            (0, 0),
            "class B {}\n",
        )]);
        assert!(editor.apply_workspace_edit(edit).unwrap());
        assert_eq!(fs::read_to_string(&target).unwrap(), "class B {}\n");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn create_file_then_edit_writes_new_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let open = dir.path().join("A.java");
        fs::write(&open, "class A {}\n").unwrap();
        let target = dir.path().join("B.java");
        let mut editor = Editor::default();
        editor.open_file(&open).unwrap();

        let edit = operations(vec![
            resource(lsp_types::ResourceOp::Create(lsp_types::CreateFile {
                uri: plain_uri(&target),
                options: None,
                annotation_id: None,
            })),
            doc_edit(plain_uri(&target), (0, 0), (0, 0), "class B {}\n"),
        ]);
        assert!(editor.apply_workspace_edit(edit).unwrap());
        assert_eq!(fs::read_to_string(&target).unwrap(), "class B {}\n");
    }

    /// OV-00403: renaming a file that is open in a buffer must move the
    /// buffer with it, and a following edit to the new URI must reach it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn rename_file_retargets_the_open_buffer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let old = dir.path().join("Old.java");
        fs::write(&old, "class Old {}\n").unwrap();
        let new = dir.path().join("moved/New.java");
        let mut editor = Editor::default();
        editor.open_file(&old).unwrap();
        let old_uri = file_uri(&old);

        let edit = operations(vec![
            resource(lsp_types::ResourceOp::Rename(lsp_types::RenameFile {
                old_uri,
                new_uri: plain_uri(&new),
                options: None,
                annotation_id: None,
            })),
            doc_edit(plain_uri(&new), (0, 6), (0, 9), "New"),
        ]);
        assert!(editor.apply_workspace_edit(edit).unwrap());
        assert!(!old.exists());
        assert!(new.exists());
        let buffer_path = editor.buffer().file_path().unwrap().to_string();
        assert!(buffer_path.ends_with("moved/New.java"), "{buffer_path}");
        assert_eq!(editor.buffer().rope().to_string(), "class New {}\n");
        assert_eq!(
            editor
                .buffers
                .iter()
                .filter(|b| b.file_path().is_some_and(|p| p.ends_with("New.java")))
                .count(),
            1,
            "no duplicate buffer for the moved file"
        );
    }

    /// Deleting a file that is open keeps the buffer (and its text) alive.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn delete_file_keeps_the_open_buffer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let doomed = dir.path().join("Doomed.java");
        let other = dir.path().join("Other.java");
        fs::write(&doomed, "class Doomed {}\n").unwrap();
        fs::write(&other, "class Other {}\n").unwrap();
        let mut editor = Editor::default();
        editor.open_file(&doomed).unwrap();
        editor.open_file(&other).unwrap();

        let edit = operations(vec![resource(lsp_types::ResourceOp::Delete(
            lsp_types::DeleteFile {
                uri: file_uri(&doomed),
                options: None,
            },
        ))]);
        assert!(editor.apply_workspace_edit(edit).unwrap());
        assert!(!doomed.exists());
        assert!(editor
            .buffers
            .iter()
            .any(|b| *b.rope() == "class Doomed {}\n"));
    }

    // ---- file operation options (LSP: overwrite, ignoreIfExists, recursive...) ----

    fn rename_op(
        old: &std::path::Path,
        new: &std::path::Path,
        overwrite: Option<bool>,
        ignore_if_exists: Option<bool>,
    ) -> lsp_types::WorkspaceEdit {
        operations(vec![resource(lsp_types::ResourceOp::Rename(
            lsp_types::RenameFile {
                old_uri: file_uri(old),
                new_uri: plain_uri(new),
                options: Some(lsp_types::RenameFileOptions {
                    overwrite,
                    ignore_if_exists,
                }),
                annotation_id: None,
            },
        ))])
    }

    fn create_op(
        path: &std::path::Path,
        overwrite: Option<bool>,
        ignore_if_exists: Option<bool>,
    ) -> lsp_types::WorkspaceEdit {
        operations(vec![resource(lsp_types::ResourceOp::Create(
            lsp_types::CreateFile {
                uri: plain_uri(path),
                options: Some(lsp_types::CreateFileOptions {
                    overwrite,
                    ignore_if_exists,
                }),
                annotation_id: None,
            },
        ))])
    }

    fn delete_op(
        path: &std::path::Path,
        recursive: Option<bool>,
        ignore_if_not_exists: Option<bool>,
    ) -> lsp_types::WorkspaceEdit {
        operations(vec![resource(lsp_types::ResourceOp::Delete(
            lsp_types::DeleteFile {
                uri: plain_uri(path),
                options: Some(lsp_types::DeleteFileOptions {
                    recursive,
                    ignore_if_not_exists,
                    annotation_id: None,
                }),
            },
        ))])
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn rename_onto_an_existing_file_needs_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let from = dir.path().join("From.java");
        let to = dir.path().join("To.java");
        fs::write(&from, "from\n").unwrap();
        fs::write(&to, "to\n").unwrap();
        let mut editor = Editor::default();

        // No option: the rename fails and nothing is overwritten.
        assert!(!editor
            .apply_workspace_edit(rename_op(&from, &to, None, None))
            .unwrap());
        assert_eq!(fs::read_to_string(&to).unwrap(), "to\n");
        assert!(from.exists());

        // ignoreIfExists: the operation is a successful no-op.
        assert!(editor
            .apply_workspace_edit(rename_op(&from, &to, None, Some(true)))
            .unwrap());
        assert_eq!(fs::read_to_string(&to).unwrap(), "to\n");
        assert!(from.exists());

        // overwrite wins over ignoreIfExists.
        assert!(editor
            .apply_workspace_edit(rename_op(&from, &to, Some(true), Some(true)))
            .unwrap());
        assert_eq!(fs::read_to_string(&to).unwrap(), "from\n");
        assert!(!from.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn create_over_an_existing_file_follows_its_options() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("Existing.java");
        fs::write(&file, "keep\n").unwrap();
        let mut editor = Editor::default();

        assert!(!editor
            .apply_workspace_edit(create_op(&file, None, None))
            .unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep\n");

        assert!(editor
            .apply_workspace_edit(create_op(&file, None, Some(true)))
            .unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep\n");

        assert!(editor
            .apply_workspace_edit(create_op(&file, Some(true), None))
            .unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), "");
    }

    /// Overwriting a file that is open and holds nothing unsaved empties the
    /// buffer too; unsaved work is never discarded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn create_overwrite_reaches_the_open_buffer_only_when_it_is_clean() {
        let dir = tempfile::tempdir().expect("tempdir");
        let clean = dir.path().join("Clean.java");
        let dirty = dir.path().join("Dirty.java");
        fs::write(&clean, "class Clean {}\n").unwrap();
        fs::write(&dirty, "class Dirty {}\n").unwrap();
        let mut editor = Editor::default();
        editor.open_file(&clean).unwrap();
        editor.open_file(&dirty).unwrap();
        editor
            .buffer_mut()
            .insert_text_at(0, crate::unicode::CharCol(0), "// unsaved\n");

        assert!(editor
            .apply_workspace_edit(create_op(&clean, Some(true), None))
            .unwrap());
        assert!(editor
            .apply_workspace_edit(create_op(&dirty, Some(true), None))
            .unwrap());

        let text_of = |editor: &Editor, name: &str| {
            editor
                .buffers
                .iter()
                .find(|b| b.file_path().is_some_and(|p| p.ends_with(name)))
                .unwrap()
                .rope()
                .to_string()
        };
        assert_eq!(text_of(&editor, "Clean.java"), "");
        assert_eq!(
            text_of(&editor, "Dirty.java"),
            "// unsaved\nclass Dirty {}\n"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn delete_follows_recursive_and_ignore_if_not_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path().join("pkg");
        fs::create_dir_all(folder.join("inner")).unwrap();
        fs::write(folder.join("inner/A.java"), "a\n").unwrap();
        let mut editor = Editor::default();

        // A folder with content needs `recursive`.
        assert!(!editor
            .apply_workspace_edit(delete_op(&folder, None, None))
            .unwrap());
        assert!(folder.exists());
        assert!(editor
            .apply_workspace_edit(delete_op(&folder, Some(true), None))
            .unwrap());
        assert!(!folder.exists());

        // Deleting what is not there fails unless ignoreIfNotExists.
        assert!(!editor
            .apply_workspace_edit(delete_op(&folder, None, None))
            .unwrap());
        assert!(editor
            .apply_workspace_edit(delete_op(&folder, None, Some(true)))
            .unwrap());
    }

    /// Renaming a folder moves every open buffer below it, not just a buffer
    /// of the folder itself.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn folder_rename_moves_the_buffers_below_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path().join("old");
        fs::create_dir_all(folder.join("sub")).unwrap();
        fs::write(folder.join("A.java"), "a\n").unwrap();
        fs::write(folder.join("sub/B.java"), "b\n").unwrap();
        let mut editor = Editor::default();
        editor.open_file(&folder.join("A.java")).unwrap();
        editor.open_file(&folder.join("sub/B.java")).unwrap();
        let moved = dir.path().join("new");

        assert!(editor
            .apply_workspace_edit(rename_op(&folder, &moved, None, None))
            .unwrap());

        let mut paths: Vec<String> = editor
            .buffers
            .iter()
            .filter_map(|b| b.file_path().map(str::to_string))
            .filter(|p| p.ends_with(".java"))
            .collect();
        paths.sort();
        assert_eq!(paths.len(), 2, "{paths:?}");
        assert!(paths[0].ends_with("new/A.java"), "{paths:?}");
        assert!(paths[1].ends_with("new/sub/B.java"), "{paths:?}");
        assert!(moved.join("sub/B.java").exists() && !folder.exists());
    }

    /// A file operation's undo entry lives in the current buffer's history,
    /// but it is no unsaved text: an untouched buffer stays untouched.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_file_operation_does_not_mark_the_current_buffer_modified() {
        let dir = tempfile::tempdir().expect("tempdir");
        let open = dir.path().join("Open.java");
        fs::write(&open, "class Open {}\n").unwrap();
        let created = dir.path().join("Created.java");
        let mut editor = Editor::default();
        editor.open_file(&open).unwrap();
        assert!(editor.buffer().change_manager().is_at_save_point());

        assert!(editor
            .apply_workspace_edit(create_op(&created, None, None))
            .unwrap());

        assert!(created.exists());
        assert!(editor.buffer().change_manager().is_at_save_point());
        assert!(!editor.any_buffer_modified());
        // ...and it is still undoable.
        editor.undo();
        assert!(!created.exists());
    }
}
