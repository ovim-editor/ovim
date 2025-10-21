mod java;
mod javascript;
mod python;
mod rust;

use ovim::editor::Editor;
use std::path::Path;

pub use java::init_java_status_sender;

/// Initialize LSP for a file based on its extension
pub async fn initialize_lsp_for_file(editor: &mut Editor, file_path: &str) {
    let path = Path::new(file_path);

    // Convert to absolute path first
    let abs_path = {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(path),
                Err(_) => {
                    editor.set_lsp_status("LSP: Failed to get current directory".to_string());
                    return;
                }
            }
        };

        match std::fs::canonicalize(&absolute) {
            Ok(canonical) => canonical,
            Err(_) => absolute,
        }
    };

    let extension = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Handle Java specially with auto-setup (spawn in background to avoid blocking UI)
    if extension == "java" {
        java::handle_java_lsp(editor, abs_path).await;
        return;
    }

    // Determine language and LSP server based on file extension
    match extension {
        "rs" => rust::initialize_rust_lsp(editor, &abs_path).await,
        "js" | "ts" | "jsx" | "tsx" => javascript::initialize_javascript_lsp(editor, &abs_path).await,
        "py" => python::initialize_python_lsp(editor, &abs_path).await,
        _ => return, // No LSP support for this file type
    }
}
