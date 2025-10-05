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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse_args();

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
        run_headless_loop(&mut editor, rx).await?;
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
    run_event_loop(&mut ui, &mut editor, api_rx).await?;

    Ok(())
}

async fn run_headless_loop(
    editor: &mut Editor,
    mut api_rx: mpsc::UnboundedReceiver<ApiRequest>,
) -> Result<()> {
    use tokio::time::{Duration, sleep};

    loop {
        // Process LSP notifications (diagnostics, etc.)
        if let Some(lsp_manager) = editor.lsp_manager() {
            let lsp = lsp_manager.lock().await;
            lsp.process_notifications().await;
        }

        // Process any pending LSP actions
        editor.process_pending_lsp_actions().await;

        // Update diagnostic cache
        editor.update_diagnostic_cache().await;

        // Check for API requests (non-blocking with timeout)
        match tokio::time::timeout(Duration::from_millis(50), api_rx.recv()).await {
            Ok(Some(request)) => {
                handle_api_request(editor, request);
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
) -> Result<()> {
    use tokio::time::{Duration, Instant};

    let mut last_edit = Instant::now();
    let debounce_delay = Duration::from_millis(100);

    while !editor.should_quit() {
        // Process LSP notifications (diagnostics, etc.)
        if let Some(lsp_manager) = editor.lsp_manager() {
            let lsp = lsp_manager.lock().await;
            lsp.process_notifications().await;
        }

        // Update diagnostic cache for UI display
        editor.update_diagnostic_cache().await;

        // Check if enough time has passed since last edit for re-highlighting
        if editor.buffer().needs_rehighlight() && last_edit.elapsed() >= debounce_delay {
            editor.process_pending_rehighlight().await;
        }

        // Process any pending LSP actions
        editor.process_pending_lsp_actions().await;

        // Render the editor
        ui.renderer_mut().render(editor)?;

        // Check for API requests (non-blocking)
        if let Some(ref mut rx) = api_rx {
            while let Ok(request) = rx.try_recv() {
                handle_api_request(editor, request);
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
    }

    Ok(())
}

fn handle_api_request(editor: &mut Editor, request: ApiRequest) {
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
            } else {
                ApiResponse::Error(ErrorResponse {
                    error: format!("Not an editor command: {}", command),
                })
            }
        }
    }
}

/// Initialize LSP for a file based on its extension
async fn initialize_lsp_for_file(editor: &mut Editor, file_path: &str) {
    use std::path::Path;

    let path = Path::new(file_path);
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Determine language and LSP server based on file extension
    let (language_id, server_command, server_args) = match extension {
        "rs" => ("rust", "rust-analyzer", vec![]),
        "js" | "ts" | "jsx" | "tsx" => ("javascript", "typescript-language-server", vec!["--stdio".to_string()]),
        "py" => ("python", "pylsp", vec![]),
        _ => return, // No LSP support for this file type
    };

    // Find the project root (look for Cargo.toml for Rust projects)
    let root_path = if extension == "rs" {
        // Look for Cargo.toml in parent directories
        let mut current = path.parent();
        while let Some(dir) = current {
            let cargo_toml = dir.join("Cargo.toml");
            if cargo_toml.exists() {
                break;
            }
            current = dir.parent();
        }
        current.unwrap_or_else(|| path.parent().unwrap_or_else(|| Path::new(".")))
    } else {
        path.parent().unwrap_or_else(|| Path::new("."))
    };

    // Start the language server
    if let Some(lsp_manager) = editor.lsp_manager() {
        let lsp = lsp_manager.lock().await;

        // Start the server (will skip if already running)
        if let Err(_e) = lsp.start_server(language_id, server_command, server_args, root_path).await {
            return;
        }

        // Start notification listener to receive diagnostics
        lsp.start_notification_listener(language_id.to_string()).await;

        // Send didOpen notification
        let file_content = editor.buffer().rope().to_string();
        let uri = match lsp_types::Url::from_file_path(path) {
            Ok(uri) => uri,
            Err(_) => {
                return;
            }
        };

        let _ = lsp.did_open(uri, language_id, 1, file_content).await;
    }
}
