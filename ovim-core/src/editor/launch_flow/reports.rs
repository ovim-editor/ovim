//! What a finished run leaves behind: build problems and `:make` output in the
//! quickfix list, and JUnit results in the test panel.

use std::time::Duration;

use tokio::sync::oneshot;

use super::{LaunchJob, PendingReports, Stage};
use crate::editor::Editor;
use crate::launch::console::LineKind;
use crate::launch::plan::{LaunchMode, LaunchPlan};
use crate::launch::process::CommandSpec;
use crate::launch::test_report::TestReport;
use crate::launch::{diagnostics, junit};

impl Editor {
    /// Parses the job's build/test output into the quickfix list. With
    /// `jump`, opens the first error. Returns a one-line summary of the failure.
    pub(super) fn report_build_problems(&mut self, job: &LaunchJob, jump: bool) -> Option<String> {
        let cwd = job.plan.as_ref().map(|p| {
            p.build
                .as_ref()
                .map(|b| b.cwd.clone())
                .or_else(|| p.task.as_ref().map(|t| t.cwd.clone()))
                .unwrap_or_else(|| p.project_root.clone())
        });
        let entries = job.diagnostics(cwd.as_deref());
        let summary = diagnostics::summarize_build_failure(&job.log, &entries);
        if entries.is_empty() {
            return summary;
        }
        let title = job
            .plan
            .as_ref()
            .and_then(|p| p.build.as_ref())
            .map(|b| b.display())
            .unwrap_or_else(|| "build".to_string());
        self.present_quickfix(entries, title, jump);
        summary
    }

    /// Shows `entries` as the quickfix list: the first error (else the first
    /// entry) selected, the window opened, and with `jump` the cursor taken
    /// to it. The one place build and `:make` output reaches the user; test
    /// runs stay silent (their panel is the surface) and set the list only.
    pub(crate) fn present_quickfix(
        &mut self,
        entries: Vec<crate::editor::QuickfixEntry>,
        title: String,
        jump: bool,
    ) {
        let first_error = entries
            .iter()
            .position(|e| e.entry_type == crate::editor::QuickfixEntryType::Error)
            .unwrap_or(0);
        self.set_quickfix_list(entries, title);
        self.ui_panels.quickfix_list.set_selected(first_error);
        if jump {
            self.jump_to_quickfix_entry();
        }
        self.open_quickfix_window();
    }

    /// End of a `:make` run: diagnostics from the output (paths resolved
    /// against the directory it ran in) become the quickfix list; the first
    /// error is opened.
    pub(super) fn finish_make(&mut self, job: &LaunchJob, ok: bool) {
        let Some(shell) = job.shell_run() else { return };
        let cwd = job
            .plan
            .as_ref()
            .and_then(|p| p.task.as_ref())
            .map(|t| t.cwd.clone());
        let entries = job.diagnostics(cwd.as_deref());
        // Keep :MakeOutput working on the full log.
        self.build.last_make_output = Some(job.log.clone());
        let title = format!(":make {}", shell.command);
        let count = entries.len();
        if count > 0 {
            self.present_quickfix(entries, title, true);
            self.set_status_message(format!("{count} error(s)/warning(s)"));
        } else {
            self.set_quickfix_list(entries, title);
            if ok {
                self.close_quickfix_window();
                self.set_status_message("Build succeeded — no errors".to_string());
            } else {
                self.set_status_message("Build failed (no parseable errors)".to_string());
            }
        }
    }

    /// Opens the test panel's record of a test plan (running).
    pub(super) fn start_test_panel_run(&mut self, job: &mut LaunchJob, plan: &LaunchPlan) {
        let Some(task) = plan.task.as_ref() else {
            return;
        };
        let debug = job.request.mode == LaunchMode::Debug;
        if let Some(shell) = &task.shell {
            self.build
                .test_panel
                .start_run(shell.label, shell.command.clone(), task.cwd.clone());
            job.panel_run = true;
            return;
        }
        let label = match (debug, task.method_name.is_some(), task.class_name.is_some()) {
            (false, true, _) => "nearest",
            (false, false, true) => "file",
            (false, false, false) => "suite",
            (true, true, _) => "debug nearest",
            (true, false, true) => "debug file",
            (true, false, false) => "debug suite",
        };
        let spec = CommandSpec {
            argv: if debug {
                task.debug_argv.clone().unwrap_or_else(|| task.argv.clone())
            } else {
                task.argv.clone()
            },
            cwd: task.cwd.clone(),
            env: Default::default(),
        };
        self.build
            .test_panel
            .start_run(label, spec.display(), task.cwd.clone());
        job.panel_run = true;
    }

    /// After a test task, read JUnit XML from the plan's reports directory
    /// and surface the outcome in the console, the test panel (per test:
    /// pass/fail, message, jumpable frames) and the quickfix list. Reading
    /// the reports and locating their stack frames can take long in a big
    /// project, so it happens on a blocking task and the result is applied by
    /// [`poll_test_reports`](Self::poll_test_reports). Returns whether the
    /// outcome is still pending.
    pub(super) fn finish_test_reports(
        &mut self,
        job: &mut LaunchJob,
        exit_ok: bool,
        exit_code: Option<i32>,
    ) -> bool {
        if job.panel_run && job.shell_run().is_some() {
            job.panel_run = false;
            self.finish_shell_test_run(exit_ok);
            return false;
        }
        let Some(dir) = job
            .plan
            .as_ref()
            .and_then(|p| p.task.as_ref())
            .and_then(|t| t.reports_dir.clone())
        else {
            self.finish_test_panel_run(job, exit_ok, Vec::new(), None, Vec::new());
            return false;
        };
        let since = job.started_wall - Duration::from_secs(2);
        let roots = job
            .plan
            .as_ref()
            .map(|p| p.source_roots())
            .unwrap_or_default();
        let method_hint = job
            .plan
            .as_ref()
            .and_then(|p| p.task.as_ref())
            .and_then(|t| t.method_name.clone());
        let read =
            move || crate::launch::test_report::build(&dir, since, &roots, method_hint.as_deref());
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            let report = read();
            self.apply_test_report(job, exit_ok, report);
            return false;
        };
        let (tx, rx) = oneshot::channel();
        runtime.spawn_blocking(move || {
            let _ = tx.send(read());
        });
        job.reports = Some(PendingReports {
            rx,
            exit_ok,
            exit_code,
        });
        true
    }

    /// Applies the test report read in the background, once it is in, and
    /// ends the run that was waiting for it.
    pub(super) fn poll_test_reports(&mut self, job: &mut LaunchJob) -> bool {
        let Some(mut pending) = job.reports.take() else {
            return false;
        };
        let report = match pending.rx.try_recv() {
            Ok(report) => report,
            Err(oneshot::error::TryRecvError::Empty) => {
                job.reports = Some(pending);
                return false;
            }
            // The reading task died: show the run without results.
            Err(oneshot::error::TryRecvError::Closed) => TestReport::default(),
        };
        self.apply_test_report(job, pending.exit_ok, report);
        if job.stage == Stage::Reporting {
            self.finish_run(job, pending.exit_ok, pending.exit_code);
        }
        true
    }

    fn apply_test_report(&mut self, job: &mut LaunchJob, exit_ok: bool, report: TestReport) {
        let TestReport {
            cases,
            summary,
            failures: failed,
        } = report;
        let Some(summary_text) = summary else {
            self.finish_test_panel_run(job, exit_ok, cases, None, Vec::new());
            return;
        };
        self.log_console(
            job.run_id,
            LineKind::System,
            format!("Tests: {summary_text}"),
        );
        let mut entries = Vec::new();
        let mut failures = Vec::new();
        for test in failed {
            let crate::launch::test_report::FailedTest {
                label,
                message,
                frames,
            } = test;
            self.log_console(
                job.run_id,
                LineKind::System,
                format!("FAILED {label}: {message}"),
            );
            // Frames in project files, in stack order: the first is where
            // to go, the rest stay reachable through :cn.
            for (i, (path, line, col)) in frames.iter().enumerate() {
                let text = if i == 0 {
                    format!("{label}: {message}")
                } else {
                    format!("  called from here ({label})")
                };
                entries.push(if i == 0 {
                    crate::editor::QuickfixEntry::error(Some(path.clone()), *line, *col, text)
                } else {
                    crate::editor::QuickfixEntry::info(Some(path.clone()), *line, *col, text)
                });
            }
            failures.push(crate::editor::TestFailure {
                test_name: Some(label),
                message,
                location: frames
                    .first()
                    .map(|(p, l, c)| crate::editor::TestSourceLocation {
                        path: p.clone(),
                        line: *l,
                        column: (*c > 0).then_some(*c),
                    }),
                frames: frames
                    .iter()
                    .map(|(p, l, c)| crate::editor::TestSourceLocation {
                        path: p.clone(),
                        line: *l,
                        column: (*c > 0).then_some(*c),
                    })
                    .collect(),
            });
        }
        if !entries.is_empty() {
            self.set_quickfix_list(entries, "test failures".to_string());
        }
        self.finish_test_panel_run(job, exit_ok, cases, Some(summary_text.clone()), failures);
        self.set_status_message(format!("Tests: {summary_text}"));
    }

    /// Fills the test panel's run for this job with per-test results.
    pub(super) fn finish_test_panel_run(
        &mut self,
        job: &mut LaunchJob,
        exit_ok: bool,
        cases: Vec<junit::TestCaseResult>,
        summary: Option<String>,
        failures: Vec<crate::editor::TestFailure>,
    ) {
        if !std::mem::take(&mut job.panel_run) {
            return;
        }
        let tail: Vec<String> = job
            .log
            .lines()
            .rev()
            .take(40)
            .map(str::to_string)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let Some(run) = self.build.test_panel.runs.last_mut() else {
            return;
        };
        if run.status != crate::editor::TestRunStatus::Running {
            return;
        }
        let all_passed = exit_ok
            && !cases.iter().any(|c| {
                matches!(
                    c.status,
                    junit::CaseStatus::Failed | junit::CaseStatus::Errored
                )
            });
        run.status = if all_passed {
            crate::editor::TestRunStatus::Passed
        } else {
            crate::editor::TestRunStatus::Failed
        };
        run.duration = Some(run.started.elapsed());
        run.summary = summary;
        run.failures = failures;
        run.lines.clear();
        if cases.is_empty() {
            run.lines.push(if exit_ok {
                "No test results were reported (no matching tests, or the build tool skipped them)."
                    .to_string()
            } else {
                "The test task failed before reporting results. Last output:".to_string()
            });
            run.lines.extend(tail);
        } else {
            run.lines.extend(junit_panel_lines(&cases));
        }
        let output = run.lines.join("\n");
        self.build.last_make_output = Some(job.log.clone())
            .filter(|l| !l.is_empty())
            .or(Some(output));
        self.mark_dirty();
    }
}

/// One block per test for the test panel: `✓ Class.name (12ms)`, failures
/// with message and the stack, skips dimmed by their `○` marker.
fn junit_panel_lines(cases: &[junit::TestCaseResult]) -> Vec<String> {
    let mut lines = Vec::new();
    for case in cases {
        let class = case
            .class_name
            .rsplit('.')
            .next()
            .unwrap_or(&case.class_name);
        let name = case.name.trim_end_matches("()");
        let time = if case.seconds >= 1.0 {
            format!("{:.1}s", case.seconds)
        } else {
            format!("{}ms", (case.seconds * 1000.0).round() as u64)
        };
        match case.status {
            junit::CaseStatus::Passed => lines.push(format!("✓ {class}.{name} ({time})")),
            junit::CaseStatus::Skipped => lines.push(format!("○ {class}.{name} skipped")),
            junit::CaseStatus::Failed | junit::CaseStatus::Errored => {
                lines.push(format!("✗ {class}.{name} ({time})"));
                if let Some(message) = &case.message {
                    for (i, l) in message.lines().take(6).enumerate() {
                        lines.push(format!("    {}{l}", if i == 0 { "" } else { "  " }));
                    }
                }
                let frames = case
                    .details
                    .as_deref()
                    .into_iter()
                    .flat_map(str::lines)
                    .map(str::trim)
                    .filter(|l| l.starts_with("at "))
                    .filter(|l| {
                        ![
                            "org.junit.",
                            "org.gradle.",
                            "jdk.internal.",
                            "java.base/",
                            "worker.org.gradle.",
                            "org.apache.maven.",
                            "org.opentest4j.",
                        ]
                        .iter()
                        .any(|p| l[3..].starts_with(p))
                    })
                    .take(8);
                for frame in frames {
                    lines.push(format!("    {frame}"));
                }
            }
        }
    }
    lines
}
