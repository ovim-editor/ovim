use ovim::editor::Editor;
use ovim::lsp::uri_from_file_path;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;

// Global channel for Java LSP status updates
static JAVA_STATUS_SENDER: OnceLock<mpsc::UnboundedSender<String>> = OnceLock::new();

/// Helper to send Java status updates
pub fn send_java_status(msg: String) {
    if let Some(tx) = JAVA_STATUS_SENDER.get() {
        let _ = tx.send(format!("Java: {}", msg));
    }
}

/// Initialize the Java status sender (called from main)
pub fn init_java_status_sender(sender: mpsc::UnboundedSender<String>) {
    JAVA_STATUS_SENDER.set(sender).ok();
}

/// Find the root of a JVM project (Maven or Gradle)
/// Searches parent directories for project markers.
/// For Gradle multi-module projects, prefers settings.gradle (root) over build.gradle (subprojects)
pub fn find_jvm_project_root(file_path: &Path) -> &Path {
    // First pass: look for Gradle root markers (settings.gradle only exists at root)
    let mut current = file_path.parent();
    while let Some(dir) = current {
        if dir.join("settings.gradle").exists() || dir.join("settings.gradle.kts").exists() {
            return dir;
        }
        current = dir.parent();
    }

    // Second pass: look for Maven or single-module Gradle
    current = file_path.parent();
    while let Some(dir) = current {
        if dir.join("pom.xml").exists()
            || dir.join("build.gradle").exists()
            || dir.join("build.gradle.kts").exists()
        {
            return dir;
        }
        current = dir.parent();
    }

    // Fall back to file's parent directory if no project root found
    file_path.parent().unwrap_or_else(|| Path::new("/"))
}

/// Find the hyperion-lsp binary
fn find_hyperion_binary() -> Option<PathBuf> {
    // Check common locations in order of preference
    let candidates: Vec<Option<PathBuf>> = vec![
        // Development build (release) - prefer this for performance
        dirs::home_dir().map(|h| h.join("Personal/hyperion-ls/target/release/hyperion-lsp")),
        // Development build (debug)
        dirs::home_dir().map(|h| h.join("Personal/hyperion-ls/target/debug/hyperion-lsp")),
        // Check PATH using `which` command
        std::process::Command::new("which")
            .arg("hyperion-lsp")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim())),
    ];

    candidates
        .into_iter()
        .flatten()
        .find(|candidate| candidate.exists())
}

/// Handle Java LSP initialization (for TUI mode - spawns background task)
pub async fn handle_java_lsp(editor: &mut Editor, abs_path: PathBuf) {
    let abs_path_clone = abs_path.clone();
    let lsp_manager = editor.lsp_manager();

    // Spawn Hyperion LSP initialization in background
    tokio::spawn(async move {
        initialize_hyperion_lsp_background(lsp_manager, abs_path_clone).await;
    });
}

/// Background Hyperion LSP initialization
pub async fn initialize_hyperion_lsp_background(
    lsp_manager: Option<Arc<ovim::lsp::LspManager>>,
    file_path: PathBuf,
) {
    ovim::lsp_debug!("Java", "Starting Hyperion LSP for {:?}", file_path);

    let Some(lsp_manager) = lsp_manager else {
        send_java_status("No LSP manager available".to_string());
        return;
    };

    // Find project root
    let project_root = find_jvm_project_root(&file_path);
    ovim::lsp_debug!("Java", "Project root: {:?}", project_root);

    send_java_status("Finding Hyperion LSP...".to_string());

    // Find the hyperion-lsp binary
    let hyperion_bin = match find_hyperion_binary() {
        Some(bin) => {
            ovim::lsp_debug!("Java", "Found Hyperion at {:?}", bin);
            bin
        }
        None => {
            send_java_status(
                "Hyperion LSP not found. Install it or build from source.".to_string(),
            );
            return;
        }
    };

    send_java_status("Starting Hyperion LSP...".to_string());

    // Start the LSP server (no args needed - runs in stdio mode)
    let server_command = hyperion_bin.to_string_lossy().to_string();
    let server_args: Vec<String> = vec![];

    match lsp_manager
        .start_server("java", &server_command, server_args, project_root)
        .await
    {
        Ok(()) => {
            send_java_status("Server started".to_string());
        }
        Err(e) => {
            send_java_status(format!("Failed to start: {}", e));
            return;
        }
    }

    // Start notification listener
    lsp_manager
        .start_notification_listener("java".to_string())
        .await;

    send_java_status("Ready".to_string());
}

/// Synchronous version for headless mode
#[allow(dead_code)]
pub async fn initialize_java_lsp(editor: &mut Editor, file_path: &Path) {
    let project_root = find_jvm_project_root(file_path);

    editor.set_lsp_status("Java: Finding Hyperion LSP...".to_string());

    let hyperion_bin = match find_hyperion_binary() {
        Some(bin) => bin,
        None => {
            editor.set_lsp_status("Java: Hyperion LSP not found".to_string());
            return;
        }
    };

    editor.set_lsp_status("Java: Starting Hyperion LSP...".to_string());

    if let Some(lsp_manager) = editor.lsp_manager() {
        let server_command = hyperion_bin.to_string_lossy().to_string();

        match lsp_manager
            .start_server("java", &server_command, vec![], project_root)
            .await
        {
            Ok(_) => {
                editor.register_lsp_server("java".to_string(), "hyperion".to_string());

                lsp_manager
                    .start_notification_listener("java".to_string())
                    .await;

                // PRE-WARM: Send didOpen immediately for faster first request
                if let Some(file_path_str) = editor.buffer().file_path().map(|s| s.to_string()) {
                    let content = editor.buffer().rope().to_string();
                    if let Some(uri) = uri_from_file_path(&file_path_str) {
                        let _ = lsp_manager.did_open(uri, "java", 1, content).await;
                        editor.mark_document_opened(&file_path_str);
                        ovim::lsp_debug!("Java", "Pre-warmed didOpen for {}", file_path_str);
                    }
                }

                editor.set_lsp_status("Java: Ready".to_string());
            }
            Err(e) => {
                editor.set_lsp_status(format!("Java: Failed to start: {}", e));
            }
        }
    }
}
