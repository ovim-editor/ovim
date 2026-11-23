//! Language-server side of launching: `hyperion.resolveLaunch` and
//! `hyperion.runConfigurations` (see the launch contract).
//!
//! Both run inside spawned tasks so a slow or still-indexing server can never
//! stall the editor tick. Servers are chosen for the *document* (its language
//! and workspace root), not hard-coded to Java.

use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::lsp::{uri_from_file_path, LspManager};

pub const RESOLVE_LAUNCH: &str = "hyperion.resolveLaunch";
pub const RUN_CONFIGURATIONS: &str = "hyperion.runConfigurations";

/// Outcome of asking the language server what to launch at a position.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolveOutcome {
    /// The server returned launch information (non-null JSON).
    Plan(Value),
    /// The server understood the request and found nothing runnable.
    Nothing,
    /// The server does not implement `hyperion.resolveLaunch`.
    Unsupported(String),
    /// No usable server: none running for the file, or still starting.
    NoServer(String),
    /// The request itself failed.
    Failed(String),
}

/// Result of the whole resolve step, including configurations to fall back
/// on when the cursor position yields nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolveResult {
    pub outcome: ResolveOutcome,
    /// `hyperion.runConfigurations` output; only fetched when `outcome` is not
    /// a plan.
    pub configurations: Vec<Value>,
}

/// Ready servers for the document that advertise `command`.
async fn capable_servers(
    manager: &LspManager,
    language_id: &str,
    file: &Path,
    command: &str,
) -> Result<Vec<String>, ResolveOutcome> {
    let ids = manager.servers_for_document(language_id, file);
    if ids.is_empty() {
        return Err(ResolveOutcome::NoServer(format!(
            "no language server is running for {language_id} files"
        )));
    }
    let mut ready = Vec::new();
    for id in &ids {
        if let Some(server) = manager.get_server(id).await {
            if server.is_ready().await {
                ready.push((id.clone(), server));
            }
        }
    }
    if ready.is_empty() {
        return Err(ResolveOutcome::NoServer(
            "the language server is still starting; try again in a moment".to_string(),
        ));
    }
    let mut capable = Vec::new();
    for (id, server) in ready {
        let listed = server
            .capabilities()
            .await
            .and_then(|caps| caps.execute_command_provider)
            .is_some_and(|opts| opts.commands.iter().any(|c| c == command));
        if listed {
            capable.push(id);
        }
    }
    if capable.is_empty() {
        return Err(ResolveOutcome::Unsupported(format!(
            "the {language_id} language server does not provide {command} (update Hyperion)"
        )));
    }
    Ok(capable)
}

async fn execute(
    manager: &LspManager,
    server_id: &str,
    command: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    manager
        .execute_command_on_server_id(command.to_string(), Some(args), server_id)
        .await
        .map(|v| v.unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

/// `hyperion.resolveLaunch` for `file` at an LSP (UTF-16) position.
pub async fn resolve_launch(
    manager: Arc<LspManager>,
    language_id: &str,
    file: &Path,
    line: u32,
    character: u32,
    target: &str,
) -> ResolveOutcome {
    let servers = match capable_servers(&manager, language_id, file, RESOLVE_LAUNCH).await {
        Ok(s) => s,
        Err(outcome) => return outcome,
    };
    let Some(uri) = uri_from_file_path(file) else {
        return ResolveOutcome::Failed(format!("cannot form a URI for {}", file.display()));
    };
    let args = json!({
        "uri": uri.as_str(),
        "position": {"line": line, "character": character},
        "target": target,
    });
    let mut last_error = String::new();
    for server_id in servers {
        match execute(&manager, &server_id, RESOLVE_LAUNCH, vec![args.clone()]).await {
            Ok(Value::Null) => return ResolveOutcome::Nothing,
            Ok(value) => return ResolveOutcome::Plan(value),
            Err(e) => last_error = e,
        }
    }
    ResolveOutcome::Failed(last_error)
}

/// Runs `command` on the first ready server for `file` that lists it in its
/// `executeCommandProvider` (`:LspExec`). `Err` carries a message fit for
/// the status line.
pub async fn execute_for_document(
    manager: Arc<LspManager>,
    language_id: &str,
    file: &Path,
    command: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    let servers = match capable_servers(&manager, language_id, file, command).await {
        Ok(s) => s,
        Err(ResolveOutcome::NoServer(reason) | ResolveOutcome::Unsupported(reason)) => {
            return Err(reason)
        }
        Err(other) => return Err(format!("{other:?}")),
    };
    let mut last = String::new();
    for server_id in servers {
        match execute(&manager, &server_id, command, args.clone()).await {
            Ok(value) => return Ok(value),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// `hyperion.runConfigurations` from the server owning `file`.
pub async fn run_configurations(
    manager: &LspManager,
    language_id: &str,
    file: &Path,
) -> Vec<Value> {
    let Ok(servers) = capable_servers(manager, language_id, file, RUN_CONFIGURATIONS).await else {
        return Vec::new();
    };
    for server_id in servers {
        if let Ok(Value::Array(items)) =
            execute(manager, &server_id, RUN_CONFIGURATIONS, Vec::new()).await
        {
            return items;
        }
    }
    Vec::new()
}

/// Resolve step used by the launch pipeline: try `resolveLaunch`; when that
/// does not produce a plan, also fetch configurations to fall back on.
pub async fn resolve_with_fallback(
    manager: Arc<LspManager>,
    language_id: String,
    file: std::path::PathBuf,
    line: u32,
    character: u32,
    target: String,
) -> ResolveResult {
    let outcome = resolve_launch(
        manager.clone(),
        &language_id,
        &file,
        line,
        character,
        &target,
    )
    .await;
    let configurations = if matches!(outcome, ResolveOutcome::Plan(_)) {
        Vec::new()
    } else {
        run_configurations(&manager, &language_id, &file).await
    };
    ResolveResult {
        outcome,
        configurations,
    }
}
