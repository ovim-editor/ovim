//! Test runner integration for vim-test style <Leader>t commands.
//!
//! - `<Space>tn` / `:TestNearest` — run the test at/near the cursor
//! - `<Space>tf` / `:TestFile`    — run the current file's tests
//! - `<Space>ta` / `<Space>ts` / `:TestSuite` — run the whole suite
//! - `<Space>tl` / `:TestLast`    — re-run the last test command
//! - `<Space>tv` / `:TestVisit`   — jump back to the last-tested position
//! - `<Space>tt` / `:TestPanel`   — toggle the right-side test panel
//! - `<Space>to` / `:TestOutput`  — raw output in a scratch buffer
//!
//! Commands run through the launch pipeline (`launch_flow.rs`: process group,
//! stop with `:RunStop`, a new run replaces the current one) in the file's own project root (nearest
//! `Cargo.toml` / `package.json` / `go.mod` / pytest marker — resolved per
//! file, so monorepos work without configuring anything). Output streams
//! live into the right-side test panel (`<Space>tt` toggles it; see
//! `test_panel.rs`). Failures also populate the quickfix list silently for
//! `:cn` navigation; `:TestOutput` shows the raw log.
//!
//! Built-in runners: cargo (rust), vitest/jest/bun/node:test/npm (js/ts), pytest
//! (python), go test (go). Other languages configure `[language.test]` in
//! languages.toml. Nearest-test discovery is tree-sitter based (see
//! `nearest.rs`).

mod jvm;
pub(crate) mod nearest;
mod runners;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod run_tests;

pub use runners::TestScope;
use runners::{build_test_command, TestContext, TestInvocation};

use crate::editor::Editor;
use crate::launch::plan::{LaunchMode, PlanKind};
use crate::syntax::Language;
use std::path::{Path, PathBuf};

/// Remembered state of the last test run, for `:TestLast` / `:TestVisit`.
#[derive(Debug, Clone)]
pub struct LastTest {
    /// Absolute path of the file the test was run from.
    pub file: String,
    /// 0-indexed cursor line at run time.
    pub line: usize,
    /// What `:TestLast` replays: every test run goes through the launch
    /// pipeline.
    pub request: crate::editor::LaunchRequest,
}

impl Editor {
    /// `<Space>tf` - Run tests for the current file.
    pub fn run_test_file(&mut self) {
        self.run_test(TestScope::File);
    }

    /// `<Space>tn` - Run the nearest test (at/above/below cursor).
    pub fn run_test_nearest(&mut self) {
        self.run_test(TestScope::Nearest);
    }

    /// `<Space>ta` / `<Space>ts` - Run the whole test suite.
    pub fn run_test_all(&mut self) {
        self.run_test(TestScope::Suite);
    }

    /// `<Space>tl` - Re-run the last test command.
    pub fn run_test_last(&mut self) {
        match self.build.last_test.clone() {
            Some(last) => self.begin_request(last.request),
            None => self.set_status_message("No previous test command".to_string()),
        }
    }

    /// `<Space>tv` - Jump back to the file/line of the last test run.
    pub fn test_visit(&mut self) {
        let Some(last) = self.build.last_test.clone() else {
            self.set_status_message("No previous test run".to_string());
            return;
        };
        let last_file = PathBuf::from(&last.file);
        let already_there = self
            .buffer()
            .file_path()
            .is_some_and(|p| absolutize(p) == last_file);
        if !already_there {
            if let Err(e) = self.load_file(&last.file) {
                self.set_status_message(format!("Failed to open {}: {}", last.file, e));
                return;
            }
        }
        let line = last.line.min(self.buffer().line_count().saturating_sub(1));
        self.buffer_mut()
            .cursor_mut()
            .set_position(line, crate::unicode::GraphemeCol(0));
        self.buffer_mut().validate_cursor_position();
        self.center_cursor_in_viewport();
    }

    /// `<Space>td` - Debug the nearest test (Java / Kotlin).
    pub fn debug_test_nearest(&mut self) {
        self.run_test_with_mode(TestScope::Nearest, LaunchMode::Debug);
    }

    /// `<Space>tD` - Debug the current file's tests (Java / Kotlin).
    pub fn debug_test_file(&mut self) {
        self.run_test_with_mode(TestScope::File, LaunchMode::Debug);
    }

    fn run_test(&mut self, scope: TestScope) {
        self.run_test_with_mode(scope, LaunchMode::Run);
    }

    fn run_test_with_mode(&mut self, scope: TestScope, mode: LaunchMode) {
        let Some(file_path) = self.buffer().file_path().map(str::to_string) else {
            self.set_status_message(
                "Buffer has no file - save it before running tests".to_string(),
            );
            return;
        };
        let abs_file = absolutize(&file_path);
        let source = self.buffer().rope().to_string();
        let cursor_line = self.buffer().cursor().line();

        let language = crate::syntax::LanguageRegistry::detect_from_path(&abs_file);
        if matches!(language, Some(Language::Java | Language::Kotlin)) {
            self.run_jvm_test(
                scope,
                mode,
                abs_file,
                source,
                cursor_line,
                language.unwrap(),
            );
            return;
        }
        if mode == LaunchMode::Debug {
            self.set_status_message(
                "Debugging tests is supported for Java and Kotlin; use :debug for this file type"
                    .to_string(),
            );
            return;
        }
        let lang_registry = crate::language_config::LanguageRegistry::try_get();
        let test_config = lang_registry
            .and_then(|reg| reg.detect(&abs_file))
            .and_then(|cfg| cfg.test.as_ref());

        let ctx = TestContext {
            file: &abs_file,
            source: &source,
            cursor_line,
            language,
            config: test_config,
        };

        match build_test_command(scope, &ctx) {
            Ok(TestInvocation { command, cwd }) => {
                let label = scope_label(scope);
                let request =
                    crate::editor::LaunchRequest::shell(PlanKind::Test, label, &command, cwd);
                self.remember_and_begin(&abs_file, cursor_line, request);
            }
            Err(msg) => self.set_status_message(msg),
        }
    }

    /// Java / Kotlin tests go through the launch pipeline: the language
    /// server's `hyperion.resolveLaunch` (target `test`) says how to run
    /// them, with a command composed from the file when it cannot. Results
    /// come back from the JUnit XML reports into the test panel.
    fn run_jvm_test(
        &mut self,
        scope: TestScope,
        mode: LaunchMode,
        file: PathBuf,
        source: String,
        cursor_line: usize,
        language: Language,
    ) {
        let project_root = self.launch_project_root(Some(&file));
        let local =
            jvm::local_test_plan(scope, &file, &source, cursor_line, language, &project_root);
        let request_source = match (scope, local) {
            (TestScope::Suite, Ok(local)) => crate::editor::LaunchSource::Plan {
                plan: Box::new(local.plan),
                project_root,
            },
            (TestScope::Suite, Err(message)) => {
                self.set_status_message(message);
                return;
            }
            (_, local) => {
                let Some(language_id) = self.language_id_for_path(&file.to_string_lossy()) else {
                    self.set_status_message(format!("Don't know how to test {}", file.display()));
                    return;
                };
                // Where to ask the server: the cursor when it is on the test
                // (or for the file scope, the class), else the discovered
                // anchor.
                let (line, character, fallback) = match local {
                    Ok(local) => {
                        let (line, col) = match scope {
                            TestScope::Nearest if local.cursor_inside => {
                                let c = self.buffer().cursor();
                                (c.line(), self.col_to_utf16(c.line(), c.col().0) as usize)
                            }
                            _ => local.anchor,
                        };
                        (line, col, Ok(Box::new(local.plan)))
                    }
                    Err(message) => {
                        let c = self.buffer().cursor();
                        (
                            c.line(),
                            self.col_to_utf16(c.line(), c.col().0) as usize,
                            Err(message),
                        )
                    }
                };
                crate::editor::LaunchSource::Cursor {
                    file: file.clone(),
                    language_id,
                    line: line as u32,
                    character: character as u32,
                    target: "test".to_string(),
                    project_root,
                    fallback,
                }
            }
        };
        let request = crate::editor::LaunchRequest {
            mode,
            source: request_source,
            adapter: None,
        };
        self.remember_and_begin(&file, cursor_line, request);
    }

    /// Records `request` for `:TestLast` / `:TestVisit` and starts it.
    fn remember_and_begin(
        &mut self,
        file: &Path,
        cursor_line: usize,
        request: crate::editor::LaunchRequest,
    ) {
        self.build.last_test = Some(LastTest {
            file: file.to_string_lossy().to_string(),
            line: cursor_line,
            request: request.clone(),
        });
        self.begin_request(request);
    }
}

fn scope_label(scope: TestScope) -> &'static str {
    match scope {
        TestScope::Nearest => "nearest",
        TestScope::File => "file",
        TestScope::Suite => "suite",
    }
}

/// Best-effort absolute path: buffer paths may be cwd-relative.
fn absolutize(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else {
        std::env::current_dir().map(|c| c.join(&p)).unwrap_or(p)
    }
}
