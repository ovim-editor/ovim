//! Run / debug launch flow: the editor-side state machine.
//!
//! One launch request moves through these stages, none of which ever block
//! the tick loop (everything slow happens in tasks or child processes that
//! are polled with `try_recv`):
//!
//! ```text
//! Resolving --> Building --> Running            (Run)
//!                        \-> StartingDebugger --> Debugging          (Debug, main)
//! Resolving --> Running                          (Run tests / tasks)
//! Resolving --> AwaitingDebugPort --> StartingDebugger --> Debugging (Debug tests / tasks)
//! ```
//!
//! *Resolving* asks the language server what to launch at the cursor
//! (`hyperion.resolveLaunch`), falling back to `.ovim/debug.toml` and
//! `hyperion.runConfigurations` when it has no answer. Every source ends up
//! as a [`LaunchPlan`]. Output of every stage streams into the run console.

mod console_nav;
mod debug_session;
mod reports;
mod resolve;
mod steps;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use self::resolve::{ConfigLookup, PendingPick};
use super::Editor;
use crate::debug_config::DebugRunConfig;
use crate::launch::console::{LineKind, RunOutcome};
use crate::launch::lsp::{self, ResolveOutcome, ResolveResult};
use crate::launch::plan::{self, LaunchMode, LaunchPlan, PlanKind};
use crate::launch::process::{ExitInfo, ProcessHandle, StreamKind};
use crate::launch::test_report::TestReport;

/// How long to wait for a `--debug-jvm` JVM to start listening.
const DEBUG_PORT_TIMEOUT: Duration = Duration::from_secs(180);
/// Tail of build/test output kept for diagnostic parsing.
const LOG_CAP: usize = 1 << 20;

/// Where a launch request gets its plan from.
#[derive(Debug, Clone)]
pub enum LaunchSource {
    /// Whatever is runnable at a position in a file.
    Cursor {
        file: PathBuf,
        language_id: String,
        /// LSP position (0-based line, UTF-16 character).
        line: u32,
        character: u32,
        /// `auto`, `main` or `test`.
        target: String,
        project_root: PathBuf,
        /// Composed locally (test discovery + build-tool filter); used when
        /// the server cannot resolve the position, or the reason it cannot.
        fallback: Result<Box<LaunchPlan>, String>,
    },
    /// A ready-made plan (e.g. the whole test suite composed locally).
    Plan {
        plan: Box<LaunchPlan>,
        project_root: PathBuf,
    },
    /// A configuration from `.ovim/debug.toml` or `hyperion.runConfigurations`.
    Config {
        config: Box<DebugRunConfig>,
        project_root: PathBuf,
    },
}

/// Something that can be started and re-started.
#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub mode: LaunchMode,
    pub source: LaunchSource,
    /// Debug adapter to use instead of the language's configured one
    /// (`:debug start <cmd> [args...]`).
    pub adapter: Option<(String, Vec<String>)>,
}

impl LaunchRequest {
    /// Runs `command` through the shell in `cwd` (see [`LaunchPlan::shell`]):
    /// `kind` is [`PlanKind::Test`] for the test panel or [`PlanKind::Task`]
    /// for `:make`.
    pub(crate) fn shell(kind: PlanKind, label: &'static str, command: &str, cwd: PathBuf) -> Self {
        Self {
            mode: LaunchMode::Run,
            source: LaunchSource::Plan {
                project_root: cwd.clone(),
                plan: Box::new(LaunchPlan::shell(kind, label, command, cwd)),
            },
            adapter: None,
        }
    }

    fn project_root(&self) -> &Path {
        match &self.source {
            LaunchSource::Cursor { project_root, .. }
            | LaunchSource::Config { project_root, .. }
            | LaunchSource::Plan { project_root, .. } => project_root,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Resolving,
    Building,
    Running,
    AwaitingDebugPort {
        deadline: Instant,
    },
    StartingDebugger,
    Debugging {
        session_ended: Option<Instant>,
    },
    /// The process is done; the run ends once its test report is read.
    Reporting,
}

/// A test report being read in the background.
struct PendingReports {
    rx: oneshot::Receiver<TestReport>,
    /// Whether the process exited successfully.
    exit_ok: bool,
    exit_code: Option<i32>,
}

struct LaunchJob {
    run_id: u64,
    request: LaunchRequest,
    plan: Option<LaunchPlan>,
    stage: Stage,
    resolve: Option<(oneshot::Receiver<ResolveResult>, JoinHandle<()>)>,
    proc: Option<ProcessHandle>,
    proc_exit: Option<ExitInfo>,
    reports: Option<PendingReports>,
    /// Tail of process output in arrival order (for summaries and tails).
    log: String,
    /// The same output per pipe. stdout and stderr are read concurrently, so
    /// arrival order can split a multi-line diagnostic (javac's snippet and
    /// caret lines) around an unrelated line from the other pipe; diagnostics
    /// are therefore parsed from each pipe on its own.
    log_stdout: String,
    log_stderr: String,
    started_wall: SystemTime,
    stopping: bool,
    /// The debug session ended (or was stopped) while a child still ran.
    session_end: Option<crate::dap::SessionEnd>,
    /// A test run shown in the test panel that has not been finalised yet.
    panel_run: bool,
    /// While waiting for a debug JVM that will not print its port (Maven
    /// surefire): the port to watch.
    debug_port: Option<u16>,
}

fn append_capped(log: &mut String, text: &str) {
    log.push_str(text);
    log.push('\n');
    if log.len() > LOG_CAP {
        let mut cut = log.len() - LOG_CAP / 2;
        while !log.is_char_boundary(cut) {
            cut += 1;
        }
        log.drain(..cut);
    }
}

impl LaunchJob {
    fn append_log(&mut self, stream: StreamKind, text: &str) {
        append_capped(&mut self.log, text);
        match stream {
            StreamKind::Stdout => append_capped(&mut self.log_stdout, text),
            StreamKind::Stderr => append_capped(&mut self.log_stderr, text),
        }
    }

    /// The shell command line behind this job, for `<Space>t*` runs of
    /// non-JVM languages and `:make`.
    fn shell_run(&self) -> Option<&plan::ShellRun> {
        self.plan.as_ref()?.task.as_ref()?.shell.as_ref()
    }

    /// `:make`: a shell task whose diagnostics go to the quickfix list.
    fn is_make(&self) -> bool {
        self.shell_run().is_some() && self.plan.as_ref().is_some_and(|p| p.kind == PlanKind::Task)
    }

    fn clear_log(&mut self) {
        self.log.clear();
        self.log_stdout.clear();
        self.log_stderr.clear();
    }

    /// Compiler diagnostics from both pipes, de-duplicated, errors first
    /// within each pipe's own order.
    fn diagnostics(&self, cwd: Option<&Path>) -> Vec<crate::editor::QuickfixEntry> {
        let mut entries = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for log in [&self.log_stderr, &self.log_stdout] {
            for entry in crate::launch::diagnostics::parse_compiler_output_in(log, cwd) {
                let key = (
                    entry.filename.clone(),
                    entry.lnum,
                    entry.col,
                    entry.entry_type as u8,
                    entry.text.clone(),
                );
                if seen.insert(key) {
                    entries.push(entry);
                }
            }
        }
        entries
    }
}

/// All launch-related editor state.
pub(crate) struct LaunchState {
    pub(crate) console: crate::launch::RunConsoleState,
    job: Option<LaunchJob>,
    last_request: Option<LaunchRequest>,
    /// A request waiting for the current job to stop (rerun replaces).
    queued: Option<LaunchRequest>,
    pending_pick: Option<PendingPick>,
    config_lookup: Option<ConfigLookup>,
    debug_port_timeout: Duration,
    pub(crate) code_lens: super::code_lens::CodeLensState,
    /// A stack frame whose source is not under the workspace, being looked up
    /// through the language server (`workspace/symbol`): the file and 1-based
    /// line to open, or a message.
    pub(crate) frame_lookup: Option<oneshot::Receiver<Result<(PathBuf, usize), String>>>,
    /// `:LspExec` in flight: (command, result).
    pub(crate) server_command:
        Option<oneshot::Receiver<(String, Result<serde_json::Value, String>)>>,
}

impl Default for LaunchState {
    fn default() -> Self {
        Self {
            console: Default::default(),
            job: None,
            last_request: None,
            queued: None,
            pending_pick: None,
            config_lookup: None,
            debug_port_timeout: DEBUG_PORT_TIMEOUT,
            code_lens: Default::default(),
            frame_lookup: None,
            server_command: None,
        }
    }
}

impl Editor {
    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    /// The run console (panel state and all retained runs).
    pub fn run_console(&self) -> &crate::launch::RunConsoleState {
        &self.launch.console
    }

    pub fn run_console_mut(&mut self) -> &mut crate::launch::RunConsoleState {
        &mut self.launch.console
    }

    /// Whether a launch (resolve, build, run, or debug session) is in flight.
    pub fn is_launch_active(&self) -> bool {
        self.launch.job.is_some()
    }

    /// Overrides how long a `--debug-jvm` launch waits for the JVM to listen
    /// (default 3 minutes). Exists for tests and slow build machines.
    #[doc(hidden)]
    pub fn set_debug_port_timeout(&mut self, timeout: Duration) {
        self.launch.debug_port_timeout = timeout;
    }

    /// The request `:RunLast` would repeat.
    pub fn last_launch_request(&self) -> Option<&LaunchRequest> {
        self.launch.last_request.as_ref()
    }

    // ------------------------------------------------------------------
    // Entry points (called from keys and commands; never block)
    // ------------------------------------------------------------------

    /// Run or debug whatever is at the cursor (`target: auto`).
    pub fn launch_at_cursor(&mut self, mode: LaunchMode) {
        self.launch_at_cursor_with(mode, None);
    }

    /// Like [`launch_at_cursor`](Self::launch_at_cursor) with a custom debug
    /// adapter command (`:debug start <cmd> [args]`).
    pub fn launch_at_cursor_with(
        &mut self,
        mode: LaunchMode,
        adapter: Option<(String, Vec<String>)>,
    ) {
        match self.cursor_launch_source("auto") {
            Ok(source) => self.begin_request(LaunchRequest {
                mode,
                source,
                adapter,
            }),
            Err(message) => self.set_status_message(message),
        }
    }

    /// Run or debug whatever is at a position (0-based line, UTF-16
    /// character) of the current file, e.g. a code lens.
    pub fn launch_at_position(&mut self, mode: LaunchMode, line: usize, character: u32) {
        match self.cursor_launch_source("auto") {
            Ok(LaunchSource::Cursor {
                file,
                language_id,
                target,
                project_root,
                fallback,
                ..
            }) => self.begin_request(LaunchRequest {
                mode,
                source: LaunchSource::Cursor {
                    file,
                    language_id,
                    line: line as u32,
                    character,
                    target,
                    project_root,
                    fallback,
                },
                adapter: None,
            }),
            Ok(_) => {}
            Err(message) => self.set_status_message(message),
        }
    }

    /// Run or debug a configuration chosen by the user.
    pub fn launch_config(
        &mut self,
        mode: LaunchMode,
        config: DebugRunConfig,
        project_root: PathBuf,
    ) {
        self.begin_request(LaunchRequest {
            mode,
            source: LaunchSource::Config {
                config: Box::new(config),
                project_root,
            },
            adapter: None,
        });
    }

    /// Repeat the last launch, replacing whatever is running now.
    pub fn launch_last(&mut self) {
        match self.launch.last_request.clone() {
            Some(request) => self.begin_request(request),
            None => self.set_status_message(
                "Nothing to rerun yet. Run the code at the cursor with <Space>rr, or debug it with <Space>rd",
            ),
        }
    }

    /// Stop everything: resolve task, build, running program, debug session.
    /// Returns false when there was nothing to stop.
    pub fn launch_stop(&mut self) -> bool {
        self.launch.queued = None;
        let lookup_cancelled = self.launch.config_lookup.take().is_some();
        let mut stopped = self.stop_current_job() || lookup_cancelled;
        if self.dap_manager.is_active() && self.launch.job.is_none() {
            // A debug session started by hand (not through a launch job).
            self.dap_manager.request_stop();
            stopped = true;
        }
        if !stopped {
            self.set_status_message("Nothing is running");
        }
        stopped
    }

    // ------------------------------------------------------------------
    // Request setup
    // ------------------------------------------------------------------

    /// Workspace root for `file` (or the current buffer): the language
    /// server's root when one owns the file, else the nearest build marker,
    /// else the file's directory, else the process working directory.
    pub(crate) fn launch_project_root(&self, file: Option<&Path>) -> PathBuf {
        let current = self.buffer().file_path().map(PathBuf::from);
        let Some(file) = file.map(Path::to_path_buf).or(current) else {
            return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        };
        if let (Some(manager), Some(language_id)) = (
            self.lsp_manager(),
            self.language_id_for_path(&file.to_string_lossy()),
        ) {
            for id in manager.servers_for_document(&language_id, &file) {
                if let Some(root) = manager.server_root(&id) {
                    return root;
                }
            }
        }
        // The markers are the language's own (languages.toml), so this is the
        // root the server would be started with; `.ovim` (project run
        // configurations) and `.git` cover projects with no build tool.
        let (mut markers, outermost) = self
            .language_catalog
            .detect(&file)
            .and_then(|language| {
                let lsp = language.config.lsp.as_ref()?;
                Some((lsp.root_markers.clone(), lsp.outermost_root_markers.clone()))
            })
            .unwrap_or_default();
        markers.extend([".ovim".to_string(), ".git".to_string()]);
        let root =
            crate::project_root::find_project_root_with_outermost(&file, &markers, &outermost);
        if root.as_os_str().is_empty() {
            file.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        } else {
            root
        }
    }

    fn cursor_launch_source(&self, target: &str) -> Result<LaunchSource, String> {
        let Some(file) = self.buffer().file_path().map(PathBuf::from) else {
            return Err("Save the buffer to a file before running it".to_string());
        };
        let language_id = self
            .language_id_for_path(&file.to_string_lossy())
            .ok_or_else(|| format!("Don't know how to run {}", file.display()))?;
        let cursor = self.buffer().cursor();
        let character = self.col_to_utf16(cursor.line(), cursor.col().0);
        Ok(LaunchSource::Cursor {
            project_root: self.launch_project_root(Some(&file)),
            file,
            language_id,
            line: cursor.line() as u32,
            character,
            target: target.to_string(),
            fallback: Err("no local plan".to_string()),
        })
    }

    /// Starts a request, replacing whatever is running.
    pub(crate) fn begin_request(&mut self, request: LaunchRequest) {
        if tokio::runtime::Handle::try_current().is_err() {
            self.set_status_message("Cannot start a run: no async runtime available");
            return;
        }
        self.launch.config_lookup = None;
        if self.launch.job.is_some() || self.dap_manager.is_active() {
            // Rerun / new run replaces the current one: stop it, start when
            // it is gone.
            self.launch.queued = Some(request);
            if !self.stop_current_job() && self.dap_manager.is_active() {
                self.dap_manager.request_stop();
            }
            self.set_status_message("Stopping the current run...");
            return;
        }
        self.launch.last_request = Some(request.clone());
        self.save_before_launch();

        let mode = request.mode;
        let root = request.project_root().to_path_buf();
        let title = match &request.source {
            LaunchSource::Cursor { file, .. } => file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".to_string()),
            LaunchSource::Config { config, .. } => config.name.clone(),
            LaunchSource::Plan { plan, .. } => plan.name.clone(),
        };
        let run_id = self
            .launch
            .console
            .start_run(title.clone(), mode, root.clone());
        if let Some(run) = self.launch.console.run_mut(run_id) {
            run.source_roots = vec![root.clone()];
        }
        let mut job = LaunchJob {
            run_id,
            request: request.clone(),
            plan: None,
            stage: Stage::Resolving,
            resolve: None,
            proc: None,
            proc_exit: None,
            reports: None,
            log: String::new(),
            log_stdout: String::new(),
            log_stderr: String::new(),
            started_wall: SystemTime::now(),
            stopping: false,
            session_end: None,
            panel_run: false,
            debug_port: None,
        };

        match request.source.clone() {
            LaunchSource::Plan { plan, .. } => self.begin_plan(&mut job, *plan),
            LaunchSource::Config {
                config,
                project_root,
            } => {
                let plan = plan::plan_from_config(&config, &project_root);
                self.begin_plan(&mut job, plan);
            }
            LaunchSource::Cursor {
                file,
                language_id,
                line,
                character,
                target,
                ..
            } => {
                self.log_console(
                    run_id,
                    LineKind::System,
                    format!(
                        "Resolving what to {} at the cursor...",
                        mode.verb().to_lowercase()
                    ),
                );
                match self.lsp_manager() {
                    Some(manager) => {
                        let (tx, rx) = oneshot::channel();
                        let task = tokio::spawn(async move {
                            let result = lsp::resolve_with_fallback(
                                manager,
                                language_id,
                                file,
                                line,
                                character,
                                target,
                            )
                            .await;
                            let _ = tx.send(result);
                        });
                        job.resolve = Some((rx, task));
                    }
                    None => {
                        let result = ResolveResult {
                            outcome: ResolveOutcome::NoServer(
                                "the language server is not running".to_string(),
                            ),
                            configurations: Vec::new(),
                        };
                        self.on_resolved(&mut job, result);
                    }
                }
            }
        }
        self.launch_put_back(job);
        self.mark_dirty();
    }

    /// Keeps a job that is still in flight; finished jobs were consumed by
    /// `finish_job` and must not be re-inserted.
    fn launch_put_back(&mut self, job: LaunchJob) {
        let finished = self
            .launch
            .console
            .run(job.run_id)
            .is_none_or(|r| !r.status.is_active());
        if !finished {
            self.launch.job = Some(job);
        } else {
            self.start_queued_launch();
        }
    }

    fn start_queued_launch(&mut self) {
        if self.launch.job.is_none() && !self.dap_manager.is_active() {
            if let Some(next) = self.launch.queued.take() {
                self.begin_request(next);
            }
        }
    }

    /// Saves modified buffers so the build sees the code on screen.
    fn save_before_launch(&mut self) {
        let (_, errors) = self.write_all_modified_buffers(false);
        if let Some(first) = errors.first() {
            self.set_status_message(format!(
                "Could not save all files before running ({first}); running what is on disk"
            ));
        }
    }

    fn log_console(&mut self, run_id: u64, kind: LineKind, text: impl Into<String>) {
        if let Some(run) = self.launch.console.run_mut(run_id) {
            run.push_line(kind, text);
        }
        self.mark_dirty();
    }

    // ------------------------------------------------------------------
    // Polling (called every tick)
    // ------------------------------------------------------------------

    /// Advances the launch state machine. Returns true when a redraw is needed.
    pub fn poll_launch(&mut self) -> bool {
        let mut changed = self.ingest_debug_output();
        changed |= self.poll_server_command();
        changed |= self.poll_frame_lookup();
        changed |= self.poll_config_lookup();
        let Some(mut job) = self.launch.job.take() else {
            changed |= self.acknowledge_orphan_session_end();
            self.start_queued_launch();
            return changed;
        };

        // -- resolving --
        if let Some((mut rx, task)) = job.resolve.take() {
            match rx.try_recv() {
                Ok(result) => {
                    self.on_resolved(&mut job, result);
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    job.resolve = Some((rx, task));
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.fail_job(
                        &mut job,
                        "The language server request was cancelled".to_string(),
                    );
                    changed = true;
                }
            }
        }

        // -- process output --
        if job.proc.is_some() {
            changed |= self.drain_process(&mut job);
        }

        // -- test results read in the background --
        changed |= self.poll_test_reports(&mut job);

        // -- debug session end / timeouts --
        changed |= self.poll_debug_state(&mut job);

        self.launch_put_back(job);
        if self.launch.console.any_active() {
            // Keeps the elapsed timer live.
            changed = true;
        }
        changed
    }

    // ------------------------------------------------------------------
    // Finishing and stopping
    // ------------------------------------------------------------------

    fn fail_job(&mut self, job: &mut LaunchJob, message: String) {
        if let Some(proc) = job.proc.as_mut() {
            proc.kill();
        }
        if let Some((_, task)) = job.resolve.take() {
            task.abort();
        }
        if matches!(job.stage, Stage::StartingDebugger | Stage::Debugging { .. })
            || self.dap_manager.is_active()
        {
            self.dap_manager.request_stop();
        }
        self.log_console(job.run_id, LineKind::System, format!("✗ {message}"));
        self.set_status_message(message.clone());
        self.finish_job(job, RunOutcome::Error(message), None);
    }

    fn finish_job(&mut self, job: &mut LaunchJob, outcome: RunOutcome, code: Option<i32>) {
        if std::mem::take(&mut job.panel_run) {
            // The test never got as far as reporting results.
            if let Some(run) = self.build.test_panel.runs.last_mut() {
                if run.status == crate::editor::TestRunStatus::Running {
                    run.duration = Some(run.started.elapsed());
                    match &outcome {
                        RunOutcome::Stopped => {
                            run.status = crate::editor::TestRunStatus::Cancelled;
                            run.lines.push("Stopped".to_string());
                        }
                        other => {
                            run.status = crate::editor::TestRunStatus::Failed;
                            run.lines.push(match other {
                                RunOutcome::Error(m) => m.clone(),
                                _ => "The test run failed before reporting results".to_string(),
                            });
                        }
                    }
                }
            }
        }
        let status = self.launch.console.run_mut(job.run_id).map(|run| {
            run.finish(outcome.clone(), code);
            format!("{}: {}", run.title, run.status_text())
        });
        if let Some(status) = status {
            match &outcome {
                RunOutcome::Error(_) | RunOutcome::BuildFailed => {}
                _ => self.set_status_message(status),
            }
        }
        if !self.dap_manager.state.panel_pinned {
            self.dap_manager.state.panels_visible = false;
        }
        self.mark_dirty();
    }

    /// Asks the current job to stop. Returns true when there was a job.
    fn stop_current_job(&mut self) -> bool {
        let Some(mut job) = self.launch.job.take() else {
            return false;
        };
        job.stopping = true;
        let mut done = false;
        match job.stage {
            Stage::Resolving => {
                if let Some((_, task)) = job.resolve.take() {
                    task.abort();
                }
                self.log_console(job.run_id, LineKind::System, "Stopped");
                self.finish_job(&mut job, RunOutcome::Stopped, None);
                done = true;
            }
            Stage::Building | Stage::Running | Stage::AwaitingDebugPort { .. } => {
                if let Some(proc) = job.proc.as_mut() {
                    proc.kill();
                }
            }
            Stage::StartingDebugger | Stage::Debugging { .. } => {
                if let Some(proc) = job.proc.as_mut() {
                    proc.kill();
                }
                self.dap_manager.request_stop();
            }
            Stage::Reporting => {
                // The program already ended: skip its test results.
                if let Some(pending) = job.reports.take() {
                    self.finish_test_panel_run(
                        &mut job,
                        pending.exit_ok,
                        Vec::new(),
                        None,
                        Vec::new(),
                    );
                    self.finish_run(&mut job, pending.exit_ok, pending.exit_code);
                }
                done = true;
            }
        }
        if !done {
            self.launch.job = Some(job);
        }
        true
    }
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join(" | ")
}
