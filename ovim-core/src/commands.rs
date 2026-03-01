//! Command execution for ex commands (:w, :q, etc.)

use crate::command_result::{CommandResult, ErrorResponse, SuccessResponse};
use crate::editor::Editor;
use crate::editor::QuickfixEntry;

/// Expands ~ to home directory in file paths
///
/// Returns an error if the path starts with ~ but the home directory cannot be determined.
fn expand_tilde(path: &str) -> Result<std::path::PathBuf, String> {
    if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            let expanded = format!("{}{}", home.display(), &path[1..]);
            return Ok(std::path::PathBuf::from(expanded));
        } else {
            return Err("Could not determine home directory".to_string());
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return Ok(home);
        } else {
            return Err("Could not determine home directory".to_string());
        }
    }
    Ok(std::path::PathBuf::from(path))
}

/// Helper function to jump to a quickfix entry
pub fn jump_to_quickfix_entry(editor: &mut Editor, entry: &QuickfixEntry) -> CommandResult {
    if let Some(ref path) = entry.filename {
        // Load the file if needed
        let path_str = path.to_string_lossy().to_string();
        if let Err(e) = editor.load_file(&path_str) {
            return CommandResult::Error(ErrorResponse {
                error: format!("Failed to load file: {}", e),
            });
        }

        // Jump to line/column (convert from 1-indexed to 0-indexed)
        let line = entry.lnum.saturating_sub(1);
        let col = entry.col.saturating_sub(1);
        editor.buffer_mut().cursor_mut().set_position(line, col);
        editor.buffer_mut().validate_cursor_position();

        CommandResult::Success(SuccessResponse {
            success: true,
            message: Some(entry.display_text()),
            line_count: None,
        })
    } else {
        CommandResult::Success(SuccessResponse {
            success: true,
            message: Some(entry.text.clone()),
            line_count: None,
        })
    }
}

/// Execute a command (e.g., :w, :q, :tabnew)
pub fn execute_command(editor: &mut Editor, command: &str) -> CommandResult {
    // Intercept write/quit commands when in a chat scratch buffer
    if editor.is_chat_scratch_buffer() {
        match command {
            "w" | "write" | "wq" | "x" => {
                let _ = editor.finish_chat_scratch(true);
                return CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("Scratch content transferred to chat input".to_string()),
                    line_count: None,
                });
            }
            "q!" | "quit!" | "bd!" | "bdelete!" | "q" | "quit" => {
                let _ = editor.finish_chat_scratch(false);
                return CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("Scratch buffer discarded".to_string()),
                    line_count: None,
                });
            }
            _ => {}
        }
    }

    match command {
        "q" | "quit" => {
            // If there are multiple tabs, close current tab. Otherwise quit.
            if editor.tab_page_manager().is_single_tab() {
                // Single tab - check modifications and quit
                if editor.is_modified() {
                    CommandResult::Error(ErrorResponse {
                        error: "No write since last change (add ! to override)".to_string(),
                    })
                } else {
                    editor.quit();
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some("Quitting".to_string()),
                        line_count: None,
                    })
                }
            } else {
                // Multiple tabs - close current tab
                editor.close_current_tab();
                let tab_index = editor.current_tab_index() + 1;
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Tab closed. Now on tab {}", tab_index)),
                    line_count: None,
                })
            }
        }
        "q!" | "quit!" => {
            // Force quit or close tab
            if editor.tab_page_manager().is_single_tab() {
                editor.quit();
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("Quitting (forced)".to_string()),
                    line_count: None,
                })
            } else {
                editor.close_current_tab();
                let tab_index = editor.current_tab_index() + 1;
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Tab closed. Now on tab {}", tab_index)),
                    line_count: None,
                })
            }
        }
        "cq" | "cquit" => {
            // Quit with non-zero exit code (like vim's :cq)
            editor.quit_with_code(1);
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Quitting with error code 1".to_string()),
                line_count: None,
            })
        }
        cmd if cmd.starts_with("cq ") || cmd.starts_with("cquit ") => {
            // :cq N - quit with specific exit code
            let code_str = cmd.split_whitespace().nth(1).unwrap_or("1");
            match code_str.parse::<i32>() {
                Ok(code) => {
                    editor.quit_with_code(code);
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("Quitting with error code {}", code)),
                        line_count: None,
                    })
                }
                Err(_) => CommandResult::Error(ErrorResponse {
                    error: format!("Invalid exit code: {}", code_str),
                }),
            }
        }
        "qa" | "qall" => {
            // Quit all - same as quit for single buffer
            if editor.is_modified() {
                CommandResult::Error(ErrorResponse {
                    error: "No write since last change (add ! to override)".to_string(),
                })
            } else {
                editor.quit();
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("Quitting all".to_string()),
                    line_count: None,
                })
            }
        }
        "qa!" | "qall!" => {
            // Force quit all without saving
            editor.quit();
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Quitting all (forced)".to_string()),
                line_count: None,
            })
        }
        "w" | "write" => {
            // Check if buffer is read-only
            if editor.buffer().is_read_only() {
                return CommandResult::Error(ErrorResponse {
                    error: "E45: 'readonly' option is set (add ! to override)".to_string(),
                });
            }
            if let Some(path) = editor.buffer().file_path().map(|s| s.to_string()) {
                let old_path = Some(path.clone());
                match editor.buffer_mut().save_as(&path) {
                    Ok(_) => {
                        let new_path = editor.buffer().file_path().map(|s| s.to_string());
                        editor.handle_file_path_transition_after_save(old_path, new_path);
                        if editor.options.blame {
                            editor.buffer_mut().load_git_blame();
                        }
                        editor.mark_saved();
                        editor.mark_buffer_saved(); // Mark for LSP didSave notification
                        let line_count = editor.buffer().rope().len_lines();
                        let char_count = editor.buffer().rope().len_chars();
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(format!(
                                "\"{}\" {}L, {}C written",
                                path, line_count, char_count
                            )),
                            line_count: None,
                        })
                    }
                    Err(e) => CommandResult::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            } else {
                CommandResult::Error(ErrorResponse {
                    error: "No file name".to_string(),
                })
            }
        }
        "w!" | "write!" => {
            // Force write even if read-only
            if let Some(path) = editor.buffer().file_path().map(|s| s.to_string()) {
                let old_path = Some(path.clone());
                match editor.buffer_mut().save_as(&path) {
                    Ok(_) => {
                        let new_path = editor.buffer().file_path().map(|s| s.to_string());
                        editor.handle_file_path_transition_after_save(old_path, new_path);
                        if editor.options.blame {
                            editor.buffer_mut().load_git_blame();
                        }
                        // Clear read-only flag after successful forced write
                        editor.buffer_mut().set_read_only(false);
                        editor.mark_saved();
                        editor.mark_buffer_saved();
                        let line_count = editor.buffer().rope().len_lines();
                        let char_count = editor.buffer().rope().len_chars();
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(format!(
                                "\"{}\" {}L, {}C written",
                                path, line_count, char_count
                            )),
                            line_count: None,
                        })
                    }
                    Err(e) => CommandResult::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            } else {
                CommandResult::Error(ErrorResponse {
                    error: "No file name".to_string(),
                })
            }
        }
        "wq" => {
            // Check if buffer is read-only
            if editor.buffer().is_read_only() {
                return CommandResult::Error(ErrorResponse {
                    error: "E45: 'readonly' option is set (add ! to override)".to_string(),
                });
            }
            if let Some(path) = editor.buffer().file_path().map(|s| s.to_string()) {
                let old_path = Some(path.clone());
                match editor.buffer_mut().save_as(&path) {
                    Ok(_) => {
                        let new_path = editor.buffer().file_path().map(|s| s.to_string());
                        editor.handle_file_path_transition_after_save(old_path, new_path);
                        editor.mark_saved();
                        editor.mark_buffer_saved(); // Mark for LSP didSave notification
                        editor.quit();
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some("Saved and quitting".to_string()),
                            line_count: None,
                        })
                    }
                    Err(e) => CommandResult::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            } else {
                CommandResult::Error(ErrorResponse {
                    error: "No file name".to_string(),
                })
            }
        }
        "wq!" => {
            // Force write even if read-only
            if let Some(path) = editor.buffer().file_path().map(|s| s.to_string()) {
                let old_path = Some(path.clone());
                match editor.buffer_mut().save_as(&path) {
                    Ok(_) => {
                        let new_path = editor.buffer().file_path().map(|s| s.to_string());
                        editor.handle_file_path_transition_after_save(old_path, new_path);
                        editor.buffer_mut().set_read_only(false);
                        editor.mark_saved();
                        editor.mark_buffer_saved();
                        editor.quit();
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some("Saved and quitting".to_string()),
                            line_count: None,
                        })
                    }
                    Err(e) => CommandResult::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            } else {
                CommandResult::Error(ErrorResponse {
                    error: "No file name".to_string(),
                })
            }
        }
        "LspInfo" => {
            // Show LSP status information in a scratch buffer
            let mut info = String::new();

            if let Some(lsp_manager) = editor.lsp_manager() {
                // Get active servers from lsp_manager (more reliable than editor's map)
                let languages = lsp_manager.active_server_languages();

                if languages.is_empty() {
                    info.push_str("No active LSP servers\n");
                    if !editor.lsp_status().is_empty() {
                        info.push_str(&format!("Status: {}\n", editor.lsp_status()));
                    }
                } else {
                    info.push_str("Active LSP servers:\n\n");
                    for lang_id in &languages {
                        if let Some(cmd) = lsp_manager.server_command(lang_id) {
                            // Extract just the binary name from the full path
                            let binary_name = std::path::Path::new(&cmd)
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or(cmd);
                            info.push_str(&format!("  {} -> {}\n", lang_id, binary_name));
                        } else {
                            info.push_str(&format!("  {}\n", lang_id));
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
                }
            } else {
                info.push_str("LSP is not enabled\n");
            }

            editor.open_scratch_buffer("LspInfo", &info);
            CommandResult::Success(SuccessResponse {
                success: true,
                message: None,
                line_count: None,
            })
        }
        "LspStatus" => {
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
            CommandResult::Success(SuccessResponse {
                success: true,
                message: None,
                line_count: None,
            })
        }
        "LspLog" => {
            // Open the actual LSP log file in a new tab so % resolves correctly
            let log_path = crate::lsp::get_log_path();
            let log_path_str = log_path.to_string_lossy().to_string();
            editor.new_tab(None);
            match editor.load_file(&log_path_str) {
                Ok(_) => {
                    // Jump to end of log
                    let line_count = editor.buffer().rope().len_lines().saturating_sub(1);
                    editor.buffer_mut().cursor_mut().set_line(line_count);
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: None,
                        line_count: None,
                    })
                }
                Err(e) => CommandResult::Error(ErrorResponse {
                    error: format!("Failed to open LSP log at {}: {}", log_path_str, e),
                }),
            }
        }
        cmd if cmd.starts_with("LspRename ") => {
            // LSP rename symbol: :LspRename new_name
            let new_name = cmd["LspRename ".len()..].trim();
            if new_name.is_empty() {
                CommandResult::Error(ErrorResponse {
                    error: "Usage: LspRename <new_name>".to_string(),
                })
            } else {
                editor.request_rename(new_name.to_string());
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Renaming to '{}'...", new_name)),
                    line_count: None,
                })
            }
        }
        "copen" => {
            // Open/show quickfix list
            let qf_list = editor.quickfix_list();
            if qf_list.is_empty() {
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("Quickfix list is empty".to_string()),
                    line_count: None,
                })
            } else {
                let title = if qf_list.title().is_empty() {
                    "Quickfix List"
                } else {
                    qf_list.title()
                };
                let entries: Vec<String> = qf_list
                    .entries()
                    .iter()
                    .enumerate()
                    .map(|(i, entry)| {
                        let marker = if i == qf_list.selected_index() {
                            ">"
                        } else {
                            " "
                        };
                        format!("{} {}", marker, entry.display_text())
                    })
                    .collect();
                let message = format!(
                    "{} ({} items)\n{}",
                    title,
                    qf_list.len(),
                    entries.join("\n")
                );
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(message),
                    line_count: None,
                })
            }
        }
        "cclose" | "ccl" => {
            // Close/clear quickfix list
            editor.quickfix_list_mut().clear();
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Quickfix list cleared".to_string()),
                line_count: None,
            })
        }
        "cnext" | "cn" => {
            // Jump to next quickfix entry
            if editor.quickfix_list().is_empty() {
                CommandResult::Error(ErrorResponse {
                    error: "Quickfix list is empty".to_string(),
                })
            } else {
                editor.quickfix_list_mut().next();
                if let Some(entry) = editor.quickfix_list().current_entry().cloned() {
                    crate::commands::jump_to_quickfix_entry(editor, &entry)
                } else {
                    CommandResult::Error(ErrorResponse {
                        error: "No current entry".to_string(),
                    })
                }
            }
        }
        "cprev" | "cp" | "cprevious" => {
            // Jump to previous quickfix entry
            if editor.quickfix_list().is_empty() {
                CommandResult::Error(ErrorResponse {
                    error: "Quickfix list is empty".to_string(),
                })
            } else {
                editor.quickfix_list_mut().previous();
                if let Some(entry) = editor.quickfix_list().current_entry().cloned() {
                    crate::commands::jump_to_quickfix_entry(editor, &entry)
                } else {
                    CommandResult::Error(ErrorResponse {
                        error: "No current entry".to_string(),
                    })
                }
            }
        }
        "cfirst" | "cfir" => {
            // Jump to first quickfix entry
            if editor.quickfix_list().is_empty() {
                CommandResult::Error(ErrorResponse {
                    error: "Quickfix list is empty".to_string(),
                })
            } else {
                editor.quickfix_list_mut().first();
                if let Some(entry) = editor.quickfix_list().current_entry().cloned() {
                    crate::commands::jump_to_quickfix_entry(editor, &entry)
                } else {
                    CommandResult::Error(ErrorResponse {
                        error: "No current entry".to_string(),
                    })
                }
            }
        }
        "clast" | "cla" => {
            // Jump to last quickfix entry
            if editor.quickfix_list().is_empty() {
                CommandResult::Error(ErrorResponse {
                    error: "Quickfix list is empty".to_string(),
                })
            } else {
                editor.quickfix_list_mut().last();
                if let Some(entry) = editor.quickfix_list().current_entry().cloned() {
                    crate::commands::jump_to_quickfix_entry(editor, &entry)
                } else {
                    CommandResult::Error(ErrorResponse {
                        error: "No current entry".to_string(),
                    })
                }
            }
        }
        "tabnew" | "tabe" | "tabedit" => {
            // Create new tab with default name
            editor.new_tab(None);
            let tab_index = editor.current_tab_index() + 1; // 1-indexed for display
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(format!("Created tab {}", tab_index)),
                line_count: None,
            })
        }
        "tabnext" | "tabn" => {
            // Switch to next tab
            editor.next_tab();
            let tab_index = editor.current_tab_index() + 1; // 1-indexed for display
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(format!("Tab {}", tab_index)),
                line_count: None,
            })
        }
        "tabprev" | "tabp" | "tabprevious" => {
            // Switch to previous tab
            editor.previous_tab();
            let tab_index = editor.current_tab_index() + 1; // 1-indexed for display
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(format!("Tab {}", tab_index)),
                line_count: None,
            })
        }
        "tabfirst" | "tabfir" => {
            // Switch to first tab
            editor.first_tab();
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Tab 1".to_string()),
                line_count: None,
            })
        }
        "tablast" | "tabl" => {
            // Switch to last tab
            editor.last_tab();
            let tab_index = editor.current_tab_index() + 1; // 1-indexed for display
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(format!("Tab {}", tab_index)),
                line_count: None,
            })
        }
        "tabclose" | "tabc" => {
            // Close current tab
            if editor.tab_page_manager().is_single_tab() {
                CommandResult::Error(ErrorResponse {
                    error: "Cannot close last tab".to_string(),
                })
            } else {
                editor.close_current_tab();
                let tab_index = editor.current_tab_index() + 1; // 1-indexed for display
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Tab closed. Now on tab {}", tab_index)),
                    line_count: None,
                })
            }
        }
        "ls" | "buffers" | "files" => {
            // List all buffers
            let buffer_list = editor.list_buffers();
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(buffer_list),
                line_count: None,
            })
        }
        "bnext" | "bn" => {
            // Switch to next buffer
            editor.next_buffer();
            let buffer_index = editor.current_buffer_index() + 1; // 1-indexed for display
            let buffer_name = editor
                .buffer()
                .file_path()
                .map(|p| {
                    std::path::Path::new(p)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("[No Name]")
                })
                .unwrap_or("[No Name]");
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(format!("Buffer {} - {}", buffer_index, buffer_name)),
                line_count: None,
            })
        }
        "bprev" | "bp" | "bprevious" => {
            // Switch to previous buffer
            editor.prev_buffer();
            let buffer_index = editor.current_buffer_index() + 1; // 1-indexed for display
            let buffer_name = editor
                .buffer()
                .file_path()
                .map(|p| {
                    std::path::Path::new(p)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("[No Name]")
                })
                .unwrap_or("[No Name]");
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(format!("Buffer {} - {}", buffer_index, buffer_name)),
                line_count: None,
            })
        }
        "tabonly" | "tabo" => {
            // Close all tabs except the current one
            if editor.tab_page_manager().is_single_tab() {
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("Already only one tab".to_string()),
                    line_count: None,
                })
            } else {
                let closed_count = editor.tab_count() - 1;
                editor.close_other_tabs();
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Closed {} tabs", closed_count)),
                    line_count: None,
                })
            }
        }
        "blame" => {
            let new_val = !editor.options.blame;
            editor.options.blame = new_val;
            if new_val {
                editor.buffer_mut().load_git_blame();
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("blame on".to_string()),
                    line_count: None,
                })
            } else {
                editor.buffer_mut().clear_git_blame();
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("blame off".to_string()),
                    line_count: None,
                })
            }
        }
        "noh" | "nohlsearch" => {
            // Clear search highlighting
            editor.clear_search_highlight();
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Search highlighting cleared".to_string()),
                line_count: None,
            })
        }
        "reg" | "registers" => {
            // Display all registers
            let registers = editor.registers().list_registers();
            if registers.is_empty() {
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("No registers in use".to_string()),
                    line_count: None,
                })
            } else {
                let display: Vec<String> = registers
                    .iter()
                    .map(|(name, content)| format!("{}: {}", name, content))
                    .collect();
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(display.join("\n")),
                    line_count: None,
                })
            }
        }
        "j" | "join" => {
            // Join current line with the next line
            if let Err(e) = editor.buffer_mut().join_lines(1) {
                return CommandResult::Error(ErrorResponse {
                    error: format!("Failed to join lines: {}", e),
                });
            }
            CommandResult::Success(SuccessResponse {
                success: true,
                message: None,
                line_count: None,
            })
        }
        "recover" | "rec" => {
            // Recover buffer content from swap file
            if !editor.buffer().has_swap_file() {
                return CommandResult::Error(ErrorResponse {
                    error: "No swap file exists for this buffer".to_string(),
                });
            }
            match editor.buffer_mut().recover_from_swap_file() {
                Ok(true) => CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("Buffer recovered from swap file".to_string()),
                    line_count: None,
                }),
                Ok(false) => CommandResult::Error(ErrorResponse {
                    error: "Failed to recover: swap file is empty or missing".to_string(),
                }),
                Err(e) => CommandResult::Error(ErrorResponse {
                    error: format!("Failed to recover: {}", e),
                }),
            }
        }
        "checktime" => {
            // Check if file has been modified externally and reload if so
            match editor.buffer().check_external_modification() {
                Ok(true) => match editor.buffer_mut().reload_if_changed_sync() {
                    Ok(true) => {
                        editor.mark_buffer_modified_force_send();
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(
                                "File reloaded from disk (external changes detected)".to_string(),
                            ),
                            line_count: None,
                        })
                    }
                    Ok(false) => CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some("No external changes detected".to_string()),
                        line_count: None,
                    }),
                    Err(e) => CommandResult::Error(ErrorResponse {
                        error: format!("Failed to reload: {}", e),
                    }),
                },
                Ok(false) => CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("No external changes detected".to_string()),
                    line_count: None,
                }),
                Err(e) => CommandResult::Error(ErrorResponse {
                    error: format!("Failed to check file: {}", e),
                }),
            }
        }
        "marks" => {
            // Display all marks
            let mut lines = Vec::new();

            // Local marks (a-z)
            let mut local_marks: Vec<_> = editor.marks().iter().collect();
            local_marks.sort_by_key(|(c, _)| *c);
            for (name, mark) in local_marks {
                lines.push(format!(" '{}  {:>5}  {:>3}", name, mark.line + 1, mark.col));
            }

            // Global marks (A-Z)
            let mut global_marks: Vec<_> = editor.marks().iter_global().collect();
            global_marks.sort_by_key(|(c, _)| *c);
            for (name, mark) in global_marks {
                let file = mark.file_path.as_deref().unwrap_or("[No Name]");
                lines.push(format!(
                    " '{}  {:>5}  {:>3}  {}",
                    name,
                    mark.line + 1,
                    mark.col,
                    file
                ));
            }

            if lines.is_empty() {
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("No marks set".to_string()),
                    line_count: None,
                })
            } else {
                lines.insert(0, "mark  line   col  file".to_string());
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(lines.join("\n")),
                    line_count: None,
                })
            }
        }
        "tabs" => {
            // List all tabs
            let tabs = editor.tab_page_manager().tabs();
            let current_index = editor.current_tab_index();
            let tab_list: Vec<String> = tabs
                .iter()
                .enumerate()
                .map(|(i, tab)| {
                    let marker = if i == current_index { ">" } else { " " };
                    format!("{} {} {}", marker, i + 1, tab.title())
                })
                .collect();
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(tab_list.join("\n")),
                line_count: None,
            })
        }
        "clearaedits" => {
            if let Some(chat) = editor.ai_state.chat.as_mut() {
                chat.agent_edits.clear();
            }
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Agent edit markers cleared.".to_string()),
                line_count: None,
            })
        }
        _ => {
            // Handle :tabnew <filename>, :tabe <filename>, :tabedit <filename>
            if let Some(raw_filename) = command
                .strip_prefix("tabnew ")
                .or_else(|| command.strip_prefix("tabe "))
                .or_else(|| command.strip_prefix("tabedit "))
            {
                // Expand ~ to home directory
                let filename = match expand_tilde(raw_filename) {
                    Ok(path) => path.to_string_lossy().to_string(),
                    Err(e) => {
                        return CommandResult::Error(ErrorResponse {
                            error: format!("Failed to expand path '{}': {}", raw_filename, e),
                        });
                    }
                };

                // Create new tab and load file (or create if doesn't exist)
                editor.new_tab(None);

                // Try to load the file, if it doesn't exist create an empty buffer
                match editor.load_file(&filename) {
                    Ok(_) => {
                        let tab_index = editor.current_tab_index() + 1;
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Opened {} in tab {}", filename, tab_index)),
                            line_count: None,
                        })
                    }
                    Err(e) => {
                        // Check if error is because file doesn't exist
                        if e.to_string().contains("Failed to read file")
                            || e.to_string().contains("No such file")
                        {
                            // Create a new empty buffer with the given filename
                            use crate::buffer::Buffer;
                            let new_buffer = Buffer::new();
                            // Normalize the path
                            let absolute_path = std::path::absolute(&filename)
                                .unwrap_or_else(|_| std::path::PathBuf::from(&filename));
                            let path_str = absolute_path.to_string_lossy().to_string();
                            editor.add_buffer(new_buffer);
                            editor.set_file_path(path_str);
                            editor.mark_dirty();

                            // Update tab title to match the new file
                            editor.update_current_tab_title();

                            // Sync tab's buffer index to match the new buffer
                            editor.sync_current_tab_buffer_index();

                            let tab_index = editor.current_tab_index() + 1;
                            CommandResult::Success(SuccessResponse {
                                success: true,
                                message: Some(format!(
                                    "Created new file {} in tab {}",
                                    filename, tab_index
                                )),
                                line_count: None,
                            })
                        } else {
                            CommandResult::Error(ErrorResponse {
                                error: format!("Failed to load file: {}", e),
                            })
                        }
                    }
                }
            // Handle :w <filename>
            } else if let Some(raw_filename) = command
                .strip_prefix("w ")
                .or_else(|| command.strip_prefix("write "))
            {
                let old_path = editor.buffer().file_path().map(|s| s.to_string());
                let filename = match expand_tilde(raw_filename) {
                    Ok(path) => path.to_string_lossy().to_string(),
                    Err(e) => {
                        return CommandResult::Error(ErrorResponse {
                            error: format!("Failed to expand path '{}': {}", raw_filename, e),
                        });
                    }
                };
                match editor.buffer_mut().save_as(&filename) {
                    Ok(_) => {
                        let new_path = editor.buffer().file_path().map(|s| s.to_string());
                        editor.handle_file_path_transition_after_save(old_path, new_path);
                        if editor.options.blame {
                            editor.buffer_mut().load_git_blame();
                        }
                        editor.mark_saved();
                        editor.mark_buffer_saved(); // Mark for LSP didSave notification
                        let saved_path = editor
                            .buffer()
                            .file_path()
                            .map(|p| p.to_string())
                            .unwrap_or(filename);
                        let line_count = editor.buffer().rope().len_lines();
                        let char_count = editor.buffer().rope().len_chars();
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(format!(
                                "\"{}\" {}L, {}C written",
                                saved_path, line_count, char_count
                            )),
                            line_count: None,
                        })
                    }
                    Err(e) => CommandResult::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            // Handle :lua <code>
            } else if let Some(_code) = command.strip_prefix("lua ") {
                #[cfg(feature = "lua")]
                {
                    match editor.execute_lua(_code) {
                        Ok(result) => CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(result),
                            line_count: None,
                        }),
                        Err(e) => CommandResult::Error(ErrorResponse {
                            error: format!("Lua error: {}", e),
                        }),
                    }
                }
                #[cfg(not(feature = "lua"))]
                CommandResult::Error(ErrorResponse {
                    error: "Lua support not compiled in".to_string(),
                })
            // Handle :luafile <path>
            } else if let Some(raw_path) = command.strip_prefix("luafile ") {
                let _expanded_path = match expand_tilde(raw_path.trim()) {
                    Ok(path) => path.to_string_lossy().to_string(),
                    Err(e) => {
                        return CommandResult::Error(ErrorResponse {
                            error: format!("Failed to expand path '{}': {}", raw_path, e),
                        });
                    }
                };
                #[cfg(feature = "lua")]
                {
                    match editor.execute_lua_file(&_expanded_path) {
                        Ok(_) => CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Executed {}", _expanded_path)),
                            line_count: None,
                        }),
                        Err(e) => CommandResult::Error(ErrorResponse {
                            error: format!("Lua error: {}", e),
                        }),
                    }
                }
                #[cfg(not(feature = "lua"))]
                CommandResult::Error(ErrorResponse {
                    error: "Lua support not compiled in".to_string(),
                })
            // Handle :colorscheme <name> or :colorscheme (to show current)
            // Also support :colo abbreviation
            } else if command == "colorscheme" || command == "colo" {
                let current = editor.current_color_scheme_name();
                let schemes = editor.list_color_schemes().join(", ");
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Current: {}\nAvailable: {}", current, schemes)),
                    line_count: None,
                })
            } else if let Some(scheme_name) = command
                .strip_prefix("colorscheme ")
                .or_else(|| command.strip_prefix("colo "))
            {
                match editor.set_color_scheme(scheme_name.trim()) {
                    Ok(_) => CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("Color scheme set to '{}'", scheme_name.trim())),
                        line_count: None,
                    }),
                    Err(e) => {
                        let available = editor.list_color_schemes().join(", ");
                        CommandResult::Error(ErrorResponse {
                            error: format!("{}. Available schemes: {}", e, available),
                        })
                    }
                }
            // Handle :set commands
            } else if let Some(set_cmd) = command
                .strip_prefix("set ")
                .or_else(|| command.strip_prefix("se "))
            {
                crate::cmd_set::handle_set_command(editor, set_cmd.trim())
            // Handle split commands
            } else if command == "sp" || command == "split" {
                editor.split_window_horizontal();
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!(
                        "Split horizontally ({} windows)",
                        editor.window_count()
                    )),
                    line_count: None,
                })
            } else if command == "vsp" || command == "vsplit" {
                editor.split_window_vertical();
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!(
                        "Split vertically ({} windows)",
                        editor.window_count()
                    )),
                    line_count: None,
                })
            } else if command == "only" || command == "on" {
                // :only - close all other windows
                if editor.window_count() == 1 {
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some("Already only one window".to_string()),
                        line_count: None,
                    })
                } else {
                    editor.close_other_windows();
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some("All other windows closed".to_string()),
                        line_count: None,
                    })
                }
            // Handle config reload
            } else if command == "ConfigReload" || command == "reload" {
                #[cfg(feature = "lua")]
                {
                    match editor.reload_lua_config() {
                        Ok(msg) => CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(msg),
                            line_count: None,
                        }),
                        Err(e) => CommandResult::Error(ErrorResponse {
                            error: format!("Failed to reload config: {}", e),
                        }),
                    }
                }
                #[cfg(not(feature = "lua"))]
                CommandResult::Error(ErrorResponse {
                    error: "Lua support not compiled in".to_string(),
                })
            // Handle :source - load and execute a Lua file
            } else if let Some(file) = command
                .strip_prefix("source ")
                .or_else(|| command.strip_prefix("so "))
            {
                let file = file.trim();
                let _expanded = match expand_tilde(file) {
                    Ok(path) => path,
                    Err(e) => {
                        return CommandResult::Error(ErrorResponse {
                            error: format!("Failed to expand path '{}': {}", file, e),
                        });
                    }
                };
                #[cfg(feature = "lua")]
                {
                    if let Some(context) = editor.lua_context_mut() {
                        let path = _expanded;
                        match context.execute_file(&path) {
                            Ok(_) => {
                                // Process any commands from the sourced file using the public API
                                let commands = editor.get_lua_commands();
                                for cmd in commands {
                                    let _ = crate::editor::InputHandler::execute_command_string(
                                        editor, &cmd,
                                    );
                                }
                                CommandResult::Success(SuccessResponse {
                                    success: true,
                                    message: Some(format!("Sourced: {}", path.display())),
                                    line_count: None,
                                })
                            }
                            Err(e) => CommandResult::Error(ErrorResponse {
                                error: format!("Failed to source {}: {}", file, e),
                            }),
                        }
                    } else {
                        CommandResult::Error(ErrorResponse {
                            error: "Lua not enabled".to_string(),
                        })
                    }
                }
                #[cfg(not(feature = "lua"))]
                CommandResult::Error(ErrorResponse {
                    error: "Lua support not compiled in".to_string(),
                })
            // Handle :bn (next buffer)
            } else if command == "bn" || command == "bnext" {
                editor.next_buffer();
                let buf_name = editor
                    .buffer()
                    .file_path()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "[No Name]".to_string());
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!(
                        "Buffer {} of {}: {}",
                        editor.current_buffer_index() + 1,
                        editor.buffer_count(),
                        buf_name
                    )),
                    line_count: None,
                })
            // Handle :bp (previous buffer)
            } else if command == "bp" || command == "bprev" || command == "bprevious" {
                editor.prev_buffer();
                let buf_name = editor
                    .buffer()
                    .file_path()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "[No Name]".to_string());
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!(
                        "Buffer {} of {}: {}",
                        editor.current_buffer_index() + 1,
                        editor.buffer_count(),
                        buf_name
                    )),
                    line_count: None,
                })
            // Handle :bd (delete buffer)
            } else if command == "bd" || command == "bdelete" {
                if editor.is_modified() {
                    CommandResult::Error(ErrorResponse {
                        error: "No write since last change (add ! to override)".to_string(),
                    })
                } else {
                    let should_quit = editor.delete_current_buffer();
                    if should_quit {
                        editor.quit();
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some("Last buffer deleted, quitting".to_string()),
                            line_count: None,
                        })
                    } else {
                        let buf_name = editor
                            .buffer()
                            .file_path()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "[No Name]".to_string());
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Buffer deleted. Now showing: {}", buf_name)),
                            line_count: None,
                        })
                    }
                }
            // Handle :bd! (force delete buffer)
            } else if command == "bd!" || command == "bdelete!" {
                let should_quit = editor.delete_current_buffer();
                if should_quit {
                    editor.quit();
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some("Last buffer deleted, quitting".to_string()),
                        line_count: None,
                    })
                } else {
                    let buf_name = editor
                        .buffer()
                        .file_path()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "[No Name]".to_string());
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("Buffer deleted. Now showing: {}", buf_name)),
                        line_count: None,
                    })
                }
            // Handle :ls or :buffers (list buffers)
            } else if command == "ls" || command == "buffers" {
                let buf_list: Vec<String> = editor
                    .buffer_names()
                    .iter()
                    .enumerate()
                    .map(|(i, name)| {
                        let marker = if i == editor.current_buffer_index() {
                            "%"
                        } else {
                            " "
                        };
                        let modified = if i < editor.buffer_count()
                            && !editor.buffers[i].change_manager().is_at_save_point()
                        {
                            "+"
                        } else {
                            " "
                        };
                        format!("{} {}  {}", marker, modified, name)
                    })
                    .collect();
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(buf_list.join("\n")),
                    line_count: None,
                })
            // Handle :e and :edit (bare) - reload current file if unmodified
            } else if command == "e" || command == "edit" {
                if editor.buffer().file_path().is_none() {
                    return CommandResult::Error(ErrorResponse {
                        error: "No file name".to_string(),
                    });
                }
                if editor.is_modified() {
                    return CommandResult::Error(ErrorResponse {
                        error: "No write since last change (add ! to override)".to_string(),
                    });
                }
                let path = editor.buffer().file_path().unwrap().to_string();
                match editor.buffer_mut().reload_from_disk() {
                    Ok(_) => {
                        editor.mark_saved();
                        editor.mark_buffer_modified_force_send();
                        let line_count = editor.buffer().rope().len_lines();
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("\"{}\" {}L reloaded", path, line_count)),
                            line_count: None,
                        })
                    }
                    Err(e) => CommandResult::Error(ErrorResponse {
                        error: format!("Failed to reload: {}", e),
                    }),
                }
            // Handle :e! and :edit! - reload current file discarding changes
            } else if command == "e!" || command == "edit!" {
                if let Some(path) = editor.buffer().file_path().map(|s| s.to_string()) {
                    match editor.buffer_mut().reload_from_disk() {
                        Ok(_) => {
                            editor.mark_saved();
                            editor.mark_buffer_modified_force_send();
                            let line_count = editor.buffer().rope().len_lines();
                            CommandResult::Success(SuccessResponse {
                                success: true,
                                message: Some(format!("\"{}\" {}L reloaded", path, line_count)),
                                line_count: None,
                            })
                        }
                        Err(e) => CommandResult::Error(ErrorResponse {
                            error: format!("Failed to reload: {}", e),
                        }),
                    }
                } else {
                    CommandResult::Error(ErrorResponse {
                        error: "No file to reload".to_string(),
                    })
                }
            // :e! <filename> - force-edit file (discard unsaved changes)
            // Must be checked before :e <filename> since "e " prefix matches "e! "
            } else if let Some(raw_filename) = command
                .strip_prefix("e! ")
                .or_else(|| command.strip_prefix("edit! "))
            {
                let filename = match expand_tilde(raw_filename) {
                    Ok(path) => path.to_string_lossy().to_string(),
                    Err(e) => {
                        return CommandResult::Error(ErrorResponse {
                            error: format!("Failed to expand path '{}': {}", raw_filename, e),
                        });
                    }
                };
                match editor.load_file(&filename) {
                    Ok(_) => {
                        let buf_name = editor
                            .buffer()
                            .file_path()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "[No Name]".to_string());
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Editing: {}", buf_name)),
                            line_count: None,
                        })
                    }
                    Err(e) => CommandResult::Error(ErrorResponse {
                        error: format!("Failed to load file: {}", e),
                    }),
                }
            } else if let Some(raw_filename) = command
                .strip_prefix("e ")
                .or_else(|| command.strip_prefix("edit "))
            {
                // :e <filename> - edit file (check for unsaved changes first)
                if editor.is_modified() {
                    return CommandResult::Error(ErrorResponse {
                        error: "No write since last change (add ! to override)".to_string(),
                    });
                }
                let filename = match expand_tilde(raw_filename) {
                    Ok(path) => path.to_string_lossy().to_string(),
                    Err(e) => {
                        return CommandResult::Error(ErrorResponse {
                            error: format!("Failed to expand path '{}': {}", raw_filename, e),
                        });
                    }
                };
                match editor.load_file(&filename) {
                    Ok(_) => {
                        let buf_name = editor
                            .buffer()
                            .file_path()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "[No Name]".to_string());
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Editing: {}", buf_name)),
                            line_count: None,
                        })
                    }
                    Err(e) => CommandResult::Error(ErrorResponse {
                        error: format!("Failed to load file: {}", e),
                    }),
                }
            // Handle :registers or :reg (list registers)
            } else if command == "registers"
                || command == "reg"
                || command.starts_with("registers ")
                || command.starts_with("reg ")
            {
                let registers = editor.registers().list_registers();
                if registers.is_empty() {
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some("No registers set".to_string()),
                        line_count: None,
                    })
                } else {
                    let lines: Vec<String> = registers
                        .into_iter()
                        .map(|(name, content)| format!("{:<4} {}", name, content))
                        .collect();
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("--- Registers ---\n{}", lines.join("\n"))),
                        line_count: None,
                    })
                }
            // Handle :marks (list marks)
            } else if command == "marks" || command.starts_with("marks ") {
                let marks = editor.marks().list_marks();
                if marks.is_empty() {
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some("No marks set".to_string()),
                        line_count: None,
                    })
                } else {
                    let lines: Vec<String> = marks
                        .into_iter()
                        .map(|(name, line, col, file)| {
                            if let Some(path) = file {
                                format!(" {}  {:>5}  {:>3}  {}", name, line + 1, col, path)
                            } else {
                                format!(" {}  {:>5}  {:>3}", name, line + 1, col)
                            }
                        })
                        .collect();
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some(format!(
                            "--- Marks ---\nmark  line  col  file\n{}",
                            lines.join("\n")
                        )),
                        line_count: None,
                    })
                }
            // Handle :help keybindings
            } else if command == "help keybindings" || command == "help keys" {
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(
                        "Keybinding compatibility guide: architecture/knowledge/keybinding-compat.md".to_string(),
                    ),
                    line_count: None,
                })
            // Handle :map, :noremap and variants
            } else if is_map_command(command) {
                handle_map_command(editor, command)
            // Handle :unmap and variants
            } else if is_unmap_command(command) {
                handle_unmap_command(editor, command)
            // Handle :mapclear and variants
            } else if is_mapclear_command(command) {
                handle_mapclear_command(editor, command)
            // Handle :session start/stop/list commands
            } else if command == "ai status" || command == "ai env" {
                handle_ai_status(editor)
            } else if command == "workflow" || command.starts_with("workflow ") {
                handle_workflow_command(editor, command)
            } else if command == "session" || command.starts_with("session ") {
                handle_session_command(editor, command)
            } else if command == "debug" || command.starts_with("debug ") {
                handle_debug_command(editor, command)
            } else if let Some(expr) = command.strip_prefix("eval ") {
                // :eval <expression> — evaluate expression in debug context
                if editor.is_debug_stopped() {
                    let expression = expr.trim().to_string();
                    if expression.is_empty() {
                        CommandResult::Error(ErrorResponse {
                            error: "Usage: :eval <expression>".to_string(),
                        })
                    } else {
                        editor.dap_manager_mut().pending_action =
                            Some(crate::dap::PendingDebugAction::Evaluate { expression });
                        CommandResult::Success(SuccessResponse {
                            success: true,
                            message: Some("Evaluating...".to_string()),
                            line_count: None,
                        })
                    }
                } else {
                    CommandResult::Error(ErrorResponse {
                        error: "Not stopped at a breakpoint".to_string(),
                    })
                }
            } else if let Some(condition) = command.strip_prefix("DebugCondition ") {
                // :DebugCondition <expr> — set conditional breakpoint at cursor
                let condition = condition.trim().to_string();
                if condition.is_empty() {
                    // Empty condition = remove condition (convert to unconditional)
                    if let Some(file_path) = editor.buffer().file_path().map(|s| s.to_string()) {
                        let line = editor.buffer().cursor().line() as u64 + 1;
                        let path = std::path::PathBuf::from(&file_path);
                        editor.dap_manager_mut().state.set_breakpoint_condition(&path, line, None);
                    }
                } else {
                    editor.toggle_conditional_breakpoint(condition);
                }
                // Sync breakpoints if debug is active.
                if editor.is_debug_active() {
                    if let Some(file_path) = editor.buffer().file_path().map(|s| s.to_string()) {
                        let path = std::path::PathBuf::from(&file_path);
                        let lines = editor.dap_manager_mut().state.breakpoint_lines(&path);
                        let _ = lines; // Sync will happen in event loop via pending action
                        editor.dap_manager_mut().pending_action =
                            Some(crate::dap::PendingDebugAction::SyncBreakpoints);
                    }
                }
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some("Conditional breakpoint set".to_string()),
                    line_count: None,
                })
            // Handle :! shell command execution
            } else if let Some(shell_cmd) = command.strip_prefix('!') {
                if shell_cmd.trim().is_empty() {
                    CommandResult::Error(ErrorResponse {
                        error: "No shell command specified".to_string(),
                    })
                } else {
                    execute_shell_command_with_expansion(editor, shell_cmd.trim())
                }
            // Handle :LspInstall / :LspManager - open LSP manager panel
            } else if command == "LspInstall" || command == "LspManager" {
                editor.open_lsp_manager();
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: None,
                    line_count: None,
                })
            // Handle line number command (e.g., :48 to go to line 48)
            } else if let Ok(line_num) = command.parse::<usize>() {
                let target_line = line_num.saturating_sub(1); // 1-indexed to 0-indexed
                let max_line = editor.buffer().line_count().saturating_sub(1);
                let final_line = target_line.min(max_line);
                editor.buffer_mut().cursor_mut().set_position(final_line, 0);
                CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Line {}", line_num)),
                    line_count: None,
                })
            } else {
                CommandResult::Error(ErrorResponse {
                    error: format!("Not an editor command: {}", command),
                })
            }
        }
    }
}

/// Check if this is a map command
fn is_map_command(cmd: &str) -> bool {
    let cmd_word = cmd.split_whitespace().next().unwrap_or("");
    matches!(
        cmd_word,
        "map"
            | "nmap"
            | "imap"
            | "vmap"
            | "xmap"
            | "cmap"
            | "noremap"
            | "nnoremap"
            | "inoremap"
            | "vnoremap"
            | "xnoremap"
            | "cnoremap"
    )
}

/// Check if this is an unmap command
fn is_unmap_command(cmd: &str) -> bool {
    let cmd_word = cmd.split_whitespace().next().unwrap_or("");
    matches!(
        cmd_word,
        "unmap" | "nunmap" | "iunmap" | "vunmap" | "xunmap" | "cunmap"
    )
}

/// Check if this is a mapclear command
fn is_mapclear_command(cmd: &str) -> bool {
    let cmd_word = cmd.split_whitespace().next().unwrap_or("");
    matches!(
        cmd_word,
        "mapclear" | "nmapclear" | "imapclear" | "vmapclear" | "xmapclear" | "cmapclear"
    )
}

/// Parse key notation for map/unmap commands with `<leader>` expansion.
fn parse_map_keys(editor: &Editor, input: &str) -> String {
    use crate::editor::KeyMapManager;

    let leader = editor.leader_key().to_string();
    let expanded = input
        .replace("<leader>", &leader)
        .replace("<Leader>", &leader);

    KeyMapManager::parse_key_notation(&expanded)
}

/// Handle map and noremap commands
fn handle_map_command(editor: &mut Editor, command: &str) -> CommandResult {
    use crate::editor::MapMode;

    let parts: Vec<&str> = command.splitn(3, char::is_whitespace).collect();
    let cmd_word = parts.first().copied().unwrap_or("");

    // Determine mode and whether it's noremap
    let (mode, noremap) = match cmd_word {
        "map" => (MapMode::All, false),
        "noremap" => (MapMode::All, true),
        "nmap" => (MapMode::Normal, false),
        "nnoremap" => (MapMode::Normal, true),
        "imap" => (MapMode::Insert, false),
        "inoremap" => (MapMode::Insert, true),
        "vmap" | "xmap" => (MapMode::Visual, false),
        "vnoremap" | "xnoremap" => (MapMode::Visual, true),
        "cmap" => (MapMode::Command, false),
        "cnoremap" => (MapMode::Command, true),
        _ => (MapMode::Normal, false),
    };

    // If no arguments, list mappings for this mode
    if parts.len() == 1 {
        let mappings = editor.keymaps().list_mappings(Some(mode));
        if mappings.is_empty() {
            return CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("No mappings".to_string()),
                line_count: None,
            });
        }
        let lines: Vec<String> = mappings
            .into_iter()
            .map(|(m, mapping)| {
                let noremap_char = if mapping.noremap { '*' } else { ' ' };
                format!(
                    "{}{}  {}  {}",
                    m.display_char(),
                    noremap_char,
                    mapping.lhs,
                    mapping.rhs
                )
            })
            .collect();
        return CommandResult::Success(SuccessResponse {
            success: true,
            message: Some(format!("--- Mappings ---\n{}", lines.join("\n"))),
            line_count: None,
        });
    }

    // If only lhs provided, show mapping for that key
    if parts.len() == 2 {
        let lhs = parse_map_keys(editor, parts[1]);
        if let Some(mapping) = editor.keymaps().get_mapping(mode, &lhs) {
            return CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(format!(
                    "{}  {}  {}",
                    mode.display_char(),
                    mapping.lhs,
                    mapping.rhs
                )),
                line_count: None,
            });
        } else {
            return CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("No mapping found".to_string()),
                line_count: None,
            });
        }
    }

    // parts.len() >= 3: lhs and rhs provided
    let lhs = parse_map_keys(editor, parts[1]);
    let rhs = parse_map_keys(editor, parts[2]);

    editor
        .keymaps_mut()
        .add_mapping(mode, lhs.clone(), rhs, noremap);

    CommandResult::Success(SuccessResponse {
        success: true,
        message: None,
        line_count: None,
    })
}

/// Handle unmap commands
fn handle_unmap_command(editor: &mut Editor, command: &str) -> CommandResult {
    use crate::editor::MapMode;

    let parts: Vec<&str> = command.split_whitespace().collect();
    let cmd_word = parts.first().copied().unwrap_or("");

    let mode = match cmd_word {
        "unmap" => MapMode::All,
        "nunmap" => MapMode::Normal,
        "iunmap" => MapMode::Insert,
        "vunmap" | "xunmap" => MapMode::Visual,
        "cunmap" => MapMode::Command,
        _ => MapMode::Normal,
    };

    if parts.len() < 2 {
        return CommandResult::Error(ErrorResponse {
            error: "E474: Invalid argument".to_string(),
        });
    }

    let lhs = parse_map_keys(editor, parts[1]);
    if editor.keymaps_mut().remove_mapping(mode, &lhs) {
        CommandResult::Success(SuccessResponse {
            success: true,
            message: None,
            line_count: None,
        })
    } else {
        CommandResult::Error(ErrorResponse {
            error: "E31: No such mapping".to_string(),
        })
    }
}

/// Handle mapclear commands
fn handle_mapclear_command(editor: &mut Editor, command: &str) -> CommandResult {
    use crate::editor::MapMode;

    let cmd_word = command.split_whitespace().next().unwrap_or("");

    let mode = match cmd_word {
        "mapclear" => MapMode::All,
        "nmapclear" => MapMode::Normal,
        "imapclear" => MapMode::Insert,
        "vmapclear" | "xmapclear" => MapMode::Visual,
        "cmapclear" => MapMode::Command,
        _ => MapMode::Normal,
    };

    editor.keymaps_mut().clear_mappings(mode);

    CommandResult::Success(SuccessResponse {
        success: true,
        message: None,
        line_count: None,
    })
}

/// Execute a shell command with % and # expansion, and return the output
fn execute_shell_command_with_expansion(editor: &Editor, cmd: &str) -> CommandResult {
    use crate::editor::shell_expansion::expand_shell_command;

    // Get current and alternate file for expansion
    let current_file = editor.buffer().file_path().unwrap_or("").to_string();
    let alternate_file = editor.registers().get(Some('#'));

    // Expand % and # in the command
    let expanded_cmd = expand_shell_command(cmd, &current_file, &alternate_file);

    execute_shell_command(&expanded_cmd)
}

/// Execute a shell command and return the output
fn execute_shell_command(cmd: &str) -> CommandResult {
    use std::process::Command;

    // Determine the shell to use based on platform
    #[cfg(target_os = "windows")]
    let (shell, shell_arg) = ("cmd", "/C");
    #[cfg(not(target_os = "windows"))]
    let (shell, shell_arg) = ("sh", "-c");

    match Command::new(shell).arg(shell_arg).arg(cmd).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut result = String::new();

            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&stderr);
            }

            // Trim trailing newlines for cleaner display
            let result = result.trim_end().to_string();

            if output.status.success() {
                if result.is_empty() {
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some("Command executed successfully".to_string()),
                        line_count: None,
                    })
                } else {
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some(result),
                        line_count: None,
                    })
                }
            } else {
                let exit_code = output
                    .status
                    .code()
                    .map(|c| format!(" (exit code {})", c))
                    .unwrap_or_default();
                if result.is_empty() {
                    CommandResult::Error(ErrorResponse {
                        error: format!("Command failed{}", exit_code),
                    })
                } else {
                    CommandResult::Error(ErrorResponse {
                        error: format!("{}\n\nCommand failed{}", result, exit_code),
                    })
                }
            }
        }
        Err(e) => CommandResult::Error(ErrorResponse {
            error: format!("Failed to execute command: {}", e),
        }),
    }
}

/// Handle :session start/stop/list commands
///
/// `:session start NAME` — writes a session file so external tools can discover this instance
/// `:session stop` — deletes the session file (API server keeps running for internal use)
/// `:session` or `:session list` — shows active sessions
/// `:ai status` / `:ai env` — show active AI profile and env var diagnostics.
fn handle_ai_status(editor: &mut Editor) -> CommandResult {
    let config = &editor.ai_state.config;
    let active = &editor.ai_state.active_profile;

    let mut lines = Vec::new();
    lines.push(format!("**AI Configuration**"));
    lines.push(format!("Active profile: {}", active));
    lines.push(format!("Default profile: {}", config.default_profile));
    lines.push(format!(
        "Profiles: {}",
        config
            .profiles
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let approval_mode = match config.tool_approval_mode {
        crate::ai::ToolApprovalMode::Auto => "auto",
        crate::ai::ToolApprovalMode::SensitivePrompt => "sensitive_prompt",
        crate::ai::ToolApprovalMode::AlwaysPrompt => "always_prompt",
    };
    lines.push(format!("Tool approval mode: {}", approval_mode));

    // Show context mappings
    if !config.contexts.is_empty() {
        let ctx_str: Vec<String> = config
            .contexts
            .iter()
            .map(|(k, v)| format!("{}→{}", k, v))
            .collect();
        lines.push(format!("Contexts: {}", ctx_str.join(", ")));
    }

    lines.push(String::new());

    // Show details for active profile
    if let Some(profile) = config.resolve_profile(active) {
        lines.push(format!("**Profile '{}' details:**", active));
        lines.push(format!("  Provider: {}", profile.provider));
        lines.push(format!("  Model: {}", profile.model));
        if let Some(ref url) = profile.base_url {
            lines.push(format!("  Base URL: {}", url));
        }
        lines.push(format!("  Edit format: {}", profile.edit_format));

        // Environment variable check
        let env_name = profile.api_key_env.as_deref().unwrap_or("(none)");
        lines.push(format!("  API key env var: {}", env_name));
        if let Some(ref name) = profile.api_key_env {
            match std::env::var(name) {
                Ok(val) => {
                    let masked = if val.len() > 8 {
                        format!("{}...{}", &val[..4], &val[val.len() - 4..])
                    } else {
                        "****".to_string()
                    };
                    lines.push(format!("  Env var status: SET ({})", masked));
                }
                Err(_) => {
                    lines.push(format!("  Env var status: NOT SET"));
                }
            }
        }
    } else {
        lines.push(format!("Active profile '{}' not found!", active));
    }

    // Show all AI-related env vars visible to this process
    lines.push(String::new());
    lines.push("**AI-related env vars visible to process:**".to_string());
    let mut found_any = false;
    for (key, val) in std::env::vars() {
        if key.contains("OPENAI")
            || key.contains("ANTHROPIC")
            || key.contains("OVIM")
            || key.contains("API_KEY")
        {
            let masked = if val.len() > 8 {
                format!("{}...{}", &val[..4], &val[val.len() - 4..])
            } else {
                "****".to_string()
            };
            lines.push(format!("  {} = {}", key, masked));
            found_any = true;
        }
    }
    if !found_any {
        lines.push("  (none found matching OPENAI/ANTHROPIC/OVIM/API_KEY)".to_string());
    }

    let message = lines.join("\n");
    editor.set_hover_info(message);
    CommandResult::Success(SuccessResponse {
        success: true,
        message: None,
        line_count: None,
    })
}

fn handle_workflow_command(editor: &mut Editor, command: &str) -> CommandResult {
    let subcmd = command.strip_prefix("workflow").unwrap_or("").trim();

    match subcmd {
        "" | "list" => {
            if let Err(err) = editor.ensure_workflows_loaded() {
                return CommandResult::Error(ErrorResponse {
                    error: format!("Failed to load workflows: {}", err),
                });
            }
            let names = editor.workflow_names_sorted();
            let message = if names.is_empty() {
                "No workflows found.".to_string()
            } else {
                format!("{} workflow(s):\n{}", names.len(), names.join("\n"))
            };
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(message),
                line_count: None,
            })
        }
        "reload" => match editor.reload_workflows() {
            Ok(count) => CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(format!("Loaded {} workflow(s)", count)),
                line_count: None,
            }),
            Err(err) => CommandResult::Error(ErrorResponse {
                error: format!("Failed to reload workflows: {}", err),
            }),
        },
        "status" => CommandResult::Success(SuccessResponse {
            success: true,
            message: Some(editor.workflow_status_report()),
            line_count: None,
        }),
        s if s.starts_with("run ") => {
            let mut parts = s["run ".len()..].split_whitespace();
            let Some(name) = parts.next() else {
                return CommandResult::Error(ErrorResponse {
                    error: "Usage: :workflow run <name> [k=v ...]".to_string(),
                });
            };

            let mut inputs = std::collections::BTreeMap::new();
            for pair in parts {
                let Some((key, raw_value)) = pair.split_once('=') else {
                    return CommandResult::Error(ErrorResponse {
                        error: format!("Invalid input '{}': expected k=v", pair),
                    });
                };
                let value = serde_json::from_str::<serde_json::Value>(raw_value)
                    .unwrap_or_else(|_| serde_json::Value::String(raw_value.to_string()));
                inputs.insert(key.to_string(), value);
            }

            match editor.run_workflow(name, inputs) {
                Ok(run_id) => CommandResult::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Workflow '{}' started (run #{})", name, run_id)),
                    line_count: None,
                }),
                Err(err) => CommandResult::Error(ErrorResponse {
                    error: format!("Failed to run workflow '{}': {}", name, err),
                }),
            }
        }
        _ => CommandResult::Error(ErrorResponse {
            error: format!(
                "Unknown workflow subcommand '{}'. Usage: :workflow [list|reload|run <name> [k=v ...]|status]",
                subcmd
            ),
        }),
    }
}

fn handle_session_command(editor: &mut Editor, command: &str) -> CommandResult {
    use crate::session::SessionInfo;

    let subcmd = command.strip_prefix("session").unwrap_or("").trim();

    match subcmd {
        "" | "list" => {
            // Show active sessions
            match SessionInfo::list_all() {
                Ok(sessions) if sessions.is_empty() => {
                    let msg = if editor.active_session.is_some() {
                        format!(
                            "Active session: {}",
                            editor.active_session.as_ref().unwrap()
                        )
                    } else {
                        "No registered sessions. Use :session start NAME to register.".to_string()
                    };
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some(msg),
                        line_count: None,
                    })
                }
                Ok(sessions) => {
                    let mut msg = format!("{} active session(s):", sessions.len());
                    for s in &sessions {
                        let marker = if editor.active_session.as_deref() == Some(&s.session_name) {
                            " (this)"
                        } else {
                            ""
                        };
                        msg.push_str(&format!(
                            "\n  {} (PID {}, port {}){}",
                            s.session_name, s.pid, s.port, marker
                        ));
                    }
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some(msg),
                        line_count: None,
                    })
                }
                Err(e) => CommandResult::Error(ErrorResponse {
                    error: format!("Failed to list sessions: {}", e),
                }),
            }
        }
        s if s.starts_with("start ") => {
            let name = s["start ".len()..].trim();
            if name.is_empty() {
                return CommandResult::Error(ErrorResponse {
                    error: "Usage: :session start NAME".to_string(),
                });
            }

            // Validate session name (alphanumeric, underscore, hyphen only)
            if !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                return CommandResult::Error(ErrorResponse {
                    error: "Session name must contain only alphanumeric characters, underscores, and hyphens".to_string(),
                });
            }

            // Check if already registered
            if let Some(ref existing) = editor.active_session {
                return CommandResult::Error(ErrorResponse {
                    error: format!(
                        "Already registered as session '{}'. Use :session stop first.",
                        existing
                    ),
                });
            }

            // Need API port to register
            let port = match editor.api_port {
                Some(p) => p,
                None => {
                    return CommandResult::Error(ErrorResponse {
                        error: "API server not running".to_string(),
                    });
                }
            };

            let file = editor.buffer().file_path().map(|s| s.to_string());

            let session_info = SessionInfo::new(port, file, name.to_string());
            match session_info.write() {
                Ok(()) => {
                    editor.active_session = Some(name.to_string());
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("Session '{}' registered", name)),
                        line_count: None,
                    })
                }
                Err(e) => CommandResult::Error(ErrorResponse {
                    error: format!("Failed to register session: {}", e),
                }),
            }
        }
        "stop" => {
            match editor.active_session.take() {
                Some(name) => {
                    // Delete the session file
                    let port = editor.api_port.unwrap_or(0);
                    let session_info = SessionInfo::new(port, None, name.clone());
                    let _ = session_info.delete();
                    CommandResult::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("Session '{}' unregistered", name)),
                        line_count: None,
                    })
                }
                None => CommandResult::Error(ErrorResponse {
                    error: "No active session to stop".to_string(),
                }),
            }
        }
        _ => CommandResult::Error(ErrorResponse {
            error: format!(
                "Unknown session subcommand: '{}'. Usage: :session [start NAME|stop|list]",
                subcmd
            ),
        }),
    }
}

fn handle_debug_command(editor: &mut Editor, command: &str) -> CommandResult {
    use crate::dap::PendingDebugAction;

    let subcmd = command.strip_prefix("debug").unwrap_or("").trim();

    match subcmd {
        "breakpoint" | "bp" => {
            editor.toggle_breakpoint();
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Breakpoint toggled".to_string()),
                line_count: None,
            })
        }
        "panels" => {
            editor.toggle_debug_panels();
            CommandResult::Success(SuccessResponse {
                success: true,
                message: None,
                line_count: None,
            })
        }
        "continue" | "c" => {
            editor.dap_manager_mut().pending_action = Some(PendingDebugAction::Continue);
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Continue".to_string()),
                line_count: None,
            })
        }
        "next" | "n" | "step" => {
            editor.dap_manager_mut().pending_action = Some(PendingDebugAction::StepOver);
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Step over".to_string()),
                line_count: None,
            })
        }
        "stepin" | "si" => {
            editor.dap_manager_mut().pending_action = Some(PendingDebugAction::StepIn);
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Step in".to_string()),
                line_count: None,
            })
        }
        "stepout" | "so" => {
            editor.dap_manager_mut().pending_action = Some(PendingDebugAction::StepOut);
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Step out".to_string()),
                line_count: None,
            })
        }
        "stop" => {
            editor.dap_manager_mut().pending_action = Some(PendingDebugAction::Stop);
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some("Stopping debug session".to_string()),
                line_count: None,
            })
        }
        "start" => {
            // Auto-detect DAP adapter from the current file's language config
            let dap_config = editor
                .buffer()
                .file_path()
                .and_then(|fp| {
                    crate::language_config::LanguageRegistry::try_get()
                        .and_then(|reg| reg.detect(fp))
                })
                .and_then(|lang| lang.dap.as_ref());

            let Some(config) = dap_config else {
                return CommandResult::Error(ErrorResponse {
                    error: "No DAP adapter configured for this language. Use :debug start <command> [args...]".to_string(),
                });
            };
            let Some(cmd) = crate::language_config::find_dap_command(config) else {
                let hint = config.install_hint.as_deref().unwrap_or("Install the debug adapter and ensure it's in PATH");
                return CommandResult::Error(ErrorResponse {
                    error: format!("DAP adapter '{}' not found. {}", config.command, hint),
                });
            };
            let args = config.args.clone();
            editor.dap_manager_mut().pending_action = Some(PendingDebugAction::Start {
                command: cmd.clone(),
                args: args.clone(),
                run_config: None,
            });
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(format!("Starting debug adapter: {} {}", cmd, args.join(" "))),
                line_count: None,
            })
        }
        s if s.starts_with("start ") => {
            let rest = s["start ".len()..].trim();
            let mut parts = rest.split_whitespace();
            let Some(cmd) = parts.next() else {
                return CommandResult::Error(ErrorResponse {
                    error: "Usage: :debug start [command] [args...]".to_string(),
                });
            };
            let args: Vec<String> = parts.map(String::from).collect();
            editor.dap_manager_mut().pending_action = Some(PendingDebugAction::Start {
                command: cmd.to_string(),
                args,
                run_config: None,
            });
            CommandResult::Success(SuccessResponse {
                success: true,
                message: Some(format!("Starting debug adapter: {}", cmd)),
                line_count: None,
            })
        }
        "" => CommandResult::Success(SuccessResponse {
            success: true,
            message: Some(
                "Usage: :debug [start <cmd>|stop|continue|next|stepin|stepout|breakpoint|panels]"
                    .to_string(),
            ),
            line_count: None,
        }),
        _ => CommandResult::Error(ErrorResponse {
            error: format!(
                "Unknown debug subcommand: '{}'. Usage: :debug [start|stop|continue|next|stepin|stepout|breakpoint|panels]",
                subcmd
            ),
        }),
    }
}

/// Handle :set commands for options.
///
/// **Deprecated**: Use [`crate::cmd_set::handle_set_command`] directly.
/// This wrapper exists for backwards compatibility with any external callers.
pub fn handle_set_command(editor: &mut Editor, args: &str) -> CommandResult {
    crate::cmd_set::handle_set_command(editor, args)
}
