mod syntax;
mod buffer;
mod editor;
mod ui;
mod mode;
mod api;
mod cli;
mod lsp;

use anyhow::Result;
use crossterm::event::Event;
use editor::{Editor, InputHandler};
use ui::UI;
use cli::Args;
use api::{ApiRequest, ApiResponse, BufferInfo, CursorPosition, EditorSnapshot, ErrorResponse, ModeInfo, PickerInfo, PickerResultInfo, RenderInfo, SuccessResponse, VisualSelection, parse_key_string};
use std::collections::HashMap;
use tokio::sync::mpsc;
use std::sync::OnceLock;

// Global channel for Java LSP status updates
static JAVA_STATUS_SENDER: OnceLock<mpsc::UnboundedSender<String>> = OnceLock::new();

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
            "Welcome to ovim!\n\nA Neovim clone written in Rust.\n\nPress 'i' to enter Insert mode.\nPress Ctrl+Q to quit.\n"
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

    // Store the sender in a static for background tasks to use
    JAVA_STATUS_SENDER.set(java_status_tx).ok();

    // Initialize LSP for the opened file if applicable
    if let Some(file_path) = &args.file {
        initialize_lsp_for_file(&mut editor, file_path).await;
    }

    // Set up API server if requested
    let api_rx = if args.headless {
        let (tx, rx) = mpsc::unbounded_channel();

        // Spawn API server in a separate task
        // Port 0 means "pick any available port"
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = api::start_server("127.0.0.1:0", tx_clone).await {
                eprintln!("API server error: {}", e);
            }
        });

        // Give the server a moment to start and print its address
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Run in headless mode (API only, no TUI)
        run_headless_loop(&mut editor, rx, java_status_rx).await?;
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
    run_event_loop(&mut ui, &mut editor, api_rx, java_status_rx).await?;

    Ok(())
}

async fn run_headless_loop(
    editor: &mut Editor,
    mut api_rx: mpsc::UnboundedReceiver<ApiRequest>,
    mut java_status_rx: mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    use tokio::time::{Duration, sleep};

    loop {
        // Check for Java LSP status updates
        while let Ok(status) = java_status_rx.try_recv() {
            editor.set_lsp_status(status);
        }

        // Process LSP notifications (diagnostics, etc.)
        // Use try_lock to avoid blocking if background task (e.g., Java init) holds lock
        if let Some(lsp_manager) = editor.lsp_manager() {
            if let Ok(lsp) = lsp_manager.try_lock() {
                lsp.process_notifications().await;
                lsp.process_flush_requests().await;
            }
            // If lock is held, skip this iteration - background task is working
        }

        // Initialize LSP for newly loaded files
        if let Some(file_path) = editor.needs_lsp_init() {
            initialize_lsp_for_file(editor, &file_path).await;
            editor.clear_lsp_init_flag();
        }

        // Process any pending LSP actions
        editor.process_pending_lsp_actions().await;

        // Process any pending Lua commands
        #[cfg(feature = "lua")]
        let _ = editor.process_lua_commands();

        // Update diagnostic cache only if diagnostics changed
        // Use try_lock to avoid blocking if background task holds lock
        if let Some(lsp_manager) = editor.lsp_manager() {
            if let Ok(lsp) = lsp_manager.try_lock() {
                if lsp.diagnostics_changed() {
                    drop(lsp); // Release lock before async call
                    editor.update_diagnostic_cache().await;
                }
            }
        }

        // Check for API requests (non-blocking with timeout)
        match tokio::time::timeout(Duration::from_millis(50), api_rx.recv()).await {
            Ok(Some(request)) => {
                handle_api_request(editor, request).await;
                // Check if quit was requested
                if editor.should_quit() {
                    break;
                }
            }
            Ok(None) => {
                // Channel closed, exit
                break;
            }
            Err(_) => {
                // Timeout - no request received, continue loop
            }
        }

        // Send LSP notifications if needed
        editor.send_lsp_changes_if_modified().await;
        editor.send_lsp_save_if_needed().await;

        // Small sleep to avoid busy loop
        sleep(Duration::from_millis(10)).await;
    }

    Ok(())
}

async fn run_event_loop(
    ui: &mut UI,
    editor: &mut Editor,
    mut api_rx: Option<mpsc::UnboundedReceiver<ApiRequest>>,
    mut java_status_rx: mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    use tokio::time::{Duration, Instant};

    let mut last_edit = Instant::now();
    let debounce_delay = Duration::from_millis(100);

    while !editor.should_quit() {
        // Check for Java LSP status updates
        while let Ok(status) = java_status_rx.try_recv() {
            editor.set_lsp_status(status);
        }

        // Process LSP notifications (diagnostics, etc.)
        // Use try_lock to avoid blocking if background task (e.g., Java init) holds lock
        if let Some(lsp_manager) = editor.lsp_manager() {
            if let Ok(lsp) = lsp_manager.try_lock() {
                lsp.process_notifications().await;
                lsp.process_flush_requests().await;
            }
            // If lock is held, skip this iteration - background task is working
        }

        // Initialize LSP for newly loaded files
        if let Some(file_path) = editor.needs_lsp_init() {
            initialize_lsp_for_file(editor, &file_path).await;
            editor.clear_lsp_init_flag();
        }

        // Update diagnostic cache only if diagnostics changed
        // Use try_lock to avoid blocking if background task holds lock
        if let Some(lsp_manager) = editor.lsp_manager() {
            if let Ok(lsp) = lsp_manager.try_lock() {
                if lsp.diagnostics_changed() {
                    drop(lsp); // Release lock before async call
                    editor.update_diagnostic_cache().await;
                }
            }
        }

        // Check if enough time has passed since last edit for re-highlighting
        if editor.buffer().needs_rehighlight() && last_edit.elapsed() >= debounce_delay {
            editor.process_pending_rehighlight().await;
        }

        // Process any pending LSP actions
        editor.process_pending_lsp_actions().await;

        // Process any pending Lua commands
        #[cfg(feature = "lua")]
        let _ = editor.process_lua_commands();

        // Render the editor
        ui.renderer_mut().render(editor)?;

        // Check for API requests (non-blocking)
        if let Some(ref mut rx) = api_rx {
            while let Ok(request) = rx.try_recv() {
                handle_api_request(editor, request).await;
            }
        }

        // Poll for events with a timeout to allow checking API requests
        if let Some(event) = InputHandler::poll_event()? {
            if let Event::Key(key_event) = event {
                InputHandler::handle_key_event(editor, key_event)?;
                // Reset debounce timer on any edit
                last_edit = Instant::now();
            }
        }

        // Send LSP notifications if needed
        editor.send_lsp_changes_if_modified().await;
        editor.send_lsp_save_if_needed().await;
        editor.send_lsp_close_if_needed().await;
    }

    // Send didClose for current file on shutdown
    editor.close_current_file_lsp().await;

    Ok(())
}

async fn handle_api_request(editor: &mut Editor, request: ApiRequest) {
    match request {
        ApiRequest::GetSnapshot(tx) => {
            let snapshot = create_snapshot(editor);
            let _ = tx.send(ApiResponse::Snapshot(snapshot));
        }
        ApiRequest::SendKeys(keys, tx) => {
            let events = parse_key_string(&keys);
            let mut success = true;

            for event in events {
                if let Err(_) = InputHandler::handle_key_event(editor, event) {
                    success = false;
                    break;
                }
            }

            // Process any LSP actions that were triggered by the keys
            editor.process_pending_lsp_actions().await;

            let response = if success {
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: None,
                    line_count: None,
                })
            } else {
                ApiResponse::Error(ErrorResponse {
                    error: "Failed to process keys".to_string(),
                })
            };
            let _ = tx.send(response);
        }
        ApiRequest::GetBuffer(tx) => {
            let buffer_info = create_buffer_info(editor);
            let _ = tx.send(ApiResponse::Buffer(buffer_info));
        }
        ApiRequest::SetBuffer(content, tx) => {
            editor.buffer_mut().replace_all(&content);
            let line_count = editor.buffer().rope().len_lines();

            let response = ApiResponse::Success(SuccessResponse {
                success: true,
                message: None,
                line_count: Some(line_count),
            });
            let _ = tx.send(response);
        }
        ApiRequest::GetCursor(tx) => {
            let cursor = editor.buffer().cursor();
            let pos = CursorPosition {
                line: cursor.line(),
                column: cursor.col(),
            };
            let _ = tx.send(ApiResponse::Cursor(pos));
        }
        ApiRequest::GetMode(tx) => {
            let mode_info = ModeInfo {
                mode: editor.mode().display_name().to_string(),
            };
            let _ = tx.send(ApiResponse::Mode(mode_info));
        }
        ApiRequest::ExecuteCommand(command, tx) => {
            let response = execute_command(editor, &command);
            let _ = tx.send(response);
        }
        ApiRequest::GetRender(tx) => {
            // Default dimensions: 80x24
            const DEFAULT_WIDTH: u16 = 80;
            const DEFAULT_HEIGHT: u16 = 24;

            match editor.render_to_ansi(DEFAULT_WIDTH, DEFAULT_HEIGHT) {
                Ok(ansi) => {
                    let render_info = RenderInfo {
                        width: DEFAULT_WIDTH,
                        height: DEFAULT_HEIGHT,
                        ansi,
                    };
                    let _ = tx.send(ApiResponse::Render(render_info));
                }
                Err(e) => {
                    let _ = tx.send(ApiResponse::Error(ErrorResponse {
                        error: format!("Failed to render: {}", e),
                    }));
                }
            }
        }
    }
}

fn create_snapshot(editor: &Editor) -> EditorSnapshot {
    let buffer_info = create_buffer_info(editor);
    let cursor = editor.buffer().cursor();

    let cursor_pos = CursorPosition {
        line: cursor.line(),
        column: cursor.col(),
    };

    let visual_selection = editor.visual_selection().map(|((start_line, start_col), (end_line, end_col))| {
        VisualSelection {
            start: CursorPosition {
                line: start_line,
                column: start_col,
            },
            end: CursorPosition {
                line: end_line,
                column: end_col,
            },
        }
    });

    // Get registers content
    let mut registers = HashMap::new();
    let reg_manager = editor.registers();
    for reg_name in &['"', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z'] {
        let content = reg_manager.get(Some(*reg_name));
        if !content.is_empty() {
            registers.insert(reg_name.to_string(), content.to_string());
        }
    }

    // Get marks
    let marks = HashMap::new();
    // TODO: Add marks iteration when MarkManager supports it

    // Get picker state if in picker mode
    let picker = editor.picker().map(|p| {
        PickerInfo {
            mode: match p.mode() {
                crate::editor::PickerMode::FindFiles => "FindFiles".to_string(),
                crate::editor::PickerMode::LiveGrep => "LiveGrep".to_string(),
                crate::editor::PickerMode::Custom => "Custom".to_string(),
                crate::editor::PickerMode::Completion => "Completion".to_string(),
                crate::editor::PickerMode::LspLocations => "LspLocations".to_string(),
            },
            query: p.query().to_string(),
            results: p.filtered_results().iter().map(|r| {
                PickerResultInfo {
                    display: r.display.clone(),
                    location: r.location.clone(),
                    line: r.line,
                    col: r.col,
                }
            }).collect(),
            selected_index: p.selected_index(),
        }
    });

    EditorSnapshot {
        buffer: buffer_info,
        cursor: cursor_pos,
        mode: editor.mode().display_name().to_string(),
        visual_selection,
        registers,
        marks,
        picker,
    }
}

fn create_buffer_info(editor: &Editor) -> BufferInfo {
    let buffer = editor.buffer();
    let content = buffer.rope().to_string();
    let line_count = buffer.rope().len_lines();
    let file_path = buffer.file_path().map(|s| s.to_string());

    BufferInfo {
        content,
        line_count,
        file_path,
    }
}

fn execute_command(editor: &mut Editor, command: &str) -> ApiResponse {
    match command {
        "q" | "quit" => {
            if editor.is_modified() {
                ApiResponse::Error(ErrorResponse {
                    error: "No write since last change (add ! to override)".to_string(),
                })
            } else {
                editor.quit();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some("Quitting".to_string()),
                    line_count: None,
                })
            }
        }
        "q!" | "quit!" => {
            editor.quit();
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("Quitting (forced)".to_string()),
                line_count: None,
            })
        }
        "qa" | "qall" => {
            // Quit all - same as quit for single buffer
            if editor.is_modified() {
                ApiResponse::Error(ErrorResponse {
                    error: "No write since last change (add ! to override)".to_string(),
                })
            } else {
                editor.quit();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some("Quitting all".to_string()),
                    line_count: None,
                })
            }
        }
        "qa!" | "qall!" => {
            // Force quit all without saving
            editor.quit();
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("Quitting all (forced)".to_string()),
                line_count: None,
            })
        }
        "w" | "write" => {
            if let Some(path) = editor.buffer().file_path().map(|s| s.to_string()) {
                match editor.buffer_mut().save_as(&path) {
                    Ok(_) => {
                        editor.mark_saved();
                        editor.mark_buffer_saved(); // Mark for LSP didSave notification
                        let line_count = editor.buffer().rope().len_lines();
                        let char_count = editor.buffer().rope().len_chars();
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("\"{}\" {}L, {}C written", path, line_count, char_count)),
                            line_count: None,
                        })
                    }
                    Err(e) => ApiResponse::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            } else {
                ApiResponse::Error(ErrorResponse {
                    error: "No file name".to_string(),
                })
            }
        }
        "wq" => {
            if let Some(path) = editor.buffer().file_path().map(|s| s.to_string()) {
                match editor.buffer_mut().save_as(&path) {
                    Ok(_) => {
                        editor.mark_saved();
                        editor.mark_buffer_saved(); // Mark for LSP didSave notification
                        editor.quit();
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some("Saved and quitting".to_string()),
                            line_count: None,
                        })
                    }
                    Err(e) => ApiResponse::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            } else {
                ApiResponse::Error(ErrorResponse {
                    error: "No file name".to_string(),
                })
            }
        }
        "LspInfo" => {
            // Show LSP status information
            let mut info = String::new();

            if editor.lsp_manager().is_none() {
                info.push_str("LSP is not enabled\n");
            } else if editor.active_lsp_servers().is_empty() {
                info.push_str("No active LSP servers\n");
                if !editor.lsp_status().is_empty() {
                    info.push_str(&format!("Status: {}\n", editor.lsp_status()));
                }
            } else {
                info.push_str("Active LSP servers:\n");
                for (lang_id, server_name) in editor.active_lsp_servers() {
                    info.push_str(&format!("  {} -> {}\n", lang_id, server_name));
                }

                let (errors, warnings, info_count, hints) = editor.cached_diagnostic_count();
                info.push_str(&format!("\nDiagnostics: {} errors, {} warnings, {} info, {} hints\n",
                    errors, warnings, info_count, hints));

                if !editor.lsp_status().is_empty() {
                    info.push_str(&format!("Status: {}\n", editor.lsp_status()));
                }

                if let Some(file_path) = editor.buffer().file_path() {
                    info.push_str(&format!("Current file: {}\n", file_path));
                }
            }

            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some(info),
                line_count: None,
            })
        }
        _ => {
            // Handle :w <filename>
            if let Some(filename) = command.strip_prefix("w ").or_else(|| command.strip_prefix("write ")) {
                editor.buffer_mut().set_file_path(filename.to_string());
                match editor.buffer_mut().save_as(filename) {
                    Ok(_) => {
                        editor.mark_saved();
                        editor.mark_buffer_saved(); // Mark for LSP didSave notification
                        let line_count = editor.buffer().rope().len_lines();
                        let char_count = editor.buffer().rope().len_chars();
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("\"{}\" {}L, {}C written", filename, line_count, char_count)),
                            line_count: None,
                        })
                    }
                    Err(e) => ApiResponse::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            // Handle :lua <code>
            } else if let Some(code) = command.strip_prefix("lua ") {
                #[cfg(feature = "lua")]
                {
                    match editor.execute_lua(code) {
                        Ok(result) => ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(result),
                            line_count: None,
                        }),
                        Err(e) => ApiResponse::Error(ErrorResponse {
                            error: format!("Lua error: {}", e),
                        }),
                    }
                }
                #[cfg(not(feature = "lua"))]
                {
                    ApiResponse::Error(ErrorResponse {
                        error: "Lua support not enabled".to_string(),
                    })
                }
            // Handle :luafile <path>
            } else if let Some(path) = command.strip_prefix("luafile ") {
                #[cfg(feature = "lua")]
                {
                    match editor.execute_lua_file(path) {
                        Ok(_) => ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Executed {}", path)),
                            line_count: None,
                        }),
                        Err(e) => ApiResponse::Error(ErrorResponse {
                            error: format!("Lua error: {}", e),
                        }),
                    }
                }
                #[cfg(not(feature = "lua"))]
                {
                    ApiResponse::Error(ErrorResponse {
                        error: "Lua support not enabled".to_string(),
                    })
                }
            // Handle :colorscheme <name> or :colorscheme (to show current)
            // Also support :colo abbreviation
            } else if command == "colorscheme" || command == "colo" {
                let current = editor.current_color_scheme_name();
                let schemes = editor.list_color_schemes().join(", ");
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Current: {}\nAvailable: {}", current, schemes)),
                    line_count: None,
                })
            } else if let Some(scheme_name) = command.strip_prefix("colorscheme ").or_else(|| command.strip_prefix("colo ")) {
                match editor.set_color_scheme(scheme_name.trim()) {
                    Ok(_) => ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("Color scheme set to '{}'", scheme_name.trim())),
                        line_count: None,
                    }),
                    Err(e) => {
                        let available = editor.list_color_schemes().join(", ");
                        ApiResponse::Error(ErrorResponse {
                            error: format!("{}. Available schemes: {}", e, available),
                        })
                    }
                }
            // Handle :set commands
            } else if let Some(set_cmd) = command.strip_prefix("set ").or_else(|| command.strip_prefix("se ")) {
                handle_set_command(editor, set_cmd.trim())
            // Handle split commands
            } else if command == "sp" || command == "split" {
                editor.split_window_horizontal();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Split horizontally ({} windows)", editor.window_count())),
                    line_count: None,
                })
            } else if command == "vsp" || command == "vsplit" {
                editor.split_window_vertical();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Split vertically ({} windows)", editor.window_count())),
                    line_count: None,
                })
            // Handle config reload
            } else if command == "ConfigReload" || command == "reload" {
                #[cfg(feature = "lua")]
                {
                    match editor.reload_lua_config() {
                        Ok(msg) => ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(msg),
                            line_count: None,
                        }),
                        Err(e) => ApiResponse::Error(ErrorResponse {
                            error: format!("Failed to reload config: {}", e),
                        }),
                    }
                }
                #[cfg(not(feature = "lua"))]
                {
                    ApiResponse::Error(ErrorResponse {
                        error: "Lua support not enabled".to_string(),
                    })
                }
            // Handle :bn (next buffer)
            } else if command == "bn" || command == "bnext" {
                editor.next_buffer();
                let buf_name = editor.buffer().file_path()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "[No Name]".to_string());
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Buffer {} of {}: {}",
                        editor.current_buffer_index() + 1,
                        editor.buffer_count(),
                        buf_name)),
                    line_count: None,
                })
            // Handle :bp (previous buffer)
            } else if command == "bp" || command == "bprev" || command == "bprevious" {
                editor.prev_buffer();
                let buf_name = editor.buffer().file_path()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "[No Name]".to_string());
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Buffer {} of {}: {}",
                        editor.current_buffer_index() + 1,
                        editor.buffer_count(),
                        buf_name)),
                    line_count: None,
                })
            // Handle :bd (delete buffer)
            } else if command == "bd" || command == "bdelete" {
                if editor.buffer().is_modified() {
                    ApiResponse::Error(ErrorResponse {
                        error: "No write since last change (add ! to override)".to_string(),
                    })
                } else {
                    let should_quit = editor.delete_current_buffer();
                    if should_quit {
                        editor.quit();
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some("Last buffer deleted, quitting".to_string()),
                            line_count: None,
                        })
                    } else {
                        let buf_name = editor.buffer().file_path()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "[No Name]".to_string());
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Buffer deleted. Now showing: {}", buf_name)),
                            line_count: None,
                        })
                    }
                }
            // Handle :bd! (force delete buffer)
            } else if command == "bd!" || command == "bdelete!" {
                let should_quit = editor.delete_current_buffer();
                if should_quit {
                    editor.quit();
                    ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some("Last buffer deleted, quitting".to_string()),
                        line_count: None,
                    })
                } else {
                    let buf_name = editor.buffer().file_path()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "[No Name]".to_string());
                    ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("Buffer deleted. Now showing: {}", buf_name)),
                        line_count: None,
                    })
                }
            // Handle :ls or :buffers (list buffers)
            } else if command == "ls" || command == "buffers" {
                let buf_list: Vec<String> = editor.buffer_names()
                    .iter()
                    .enumerate()
                    .map(|(i, name)| {
                        let marker = if i == editor.current_buffer_index() { "%" } else { " " };
                        let modified = if i < editor.buffer_count() && editor.buffers[i].is_modified() { "+" } else { " " };
                        format!("{} {}  {}", marker, modified, name)
                    })
                    .collect();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(buf_list.join("\n")),
                    line_count: None,
                })
            } else if let Some(filename) = command.strip_prefix("e ").or_else(|| command.strip_prefix("edit ")) {
                // :e <filename> - edit file
                match editor.load_file(filename) {
                    Ok(_) => {
                        let buf_name = editor.buffer().file_path()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "[No Name]".to_string());
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Editing: {}", buf_name)),
                            line_count: None,
                        })
                    }
                    Err(e) => ApiResponse::Error(ErrorResponse {
                        error: format!("Failed to load file: {}", e),
                    }),
                }
            } else {
                ApiResponse::Error(ErrorResponse {
                    error: format!("Not an editor command: {}", command),
                })
            }
        }
    }
}

/// Handle :set commands for options
fn handle_set_command(editor: &mut Editor, args: &str) -> ApiResponse {
    // Handle empty :set (show all options)
    if args.is_empty() {
        let opts = &editor.options;
        let msg = format!(
            "  {}number\n  {}relativenumber\n  {}expandtab\n  tabstop={}\n  shiftwidth={}\n  scroll={}",
            if opts.number { "" } else { "no" },
            if opts.relative_number { "" } else { "no" },
            if opts.expand_tab { "" } else { "no" },
            opts.tab_width,
            opts.shift_width,
            opts.scroll.map(|s| s.to_string()).unwrap_or_else(|| "auto".to_string())
        );
        return ApiResponse::Success(SuccessResponse {
            success: true,
            message: Some(msg),
            line_count: None,
        });
    }

    // Parse option
    let (opt_name, opt_value) = if let Some((name, value)) = args.split_once('=') {
        (name.trim(), Some(value.trim()))
    } else {
        (args, None)
    };

    // Check for query (option?)
    if let Some(query_opt) = opt_name.strip_suffix('?') {
        let opts = &editor.options;
        let msg = match query_opt {
            "number" | "nu" => format!("  {}number", if opts.number { "" } else { "no" }),
            "relativenumber" | "rnu" => format!("  {}relativenumber", if opts.relative_number { "" } else { "no" }),
            "expandtab" | "et" => format!("  {}expandtab", if opts.expand_tab { "" } else { "no" }),
            "tabstop" | "ts" => format!("  tabstop={}", opts.tab_width),
            "shiftwidth" | "sw" => format!("  shiftwidth={}", opts.shift_width),
            "scroll" => format!("  scroll={}", opts.scroll.map(|s| s.to_string()).unwrap_or_else(|| "auto".to_string())),
            _ => return ApiResponse::Error(ErrorResponse {
                error: format!("Unknown option: {}", query_opt),
            }),
        };
        return ApiResponse::Success(SuccessResponse {
            success: true,
            message: Some(msg),
            line_count: None,
        });
    }

    // Handle boolean options
    match opt_name {
        "number" | "nu" => {
            editor.options.number = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  number".to_string()),
                line_count: None,
            });
        }
        "nonumber" | "nonu" => {
            editor.options.number = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  nonumber".to_string()),
                line_count: None,
            });
        }
        "relativenumber" | "rnu" => {
            editor.options.relative_number = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  relativenumber".to_string()),
                line_count: None,
            });
        }
        "norelativenumber" | "nornu" => {
            editor.options.relative_number = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  norelativenumber".to_string()),
                line_count: None,
            });
        }
        "expandtab" | "et" => {
            editor.options.expand_tab = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  expandtab".to_string()),
                line_count: None,
            });
        }
        "noexpandtab" | "noet" => {
            editor.options.expand_tab = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  noexpandtab".to_string()),
                line_count: None,
            });
        }
        _ => {}
    }

    // Handle value-based options
    if let Some(value) = opt_value {
        match opt_name {
            "tabstop" | "ts" => {
                match value.parse::<usize>() {
                    Ok(n) if n > 0 && n <= 16 => {
                        editor.options.tab_width = n;
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("  tabstop={}", n)),
                            line_count: None,
                        })
                    }
                    Ok(_) => ApiResponse::Error(ErrorResponse {
                        error: "tabstop must be between 1 and 16".to_string(),
                    }),
                    Err(_) => ApiResponse::Error(ErrorResponse {
                        error: format!("Invalid number: {}", value),
                    }),
                }
            }
            "shiftwidth" | "sw" => {
                match value.parse::<usize>() {
                    Ok(n) if n > 0 && n <= 16 => {
                        editor.options.shift_width = n;
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("  shiftwidth={}", n)),
                            line_count: None,
                        })
                    }
                    Ok(_) => ApiResponse::Error(ErrorResponse {
                        error: "shiftwidth must be between 1 and 16".to_string(),
                    }),
                    Err(_) => ApiResponse::Error(ErrorResponse {
                        error: format!("Invalid number: {}", value),
                    }),
                }
            }
            "scroll" => {
                match value.parse::<usize>() {
                    Ok(n) if n > 0 => {
                        editor.options.scroll = Some(n);
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("  scroll={}", n)),
                            line_count: None,
                        })
                    }
                    Ok(_) => ApiResponse::Error(ErrorResponse {
                        error: "scroll must be greater than 0".to_string(),
                    }),
                    Err(_) => ApiResponse::Error(ErrorResponse {
                        error: format!("Invalid number: {}", value),
                    }),
                }
            }
            _ => ApiResponse::Error(ErrorResponse {
                error: format!("Unknown option: {}", opt_name),
            }),
        }
    } else {
        ApiResponse::Error(ErrorResponse {
            error: format!("Unknown option: {}", opt_name),
        })
    }
}

/// Find the root of a JVM project (Maven or Gradle)
/// Searches parent directories for pom.xml, build.gradle, build.gradle.kts, or settings.gradle
fn find_jvm_project_root(file_path: &std::path::Path) -> &std::path::Path {
    let mut current = file_path.parent();
    while let Some(dir) = current {
        // Check for Maven project (pom.xml)
        if dir.join("pom.xml").exists() {
            return dir;
        }
        // Check for Gradle project (build.gradle, build.gradle.kts, or settings.gradle)
        if dir.join("build.gradle").exists()
            || dir.join("build.gradle.kts").exists()
            || dir.join("settings.gradle").exists()
            || dir.join("settings.gradle.kts").exists()
        {
            return dir;
        }
        current = dir.parent();
    }
    // Fall back to file's parent directory if no project root found
    file_path.parent().unwrap_or_else(|| std::path::Path::new("/"))
}

/// Initialize Java LSP with auto-download and configuration
/// Helper to send Java status updates
fn send_java_status(msg: String) {
    if let Some(tx) = JAVA_STATUS_SENDER.get() {
        let _ = tx.send(format!("Java: {}", msg));
    }
}

/// Background Java LSP initialization that doesn't block the UI
async fn initialize_java_lsp_background(
    lsp_manager: Option<std::sync::Arc<tokio::sync::Mutex<lsp::LspManager>>>,
    file_path: std::path::PathBuf,
) {
    crate::lsp_debug!("Java", "Background task started for {:?}", file_path);
    use ovim::java::{JdtlsDownloader, JdtlsLauncher, parser};

    // Early exit if no LSP manager
    let Some(lsp_manager) = lsp_manager else {
        send_java_status("No LSP manager available".to_string());
        return;
    };

    // Find project root
    let project_root = find_jvm_project_root(&file_path);
    crate::lsp_debug!("Java", "Project root: {:?}", project_root);

    send_java_status("Detecting project configuration...".to_string());
    crate::lsp_debug!("Java", "Sent status: Detecting project configuration...");

    // Detect Java version from build files
    let project_config = match parser::detect_java_version(project_root).await {
        Ok(config) => config,
        Err(e) => {
            send_java_status(format!("Failed to detect version: {}", e));
            return;
        }
    };

    send_java_status(format!("Detected Java {} project", project_config.java_version.as_str()));

    // Get jdtls installation directory
    let jdtls_dir = match ovim::java::jdtls_dir().await {
        Ok(dir) => dir,
        Err(e) => {
            send_java_status(format!("Failed to get cache dir: {}", e));
            return;
        }
    };

    // Ensure jdtls is installed
    let downloader = JdtlsDownloader::new(jdtls_dir.clone());

    if !downloader.is_installed().await {
        send_java_status("Downloading jdtls... (first time setup)".to_string());

        match downloader.ensure_installed(|msg| {
            send_java_status(msg);
        }).await {
            Ok(()) => send_java_status("Download complete!".to_string()),
            Err(e) => {
                send_java_status(format!("Download failed: {}", e));
                return;
            }
        }
    } else {
        send_java_status("Using cached jdtls".to_string());
    }

    // Ensure Lombok is installed
    if !downloader.is_lombok_installed().await {
        send_java_status("Downloading Lombok... (first time setup)".to_string());

        match downloader.ensure_lombok_installed(|msg| {
            send_java_status(msg);
        }).await {
            Ok(()) => send_java_status("Lombok download complete!".to_string()),
            Err(e) => {
                send_java_status(format!("Lombok download failed: {}", e));
                // Non-fatal: continue without Lombok
            }
        }
    } else {
        send_java_status("Using cached Lombok".to_string());
    }

    // Get Lombok JAR path (if installed)
    let lombok_jar = if downloader.is_lombok_installed().await {
        Some(downloader.lombok_jar_path())
    } else {
        None
    };

    // Get workspace directory
    let workspace_dir = match ovim::java::workspace_dir(project_root).await {
        Ok(dir) => dir,
        Err(e) => {
            send_java_status(format!("Failed to create workspace: {}", e));
            return;
        }
    };

    send_java_status("Configuring launcher...".to_string());

    // Create launcher
    let launcher = JdtlsLauncher::from_project_config(
        project_config,
        jdtls_dir,
        workspace_dir,
        lombok_jar,
    );

    send_java_status("Finding JVM...".to_string());

    // Get launch command (async JVM detection)
    let launch_args = match launcher.launch_command().await {
        Ok(args) => {
            send_java_status("JVM found, launching jdtls...".to_string());
            args
        }
        Err(e) => {
            send_java_status(format!("Failed to find JVM: {}", e));
            return;
        }
    };

    // Extract java command and args
    if launch_args.is_empty() {
        send_java_status("Invalid launch configuration".to_string());
        return;
    }

    let server_command = &launch_args[0];
    let server_args: Vec<String> = launch_args[1..].to_vec();

    send_java_status("Starting LSP server...".to_string());
    crate::lsp_debug!("Java", "About to spawn start_server task");
    crate::lsp_debug!("Java", "Server command: {:?}", server_command);
    crate::lsp_debug!("Java", "Server args: {:?}", server_args);

    // Start the LSP server with progress updates during initialization
    // jdtls can take 60-120 seconds to initialize, so we send periodic updates
    let lsp_clone = lsp_manager.clone();
    let server_command_clone = server_command.to_string();
    let server_args_clone = server_args.clone();
    let project_root_clone = project_root.to_path_buf();

    let mut start_task = tokio::spawn(async move {
        crate::lsp_debug!("Java", "Inside start_server task, acquiring lock...");
        let lsp = lsp_clone.lock().await;
        crate::lsp_debug!("Java", "Lock acquired, calling start_server...");
        let result = lsp.start_server("java", &server_command_clone, server_args_clone, &project_root_clone).await;
        crate::lsp_debug!("Java", "start_server returned: {:?}", result);
        result
    });

    // Poll for completion with progress updates
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3));
    let mut dots = 1;
    let start_result = loop {
        tokio::select! {
            result = &mut start_task => {
                break result;
            }
            _ = interval.tick() => {
                let dot_str = ".".repeat(dots);
                send_java_status(format!("Starting LSP server{}", dot_str));
                dots = (dots % 3) + 1;
            }
        }
    };

    match start_result {
        Ok(Ok(())) => {
            send_java_status("Server started successfully".to_string());
        }
        Ok(Err(e)) => {
            send_java_status(format!("Failed to start server: {}", e));
            return;
        }
        Err(e) => {
            send_java_status(format!("Server task failed: {}", e));
            return;
        }
    }

    send_java_status("Initializing LSP connection...".to_string());

    // Start notification listener - acquire lock again
    {
        let lsp = lsp_manager.lock().await;
        lsp.start_notification_listener("java".to_string()).await;
    }

    send_java_status("Opening file...".to_string());

    // Send didOpen notification - acquire lock again
    let uri = match lsp_types::Url::from_file_path(&file_path) {
        Ok(uri) => uri,
        Err(_) => {
            send_java_status(format!("Invalid file path: {:?}", file_path));
            return;
        }
    };

    // Read the actual file content (async to avoid blocking)
    let file_content = match tokio::fs::read_to_string(&file_path).await {
        Ok(content) => content,
        Err(e) => {
            send_java_status(format!("Failed to read file: {}", e));
            String::new()
        }
    };

    {
        let lsp = lsp_manager.lock().await;
        match lsp.did_open(uri, "java", 1, file_content).await {
            Ok(_) => {
                send_java_status("Ready ✓".to_string());
            }
            Err(e) => {
                send_java_status(format!("Failed to initialize: {}", e));
            }
        }
    }
}

/// Old version that requires mutable editor (used in headless mode)
async fn initialize_java_lsp(editor: &mut Editor, file_path: &std::path::Path) {
    use ovim::java::{JdtlsDownloader, JdtlsLauncher, parser};

    // Find project root
    let project_root = find_jvm_project_root(file_path);

    editor.set_lsp_status("Java: Detecting project configuration...".to_string());

    // Detect Java version from build files
    let project_config = match parser::detect_java_version(project_root).await {
        Ok(config) => config,
        Err(e) => {
            editor.set_lsp_status(format!("Java: Failed to detect version: {}", e));
            return;
        }
    };

    editor.set_lsp_status(format!(
        "Java: Detected Java {} project",
        project_config.java_version.as_str()
    ));

    // Get jdtls installation directory
    let jdtls_dir = match ovim::java::jdtls_dir().await {
        Ok(dir) => dir,
        Err(e) => {
            editor.set_lsp_status(format!("Java: Failed to get cache dir: {}", e));
            return;
        }
    };

    // Ensure jdtls is installed
    let downloader = JdtlsDownloader::new(jdtls_dir.clone());

    if !downloader.is_installed().await {
        editor.set_lsp_status("Java: Downloading jdtls... (first time setup)".to_string());

        // Create a channel for async progress updates
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn download task
        let mut download_task = tokio::spawn(async move {
            downloader.ensure_installed(move |msg| {
                let _ = progress_tx.send(msg);
            }).await
        });

        // Poll for progress updates without blocking
        loop {
            tokio::select! {
                Some(msg) = progress_rx.recv() => {
                    editor.set_lsp_status(format!("Java: {}", msg));
                }
                result = &mut download_task => {
                    match result {
                        Ok(Ok(())) => {
                            editor.set_lsp_status("Java: Download complete!".to_string());
                            break;
                        }
                        Ok(Err(e)) => {
                            editor.set_lsp_status(format!("Java: Download failed: {}", e));
                            return;
                        }
                        Err(e) => {
                            editor.set_lsp_status(format!("Java: Download task failed: {}", e));
                            return;
                        }
                    }
                }
            }
        }
    } else {
        editor.set_lsp_status("Java: Using cached jdtls".to_string());
    }

    // Ensure Lombok is installed
    let lombok_downloader = JdtlsDownloader::new(jdtls_dir.clone());
    if !lombok_downloader.is_lombok_installed().await {
        editor.set_lsp_status("Java: Downloading Lombok... (first time setup)".to_string());

        // Create a channel for async progress updates
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn download task
        let mut download_task = tokio::spawn(async move {
            lombok_downloader.ensure_lombok_installed(move |msg| {
                let _ = progress_tx.send(msg);
            }).await
        });

        // Poll for progress updates without blocking
        loop {
            tokio::select! {
                Some(msg) = progress_rx.recv() => {
                    editor.set_lsp_status(format!("Java: {}", msg));
                }
                result = &mut download_task => {
                    match result {
                        Ok(Ok(())) => {
                            editor.set_lsp_status("Java: Lombok download complete!".to_string());
                            break;
                        }
                        Ok(Err(e)) => {
                            editor.set_lsp_status(format!("Java: Lombok download failed: {}", e));
                            // Non-fatal: continue without Lombok
                            break;
                        }
                        Err(e) => {
                            editor.set_lsp_status(format!("Java: Lombok download task failed: {}", e));
                            // Non-fatal: continue without Lombok
                            break;
                        }
                    }
                }
            }
        }
    } else {
        editor.set_lsp_status("Java: Using cached Lombok".to_string());
    }

    // Get Lombok JAR path (if installed)
    let lombok_downloader2 = JdtlsDownloader::new(jdtls_dir.clone());
    let lombok_jar = if lombok_downloader2.is_lombok_installed().await {
        Some(lombok_downloader2.lombok_jar_path())
    } else {
        None
    };

    // Get workspace directory
    let workspace_dir = match ovim::java::workspace_dir(project_root).await {
        Ok(dir) => dir,
        Err(e) => {
            editor.set_lsp_status(format!("Java: Failed to create workspace: {}", e));
            return;
        }
    };

    editor.set_lsp_status("Java: Configuring launcher...".to_string());

    // Create launcher
    let launcher = JdtlsLauncher::from_project_config(
        project_config,
        jdtls_dir,
        workspace_dir,
        lombok_jar,
    );

    editor.set_lsp_status("Java: Finding JVM...".to_string());

    // Get launch command (async JVM detection)
    let launch_args = match launcher.launch_command().await {
        Ok(args) => {
            editor.set_lsp_status("Java: JVM found, launching jdtls...".to_string());
            args
        }
        Err(e) => {
            editor.set_lsp_status(format!("Java: Failed to find JVM: {}", e));
            return;
        }
    };

    // Start LSP server using the launch args
    if let Some(lsp_manager) = editor.lsp_manager() {
        let lsp = lsp_manager.lock().await;

        // Extract java command and args
        if launch_args.is_empty() {
            editor.set_lsp_status("Java: Invalid launch configuration".to_string());
            return;
        }

        let server_command = &launch_args[0];
        let server_args: Vec<String> = launch_args[1..].to_vec();

        editor.set_lsp_status("Java: Starting LSP server...".to_string());

        match lsp.start_server("java", server_command, server_args, project_root).await {
            Ok(_) => {
                drop(lsp);
                editor.register_lsp_server("java".to_string(), "jdtls".to_string());

                editor.set_lsp_status("Java: Initializing LSP connection...".to_string());

                let lsp = lsp_manager.lock().await;
                lsp.start_notification_listener("java".to_string()).await;

                editor.set_lsp_status("Java: Opening file...".to_string());

                // Send didOpen notification
                let file_content = editor.buffer().rope().to_string();
                let uri = match lsp_types::Url::from_file_path(file_path) {
                    Ok(uri) => uri,
                    Err(_) => {
                        drop(lsp);
                        editor.set_lsp_status("Java: Invalid file path".to_string());
                        return;
                    }
                };

                match lsp.did_open(uri, "java", 1, file_content.clone()).await {
                    Ok(_) => {
                        drop(lsp);
                        // CRITICAL FIX: Initialize last_synced_content after successful didOpen
                        // Without this, the first didChange uses empty string as old_text,
                        // breaking incremental sync
                        editor.set_last_synced_content(Some(file_content));
                        editor.set_lsp_status("Java: Ready ✓".to_string());
                    }
                    Err(e) => {
                        drop(lsp);
                        editor.set_lsp_status(format!("Java: Failed to initialize: {}", e));
                    }
                }
            }
            Err(e) => {
                drop(lsp);
                editor.set_lsp_status(format!("Java: Failed to start server: {}", e));
            }
        }
    }
}

/// Initialize LSP for a file based on its extension
async fn initialize_lsp_for_file(editor: &mut Editor, file_path: &str) {
    use std::path::Path;
    use ovim::java::{JdtlsDownloader, JdtlsLauncher};

    let path = Path::new(file_path);

    // Convert to absolute path first
    let abs_path = if path.is_absolute() {
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

    let extension = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Handle Java specially with auto-setup (spawn in background to avoid blocking UI)
    if extension == "java" {
        // We need to move values into the spawned task, so clone what we need
        let abs_path_clone = abs_path.clone();
        let lsp_manager = editor.lsp_manager().map(|arc| arc.clone());

        // Spawn Java LSP initialization in background
        tokio::spawn(async move {
            initialize_java_lsp_background(lsp_manager, abs_path_clone).await;
        });

        // Initial status will be updated immediately by the background task
        return;
    }

    // Determine language and LSP server based on file extension
    let (language_id, server_command, server_args) = match extension {
        "rs" => ("rust", "rust-analyzer", vec![]),
        "js" | "ts" | "jsx" | "tsx" => ("javascript", "typescript-language-server", vec!["--stdio".to_string()]),
        "py" => ("python", "pylsp", vec![]),
        _ => return, // No LSP support for this file type
    };

    // Find the project root based on language
    let root_path = match extension {
        "rs" => {
            // Look for Cargo.toml in parent directories for Rust
            let mut current = abs_path.parent();
            while let Some(dir) = current {
                let cargo_toml = dir.join("Cargo.toml");
                if cargo_toml.exists() {
                    break;
                }
                current = dir.parent();
            }
            current.unwrap_or_else(|| abs_path.parent().unwrap_or_else(|| Path::new("/")))
        }
        _ => abs_path.parent().unwrap_or_else(|| Path::new("/")),
    };

    // Start the language server
    if let Some(lsp_manager) = editor.lsp_manager() {
        let lsp = lsp_manager.lock().await;

        // Start the server (will skip if already running)
        match lsp.start_server(language_id, server_command, server_args, root_path).await {
            Ok(_) => {
                drop(lsp); // Release lock before calling editor methods
                editor.register_lsp_server(language_id.to_string(), server_command.to_string());

                // Re-acquire lock for remaining operations
                let lsp = lsp_manager.lock().await;

                // Start notification listener to receive diagnostics
                lsp.start_notification_listener(language_id.to_string()).await;

                // Send didOpen notification
                let file_content = editor.buffer().rope().to_string();
                let uri = match lsp_types::Url::from_file_path(&abs_path) {
                    Ok(uri) => uri,
                    Err(_) => {
                        drop(lsp);
                        editor.set_lsp_status("LSP: Invalid file path".to_string());
                        return;
                    }
                };

                match lsp.did_open(uri, language_id, 1, file_content.clone()).await {
                    Ok(_) => {
                        drop(lsp);
                        // CRITICAL FIX: Initialize last_synced_content after successful didOpen
                        // Without this, the first didChange uses empty string as old_text,
                        // breaking incremental sync
                        editor.set_last_synced_content(Some(file_content));
                        editor.set_lsp_status(format!("LSP: {} ready", server_command));
                    }
                    Err(e) => {
                        drop(lsp);
                        editor.set_lsp_status(format!("LSP: didOpen failed: {}", e));
                    }
                }
            }
            Err(e) => {
                drop(lsp);
                editor.set_lsp_status(format!("LSP: Failed to start {}: {}", server_command, e));
                crate::lsp_warn!("LSP", "Failed to start server '{}': {}", server_command, e);
            }
        }
    }
}
