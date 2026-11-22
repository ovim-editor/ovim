//! Language server commands: `:LspInfo`, `:LspStatus`, `:LspLog`,
//! `:LspRestart`, `:LspUpdate`, `:LspExec`, `:LspReloadProject`,
//! `:LspRename`, `:LspInstall` / `:LspManager` and `:format`.

use super::Ex;
use crate::command_result::{err, ok, ok_silent, CommandResult};
use crate::editor::Editor;

/// `:LspInfo`: server states in a scratch buffer.
pub(super) fn info(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    // Show LSP status information in a scratch buffer
    let mut info = String::new();

    if let Some(lsp_manager) = editor.lsp_manager() {
        let reports = lsp_manager.server_status_reports();

        if reports.is_empty() {
            info.push_str("No active LSP servers\n");
            if !editor.lsp_status().is_empty() {
                info.push_str(&format!("Status: {}\n", editor.lsp_status()));
            }
        } else {
            info.push_str("Active LSP servers:\n\n");
            for report in &reports {
                // Extract just the binary name from the full path
                let binary_name = std::path::Path::new(&report.command)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| report.command.clone());
                info.push_str(&format!(
                    "  {} -> {}  [{}{}]\n",
                    report.server_id,
                    binary_name,
                    report.state,
                    if report.process_alive {
                        ""
                    } else {
                        ", process not running"
                    }
                ));
                if let Some(root) = &report.root {
                    info.push_str(&format!("      root: {}\n", root.display()));
                }
                if report.total_restarts > 0 {
                    info.push_str(&format!("      restarts: {}\n", report.total_restarts));
                }
                if report.gave_up {
                    info.push_str("      auto-restart gave up; run :LspRestart to try again\n");
                } else if report.restarting {
                    info.push_str("      restarting...\n");
                }
            }

            let (errors, warnings, info_count, hints) = editor.cached_diagnostic_count();
            info.push_str(&format!(
                "\nDiagnostics: {} errors, {} warnings, {} info, {} hints\n",
                errors, warnings, info_count, hints
            ));

            if !editor.lsp_status().is_empty() {
                info.push_str(&format!("\nStatus: {}\n", editor.lsp_status()));
            }

            if let Some(file_path) = editor.buffer().file_path() {
                info.push_str(&format!("\nCurrent file: {}\n", file_path));
            }
            info.push_str("\nCommands: :LspRestart [server|language]\n");
        }
    } else {
        info.push_str("LSP is not enabled\n");
    }

    editor.open_scratch_buffer("LspInfo", &info);
    ok_silent()
}

/// `:LspStatus`: the current file's diagnostics in a scratch buffer.
pub(super) fn status(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    // Show detailed diagnostics list for current file
    use lsp_types::DiagnosticSeverity;

    let mut output = String::new();
    let diagnostics = editor.all_diagnostics();

    if diagnostics.is_empty() {
        output.push_str("No diagnostics for current file\n");
    } else {
        output.push_str(&format!("Diagnostics ({} total):\n\n", diagnostics.len()));

        // Group by severity
        let mut errors: Vec<_> = vec![];
        let mut warnings: Vec<_> = vec![];
        let mut infos: Vec<_> = vec![];
        let mut hints: Vec<_> = vec![];

        for d in diagnostics {
            match d.severity {
                Some(DiagnosticSeverity::ERROR) => errors.push(d),
                Some(DiagnosticSeverity::WARNING) => warnings.push(d),
                Some(DiagnosticSeverity::INFORMATION) => infos.push(d),
                Some(DiagnosticSeverity::HINT) => hints.push(d),
                None => infos.push(d), // Default to info if no severity
                _ => infos.push(d),
            }
        }

        // Print errors first
        if !errors.is_empty() {
            output.push_str("ERRORS:\n");
            for d in &errors {
                let line = d.range.start.line + 1;
                let col = d.range.start.character + 1;
                // Truncate message to first line for cleaner display
                let msg = d.message.lines().next().unwrap_or(&d.message);
                output.push_str(&format!("  {}:{}: {}\n", line, col, msg));
            }
            output.push('\n');
        }

        // Print warnings
        if !warnings.is_empty() {
            output.push_str("WARNINGS:\n");
            for d in &warnings {
                let line = d.range.start.line + 1;
                let col = d.range.start.character + 1;
                let msg = d.message.lines().next().unwrap_or(&d.message);
                output.push_str(&format!("  {}:{}: {}\n", line, col, msg));
            }
            output.push('\n');
        }

        // Print info
        if !infos.is_empty() {
            output.push_str("INFO:\n");
            for d in &infos {
                let line = d.range.start.line + 1;
                let col = d.range.start.character + 1;
                let msg = d.message.lines().next().unwrap_or(&d.message);
                output.push_str(&format!("  {}:{}: {}\n", line, col, msg));
            }
            output.push('\n');
        }

        // Print hints
        if !hints.is_empty() {
            output.push_str("HINTS:\n");
            for d in &hints {
                let line = d.range.start.line + 1;
                let col = d.range.start.character + 1;
                let msg = d.message.lines().next().unwrap_or(&d.message);
                output.push_str(&format!("  {}:{}: {}\n", line, col, msg));
            }
        }
    }

    // Also show LSP status if set
    if !editor.lsp_status().is_empty() {
        output.push_str(&format!("\nLSP Status: {}\n", editor.lsp_status()));
    }

    editor.open_scratch_buffer("LspStatus", &output);
    ok_silent()
}

/// `:LspLog`: open the LSP log in a new tab, at its end.
pub(super) fn log(editor: &mut Editor, _ex: &Ex) -> CommandResult {
    // Open the actual LSP log file in a new tab so % resolves correctly
    let log_path = crate::lsp::get_log_path();
    let log_path_str = log_path.to_string_lossy().to_string();
    editor.new_tab();
    match editor.load_file(&log_path_str) {
        Ok(_) => {
            // Jump to end of log
            let line_count = editor.buffer().rope().len_lines().saturating_sub(1);
            editor.buffer_mut().cursor_mut().set_line(line_count);
            ok_silent()
        }
        Err(e) => err(format!("Failed to open LSP log at {}: {}", log_path_str, e)),
    }
}

/// `:LspRestart [server|language]` — restart language servers now, resetting
/// their automatic-restart budget. Open documents are re-sent by the sync tick.
pub(super) fn restart(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let Some(lsp_manager) = editor.lsp_manager() else {
        return err("LSP is not enabled");
    };
    match lsp_manager.request_restart((!ex.args.is_empty()).then_some(ex.args)) {
        Ok(ids) => {
            editor.set_lsp_status(format!("LSP: restarting {}...", ids.join(", ")));
            ok_silent()
        }
        Err(message) => err(message),
    }
}

/// `:LspUpdate [language]`: install the latest release of the server ovim
/// installed for a language (the current file's by default). The outcome
/// arrives on the status bar; the running server keeps going until
/// `:LspRestart`.
pub(super) fn update(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let language_id = if ex.args.is_empty() {
        let Some(path) = editor.buffer().file_path().map(str::to_string) else {
            return err("LspUpdate: this buffer has no file; name a language");
        };
        match editor.language_catalog().detect(&path) {
            Some(language) => language.config.id.clone(),
            None => return err("LspUpdate: no language for this file; name one"),
        }
    } else {
        ex.args.to_ascii_lowercase()
    };
    match editor.request_lsp_update(&language_id) {
        Ok(command) => ok(format!("Checking for a newer {command}...")),
        Err(message) => err(format!("LspUpdate: {message}")),
    }
}

/// `:LspExec {command} [json arguments...]`: workspace/executeCommand.
pub(super) fn exec(editor: &mut Editor, ex: &Ex) -> CommandResult {
    let (name, args) = match ex.args.split_once(char::is_whitespace) {
        Some((name, args)) => (name, args.trim()),
        None => (ex.args, ""),
    };
    if name.is_empty() {
        return err("Usage: LspExec <command> [json arguments...]");
    }
    let parsed: Result<Vec<serde_json::Value>, _> = serde_json::Deserializer::from_str(args)
        .into_iter::<serde_json::Value>()
        .collect();
    match parsed {
        Ok(arguments) => {
            editor.lsp_execute_command(name, arguments);
            ok_silent()
        }
        Err(e) => err(format!("LspExec: arguments must be JSON values: {e}")),
    }
}

/// `:LspRename {name}`: rename the symbol under the cursor.
pub(super) fn rename(editor: &mut Editor, ex: &Ex) -> CommandResult {
    if ex.args.is_empty() {
        return err("Usage: LspRename <new_name>");
    }
    editor.request_rename(ex.args.to_string());
    ok(format!("Renaming to '{}'...", ex.args))
}
