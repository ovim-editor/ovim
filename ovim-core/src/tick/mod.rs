//! The editor tick: one ordered round of background work that every frontend
//! (TUI, headless, GUI) runs on its timer through [`Editor::tick`].
//!
//! The tick owns every periodic poll the core needs — LSP, DAP, syntax,
//! launch, AI jobs, git, picker loading and result delivery, the external-file
//! check and the rehighlight debounce — so a new background feature is live
//! in every frontend as soon as it is polled here. `tests::every_public_poll_is_driven_by_the_tick`
//! fails when a public `poll_*` on the core is not.
//!
//! Frontends keep only what is theirs: rendering, input, their own timer
//! and what to do with [`TickReport::terminal_request`] (only the TUI owns a
//! real terminal). Deliberately frontend-only today: the TUI's terminal bell
//! for new AI attention (`ai_chat_attention_generation`; the GUI and the
//! headless API project the same generation into their snapshots) and the
//! GUI's diff-review geometry.

mod dap;
mod loading;
mod state;

use std::time::{Duration, Instant};

use crate::buffer::{BufferId, LineHighlights};
use crate::editor::{Editor, PendingShellCommand, PendingTerminalSession};
use crate::mode::Mode;
use crate::syntax::{Language, LanguageRegistry, SyntaxHighlighter};

use loading::{process_picker_results, spawn_file_finder_loading, spawn_picker_preview_loading};
pub use state::TickState;

/// How often a clean buffer is compared against its file on disk.
const EXTERNAL_FILE_CHECK_INTERVAL: Duration = Duration::from_millis(500);

/// Full-buffer rehighlight waits this long after the last edit. The visible
/// lines are rehighlighted immediately after input (`refresh_after_input`),
/// so this only bounds how stale off-screen highlights may get while typing.
/// It used to be a TUI/headless policy while the GUI rehighlighted on every
/// tick; now all frontends share it.
const REHIGHLIGHT_DEBOUNCE: Duration = Duration::from_millis(200);

/// A command that needs the user's real terminal, queued by `:!cmd` or
/// `:terminal`. The core cannot run these; the frontend decides.
#[derive(Debug, PartialEq, Eq)]
pub enum TerminalRequest {
    /// `:!cmd`: run with inherited stdio, then wait for Enter.
    Shell(PendingShellCommand),
    /// `:terminal [cmd]`: an interactive shell (or command) session.
    Session(PendingTerminalSession),
}

impl TerminalRequest {
    /// The ex command the user typed, for status messages.
    pub fn describe(&self) -> String {
        match self {
            Self::Shell(shell) => format!(":!{}", shell.command),
            Self::Session(PendingTerminalSession { command: None }) => ":terminal".to_string(),
            Self::Session(PendingTerminalSession {
                command: Some(command),
            }) => format!(":terminal {command}"),
        }
    }
}

/// What one tick leaves for the frontend. Whether anything changed on screen
/// is not repeated here: the tick marks the editor dirty (`Editor::is_dirty`),
/// which is what every renderer already reads.
#[derive(Debug, Default)]
#[must_use = "a terminal request must be run or declined by the frontend"]
pub struct TickReport {
    /// A queued `:!`/`:terminal` command, taken off the editor. The TUI runs
    /// it with the real terminal; the GUI and headless frontends decline it
    /// with a status message. At most one per tick, shell commands first.
    pub terminal_request: Option<TerminalRequest>,
}

impl Editor {
    /// Drive one round of background work. Call on a periodic interval
    /// (the TUI uses 16ms, headless and GUI 50ms) with the frontend's one
    /// [`TickState`].
    pub async fn tick(&mut self, state: &mut TickState) -> TickReport {
        self.tick_at(state, Instant::now()).await
    }

    /// [`Editor::tick`] with the clock injected, so the cadence policies can
    /// be tested without sleeping.
    pub(crate) async fn tick_at(&mut self, state: &mut TickState, now: Instant) -> TickReport {
        let editor = self;
        // Do not let a slow LSP initialization trap a yank flash on screen. Keep
        // deferring LSP while the flash is visible and for the tick that clears
        // it, giving the frontend one complete tick to paint the clear frame.
        let defer_lsp_for_yank_flash = process_yank_flash(editor);

        // Syntax must get a complete tick before LSP startup. Starting a language
        // server can take several seconds, and awaiting it first used to leave a
        // newly opened file unhighlighted for the entire startup window.
        let defer_lsp_init = process_syntax_highlighting(editor, state) || defer_lsp_for_yank_flash;

        // === LSP lifecycle ===
        process_lsp_notifications(editor).await;
        state.lsp_startup.poll(editor).await;
        if !defer_lsp_init {
            process_lsp_init(editor, state);
        }
        process_lsp_sync_and_inlay_hints(editor).await;

        // === Debug adapter ===
        dap::process_dap_events(editor);
        dap::process_pending_debug_action(editor);
        // Build / run / debug launch state machine (non-blocking: child output
        // and language-server answers are polled, never awaited).
        if editor.poll_launch() {
            editor.mark_dirty();
        }
        // Code lenses: request once edits settle, apply when the server answers.
        editor.request_code_lens_if_needed().await;
        if editor.poll_code_lens() {
            editor.mark_dirty();
        }

        // === LSP responses & intents ===
        if editor.poll_pending_lsp_responses() {
            editor.mark_dirty();
        }
        editor.dispatch_pending_intents().await;

        // === Background tasks ===
        poll_background_tasks(editor).await;

        // === Transient UI state ===
        tick_transient_ui(editor);

        // === Lua ===
        let _ = editor.process_lua_commands();

        // === LSP installs ===
        crate::lsp_init::spawn_pending_installs(editor);
        if editor.poll_install_progress() {
            editor.mark_dirty();
        }

        // === Picker ===
        if editor.mode() == Mode::Picker {
            process_picker_tick(editor, state);
        }

        // File switches queue didClose outside the async input dispatcher. Drive
        // that lifecycle from the shared tick so every frontend agrees.
        editor.send_lsp_close_if_needed().await;

        // === Deliver background results and run the cadenced checks ===
        process_picker_results(editor, state);
        if now.duration_since(state.last_external_file_check) >= EXTERNAL_FILE_CHECK_INTERVAL {
            process_external_file_change(editor);
            state.last_external_file_check = now;
        }
        process_debounced_rehighlight(editor, state, now).await;

        TickReport {
            terminal_request: take_terminal_request(editor),
        }
    }
}

fn take_terminal_request(editor: &mut Editor) -> Option<TerminalRequest> {
    if let Some(shell) = editor.take_pending_shell_command() {
        return Some(TerminalRequest::Shell(shell));
    }
    editor
        .take_pending_terminal_session()
        .map(TerminalRequest::Session)
}

/// Rebuild the full highlight cache once the buffer has been left alone for
/// [`REHIGHLIGHT_DEBOUNCE`]. An edit is observed as a change in the current
/// buffer's (id, version), so frontends need no edit bookkeeping of their own.
async fn process_debounced_rehighlight(editor: &mut Editor, state: &mut TickState, now: Instant) {
    let seen = (editor.buffer().id(), editor.buffer().version());
    if state.edit_seen != Some(seen) {
        state.edit_seen = Some(seen);
        state.last_edit = now;
    }
    if editor.buffer().needs_rehighlight()
        && now.duration_since(state.last_edit) >= REHIGHLIGHT_DEBOUNCE
    {
        editor.process_pending_rehighlight().await;
    }
}

/// Reload a clean buffer after an external write, but never discard local
/// edits. The tick runs this every [`EXTERNAL_FILE_CHECK_INTERVAL`]; the TUI
/// also runs it on terminal focus.
pub fn process_external_file_change(editor: &mut Editor) {
    // Buffers in other windows/tabs follow the same autoread rule.
    if !editor
        .reload_background_buffers_changed_on_disk()
        .is_empty()
    {
        editor.mark_dirty();
    }
    match editor.buffer().check_external_modification() {
        Ok(false) | Err(_) => {}
        Ok(true) if editor.is_modified() => {
            let status = "File changed on disk; local changes were kept (use :e! to reload)";
            if editor.status_message() != status {
                editor.set_status_message(status);
                editor.mark_dirty();
            }
        }
        Ok(true) => match editor.buffer_mut().reload_if_changed_sync() {
            Ok(true) => {
                editor.mark_saved();
                editor.mark_buffer_modified_force_send();
                // The on-disk file IS the new buffer content, so a didSave is
                // truthful here — and servers that run flycheck on save
                // (rust-analyzer's cargo/clippy check) need it to produce
                // their full diagnostics for the reloaded text. (OV-00324)
                editor.mark_buffer_saved();
                editor.request_diagnostics_refresh();
                if editor.buffer().needs_rehighlight() {
                    editor.process_viewport_rehighlight();
                }
                editor.set_status_message("File reloaded after external change");
                editor.mark_dirty();
            }
            Ok(false) => {}
            Err(error) => {
                editor.set_status_message(format!("External file change: {error}"));
                editor.mark_dirty();
            }
        },
    }
}

/// Start or finish initial syntax work and report whether LSP initialization
/// should wait until a later tick. The extra tick lets the frontend paint the
/// completed syntax cache before a slow language-server startup is awaited.
fn process_syntax_highlighting(editor: &mut Editor, state: &mut TickState) -> bool {
    let defer_lsp_init =
        editor.buffer().should_init_syntax() || editor.buffer().syntax_highlighting_is_loading();
    spawn_syntax_highlighting(editor, &state.syntax_tx);
    drain_syntax_results(editor, &mut state.syntax_rx);
    defer_lsp_init
}

/// Expire the yank flash without allowing slow LSP startup to delay the frame
/// that removes it. Returns true while LSP initialization should be deferred.
fn process_yank_flash(editor: &mut Editor) -> bool {
    let expired = editor.tick_yank_flash();
    if expired {
        editor.mark_dirty();
    }
    expired || editor.yank_flash().is_some()
}

fn tick_transient_ui(editor: &mut Editor) {
    if editor.tick_cat_animation()
        | editor.tick_toasts()
        | editor.tick_ai_chat_working_animation()
        | editor.tick_ai_chat_text_selection_autoscroll()
        | editor.poll_ai_subagent_repaint()
    {
        editor.mark_dirty();
    }
}

/// Spawn background syntax highlighting if the buffer needs it.
fn spawn_syntax_highlighting(
    editor: &mut Editor,
    syntax_tx: &tokio::sync::mpsc::Sender<(BufferId, Language, Option<LineHighlights>, u64)>,
) {
    if !editor.buffer().should_init_syntax() {
        return;
    }
    let buf = editor.buffer();
    let buffer_id = buf.id();
    let source = buf.rope().to_string();
    let version = buf.highlight_version();
    if let Some(path) = buf.file_path() {
        if let Some(lang) = LanguageRegistry::detect_from_path(path) {
            editor.buffer_mut().mark_syntax_loading();
            let tx = syntax_tx.clone();
            tokio::task::spawn_blocking(move || {
                let highlights = if let Ok(mut h) = SyntaxHighlighter::new(lang) {
                    h.parse(&source);
                    Some(h.highlights_for_all_lines(&source))
                } else {
                    None
                };
                let _ = tx.blocking_send((buffer_id, lang, highlights, version));
            });
        } else if buf
            .language_catalog()
            .detect(path)
            .and_then(|language| language.syntax.clone())
            .is_some()
        {
            // Plugin parsers are already validated at startup. Keep the v1
            // handoff simple and initialize them on first display.
            editor.buffer_mut().enable_syntax_highlighting();
        }
    }
}

/// Drain completed background syntax results into buffers.
fn drain_syntax_results(
    editor: &mut Editor,
    syntax_rx: &mut tokio::sync::mpsc::Receiver<(BufferId, Language, Option<LineHighlights>, u64)>,
) {
    while let Ok((buffer_id, lang, highlights, version)) = syntax_rx.try_recv() {
        let is_current = editor.buffer().id() == buffer_id;
        if let Some(buffer) = editor.get_buffer_by_id_mut(buffer_id) {
            let applied = if let Some(highlights) = highlights {
                buffer.apply_background_syntax(lang, highlights, version)
            } else {
                buffer.clear_syntax_loading();
                false
            };

            if is_current && applied {
                editor.mark_dirty();
            }
        }
    }
}

/// Process LSP notifications and server-initiated workspace edits.
async fn process_lsp_notifications(editor: &mut Editor) {
    if let Some(lsp_manager) = editor.lsp_manager() {
        let notification_count = lsp_manager.process_notifications().await;
        let flush_count = lsp_manager.process_flush_requests().await;

        if notification_count > 0 || flush_count > 0 {
            crate::log_debug!(
                "tick",
                "LSP: {} notifications, {} flushes",
                notification_count,
                flush_count
            );
            editor.mark_dirty();
        }

        let pending_edits = lsp_manager.poll_pending_workspace_edits().await;
        for pending in pending_edits {
            crate::log_debug!("tick", "Applying workspace edit from LSP server");
            // The server hears the real outcome, not that the edit was queued.
            pending.resolve(|workspace_edit| {
                let response = editor.apply_workspace_edit_reporting(workspace_edit);
                match &response.failure_reason {
                    None => editor.set_lsp_status("Applied workspace edit".to_string()),
                    Some(reason) => {
                        crate::log_error!("tick", "Failed to apply workspace edit: {}", reason);
                        editor.set_lsp_status(format!("Failed to apply edit: {reason}"));
                    }
                }
                response
            });
            editor.mark_dirty();
        }
    }
}

/// Initialize LSP for a newly opened file if needed.
fn process_lsp_init(editor: &mut Editor, state: &mut TickState) {
    if let Some(approved) = editor.take_approved_lsp_install() {
        let approval = match approved.companion_id {
            Some(id) => crate::lsp_init::InstallApproval::Companion(id),
            None => crate::lsp_init::InstallApproval::Server,
        };
        state
            .lsp_startup
            .start(editor, &approved.file_path, approval);
    }
    if let Some(file_path) = editor.needs_lsp_init() {
        crate::log_debug!("tick", "Initializing LSP for {}", file_path);
        state
            .lsp_startup
            .start(editor, &file_path, crate::lsp_init::InstallApproval::None);
        editor.clear_lsp_init_flag();
    }
}

/// Sync edits to the LSP server, refresh diagnostics, and poll inlay hints.
/// Colocated to enforce: server always has latest content before we check for fresh diagnostics.
async fn process_lsp_sync_and_inlay_hints(editor: &mut Editor) {
    if editor.sync_lsp_and_refresh_diagnostics().await {
        editor.mark_dirty();
    }
    if let Some(_lsp_manager) = editor.lsp_manager() {
        if editor.poll_pending_inlay_hint_response() {
            editor.mark_dirty();
        }
        if editor.inlay_hints_refresh_needed() {
            editor.request_inlay_hints_refresh().await;
        }
    }
}

/// Poll all independent background tasks (AI, make, git, chat, workflows).
async fn poll_background_tasks(editor: &mut Editor) {
    if let Some(url) = editor.take_pending_external_url() {
        let _ = open::that_in_background(&url);
    }
    if editor.poll_pending_codex_auth() {
        editor.mark_dirty();
    }
    if editor.poll_search_replace() {
        editor.mark_dirty();
    }
    editor.track_recent_file();
    editor.request_outline_if_needed().await;
    if editor.poll_outline() {
        editor.mark_dirty();
    }
    if editor.poll_git_refresh() {
        editor.mark_dirty();
    }
    if editor.poll_git_fetch() {
        editor.mark_dirty();
    }
    if editor.poll_git_commit() {
        editor.mark_dirty();
    }
    if editor.poll_git_history() {
        editor.mark_dirty();
    }
    // The side-by-side diff review is laid out to a fixed width, so it has to
    // re-flow when the window changes size.
    if editor.relayout_diff_review() {
        editor.mark_dirty();
    }
    if editor.poll_pending_ai_chat_job() {
        editor.mark_dirty();
    }
    if editor.poll_pending_workflow_jobs() {
        editor.mark_dirty();
    }
}

/// Drive the picker: nucleo matching, grep drain, debounced filter, preview/file loading.
fn process_picker_tick(editor: &mut Editor, state: &mut TickState) {
    let mut picker_changed = false;
    if let Some(picker) = editor.picker_mut() {
        if picker.tick() {
            picker_changed = true;
        }
        if picker.drain_grep_results() {
            picker_changed = true;
        }
    }
    if picker_changed {
        editor.mark_dirty();
    }
    if editor.apply_pending_picker_filter(50) {
        editor.mark_dirty();
    }
    spawn_picker_preview_loading(editor, &state.preview_tx);
    spawn_file_finder_loading(editor, &state.file_tx, &state.file_list_cache_tx);
    if editor.picker_rapid_scrolling_just_stopped() {
        editor.mark_dirty();
    }
}

#[cfg(test)]
mod tests;
