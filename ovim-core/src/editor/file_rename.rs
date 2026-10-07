//! File-explorer renames coordinated with language servers and open buffers.
//!
//! Order matters (LSP 3.16 file operations): `workspace/willRenameFiles` is
//! asked BEFORE the rename so the server can compute import/package edits
//! against the old paths; the edit is applied, the file is renamed, open
//! buffers follow their files, and `workspace/didRenameFiles` goes out AFTER.

use super::Editor;
use std::path::{Path, PathBuf};

impl Editor {
    /// Queues an explorer rename; the async part runs on the next LSP tick.
    pub fn request_explorer_rename(&mut self, original: PathBuf, input: String) {
        self.lsp.state.pending_file_rename = Some((original, input));
        self.set_status_message("Renaming...");
    }

    /// Open buffers whose file is `path` or lives below it, with the part of
    /// their path relative to `path` (empty for the file itself). Must run
    /// before the rename: it canonicalizes paths that stop existing after it.
    fn buffers_at_or_below(&self, path: &Path) -> Vec<(usize, PathBuf)> {
        let Ok(path) = path.canonicalize() else {
            return Vec::new();
        };
        self.buffers
            .iter()
            .enumerate()
            .filter_map(|(index, buffer)| {
                let buffer_path = Path::new(buffer.file_path()?).canonicalize().ok()?;
                let relative = buffer_path.strip_prefix(&path).ok()?;
                Some((index, relative.to_path_buf()))
            })
            .collect()
    }

    /// Drives an explorer rename. The servers' `willRenameFiles` answers can
    /// take seconds, so they are collected in a task and the rename carries on
    /// in a later tick once they are in: the editor loop never waits for them.
    pub async fn process_pending_file_rename(&mut self) {
        if let Some(in_flight) = self.lsp.state.file_rename_in_flight.take() {
            if !in_flight.edits.is_finished() {
                self.lsp.state.file_rename_in_flight = Some(in_flight);
                return;
            }
            let edits = in_flight.edits.await.unwrap_or_default();
            self.finish_file_rename(in_flight.original, in_flight.input, in_flight.old, edits)
                .await;
            return;
        }

        let Some((original, input)) = self.lsp.state.pending_file_rename.take() else {
            return;
        };
        let (old, new) = match self.file_tree().plan_rename(&original, &input) {
            Ok(Some(paths)) => paths,
            Ok(None) => return,
            Err(error) => {
                self.set_status_message(format!("Rename failed: {error}"));
                return;
            }
        };

        // 1. Let servers rewrite references before the file moves.
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            self.finish_file_rename(original, input, old, Vec::new())
                .await;
            return;
        };
        let renames = [(old.clone(), new)];
        let edits = tokio::spawn(async move { lsp.will_rename_files(&renames).await });
        self.lsp.state.file_rename_in_flight = Some(RenameInFlight {
            original,
            input,
            old,
            edits,
        });
    }

    /// Applies the servers' pre-rename edits, renames on disk, moves the open
    /// buffers along and tells the servers it happened.
    async fn finish_file_rename(
        &mut self,
        original: PathBuf,
        input: String,
        old: PathBuf,
        will_rename_edits: Vec<lsp_types::WorkspaceEdit>,
    ) {
        let mut edit_problems = Vec::new();
        for edit in will_rename_edits {
            match self.apply_workspace_edit(edit) {
                Ok(true) => {}
                Ok(false) => {
                    edit_problems.push("some reference updates were not applied".to_string())
                }
                Err(error) => edit_problems.push(error.to_string()),
            }
        }

        // 2. Rename on disk (also refreshes the explorer).
        let affected = self.buffers_at_or_below(&old);
        let new_path = match self.file_tree_mut().rename_entry(&original, &input) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                self.set_status_message(format!("Rename failed: {error}"));
                return;
            }
        };

        // 3. Open buffers follow their files.
        for (index, relative) in affected {
            let target = if relative.as_os_str().is_empty() {
                new_path.clone()
            } else {
                new_path.join(relative)
            };
            self.retarget_buffer_path(index, target);
        }

        // 4. Tell servers it happened.
        if let Some(lsp) = self.lsp.state.lsp_manager.clone() {
            let renames = [(old.clone(), new_path.clone())];
            tokio::spawn(async move { lsp.did_rename_files(&renames).await });
        }

        let name = |p: &Path| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        };
        let mut status = format!("Renamed: {} -> {}", name(&old), name(&new_path));
        if !edit_problems.is_empty() {
            status.push_str(&format!(" ({})", edit_problems.join("; ")));
        }
        self.set_status_message(status);
        self.mark_dirty();
    }
}

/// An explorer rename waiting for the servers' `willRenameFiles` answers.
pub(crate) struct RenameInFlight {
    original: PathBuf,
    input: String,
    old: PathBuf,
    edits: tokio::task::JoinHandle<Vec<lsp_types::WorkspaceEdit>>,
}
