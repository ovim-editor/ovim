use anyhow::Result;
use crossterm::event::Event;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration, Instant};

use ovim::api::{
    parse_key_string, ApiRequest, ApiResponse, BufferInfo, CursorPosition, EditorSnapshot,
    ErrorResponse, HealthInfo, LspServerInfoItem, LspStatusInfo, ModeInfo, PickerInfo,
    PickerResultInfo, RenderInfo, SuccessResponse, VisualSelection,
};
use ovim::commands;
use ovim::editor::{self, Editor, InputHandler};
use ovim::mode::Mode;
use ovim::session::SessionInfo;
use ovim::syntax::LanguageRegistry;
use ovim::ui::UI;

/// Runs the headless event loop (API only, no TUI)
pub async fn run_headless_loop(
    editor: &mut Editor,
    mut api_rx: mpsc::UnboundedReceiver<ApiRequest>,
    mut java_status_rx: mpsc::UnboundedReceiver<String>,
    start_time: SystemTime,
    session_info: Arc<Mutex<SessionInfo>>,
) -> Result<()> {
    // Create channel for async preview loading
    let (preview_tx, mut preview_rx) =
        tokio::sync::mpsc::channel::<(String, editor::PreviewCache)>(100);

    // Create channel for async file loading
    let (file_tx, mut file_rx) = tokio::sync::mpsc::channel::<editor::PickerResult>(1000);

    // Periodic interval for LSP processing (100ms - 10 times per second instead of 100)
    let mut lsp_interval = interval(Duration::from_millis(100));
    lsp_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Event-driven loop using tokio::select! - blocks until an event occurs
        tokio::select! {
            // API request received
            Some(request) = api_rx.recv() => {
                handle_api_request(editor, request, start_time, &session_info).await;
                // Check if quit was requested
                if editor.should_quit() {
                    break;
                }
            }

            // Java status update received
            Some(status) = java_status_rx.recv() => {
                editor.set_lsp_status(status);
            }

            // Completed preview received
            Some((file_path, cache)) = preview_rx.recv() => {
                editor.insert_preview(file_path, cache);
            }

            // File result received
            Some(result) = file_rx.recv() => {
                if let Some(picker) = editor.picker_mut() {
                    picker.add_file_result(result);
                }
            }

            // Periodic LSP processing tick
            _ = lsp_interval.tick() => {
                // Process LSP notifications (diagnostics, etc.)
                if let Some(lsp_manager) = editor.lsp_manager() {
                    lsp_manager.process_notifications().await;
                    lsp_manager.process_flush_requests().await;
                }

                // Initialize LSP for newly loaded files
                if let Some(file_path) = editor.needs_lsp_init() {
                    crate::lsp_init::initialize_lsp_for_file(editor, &file_path).await;
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
                if editor.mode() == Mode::Picker {
                    spawn_picker_preview_loading(editor, &preview_tx);
                    spawn_file_finder_loading(editor, &file_tx);
                }

                // Update diagnostic cache only if diagnostics changed
                if let Some(lsp_manager) = editor.lsp_manager() {
                    if lsp_manager.diagnostics_changed() {
                        editor.update_diagnostic_cache().await;
                    }
                }

                // Send LSP notifications if needed
                editor.send_lsp_changes_if_modified().await;
                editor.send_lsp_save_if_needed().await;
            }
        }
    }

    Ok(())
}

/// Runs the main TUI event loop
pub async fn run_event_loop(
    ui: &mut UI,
    editor: &mut Editor,
    mut api_rx: Option<mpsc::UnboundedReceiver<ApiRequest>>,
    mut java_status_rx: mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    let mut last_edit = Instant::now();
    let debounce_delay = Duration::from_millis(100);
    let mut last_input_time: Option<Instant> = None;

    // Create channel for async preview loading
    let (preview_tx, mut preview_rx) =
        tokio::sync::mpsc::channel::<(String, editor::PreviewCache)>(100);

    // Create channel for async file loading
    let (file_tx, mut file_rx) = tokio::sync::mpsc::channel::<editor::PickerResult>(1000);

    while !editor.should_quit() {
        // Check for Java LSP status updates
        while let Ok(status) = java_status_rx.try_recv() {
            editor.set_lsp_status(status);
        }

        // Process LSP notifications (diagnostics, etc.)
        if let Some(lsp_manager) = editor.lsp_manager() {
            lsp_manager.process_notifications().await;
            lsp_manager.process_flush_requests().await;
        }

        // Initialize LSP for newly loaded files
        if let Some(file_path) = editor.needs_lsp_init() {
            crate::lsp_init::initialize_lsp_for_file(editor, &file_path).await;
            editor.clear_lsp_init_flag();
        }

        // Update diagnostic cache only if diagnostics changed
        if let Some(lsp_manager) = editor.lsp_manager() {
            if lsp_manager.diagnostics_changed() {
                editor.update_diagnostic_cache().await;
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
        if editor.mode() == Mode::Picker {
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
                editor.mark_dirty(); // Picker results changed
            }
        }

        // Only render if the editor state has changed (dirty flag)
        if editor.is_dirty() {
            let start = std::time::Instant::now();
            ui.renderer_mut().render(editor)?;
            let duration = start.elapsed();

            editor.record_render_duration(duration.as_micros() as u64);
            editor.increment_render_count();
            editor.mark_clean();

            // Record input latency if we have a pending input time
            if let Some(input_time) = last_input_time.take() {
                let latency = input_time.elapsed().as_micros() as u64;
                editor.record_input_latency(latency);
            }
        }

        // Check for API requests (non-blocking)
        if let Some(ref mut rx) = api_rx {
            while let Ok(request) = rx.try_recv() {
                // For TUI mode, use dummy start time and session info since /health isn't typically used
                let dummy_start = SystemTime::now();
                let dummy_session =
                    Arc::new(Mutex::new(SessionInfo::new(0, None, "tui".to_string())));
                handle_api_request(editor, request, dummy_start, &dummy_session).await;
            }
        }

        // Poll for events with a timeout to allow checking API requests
        if let Some(event) = InputHandler::poll_event()? {
            if let Event::Key(key_event) = event {
                // Record input time for latency tracking
                last_input_time = Some(Instant::now());

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

/// Spawns a background task to load picker preview if debounce time has elapsed
/// Returns immediately without blocking input handling
fn spawn_picker_preview_loading(
    editor: &mut Editor,
    preview_tx: &tokio::sync::mpsc::Sender<(String, editor::PreviewCache)>,
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
    file_tx: &tokio::sync::mpsc::Sender<editor::PickerResult>,
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
                .hidden(false) // Don't automatically skip hidden files
                .git_ignore(true) // Respect .gitignore files
                .git_global(true) // Respect global gitignore
                .git_exclude(true) // Respect .git/info/exclude
                .build();

            // Walk the directory tree and send files as we find them
            for entry in walker.filter_map(|e| e.ok()) {
                let path = entry.path();

                if path.is_file() {
                    if let Ok(relative_path) = path.strip_prefix(&base_dir) {
                        let display_path = relative_path.to_string_lossy().to_string();
                        let result = editor::PickerResult {
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
async fn load_preview_async(file_path: &str) -> Option<editor::PreviewCache> {
    // Check file size before loading (max 1MB for preview)
    const MAX_PREVIEW_SIZE: u64 = 1024 * 1024;
    if let Ok(metadata) = tokio::fs::metadata(file_path).await {
        if metadata.len() > MAX_PREVIEW_SIZE {
            // File too large, create a placeholder
            return Some(editor::PreviewCache {
                content: format!("File too large for preview ({} bytes)", metadata.len()),
                highlighted_lines: std::cell::RefCell::new(HashMap::new()),
                language: None,
            });
        }
    }

    // Load the file
    let content = tokio::fs::read_to_string(file_path).await.ok()?;

    // Detect language
    let language = LanguageRegistry::detect_from_path(file_path);

    // Create cache entry
    Some(editor::PreviewCache {
        content,
        highlighted_lines: std::cell::RefCell::new(HashMap::new()),
        language,
    })
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
            let events_result = parse_key_string(&keys);
            let response = match events_result {
                Ok(events) => {
                    let mut success = true;

                    for event in events {
                        if let Err(_) = InputHandler::handle_key_event(editor, event) {
                            success = false;
                            break;
                        }
                    }

                    // Process any LSP actions that were triggered by the keys
                    editor.process_pending_lsp_actions().await;

                    if success {
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: None,
                            line_count: None,
                        })
                    } else {
                        ApiResponse::Error(ErrorResponse {
                            error: "Failed to process keys".to_string(),
                        })
                    }
                }
                Err(parse_error) => ApiResponse::Error(ErrorResponse {
                    error: format!("Failed to parse keys: {}", parse_error),
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
            let response = commands::execute_command(editor, &command);
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
                        lsp_manager_arc.get_lsp_status().await
                    })
                });

                let lsp_status_info = LspStatusInfo {
                    servers: servers
                        .into_iter()
                        .map(|s| LspServerInfoItem {
                            language: s.language,
                            command: s.command,
                            state: s.state,
                            pending_requests: s.pending_requests,
                            has_capabilities: s.has_capabilities,
                        })
                        .collect(),
                    progress: editor.lsp_progress_message(),
                };

                let _ = tx.send(ApiResponse::LspStatus(lsp_status_info));
            } else {
                // No LSP manager available
                let lsp_status_info = LspStatusInfo {
                    servers: vec![],
                    progress: editor.lsp_progress_message(),
                };
                let _ = tx.send(ApiResponse::LspStatus(lsp_status_info));
            }
        }
        ApiRequest::GetHealth(tx) => {
            // Calculate uptime
            let uptime = start_time.elapsed().unwrap_or_default().as_secs();

            // Get file being edited
            let file = editor.buffer().file_path().map(|p| p.to_string());

            // Get LSP server statuses
            let mut lsp_servers = HashMap::new();
            if let Some(lsp_manager_arc) = editor.lsp_manager() {
                let lsp_manager_arc = lsp_manager_arc.clone();
                let servers = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        lsp_manager_arc.get_lsp_status().await
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
        ApiRequest::GetMetrics(tx) => {
            // Get memory usage (approximate)
            let buffer = editor.buffer();
            let buffer_byte_size = buffer.rope().len_bytes();
            let buffer_line_count = buffer.rope().len_lines();

            // Memory usage estimate in MB (rough approximation)
            let memory_usage_mb = (buffer_byte_size as f64) / (1024.0 * 1024.0);

            let metrics_info = ovim::api::MetricsInfo {
                buffer_line_count,
                buffer_byte_size,
                syntax_enabled: buffer.has_syntax_highlighting(),
                is_large_file: buffer_line_count > 50_000, // Threshold for "large file"
                render_count: editor.render_count(),
                last_render_duration_micros: editor.last_render_duration_micros(),
                last_syntax_duration_micros: editor.last_syntax_duration_micros(),
                memory_usage_mb,
                // Input latency percentiles
                input_latency_p50_micros: editor.input_latency_p50_micros(),
                input_latency_p95_micros: editor.input_latency_p95_micros(),
                input_latency_p99_micros: editor.input_latency_p99_micros(),
                input_latency_samples: editor.input_latency_sample_count(),
                // Operation timings
                last_lsp_serialize_micros: editor.last_lsp_serialize_micros(),
                last_git_status_micros: editor.last_git_status_micros(),
                last_fold_calc_micros: editor.last_fold_calc_micros(),
                last_diagnostic_query_micros: editor.last_diagnostic_query_micros(),
            };

            let _ = tx.send(ApiResponse::Metrics(metrics_info));
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

    let visual_selection =
        editor
            .visual_selection()
            .map(
                |((start_line, start_col), (end_line, end_col))| VisualSelection {
                    start: CursorPosition {
                        line: start_line,
                        column: start_col,
                    },
                    end: CursorPosition {
                        line: end_line,
                        column: end_col,
                    },
                },
            );

    // Get registers content
    let mut registers = HashMap::new();
    let reg_manager = editor.registers();
    for reg_name in &[
        '"', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g',
        'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y',
        'z',
    ] {
        let content = reg_manager.get(Some(*reg_name));
        if !content.is_empty() {
            registers.insert(reg_name.to_string(), content);
        }
    }

    // Get marks
    let mut marks = HashMap::new();
    let mark_manager = editor.marks();
    for (name, mark) in mark_manager.iter() {
        marks.insert(
            name.to_string(),
            CursorPosition {
                line: mark.line,
                column: mark.col,
            },
        );
    }

    // Get picker state if in picker mode
    let picker = editor.picker().map(|p| PickerInfo {
        mode: match p.mode() {
            editor::PickerMode::FindFiles => "FindFiles".to_string(),
            editor::PickerMode::LiveGrep => "LiveGrep".to_string(),
            editor::PickerMode::Custom => "Custom".to_string(),
            editor::PickerMode::Completion => "Completion".to_string(),
            editor::PickerMode::LspLocations => "LspLocations".to_string(),
        },
        query: p.query().to_string(),
        results: p
            .filtered_results()
            .iter()
            .map(|r| PickerResultInfo {
                display: r.display.clone(),
                location: r.location.clone(),
                line: r.line,
                col: r.col,
            })
            .collect(),
        selected_index: p.selected_index(),
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
