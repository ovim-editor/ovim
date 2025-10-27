use ovim::editor::Editor;
use std::path::Path;

/// Initialize JavaScript/TypeScript LSP (typescript-language-server)
pub async fn initialize_javascript_lsp(editor: &mut Editor, abs_path: &Path) {
    // Determine language ID based on file extension
    let language_id = if abs_path.extension().and_then(|e| e.to_str()) == Some("ts") ||
                         abs_path.extension().and_then(|e| e.to_str()) == Some("tsx") {
        "typescript"
    } else {
        "javascript"
    };
    let server_command = "typescript-language-server";
    let server_args = vec!["--stdio".to_string()];

    // Use the file's parent directory as root
    let root_path = abs_path.parent().unwrap_or_else(|| Path::new("/"));

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

                // IMPORTANT: Don't send didOpen here - it will be handled by ensure_document_opened
                // when the editor actually needs to use LSP features. This avoids race conditions
                // and duplicate didOpen notifications.
                editor.set_lsp_status(format!("LSP: {} ready", server_command));
            }
            Err(e) => {
                editor.set_lsp_status(format!("LSP: Failed to start {}: {}", server_command, e));
                ovim::lsp_warn!("LSP", "Failed to start server '{}': {}", server_command, e);
            }
        }
    }
}
