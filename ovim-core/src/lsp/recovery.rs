//! Crash recovery for language servers.
//!
//! The manager remembers how every server was launched (`ServerSpec`). Each
//! editor tick calls [`LspManager::supervise_servers`], which notices servers
//! whose process died (or whose I/O failed), announces it, and respawns them
//! with exponential backoff and a bounded number of consecutive attempts.
//! `:LspRestart` forces an immediate restart and resets the budget. Documents
//! are re-opened lazily by the editor: forgetting a server's documents zeroes
//! their manager-side version, which is the editor's signal to send `didOpen`
//! again.

use super::server::ServerState;
use super::*;

/// Consecutive automatic restarts before giving up until `:LspRestart`.
pub const MAX_AUTO_RESTARTS: u32 = 5;

/// A restarted server that stays `Ready` this long earns its budget back.
const HEALTHY_RESET_AFTER: Duration = Duration::from_secs(60);

/// How long a graceful shutdown of the old instance may take during restart.
const OLD_SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

fn backoff(base: Duration, attempts: u32) -> Duration {
    base.saturating_mul(1u32 << attempts.min(6))
        .min(Duration::from_secs(30))
}

/// How a server was launched; everything needed to launch it again.
#[derive(Clone, Debug)]
pub(super) struct ServerSpec {
    pub language: String,
    pub command: String,
    pub args: Vec<String>,
    pub root: PathBuf,
    /// Companion servers serialize startup on their own id, primaries on
    /// their language (matching `start_server` / `start_companion_server`).
    pub companion: bool,
}

#[derive(Debug, Default)]
pub(super) struct RestartState {
    /// Automatic restarts spent since the server was last healthy.
    attempts: u32,
    /// When the next (already announced) restart may begin.
    next_attempt: Option<Instant>,
    in_progress: bool,
    given_up: bool,
    /// `:LspRestart` was requested.
    forced: bool,
    /// Lifetime restart count, for `:LspInfo`.
    total_restarts: u32,
    last_failure: Option<String>,
}

/// Snapshot of one server for `:LspInfo` / `ovim lsp status`.
#[derive(Clone, Debug)]
pub struct ServerStatusReport {
    pub server_id: String,
    pub command: String,
    pub root: Option<PathBuf>,
    pub state: String,
    pub process_alive: bool,
    pub total_restarts: u32,
    pub restarting: bool,
    pub gave_up: bool,
}

impl LspManager {
    pub(super) fn record_server_spec(&self, server_id: &str, spec: ServerSpec) {
        self.server_specs.insert(server_id.to_string(), spec);
    }

    fn push_lifecycle_event(&self, event: String) {
        lsp_info!("LspManager", "{}", event);
        if let Ok(mut events) = self.lifecycle_events.lock() {
            events.push(event);
        }
    }

    fn restart_base_backoff(&self) -> Duration {
        Duration::from_millis(self.restart_base_backoff_ms.load(Ordering::Relaxed))
    }

    /// Overrides the first restart delay (500 ms by default); later attempts
    /// double it up to 30 s. Intended for tests.
    pub fn set_restart_base_backoff(&self, base: Duration) {
        self.restart_base_backoff_ms
            .store(base.as_millis() as u64, Ordering::Relaxed);
    }

    /// Drains human-readable crash/restart announcements for the status line.
    pub fn take_lifecycle_events(&self) -> Vec<String> {
        self.lifecycle_events
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default()
    }

    /// Ask for an immediate restart of `target` (a server id or language id;
    /// every server when `None`). The restart itself happens on the next
    /// `supervise_servers` tick. Returns the server ids that will restart.
    pub fn request_restart(
        &self,
        target: Option<&str>,
    ) -> std::result::Result<Vec<String>, String> {
        let ids: Vec<String> = self
            .server_specs
            .iter()
            .filter(|entry| match target {
                None => true,
                Some(t) => entry.key() == t || entry.value().language == t,
            })
            .map(|entry| entry.key().clone())
            .collect();
        if ids.is_empty() {
            return Err(match target {
                Some(t) => format!("No LSP server matching '{t}'"),
                None => "No LSP servers are running".to_string(),
            });
        }
        for id in &ids {
            let mut state = self.restart_states.entry(id.clone()).or_default();
            state.attempts = 0;
            state.given_up = false;
            state.forced = true;
            state.next_attempt = Some(Instant::now());
        }
        Ok(ids)
    }

    /// Detects dead servers and drives their bounded, backed-off restart.
    /// Cheap when everything is healthy; never blocks on a restart (the
    /// respawn runs in its own task).
    pub async fn supervise_servers(self: &Arc<Self>) {
        let ids: Vec<String> = self
            .server_specs
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        for id in ids {
            let server = self.servers.get(&id).map(|entry| entry.value().clone());
            let failure: Option<String> = match &server {
                Some(server) => match server.state().await {
                    ServerState::Failed { error, .. } => Some(error),
                    ServerState::Terminated => Some("LSP server terminated".to_string()),
                    ServerState::Ready { initialized_at, .. }
                        if initialized_at.elapsed() >= HEALTHY_RESET_AFTER =>
                    {
                        self.restart_states
                            .remove_if(&id, |_, state| !state.in_progress && !state.forced);
                        None
                    }
                    _ => None,
                },
                None => Some("LSP server is missing".to_string()),
            };

            let start = {
                let mut state = self.restart_states.entry(id.clone()).or_default();
                if state.in_progress || (failure.is_none() && !state.forced) {
                    continue;
                }
                if state.given_up && !state.forced {
                    continue;
                }
                if let Some(reason) = &failure {
                    state.last_failure = Some(reason.clone());
                }
                let now = Instant::now();
                match state.next_attempt {
                    None => {
                        if state.attempts >= MAX_AUTO_RESTARTS {
                            state.given_up = true;
                            self.push_lifecycle_event(format!(
                                "LSP: {id} failed repeatedly ({}); giving up after {MAX_AUTO_RESTARTS} restarts. Use :LspRestart to try again",
                                failure.as_deref().unwrap_or("unknown error")
                            ));
                        } else {
                            let delay = backoff(self.restart_base_backoff(), state.attempts);
                            state.next_attempt = Some(now + delay);
                            self.push_lifecycle_event(format!(
                                "LSP: {id} crashed ({}); restarting in {:.1}s (attempt {}/{MAX_AUTO_RESTARTS})",
                                failure.as_deref().unwrap_or("unknown error"),
                                delay.as_secs_f32(),
                                state.attempts + 1,
                            ));
                        }
                        false
                    }
                    Some(at) if now >= at => {
                        if !state.forced {
                            state.attempts += 1;
                        }
                        state.forced = false;
                        state.next_attempt = None;
                        state.in_progress = true;
                        true
                    }
                    Some(_) => false,
                }
            };

            if start {
                self.push_lifecycle_event(format!("LSP: restarting {id}..."));
                let manager = Arc::clone(self);
                tokio::spawn(async move {
                    let outcome = manager.restart_server(&id).await;
                    let mut state = manager.restart_states.entry(id.clone()).or_default();
                    state.in_progress = false;
                    match outcome {
                        Ok(()) => {
                            state.total_restarts += 1;
                            manager.push_lifecycle_event(format!("LSP: {id} restarted"));
                        }
                        Err(error) => {
                            state.last_failure = Some(format!("{error:#}"));
                            manager.push_lifecycle_event(format!(
                                "LSP: restart of {id} failed: {error:#}"
                            ));
                        }
                    }
                });
            }
        }
    }

    /// Replaces the server `server_id` with a freshly spawned one launched
    /// from its recorded spec. The old instance is shut down (and reaped)
    /// first so two heavyweight servers never overlap.
    async fn restart_server(self: &Arc<Self>, server_id: &str) -> Result<()> {
        let spec = self
            .server_specs
            .get(server_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow::anyhow!("no launch record for {server_id}"))?;
        let gate_key = if spec.companion {
            server_id.to_string()
        } else {
            spec.language.clone()
        };
        let gate = self
            .startup_gates
            .entry(gate_key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _startup_permit = gate.lock().await;

        if let Some((_, handle)) = self.listener_handles.remove(server_id) {
            handle.abort();
        }
        let old = self
            .servers
            .get(server_id)
            .map(|entry| entry.value().clone());
        if let Some(mut old) = old {
            if tokio::time::timeout(OLD_SERVER_SHUTDOWN_TIMEOUT, old.shutdown())
                .await
                .is_err()
            {
                lsp_warn!("LspManager", "Timed out shutting down old {}", server_id);
            }
        }

        let root_uri = uri_from_file_path(&spec.root)
            .ok_or_else(|| anyhow::anyhow!("Invalid root path {}", spec.root.display()))?;
        let server = server::LanguageServer::spawn_initialized(
            &spec.language,
            &spec.command,
            spec.args.clone(),
            root_uri,
        )
        .await?;

        self.servers.insert(server_id.to_string(), server);
        // The fresh process has no documents open: forgetting the old
        // instance's documents makes the editor send didOpen for each again.
        self.forget_server_documents(server_id).await;
        self.start_notification_listener(server_id.to_string())
            .await;
        self.diagnostics_changed.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Drops everything the manager remembers about documents opened on
    /// `server_id` (and its diagnostics). A document that no other server
    /// still holds also loses its version claim, so the next `didOpen` for
    /// it is not treated as a duplicate.
    pub(super) async fn forget_server_documents(&self, server_id: &str) {
        let uris: Vec<Uri> = self
            .server_documents
            .iter()
            .filter(|entry| entry.key().0 == server_id)
            .map(|entry| entry.key().1.clone())
            .collect();
        self.server_documents.retain(|(sid, _), _| sid != server_id);
        for uri in uris {
            if self
                .server_documents
                .iter()
                .any(|entry| entry.key().1 == uri)
            {
                continue;
            }
            self.document_versions.lock().await.remove(&uri);
            self.last_sent_versions.lock().await.remove(&uri);
            self.last_local_edit.lock().await.remove(&uri);
            self.change_debouncers.remove(&uri);
            self.flush_gates.remove(&uri);
        }
        {
            let mut diags = self.diagnostics.lock().await;
            for server_map in diags.values_mut() {
                server_map.remove(server_id);
            }
        }
        self.merged_diagnostics_cache.lock().await.clear();
        self.deferred_diagnostics
            .lock()
            .await
            .retain(|(_, sid), _| sid != server_id);
        self.file_watch_registrations.remove(server_id);
        self.diagnostics_changed.store(true, Ordering::SeqCst);
    }

    /// Per-server status for `:LspInfo`.
    pub fn server_status_reports(&self) -> Vec<ServerStatusReport> {
        let servers: Vec<(String, server::LanguageServer)> = self
            .servers
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        let mut reports = Vec::new();
        for (server_id, server) in servers {
            let (state, process_alive) = server.status_snapshot();
            let restart = self.restart_states.get(&server_id);
            reports.push(ServerStatusReport {
                command: server.command().to_string(),
                root: self.server_root(&server_id),
                state,
                process_alive,
                total_restarts: restart.as_ref().map_or(0, |s| s.total_restarts),
                restarting: restart
                    .as_ref()
                    .is_some_and(|s| s.in_progress || s.next_attempt.is_some()),
                gave_up: restart.as_ref().is_some_and(|s| s.given_up),
                server_id,
            });
        }
        reports.sort_by(|a, b| a.server_id.cmp(&b.server_id));
        reports
    }
}
