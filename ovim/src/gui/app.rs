//! Tauri application shell shared by `ovim gui` and the `ovim-gui` desktop entry.

use super::browser::BrowserHost;
use super::{GuiBridge, GuiKeyInput, GuiSnapshot, GuiVectorSource};
use crate::cli::FileArg;
use anyhow::{Context, Result};
use base64::Engine as _;
use serde::Serialize;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::ipc::{Channel, InvokeBody, Request};
use tauri::{DragDropEvent, Emitter, EventTarget, Manager, RunEvent, State, Window, WindowEvent};

#[derive(Clone, Default)]
struct GuiExitGate(Arc<AtomicBool>);

#[tauri::command]
async fn gui_diff_action(
    bridge: State<'_, GuiBridge>,
    pane: usize,
    buffer_id: u64,
    action: String,
) -> Result<(), String> {
    bridge.diff_action(pane, buffer_id, action).await
}

#[tauri::command]
async fn gui_diff_comment(
    bridge: State<'_, GuiBridge>,
    pane: usize,
    buffer_id: u64,
    comment: super::diff::GuiDiffCommentInput,
) -> Result<(), String> {
    bridge.diff_comment(pane, buffer_id, comment).await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiffExportKind {
    Png,
    Zip,
}

impl DiffExportKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Zip => "zip",
        }
    }

    fn filter_name(self) -> &'static str {
        match self {
            Self::Png => "PNG image",
            Self::Zip => "ZIP archive",
        }
    }
}

fn validate_diff_export(filename: &str, data: &[u8]) -> Result<DiffExportKind, String> {
    if filename.is_empty()
        || filename.len() > 128
        || filename.starts_with('.')
        || filename.chars().any(|character| {
            !character.is_ascii_alphanumeric()
                && !matches!(character, ' ' | '-' | '_' | '.' | '(' | ')')
        })
    {
        return Err("Diff export filename must be a simple filename".to_string());
    }
    let kind = match Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => DiffExportKind::Png,
        Some("zip") => DiffExportKind::Zip,
        _ => return Err("Diff export must be a PNG or ZIP file".to_string()),
    };
    if data.is_empty() {
        return Err("Diff export must not be empty".to_string());
    }
    let valid_signature = match kind {
        DiffExportKind::Png => data.starts_with(b"\x89PNG\r\n\x1a\n"),
        DiffExportKind::Zip => data.starts_with(b"PK\x03\x04"),
    };
    if !valid_signature {
        return Err(format!(
            "Diff export is not a valid {} file",
            kind.extension()
        ));
    }
    Ok(kind)
}

fn save_diff_export_at(path: &Path, data: &[u8], kind: DiffExportKind) -> Result<(), String> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(kind.extension()))
    {
        return Err(format!("Choose a .{} filename", kind.extension()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Choose a destination folder for the diff export".to_string())?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".ovim-diff-export-")
        .tempfile_in(parent)
        .map_err(|error| format!("Could not prepare diff export: {error}"))?;
    temporary
        .write_all(data)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| format!("Could not write diff export: {error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("Could not save diff export: {}", error.error))?;
    Ok(())
}

#[tauri::command]
async fn gui_save_diff_export(request: Request<'_>, window: Window) -> Result<bool, String> {
    let data = match request.body() {
        InvokeBody::Raw(data) => data.clone(),
        InvokeBody::Json(_) => {
            return Err("Diff export requires a binary request body".to_string());
        }
    };
    let filename = request
        .headers()
        .get("x-ovim-export-filename")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "Diff export filename is missing".to_string())?;
    let kind = validate_diff_export(filename, &data)?;
    let destination = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title("Save diff export")
        .set_file_name(filename)
        .add_filter(kind.filter_name(), &[kind.extension()])
        .save_file()
        .await;
    let Some(destination) = destination else {
        return Ok(false);
    };
    let path = destination.path().to_path_buf();
    tokio::task::spawn_blocking(move || save_diff_export_at(&path, &data, kind))
        .await
        .map_err(|error| format!("Diff export task stopped: {error}"))??;
    Ok(true)
}

#[tauri::command]
async fn gui_open_diff_source(
    bridge: State<'_, GuiBridge>,
    pane: usize,
    buffer_id: u64,
    path: String,
    line: usize,
    side: String,
    definition_column: Option<u32>,
) -> Result<(), String> {
    bridge
        .open_diff_source(pane, buffer_id, path, line, side, definition_column)
        .await
}

#[tauri::command]
async fn gui_snapshot(
    bridge: State<'_, GuiBridge>,
    columns: u16,
    rows: u16,
) -> Result<GuiSnapshot, String> {
    bridge.snapshot(columns, rows).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GuiVectorPreview {
    data_url: String,
    width: u32,
    height: u32,
    file_name: String,
}

fn render_vector_preview(source: GuiVectorSource) -> Result<GuiVectorPreview, String> {
    let mut file = tempfile::Builder::new()
        .prefix("ovim-vector-")
        .suffix(".strok")
        .tempfile()
        .map_err(|error| format!("Could not create Strøk preview file: {error}"))?;
    file.write_all(source.source.as_bytes())
        .map_err(|error| format!("Could not write Strøk preview file: {error}"))?;
    let mut child = Command::new("strok")
        .arg("-f")
        .arg(file.path())
        .args(["inspect", "--svg"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "Strøk is not installed or `strok` is not on Ovim's PATH. Install it with `brew install adhvfed/tap/strok`".to_string()
            } else {
                format!("Could not run Strøk: {error}")
            }
        })?;
    let stdout = child.stdout.take().expect("piped Strøk preview stdout");
    let stderr = child.stderr.take().expect("piped Strøk preview stderr");
    let stdout_task = std::thread::spawn(move || read_vector_output(stdout, 16 * 1024 * 1024));
    let stderr_task = std::thread::spawn(move || read_vector_output(stderr, 4 * 1024));
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("Strøk preview timed out after 15 seconds".to_string());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("Could not wait for Strøk preview: {error}"));
            }
        }
    }?;
    let (stdout, stdout_truncated) = stdout_task
        .join()
        .map_err(|_| "Strøk preview output reader stopped".to_string())?;
    let (stderr, _) = stderr_task
        .join()
        .map_err(|_| "Strøk preview error reader stopped".to_string())?;
    if !status.success() {
        let detail = String::from_utf8_lossy(&stderr);
        return Err(if detail.trim().is_empty() {
            "Strøk could not render this document".to_string()
        } else {
            detail.trim().chars().take(4_096).collect()
        });
    }
    if stdout_truncated {
        return Err("Strøk preview SVG exceeds 16 MiB".to_string());
    }
    rasterize_vector_svg(&stdout, source.file_name)
}

fn read_vector_output(mut reader: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut retained = Vec::with_capacity(limit.min(16 * 1024));
    let mut buffer = [0u8; 16 * 1024];
    let mut truncated = false;
    while let Ok(read) = reader.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..read.min(remaining)]);
        truncated |= read > remaining;
    }
    (retained, truncated)
}

fn rasterize_vector_svg(svg: &[u8], file_name: String) -> Result<GuiVectorPreview, String> {
    let svg = std::str::from_utf8(svg)
        .map_err(|_| "Strøk preview output was not UTF-8 SVG".to_string())?;
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default())
        .map_err(|error| format!("Strøk produced invalid SVG: {error}"))?;
    let natural = tree.size();
    if natural.width() <= 0.0 || natural.height() <= 0.0 {
        return Err("Strøk produced an empty vector preview".to_string());
    }
    let scale = (2048.0 / natural.width())
        .min(2048.0 / natural.height())
        .min(2.0);
    let width = (natural.width() * scale).ceil().max(1.0) as u32;
    let height = (natural.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| "Strøk preview dimensions are too large".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let png = pixmap
        .encode_png()
        .map_err(|error| format!("Could not encode Strøk preview: {error}"))?;
    Ok(GuiVectorPreview {
        data_url: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        ),
        width,
        height,
        file_name,
    })
}

#[tauri::command]
async fn gui_vector_preview(bridge: State<'_, GuiBridge>) -> Result<GuiVectorPreview, String> {
    let source = bridge.vector_source().await?;
    tauri::async_runtime::spawn_blocking(move || render_vector_preview(source))
        .await
        .map_err(|error| format!("Strøk preview worker stopped: {error}"))?
}

#[tauri::command]
async fn gui_vector_feedback(bridge: State<'_, GuiBridge>, feedback: String) -> Result<(), String> {
    bridge.vector_feedback(feedback).await
}

/// Attach a coalesced, event-driven snapshot stream to one webview.
#[tauri::command]
async fn gui_subscribe(
    bridge: State<'_, GuiBridge>,
    columns: u16,
    rows: u16,
    on_event: Channel<GuiSnapshot>,
) -> Result<(), String> {
    bridge.snapshot(columns, rows).await?;
    let mut updates = bridge.subscribe();
    if let Some(snapshot) = updates.borrow_and_update().clone() {
        on_event.send(snapshot).map_err(|error| error.to_string())?;
    }

    tauri::async_runtime::spawn(async move {
        while updates.changed().await.is_ok() {
            let update = updates.borrow_and_update().clone();
            let Some(snapshot) = update else { continue };
            if on_event.send(snapshot).is_err() {
                break;
            }
        }
    });
    Ok(())
}

#[tauri::command]
async fn gui_key(bridge: State<'_, GuiBridge>, input: GuiKeyInput) -> Result<(), String> {
    bridge.key(input).await
}

#[tauri::command]
async fn gui_paste(bridge: State<'_, GuiBridge>, text: String) -> Result<(), String> {
    bridge.paste(text).await
}

#[tauri::command]
async fn gui_attach_image(
    request: Request<'_>,
    bridge: State<'_, GuiBridge>,
) -> Result<(), String> {
    let data = match request.body() {
        InvokeBody::Raw(data) if data.len() <= 20 * 1024 * 1024 => data.clone(),
        InvokeBody::Raw(_) => return Err("Clipboard image exceeds the 20 MiB limit".to_string()),
        InvokeBody::Json(_) => {
            return Err("Image upload requires a binary request body".to_string())
        }
    };
    let extension = request
        .headers()
        .get("x-ovim-image-extension")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("png");
    let name = format!("pasted-image.{extension}");
    bridge.attach_image_data(name, data).await
}

#[tauri::command]
async fn gui_position_walkthrough(
    bridge: State<'_, GuiBridge>,
    current: usize,
    rows: usize,
) -> Result<(), String> {
    bridge.position_walkthrough(current, rows).await
}

#[tauri::command]
async fn gui_set_cursor(
    bridge: State<'_, GuiBridge>,
    pane: usize,
    line: usize,
    display_column: usize,
) -> Result<(), String> {
    bridge.set_cursor(pane, line, display_column).await
}

#[tauri::command]
async fn gui_set_chat_input_cursor(
    bridge: State<'_, GuiBridge>,
    offset: usize,
) -> Result<(), String> {
    bridge.set_chat_input_cursor(offset).await
}

#[tauri::command]
async fn gui_open_ai_chat(bridge: State<'_, GuiBridge>) -> Result<(), String> {
    bridge.open_ai_chat().await
}

#[tauri::command]
async fn gui_update_chat_input(
    bridge: State<'_, GuiBridge>,
    expected_input: String,
    expected_cursor: usize,
    input: String,
    cursor: usize,
    action: Option<GuiKeyInput>,
) -> Result<(), String> {
    bridge
        .update_chat_input(expected_input, expected_cursor, input, cursor, action)
        .await
}

#[tauri::command]
async fn gui_set_chat_input_width(
    bridge: State<'_, GuiBridge>,
    columns: usize,
) -> Result<(), String> {
    bridge.set_chat_input_width(columns).await
}

#[tauri::command]
async fn gui_remove_chat_image(bridge: State<'_, GuiBridge>, index: usize) -> Result<(), String> {
    bridge.remove_chat_image(index).await
}

#[tauri::command]
async fn gui_select_ai_profile(
    bridge: State<'_, GuiBridge>,
    profile: String,
    model: Option<String>,
) -> Result<(), String> {
    bridge.select_ai_profile(profile, model).await
}

#[tauri::command]
async fn gui_select_reasoning_effort(
    bridge: State<'_, GuiBridge>,
    effort: String,
) -> Result<(), String> {
    bridge.select_reasoning_effort(effort).await
}

#[tauri::command]
async fn gui_select_permission_mode(
    bridge: State<'_, GuiBridge>,
    mode: String,
) -> Result<(), String> {
    bridge.select_permission_mode(mode).await
}

#[tauri::command]
async fn gui_ai_policy(bridge: State<'_, GuiBridge>, action: String) -> Result<(), String> {
    bridge.ai_policy(action).await
}

#[tauri::command]
async fn gui_editor_command(bridge: State<'_, GuiBridge>, command: String) -> Result<(), String> {
    bridge.editor_command(command).await
}

#[tauri::command]
async fn gui_replay_chat_tool(
    bridge: State<'_, GuiBridge>,
    tool_call_id: String,
) -> Result<(), String> {
    bridge.replay_chat_tool(tool_call_id).await
}

#[tauri::command]
async fn gui_select_chat_message(bridge: State<'_, GuiBridge>, index: usize) -> Result<(), String> {
    bridge.select_chat_message(index).await
}

#[tauri::command]
async fn gui_manage_queued_chat_input(
    bridge: State<'_, GuiBridge>,
    id: u64,
    action: String,
) -> Result<(), String> {
    bridge.manage_queued_chat_input(id, action).await
}

#[tauri::command]
async fn gui_select_chat_agent(
    bridge: State<'_, GuiBridge>,
    agent_id: Option<String>,
) -> Result<(), String> {
    bridge.select_chat_agent(agent_id).await
}

#[tauri::command]
async fn gui_select_tab(bridge: State<'_, GuiBridge>, index: usize) -> Result<(), String> {
    bridge.select_tab(index).await
}

#[tauri::command]
async fn gui_focus_pane(bridge: State<'_, GuiBridge>, index: usize) -> Result<(), String> {
    bridge.focus_pane(index).await
}

#[tauri::command]
async fn gui_select_picker(bridge: State<'_, GuiBridge>, index: usize) -> Result<(), String> {
    bridge.select_picker(index).await
}

#[tauri::command]
async fn gui_select_completion(
    bridge: State<'_, GuiBridge>,
    index: usize,
    activate: bool,
) -> Result<(), String> {
    bridge.select_completion(index, activate).await
}

#[tauri::command]
async fn gui_select_file_tree(
    bridge: State<'_, GuiBridge>,
    index: usize,
    activate: bool,
) -> Result<(), String> {
    bridge.select_file_tree(index, activate).await
}

#[tauri::command]
async fn gui_select_problem(
    bridge: State<'_, GuiBridge>,
    kind: String,
    index: usize,
    activate: bool,
) -> Result<(), String> {
    bridge.select_problem(kind, index, activate).await
}

#[tauri::command]
async fn gui_select_lsp(
    bridge: State<'_, GuiBridge>,
    index: usize,
    activate: bool,
) -> Result<(), String> {
    bridge.select_lsp(index, activate).await
}

#[tauri::command]
async fn gui_search_replace_row(
    bridge: State<'_, GuiBridge>,
    index: usize,
    action: String,
) -> Result<(), String> {
    bridge.search_replace_row(index, action).await
}

#[tauri::command]
async fn gui_debug_panel_row(
    bridge: State<'_, GuiBridge>,
    index: usize,
    action: String,
) -> Result<(), String> {
    bridge.debug_panel_row(index, action).await
}

#[tauri::command]
async fn gui_toggle_breakpoint(bridge: State<'_, GuiBridge>, line: usize) -> Result<(), String> {
    bridge.toggle_breakpoint(line).await
}

#[tauri::command]
async fn gui_toggle_fold(bridge: State<'_, GuiBridge>, line: usize) -> Result<(), String> {
    bridge.toggle_fold(line).await
}

#[tauri::command]
async fn gui_select_debug_frame(bridge: State<'_, GuiBridge>, index: usize) -> Result<(), String> {
    bridge.select_debug_frame(index).await
}

#[tauri::command]
fn gui_window_action(
    window: Window,
    exit_gate: State<'_, GuiExitGate>,
    action: String,
) -> Result<(), String> {
    match action.as_str() {
        "minimize" => window.minimize(),
        "toggle-maximize" => window.is_maximized().and_then(|maximized| {
            if maximized {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }),
        "close" => window.close(),
        "close-approved" => {
            exit_gate.0.store(true, Ordering::SeqCst);
            window.close()
        }
        _ => return Err(format!("Unknown window action: {action}")),
    }
    .map_err(|error| error.to_string())
}

fn validate_external_url(url: &str) -> Result<(), String> {
    let normalized = url.to_ascii_lowercase();
    if url.is_empty()
        || url.trim() != url
        || url.chars().any(char::is_control)
        || !["https://", "http://", "mailto:"]
            .iter()
            .any(|scheme| normalized.starts_with(scheme))
    {
        return Err("Only HTTP(S) and email links can be opened".to_string());
    }
    Ok(())
}

#[tauri::command]
fn gui_open_external(url: String) -> Result<(), String> {
    validate_external_url(&url)?;
    let _ = open::that_in_background(url);
    Ok(())
}

/// Run the native application on the calling thread until its last window closes.
pub fn run(file: Option<FileArg>, resume: bool) -> Result<()> {
    // Keep Tauri's patchable bundle marker linked even without the updater
    // plugin. The bundler uses it to distinguish deb/AppImage/MSI installs.
    std::hint::black_box(tauri::utils::platform::bundle_type());
    crate::lsp_init::set_headless_mode(false);
    let _ = crate::lsp::init_lsp_logging();

    let (browser_client, browser_requests) = ovim_core::browser::browser_channel();
    let browser_host = BrowserHost::new(browser_requests);
    #[cfg(debug_assertions)]
    let browser_smoke_client =
        std::env::var_os("OVIM_BROWSER_SMOKE").map(|_| browser_client.clone());
    let services = ovim_core::editor::EditorServices::default().with_browser(browser_client);
    let bridge = GuiBridge::spawn(file, resume, services)?;
    let shutdown_bridge = bridge.clone();
    let exit_gate = GuiExitGate::default();
    let setup_exit_gate = exit_gate.clone();
    let run_exit_gate = exit_gate.clone();
    let application = tauri::Builder::default()
        .manage(bridge)
        .manage(browser_host)
        .manage(super::terminal::TerminalHost::new())
        .manage(exit_gate)
        .invoke_handler(tauri::generate_handler![
            gui_snapshot,
            gui_diff_action,
            gui_diff_comment,
            gui_save_diff_export,
            gui_open_diff_source,
            gui_vector_preview,
            gui_vector_feedback,
            gui_subscribe,
            gui_key,
            gui_paste,
            gui_attach_image,
            gui_set_cursor,
            gui_position_walkthrough,
            gui_open_ai_chat,
            gui_set_chat_input_cursor,
            gui_update_chat_input,
            gui_set_chat_input_width,
            gui_remove_chat_image,
            gui_select_ai_profile,
            gui_select_reasoning_effort,
            gui_select_permission_mode,
            gui_ai_policy,
            gui_editor_command,
            gui_select_chat_message,
            gui_replay_chat_tool,
            gui_manage_queued_chat_input,
            gui_select_chat_agent,
            gui_select_tab,
            gui_focus_pane,
            gui_select_picker,
            gui_select_completion,
            gui_select_file_tree,
            gui_select_problem,
            gui_search_replace_row,
            gui_select_lsp,
            gui_select_debug_frame,
            gui_debug_panel_row,
            gui_toggle_breakpoint,
            gui_toggle_fold,
            gui_window_action,
            gui_open_external,
            super::terminal::gui_terminal_open,
            super::terminal::gui_terminal_write,
            super::terminal::gui_terminal_ack,
            super::terminal::gui_terminal_resize,
            super::terminal::gui_terminal_close,
            super::menu::gui_set_menu_surface,
            super::browser::gui_browser_open,
            super::browser::gui_browser_state,
            super::browser::gui_browser_subscribe,
            super::browser::gui_browser_set_bounds,
            super::browser::gui_browser_activate,
            super::browser::gui_browser_ack_presentation,
            super::browser::gui_browser_navigate,
            super::browser::gui_browser_toolbar,
            super::browser::gui_browser_close,
            super::browser::gui_browser_set_vim_keys,
        ])
        .setup(move |app| {
            let menu = super::menu::install(app)?;
            app.manage(menu);
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                {
                    if let Err(error) = window.with_webview(|webview| {
                        if let Err(error) = super::mac_arrow_keys::install(webview) {
                            let _ = std::io::stderr().write_all(
                                format!("Failed to install macOS arrow-key handler: {error:#}\n")
                                    .as_bytes(),
                            );
                        }
                    }) {
                        let _ = std::io::stderr().write_all(
                            format!(
                                "Failed to access the main webview for arrow keys: {error:#}\n"
                            )
                            .as_bytes(),
                        );
                    }
                }
                let browser_host = app.state::<BrowserHost>().inner().clone();
                browser_host
                    .attach(window.as_ref().window())
                    .map_err(anyhow::Error::msg)?;
                #[cfg(debug_assertions)]
                if let Some(browser_client) = browser_smoke_client.clone() {
                    let app_handle = app.handle().clone();
                    let smoke_exit_gate = setup_exit_gate.clone();
                    tauri::async_runtime::spawn(async move {
                        let result =
                            super::browser::run_native_browser_smoke(browser_client, browser_host)
                                .await;
                        let (exit_code, message) = match result {
                            Ok(()) => (0, "OVIM_BROWSER_SMOKE_OK\n".to_string()),
                            Err(error) => (1, format!("OVIM_BROWSER_SMOKE_FAILED: {error}\n")),
                        };
                        let _ = std::io::stderr().write_all(message.as_bytes());
                        smoke_exit_gate.0.store(true, Ordering::SeqCst);
                        app_handle.exit(exit_code);
                    });
                }
                window
                    .set_title("Ovim")
                    .context("Failed to set the GUI window title")?;
                let drop_bridge = app.state::<GuiBridge>().inner().clone();
                let close_window = window.clone();
                let close_gate = setup_exit_gate.clone();
                window.on_window_event(move |event| match event {
                    WindowEvent::CloseRequested { api, .. }
                        if !close_gate.0.load(Ordering::SeqCst) =>
                    {
                        api.prevent_close();
                        let _ = close_window.emit("ovim://close-requested", "close");
                    }
                    WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
                        let bridge = drop_bridge.clone();
                        let paths = paths.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = bridge.attach_images(paths).await;
                        });
                    }
                    _ => {}
                });
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            let _ = app.emit_to(
                EventTarget::webview("main"),
                "ovim://menu-action",
                event.id().as_ref(),
            );
        })
        .build(tauri::generate_context!())
        .context("Failed to build the Ovim GUI")?;

    application.run(move |handle, event| match event {
        RunEvent::ExitRequested { api, .. } if !run_exit_gate.0.load(Ordering::SeqCst) => {
            api.prevent_exit();
            if let Some(window) = handle.get_webview_window("main") {
                let _ = window.emit("ovim://close-requested", "quit");
            }
        }
        RunEvent::Exit => {
            handle.state::<super::terminal::TerminalHost>().shutdown();
            shutdown_bridge.shutdown();
        }
        _ => {}
    });
    Ok(())
}

#[cfg(test)]
mod vector_tests {
    use super::*;

    #[test]
    fn vector_svg_is_rasterized_as_a_bounded_png_data_url() {
        let preview = rasterize_vector_svg(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#f00"/></svg>"##,
            "sample.strok".to_string(),
        )
        .unwrap();
        assert_eq!(preview.file_name, "sample.strok");
        assert_eq!((preview.width, preview.height), (80, 40));
        assert!(preview.data_url.starts_with("data:image/png;base64,iVBOR"));
    }
}

#[cfg(test)]
mod tests {
    use super::{save_diff_export_at, validate_diff_export, validate_external_url, DiffExportKind};

    #[test]
    fn external_links_are_limited_to_non_executable_schemes() {
        assert!(validate_external_url("https://example.com/guide").is_ok());
        assert!(validate_external_url("mailto:hello@example.com").is_ok());
        assert!(validate_external_url("javascript:alert(1)").is_err());
        assert!(validate_external_url("file:///etc/passwd").is_err());
        assert!(validate_external_url("https://example.com\nfile:///etc/passwd").is_err());
    }

    #[test]
    fn diff_export_rejects_unsafe_names_and_mismatched_content() {
        let png = b"\x89PNG\r\n\x1a\nimage";
        let zip = b"PK\x03\x04archive";
        assert_eq!(
            validate_diff_export("review.png", png).unwrap(),
            DiffExportKind::Png
        );
        assert_eq!(
            validate_diff_export("review.zip", zip).unwrap(),
            DiffExportKind::Zip
        );
        for unsafe_name in [
            "../review.png",
            "folder/review.png",
            "folder\\review.png",
            ".hidden.png",
            "review:copy.png",
        ] {
            assert!(validate_diff_export(unsafe_name, png).is_err());
        }
        assert!(validate_diff_export("review.zip", png).is_err());
        assert!(validate_diff_export("review.png", zip).is_err());
        assert!(validate_diff_export("review.png", &[]).is_err());
    }

    #[test]
    fn diff_export_accepts_content_above_the_previous_size_limit() {
        let mut data = vec![0; 64 * 1024 * 1024 + 1];
        data[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            validate_diff_export("review.png", &data).unwrap(),
            DiffExportKind::Png
        );
    }

    #[test]
    fn diff_export_writes_only_to_the_selected_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("review.png");
        let data = b"\x89PNG\r\n\x1a\nimage";
        save_diff_export_at(&path, data, DiffExportKind::Png).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        let wrong_extension = directory.path().join("review.zip");
        assert!(save_diff_export_at(&wrong_extension, data, DiffExportKind::Png).is_err());
        assert!(!wrong_extension.exists());
    }
}
