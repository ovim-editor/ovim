//! Workspace file operations (`workspace/willRenameFiles`,
//! `workspace/didRenameFiles`).
//!
//! A server that lists `workspace.fileOperations.{will,did}Rename` filters in
//! its capabilities wants to know when the user renames or moves a file in the
//! client (for example to rewrite imports/package declarations). `willRename`
//! is a request answered with a `WorkspaceEdit` the client applies BEFORE
//! renaming; `didRename` is a notification sent AFTER.

use super::*;
use lsp_types::{
    FileOperationFilter, FileOperationPatternKind, FileRename, RenameFilesParams, WorkspaceEdit,
};

/// How long the editor waits for a `willRenameFiles` answer before renaming anyway.
const WILL_RENAME_TIMEOUT: Duration = Duration::from_secs(5);

fn filter_matches(filter: &FileOperationFilter, path: &Path, is_dir: bool) -> bool {
    if filter.scheme.as_deref().is_some_and(|s| s != "file") {
        return false;
    }
    match filter.pattern.matches {
        Some(FileOperationPatternKind::File) if is_dir => return false,
        Some(FileOperationPatternKind::Folder) if !is_dir => return false,
        _ => {}
    }
    let ignore_case = filter
        .pattern
        .options
        .as_ref()
        .and_then(|options| options.ignore_case)
        .unwrap_or(false);
    globset::GlobBuilder::new(&filter.pattern.glob)
        .literal_separator(true)
        .case_insensitive(ignore_case)
        .build()
        .map(|glob| glob.compile_matcher().is_match(path))
        .unwrap_or(false)
}

fn options_match(
    options: &Option<lsp_types::FileOperationRegistrationOptions>,
    path: &Path,
    is_dir: bool,
) -> bool {
    options.as_ref().is_some_and(|options| {
        options
            .filters
            .iter()
            .any(|f| filter_matches(f, path, is_dir))
    })
}

fn file_renames(renames: &[(PathBuf, PathBuf)]) -> Vec<FileRename> {
    renames
        .iter()
        .filter_map(|(old, new)| {
            Some(FileRename {
                old_uri: uri_from_file_path(old)?.as_str().to_string(),
                new_uri: uri_from_file_path(new)?.as_str().to_string(),
            })
        })
        .collect()
}

impl LspManager {
    /// Servers (and which of will/did) interested in renaming `old`.
    async fn rename_interested_servers(
        &self,
        old: &Path,
        is_dir: bool,
    ) -> Vec<(server::LanguageServer, bool, bool)> {
        let servers: Vec<server::LanguageServer> = self
            .servers
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        let mut interested = Vec::new();
        for server in servers {
            let Some(operations) = server
                .capabilities()
                .await
                .and_then(|caps| caps.workspace)
                .and_then(|workspace| workspace.file_operations)
            else {
                continue;
            };
            let will = options_match(&operations.will_rename, old, is_dir);
            let did = options_match(&operations.did_rename, old, is_dir);
            if will || did {
                interested.push((server, will, did));
            }
        }
        interested
    }

    /// Asks interested servers what must change when `renames` happen and
    /// returns their edits (to be applied before the rename). Servers that
    /// fail or time out are skipped: the rename must never be blocked by them.
    pub async fn will_rename_files(&self, renames: &[(PathBuf, PathBuf)]) -> Vec<WorkspaceEdit> {
        let mut edits = Vec::new();
        let mut asked: Vec<String> = Vec::new();
        for (old, _) in renames {
            let is_dir = old.is_dir();
            for (server, will, _) in self.rename_interested_servers(old, is_dir).await {
                let key = server.command().to_string() + server.language();
                if !will || asked.contains(&key) {
                    continue;
                }
                asked.push(key);
                let Ok(params) = serde_json::to_value(RenameFilesParams {
                    files: file_renames(renames),
                }) else {
                    continue;
                };
                match tokio::time::timeout(
                    WILL_RENAME_TIMEOUT,
                    server.request("workspace/willRenameFiles", params),
                )
                .await
                {
                    Ok(Ok(value)) if !value.is_null() => {
                        match serde_json::from_value::<WorkspaceEdit>(value) {
                            Ok(edit) => edits.push(edit),
                            Err(error) => lsp_warn!(
                                "LspManager",
                                "Unparsable willRenameFiles edit from {}: {}",
                                server.language(),
                                error
                            ),
                        }
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => lsp_warn!(
                        "LspManager",
                        "willRenameFiles failed for {}: {}",
                        server.language(),
                        error
                    ),
                    Err(_) => lsp_warn!(
                        "LspManager",
                        "willRenameFiles timed out for {}",
                        server.language()
                    ),
                }
            }
        }
        edits
    }

    /// Tells interested servers the rename happened.
    pub async fn did_rename_files(&self, renames: &[(PathBuf, PathBuf)]) {
        let mut notified: Vec<String> = Vec::new();
        for (_, new) in renames {
            // The old path no longer exists; the new one tells us file vs folder.
            let is_dir = new.is_dir();
            for (server, _, did) in self.rename_interested_servers(new, is_dir).await {
                let key = server.command().to_string() + server.language();
                if !did || notified.contains(&key) {
                    continue;
                }
                notified.push(key);
                let Ok(params) = serde_json::to_value(RenameFilesParams {
                    files: file_renames(renames),
                }) else {
                    continue;
                };
                if let Err(error) = server.notify("workspace/didRenameFiles", params).await {
                    lsp_warn!(
                        "LspManager",
                        "didRenameFiles failed for {}: {}",
                        server.language(),
                        error
                    );
                }
            }
        }
    }
}
