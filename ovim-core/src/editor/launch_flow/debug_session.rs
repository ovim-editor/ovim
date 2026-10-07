//! The debugger half of a launch: starting the adapter, attaching to a JVM that
//! is waiting for one, and following the session until it ends.

use std::time::{Duration, Instant};

use super::{last_lines, LaunchJob, LaunchSource, Stage};
use crate::dap::PendingDebugAction;
use crate::editor::Editor;
use crate::launch::console::{LineKind, RunOutcome, RunPhase, RunStatus};
use crate::launch::plan::{LaunchMode, PlanKind};

/// After a debug session ends, how long a still-running build-tool child may
/// linger before it is killed.
const POST_SESSION_GRACE: Duration = Duration::from_secs(15);

impl Editor {
    /// Queues the debug adapter start for `request`.
    pub(super) fn begin_debugger(&mut self, job: &mut LaunchJob, attach: serde_json::Value) {
        let (command, args) = match self.debug_adapter_for(job) {
            Ok(found) => found,
            Err(message) => {
                self.fail_job(job, message);
                return;
            }
        };
        self.log_console(
            job.run_id,
            LineKind::System,
            format!("Starting debug adapter: {command} {}", args.join(" ")),
        );
        job.stage = Stage::StartingDebugger;
        self.set_phase(job, RunPhase::Debugging);
        // A JVM the user attached to is theirs to keep; the ones ovim started
        // are killed with the session.
        let attached = job
            .plan
            .as_ref()
            .is_some_and(|p| p.kind == PlanKind::Attach);
        self.dap_manager.queue(PendingDebugAction::Start {
            command,
            args,
            attach,
            terminate_debuggee: !attached,
        });
    }

    fn debug_adapter_for(&self, job: &LaunchJob) -> Result<(String, Vec<String>), String> {
        if let Some(custom) = &job.request.adapter {
            return Ok(custom.clone());
        }
        let language = job
            .plan
            .as_ref()
            .and_then(|p| p.language.clone())
            .or_else(|| match &job.request.source {
                LaunchSource::Cursor { language_id, .. } => Some(language_id.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "java".to_string());
        let registry = crate::language_config::LanguageRegistry::try_get()
            .ok_or("language registry unavailable")?;
        let config = registry
            .get_by_id(&language)
            .and_then(|l| l.dap.as_ref())
            .ok_or_else(|| {
                format!("No debug adapter is configured for {language}. Use :debug start <command> [args...]")
            })?;
        match crate::language_config::find_dap_command(config) {
            Some(command) => Ok((command, config.args.clone())),
            None => Err(format!(
                "Debug adapter '{}' not found. {}",
                config.command,
                config
                    .install_hint
                    .as_deref()
                    .unwrap_or("Install it and make sure it is on PATH")
            )),
        }
    }

    // ------------------------------------------------------------------
    // Debug session hooks (called from the tick's async DAP handling)
    // ------------------------------------------------------------------

    /// The adapter accepted launch/attach and breakpoints are synced.
    pub fn launch_debug_started(&mut self) {
        if let Some(mut job) = self.launch.job.take() {
            if matches!(job.stage, Stage::StartingDebugger) {
                job.stage = Stage::Debugging {
                    session_ended: None,
                };
                self.log_console(job.run_id, LineKind::System, "Debugger attached");
                self.set_phase(&job, RunPhase::Debugging);
            }
            self.launch.job = Some(job);
        }
        self.clear_status_message();
    }

    /// Starting the adapter, launching, or attaching failed. The session is
    /// torn down completely (adapter killed, state reset).
    pub fn launch_debug_failed(&mut self, message: String) {
        self.dap_manager.abort_session();
        if let Some(mut job) = self.launch.job.take() {
            self.fail_job(&mut job, format!("Debug failed: {message}"));
        } else {
            self.set_status_message(format!("Debug failed: {message}"));
        }
    }

    /// A session ended with no job to attribute it to (manual adapter start).
    pub(super) fn acknowledge_orphan_session_end(&mut self) -> bool {
        self.dap_manager.take_session_end().is_some()
    }

    pub(super) fn attach_to_listening_jvm(&mut self, job: &mut LaunchJob, port: u16) {
        let root = job
            .plan
            .as_ref()
            .map(|p| p.project_root.clone())
            .unwrap_or_default();
        self.log_console(
            job.run_id,
            LineKind::System,
            format!("JVM is listening on port {port}; attaching debugger"),
        );
        self.begin_debugger(
            job,
            serde_json::json!({
                "host": "127.0.0.1",
                "port": port,
                "projectRoot": root,
            }),
        );
    }

    pub(super) fn poll_debug_state(&mut self, job: &mut LaunchJob) -> bool {
        let mut changed = false;
        if let Stage::AwaitingDebugPort { deadline } = job.stage {
            if Instant::now() >= deadline {
                if let Some(proc) = job.proc.as_mut() {
                    proc.kill();
                }
                let tail = last_lines(&job.log, 3);
                let message = format!(
                    "Timed out after {} waiting for the JVM to listen for a debugger. Last output: {}",
                    crate::editor::format_duration(self.launch.debug_port_timeout),
                    if tail.is_empty() { "(none)".to_string() } else { tail }
                );
                self.fail_job(job, message);
                return true;
            }
        }
        if let (Stage::AwaitingDebugPort { .. }, Some(port)) = (job.stage, job.debug_port) {
            if crate::launch::process::port_is_listening(port) {
                self.attach_to_listening_jvm(job, port);
                return true;
            }
        }
        if !matches!(job.stage, Stage::StartingDebugger | Stage::Debugging { .. }) {
            return changed;
        }
        if let Some(end) = self.dap_manager.take_session_end() {
            job.session_end = Some(end);
            job.stage = Stage::Debugging {
                session_ended: Some(Instant::now()),
            };
            changed = true;
        }
        if let Stage::Debugging {
            session_ended: Some(at),
        } = job.stage
        {
            let child_running = job.proc.is_some() || job.reports.is_some();
            let crashed = job
                .session_end
                .as_ref()
                .is_some_and(|end| end.adapter_crash.is_some());
            // A JVM that lost its debugger to a crash has nobody left to
            // drive it (it would run on, suspended or unobserved).
            if child_running && (crashed || at.elapsed() > POST_SESSION_GRACE) {
                if let Some(proc) = job.proc.as_mut() {
                    proc.kill();
                }
            }
            if !child_running {
                let end = job.session_end.clone().unwrap_or(crate::dap::SessionEnd {
                    exit_code: None,
                    adapter_crash: None,
                });
                if let (Some(detail), false) = (&end.adapter_crash, job.stopping) {
                    let message = format!("The debug adapter crashed ({detail})");
                    self.fail_job(job, message);
                    return true;
                }
                let exit = end.exit_code.or(job.proc_exit.and_then(|p| p.code));
                let outcome = if job.stopping {
                    RunOutcome::Stopped
                } else {
                    match exit {
                        Some(0) => RunOutcome::Succeeded,
                        Some(_) => RunOutcome::Failed,
                        None => RunOutcome::Ended,
                    }
                };
                let text = match (job.stopping, exit) {
                    (true, _) => "Debug session stopped".to_string(),
                    (false, Some(code)) => format!("Debug session ended (exit code {code})"),
                    (false, None) => "Debug session ended".to_string(),
                };
                self.log_console(job.run_id, LineKind::System, text);
                self.finish_job(job, outcome, exit);
                changed = true;
            }
        }
        changed
    }

    /// Debuggee/adapter output goes into the run console.
    pub(super) fn ingest_debug_output(&mut self) -> bool {
        let output = self.dap_manager.take_console_output();
        if output.is_empty() {
            return false;
        }
        let run_id = match self.launch.job.as_ref() {
            Some(job) => job.run_id,
            None => {
                // A session started by hand: give it a record of its own.
                match self
                    .launch
                    .console
                    .runs
                    .last()
                    .filter(|r| r.status.is_active())
                {
                    Some(run) => run.id,
                    None => {
                        let cwd = std::env::current_dir().unwrap_or_default();
                        let id = self.launch.console.start_run(
                            "Debug session".to_string(),
                            LaunchMode::Debug,
                            cwd.clone(),
                        );
                        if let Some(run) = self.launch.console.run_mut(id) {
                            run.source_roots = vec![cwd];
                            run.status = RunStatus::Active(RunPhase::Debugging);
                        }
                        id
                    }
                }
            }
        };
        if let Some(run) = self.launch.console.run_mut(run_id) {
            for (category, text) in output {
                let kind = match category.as_str() {
                    "stdout" => LineKind::Stdout,
                    "stderr" => LineKind::Stderr,
                    _ => LineKind::Debugger,
                };
                run.push_chunk(kind, &text);
            }
        }
        true
    }
}
