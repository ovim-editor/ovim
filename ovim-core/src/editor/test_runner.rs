//! Test runner integration for vim-test style <Leader>t commands.
//!
//! - `<Space>tf` - Test file: run tests matching current file
//! - `<Space>tn` - Test nearest: run test function at/above cursor
//! - `<Space>ta` - Test all: run full test suite
//! - `<Space>tl` - Test last: re-run last test command

use crate::editor::Editor;

impl Editor {
    /// `<Space>tf` - Run tests for the current file.
    ///
    /// For Rust: `cargo test --lib <module_name>` or `cargo test --test <test_file>`
    pub fn run_test_file(&mut self) {
        let filter = self.test_file_filter();
        let cmd = match filter {
            Some(f) => format!("cargo test {}", f),
            None => "cargo test".to_string(),
        };
        self.last_test_command = Some(cmd.clone());
        self.run_make_with_command(&cmd);
    }

    /// `<Space>tn` - Run the nearest test function (at or above cursor).
    pub fn run_test_nearest(&mut self) {
        let test_name = self.find_nearest_test_name();
        let cmd = match test_name {
            Some(name) => format!("cargo test -- {} --exact", name),
            None => {
                self.set_lsp_status("No test function found near cursor".to_string());
                return;
            }
        };
        self.last_test_command = Some(cmd.clone());
        self.run_make_with_command(&cmd);
    }

    /// `<Space>ta` - Run all tests.
    pub fn run_test_all(&mut self) {
        let cmd = "cargo test".to_string();
        self.last_test_command = Some(cmd.clone());
        self.run_make_with_command(&cmd);
    }

    /// `<Space>tl` - Re-run the last test command.
    pub fn run_test_last(&mut self) {
        if let Some(cmd) = self.last_test_command.clone() {
            self.run_make_with_command(&cmd);
        } else {
            self.set_lsp_status("No previous test command".to_string());
        }
    }

    /// Runs a command as a background `:make` job.
    fn run_make_with_command(&mut self, cmd: &str) {
        use crate::editor::{MakeResult, PendingMake};
        use std::process::Command;

        let (tx, rx) = std::sync::mpsc::channel();
        let cmd_owned = cmd.to_string();

        std::thread::spawn(move || {
            let result = match Command::new("sh").arg("-c").arg(&cmd_owned).output() {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    MakeResult {
                        output: format!("{}{}", stdout, stderr),
                        success: output.status.success(),
                    }
                }
                Err(e) => MakeResult {
                    output: format!("Failed to run '{}': {}", cmd_owned, e),
                    success: false,
                },
            };
            let _ = tx.send(result);
        });

        self.set_pending_make(PendingMake {
            receiver: rx,
            command: cmd.to_string(),
        });
        self.set_lsp_status(format!("Running: {}", cmd));
    }

    /// Determines the test filter for the current file.
    ///
    /// For Rust files in `src/`: uses module path (e.g., `editor::test_runner`)
    /// For Rust files in `tests/`: uses `--test <filename>`
    fn test_file_filter(&self) -> Option<String> {
        let path_str = self.buffer().file_path()?;

        if path_str.ends_with(".rs") {
            // Check if it's an integration test (in tests/ directory)
            if let Some(test_name) = self.extract_integration_test_name(path_str) {
                return Some(format!("--test {}", test_name));
            }

            // It's a lib/bin file — extract module path from file path
            // e.g., src/editor/test_runner.rs -> editor::test_runner
            if let Some(mod_path) = self.extract_rust_module_path(path_str) {
                return Some(mod_path);
            }
        }

        None
    }

    /// Extracts integration test name from a path like `tests/foo_test.rs` -> `foo_test`
    fn extract_integration_test_name(&self, path: &str) -> Option<String> {
        // Look for /tests/ directory pattern
        let parts: Vec<&str> = path.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            if *part == "tests" && i + 1 < parts.len() {
                let file = parts[i + 1];
                if file.ends_with(".rs") && !file.contains('/') {
                    return Some(file.trim_end_matches(".rs").to_string());
                }
            }
        }
        None
    }

    /// Extracts Rust module path from file path.
    /// e.g., `src/editor/test_runner.rs` -> `editor::test_runner`
    fn extract_rust_module_path(&self, path: &str) -> Option<String> {
        // Find src/ and take everything after
        let src_idx = path.find("/src/")?;
        let relative = &path[src_idx + 5..]; // skip "/src/"
        let without_ext = relative.trim_end_matches(".rs");
        let mod_path = without_ext
            .replace('/', "::")
            .replace("mod", "")
            .trim_end_matches("::")
            .to_string();

        if mod_path.is_empty() {
            None
        } else {
            Some(mod_path)
        }
    }

    /// Finds the nearest `#[test]` function name at or above the cursor.
    fn find_nearest_test_name(&self) -> Option<String> {
        let cursor_line = self.buffer().cursor().line();
        let line_count = self.buffer().line_count();

        // Search backwards from cursor for `#[test]` or `fn test_`
        let mut test_attr_line = None;

        for line_idx in (0..=cursor_line).rev() {
            if line_idx >= line_count {
                continue;
            }
            let line_text = self.buffer().line(line_idx)?;
            let trimmed = line_text.trim();

            if trimmed == "#[test]" || trimmed == "#[tokio::test]" {
                test_attr_line = Some(line_idx);
                break;
            }
        }

        let attr_line = test_attr_line?;

        // The fn definition should be on the next non-attribute, non-empty line
        for line_idx in (attr_line + 1)..line_count.min(attr_line + 5) {
            let line_text = self.buffer().line(line_idx)?;
            let trimmed = line_text.trim();

            // Skip other attributes
            if trimmed.starts_with('#') {
                continue;
            }

            // Look for `fn test_name(` or `async fn test_name(`
            if let Some(fn_name) = extract_fn_name(trimmed) {
                return Some(fn_name.to_string());
            }
        }

        None
    }
}

/// Extracts function name from a line like `fn foo_bar(` or `async fn foo_bar(`.
fn extract_fn_name(line: &str) -> Option<&str> {
    // Strip optional visibility and async keywords
    let line = line.strip_prefix("pub ").unwrap_or(line);
    let line = line.strip_prefix("async ").unwrap_or(line);

    if !line.starts_with("fn ") {
        return None;
    }
    let rest = &line[3..];
    let end = rest.find('(')?;
    let name = rest[..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}
