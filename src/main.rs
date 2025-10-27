mod event_loop;
mod lsp_init;

use anyhow::Result;
use ovim::cli::{Cli, EditorArgs};
use ovim::editor::Editor;
use ovim::session::{SessionGuard, SessionInfo};
use ovim::subcommands;
use ovim::ui::UI;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

/// Sanitize session name to prevent path traversal attacks
fn sanitize_session_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .take(64) // Limit length
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse_args();

    // Check if we're running a subcommand (client mode)
    if let Some(command) = cli.command {
        // Run subcommand and exit
        return subcommands::execute_subcommand(command);
    }

    // Otherwise, run editor mode
    let args = cli.editor_args();

    // Initialize LSP logging to file
    if let Err(e) = ovim::lsp::init_lsp_logging() {
        eprintln!("Warning: Failed to initialize LSP logging: {}", e);
    }

    // Load file from command line argument if provided
    let mut editor = if let Some(file_path) = &args.file {
        let mut ed = Editor::new();
        if let Err(e) = ed.load_file(file_path) {
            // If file doesn't exist, create empty buffer with that filename
            eprintln!(
                "Note: Could not load file '{}': {}. Starting with empty buffer.",
                file_path, e
            );
            ed = Editor::new();
            ed.buffer_mut().set_file_path(file_path.clone());
        }
        ed
    } else {
        // No file specified, show welcome message
        Editor::with_content(
            "Welcome to ovim!\n\nA Neovim clone written in Rust.\n\nPress 'i' to enter Insert mode.\nPress Ctrl+Q to quit.\n",
        )
    };

    // Handle --render flag (render to ANSI and exit)
    if args.render {
        let (width, height) = args.dimension.unwrap_or((80, 24));
        match editor.render_to_ansi(width, height) {
            Ok(ansi) => {
                print!("{}", ansi);
                return Ok(());
            }
            Err(e) => {
                eprintln!("Failed to render: {}", e);
                return Err(e);
            }
        }
    }

    // Enable LSP support
    editor.enable_lsp();

    // Enable Lua support
    #[cfg(feature = "lua")]
    if let Err(e) = editor.enable_lua() {
        eprintln!("Warning: Failed to enable Lua support: {}", e);
    }

    // Create channel for Java LSP status updates (needed for both headless and TUI modes)
    let (java_status_tx, java_status_rx) = mpsc::unbounded_channel();

    // Initialize the Java status sender in the lsp_init module
    lsp_init::init_java_status_sender(java_status_tx);

    // Initialize LSP for the opened file if applicable
    if let Some(file_path) = &args.file {
        lsp_init::initialize_lsp_for_file(&mut editor, file_path).await;
        editor.clear_lsp_init_flag(); // Clear flag to prevent duplicate initialization in event loop
    }

    // Set up API server (always start in both headless and UI modes)
    let (tx, rx) = mpsc::unbounded_channel();
    let (port_tx, port_rx) = tokio::sync::oneshot::channel();

    // Spawn API server in a separate task
    // Port 0 means "pick any available port"
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        if let Err(e) = ovim::api::start_server("127.0.0.1:0", tx_clone, port_tx).await {
            eprintln!("API server error: {}", e);
        }
    });

    // Wait for the server to start and get the actual port
    let port = match port_rx.await {
        Ok(port) => port,
        Err(_) => {
            eprintln!("Failed to receive port from API server");
            return Err(anyhow::anyhow!("API server port channel closed"));
        }
    };

    // Handle headless mode
    if args.headless {
        // Write session info
        let session_name = args
            .session
            .clone()
            .unwrap_or_else(|| "default".to_string());

        // Sanitize session name to prevent path traversal attacks
        let session_name = sanitize_session_name(&session_name);

        let session_info = SessionInfo::new(port, args.file.clone(), session_name.clone());

        if let Err(e) = session_info.write() {
            eprintln!("Warning: Failed to write session info: {}", e);
        } else {
            eprintln!(
                "Session '{}' created at ~/.cache/ovim/sessions/{}.json",
                session_name, session_name
            );
        }

        // Create a guard to ensure cleanup on panic
        // This guard will automatically delete the session file when dropped,
        // even if the process panics before the signal handlers run
        let _session_guard = SessionGuard::new(session_info.clone());

        // Set up cleanup on exit - handle both SIGINT and SIGTERM
        // This fixes stale session file accumulation when killed with `kill` or `ovim-ctl kill`

        // Handle SIGINT (Ctrl+C)
        let session_info_for_sigint = session_info.clone();
        let sigint_handle = tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            match session_info_for_sigint.delete() {
                Ok(_) => eprintln!("\nSession cleaned up successfully (SIGINT)"),
                Err(e) => eprintln!("\nError during session cleanup: {}", e),
            }
            std::process::exit(0);
        });

        // Handle SIGTERM (kill command, ovim-ctl kill)
        let session_info_for_sigterm = session_info.clone();
        let sigterm_handle = tokio::spawn(async move {
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to register SIGTERM handler: {}", e);
                    return;
                }
            };
            sigterm.recv().await;
            match session_info_for_sigterm.delete() {
                Ok(_) => eprintln!("\nSession cleaned up successfully (SIGTERM)"),
                Err(e) => eprintln!("\nError during session cleanup: {}", e),
            }
            std::process::exit(0);
        });

        // Store session info and start time for health checks
        let start_time = SystemTime::now();
        let session_info_arc = Arc::new(Mutex::new(session_info));

        // Run in headless mode (API only, no TUI)
        event_loop::run_headless_loop(
            &mut editor,
            rx,
            java_status_rx,
            start_time,
            session_info_arc,
        )
        .await?;
        sigint_handle.abort();
        sigterm_handle.abort();
        return Ok(());
    }

    // Create UI for TUI mode
    let mut ui = if let Some(dimensions) = args.dimension {
        UI::with_dimensions(Some(dimensions))?
    } else {
        UI::new()?
    };

    // Print API server info for TUI mode
    eprintln!("REST API server listening on http://127.0.0.1:{}", port);

    // Main event loop with TUI (now with API support)
    event_loop::run_event_loop(&mut ui, &mut editor, Some(rx), java_status_rx).await?;

    Ok(())
}
