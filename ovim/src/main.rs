//! # TUI Safety: No stdout/stderr output!
//! Use log_info!, log_warn!, log_error!, log_debug! instead of println!/eprintln!
#![deny(clippy::print_stdout, clippy::print_stderr)]

mod event_loop;
mod lsp_init;

use anyhow::Result;
use ovim::cli::Cli;
use ovim::editor::Editor;
use ovim::mode::Mode;
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
    // Initialize logging FIRST - before anything else
    if let Err(e) = ovim::log::init() {
        let _ = e;
    }
    ovim_core::log_info!("main", "ovim starting up");

    let cli = Cli::parse_args();

    // Initialize language registry early (needed for both editor and subcommands)
    if let Err(e) = ovim::language_config::LanguageRegistry::init() {
        ovim_core::log_warn!("main", "Failed to initialize language registry: {}", e);
        ovim_core::log_warn!("main", "Continuing with limited language support...");
    }

    // Check if we're running a subcommand (client mode)
    if let Some(command) = cli.command {
        // Run subcommand and exit
        return subcommands::execute_subcommand(command);
    }

    // Otherwise, run editor mode
    let file_arg = cli.file_arg();
    let headless = cli.headless;
    let session_name = cli.session.clone();
    let dimension = cli.dimension;
    let render = cli.render;

    // Track runtime mode for components that need different behavior in headless mode.
    if headless {
        std::env::set_var("OVIM_HEADLESS", "1");
    } else {
        std::env::remove_var("OVIM_HEADLESS");
    }

    // Initialize LSP logging to file
    if let Err(e) = ovim::lsp::init_lsp_logging() {
        ovim_core::log_warn!("main", "Failed to initialize LSP logging: {}", e);
    }

    // Load file from command line argument if provided
    let mut editor = if let Some(ref file) = file_arg {
        let mut ed = Editor::new();
        if let Err(e) = ed.load_file(&file.path) {
            ovim_core::log_warn!(
                "main",
                "Could not load file '{}': {}. Starting with empty buffer.",
                file.path,
                e
            );
            ed = Editor::new();
            ed.set_file_path(file.path.clone());
        }
        // Jump to line:col if specified
        if let Some(line) = file.line {
            let line_0 = line.saturating_sub(1);
            let col_0 = file.col.unwrap_or(1).saturating_sub(1);
            ed.buffer_mut().cursor_mut().set_position(line_0, col_0);
            ed.buffer_mut().validate_cursor_position();
        }
        // Switch from Dashboard to Normal mode when a file is loaded
        ed.set_mode(Mode::Normal);
        ed
    } else {
        // No file specified, start with empty buffer (dashboard will show)
        Editor::new()
    };
    // Set up cat animation (concrete type lives in binary crate)
    editor.ui_panels.cat_animation = Some(Box::new(ovim::ui::CatAnimation::new()));

    // Handle --render flag (render to ANSI and exit)
    if render {
        let (width, height) = dimension.unwrap_or((80, 24));
        match ovim::ui::render_editor_to_ansi(&mut editor, width, height) {
            Ok(ansi) => {
                #[allow(clippy::print_stdout)]
                {
                    print!("{}", ansi);
                }
                return Ok(());
            }
            Err(e) => {
                #[allow(clippy::print_stderr)]
                {
                    eprintln!("Failed to render: {}", e);
                }
                return Err(e);
            }
        }
    }

    // Enable LSP support
    editor.enable_lsp();

    // Enable Lua support
    if let Err(e) = editor.enable_lua() {
        ovim_core::log_error!("main", "Failed to enable Lua support: {}", e);
    }

    // Create channel for Java LSP status updates (needed for both headless and TUI modes)
    let (java_status_tx, java_status_rx) = mpsc::unbounded_channel();

    // Initialize the Java status sender in the lsp_init module
    lsp_init::init_java_status_sender(java_status_tx);

    // Set up API server (always start in both headless and UI modes)
    let (tx, rx) = mpsc::unbounded_channel();
    let (port_tx, port_rx) = tokio::sync::oneshot::channel();

    // Spawn API server in a separate task
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        if let Err(e) = ovim::api::start_server("127.0.0.1:0", tx_clone, port_tx).await {
            ovim_core::lsp_error!("API", "API server error: {}", e);
        }
    });

    // Wait for the server to start and get the actual port
    let port = match port_rx.await {
        Ok(port) => port,
        Err(_) => {
            ovim_core::lsp_error!("API", "Failed to receive port from API server");
            return Err(anyhow::anyhow!("API server port channel closed"));
        }
    };

    // Store API port in editor for :session start/stop commands
    editor.api_port = Some(port);

    // Handle headless mode
    // Headless mode uses stderr for user feedback (no TUI), so eprintln! is safe
    #[allow(clippy::print_stderr)]
    if headless {
        // Require --session NAME for headless mode
        let session_name = match session_name {
            Some(name) => sanitize_session_name(&name),
            None => {
                eprintln!("Error: --headless requires --session NAME");
                eprintln!("Usage: ovim <file> --headless --session <name>");
                std::process::exit(1);
            }
        };

        let file_path = file_arg.map(|f| f.path);
        let session_info = SessionInfo::new(port, file_path, session_name.clone());

        if let Err(e) = session_info.write() {
            eprintln!("Warning: Failed to write session info: {}", e);
        } else {
            eprintln!(
                "Session '{}' created at ~/.cache/ovim/sessions/{}.json",
                session_name, session_name
            );
        }

        // Create a guard to ensure cleanup on panic
        let _session_guard = SessionGuard::new(session_info.clone());

        // Set up cleanup on exit - handle both SIGINT and SIGTERM
        let session_info_for_sigint = session_info.clone();
        let sigint_handle = tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            match session_info_for_sigint.delete() {
                Ok(_) => eprintln!("\nSession cleaned up successfully (SIGINT)"),
                Err(e) => eprintln!("\nError during session cleanup: {}", e),
            }
            std::process::exit(0);
        });

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
        let code = editor.exit_code();
        if code != 0 {
            std::process::exit(code);
        }
        return Ok(());
    }

    // TUI mode - no session registration by default
    // The API server still runs for internal communication,
    // but no session file is written. Users can opt in with :session start NAME.

    // Create UI for TUI mode
    let mut ui = if let Some(dimensions) = dimension {
        UI::with_dimensions(Some(dimensions))?
    } else {
        UI::new()?
    };

    // Main event loop with TUI (now with API support)
    event_loop::run_event_loop(&mut ui, &mut editor, Some(rx), java_status_rx).await?;

    let code = editor.exit_code();

    // Drop UI first to restore terminal before exiting
    drop(ui);

    if code != 0 {
        std::process::exit(code);
    }

    Ok(())
}
