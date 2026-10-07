//! Debug-adapter work driven by the tick: adapter events and the one queued
//! [`PendingDebugAction`] (start, step, evaluate, attach, ...).

use crate::dap::PendingDebugAction;
use crate::editor::Editor;

/// Poll DAP events and auto-fetch stack trace on stop.
pub(super) fn process_dap_events(editor: &mut Editor) {
    // Edits that did not come through a key (API, LSP, Lua) move breakpoints too.
    editor.follow_breakpoints_through_edits();
    let dap_count = editor.process_dap_events();
    if dap_count > 0 {
        crate::log_debug!("tick", "Processed {} DAP events", dap_count);
        editor.mark_dirty();
        if editor.debug_state().stopped_thread.is_some()
            && editor.debug_state().stack_frames.is_empty()
        {
            editor.dap_manager_mut().pending_action = Some(PendingDebugAction::FetchState);
        }
    }
}

/// Dispatch the pending debug action (start, stop, step, evaluate, etc.).
pub(super) async fn process_pending_debug_action(editor: &mut Editor) {
    // Stop outranks everything else queued.
    if editor.dap_manager_mut().take_stop_request() {
        editor.dap_manager_mut().pending_action = None;
        if let Err(e) = editor.stop_debug_session().await {
            editor.set_status_message(format!("Debug stop failed: {e}"));
        }
        editor.mark_dirty();
        return;
    }
    if editor.dap_manager_mut().take_breakpoint_sync_request() && editor.is_debug_active() {
        sync_all_breakpoints(editor).await;
        editor.mark_dirty();
    }
    let Some(action) = editor.dap_manager_mut().pending_action.take() else {
        return;
    };

    match action {
        PendingDebugAction::Start {
            command,
            args,
            attach,
        } => {
            if let Err(e) = editor.start_debug_session(&command, &args, attach).await {
                editor.launch_debug_failed(e.to_string());
            }
            editor.mark_dirty();
        }
        PendingDebugAction::Continue => {
            if let Err(e) = editor.debug_continue().await {
                editor.set_status_message(format!("Debug continue failed: {e}"));
            }
            editor.mark_dirty();
        }
        PendingDebugAction::StepOver => {
            if let Err(e) = editor.debug_step_over().await {
                editor.set_status_message(format!("Debug step failed: {e}"));
            }
            editor.mark_dirty();
        }
        PendingDebugAction::StepIn => {
            if let Err(e) = editor.debug_step_in().await {
                editor.set_status_message(format!("Debug step in failed: {e}"));
            }
            editor.mark_dirty();
        }
        PendingDebugAction::StepOut => {
            if let Err(e) = editor.debug_step_out().await {
                editor.set_status_message(format!("Debug step out failed: {e}"));
            }
            editor.mark_dirty();
        }
        PendingDebugAction::Attach => {
            process_dap_attach(editor).await;
        }
        PendingDebugAction::SyncBreakpoints => {
            sync_all_breakpoints(editor).await;
            match editor.dap_manager_mut().configuration_done().await {
                Ok(()) => editor.launch_debug_started(),
                Err(e) => {
                    let _ = editor.stop_debug_session().await;
                    editor.launch_debug_failed(format!("configurationDone failed: {e}"));
                }
            }
            editor.mark_dirty();
        }
        PendingDebugAction::RefreshWatches => {
            editor.debug_refresh_watches().await;
        }
        PendingDebugAction::EvaluateHover { expression } => {
            let frame_id = editor.selected_frame_id();
            match editor
                .dap_manager()
                .evaluate(&expression, frame_id, Some("hover"))
                .await
            {
                Ok((result, type_, var_ref)) => {
                    let mut text = format!("{expression} = {result}");
                    if let Some(type_) = type_.filter(|t| !t.is_empty()) {
                        text.push_str(&format!("\ntype: {type_}"));
                    }
                    if var_ref > 0 {
                        if let Ok(children) = editor.dap_manager().variables(var_ref).await {
                            text.push('\n');
                            for child in children.iter().take(20) {
                                text.push_str(&format!("\n  {} = {}", child.name, child.value));
                            }
                            if children.len() > 20 {
                                text.push_str(&format!("\n  ... {} more", children.len() - 20));
                            }
                        }
                    }
                    editor.set_hover_info(text);
                }
                Err(e) => editor.set_status_message(format!("{expression}: {e}")),
            }
            editor.mark_dirty();
        }
        PendingDebugAction::FetchState => {
            let _ = editor.debug_fetch_stack_trace().await;
            editor.debug_fetch_exception_info().await;
            editor.debug_fetch_threads().await;
            let _ = editor.debug_fetch_scopes().await;
            let scope_refs: Vec<u64> = editor
                .debug_state()
                .scopes
                .iter()
                .filter(|s| !s.expensive)
                .map(|s| s.variables_reference)
                .collect();
            for var_ref in scope_refs {
                let _ = editor.debug_fetch_variables(var_ref).await;
            }
            let expanded: Vec<u64> = editor.debug_state().expanded_refs.iter().copied().collect();
            for var_ref in expanded {
                let _ = editor.debug_fetch_variables(var_ref).await;
            }
            editor.debug_refresh_watches().await;
            editor.mark_dirty();
        }
        PendingDebugAction::SelectFrame { index: _ } => {
            let _ = editor.debug_fetch_scopes().await;
            let scope_refs: Vec<u64> = editor
                .debug_state()
                .scopes
                .iter()
                .filter(|s| !s.expensive)
                .map(|s| s.variables_reference)
                .collect();
            for var_ref in scope_refs {
                let _ = editor.debug_fetch_variables(var_ref).await;
            }
            editor.debug_refresh_watches().await;
            editor.mark_dirty();
        }
        PendingDebugAction::Evaluate { expression } => {
            let frame_id = editor.selected_frame_id();
            match editor
                .dap_manager()
                .evaluate(&expression, frame_id, Some("repl"))
                .await
            {
                Ok((result, _type, _var_ref)) => {
                    editor.set_status_message(format!("{expression} = {result}"));
                }
                Err(e) => {
                    editor.set_status_message(format!("Eval error: {e}"));
                }
            }
            editor.mark_dirty();
        }
        PendingDebugAction::FetchVariables { var_ref } => {
            let _ = editor.debug_fetch_variables(var_ref).await;
            editor.mark_dirty();
        }
    }
}

/// Send the DAP `attach` request for the session being started.
///
/// Every way of starting a session (F5, `:debug start`, `<Space>dc`, the
/// config picker, test debugging) arrives here: ovim has already started the
/// JVM and the adapter only attaches to it.
async fn process_dap_attach(editor: &mut Editor) {
    let Some(arguments) = editor.dap_manager().attach_request.clone() else {
        // Nothing was asked for; a stray `initialized` event. Do not send
        // `configurationDone` to an adapter that has no debuggee.
        return;
    };
    match editor.dap_manager_mut().attach(arguments).await {
        Ok(()) => {
            editor.dap_manager_mut().pending_action = Some(PendingDebugAction::SyncBreakpoints);
        }
        Err(e) => {
            let _ = editor.stop_debug_session().await;
            editor.launch_debug_failed(format!("attach failed: {e}"));
        }
    }
    editor.mark_dirty();
}

/// Sends every breakpoint file and the exception filters to the adapter.
async fn sync_all_breakpoints(editor: &mut Editor) {
    let paths: Vec<std::path::PathBuf> = editor.debug_state().breakpoints.keys().cloned().collect();
    for path in &paths {
        let _ = editor.debug_sync_breakpoints(path).await;
    }
    let _ = editor.dap_manager().sync_exception_breakpoints().await;
}
