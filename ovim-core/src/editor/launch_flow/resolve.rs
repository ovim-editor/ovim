//! Resolving a launch request into a plan: the language server's answer, the
//! locally composed fallback, and the run-configuration picker.

use std::path::PathBuf;

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::{LaunchJob, LaunchRequest, LaunchSource};
use crate::debug_config::DebugRunConfig;
use crate::editor::Editor;
use crate::launch::console::LineKind;
use crate::launch::lsp::{self, ResolveOutcome, ResolveResult};
use crate::launch::plan::{self, LaunchMode, LaunchPlan};

/// Configurations waiting for the user to choose one in the picker.
pub(crate) struct PendingPick {
    mode: LaunchMode,
    configs: Vec<DebugRunConfig>,
    project_root: PathBuf,
    adapter: Option<(String, Vec<String>)>,
}

/// Run configurations being fetched from the language server for the picker.
/// It is not a launch: it never takes the place of a program that is running.
pub(super) struct ConfigLookup {
    mode: LaunchMode,
    project_root: PathBuf,
    /// The configurations from `.ovim/debug.toml`.
    configs: Vec<DebugRunConfig>,
    rx: oneshot::Receiver<Vec<serde_json::Value>>,
    task: JoinHandle<()>,
}

impl Drop for ConfigLookup {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Editor {
    /// Open the picker of available configurations
    /// (`.ovim/debug.toml` + `hyperion.runConfigurations`).
    pub fn launch_pick_config(&mut self, mode: LaunchMode) {
        // A lookup still in flight is replaced.
        self.launch.config_lookup = None;
        let project_root = self.launch_project_root(None);
        let mut loaded = crate::debug_config::load_debug_configs_reporting(&project_root);
        for problem in std::mem::take(&mut loaded.problems) {
            self.set_status_message(problem);
        }
        let file = self.buffer().file_path().map(PathBuf::from);
        let language_id = file
            .as_ref()
            .and_then(|f| self.language_id_for_path(&f.to_string_lossy()));
        let manager = self.lsp_manager();
        let (Some(manager), Some(file), Some(language_id), Ok(_)) = (
            manager,
            file,
            language_id,
            tokio::runtime::Handle::try_current(),
        ) else {
            self.offer_configs(
                mode,
                loaded.configs,
                project_root,
                None,
                "no language server",
            );
            return;
        };
        // The LSP half is fetched off-tick, then merged with the TOML half.
        let (tx, rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = tx.send(lsp::run_configurations(&manager, &language_id, &file).await);
        });
        self.launch.config_lookup = Some(ConfigLookup {
            mode,
            project_root,
            configs: loaded.configs,
            rx,
            task,
        });
        self.set_status_message("Looking up run configurations...");
    }

    /// A configuration was chosen in the picker.
    pub(crate) fn select_launch_config(&mut self, index: usize) {
        let Some(pick) = self.launch.pending_pick.take() else {
            self.set_status_message("No configuration list is open");
            return;
        };
        let Some(config) = pick.configs.get(index).cloned() else {
            self.set_status_message("Invalid configuration index");
            return;
        };
        self.begin_request(LaunchRequest {
            mode: pick.mode,
            source: LaunchSource::Config {
                config: Box::new(config),
                project_root: pick.project_root,
            },
            adapter: pick.adapter,
        });
    }

    // ------------------------------------------------------------------
    // Resolution
    // ------------------------------------------------------------------

    pub(super) fn on_resolved(&mut self, job: &mut LaunchJob, result: ResolveResult) {
        let mode = job.request.mode;
        // Tests fall back to a plan composed locally (never to run configs).
        let test_fallback = match &job.request.source {
            LaunchSource::Cursor {
                target, fallback, ..
            } if target == "test" => Some(fallback.clone()),
            _ => None,
        };
        let _ = mode;
        match result.outcome {
            ResolveOutcome::Plan(value) => match plan::plan_from_resolved(&value) {
                Ok(Some(plan)) => self.begin_plan(job, plan),
                Ok(None) => match test_fallback {
                    Some(fallback) => {
                        self.begin_test_fallback(job, fallback, "no test found by the server")
                    }
                    None => self.no_plan_here(
                        job,
                        "nothing runnable at the cursor",
                        result.configurations,
                    ),
                },
                Err(message) => self.fail_job(job, message),
            },
            ResolveOutcome::Nothing => match test_fallback {
                Some(fallback) => {
                    self.begin_test_fallback(job, fallback, "the server found no test here")
                }
                None => self.no_plan_here(
                    job,
                    "no main method or test at the cursor",
                    result.configurations,
                ),
            },
            ResolveOutcome::Unsupported(reason)
            | ResolveOutcome::NoServer(reason)
            | ResolveOutcome::Failed(reason) => match test_fallback {
                Some(fallback) => self.begin_test_fallback(job, fallback, &reason),
                None => self.no_plan_here(job, &reason, result.configurations),
            },
        }
    }

    /// Runs the locally composed test plan when the server had none.
    fn begin_test_fallback(
        &mut self,
        job: &mut LaunchJob,
        fallback: Result<Box<LaunchPlan>, String>,
        why: &str,
    ) {
        match fallback {
            Ok(plan) => {
                self.log_console(
                    job.run_id,
                    LineKind::System,
                    format!("({why}; using a test command composed from the file)"),
                );
                self.begin_plan(job, *plan);
            }
            Err(message) => self.fail_job(job, message),
        }
    }

    /// The server gave no launch information for the cursor: fall back to
    /// configurations, or explain what to do.
    fn no_plan_here(
        &mut self,
        job: &mut LaunchJob,
        reason: &str,
        lsp_configs: Vec<serde_json::Value>,
    ) {
        let mode = job.request.mode;
        let root = job.request.project_root().to_path_buf();
        let mut loaded = crate::debug_config::load_debug_configs_reporting(&root);
        for problem in std::mem::take(&mut loaded.problems) {
            self.log_console(job.run_id, LineKind::System, format!("warning: {problem}"));
        }
        let mut configs = loaded.configs;
        configs.extend(crate::debug_config::parse_lsp_run_configs(&lsp_configs));
        self.offer_configs_for_job(job, mode, configs, reason);
    }

    /// Opens the picker once the language server's run configurations are in.
    pub(super) fn poll_config_lookup(&mut self) -> bool {
        let Some(lookup) = self.launch.config_lookup.as_mut() else {
            return false;
        };
        let from_server = match lookup.rx.try_recv() {
            Ok(values) => values,
            Err(oneshot::error::TryRecvError::Empty) => return false,
            // The lookup died: offer what the project file has.
            Err(oneshot::error::TryRecvError::Closed) => Vec::new(),
        };
        let Some(mut lookup) = self.launch.config_lookup.take() else {
            return false;
        };
        let mut configs = std::mem::take(&mut lookup.configs);
        configs.extend(crate::debug_config::parse_lsp_run_configs(&from_server));
        let (mode, root) = (lookup.mode, std::mem::take(&mut lookup.project_root));
        self.offer_configs(mode, configs, root, None, "");
        true
    }

    /// Chooses among configurations when a job could not resolve a plan.
    fn offer_configs_for_job(
        &mut self,
        job: &mut LaunchJob,
        mode: LaunchMode,
        configs: Vec<DebugRunConfig>,
        reason: &str,
    ) {
        let root = job.request.project_root().to_path_buf();
        let runnable: Vec<DebugRunConfig> = configs
            .into_iter()
            .filter(|c| {
                let plan = plan::plan_from_config(c, &root);
                plan.unsupported_reason(mode).is_none()
            })
            .collect();
        match runnable.len() {
            0 => {
                let what = match mode {
                    LaunchMode::Run => "run",
                    LaunchMode::Debug => "debug",
                };
                self.fail_job(
                    job,
                    format!(
                        "Nothing to {what} here: {reason}. Put the cursor in a class with a main method or in a test, \
                         or add a configuration to .ovim/debug.toml"
                    ),
                );
            }
            1 => {
                let plan = plan::plan_from_config(&runnable[0], &root);
                self.log_console(
                    job.run_id,
                    LineKind::System,
                    format!("({reason}; using configuration '{}')", runnable[0].name),
                );
                self.begin_plan(job, plan);
            }
            _ => {
                self.discard_run(job);
                let adapter = job.request.adapter.clone();
                self.offer_configs(mode, runnable, root, adapter, reason);
            }
        }
    }

    /// Shows the configuration picker (or explains why there is nothing to pick).
    fn offer_configs(
        &mut self,
        mode: LaunchMode,
        configs: Vec<DebugRunConfig>,
        project_root: PathBuf,
        adapter: Option<(String, Vec<String>)>,
        reason: &str,
    ) {
        let configs: Vec<DebugRunConfig> = configs
            .into_iter()
            .filter(|c| {
                plan::plan_from_config(c, &project_root)
                    .unsupported_reason(mode)
                    .is_none()
            })
            .collect();
        if configs.is_empty() {
            self.set_status_message(
                "No run configurations found. Add one to .ovim/debug.toml, or run the code at the cursor with <Space>rr",
            );
            return;
        }
        let names: Vec<String> = configs.iter().map(|c| c.name.clone()).collect();
        self.launch.pending_pick = Some(PendingPick {
            mode,
            configs,
            project_root: project_root.clone(),
            adapter,
        });
        let picker = crate::editor::picker::Picker::new_debug_config(project_root, names);
        self.set_picker(picker);
        self.set_mode(crate::mode::Mode::Picker);
        self.mark_picker_selection_changed();
        if !reason.is_empty() {
            self.set_status_message(format!("{reason}: pick a configuration"));
        }
    }

    /// Removes the console record of a job that turned into a picker.
    fn discard_run(&mut self, job: &LaunchJob) {
        self.launch.console.runs.retain(|r| r.id != job.run_id);
        self.launch.console.selected = None;
        if self.launch.console.runs.is_empty() {
            self.launch.console.open = false;
        }
        // Consumed: mark finished so `launch_put_back` drops it.
    }
}
