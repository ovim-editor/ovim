use anyhow::Result;
use crossterm::event::Event;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration, Instant};

use ovim::api::{
    parse_key_string, ApiRequest, ApiResponse, BufferInfo, CursorPosition, DiagnosticCounts,
    DiagnosticItem, DiagnosticsInfo, EditorSnapshot, ErrorResponse, HealthInfo, LspServerInfoItem,
    LspStatusInfo, ModeInfo, PickerInfo, PickerResultInfo, RenderInfo, SuccessResponse,
    VisualSelection,
};
use ovim::commands;
use ovim::editor::{self, handle_mouse_event, Editor, InputHandler};
use ovim::mode::Mode;
use ovim::session::SessionInfo;
use ovim::syntax::LanguageRegistry;
use ovim::ui::UI;

/// Shared editor tick for both loops.
/// Handles LSP, diagnostics, syntax, Lua, and background file tasks.
async fn process_editor_tick(
    editor: &mut Editor,
    java_status_rx: &mut mpsc::UnboundedReceiver<String>,
    preview_tx: &tokio::sync::mpsc::Sender<(String, editor::PreviewCache)>,
    file_tx: &tokio::sync::mpsc::Sender<editor::PickerResult>,
) {
    while let Ok(status) = java_status_rx.try_recv() {
        editor.set_lsp_status(status);
    }

    if let Some(lsp_manager) = editor.lsp_manager() {
        let notification_count = lsp_manager.process_notifications().await;
        let flush_count = lsp_manager.process_flush_requests().await;

        // Mark dirty if we processed any LSP messages
        if notification_count > 0 || flush_count > 0 {
            editor.mark_dirty();
        }

        // Poll for server-initiated workspace edits (e.g., from refactoring operations)
        let pending_edits = lsp_manager.poll_pending_workspace_edits().await;
        for workspace_edit in pending_edits {
            match editor.apply_workspace_edit(workspace_edit).await {
                Ok(applied) => {
                    if applied {
                        editor.set_lsp_status("Applied workspace edit".to_string());
                        editor.mark_dirty(); // Redraw after applying workspace edit
                    } else {
                        editor.set_lsp_status("Partially applied workspace edit".to_string());
                        editor.mark_dirty(); // Redraw even if partially applied
                    }
                }
                Err(e) => {
                    editor.set_lsp_status(format!("Failed to apply edit: {}", e));
                    editor.mark_dirty(); // Redraw to show error status
                }
            }
        }
    }

    if let Some(file_path) = editor.needs_lsp_init() {
        crate::lsp_init::initialize_lsp_for_file(editor, &file_path).await;
        editor.clear_lsp_init_flag();
    }

    if let Some(lsp_manager) = editor.lsp_manager() {
        if lsp_manager.diagnostics_changed() {
            editor.update_diagnostic_cache().await;
            editor.mark_dirty(); // Redraw when diagnostics change
        }
    }

    if editor.buffer().should_init_syntax() {
        editor.buffer_mut().enable_syntax_highlighting();
        editor.mark_dirty(); // Redraw when syntax highlighting is enabled
    }

    // Poll pending LSP responses (non-blocking)
    if editor.poll_pending_lsp_responses() {
        editor.mark_dirty(); // Redraw when response arrives
    }

    // Only process new actions if not waiting for response
    if !editor.has_pending_lsp_response() {
        editor.process_pending_lsp_actions().await;
    }

    let _ = editor.process_lua_commands();

    // Process LSP install requests from the LSP Manager panel
    spawn_pending_installs(editor);
    if editor.poll_install_progress() {
        editor.mark_dirty();
    }

    if editor.mode() == Mode::Picker {
        // Tick picker: drives nucleo matching (FindFiles) or applies debounced filter (other modes)
        if let Some(picker) = editor.picker_mut() {
            if picker.tick() {
                editor.mark_dirty();
            }
        }
        // Apply debounced filter for non-nucleo modes
        if editor.apply_pending_picker_filter(50) {
            editor.mark_dirty();
        }
        spawn_picker_preview_loading(editor, preview_tx);
        spawn_file_finder_loading(editor, file_tx);
    }

    editor.send_lsp_changes_if_modified().await;
    editor.send_lsp_save_if_needed().await;
}

/// Spawn background tasks for pending LSP install requests
fn spawn_pending_installs(editor: &mut Editor) {
    use ovim::editor::lsp_manager_panel::{InstallProgress, InstallStatus};

    let pending = editor.take_pending_installs();
    if pending.is_empty() {
        return;
    }

    let tx = editor.install_progress_tx().cloned();
    let Some(tx) = tx else { return };

    for request in pending {
        let tx = tx.clone();
        let lang_name = request.language_name.clone();
        let lang_id = request.language_id.clone();
        let config = request.auto_install_config.clone();
        let command = request.lsp_command.clone();

        tokio::spawn(async move {
            let _ = tx.send(InstallProgress {
                language_id: lang_id.clone(),
                status: InstallStatus::Installing(format!("Installing {lang_name}...")),
            });

            let result = crate::lsp_init::auto_install::attempt_auto_install(
                &lang_name,
                &command,
                &config,
            )
            .await;

            let status = match result {
                crate::lsp_init::auto_install::InstallResult::Success(_) => InstallStatus::Success,
                crate::lsp_init::auto_install::InstallResult::Failed(msg) => InstallStatus::Failed(msg),
                crate::lsp_init::auto_install::InstallResult::PrerequisitesMissing(msg) => InstallStatus::Failed(msg),
                crate::lsp_init::auto_install::InstallResult::Declined => InstallStatus::Failed("Declined".to_string()),
            };

            let _ = tx.send(InstallProgress {
                language_id: lang_id,
                status,
            });
        });
    }
}

/// Helper to process preview and file picker results
fn process_picker_results(
    editor: &mut Editor,
    preview_rx: &mut tokio::sync::mpsc::Receiver<(String, editor::PreviewCache)>,
    file_rx: &mut tokio::sync::mpsc::Receiver<editor::PickerResult>,
) {
    // Try to drain pending preview loads (single mark_dirty after batch)
    let mut previews_loaded = false;
    while let Ok((path, cache)) = preview_rx.try_recv() {
        editor.insert_preview(path, cache);
        previews_loaded = true;
    }
    if previews_loaded {
        editor.mark_dirty();
    }
    // Drain pending file results with a time budget to avoid stalling input
    let mut files_added = false;
    let drain_start = std::time::Instant::now();
    let drain_budget = std::time::Duration::from_millis(2);
    loop {
        if drain_start.elapsed() >= drain_budget {
            break;
        }
        match file_rx.try_recv() {
            Ok(result) => {
                if let Some(picker) = editor.picker_mut() {
                    picker.add_file_result(result);
                    files_added = true;
                }
            }
            Err(_) => break,
        }
    }
    if files_added {
        editor.mark_dirty();
    }
    // Update file list cache from background task (if completed)
    update_file_list_cache_from_background(editor);
}

/// Headless (API-only) event loop.
pub async fn run_headless_loop(
    editor: &mut Editor,
    mut api_rx: mpsc::UnboundedReceiver<ApiRequest>,
    mut java_status_rx: mpsc::UnboundedReceiver<String>,
    start_time: SystemTime,
    session_info: Arc<Mutex<SessionInfo>>,
) -> Result<()> {
    let (preview_tx, mut preview_rx) =
        tokio::sync::mpsc::channel::<(String, editor::PreviewCache)>(100);
    let (file_tx, mut file_rx) = tokio::sync::mpsc::channel::<editor::PickerResult>(1000);
    let mut lsp_interval = interval(Duration::from_millis(50));
    lsp_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(request) = api_rx.recv() => {
                handle_api_request(editor, request, start_time, &session_info).await;
                if editor.should_quit() { break; }
            }
            Some((path, cache)) = preview_rx.recv() => {
                editor.insert_preview(path, cache);
                // Note: headless mode doesn't need mark_dirty() since there's no UI to redraw
            }
            Some(result) = file_rx.recv() => {
                if let Some(picker) = editor.picker_mut() { picker.add_file_result(result); }
            }
            _ = lsp_interval.tick() => {
                process_editor_tick(editor, &mut java_status_rx, &preview_tx, &file_tx).await;
            }
        }
    }
    Ok(())
}

/// TUI event loop (optionally with API).
pub async fn run_event_loop(
    ui: &mut UI,
    editor: &mut Editor,
    mut api_rx: Option<mpsc::UnboundedReceiver<ApiRequest>>,
    mut java_status_rx: mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    let mut last_edit = Instant::now();
    let debounce_delay = Duration::from_millis(200);
    let mut last_input_time: Option<Instant> = None;
    let mut skip_render = false;
    let (preview_tx, mut preview_rx) =
        tokio::sync::mpsc::channel::<(String, editor::PreviewCache)>(100);
    let (file_tx, mut file_rx) = tokio::sync::mpsc::channel::<editor::PickerResult>(1000);

    while !editor.should_quit() {
        process_editor_tick(editor, &mut java_status_rx, &preview_tx, &file_tx).await;

        // Drain pending picker results
        process_picker_results(editor, &mut preview_rx, &mut file_rx);

        // Tick dashboard cat animation
        if editor.tick_cat_animation() {
            editor.mark_dirty();
        }

        if editor.is_dirty() && !skip_render {
            let start = Instant::now();
            ui.renderer_mut().render(editor)?;
            editor.record_render_duration(start.elapsed().as_micros() as u64);
            editor.increment_render_count();
            editor.mark_clean();
            if let Some(input_time) = last_input_time.take() {
                editor.record_input_latency(input_time.elapsed().as_micros() as u64);
            }
        }
        skip_render = false;

        if let Some(ref mut rx) = api_rx {
            while let Ok(request) = rx.try_recv() {
                let dummy_start = SystemTime::now();
                let dummy_session = Arc::new(Mutex::new(SessionInfo::new(0, None, "tui".into())));
                handle_api_request(editor, request, dummy_start, &dummy_session).await;
            }
        }

        // Batch all pending events before rendering (improves paste performance)
        let events = InputHandler::poll_all_events()?;

        if !events.is_empty() {
            last_input_time = Some(Instant::now());

            for event in events {
                match event {
                    Event::Key(key_event) => {
                        InputHandler::handle_key_event_no_dirty(editor, key_event)?;
                        last_edit = Instant::now();
                    }
                    Event::Paste(text) => {
                        editor.handle_paste_event(&text)?;
                        last_edit = Instant::now();
                    }
                    Event::Resize(_, _) => {
                        // Terminal was resized - handled by dirty flag below
                        // Startle the dashboard cat if it's on the logo
                        editor.startle_cat();
                    }
                    Event::FocusGained => {
                        // Auto-reload file if changed externally while terminal was unfocused
                        if let Ok(true) = editor.buffer_mut().reload_if_changed_sync() {
                            // File was reloaded - trigger rehighlight
                            if editor.buffer().needs_rehighlight() {
                                editor.process_viewport_rehighlight();
                            }
                        }
                    }
                    Event::Mouse(mouse_event) => {
                        handle_mouse_event(editor, mouse_event)?;
                        last_edit = Instant::now();
                    }
                    _ => {
                        // Ignore other events (focus lost, etc.)
                    }
                }
            }

            // Mark dirty ONCE after all events processed
            editor.mark_dirty();

            // Immediate viewport rehighlight for accurate visible highlights (no debounce)
            if editor.buffer().needs_rehighlight() {
                editor.process_viewport_rehighlight();
            }

            // Immediately process any LSP actions triggered by input (don't wait for tick)
            // This makes hover/goto/completion feel much snappier
            if !editor.has_pending_lsp_response() {
                editor.process_pending_lsp_actions().await;
            }

            // If more input is already queued, skip the next render frame
            // so keystrokes flow through without delay
            if crossterm::event::poll(std::time::Duration::ZERO).unwrap_or(false) {
                skip_render = true;
            }
        }

        if editor.buffer().needs_rehighlight() && last_edit.elapsed() >= debounce_delay {
            editor.process_pending_rehighlight().await;
        }

        editor.send_lsp_close_if_needed().await;

        tokio::task::yield_now().await;
    }

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
            let events_result = parse_key_string(&keys);
            let response = match events_result {
                Ok(events) => {
                    let mut success = true;

                    for event in events {
                        if InputHandler::handle_key_event(editor, event).is_err() {
                            success = false;
                            break;
                        }
                    }

                    // Process any LSP actions that were triggered by the keys
                    editor.process_pending_lsp_actions().await;

                    if success {
                        // Create context window showing the result of the key operation
                        let buffer = editor.buffer();
                        let cursor = buffer.cursor();
                        let buffer_content = buffer.rope().to_string();
                        let file_path = buffer.file_path();
                        let mode_str = editor.mode().display_name().to_string();

                        let context_str = ovim::api::format_context_window(
                            &buffer_content,
                            cursor.line(),
                            cursor.col(),
                            file_path,
                            &mode_str,
                        );

                        let context_info = ovim::api::ContextWindowInfo {
                            context: context_str,
                            file: file_path.map(|s| s.to_string()),
                            mode: mode_str,
                            line: cursor.line(),
                            column: cursor.col(),
                        };

                        ApiResponse::SendKeysResult(ovim::api::SendKeysResult {
                            success: true,
                            message: None,
                            context: context_info,
                        })
                    } else {
                        ApiResponse::Error(ErrorResponse {
                            error: "Failed to process keys".to_string(),
                        })
                    }
                }
                Err(parse_error) => ApiResponse::Error(ErrorResponse {
                    error: format!("Failed to parse keys: {}", parse_error),
                }),
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
        ApiRequest::SetMode(mode_str, tx) => {
            let new_mode = match mode_str.to_uppercase().as_str() {
                "NORMAL" => Mode::Normal,
                "INSERT" => Mode::Insert,
                "VISUAL" => Mode::Visual,
                "VISUAL_LINE" => Mode::VisualLine,
                "VISUAL_BLOCK" => Mode::VisualBlock,
                "COMMAND" => Mode::Command,
                "SEARCH" => Mode::Search,
                "PICKER" => Mode::Picker,
                _ => {
                    let _ = tx.send(ApiResponse::Error(ErrorResponse {
                        error: format!("Invalid mode: {}. Valid modes: NORMAL, INSERT, VISUAL, VISUAL_LINE, VISUAL_BLOCK, COMMAND, SEARCH, PICKER", mode_str),
                    }));
                    return;
                }
            };

            editor.set_mode(new_mode);
            let _ = tx.send(ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some(format!("Mode set to {}", mode_str.to_uppercase())),
                line_count: None,
            }));
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
                    tokio::runtime::Handle::current()
                        .block_on(async { lsp_manager_arc.get_lsp_status().await })
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
                    tokio::runtime::Handle::current()
                        .block_on(async { lsp_manager_arc.get_lsp_status().await })
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
        ApiRequest::GetOutline(tx) => {
            let info = editor.get_outline().await;
            let _ = tx.send(ApiResponse::Outline(info));
        }
        ApiRequest::SearchSymbol(query, tx) => {
            let info = editor.search_symbols(&query).await;
            let _ = tx.send(ApiResponse::SymbolSearch(info));
        }
        ApiRequest::GetTrace(tx) => {
            let info = editor.get_trace().await;
            let _ = tx.send(ApiResponse::Trace(info));
        }
        ApiRequest::GetDiagnostics(tx) => {
            let file = editor.buffer().file_path().map(|s| s.to_string());
            let raw_diagnostics = editor.all_diagnostics();
            let (errors, warnings, info_count, hints) = editor.cached_diagnostic_count();

            let diagnostics: Vec<DiagnosticItem> = raw_diagnostics
                .iter()
                .map(|d| {
                    let severity = match d.severity {
                        Some(lsp_types::DiagnosticSeverity::ERROR) => "error",
                        Some(lsp_types::DiagnosticSeverity::WARNING) => "warning",
                        Some(lsp_types::DiagnosticSeverity::INFORMATION) => "info",
                        Some(lsp_types::DiagnosticSeverity::HINT) => "hint",
                        _ => "unknown",
                    };
                    let code = d.code.as_ref().map(|c| match c {
                        lsp_types::NumberOrString::Number(n) => n.to_string(),
                        lsp_types::NumberOrString::String(s) => s.clone(),
                    });
                    DiagnosticItem {
                        line: d.range.start.line as usize + 1,
                        column: d.range.start.character as usize + 1,
                        end_line: d.range.end.line as usize + 1,
                        end_column: d.range.end.character as usize + 1,
                        severity: severity.to_string(),
                        message: d.message.clone(),
                        source: d.source.clone(),
                        code,
                    }
                })
                .collect();

            let info = DiagnosticsInfo {
                file,
                diagnostics,
                counts: DiagnosticCounts {
                    errors,
                    warnings,
                    info: info_count,
                    hints,
                },
            };
            let _ = tx.send(ApiResponse::Diagnostics(info));
        }
        ApiRequest::GetContextWindow(tx) => {
            let buffer = editor.buffer();
            let cursor = buffer.cursor();
            let cursor_line = cursor.line();
            let cursor_column = cursor.col();

            let buffer_content = buffer.rope().to_string();
            let file_path = buffer.file_path();
            let mode_str = editor.mode().display_name().to_string();

            let context_str = ovim::api::format_context_window(
                &buffer_content,
                cursor_line,
                cursor_column,
                file_path,
                &mode_str,
            );

            let context_info = ovim::api::ContextWindowInfo {
                context: context_str,
                file: file_path.map(|s| s.to_string()),
                mode: mode_str,
                line: cursor_line,
                column: cursor_column,
            };

            let _ = tx.send(ApiResponse::ContextWindow(context_info));
        }
    }
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
/// Uses cache when available to speed up repeated picker opens
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

        // Check for cached file list (5-minute TTL)
        if let Some(cached_files) = editor.get_cached_file_list(&base_dir) {
            // Use cache! Send all files via channel immediately
            let cached_files: Vec<editor::PickerResult> = cached_files.to_vec();
            let tx = file_tx.clone();

            // Mark as spawned to avoid spawning multiple tasks
            if let Some(picker_mut) = editor.picker_mut() {
                picker_mut.mark_loading_spawned();
            }

            // Spawn quick task to send cached results
            tokio::spawn(async move {
                for result in cached_files {
                    if tx.send(result).await.is_err() {
                        break;
                    }
                }
            });
            return;
        }

        // Mark as spawned to avoid spawning multiple tasks
        if let Some(picker_mut) = editor.picker_mut() {
            picker_mut.mark_loading_spawned();
        }

        let tx = file_tx.clone();
        let base_dir_clone = base_dir.clone();

        // Spawn background task - doesn't block!
        // Also collects results for cache update
        tokio::spawn(async move {
            use ignore::WalkBuilder;

            let mut collected_files = Vec::new();

            // Use ignore crate's WalkBuilder which respects .gitignore
            let walker = WalkBuilder::new(&base_dir_clone)
                .hidden(false) // Don't automatically skip hidden files (keep .env, .eslintrc, etc.)
                .git_ignore(true) // Respect .gitignore files
                .git_global(true) // Respect global gitignore
                .git_exclude(true) // Respect .git/info/exclude
                .filter_entry(|entry| {
                    // Skip .git directory (not in .gitignore but shouldn't be shown)
                    entry.file_name() != ".git"
                })
                .build();

            // Walk the directory tree and send files as we find them
            for entry in walker.filter_map(|e| e.ok()) {
                let path = entry.path();

                if path.is_file() {
                    if let Ok(relative_path) = path.strip_prefix(&base_dir_clone) {
                        let display_path = relative_path.to_string_lossy().to_string();
                        let result = editor::PickerResult {
                            display: display_path,
                            location: path.to_string_lossy().to_string(),
                            line: 0,
                            col: 0,
                            match_positions: Vec::new(),
                            content: None,
                        };

                        // Collect for cache
                        collected_files.push(result.clone());

                        // Send result back (non-blocking)
                        // If channel is closed (picker was closed), task will exit
                        if tx.send(result).await.is_err() {
                            break;
                        }
                    }
                }
            }

            // Store collected files in a static to be picked up by cache update
            // This is a workaround since we can't update Editor state from within a spawned task
            FILE_LIST_CACHE_RESULTS.lock().await.replace((base_dir_clone, collected_files));
        });
    }
}

/// Temporary storage for file list results from background task
/// The main event loop will pick these up and update the Editor cache
static FILE_LIST_CACHE_RESULTS: tokio::sync::Mutex<Option<(std::path::PathBuf, Vec<editor::PickerResult>)>> =
    tokio::sync::Mutex::const_new(None);

/// Picks up cached file list results from the background task and updates Editor cache
pub fn update_file_list_cache_from_background(editor: &mut Editor) {
    // Non-blocking try_lock to avoid any contention with background task
    if let Ok(mut guard) = FILE_LIST_CACHE_RESULTS.try_lock() {
        if let Some((root, files)) = guard.take() {
            editor.update_file_list_cache(root, files);
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
        results: (0..p.filtered_result_count())
            .filter_map(|i| p.filtered_result(i))
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
