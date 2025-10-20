mod event_loop;
mod lsp_init;

use anyhow::Result;
use ovim::cli::Args;
use ovim::editor::Editor;
use ovim::session::SessionInfo;
use ovim::ui::UI;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse_args();

    // Initialize LSP logging to file
    if let Err(e) = ovim::lsp::init_lsp_logging() {
        eprintln!("Warning: Failed to initialize LSP logging: {}", e);
    }

    // Load file from command line argument if provided
    let mut editor = if let Some(file_path) = &args.file {
        let mut ed = Editor::new();
        if let Err(_e) = ed.load_file(file_path) {
            // If file doesn't exist, create empty buffer with that filename
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

    // Set up API server if requested
    let api_rx = if args.headless {
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
        let port = port_rx.await.expect("Failed to get server port");

        // Write session info
        let session_name = args
            .session
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let session_info = SessionInfo::new(port, args.file.clone(), session_name.clone());

        if let Err(e) = session_info.write() {
            eprintln!("Warning: Failed to write session info: {}", e);
        } else {
            eprintln!(
                "Session '{}' created at ~/.cache/ovim/sessions/{}.json",
                session_name, session_name
            );
        }

        // Set up cleanup on exit
        let session_info_for_cleanup = session_info.clone();
        let cleanup_handle = tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            let _ = session_info_for_cleanup.delete();
            eprintln!("\nSession cleaned up");
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
        cleanup_handle.abort();
        return Ok(());
    } else {
        None
    };

    // Create UI only if not running in API mode
    let mut ui = if let Some(dimensions) = args.dimension {
        UI::with_dimensions(Some(dimensions))?
    } else {
        UI::new()?
    };

    // Main event loop with TUI
    event_loop::run_event_loop(&mut ui, &mut editor, api_rx, java_status_rx).await?;

    Ok(())
}
