use anyhow::Result;
use crossterm::event::Event;
use ovim::editor::{Editor, InputHandler};
use ovim::ui::UI;
use ovim::cli::Args;
use ovim::api::{ApiRequest, ApiResponse, BufferInfo, CursorPosition, EditorSnapshot, ErrorResponse, HealthInfo, LspServerInfoItem, LspStatusInfo, ModeInfo, PickerInfo, PickerResultInfo, RenderInfo, SuccessResponse, VisualSelection, parse_key_string};
use ovim::session::SessionInfo;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;
use tokio::sync::mpsc;

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
        let session_name = args.session.clone().unwrap_or_else(|| "default".to_string());
        let session_info = SessionInfo::new(
            port,
            args.file.clone(),
            session_name.clone(),
        );

        if let Err(e) = session_info.write() {
            eprintln!("Warning: Failed to write session info: {}", e);
        } else {
            eprintln!("Session '{}' created at ~/.cache/ovim/sessions/{}.json", session_name, session_name);
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
        run_headless_loop(&mut editor, rx, java_status_rx, start_time, session_info_arc).await?;
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
    run_event_loop(&mut ui, &mut editor, api_rx, java_status_rx).await?;

    Ok(())
}

async fn run_headless_loop(
    editor: &mut Editor,
    mut api_rx: mpsc::UnboundedReceiver<ApiRequest>,
    mut java_status_rx: mpsc::UnboundedReceiver<String>,
    start_time: SystemTime,
    session_info: Arc<Mutex<SessionInfo>>,
) -> Result<()> {
    use tokio::time::{Duration, sleep};

    // Create channel for async preview loading
    let (preview_tx, mut preview_rx) = tokio::sync::mpsc::channel::<(String, ovim::editor::PreviewCache)>(100);

    // Create channel for async file loading
    let (file_tx, mut file_rx) = tokio::sync::mpsc::channel::<ovim::editor::PickerResult>(1000);

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

        // Initialize syntax highlighting lazily (after file is displayed)
        if editor.buffer().should_init_syntax() {
            editor.buffer_mut().enable_syntax_highlighting();
        }

        // Process any pending LSP actions
        editor.process_pending_lsp_actions().await;

        // Process any pending Lua commands
        #[cfg(feature = "lua")]
        let _ = editor.process_lua_commands();

        // Spawn async preview loading if needed (non-blocking!)
        if editor.mode() == ovim::mode::Mode::Picker {
            spawn_picker_preview_loading(editor, &preview_tx);
            spawn_file_finder_loading(editor, &file_tx);
        }

        // Poll for completed previews (non-blocking)
        while let Ok((file_path, cache)) = preview_rx.try_recv() {
            editor.insert_preview(file_path, cache);
        }

        // Poll for file results (non-blocking)
        while let Ok(result) = file_rx.try_recv() {
            if let Some(picker) = editor.picker_mut() {
                picker.add_file_result(result);
            }
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

        // Check for API requests (non-blocking with timeout)
        match tokio::time::timeout(Duration::from_millis(50), api_rx.recv()).await {
            Ok(Some(request)) => {
                handle_api_request(editor, request, start_time, &session_info).await;
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

/// Spawns a background task to load picker preview if debounce time has elapsed
/// Returns immediately without blocking input handling
fn spawn_picker_preview_loading(
    editor: &mut Editor,
    preview_tx: &tokio::sync::mpsc::Sender<(String, ovim::editor::PreviewCache)>,
) {
    if !editor.should_load_picker_preview(200) {
        return;
    }

    // Get the file to load (returns None if already cached/loading)
    if let Some(file_path) = editor.get_preview_to_load() {
        let tx = preview_tx.clone();

        // Spawn background task - doesn't block!
        tokio::spawn(async move {
            // Load preview asynchronously
            if let Some(cache) = load_preview_async(&file_path).await {
                // Send result back (non-blocking)
                let _ = tx.send((file_path, cache)).await;
            }
        });
    }
}

/// Spawns a background task to load files for file finder picker
/// Returns immediately without blocking - files are sent via channel as they're discovered
fn spawn_file_finder_loading(
    editor: &mut Editor,
    file_tx: &tokio::sync::mpsc::Sender<ovim::editor::PickerResult>,
) {
    // Check if we should spawn file loading
    if let Some(picker) = editor.picker() {
        if !picker.should_spawn_file_loading() {
            return;
        }

        // Get the base directory for file search
        let base_dir = picker.base_dir().to_path_buf();

        // Mark as spawned to avoid spawning multiple tasks
        if let Some(picker_mut) = editor.picker_mut() {
            picker_mut.mark_loading_spawned();
        }

        let tx = file_tx.clone();

        // Spawn background task - doesn't block!
        tokio::spawn(async move {
            use ignore::WalkBuilder;

            // Use ignore crate's WalkBuilder which respects .gitignore
            let walker = WalkBuilder::new(&base_dir)
                .hidden(false)  // Don't automatically skip hidden files
                .git_ignore(true)  // Respect .gitignore files
                .git_global(true)  // Respect global gitignore
                .git_exclude(true)  // Respect .git/info/exclude
                .build();

            // Walk the directory tree and send files as we find them
            for entry in walker.filter_map(|e| e.ok()) {
                let path = entry.path();

                if path.is_file() {
                    if let Ok(relative_path) = path.strip_prefix(&base_dir) {
                        let display_path = relative_path.to_string_lossy().to_string();
                        let result = ovim::editor::PickerResult {
                            display: display_path,
                            location: path.to_string_lossy().to_string(),
                            line: 0,
                            col: 0,
                        };

                        // Send result back (non-blocking)
                        // If channel is closed (picker was closed), task will exit
                        if tx.send(result).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
}

/// Loads a file preview asynchronously (can be called from background task)
async fn load_preview_async(file_path: &str) -> Option<ovim::editor::PreviewCache> {
    use std::collections::HashMap;

    // Check file size before loading (max 1MB for preview)
    const MAX_PREVIEW_SIZE: u64 = 1024 * 1024;
    if let Ok(metadata) = tokio::fs::metadata(file_path).await {
        if metadata.len() > MAX_PREVIEW_SIZE {
            // File too large, create a placeholder
            return Some(ovim::editor::PreviewCache {
                content: format!("File too large for preview ({} bytes)", metadata.len()),
                highlighted_lines: std::cell::RefCell::new(HashMap::new()),
                language: None,
            });
        }
    }

    // Load the file
    let content = tokio::fs::read_to_string(file_path).await.ok()?;

    // Detect language
    let language = ovim::syntax::LanguageRegistry::detect_from_path(file_path);

    // Create cache entry
    Some(ovim::editor::PreviewCache {
        content,
        highlighted_lines: std::cell::RefCell::new(HashMap::new()),
        language,
    })
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

    // Create channel for async preview loading
    let (preview_tx, mut preview_rx) = tokio::sync::mpsc::channel::<(String, ovim::editor::PreviewCache)>(100);

    // Create channel for async file loading
    let (file_tx, mut file_rx) = tokio::sync::mpsc::channel::<ovim::editor::PickerResult>(1000);

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

        // Initialize syntax highlighting lazily (after file is displayed)
        if editor.buffer().should_init_syntax() {
            editor.buffer_mut().enable_syntax_highlighting();
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

        // Spawn async preview loading if needed (non-blocking!)
        if editor.mode() == ovim::mode::Mode::Picker {
            spawn_picker_preview_loading(editor, &preview_tx);
            spawn_file_finder_loading(editor, &file_tx);
        }

        // Poll for completed previews (non-blocking)
        while let Ok((file_path, cache)) = preview_rx.try_recv() {
            editor.insert_preview(file_path, cache);
        }

        // Poll for file results (non-blocking)
        while let Ok(result) = file_rx.try_recv() {
            if let Some(picker) = editor.picker_mut() {
                picker.add_file_result(result);
            }
        }

        // Render the editor
        ui.renderer_mut().render(editor)?;

        // Check for API requests (non-blocking)
        if let Some(ref mut rx) = api_rx {
            while let Ok(request) = rx.try_recv() {
                // For TUI mode, use dummy start time and session info since /health isn't typically used
                let dummy_start = SystemTime::now();
                let dummy_session = Arc::new(Mutex::new(SessionInfo::new(0, None, "tui".to_string())));
                handle_api_request(editor, request, dummy_start, &dummy_session).await;
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

async fn handle_api_request(
    editor: &mut Editor,
    request: ApiRequest,
    start_time: SystemTime,
    session_info: &Arc<Mutex<SessionInfo>>,
) {
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
            let response = ovim::commands::execute_command(editor, &command);
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
        ApiRequest::GetLspStatus(tx) => {
            // Get LSP status from the editor's LSP manager
            if let Some(lsp_manager_arc) = editor.lsp_manager() {
                let servers = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        let lsp_manager = lsp_manager_arc.lock().await;
                        lsp_manager.get_lsp_status().await
                    })
                });

                let lsp_status_info = LspStatusInfo {
                    servers: servers.into_iter().map(|s| LspServerInfoItem {
                        language: s.language,
                        command: s.command,
                        state: s.state,
                        pending_requests: s.pending_requests,
                        has_capabilities: s.has_capabilities,
                    }).collect(),
                };

                let _ = tx.send(ApiResponse::LspStatus(lsp_status_info));
            } else {
                // No LSP manager available
                let lsp_status_info = LspStatusInfo {
                    servers: vec![],
                };
                let _ = tx.send(ApiResponse::LspStatus(lsp_status_info));
            }
        }
        ApiRequest::GetHealth(tx) => {
            // Calculate uptime
            let uptime = start_time
                .elapsed()
                .unwrap_or_default()
                .as_secs();

            // Get file being edited
            let file = editor.buffer().file_path().map(|p| p.to_string());

            // Get LSP server statuses
            let mut lsp_servers = HashMap::new();
            if let Some(lsp_manager_arc) = editor.lsp_manager() {
                if let Ok(lsp_manager) = lsp_manager_arc.try_lock() {
                    let servers = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            lsp_manager.get_lsp_status().await
                        })
                    });

                    for server in servers {
                        let state = if server.has_capabilities {
                            "ready"
                        } else if server.state.contains("Initializing") {
                            "initializing"
                        } else {
                            "unknown"
                        };
                        lsp_servers.insert(server.language, state.to_string());
                    }
                }
            }

            // Determine if the system is ready
            let ready = lsp_servers.values().all(|s| s == "ready") || lsp_servers.is_empty();

            // Update session info with LSP ready status
            if let Ok(mut session) = session_info.lock() {
                let _ = session.set_lsp_ready(ready);
            }

            let health_info = HealthInfo {
                status: "healthy".to_string(),
                uptime_seconds: uptime,
                file,
                lsp_servers,
                ready,
            };

            let _ = tx.send(ApiResponse::Health(health_info));
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
    let mut marks = HashMap::new();
    let mark_manager = editor.marks();
    for (name, mark) in mark_manager.iter() {
        marks.insert(name.to_string(), CursorPosition {
            line: mark.line,
            column: mark.col,
        });
    }

    // Get picker state if in picker mode
    let picker = editor.picker().map(|p| {
        PickerInfo {
            mode: match p.mode() {
                ovim::editor::PickerMode::FindFiles => "FindFiles".to_string(),
                ovim::editor::PickerMode::LiveGrep => "LiveGrep".to_string(),
                ovim::editor::PickerMode::Custom => "Custom".to_string(),
                ovim::editor::PickerMode::Completion => "Completion".to_string(),
                ovim::editor::PickerMode::LspLocations => "LspLocations".to_string(),
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
        hover_info: editor.hover_info().map(|s| s.to_string()),
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
    lsp_manager: Option<std::sync::Arc<tokio::sync::Mutex<ovim::lsp::LspManager>>>,
    file_path: std::path::PathBuf,
) {
    ovim::lsp_debug!("Java", "Background task started for {:?}", file_path);
    use ovim::java::{JdtlsDownloader, JdtlsLauncher, parser};

    // Early exit if no LSP manager
    let Some(lsp_manager) = lsp_manager else {
        send_java_status("No LSP manager available".to_string());
        return;
    };

    // Find project root
    let project_root = find_jvm_project_root(&file_path);
    ovim::lsp_debug!("Java", "Project root: {:?}", project_root);

    send_java_status("Detecting project configuration...".to_string());
    ovim::lsp_debug!("Java", "Sent status: Detecting project configuration...");

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
    ovim::lsp_debug!("Java", "About to spawn start_server task");
    ovim::lsp_debug!("Java", "Server command: {:?}", server_command);
    ovim::lsp_debug!("Java", "Server args: {:?}", server_args);

    // Start the LSP server with progress updates during initialization
    // jdtls can take 60-120 seconds to initialize, so we send periodic updates
    let lsp_clone = lsp_manager.clone();
    let server_command_clone = server_command.to_string();
    let server_args_clone = server_args.clone();
    let project_root_clone = project_root.to_path_buf();

    let mut start_task = tokio::spawn(async move {
        ovim::lsp_debug!("Java", "Inside start_server task, acquiring lock...");
        let lsp = lsp_clone.lock().await;
        ovim::lsp_debug!("Java", "Lock acquired, calling start_server...");
        let result = lsp.start_server("java", &server_command_clone, server_args_clone, &project_root_clone).await;
        ovim::lsp_debug!("Java", "start_server returned: {:?}", result);
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
                ovim::lsp_warn!("LSP", "Failed to start server '{}': {}", server_command, e);
            }
        }
    }
}
