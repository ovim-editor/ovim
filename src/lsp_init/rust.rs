use ovim::editor::Editor;
use std::path::Path;

/// Initialize Rust LSP (rust-analyzer)
pub async fn initialize_rust_lsp(editor: &mut Editor, abs_path: &Path) {
    let language_id = "rust";
    let server_command = "rust-analyzer";
    let server_args: Vec<String> = vec![];

    // Look for Cargo.toml in parent directories for Rust
    let mut current = abs_path.parent();
    let root_path = loop {
        match current {
            Some(dir) => {
                let cargo_toml = dir.join("Cargo.toml");
                if cargo_toml.exists() {
                    break dir;
                }
                current = dir.parent();
            }
            None => {
                break abs_path.parent().unwrap_or_else(|| Path::new("/"));
            }
        }
    };

    // Start the language server
    if let Some(lsp_manager) = editor.lsp_manager() {
        // Start the server (will skip if already running)
        match lsp_manager
            .start_server(language_id, server_command, server_args, root_path)
            .await
        {
            Ok(_) => {
                editor.register_lsp_server(language_id.to_string(), server_command.to_string());

                // Start notification listener to receive diagnostics
                lsp_manager
                    .start_notification_listener(language_id.to_string())
                    .await;

                // Send didOpen notification
                let file_content = editor.buffer().rope().to_string();
                let uri = match lsp_types::Url::from_file_path(abs_path) {
                    Ok(uri) => uri,
                    Err(_) => {
                        editor.set_lsp_status("LSP: Invalid file path".to_string());
                        return;
                    }
                };

                let path_str = abs_path.to_string_lossy().to_string();

                match lsp_manager
                    .did_open(uri, language_id, 1, file_content.clone())
                    .await
                {
                    Ok(_) => {
                        // CRITICAL FIX: Initialize last_synced_content after successful didOpen
                        // Without this, the first didChange uses empty string as old_text,
                        // breaking incremental sync
                        editor.set_last_synced_content(&path_str, Some(file_content));
                        editor.set_lsp_status(format!("LSP: {} ready", server_command));
                    }
                    Err(e) => {
                        editor.set_lsp_status(format!("LSP: didOpen failed: {}", e));
                    }
                }
            }
            Err(e) => {
                editor.set_lsp_status(format!("LSP: Failed to start {}: {}", server_command, e));
                ovim::lsp_warn!("LSP", "Failed to start server '{}': {}", server_command, e);
            }
        }
    }
}
