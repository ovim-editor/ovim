use super::{initialize_configured_lsp, install_approved, normalize_path};
use crate::editor::{AutoInstallMode, Editor, PendingLspInstall};
use crate::language_catalog::LanguageDefinition;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

enum Update {
    Status(String),
    Prompt(PendingLspInstall),
    Ready {
        language_id: String,
        server_id: String,
        command: String,
        primary: bool,
    },
}

/// Startup owns only configuration and a manager handle. Buffer state is read
/// on the frontend when readiness arrives, so edits and file switches during
/// initialization cannot pre-warm a server with the wrong document.
pub(super) struct InitRequest {
    pub file_path: String,
    pub abs_path: PathBuf,
    pub language: Arc<LanguageDefinition>,
    pub manager: Arc<crate::lsp::LspManager>,
    pub install_mode: AutoInstallMode,
    updates: mpsc::Sender<(String, Update)>,
}

impl InitRequest {
    pub async fn status(&self, message: String) {
        let _ = self
            .updates
            .send((self.file_path.clone(), Update::Status(message)))
            .await;
    }

    pub async fn prompt(&self, prompt: PendingLspInstall) {
        let _ = self
            .updates
            .send((self.file_path.clone(), Update::Prompt(prompt)))
            .await;
    }

    pub async fn ready(
        &self,
        language_id: &str,
        server_id: String,
        command: String,
        primary: bool,
    ) {
        let _ = self
            .updates
            .send((
                self.file_path.clone(),
                Update::Ready {
                    language_id: language_id.to_string(),
                    server_id,
                    command,
                    primary,
                },
            ))
            .await;
    }
}

/// Per-frontend startup tasks, deduplicated by file and aborted with their
/// frontend. No task holds an Editor borrow while installing or initializing.
pub struct LspStartup {
    jobs: HashMap<String, JoinHandle<()>>,
    tx: mpsc::Sender<(String, Update)>,
    rx: mpsc::Receiver<(String, Update)>,
}

impl Default for LspStartup {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(100);
        Self {
            jobs: HashMap::new(),
            tx,
            rx,
        }
    }
}

impl Drop for LspStartup {
    fn drop(&mut self) {
        for job in self.jobs.values() {
            job.abort();
        }
    }
}

impl LspStartup {
    pub fn start(&mut self, editor: &mut Editor, file_path: &str, approved: bool) {
        if self.jobs.contains_key(file_path) {
            return;
        }
        let abs_path = normalize_path(Path::new(file_path), editor);
        if abs_path.as_os_str().is_empty() {
            return;
        }
        let Some(manager) = editor.lsp_manager() else {
            return;
        };
        let Some(language) = editor.language_catalog().detect(&abs_path) else {
            return;
        };
        if language.lsp().is_none() {
            return;
        }
        editor.set_lsp_status(format!("LSP: Starting {}...", language.config.name));
        let request = InitRequest {
            file_path: file_path.to_string(),
            abs_path,
            language,
            manager,
            install_mode: editor.options.lsp_auto_install,
            updates: self.tx.clone(),
        };
        self.jobs.insert(
            file_path.to_string(),
            tokio::spawn(async move {
                if approved {
                    install_approved(&request).await;
                } else {
                    initialize_configured_lsp(&request).await;
                }
            }),
        );
    }

    pub async fn poll(&mut self, editor: &mut Editor) {
        let finished: Vec<_> = self
            .jobs
            .iter()
            .filter(|(_, task)| task.is_finished())
            .map(|(path, _)| path.clone())
            .collect();
        for path in finished {
            if let Some(task) = self.jobs.remove(&path) {
                if let Err(error) = task.await {
                    crate::lsp_warn!("LSP", "Startup task failed for {}: {}", path, error);
                    if editor.buffer().file_path() == Some(path.as_str()) {
                        editor.set_lsp_status(format!("LSP: Startup failed: {error}"));
                    }
                }
            }
        }
        while let Ok((file_path, update)) = self.rx.try_recv() {
            if let Update::Ready {
                language_id,
                command,
                primary: true,
                ..
            } = &update
            {
                editor.register_lsp_server(language_id.clone(), command.clone());
            }
            if editor.buffer().file_path() != Some(file_path.as_str()) {
                continue;
            }
            match update {
                Update::Status(status) => editor.set_lsp_status(status),
                Update::Prompt(prompt) => editor.set_pending_lsp_install(prompt),
                Update::Ready {
                    language_id,
                    server_id,
                    primary,
                    ..
                } => {
                    if primary {
                        editor.ensure_lsp_document_synced().await;
                    } else if let (Some(manager), Some(uri), Some(document_language)) = (
                        editor.lsp_manager(),
                        crate::lsp::uri_from_file_path(&file_path),
                        editor.language_id_for_path(&file_path),
                    ) {
                        // Open the document on the companion under the
                        // document's own language id (not its server id).
                        let version = manager.get_document_version(&uri).await.max(1);
                        let content = editor.buffer().rope().to_string();
                        let _ = manager
                            .did_open(uri, &server_id, &document_language, version, content)
                            .await;
                    }
                    editor.set_lsp_status(format!("LSP: {language_id} ready"));
                    editor.request_diagnostics_refresh();
                }
            }
            editor.mark_dirty();
        }
    }
}
