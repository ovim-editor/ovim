pub mod auto_install;
mod java;

use auto_install::{attempt_auto_install, InstallResult};
use ovim::editor::Editor;
use ovim::language_config::{
    find_lsp_command, find_project_root, CompanionLspConfig, LanguageRegistry,
};
use ovim::lsp::companion_server_id;
use ovim::lsp::uri_from_file_path;
use std::path::{Path, PathBuf};

pub use java::init_java_status_sender;

/// Initialize LSP for a file using the language configuration system
///
/// Educational Note: Refactoring Strategy
/// This function previously used hardcoded match statements for each language.
/// Now it uses the declarative LanguageRegistry for a data-driven approach:
///
/// Benefits:
/// - Add languages via config file (no code changes)
/// - Unified root finding and command discovery
/// - Better error messages with install hints
/// - Easier to test (mock configs vs mock modules)
///
/// Special Cases Preserved:
/// - Java: Complex auto-download logic remains in java.rs module
///   This is intentional - when special cases provide real value, keep them.
///   We preserve backward compatibility while improving the common case.
pub async fn initialize_lsp_for_file(editor: &mut Editor, file_path: &str) {
    let path = Path::new(file_path);

    // Convert to absolute path first
    let abs_path = normalize_path(path, editor);
    if abs_path.as_os_str().is_empty() {
        return; // Error already set in normalize_path
    }

    // Special case: Java has complex auto-download logic
    // Keep using the dedicated module for now (will be generalized in Phase 3+)
    let extension = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if extension == "java" {
        java::handle_java_lsp(editor, abs_path).await;
        return;
    }

    // Detect language from registry
    let Some(lang_config) = LanguageRegistry::get().detect(&abs_path) else {
        // No language configuration found - this is fine for unknown file types
        return;
    };

    // Check if LSP is configured for this language
    let Some(lsp_config) = &lang_config.lsp else {
        // Syntax highlighting only, no LSP - this is normal for languages like Markdown
        return;
    };

    // Try to find LSP server binary (primary command + fallbacks)
    let server_command = match find_lsp_command(lsp_config) {
        Some(cmd) => cmd,
        None => {
            // LSP server not found - try auto-install if configured
            if let Some(auto_install_config) = &lsp_config.auto_install {
                ovim_core::lsp_info!(
                    "LSP",
                    "{} language server not found. Attempting auto-install...",
                    lang_config.name
                );

                editor.set_lsp_status(format!("LSP: Installing {}...", lsp_config.command));

                // Attempt auto-install
                let install_result = attempt_auto_install(
                    &lang_config.name,
                    &lsp_config.command,
                    auto_install_config,
                )
                .await;

                match install_result {
                    InstallResult::Success(path) => {
                        editor.set_lsp_status(format!(
                            "LSP: {} installed successfully!",
                            lsp_config.command
                        ));
                        ovim_core::lsp_info!(
                            "LSP",
                            "Auto-installed {} to {}",
                            lsp_config.command,
                            path.display()
                        );

                        // Use the installed command
                        path.to_string_lossy().to_string()
                    }
                    InstallResult::Failed(error) => {
                        editor.set_lsp_status(format!("LSP: Auto-install failed: {}", error));
                        ovim_core::lsp_warn!("LSP", "Auto-install failed: {}", error);
                        return;
                    }
                    InstallResult::PrerequisitesMissing(msg) => {
                        editor.set_lsp_status(format!("LSP: {}", msg));
                        ovim_core::lsp_warn!("LSP", "Prerequisites missing: {}", msg);
                        return;
                    }
                    InstallResult::Declined => {
                        editor.set_lsp_status("LSP: Installation declined".to_string());
                        return;
                    }
                }
            } else {
                // No auto-install configured - show manual install hint
                let hint = lsp_config
                    .install_hint
                    .as_deref()
                    .unwrap_or("LSP server not found in PATH");

                editor.set_lsp_status(format!("LSP: {}", hint));
                ovim_core::lsp_warn!(
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
    let root_path = find_project_root(&abs_path, &lsp_config.root_markers);

    // Determine language ID (for TypeScript vs JavaScript, use extension-based logic)
    let language_id = determine_language_id(&lang_config.id, &abs_path);

    ovim_core::lsp_info!(
        "LSP",
        "Initializing {} LSP: command={}, root={}, language_id={}",
        lang_config.name,
        server_command,
        root_path.display(),
        language_id
    );

    // Start LSP server using the unified path
    if let Some(lsp_manager) = editor.lsp_manager() {
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
                editor.register_lsp_server(language_id.clone(), server_command.clone());

                // Start notification listener to receive diagnostics
                // Use server_id (may differ from language_id for multi-root)
                lsp_manager.start_notification_listener(server_id).await;

                // PRE-WARM: Send didOpen immediately to eliminate first-request latency
                // This ensures the LSP server has indexed the document before the first
                // hover/goto_definition request, making K/gd feel instant.
                if let Some(file_path) = editor.buffer().file_path().map(|s| s.to_string()) {
                    let content = editor.buffer().rope().to_string();
                    if let Some(uri) = uri_from_file_path(&file_path) {
                        match lsp_manager
                            .did_open_broadcast(uri, &language_id, 1, content.clone())
                            .await
                        {
                            Ok(_) => {
                                // Mark as sent+synced to prevent duplicate from ensure_lsp_document_synced
                                editor.mark_document_opened_with_content(&file_path, content);
                                ovim_core::lsp_debug!(
                                    "LSP",
                                    "Pre-warmed didOpen for {}",
                                    file_path
                                );
                            }
                            Err(e) => {
                                // Don't mark as opened — ensure_lsp_document_synced will retry
                                ovim_core::lsp_warn!(
                                    "LSP",
                                    "Pre-warm didOpen failed for {}: {} (will retry on next LSP request)",
                                    file_path,
                                    e
                                );
                            }
                        }
                    }
                }

                editor.set_lsp_status(format!("LSP: {} ready", lang_config.name));

                // Initialize companion LSP servers (e.g., Tailwind CSS for TypeScript)
                initialize_companions(editor, &language_id, &abs_path).await;
            }
            Err(e) => {
                editor.set_lsp_status(format!("LSP: Failed to start {}: {}", server_command, e));
                ovim_core::lsp_warn!(
                    "LSP",
                    "Failed to start {} server '{}': {}",
                    lang_config.name,
                    server_command,
                    e
                );
            }
        }
    }
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
async fn initialize_companions(editor: &mut Editor, language_id: &str, abs_path: &Path) {
    let companions = LanguageRegistry::get().companions_for_language(language_id);
    if companions.is_empty() {
        return;
    }

    let lsp_manager = match editor.lsp_manager() {
        Some(lsp) => lsp.clone(),
        None => return,
    };

    for companion in companions {
        // Check activation markers - skip if none found in project tree
        if !companion.activation_markers.is_empty()
            && !has_activation_marker(abs_path, &companion.activation_markers)
        {
            ovim_core::lsp_debug!(
                "LSP",
                "Skipping companion {} - no activation markers found",
                companion.name
            );
            continue;
        }

        // Find companion server command
        let server_command = match find_companion_command(companion) {
            Some(cmd) => cmd,
            None => {
                if let Some(hint) = &companion.install_hint {
                    ovim_core::lsp_info!("LSP", "Companion {} not found. {}", companion.name, hint);
                } else {
                    ovim_core::lsp_info!(
                        "LSP",
                        "Companion {} not found (command: {})",
                        companion.name,
                        companion.command
                    );
                }
                continue;
            }
        };

        // Find project root using companion's root markers
        let root_path = if companion.root_markers.is_empty() {
            find_project_root(abs_path, &[]) // Falls back to file's directory
        } else {
            find_project_root(abs_path, &companion.root_markers)
        };

        let server_id = companion_server_id(language_id, &companion.id);

        ovim_core::lsp_info!(
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
                // Start notification listener for companion
                lsp_manager
                    .start_notification_listener(server_id.clone())
                    .await;

                // Send didOpen to companion for current file
                if let Some(file_path) = editor.buffer().file_path().map(|s| s.to_string()) {
                    let content = editor.buffer().rope().to_string();
                    if let Some(uri) = uri_from_file_path(&file_path) {
                        let _ = lsp_manager.did_open(uri, &server_id, 1, content).await;
                        ovim_core::lsp_debug!(
                            "LSP",
                            "Sent didOpen to companion {} for {}",
                            companion.name,
                            file_path
                        );
                    }
                }

                ovim_core::lsp_info!("LSP", "Companion {} ready", companion.name);
            }
            Err(e) => {
                // Log but don't fail - companions are optional
                ovim_core::lsp_warn!("LSP", "Failed to start companion {}: {}", companion.name, e);
            }
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
