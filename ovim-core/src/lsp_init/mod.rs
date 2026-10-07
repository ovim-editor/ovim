pub mod auto_install;
mod background;
use background::InitRequest;
pub use background::{InstallApproval, LspStartup};

use crate::editor::Editor;
use crate::language_catalog::LanguageDefinition;
use crate::language_config::{
    find_lsp_command, AutoInstallConfig, AutoInstallPolicy, CompanionLspConfig, InstallMethod,
    LanguageRegistry,
};
use crate::lsp::companion_server_id;
use crate::project_root::find_project_root;
use auto_install::{attempt_auto_install, InstallResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Install and initialize configured servers without borrowing the frontend.
async fn initialize_configured_lsp(request: &InitRequest) {
    let abs_path = &request.abs_path;
    let language = &request.language;
    let lang_config = &language.config;
    let Some(lsp_config) = language.lsp() else {
        return;
    };

    // Try to find LSP server binary (primary command + fallbacks)
    let mut server_command = match find_lsp_command(lsp_config) {
        Some(cmd) => cmd,
        None => {
            // LSP server not found - try auto-install if configured
            if let Some(auto_install_config) = &lsp_config.auto_install {
                // Check user's global autoinstall preference
                if request.install_mode == crate::editor::AutoInstallMode::Off {
                    let hint = lsp_config
                        .install_hint
                        .as_deref()
                        .unwrap_or("LSP server not found in PATH");
                    request.status(format!("LSP: {}", hint)).await;
                    crate::lsp_info!(
                        "LSP",
                        "Skipping auto-install for {} because autoinstall=off",
                        lang_config.name
                    );
                    return;
                }

                if !auto_install_on_missing_enabled(auto_install_config) {
                    let hint = lsp_config
                        .install_hint
                        .as_deref()
                        .unwrap_or("LSP server not found in PATH");
                    request.status(format!("LSP: {}", hint)).await;
                    crate::lsp_info!(
                        "LSP",
                        "Skipping auto-install for {} because policy is manual_only",
                        lang_config.name
                    );
                    return;
                }

                if !is_auto_install_allowed_for_current_mode(auto_install_config) {
                    let hint = lsp_config
                        .install_hint
                        .as_deref()
                        .unwrap_or("LSP server not found in PATH");
                    request
                        .status(format!(
                            "LSP: {} (auto-install skipped in headless mode)",
                            hint
                        ))
                        .await;
                    crate::lsp_info!(
                        "LSP",
                        "Skipping auto-install for {} in headless mode (allow_headless=false)",
                        lang_config.name
                    );
                    return;
                }

                // If autoinstall=prompt, show consent dialog and return early.
                // The event loop will pick up the approved install and re-trigger.
                if request.install_mode == crate::editor::AutoInstallMode::Prompt {
                    let method_desc = describe_install_method(&auto_install_config.method);
                    request
                        .prompt(crate::editor::PendingLspInstall {
                            language_name: lang_config.name.clone(),
                            server_command: lsp_config.command.clone(),
                            method_description: method_desc,
                            file_path: request.file_path.clone(),
                            companion_id: None,
                        })
                        .await;
                    crate::lsp_info!(
                        "LSP",
                        "Prompting user for auto-install consent for {}",
                        lang_config.name
                    );
                    return;
                }

                crate::lsp_info!(
                    "LSP",
                    "{} language server not found. Attempting auto-install...",
                    lang_config.name
                );

                request
                    .status(format!("LSP: Installing {}...", lsp_config.command))
                    .await;

                // Attempt auto-install
                let install_result = attempt_auto_install(
                    &lang_config.name,
                    &lsp_config.command,
                    auto_install_config,
                )
                .await;

                match install_result {
                    InstallResult::Success(path) => {
                        request
                            .status(format!(
                                "LSP: {} installed successfully!",
                                lsp_config.command
                            ))
                            .await;
                        crate::lsp_info!(
                            "LSP",
                            "Auto-installed {} to {}",
                            lsp_config.command,
                            path.display()
                        );

                        // Resolve again so PATH/fallback logic can pick the preferred command.
                        find_lsp_command(lsp_config)
                            .unwrap_or_else(|| path.to_string_lossy().to_string())
                    }
                    InstallResult::Failed(error) => {
                        request
                            .status(format!("LSP: Auto-install failed: {}", error))
                            .await;
                        crate::lsp_warn!("LSP", "Auto-install failed: {}", error);
                        return;
                    }
                    InstallResult::PrerequisitesMissing(msg) => {
                        request.status(format!("LSP: {}", msg)).await;
                        crate::lsp_warn!("LSP", "Prerequisites missing: {}", msg);
                        return;
                    }
                }
            } else {
                // No auto-install configured - show manual install hint
                let hint = lsp_config
                    .install_hint
                    .as_deref()
                    .unwrap_or("LSP server not found in PATH");

                request.status(format!("LSP: {}", hint)).await;
                crate::lsp_warn!(
                    "LSP",
                    "Language server not found for {} (tried: {}, fallbacks: {:?})",
                    lang_config.name,
                    lsp_config.command,
                    lsp_config.fallback_commands
                );
                return;
            }
        }
    };

    // Find project root using configured markers
    let root_path = lsp_config.find_root(abs_path);

    // Determine language ID (for TypeScript vs JavaScript, use extension-based logic)
    let language_id = document_language_id(language, abs_path);

    crate::lsp_info!(
        "LSP",
        "Initializing {} LSP: command={}, root={}, language_id={}",
        lang_config.name,
        server_command,
        root_path.display(),
        language_id
    );

    // Start LSP server using the unified path
    {
        let lsp_manager = &request.manager;
        let mut attempted_known_failure_repair = false;

        loop {
            match lsp_manager
                .start_server(
                    &language_id,
                    &server_command,
                    lsp_config.args.clone(),
                    &root_path,
                )
                .await
            {
                Ok(server_id) => {
                    if lsp_config.root_is_fallback(abs_path) {
                        lsp_manager.mark_fallback_root(&server_id);
                    }
                    lsp_manager
                        .start_notification_listener(server_id.clone())
                        .await;
                    request
                        .ready(&language_id, server_id, server_command.clone(), true)
                        .await;
                    initialize_companions(request, &language_id, abs_path).await;
                    break;
                }
                Err(e) => {
                    // {:#} prints the whole context chain; the root cause
                    // (e.g. a server's initialize error) is otherwise hidden
                    // behind the outermost "Failed to send initialize request".
                    let error = format!("{:#}", e);
                    let repair = if attempted_known_failure_repair {
                        KnownFailureRepair::No
                    } else {
                        known_failure_repair(
                            &lang_config.id,
                            &lsp_config.command,
                            lsp_config.auto_install.as_ref(),
                            &error,
                            request.install_mode,
                        )
                    };
                    if repair == KnownFailureRepair::Ask {
                        // The user decides whether a reinstall may touch
                        // their machine; approval re-runs startup.
                        let auto_install_config = lsp_config
                            .auto_install
                            .as_ref()
                            .expect("repair precondition checked");
                        request
                            .prompt(crate::editor::PendingLspInstall {
                                language_name: lang_config.name.clone(),
                                server_command: lsp_config.command.clone(),
                                method_description: format!(
                                    "reinstall to repair a failing start: {}",
                                    describe_install_method(&auto_install_config.method)
                                ),
                                file_path: request.file_path.clone(),
                                companion_id: None,
                            })
                            .await;
                        return;
                    }
                    if repair == KnownFailureRepair::Install {
                        attempted_known_failure_repair = true;
                        let auto_install_config = lsp_config
                            .auto_install
                            .as_ref()
                            .expect("repair precondition checked");

                        crate::lsp_warn!(
                            "LSP",
                            "Known startup failure for {} ({}). Attempting one auto-repair install.",
                            lang_config.name,
                            error
                        );
                        request
                            .status(format!("LSP: Repairing {}...", lsp_config.command))
                            .await;

                        match attempt_auto_install(
                            &lang_config.name,
                            &lsp_config.command,
                            auto_install_config,
                        )
                        .await
                        {
                            InstallResult::Success(path) => {
                                server_command = find_lsp_command(lsp_config)
                                    .unwrap_or_else(|| path.to_string_lossy().to_string());
                                crate::lsp_info!(
                                    "LSP",
                                    "Auto-repair completed for {}. Retrying with '{}'",
                                    lang_config.name,
                                    server_command
                                );
                                continue;
                            }
                            InstallResult::Failed(msg) => {
                                request
                                    .status(format!("LSP: Auto-repair failed: {}", msg))
                                    .await;
                                crate::lsp_warn!("LSP", "Auto-repair failed: {}", msg);
                                return;
                            }
                            InstallResult::PrerequisitesMissing(msg) => {
                                request.status(format!("LSP: {}", msg)).await;
                                crate::lsp_warn!(
                                    "LSP",
                                    "Auto-repair prerequisites missing: {}",
                                    msg
                                );
                                return;
                            }
                        }
                    }

                    request
                        .status(format!(
                            "LSP: Failed to start {}: {}",
                            server_command, error
                        ))
                        .await;
                    crate::lsp_warn!(
                        "LSP",
                        "Failed to start {} server '{}': {}",
                        lang_config.name,
                        server_command,
                        error
                    );
                    break;
                }
            }
        }
    }
}

const KNOWN_FAILURE_REPAIR_COOLDOWN: Duration = Duration::from_secs(300);
static HEADLESS_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_headless_mode(headless: bool) {
    HEADLESS_MODE.store(headless, Ordering::Relaxed);
}

fn auto_install_on_missing_enabled(config: &AutoInstallConfig) -> bool {
    !matches!(config.policy, AutoInstallPolicy::ManualOnly)
}

fn is_auto_install_allowed_for_current_mode(config: &AutoInstallConfig) -> bool {
    if config.allow_headless {
        return true;
    }
    !is_headless_mode()
}

fn is_headless_mode() -> bool {
    HEADLESS_MODE.load(Ordering::Relaxed)
}

/// What to do about a server that failed to start in a way a reinstall
/// usually fixes.
#[derive(Debug, PartialEq, Eq)]
enum KnownFailureRepair {
    /// Nothing: not eligible, switched off, or tried recently.
    No,
    /// Reinstall right away (`autoinstall=auto`).
    Install,
    /// Ask the user first (`autoinstall=prompt`).
    Ask,
}

fn known_failure_repair(
    language_id: &str,
    command: &str,
    auto_install_config: Option<&AutoInstallConfig>,
    error: &str,
    install_mode: crate::editor::AutoInstallMode,
) -> KnownFailureRepair {
    use crate::editor::AutoInstallMode;

    let Some(config) = auto_install_config else {
        return KnownFailureRepair::No;
    };

    if !matches!(
        config.policy,
        AutoInstallPolicy::AutoOnMissingOrKnownFailure
    ) {
        return KnownFailureRepair::No;
    }

    if !is_auto_install_allowed_for_current_mode(config) {
        return KnownFailureRepair::No;
    }

    if !is_known_startup_failure(error) || install_mode == AutoInstallMode::Off {
        return KnownFailureRepair::No;
    }

    if !record_known_failure_repair_attempt(language_id, command) {
        return KnownFailureRepair::No;
    }
    match install_mode {
        AutoInstallMode::Auto => KnownFailureRepair::Install,
        _ => KnownFailureRepair::Ask,
    }
}

fn is_known_startup_failure(error: &str) -> bool {
    const KNOWN_STARTUP_FAILURE_MARKERS: &[&str] = &[
        "Failed to send initialize request",
        "Response channel closed for method 'initialize'",
        "Request 'initialize' timed out",
        "LSP server process failed to start or exited immediately",
        "LSP server not responding — channel closed (method: initialize)",
        "Failed to spawn language server",
    ];

    KNOWN_STARTUP_FAILURE_MARKERS
        .iter()
        .any(|marker| error.contains(marker))
}

fn record_known_failure_repair_attempt(language_id: &str, command: &str) -> bool {
    static KNOWN_FAILURE_REPAIR_ATTEMPTS: OnceLock<Mutex<HashMap<String, Instant>>> =
        OnceLock::new();
    let attempts = KNOWN_FAILURE_REPAIR_ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{}:{}", language_id, command);
    let now = Instant::now();

    let Ok(mut guard) = attempts.lock() else {
        // If lock is poisoned, fail open and allow one repair attempt.
        return true;
    };

    if let Some(last_attempt) = guard.get(&key) {
        if now.duration_since(*last_attempt) < KNOWN_FAILURE_REPAIR_COOLDOWN {
            return false;
        }
    }

    guard.insert(key, now);
    true
}

/// Normalize path to absolute and canonicalize if possible
///
/// Educational Note: Error Handling
/// This function returns empty PathBuf on error rather than Result<PathBuf, Error>.
/// Why? Because the caller doesn't have multiple error handling strategies - it just
/// needs to know "did this work or not". The error message is already set on the editor,
/// so returning an empty path is a simple signal to abort.
///
/// This is a pragmatic choice - not every error needs to be a Result. When there's only
/// one way to handle failure (abort), a sentinel value (empty path) is simpler.
fn normalize_path(path: &Path, editor: &mut Editor) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(path),
            Err(_) => {
                editor.set_lsp_status("LSP: Failed to get current directory".to_string());
                return PathBuf::new();
            }
        }
    };

    // Try to canonicalize, but don't fail if it doesn't work
    // (file might not exist yet, which is fine)
    match std::fs::canonicalize(&absolute) {
        Ok(canonical) => canonical,
        Err(_) => absolute,
    }
}

/// Initialize companion LSP servers for a language
///
/// After the primary LSP server starts, this checks for configured companion
/// servers (e.g., Tailwind CSS for TypeScript) and starts any that should be
/// active for the current project.
async fn initialize_companions(request: &InitRequest, language_id: &str, abs_path: &Path) {
    let companions = LanguageRegistry::get().companions_for_language(language_id);
    if companions.is_empty() {
        return;
    }

    for companion in companions {
        // Check activation markers - skip if none found in project tree
        if !companion.activation_markers.is_empty()
            && !has_activation_marker(abs_path, &companion.activation_markers)
        {
            crate::lsp_debug!(
                "LSP",
                "Skipping companion {} - no activation markers found",
                companion.name
            );
            continue;
        }

        // Find companion server command; a missing one is installed only as
        // far as the user's autoinstall setting allows.
        let Some(server_command) = find_companion_command(companion) else {
            offer_companion_install(request, companion).await;
            continue;
        };
        start_companion(request, language_id, abs_path, companion, server_command).await;
    }
}

/// A companion server that is not installed: installs it under
/// `autoinstall=auto`, asks the user under `prompt`, and only reports the
/// install hint otherwise.
async fn offer_companion_install(request: &InitRequest, companion: &CompanionLspConfig) {
    use crate::editor::AutoInstallMode;

    let installable = companion.auto_install.as_ref().filter(|config| {
        request.install_mode != AutoInstallMode::Off
            && auto_install_on_missing_enabled(config)
            && is_auto_install_allowed_for_current_mode(config)
    });
    let Some(auto_install_config) = installable else {
        match &companion.install_hint {
            Some(hint) => {
                crate::lsp_info!("LSP", "Companion {} not found. {}", companion.name, hint)
            }
            None => crate::lsp_info!(
                "LSP",
                "Companion {} not found (command: {})",
                companion.name,
                companion.command
            ),
        }
        return;
    };

    if request.install_mode == AutoInstallMode::Prompt {
        request
            .prompt(crate::editor::PendingLspInstall {
                language_name: companion.name.clone(),
                server_command: companion.command.clone(),
                method_description: describe_install_method(&auto_install_config.method),
                file_path: request.file_path.clone(),
                companion_id: Some(companion.id.clone()),
            })
            .await;
        crate::lsp_info!(
            "LSP",
            "Prompting user for auto-install consent for companion {}",
            companion.name
        );
        return;
    }

    crate::lsp_info!("LSP", "Auto-installing companion {}...", companion.name);
    request
        .status(format!("Installing {}...", companion.name))
        .await;
    install_and_start_companion(request, companion, auto_install_config).await;
}

/// Installs a companion server and, once it is there, starts it.
async fn install_and_start_companion(
    request: &InitRequest,
    companion: &CompanionLspConfig,
    auto_install_config: &AutoInstallConfig,
) {
    match attempt_auto_install(&companion.name, &companion.command, auto_install_config).await {
        InstallResult::Success(installed_path) => {
            crate::lsp_info!(
                "LSP",
                "Installed companion {}: {}",
                companion.name,
                installed_path.display()
            );
            let server_command = find_companion_command(companion)
                .unwrap_or_else(|| installed_path.to_string_lossy().to_string());
            let language_id = document_language_id(&request.language, &request.abs_path);
            start_companion(
                request,
                &language_id,
                &request.abs_path,
                companion,
                server_command,
            )
            .await;
        }
        InstallResult::Failed(e) | InstallResult::PrerequisitesMissing(e) => {
            crate::lsp_warn!(
                "LSP",
                "Failed to install companion {}: {}",
                companion.name,
                e
            );
        }
    }
}

/// The user agreed to install the companion `companion_id`.
async fn install_approved_companion(request: &InitRequest, companion_id: &str) {
    let language_id = document_language_id(&request.language, &request.abs_path);
    let companion = LanguageRegistry::get()
        .companions_for_language(&language_id)
        .into_iter()
        .find(|companion| companion.id == companion_id);
    let Some((companion, auto_install_config)) =
        companion.and_then(|c| c.auto_install.as_ref().map(|config| (c, config)))
    else {
        return;
    };
    request
        .status(format!("Installing {}...", companion.name))
        .await;
    install_and_start_companion(request, companion, auto_install_config).await;
}

/// Starts the companion server `companion` for the project of `abs_path`.
async fn start_companion(
    request: &InitRequest,
    language_id: &str,
    abs_path: &Path,
    companion: &CompanionLspConfig,
    server_command: String,
) {
    let lsp_manager = &request.manager;

    // Find project root using companion's root markers
    let root_path = if companion.root_markers.is_empty() {
        find_project_root(abs_path, &[]) // Falls back to file's directory
    } else {
        find_project_root(abs_path, &companion.root_markers)
    };

    let server_id = companion_server_id(language_id, &companion.id);

    crate::lsp_info!(
        "LSP",
        "Starting companion {} (server_id={}, command={}, root={})",
        companion.name,
        server_id,
        server_command,
        root_path.display()
    );

    match lsp_manager
        .start_companion_server(
            &server_id,
            &server_command,
            companion.args.clone(),
            &root_path,
        )
        .await
    {
        Ok(_) => {
            if crate::project_root::marker_root_with_outermost(
                abs_path,
                &companion.root_markers,
                &[],
            )
            .is_none()
            {
                lsp_manager.mark_fallback_root(&server_id);
            }
            // Start notification listener for companion
            lsp_manager
                .start_notification_listener(server_id.clone())
                .await;

            request
                .ready(language_id, server_id, server_command, false)
                .await;

            crate::lsp_info!("LSP", "Companion {} ready", companion.name);
        }
        Err(e) => {
            // Log but don't fail - companions are optional
            crate::lsp_warn!("LSP", "Failed to start companion {}: {}", companion.name, e);
        }
    }
}

/// Check if any activation marker exists in the project tree
/// Walks up from the file path looking for marker files
fn has_activation_marker(file_path: &Path, markers: &[String]) -> bool {
    let mut current = file_path.parent();
    while let Some(dir) = current {
        for marker in markers {
            if dir.join(marker).exists() {
                return true;
            }
        }
        current = dir.parent();
    }
    false
}

/// Find companion server command (primary + fallbacks)
fn find_companion_command(companion: &CompanionLspConfig) -> Option<String> {
    // Try primary command in PATH
    if which::which(&companion.command).is_ok() {
        return Some(companion.command.clone());
    }

    // Try fallback commands
    for fallback in &companion.fallback_commands {
        let expanded = shellexpand::tilde(fallback).to_string();
        if std::path::Path::new(&expanded).exists() {
            return Some(expanded);
        }
        if which::which(&expanded).is_ok() {
            return Some(expanded);
        }
    }

    None
}

/// The language id servers of `language` are started and addressed under.
fn document_language_id(language: &LanguageDefinition, abs_path: &Path) -> String {
    if language.lsp_language_id == language.config.id {
        determine_language_id(&language.config.id, abs_path)
    } else {
        language.lsp_language_id.clone()
    }
}

/// Determine language ID for LSP initialization
///
/// Educational Note: Why Special Case TypeScript?
/// The LSP protocol requires different language IDs for TypeScript ("typescript")
/// vs JavaScript ("javascript"), even though they use the same server command.
/// This is because the server needs to know which type system to use.
///
/// Alternative Approach: We could store language_id in the config as a separate
/// field, but that would duplicate data (id vs language_id). This function
/// encapsulates the special case logic in one place.
fn determine_language_id(config_id: &str, abs_path: &Path) -> String {
    // Special case: TypeScript and JavaScript share typescript-language-server
    // but need different language IDs based on file extension
    // LSP standard language IDs: typescript, typescriptreact, javascript, javascriptreact
    if config_id == "typescript" || config_id == "javascript" || config_id == "tsx" {
        let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        return match ext {
            "tsx" => "typescriptreact".to_string(),
            "jsx" => "javascriptreact".to_string(),
            "ts" | "mts" | "cts" => "typescript".to_string(),
            _ => "javascript".to_string(),
        };
    }

    // Default: use config ID as language ID
    config_id.to_string()
}

/// Generate a human-readable description of an install method for the consent dialog.
fn describe_install_method(method: &InstallMethod) -> String {
    match method {
        InstallMethod::Npm {
            package,
            packages,
            global,
            ..
        } => {
            let pkgs: Vec<&str> = packages
                .iter()
                .map(String::as_str)
                .chain(package.as_deref())
                .collect();
            if *global {
                format!("npm install -g {}", pkgs.join(" "))
            } else {
                format!(
                    "pnpm add {} (sandboxed in ~/.local/share/ovim/lsp)",
                    pkgs.join(" ")
                )
            }
        }
        InstallMethod::Cargo {
            package, features, ..
        } => {
            let feat = if features.is_empty() {
                String::new()
            } else {
                format!(" --features {}", features.join(","))
            };
            format!(
                "cargo install {}{} (sandboxed in ~/.local/share/ovim/lsp)",
                package, feat
            )
        }
        InstallMethod::Github { repo, .. } => {
            format!("download from github.com/{}", repo)
        }
        InstallMethod::Shell { command } => command.clone(),
    }
}

/// Run consented installation in the same background lifecycle as startup.
async fn install_approved(request: &InitRequest) {
    let lang_config = &request.language.config;
    let Some(lsp_config) = request.language.lsp() else {
        return;
    };
    let Some(auto_install_config) = &lsp_config.auto_install else {
        return;
    };

    // Run the actual install
    let install_result =
        attempt_auto_install(&lang_config.name, &lsp_config.command, auto_install_config).await;

    match install_result {
        InstallResult::Success(path) => {
            crate::lsp_info!(
                "LSP",
                "Auto-installed {} to {}",
                lsp_config.command,
                path.display()
            );

            // Guard against an infinite consent loop: only re-run init if the
            // binary is now discoverable. Re-running init when find_lsp_command
            // still can't locate the server would drop straight back into the
            // auto-install branch and re-raise the install prompt (the bug this
            // guard fixes — seen with go/dotnet installs whose bin dirs aren't
            // on PATH). If we can't find it, report an actionable status instead.
            if find_lsp_command(lsp_config).is_some() {
                request
                    .status(format!(
                        "LSP: {} installed successfully!",
                        lsp_config.command
                    ))
                    .await;
                // find_lsp_command will now succeed and skip auto-install.
                initialize_configured_lsp(request).await;
            } else {
                request
                    .status(format!(
                        "LSP: {} installed to {} but not found in PATH. \
                     Add its install dir to PATH and reopen the file.",
                        lsp_config.command,
                        path.display()
                    ))
                    .await;
                crate::lsp_warn!(
                    "LSP",
                    "Installed {} but it is not discoverable via find_lsp_command; \
                     not re-running init to avoid a repeated install prompt",
                    lsp_config.command
                );
            }
        }
        InstallResult::Failed(error) => {
            request
                .status(format!("LSP: Auto-install failed: {}", error))
                .await;
        }
        InstallResult::PrerequisitesMissing(msg) => {
            request.status(format!("LSP: {}", msg)).await;
        }
    }
}

/// Spawn background tasks for pending LSP install requests.
pub(crate) fn spawn_pending_installs(editor: &mut Editor) {
    use crate::editor::lsp_manager_panel::{InstallProgress, InstallStatus};

    let pending = editor.take_pending_installs();
    if pending.is_empty() {
        return;
    }

    let tx = editor.install_progress_tx().cloned();
    let Some(tx) = tx else { return };

    for request in pending {
        let tx = tx.clone();
        let lang_name = request.language_name.clone();
        let lang_id = request.language_id.clone();
        let config = request.auto_install_config.clone();
        let command = request.lsp_command.clone();

        tokio::spawn(async move {
            let _ = tx.send(InstallProgress {
                language_id: lang_id.clone(),
                status: InstallStatus::Installing(format!("Installing {lang_name}...")),
            });

            let result = auto_install::attempt_auto_install(&lang_name, &command, &config).await;

            let status = match result {
                auto_install::InstallResult::Success(_) => InstallStatus::Success,
                auto_install::InstallResult::Failed(msg) => InstallStatus::Failed(msg),
                auto_install::InstallResult::PrerequisitesMissing(msg) => {
                    InstallStatus::Failed(msg)
                }
            };

            let _ = tx.send(InstallProgress {
                language_id: lang_id,
                status,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::background::Update;
    use super::*;
    use crate::editor::AutoInstallMode;

    fn shell_install(command: &str) -> AutoInstallConfig {
        serde_json::from_value(serde_json::json!({
            "method": {"type": "shell", "command": command},
            "policy": "auto_on_missing_or_known_failure",
            "allow_headless": true,
        }))
        .unwrap()
    }

    #[test]
    fn a_failing_start_is_repaired_only_as_far_as_autoinstall_allows() {
        let config = shell_install("true");
        let error = "Failed to send initialize request";
        let decide = |language: &str, mode| {
            known_failure_repair(language, "server", Some(&config), error, mode)
        };

        assert_eq!(
            decide("repair-off", AutoInstallMode::Off),
            KnownFailureRepair::No
        );
        assert_eq!(
            decide("repair-prompt", AutoInstallMode::Prompt),
            KnownFailureRepair::Ask
        );
        assert_eq!(
            decide("repair-auto", AutoInstallMode::Auto),
            KnownFailureRepair::Install
        );
        // Once per cooldown, whatever the answer was.
        assert_eq!(
            decide("repair-auto", AutoInstallMode::Auto),
            KnownFailureRepair::No
        );
        // Not a failure a reinstall fixes.
        assert_eq!(
            known_failure_repair(
                "repair-other",
                "server",
                Some(&config),
                "permission denied",
                AutoInstallMode::Auto
            ),
            KnownFailureRepair::No
        );
    }

    /// A language server that answers `initialize` and nothing else.
    const MINIMAL_SERVER: &str = r#"
import json, sys
def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line == b"\r\n":
            break
        key, value = line.decode().split(":", 1)
        if key.lower() == "content-length":
            length = int(value)
    return json.loads(sys.stdin.buffer.read(length))
while (message := read()) is not None:
    if message.get("method") == "initialize":
        body = json.dumps({"jsonrpc": "2.0", "id": message["id"],
                           "result": {"capabilities": {}}}).encode()
        sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        sys.stdout.buffer.flush()
"#;

    fn companion(marker: &std::path::Path) -> CompanionLspConfig {
        CompanionLspConfig {
            id: "comp".into(),
            name: "Companion".into(),
            command: "python3".into(),
            args: vec!["-c".into(), MINIMAL_SERVER.into()],
            applies_to: vec!["rust".into()],
            root_markers: Vec::new(),
            activation_markers: Vec::new(),
            install_hint: None,
            auto_install: Some(shell_install(&format!("touch {}", marker.display()))),
            fallback_commands: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_missing_companion_is_installed_only_under_autoinstall_auto() {
        for (mode, installs, asks) in [
            (AutoInstallMode::Off, false, false),
            (AutoInstallMode::Prompt, false, true),
            (AutoInstallMode::Auto, true, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("installed");
            let (request, mut updates) = InitRequest::for_test("/project/main.rs", mode);

            offer_companion_install(&request, &companion(&marker)).await;

            assert_eq!(marker.exists(), installs, "{mode:?}");
            let updates: Vec<Update> = std::iter::from_fn(|| updates.try_recv().ok())
                .map(|(_, update)| update)
                .collect();
            let prompted = updates.iter().find_map(|update| match update {
                Update::Prompt(prompt) => Some(prompt),
                _ => None,
            });
            assert_eq!(prompted.is_some(), asks, "{mode:?}");
            if let Some(prompt) = prompted {
                assert_eq!(prompt.companion_id.as_deref(), Some("comp"));
                assert_eq!(prompt.server_command, "python3");
            }
            // An install that went through is verified against the companion's
            // command, and the companion is started.
            let started = updates
                .iter()
                .any(|update| matches!(update, Update::Ready { primary: false, .. }));
            assert_eq!(started, installs, "{mode:?}");
        }
    }
}
