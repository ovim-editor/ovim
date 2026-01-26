mod auto_install;
mod java;

use auto_install::{attempt_auto_install, InstallResult};
use ovim::editor::Editor;
use ovim::language_config::{find_lsp_command, find_project_root, LanguageRegistry};
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
                ovim::lsp_info!(
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
                        ovim::lsp_info!(
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
                        ovim::lsp_warn!("LSP", "Auto-install failed: {}", error);
                        return;
                    }
                    InstallResult::PrerequisitesMissing(msg) => {
                        editor.set_lsp_status(format!("LSP: {}", msg));
                        ovim::lsp_warn!("LSP", "Prerequisites missing: {}", msg);
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
                ovim::lsp_warn!(
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

    ovim::lsp_info!(
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
            .start_server(&language_id, &server_command, lsp_config.args.clone(), &root_path)
            .await
        {
            Ok(_) => {
                editor.register_lsp_server(language_id.clone(), server_command.clone());

                // Start notification listener to receive diagnostics
                lsp_manager.start_notification_listener(language_id.clone()).await;

                // PRE-WARM: Send didOpen immediately to eliminate first-request latency
                // This ensures the LSP server has indexed the document before the first
                // hover/goto_definition request, making K/gd feel instant.
                if let Some(file_path) = editor.buffer().file_path().map(|s| s.to_string()) {
                    let content = editor.buffer().rope().to_string();
                    if let Some(uri) = uri_from_file_path(&file_path) {
                        let _ = lsp_manager.did_open(uri, &language_id, 1, content).await;
                        // Mark as sent to prevent duplicate from ensure_lsp_document_synced
                        editor.mark_document_opened(&file_path);
                        ovim::lsp_debug!("LSP", "Pre-warmed didOpen for {}", file_path);
                    }
                }

                editor.set_lsp_status(format!("LSP: {} ready", lang_config.name));
            }
            Err(e) => {
                editor.set_lsp_status(format!("LSP: Failed to start {}: {}", server_command, e));
                ovim::lsp_warn!(
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
