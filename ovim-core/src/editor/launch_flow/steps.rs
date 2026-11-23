//! Running a plan: the build, run and debug-launch child processes, their
//! output, and what happens when each one exits.

use std::time::{Instant, SystemTime};

use super::{last_lines, LaunchJob, Stage};
use crate::editor::Editor;
use crate::launch::console::{LineKind, RunOutcome, RunPhase, RunStatus};
use crate::launch::plan::{self, LaunchMode, LaunchPlan, PlanKind};
use crate::launch::process::{CommandSpec, ExitInfo, ProcEvent, ProcessHandle, StreamKind};

/// Cap on process events handled per tick so a chatty program cannot starve
/// input handling.
const EVENTS_PER_TICK: usize = 2_000;

fn stream_kind(mapping: (LineKind, LineKind), stream: StreamKind) -> LineKind {
    match stream {
        StreamKind::Stdout => mapping.0,
        StreamKind::Stderr => mapping.1,
    }
}

impl Editor {
    // ------------------------------------------------------------------
    // Plan execution
    // ------------------------------------------------------------------

    pub(super) fn begin_plan(&mut self, job: &mut LaunchJob, plan: LaunchPlan) {
        let mode = job.request.mode;
        if let Some(reason) = plan.unsupported_reason(mode) {
            self.fail_job(job, reason);
            return;
        }
        if let Some(run) = self.launch.console.run_mut(job.run_id) {
            run.title = plan.name.clone();
            run.source_roots = plan.source_roots();
            run.cwd = plan.project_root.clone();
        }
        for warning in &plan.warnings {
            self.log_console(job.run_id, LineKind::System, format!("warning: {warning}"));
        }
        job.plan = Some(plan.clone());
        if plan.kind == PlanKind::Test {
            // Test output has its own surface. Retain the shared console
            // record for explicit history/navigation, without a second pane.
            self.close_run_console();
            self.start_test_panel_run(job, &plan);
        }

        match (plan.kind, mode) {
            (PlanKind::Attach, _) => {
                let attach = plan.attach.clone().expect("attach plans carry a target");
                let request = serde_json::json!({
                    "host": attach.host,
                    "port": attach.port,
                    "projectRoot": attach.project_root,
                });
                self.log_console(
                    job.run_id,
                    LineKind::System,
                    format!("Attaching to {}:{}", attach.host, attach.port),
                );
                self.begin_debugger(job, request);
            }
            (PlanKind::Main, _) if plan.build.is_some() => self.start_build(job),
            (PlanKind::Main, LaunchMode::Run) => self.start_run_process(job),
            (PlanKind::Main, LaunchMode::Debug) => self.start_debug_process(job),
            (PlanKind::Test | PlanKind::Task, LaunchMode::Run) => self.start_run_process(job),
            (PlanKind::Test | PlanKind::Task, LaunchMode::Debug) => self.start_debug_task(job),
        }
    }

    pub(super) fn set_phase(&mut self, job: &LaunchJob, phase: RunPhase) {
        if let Some(run) = self.launch.console.run_mut(job.run_id) {
            run.status = RunStatus::Active(phase);
        }
        self.mark_dirty();
    }

    fn spawn_step(
        &mut self,
        job: &mut LaunchJob,
        spec: &CommandSpec,
        phase: RunPhase,
        stage: Stage,
    ) {
        // Only the program itself reads stdin, not builds or test tasks.
        let interactive = matches!(stage, Stage::Running | Stage::AwaitingDebugPort { .. })
            && job.plan.as_ref().is_some_and(|p| p.kind == PlanKind::Main);
        // A shell command is shown as typed, not as its `sh -c` wrapper.
        let shown = match job.shell_run() {
            Some(shell) if stage == Stage::Running => shell.command.clone(),
            _ => spec.display(),
        };
        self.log_console(job.run_id, LineKind::System, format!("$ {shown}"));
        if let Some(run) = self.launch.console.run_mut(job.run_id) {
            run.command = shown;
            run.cwd = spec.cwd.clone();
        }
        let spawned = if interactive {
            ProcessHandle::spawn_interactive(spec)
        } else {
            ProcessHandle::spawn(spec)
        };
        match spawned {
            Ok(proc) => {
                job.proc = Some(proc);
                job.proc_exit = None;
                job.clear_log();
                job.stage = stage;
                self.set_phase(job, phase);
            }
            Err(message) => self.fail_job(job, message),
        }
    }

    fn start_build(&mut self, job: &mut LaunchJob) {
        let Some(build) = job.plan.as_ref().and_then(|p| p.build.clone()) else {
            return;
        };
        self.spawn_step(job, &build, RunPhase::Building, Stage::Building);
    }

    fn start_run_process(&mut self, job: &mut LaunchJob) {
        let Some(plan) = job.plan.clone() else { return };
        let spec = match (plan.kind, &plan.launch, &plan.task) {
            (PlanKind::Main, Some(launch), _) => launch.run_command(),
            (_, _, Some(task)) => CommandSpec {
                argv: task.argv.clone(),
                cwd: task.cwd.clone(),
                env: Default::default(),
            },
            _ => return,
        };
        job.started_wall = SystemTime::now();
        self.spawn_step(job, &spec, RunPhase::Running, Stage::Running);
    }

    /// Debugging a `main`: ovim starts the JVM itself (suspended, listening
    /// on a free port) so the program has the same stdin, output and process
    /// group handling as Run; the adapter then attaches.
    fn start_debug_process(&mut self, job: &mut LaunchJob) {
        let Some(launch) = job.plan.as_ref().and_then(|p| p.launch.clone()) else {
            return;
        };
        job.started_wall = SystemTime::now();
        self.spawn_step(
            job,
            &launch.debug_command(),
            RunPhase::WaitingForDebugger,
            Stage::AwaitingDebugPort {
                deadline: Instant::now() + self.launch.debug_port_timeout,
            },
        );
    }

    fn start_debug_task(&mut self, job: &mut LaunchJob) {
        let Some(task) = job.plan.as_ref().and_then(|p| p.task.clone()) else {
            return;
        };
        let Some(mut argv) = task.debug_argv.clone() else {
            return;
        };
        job.debug_port = plan::surefire_debug_port(&argv);
        if let Some(port) = crate::launch::process::free_port() {
            if let Some(pinned) = plan::pin_surefire_debug_port(&argv, port) {
                argv = pinned;
                job.debug_port = Some(port);
            }
        }
        let spec = CommandSpec {
            argv,
            cwd: task.cwd.clone(),
            env: Default::default(),
        };
        job.started_wall = SystemTime::now();
        self.spawn_step(
            job,
            &spec,
            RunPhase::WaitingForDebugger,
            Stage::AwaitingDebugPort {
                deadline: Instant::now() + self.launch.debug_port_timeout,
            },
        );
    }

    pub(super) fn drain_process(&mut self, job: &mut LaunchJob) -> bool {
        let mapping = match job.stage {
            Stage::Building => (LineKind::Build, LineKind::Stderr),
            _ => (LineKind::Stdout, LineKind::Stderr),
        };
        let mut changed = false;
        let mut exit: Option<ExitInfo> = None;
        for _ in 0..EVENTS_PER_TICK {
            let Some(proc) = job.proc.as_mut() else { break };
            match proc.try_recv() {
                Some(ProcEvent::Line { stream, text }) => {
                    changed = true;
                    job.append_log(stream, &text);
                    if let Some(run) = self.launch.console.run_mut(job.run_id) {
                        run.push_line(stream_kind(mapping, stream), text.clone());
                    }
                    if job.panel_run && job.shell_run().is_some() {
                        // Shell test output is the panel's content (JUnit
                        // runs replace theirs with the parsed reports).
                        if let Some(run) = self.build.test_panel.runs.last_mut() {
                            run.push_line(text.clone());
                        }
                    }
                    if let Stage::AwaitingDebugPort { .. } = job.stage {
                        if let Some(port) = plan::parse_listening_port(&text) {
                            self.attach_to_listening_jvm(job, port);
                        }
                    }
                }
                Some(ProcEvent::Exit(info)) => {
                    exit = Some(info);
                    break;
                }
                None => break,
            }
        }
        if let Some(info) = exit {
            job.proc = None;
            job.proc_exit = Some(info);
            self.on_process_exit(job, info);
            changed = true;
        }
        changed
    }

    fn on_process_exit(&mut self, job: &mut LaunchJob, info: ExitInfo) {
        let code = info.code;
        let code_text = code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        match job.stage {
            Stage::Building => {
                if info.killed {
                    self.log_console(job.run_id, LineKind::System, "Build stopped");
                    self.finish_job(job, RunOutcome::Stopped, code);
                } else if code == Some(0) {
                    self.build_succeeded(job);
                } else {
                    self.log_console(
                        job.run_id,
                        LineKind::System,
                        format!("Build failed (exit {code_text}); launch aborted"),
                    );
                    let summary = self.report_build_problems(job, true);
                    let message = match summary {
                        Some(s) => format!("Build failed: {s}. Launch aborted"),
                        None => format!("Build failed (exit {code_text}). Launch aborted"),
                    };
                    self.set_status_message(message);
                    self.finish_job(job, RunOutcome::BuildFailed, code);
                }
            }
            Stage::Running => {
                if info.killed {
                    self.log_console(job.run_id, LineKind::System, "Stopped");
                    self.finish_job(job, RunOutcome::Stopped, code);
                    return;
                }
                let ok = code == Some(0);
                self.log_console(
                    job.run_id,
                    LineKind::System,
                    format!("Process finished with exit code {code_text}"),
                );
                if self.finish_test_reports(job, ok, code) {
                    job.stage = Stage::Reporting;
                } else {
                    self.finish_run(job, ok, code);
                }
            }
            Stage::AwaitingDebugPort { .. } => {
                if info.killed {
                    self.finish_job(job, RunOutcome::Stopped, code);
                    return;
                }
                let summary = self.report_build_problems(job, true);
                let tail = last_lines(&job.log, 3);
                let message = format!(
                    "The JVM never started listening for a debugger (exit {code_text}){}{}",
                    summary.map(|s| format!(": {s}")).unwrap_or_default(),
                    if tail.is_empty() {
                        String::new()
                    } else {
                        format!(". Last output: {tail}")
                    }
                );
                self.fail_job(job, message);
            }
            Stage::StartingDebugger | Stage::Debugging { .. } => {
                // The build-tool child of a test-debug session ended.
                self.log_console(
                    job.run_id,
                    LineKind::System,
                    format!("Process finished with exit code {code_text}"),
                );
                self.finish_test_reports(job, code == Some(0), code);
            }
            Stage::Resolving | Stage::Reporting => {}
        }
    }

    /// The program of a run ended (and its test results, if any, are in).
    pub(super) fn finish_run(&mut self, job: &mut LaunchJob, ok: bool, code: Option<i32>) {
        let is_make = job.is_make();
        // Shell runs report through their own output parsing (test
        // panel, quickfix), not through build diagnostics.
        if !ok && job.shell_run().is_none() {
            let summary = self.report_build_problems(job, false);
            if let Some(s) = summary {
                self.set_status_message(s);
            }
        }
        let outcome = if ok {
            RunOutcome::Succeeded
        } else {
            RunOutcome::Failed
        };
        self.finish_job(job, outcome, code);
        if is_make {
            self.finish_make(job, ok);
        }
    }

    fn build_succeeded(&mut self, job: &mut LaunchJob) {
        self.log_console(job.run_id, LineKind::System, "Build succeeded");
        // Warnings still go to quickfix (silently, no jump).
        let cwd = job
            .plan
            .as_ref()
            .and_then(|p| p.build.as_ref())
            .map(|b| b.cwd.clone());
        let entries = job.diagnostics(cwd.as_deref());
        if entries.is_empty() {
            self.close_quickfix_window();
            self.set_quickfix_list(Vec::new(), String::new());
        } else {
            self.set_quickfix_list(entries, "build".to_string());
        }
        let Some(plan) = job.plan.clone() else { return };
        job.clear_log();
        match (plan.kind, job.request.mode) {
            (_, LaunchMode::Run) => self.start_run_process(job),
            (_, LaunchMode::Debug) => self.start_debug_process(job),
        }
    }
}
