//! What the debug session task found out, applied to the editor.
//!
//! Requests to the adapter run on the session task (see `dap::session`); the
//! tick polls their answers and lands here. Answers that were asked for about
//! an earlier stop are dropped: the debuggee has moved on since.

use super::*;
use crate::dap::session::{DapResult, ResumeKind};

impl Editor {
    pub(super) fn apply_dap_result(&mut self, result: DapResult) {
        match result {
            DapResult::StartFailed(message) => self.launch_debug_failed(message),
            DapResult::Configured { batches, failure } => {
                self.dap_manager.apply_breakpoint_outcomes(batches);
                match failure {
                    None => self.launch_debug_started(),
                    Some(message) => self.launch_debug_failed(message),
                }
            }
            DapResult::BreakpointsSynced { batches } => {
                self.dap_manager.apply_breakpoint_outcomes(batches);
            }
            DapResult::Resumed { kind, epoch, error } => {
                if let Some(error) = error {
                    if kind == ResumeKind::Continue && self.dap_manager.is_current(epoch) {
                        self.dap_manager.state.is_running = false;
                    }
                    self.set_status_message(format!("{}: {error}", kind.failure_text()));
                }
            }
            DapResult::Frames { epoch, frames } => {
                if !self.dap_manager.is_current(epoch) {
                    return;
                }
                let state = &mut self.dap_manager.state;
                state.stack_frames = frames;
                state.selected_frame = 0;
                state.update_execution_position();
                // Show where the debuggee stopped, even in a file that is not open.
                self.show_frame_source(0);
            }
            DapResult::Exception { epoch, summary } => {
                if !self.dap_manager.is_current(epoch) {
                    return;
                }
                // The stop event's own description is the fallback.
                let summary = summary.or_else(|| self.dap_manager.state.exception.clone());
                if let Some(summary) = summary {
                    self.set_status_message(format!("Exception: {summary}"));
                    self.dap_manager
                        .log_console(format!("Stopped on exception: {summary}"));
                    self.dap_manager.state.exception = Some(summary);
                }
            }
            DapResult::Threads { epoch, threads } => {
                if self.dap_manager.is_current(epoch) {
                    self.dap_manager.state.threads = threads;
                }
            }
            DapResult::Scopes {
                epoch,
                frame_id,
                scopes,
            } => {
                if self.dap_manager.is_current(epoch) && self.selected_frame_id() == Some(frame_id)
                {
                    self.dap_manager.state.scopes = scopes;
                }
            }
            DapResult::Variables {
                epoch,
                reference,
                variables,
            } => {
                if self.dap_manager.is_current(epoch) {
                    self.dap_manager
                        .state
                        .variables
                        .insert(reference, variables);
                }
            }
            DapResult::Watch {
                epoch,
                frame_id,
                expression,
                result,
            } => {
                if !self.dap_manager.is_current(epoch) || self.selected_frame_id() != frame_id {
                    return;
                }
                let Some(watch) = self
                    .dap_manager
                    .state
                    .watches
                    .iter_mut()
                    .find(|w| w.expression == expression)
                else {
                    return;
                };
                match result {
                    Ok((value, type_, var_ref)) => {
                        watch.result = Some(Ok(value));
                        watch.type_ = type_;
                        watch.variables_reference = var_ref;
                    }
                    Err(e) => {
                        watch.result = Some(Err(e));
                        watch.type_ = None;
                        watch.variables_reference = 0;
                    }
                }
            }
            DapResult::Evaluated { expression, result } => match result {
                Ok((value, _type, _var_ref)) => {
                    self.set_status_message(format!("{expression} = {value}"));
                }
                Err(e) => self.set_status_message(format!("Eval error: {e}")),
            },
            DapResult::Hover { expression, result } => match result {
                Ok(text) => self.set_hover_info(text),
                Err(e) => self.set_status_message(format!("{expression}: {e}")),
            },
        }
    }
}
