use anyhow::Result;
use crossterm::event::{self, Event, EventStream};
use futures::StreamExt;
use ovim::key_convert::{convert_key_event, convert_mouse_event};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration, Instant};

use ovim::api::{
    parse_key_string, ApiRequest, ApiResponse, BufferInfo, CursorPosition, DiagnosticCounts,
    DiagnosticItem, DiagnosticsInfo, EditorSnapshot, ErrorResponse, HealthInfo, LineEntry,
    LinesResponse, LspServerInfoItem, LspStatusInfo, ModeInfo, PickerInfo, PickerResultInfo,
    RenderInfo, SuccessResponse, VisualSelection,
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
            match editor.apply_workspace_edit(workspace_edit) {
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
        if editor.take_diagnostics_refresh_request() || lsp_manager.diagnostics_changed() {
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

    if editor.poll_pending_completion_response() {
        editor.mark_dirty();
    }

    if editor.poll_pending_ai_jobs() {
        editor.mark_dirty();
    }

    if editor.poll_pending_ai_chat_job() {
        editor.mark_dirty();
    }

    if editor.poll_pending_workflow_jobs() {
        editor.mark_dirty();
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
        let mut picker_changed = false;
        if let Some(picker) = editor.picker_mut() {
            if picker.tick() {
                picker_changed = true;
            }
            // Drain streaming grep results (LiveGrep mode)
            if picker.drain_grep_results() {
                picker_changed = true;
            }
        }
        if picker_changed {
            editor.mark_dirty();
        }
        // Apply debounced filter for non-nucleo modes
        if editor.apply_pending_picker_filter(50) {
            editor.mark_dirty();
        }
        spawn_picker_preview_loading(editor, preview_tx);
        spawn_file_finder_loading(editor, file_tx);
        // Re-render when rapid scrolling stops so syntax highlighting gets applied
        if editor.picker_rapid_scrolling_just_stopped() {
            editor.mark_dirty();
        }
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

            let result =
                crate::lsp_init::auto_install::attempt_auto_install(&lang_name, &command, &config)
                    .await;

            let status = match result {
                crate::lsp_init::auto_install::InstallResult::Success(_) => InstallStatus::Success,
                crate::lsp_init::auto_install::InstallResult::Failed(msg) => {
                    InstallStatus::Failed(msg)
                }
                crate::lsp_init::auto_install::InstallResult::PrerequisitesMissing(msg) => {
                    InstallStatus::Failed(msg)
                }
                crate::lsp_init::auto_install::InstallResult::Declined => {
                    InstallStatus::Failed("Declined".to_string())
                }
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

/// Process a batch of terminal input events.
/// Returns true if any events were edit-related (for debounce tracking).
fn process_input_events(editor: &mut Editor, events: Vec<Event>) -> Result<bool> {
    let mut had_edit = false;
    for event in events {
        match event {
            Event::Key(key_event) => {
                let key = convert_key_event(key_event);
                InputHandler::handle_key_event_no_dirty(editor, key)?;
                had_edit = true;
            }
            Event::Paste(text) => {
                editor.handle_paste_event(&text)?;
                had_edit = true;
            }
            Event::Resize(w, h) => {
                // Keep cached viewport geometry in sync with the terminal size so vertical
                // scrolling (especially near EOF) works correctly after pane resizes.
                //
                // Without this, rapid post-resize navigation can use stale viewport/wrap
                // dimensions until the next render pass updates them, which can make the
                // cursor move past the visible buffer without the viewport following.
                handle_terminal_resize(editor, w, h)?;
                editor.startle_cat();
            }
            Event::FocusGained => {
                if let Ok(true) = editor.buffer_mut().reload_if_changed_sync() {
                    if editor.buffer().needs_rehighlight() {
                        editor.process_viewport_rehighlight();
                    }
                }
            }
            Event::Mouse(mouse_event) => {
                // Skip mouse-move events — they don't change editor state and
                // would otherwise trigger unnecessary redraws on every movement.
                if matches!(mouse_event.kind, crossterm::event::MouseEventKind::Moved) {
                    continue;
                }
                let mouse = convert_mouse_event(mouse_event);
                handle_mouse_event(editor, mouse)?;
                had_edit = true;
            }
            _ => {}
        }
    }
    Ok(had_edit)
}

fn handle_terminal_resize(editor: &mut Editor, width: u16, height: u16) -> Result<()> {
    // Recompute the buffer chunk dimensions using the same high-level rules as the renderer.
    // This is intentionally approximate (position doesn't matter here) — it just needs to
    // keep viewport height and wrap width correct for scroll calculations.
    let mut content_width = width;
    let mut content_height = height;

    // Tab bar consumes one row when multiple tabs are present.
    if editor.tab_count() > 1 {
        content_height = content_height.saturating_sub(1);
    }

    // File tree consumes fixed width when visible.
    if editor.file_tree().is_visible() {
        content_width = content_width.saturating_sub(50);
    }

    // Progress line consumes one row when present.
    if editor.lsp_progress_message().is_some() {
        content_height = content_height.saturating_sub(1);
    }

    // Status line + command/message line.
    content_height = content_height.saturating_sub(2);

    // Apply textwidth centering: narrowing changes wrap width, but not viewport height.
    if let Some(textwidth) = editor.options.textwidth {
        let max_width = textwidth as u16;
        if content_width > max_width {
            content_width = max_width;
        }
    }

    // Update cached viewport dimensions for scroll calculations.
    editor.set_viewport_height(content_height as usize);

    // Update window sizes so horizontal scrolling calculations use the latest width.
    if let Some(wm) = editor.window_manager_mut() {
        wm.update_dimensions(content_width, content_height);
    } else {
        editor.init_window_manager(content_width, content_height);
    }

    // Keep the wrap map in sync with the new width so vertical scrolling stays accurate
    // in wrap mode.
    if editor.options.wrap {
        let text_width = compute_text_width(editor, content_width);
        editor.ensure_wrap_map(text_width);
    }

    // Re-run scroll update so the cursor remains visible in the resized viewport.
    editor.update_scroll_offset();

    Ok(())
}

fn compute_text_width(editor: &Editor, content_width: u16) -> usize {
    // Keep in sync with `ovim::ui::renderer::layout::BufferLayout::compute`.
    const SIGN_WIDTH: usize = 2;
    const GUTTER_SPACING: usize = 1;

    let show_numbers = editor.options.number || editor.options.relative_number;
    let line_count = editor.buffer().line_count();
    let line_num_width = if show_numbers {
        line_count.to_string().len().max(3)
    } else {
        0
    };

    let blame_width = if editor.options.blame {
        if let Some(blame) = editor.buffer().git_blame() {
            let author_len = blame.max_author_len().min(15);
            1 + 1 + 5 + 1 + author_len.max(3) + 1
        } else {
            0
        }
    } else {
        0
    };

    let gutter_width = blame_width + SIGN_WIDTH + line_num_width + GUTTER_SPACING;
    (content_width as usize).saturating_sub(gutter_width)
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
    let (preview_tx, mut preview_rx) =
        tokio::sync::mpsc::channel::<(String, editor::PreviewCache)>(100);
    let (file_tx, mut file_rx) = tokio::sync::mpsc::channel::<editor::PickerResult>(1000);

    let mut event_stream = EventStream::new();
    let mut tick_interval = interval(Duration::from_millis(16));
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    while !editor.should_quit() {
        // Wait for input, API request, or tick — input has priority via `biased`
        tokio::select! {
            biased;

            // Terminal input (highest priority)
            maybe_event = event_stream.next() => {
                if let Some(Ok(first_event)) = maybe_event {
                    last_input_time = Some(Instant::now());

                    // Batch: collect first event + drain all queued events
                    let mut events = vec![first_event];
                    while event::poll(std::time::Duration::ZERO).unwrap_or(false) {
                        if let Ok(ev) = event::read() {
                            events.push(ev);
                        }
                    }

                    let had_edit = process_input_events(editor, events)?;
                    if had_edit {
                        last_edit = Instant::now();
                    }

                    // Mark dirty ONCE after all events processed
                    editor.mark_dirty();

                    // Immediate viewport rehighlight
                    if editor.buffer().needs_rehighlight() {
                        editor.process_viewport_rehighlight();
                    }

                    // Immediately process LSP actions triggered by input
                    if !editor.has_pending_lsp_response() {
                        editor.process_pending_lsp_actions().await;
                    }

                    // If more input queued, skip render to keep input flowing
                    if crossterm::event::poll(std::time::Duration::ZERO).unwrap_or(false) {
                        continue;
                    }
                }
            }

            // API requests
            Some(request) = async {
                if let Some(ref mut rx) = api_rx { rx.recv().await } else { std::future::pending().await }
            } => {
                let dummy_start = SystemTime::now();
                let dummy_session = Arc::new(Mutex::new(SessionInfo::new(0, None, "tui".into())));
                handle_api_request(editor, request, dummy_start, &dummy_session).await;
                // Drain remaining queued API requests
                if let Some(ref mut rx) = api_rx {
                    while let Ok(req) = rx.try_recv() {
                        handle_api_request(editor, req, dummy_start, &dummy_session).await;
                    }
                }
            }

            // Tick timer — background work (LSP, picker, animations)
            _ = tick_interval.tick() => {
                process_editor_tick(editor, &mut java_status_rx, &preview_tx, &file_tx).await;
                process_picker_results(editor, &mut preview_rx, &mut file_rx);

                if editor.tick_cat_animation() {
                    editor.mark_dirty();
                }
                if editor.tick_yank_flash() {
                    editor.mark_dirty();
                }
                if editor.tick_toasts() {
                    editor.mark_dirty();
                }
            }
        }

        // Render after any select branch (if dirty)
        if editor.is_dirty() {
            let start = Instant::now();
            ui.renderer_mut().render(editor)?;
            editor.record_render_duration(start.elapsed().as_micros() as u64);
            editor.increment_render_count();
            editor.mark_clean();
            if let Some(input_time) = last_input_time.take() {
                editor.record_input_latency(input_time.elapsed().as_micros() as u64);
            }
        }

        // Debounced rehighlight
        if editor.buffer().needs_rehighlight() && last_edit.elapsed() >= debounce_delay {
            editor.process_pending_rehighlight().await;
        }

        editor.send_lsp_close_if_needed().await;
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
        ApiRequest::GetSnapshotLight(tx) => {
            let snapshot = create_snapshot_light(editor);
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
                "AI_PROMPT" => Mode::AiPrompt,
                "AI_CHAT" => Mode::AiChat,
                _ => {
                    let _ = tx.send(ApiResponse::Error(ErrorResponse {
                        error: format!("Invalid mode: {}. Valid modes: NORMAL, INSERT, VISUAL, VISUAL_LINE, VISUAL_BLOCK, COMMAND, SEARCH, PICKER, AI_PROMPT, AI_CHAT", mode_str),
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
            let response: ApiResponse = commands::execute_command(editor, &command).into();
            let _ = tx.send(response);
        }
        ApiRequest::GetRender(tx) => {
            // Default dimensions: 80x24
            const DEFAULT_WIDTH: u16 = 80;
            const DEFAULT_HEIGHT: u16 = 24;

            match ovim::ui::render_editor_to_ansi(editor, DEFAULT_WIDTH, DEFAULT_HEIGHT) {
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
        ApiRequest::EditLine { line, old, new, tx } => {
            let response = handle_edit_line(editor, line, &old, &new);
            let _ = tx.send(response);
        }
        ApiRequest::InsertLines {
            line,
            before,
            text,
            tx,
        } => {
            let response = handle_insert_lines(editor, line, before, &text);
            let _ = tx.send(response);
        }
        ApiRequest::DeleteLines { from, to, tx } => {
            let response = handle_delete_lines(editor, from, to);
            let _ = tx.send(response);
        }
        ApiRequest::ReadLines { from, to, tx } => {
            let response = handle_read_lines(editor, from, to);
            let _ = tx.send(response);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::compute_text_width;
    use super::handle_terminal_resize;
    use ovim::editor::Editor;

    #[test]
    fn resize_updates_viewport_and_wrap_map_and_keeps_cursor_visible() {
        // 200 logical lines, wide enough to exercise gutter sizing.
        let content: String = (1..=200)
            .map(|i| format!("line {i}: {}\n", "x".repeat(120)))
            .collect();

        let mut editor = Editor::with_content(&content);
        editor.options.number = true;
        editor.options.wrap = true;
        editor.options.scrolloff = 0;

        // Initial size.
        handle_terminal_resize(&mut editor, 80, 20).unwrap();
        assert_eq!(editor.viewport_height(), 18);

        // Move cursor to EOF and ensure scroll offset is set.
        let last_line = editor.buffer().line_count().saturating_sub(1);
        editor.buffer_mut().cursor_mut().set_position(last_line, 0);
        editor.update_scroll_offset();

        // Shrink the pane; cursor should remain visible in the new viewport.
        handle_terminal_resize(&mut editor, 80, 10).unwrap();
        assert_eq!(editor.viewport_height(), 8);

        let cursor_line = editor.buffer().cursor().line();
        let scroll_offset = editor.scroll_offset();
        let visible = editor.viewport_height().max(1);
        assert!(
            cursor_line >= scroll_offset && cursor_line < scroll_offset + visible,
            "cursor should remain visible after resize: cursor_line={cursor_line} scroll_offset={scroll_offset} viewport={visible}"
        );

        // Wrap map should match the new text width (buffer width minus gutter).
        let wrap_width = editor.wrap_map().map(|m| m.wrap_width()).unwrap_or(0);
        assert_eq!(wrap_width, compute_text_width(&editor, 80).max(1));
    }
}

/// Handle edit-line API request: find and replace text on a specific line or whole buffer
fn handle_edit_line(editor: &mut Editor, line: Option<usize>, old: &str, new: &str) -> ApiResponse {
    let rope = editor.buffer().rope();
    let total_lines = rope.len_lines();

    // Find the match
    let matches: Vec<(usize, usize)> = if let Some(line_idx) = line {
        // Search within a specific line (0-indexed)
        if line_idx >= total_lines {
            return ApiResponse::Error(ErrorResponse {
                error: format!(
                    "Line {} out of range (buffer has {} lines)",
                    line_idx + 1,
                    total_lines
                ),
            });
        }
        let line_text = rope.line(line_idx).to_string();
        // Trim trailing newline for matching
        let line_content = line_text.trim_end_matches('\n');
        let mut found = Vec::new();
        let mut search_start = 0;
        while let Some(pos) = line_content[search_start..].find(old) {
            found.push((line_idx, search_start + pos));
            search_start += pos + old.len();
        }
        found
    } else {
        // Search whole buffer
        let mut found = Vec::new();
        for line_idx in 0..total_lines {
            let line_text = rope.line(line_idx).to_string();
            let line_content = line_text.trim_end_matches('\n');
            let mut search_start = 0;
            while let Some(pos) = line_content[search_start..].find(old) {
                found.push((line_idx, search_start + pos));
                search_start += pos + old.len();
            }
        }
        found
    };

    if matches.is_empty() {
        return ApiResponse::Error(ErrorResponse {
            error: "Text not found".to_string(),
        });
    }

    if matches.len() > 1 && line.is_none() {
        return ApiResponse::Error(ErrorResponse {
            error: format!(
                "Ambiguous: found {} matches. Use --line to specify which line.",
                matches.len()
            ),
        });
    }

    let (match_line, match_col) = matches[0];

    // Record cursor position before change
    let cursor_before = {
        let c = editor.buffer().cursor();
        (c.line(), c.col())
    };

    // Perform the edit: delete old text, insert new text
    let end_col = match_col + old.len();
    let deleted = editor
        .buffer_mut()
        .delete_range(match_line, match_col, match_line, end_col);
    editor
        .buffer_mut()
        .insert_text_at(match_line, match_col, new);

    // Record composite change for undo
    let change = ovim::editor::Change::composite(
        vec![
            ovim::editor::Change::delete(
                ovim::editor::Range::new((match_line, match_col), (match_line, end_col)),
                deleted,
                cursor_before,
            ),
            ovim::editor::Change::insert((match_line, match_col), new.to_string(), cursor_before),
        ],
        cursor_before,
        (match_line, match_col + new.len()),
    );
    editor.add_change(change);

    ApiResponse::Success(SuccessResponse {
        success: true,
        message: Some(format!("Replaced on line {}", match_line + 1)),
        line_count: Some(editor.buffer().rope().len_lines()),
    })
}

/// Handle insert-lines API request: insert text before a specific line
fn handle_insert_lines(editor: &mut Editor, line: usize, _before: bool, text: &str) -> ApiResponse {
    let total_lines = editor.buffer().rope().len_lines();

    // `line` is 0-indexed insert position
    // Clamp to valid range
    if line > total_lines {
        return ApiResponse::Error(ErrorResponse {
            error: format!(
                "Line {} out of range (buffer has {} lines)",
                line + 1,
                total_lines
            ),
        });
    }

    // Calculate char position for insertion
    let char_idx = if line >= total_lines {
        editor.buffer().rope().len_chars()
    } else {
        editor.buffer().rope().line_to_char(line)
    };

    // Ensure text ends with newline
    let text_with_nl = if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{}\n", text)
    };

    let cursor_before = {
        let c = editor.buffer().cursor();
        (c.line(), c.col())
    };

    // Convert char_idx to line/col for insert_text_at
    let rope = editor.buffer().rope();
    let ins_line = rope.char_to_line(char_idx);
    let ins_col = char_idx - rope.line_to_char(ins_line);

    editor
        .buffer_mut()
        .insert_text_at(ins_line, ins_col, &text_with_nl);

    // Record change for undo
    let change = ovim::editor::Change::insert((ins_line, ins_col), text_with_nl, cursor_before);
    editor.add_change(change);

    ApiResponse::Success(SuccessResponse {
        success: true,
        message: Some(format!("Inserted at line {}", line + 1)),
        line_count: Some(editor.buffer().rope().len_lines()),
    })
}

/// Handle delete-lines API request: delete a range of lines (0-indexed, inclusive)
fn handle_delete_lines(editor: &mut Editor, from: usize, to: usize) -> ApiResponse {
    let total_lines = editor.buffer().rope().len_lines();

    if from >= total_lines {
        return ApiResponse::Error(ErrorResponse {
            error: format!(
                "Line {} out of range (buffer has {} lines)",
                from + 1,
                total_lines
            ),
        });
    }

    let to = to.min(total_lines.saturating_sub(1));

    if from > to {
        return ApiResponse::Error(ErrorResponse {
            error: format!("Invalid range: from {} > to {}", from + 1, to + 1),
        });
    }

    let cursor_before = {
        let c = editor.buffer().cursor();
        (c.line(), c.col())
    };

    // Calculate end position for delete_range
    let end_line = if to + 1 >= total_lines {
        total_lines.saturating_sub(1)
    } else {
        to + 1
    };
    let end_col = if to + 1 >= total_lines {
        let last_line = editor.buffer().rope().line(total_lines - 1);
        last_line.len_chars()
    } else {
        0
    };

    let deleted = editor.buffer_mut().delete_range(from, 0, end_line, end_col);

    // Record change for undo
    let change = ovim::editor::Change::delete(
        ovim::editor::Range::new((from, 0), (end_line, end_col)),
        deleted,
        cursor_before,
    );
    editor.add_change(change);

    // Adjust cursor if it was in deleted range
    let new_total = editor.buffer().rope().len_lines();
    let cursor = editor.buffer().cursor();
    if cursor.line() >= new_total && new_total > 0 {
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(new_total - 1, 0);
    }

    ApiResponse::Success(SuccessResponse {
        success: true,
        message: Some(format!("Deleted lines {}-{}", from + 1, to + 1)),
        line_count: Some(new_total),
    })
}

/// Handle read-lines API request: read a range of lines (0-indexed, inclusive)
fn handle_read_lines(editor: &Editor, from: usize, to: usize) -> ApiResponse {
    let rope = editor.buffer().rope();
    let total_lines = rope.len_lines();

    if from >= total_lines {
        return ApiResponse::Error(ErrorResponse {
            error: format!(
                "Line {} out of range (buffer has {} lines)",
                from + 1,
                total_lines
            ),
        });
    }

    let to = to.min(total_lines.saturating_sub(1));

    let mut lines = Vec::new();
    for idx in from..=to {
        let line_text = rope.line(idx).to_string();
        // Strip trailing newline
        let text = line_text.trim_end_matches('\n').to_string();
        lines.push(LineEntry {
            number: idx + 1, // 1-indexed for display
            text,
        });
    }

    ApiResponse::Lines(LinesResponse { lines, total_lines })
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

        // Get the base directory for file search (git root when available)
        let base_dir = picker.base_dir().to_path_buf();
        // Preferred directory for local-first ordering (typically current file's folder)
        let preferred_dir = picker.preferred_dir().to_path_buf();

        // Check for cached file list (5-minute TTL)
        if let Some(cached_files) = editor.get_cached_file_list(&base_dir, &preferred_dir) {
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
        let preferred_dir_clone = preferred_dir.clone();

        // Spawn background task - doesn't block!
        // Also collects results for cache update
        tokio::spawn(async move {
            use ignore::WalkBuilder;

            let mut collected_files = Vec::new();

            let mut roots: Vec<(std::path::PathBuf, bool)> = Vec::new();
            if preferred_dir_clone != base_dir_clone
                && preferred_dir_clone.starts_with(&base_dir_clone)
            {
                roots.push((preferred_dir_clone.clone(), true));
            }
            roots.push((base_dir_clone.clone(), false));

            // Walk preferred subtree first, then base (excluding preferred) for local-first ordering.
            for (root, is_preferred_root) in roots {
                let base_dir_for_strip = base_dir_clone.clone();
                let preferred_for_filter = preferred_dir_clone.clone();

                // Use ignore crate's WalkBuilder which respects .gitignore
                let walker = WalkBuilder::new(&root)
                    .hidden(false) // Don't automatically skip hidden files (keep .env, .eslintrc, etc.)
                    .git_ignore(true) // Respect .gitignore files
                    .git_global(true) // Respect global gitignore
                    .git_exclude(true) // Respect .git/info/exclude
                    .filter_entry(move |entry| {
                        // Skip .git directory (not in .gitignore but shouldn't be shown)
                        if entry.file_name() == ".git" {
                            return false;
                        }
                        // For the base-dir pass, skip the preferred subtree entirely to avoid duplicates.
                        if !is_preferred_root
                            && preferred_for_filter != base_dir_for_strip
                            && entry.path().starts_with(&preferred_for_filter)
                        {
                            return false;
                        }
                        true
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
            }

            // Store collected files in a static to be picked up by cache update
            // This is a workaround since we can't update Editor state from within a spawned task
            FILE_LIST_CACHE_RESULTS.lock().await.replace((
                base_dir_clone,
                preferred_dir_clone,
                collected_files,
            ));
        });
    }
}

/// Temporary storage for file list results from background task
/// The main event loop will pick these up and update the Editor cache
static FILE_LIST_CACHE_RESULTS: tokio::sync::Mutex<
    Option<(
        std::path::PathBuf,
        std::path::PathBuf,
        Vec<editor::PickerResult>,
    )>,
> = tokio::sync::Mutex::const_new(None);

/// Picks up cached file list results from the background task and updates Editor cache
pub fn update_file_list_cache_from_background(editor: &mut Editor) {
    // Non-blocking try_lock to avoid any contention with background task
    if let Ok(mut guard) = FILE_LIST_CACHE_RESULTS.try_lock() {
        if let Some((base_dir, preferred_dir, files)) = guard.take() {
            editor.update_file_list_cache(base_dir, preferred_dir, files);
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

/// Lightweight snapshot: skips buffer content, registers, marks, and picker.
/// Used by MCP polling and other callers that only need mode/cursor/hover.
fn create_snapshot_light(editor: &Editor) -> EditorSnapshot {
    let cursor = editor.buffer().cursor();
    let cursor_pos = CursorPosition {
        line: cursor.line(),
        column: cursor.col(),
    };

    EditorSnapshot {
        buffer: BufferInfo {
            content: String::new(),
            line_count: editor.buffer().rope().len_lines(),
            file_path: editor.buffer().file_path().map(|s| s.to_string()),
        },
        cursor: cursor_pos,
        mode: editor.mode().display_name().to_string(),
        visual_selection: None,
        registers: HashMap::new(),
        marks: HashMap::new(),
        picker: None,
        hover_info: editor.hover_info().map(|s| s.to_string()),
    }
}

fn create_buffer_info(editor: &Editor) -> BufferInfo {
    let buffer = editor.buffer();
    let rope = buffer.rope();

    // Write rope chunks directly into a pre-allocated String.
    // This avoids the intermediate allocations that rope.to_string() can cause.
    let byte_len = rope.len_bytes();
    let mut content = String::with_capacity(byte_len);
    for chunk in rope.chunks() {
        content.push_str(chunk);
    }

    let line_count = rope.len_lines();
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
