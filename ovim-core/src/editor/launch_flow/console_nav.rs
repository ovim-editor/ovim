//! Console and command entry points: stdin to the running program, the
//! console panel's visibility and focus, jumping from a console line to its
//! source, and `:LspExec`.

use std::path::{Path, PathBuf};

use tokio::sync::oneshot;

use crate::editor::Editor;
use crate::launch::console::LineKind;
use crate::launch::lsp;
use crate::launch::stacktrace;

impl Editor {
    // ------------------------------------------------------------------
    // Entry points (called from keys and commands; never block)
    // ------------------------------------------------------------------

    /// `:RunInput <text>`: sends a line to the running program's stdin
    /// (`System.in`). Empty text sends an empty line.
    pub fn run_input(&mut self, text: &str) {
        let Some(job) = self.launch.job.as_mut() else {
            self.set_status_message("No program is running");
            return;
        };
        let run_id = job.run_id;
        let sent = job
            .proc
            .as_ref()
            .is_some_and(|p| p.accepts_stdin() && p.send_stdin(&format!("{text}\n")));
        if sent {
            self.log_console(run_id, LineKind::System, format!("» {text}"));
        } else {
            self.set_status_message(
                "The running program does not take input (only a Run or Debug of a main feeds stdin, not tests)",
            );
        }
    }

    /// `:RunEof`: closes the program's stdin (like Ctrl-D in a terminal).
    pub fn run_eof(&mut self) {
        let Some(job) = self.launch.job.as_mut() else {
            self.set_status_message("No program is running");
            return;
        };
        let run_id = job.run_id;
        let closed = job.proc.as_mut().is_some_and(|p| {
            let open = p.accepts_stdin();
            p.close_stdin();
            open
        });
        if closed {
            self.log_console(run_id, LineKind::System, "» (end of input)");
        } else {
            self.set_status_message("Input is already closed");
        }
    }

    /// `:LspExec <command> [json args]`: runs a `workspace/executeCommand`
    /// on the language server that owns the current file and reports the
    /// outcome (result in the status line, or a scratch buffer when long).
    pub fn lsp_execute_command(&mut self, command: &str, arguments: Vec<serde_json::Value>) {
        let Some(file) = self.buffer().file_path().map(PathBuf::from) else {
            self.set_status_message("Open a file first: the command runs on its language server");
            return;
        };
        let Some(language_id) = self.language_id_for_path(&file.to_string_lossy()) else {
            self.set_status_message(format!("No language server for {}", file.display()));
            return;
        };
        let Some(manager) = self.lsp_manager() else {
            self.set_status_message("LSP is not enabled");
            return;
        };
        if tokio::runtime::Handle::try_current().is_err() {
            self.set_status_message("Cannot run a server command: no async runtime available");
            return;
        }
        let (tx, rx) = oneshot::channel();
        let command = command.to_string();
        let name = command.clone();
        tokio::spawn(async move {
            let result =
                lsp::execute_for_document(manager, &language_id, &file, &command, arguments).await;
            let _ = tx.send((command, result));
        });
        self.launch.server_command = Some(rx);
        self.set_status_message(format!("Running {name}..."));
    }

    pub(super) fn poll_server_command(&mut self) -> bool {
        let Some(rx) = self.launch.server_command.as_mut() else {
            return false;
        };
        let (command, result) = match rx.try_recv() {
            Ok(done) => done,
            Err(oneshot::error::TryRecvError::Empty) => return false,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.launch.server_command = None;
                return false;
            }
        };
        self.launch.server_command = None;
        match result {
            Err(message) => self.set_status_message(format!("{command} failed: {message}")),
            Ok(value) => {
                let compact = if value.is_null() {
                    String::new()
                } else {
                    value.to_string()
                };
                if compact.is_empty() {
                    self.set_status_message(format!("{command}: done"));
                } else if compact.len() <= 120 {
                    self.set_status_message(format!("{command}: {compact}"));
                } else {
                    let pretty = serde_json::to_string_pretty(&value).unwrap_or(compact);
                    self.open_scratch_buffer("LspExec", &pretty);
                    self.set_status_message(format!("{command}: result in the LspExec buffer"));
                }
            }
        }
        true
    }

    pub fn toggle_run_console(&mut self) {
        let console = &mut self.launch.console;
        console.open = !console.open;
        if !console.open && self.mode == crate::mode::Mode::RunConsole {
            self.mode = crate::mode::Mode::Normal;
        }
        if self.launch.console.open && self.launch.console.runs.is_empty() {
            self.set_status_message(
                "Run console. <Space>rr runs the code at the cursor, <Space>rd debugs it, <Space>rl reruns the last one",
            );
        }
        self.mark_dirty();
    }

    /// Hide output without stopping its process or discarding history.
    pub fn close_run_console(&mut self) {
        self.launch.console.open = false;
        if self.mode == crate::mode::Mode::RunConsole {
            self.mode = crate::mode::Mode::Normal;
        }
        self.mark_dirty();
    }

    /// Give the run console keyboard focus (scroll, jump, rerun, stop).
    pub fn focus_run_console(&mut self) {
        self.launch.console.open = true;
        let last = self
            .launch
            .console
            .viewed()
            .map(|r| r.lines.len().saturating_sub(1))
            .unwrap_or(0);
        self.launch.console.cursor = last;
        self.launch.console.scroll = 0;
        self.mode = crate::mode::Mode::RunConsole;
        self.mark_dirty();
    }

    pub fn clear_run_console(&mut self) {
        self.launch.console.clear_finished();
        self.mark_dirty();
    }

    /// Jump to the source location on a console line. Returns whether a
    /// location was opened.
    pub fn run_console_jump(&mut self, line_index: usize) -> bool {
        let Some(run) = self.launch.console.viewed() else {
            return false;
        };
        let Some(line) = run.lines.get(line_index) else {
            return false;
        };
        let Some(location) = line.location.clone() else {
            self.set_status_message("No source location on this line");
            return false;
        };
        let (cwd, roots) = (run.cwd.clone(), run.source_roots.clone());
        match stacktrace::resolve_location(&location, &cwd, &roots) {
            Some((path, line, col)) => {
                self.open_location(&path, line, col);
                true
            }
            None => {
                if let stacktrace::ConsoleLocation::Frame {
                    class, file, line, ..
                } = &location
                {
                    // Not in the workspace: libraries, other modules, the JDK.
                    // Ask the language server that indexes them.
                    if self.lookup_frame_via_lsp(class, file, *line) {
                        self.set_status_message(format!("Looking up {class}..."));
                        return true;
                    }
                }
                let what = match &location {
                    stacktrace::ConsoleLocation::Frame { class, file, .. } => {
                        format!("{class} ({file})")
                    }
                    stacktrace::ConsoleLocation::File { path, .. } => path.display().to_string(),
                };
                self.set_status_message(format!("Source not found for {what}"));
                false
            }
        }
    }

    /// Starts a `workspace/symbol` lookup for a stack frame's class. Returns
    /// false when there is no server to ask.
    fn lookup_frame_via_lsp(&mut self, class: &str, file: &str, line: usize) -> bool {
        let Some(manager) = self.lsp_manager() else {
            return false;
        };
        if tokio::runtime::Handle::try_current().is_err() {
            return false;
        }
        let language_id = if file.ends_with(".kt") || file.ends_with(".kts") {
            "kotlin"
        } else {
            "java"
        }
        .to_string();
        let (class, file) = (class.to_string(), file.to_string());
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let result = stacktrace::lookup_frame_source(&manager, &language_id, &class, &file)
                .await
                .map(|path| (path, line));
            let _ = tx.send(result);
        });
        self.launch.frame_lookup = Some(rx);
        true
    }

    pub(super) fn poll_frame_lookup(&mut self) -> bool {
        let Some(rx) = self.launch.frame_lookup.as_mut() else {
            return false;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return false,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.launch.frame_lookup = None;
                return false;
            }
        };
        self.launch.frame_lookup = None;
        match result {
            Ok((path, line)) => self.open_location(&path, line, 1),
            Err(message) => self.set_status_message(message),
        }
        true
    }

    /// Opens `path` at a 1-based line/column and leaves console focus.
    pub(crate) fn open_location(&mut self, path: &Path, line: usize, col: usize) {
        if let Err(e) = self.open_file(path) {
            self.set_status_message(format!("Failed to open {}: {e}", path.display()));
            return;
        }
        if matches!(
            self.mode,
            crate::mode::Mode::RunConsole | crate::mode::Mode::DebugPanel
        ) {
            self.mode = crate::mode::Mode::Normal;
        }
        let line0 = line.saturating_sub(1);
        self.buffer_mut()
            .cursor_mut()
            .set_position(line0, crate::unicode::GraphemeCol(col.saturating_sub(1)));
        self.buffer_mut().validate_cursor_position();
        self.center_cursor_in_viewport();
        self.mark_dirty();
    }
}
