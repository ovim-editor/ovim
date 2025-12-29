//! Command execution for ex commands (:w, :q, etc.)

use crate::api::{ApiResponse, ErrorResponse, SuccessResponse};
use crate::editor::Editor;
use crate::editor::QuickfixEntry;

/// Helper function to jump to a quickfix entry
pub fn jump_to_quickfix_entry(editor: &mut Editor, entry: &QuickfixEntry) -> ApiResponse {
    if let Some(ref path) = entry.filename {
        // Load the file if needed
        let path_str = path.to_string_lossy().to_string();
        if let Err(e) = editor.load_file(&path_str) {
            return ApiResponse::Error(ErrorResponse {
                error: format!("Failed to load file: {}", e),
            });
        }

        // Jump to line/column (convert from 1-indexed to 0-indexed)
        let line = entry.lnum.saturating_sub(1);
        let col = entry.col.saturating_sub(1);
        editor.buffer_mut().cursor_mut().set_position(line, col);

        ApiResponse::Success(SuccessResponse {
            success: true,
            message: Some(entry.display_text()),
            line_count: None,
        })
    } else {
        ApiResponse::Success(SuccessResponse {
            success: true,
            message: Some(entry.text.clone()),
            line_count: None,
        })
    }
}

/// Execute a command (e.g., :w, :q, :tabnew)
pub fn execute_command(editor: &mut Editor, command: &str) -> ApiResponse {
    match command {
        "q" | "quit" => {
            // If there are multiple tabs, close current tab. Otherwise quit.
            if editor.tab_page_manager().is_single_tab() {
                // Single tab - check modifications and quit
                if editor.is_modified() {
                    ApiResponse::Error(ErrorResponse {
                        error: "No write since last change (add ! to override)".to_string(),
                    })
                } else {
                    editor.quit();
                    ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some("Quitting".to_string()),
                        line_count: None,
                    })
                }
            } else {
                // Multiple tabs - close current tab
                editor.close_current_tab();
                let tab_index = editor.current_tab_index() + 1;
                ApiResponse::Success(SuccessResponse {
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
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some("Quitting (forced)".to_string()),
                    line_count: None,
                })
            } else {
                editor.close_current_tab();
                let tab_index = editor.current_tab_index() + 1;
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Tab closed. Now on tab {}", tab_index)),
                    line_count: None,
                })
            }
        }
        "qa" | "qall" => {
            // Quit all - same as quit for single buffer
            if editor.is_modified() {
                ApiResponse::Error(ErrorResponse {
                    error: "No write since last change (add ! to override)".to_string(),
                })
            } else {
                editor.quit();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some("Quitting all".to_string()),
                    line_count: None,
                })
            }
        }
        "qa!" | "qall!" => {
            // Force quit all without saving
            editor.quit();
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("Quitting all (forced)".to_string()),
                line_count: None,
            })
        }
        "w" | "write" => {
            if let Some(path) = editor.buffer().file_path().map(|s| s.to_string()) {
                match editor.buffer_mut().save_as(&path) {
                    Ok(_) => {
                        editor.mark_saved();
                        editor.mark_buffer_saved(); // Mark for LSP didSave notification
                        let line_count = editor.buffer().rope().len_lines();
                        let char_count = editor.buffer().rope().len_chars();
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!(
                                "\"{}\" {}L, {}C written",
                                path, line_count, char_count
                            )),
                            line_count: None,
                        })
                    }
                    Err(e) => ApiResponse::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            } else {
                ApiResponse::Error(ErrorResponse {
                    error: "No file name".to_string(),
                })
            }
        }
        "wq" => {
            if let Some(path) = editor.buffer().file_path().map(|s| s.to_string()) {
                match editor.buffer_mut().save_as(&path) {
                    Ok(_) => {
                        editor.mark_saved();
                        editor.mark_buffer_saved(); // Mark for LSP didSave notification
                        editor.quit();
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some("Saved and quitting".to_string()),
                            line_count: None,
                        })
                    }
                    Err(e) => ApiResponse::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            } else {
                ApiResponse::Error(ErrorResponse {
                    error: "No file name".to_string(),
                })
            }
        }
        "LspInfo" => {
            // Show LSP status information
            let mut info = String::new();

            if editor.lsp_manager().is_none() {
                info.push_str("LSP is not enabled\n");
            } else if editor.active_lsp_servers().is_empty() {
                info.push_str("No active LSP servers\n");
                if !editor.lsp_status().is_empty() {
                    info.push_str(&format!("Status: {}\n", editor.lsp_status()));
                }
            } else {
                info.push_str("Active LSP servers:\n");
                for (lang_id, server_name) in editor.active_lsp_servers() {
                    info.push_str(&format!("  {} -> {}\n", lang_id, server_name));
                }

                let (errors, warnings, info_count, hints) = editor.cached_diagnostic_count();
                info.push_str(&format!(
                    "\nDiagnostics: {} errors, {} warnings, {} info, {} hints\n",
                    errors, warnings, info_count, hints
                ));

                if !editor.lsp_status().is_empty() {
                    info.push_str(&format!("Status: {}\n", editor.lsp_status()));
                }

                if let Some(file_path) = editor.buffer().file_path() {
                    info.push_str(&format!("Current file: {}\n", file_path));
                }
            }

            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some(info),
                line_count: None,
            })
        }
        "copen" => {
            // Open/show quickfix list
            let qf_list = editor.quickfix_list();
            if qf_list.is_empty() {
                ApiResponse::Success(SuccessResponse {
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
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(message),
                    line_count: None,
                })
            }
        }
        "cclose" | "ccl" => {
            // Close/clear quickfix list
            editor.quickfix_list_mut().clear();
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("Quickfix list cleared".to_string()),
                line_count: None,
            })
        }
        "cnext" | "cn" => {
            // Jump to next quickfix entry
            if editor.quickfix_list().is_empty() {
                ApiResponse::Error(ErrorResponse {
                    error: "Quickfix list is empty".to_string(),
                })
            } else {
                editor.quickfix_list_mut().next();
                if let Some(entry) = editor.quickfix_list().current_entry().cloned() {
                    crate::commands::jump_to_quickfix_entry(editor, &entry)
                } else {
                    ApiResponse::Error(ErrorResponse {
                        error: "No current entry".to_string(),
                    })
                }
            }
        }
        "cprev" | "cp" | "cprevious" => {
            // Jump to previous quickfix entry
            if editor.quickfix_list().is_empty() {
                ApiResponse::Error(ErrorResponse {
                    error: "Quickfix list is empty".to_string(),
                })
            } else {
                editor.quickfix_list_mut().previous();
                if let Some(entry) = editor.quickfix_list().current_entry().cloned() {
                    crate::commands::jump_to_quickfix_entry(editor, &entry)
                } else {
                    ApiResponse::Error(ErrorResponse {
                        error: "No current entry".to_string(),
                    })
                }
            }
        }
        "cfirst" | "cfir" => {
            // Jump to first quickfix entry
            if editor.quickfix_list().is_empty() {
                ApiResponse::Error(ErrorResponse {
                    error: "Quickfix list is empty".to_string(),
                })
            } else {
                editor.quickfix_list_mut().first();
                if let Some(entry) = editor.quickfix_list().current_entry().cloned() {
                    crate::commands::jump_to_quickfix_entry(editor, &entry)
                } else {
                    ApiResponse::Error(ErrorResponse {
                        error: "No current entry".to_string(),
                    })
                }
            }
        }
        "clast" | "cla" => {
            // Jump to last quickfix entry
            if editor.quickfix_list().is_empty() {
                ApiResponse::Error(ErrorResponse {
                    error: "Quickfix list is empty".to_string(),
                })
            } else {
                editor.quickfix_list_mut().last();
                if let Some(entry) = editor.quickfix_list().current_entry().cloned() {
                    crate::commands::jump_to_quickfix_entry(editor, &entry)
                } else {
                    ApiResponse::Error(ErrorResponse {
                        error: "No current entry".to_string(),
                    })
                }
            }
        }
        "tabnew" | "tabe" | "tabedit" => {
            // Create new tab with default name
            editor.new_tab(None);
            let tab_index = editor.current_tab_index() + 1; // 1-indexed for display
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some(format!("Created tab {}", tab_index)),
                line_count: None,
            })
        }
        "tabnext" | "tabn" => {
            // Switch to next tab
            editor.next_tab();
            let tab_index = editor.current_tab_index() + 1; // 1-indexed for display
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some(format!("Tab {}", tab_index)),
                line_count: None,
            })
        }
        "tabprev" | "tabp" | "tabprevious" => {
            // Switch to previous tab
            editor.previous_tab();
            let tab_index = editor.current_tab_index() + 1; // 1-indexed for display
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some(format!("Tab {}", tab_index)),
                line_count: None,
            })
        }
        "tabfirst" | "tabfir" => {
            // Switch to first tab
            editor.first_tab();
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("Tab 1".to_string()),
                line_count: None,
            })
        }
        "tablast" | "tabl" => {
            // Switch to last tab
            editor.last_tab();
            let tab_index = editor.current_tab_index() + 1; // 1-indexed for display
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some(format!("Tab {}", tab_index)),
                line_count: None,
            })
        }
        "tabclose" | "tabc" => {
            // Close current tab
            if editor.tab_page_manager().is_single_tab() {
                ApiResponse::Error(ErrorResponse {
                    error: "Cannot close last tab".to_string(),
                })
            } else {
                editor.close_current_tab();
                let tab_index = editor.current_tab_index() + 1; // 1-indexed for display
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Tab closed. Now on tab {}", tab_index)),
                    line_count: None,
                })
            }
        }
        "tabonly" | "tabo" => {
            // Close all tabs except the current one
            if editor.tab_page_manager().is_single_tab() {
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some("Already only one tab".to_string()),
                    line_count: None,
                })
            } else {
                let closed_count = editor.tab_count() - 1;
                editor.close_other_tabs();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Closed {} tabs", closed_count)),
                    line_count: None,
                })
            }
        }
        "noh" | "nohlsearch" => {
            // Clear search highlighting
            editor.clear_search_highlight();
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("Search highlighting cleared".to_string()),
                line_count: None,
            })
        }
        "reg" | "registers" => {
            // Display all registers
            let registers = editor.registers().list_registers();
            if registers.is_empty() {
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some("No registers in use".to_string()),
                    line_count: None,
                })
            } else {
                let display: Vec<String> = registers
                    .iter()
                    .map(|(name, content)| format!("{}: {}", name, content))
                    .collect();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(display.join("\n")),
                    line_count: None,
                })
            }
        }
        "j" | "join" => {
            // Join current line with the next line
            use crate::editor::Operators;
            if let Err(e) = Operators::join_lines(editor.buffer_mut(), 1) {
                return ApiResponse::Error(ErrorResponse {
                    error: format!("Failed to join lines: {}", e),
                });
            }
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: None,
                line_count: None,
            })
        }
        "recover" | "rec" => {
            // Recover buffer content from swap file
            if !editor.buffer().has_swap_file() {
                return ApiResponse::Error(ErrorResponse {
                    error: "No swap file exists for this buffer".to_string(),
                });
            }
            match editor.buffer_mut().recover_from_swap_file() {
                Ok(true) => ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some("Buffer recovered from swap file".to_string()),
                    line_count: None,
                }),
                Ok(false) => ApiResponse::Error(ErrorResponse {
                    error: "Failed to recover: swap file is empty or missing".to_string(),
                }),
                Err(e) => ApiResponse::Error(ErrorResponse {
                    error: format!("Failed to recover: {}", e),
                }),
            }
        }
        "checktime" => {
            // Check if file has been modified externally and reload if so
            match editor.buffer().check_external_modification() {
                Ok(true) => {
                    match editor.buffer_mut().reload_if_changed_sync() {
                        Ok(true) => ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some("File reloaded from disk (external changes detected)".to_string()),
                            line_count: None,
                        }),
                        Ok(false) => ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some("No external changes detected".to_string()),
                            line_count: None,
                        }),
                        Err(e) => ApiResponse::Error(ErrorResponse {
                            error: format!("Failed to reload: {}", e),
                        }),
                    }
                }
                Ok(false) => ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some("No external changes detected".to_string()),
                    line_count: None,
                }),
                Err(e) => ApiResponse::Error(ErrorResponse {
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
                lines.push(format!(
                    " '{}  {:>5}  {:>3}  {}",
                    name,
                    mark.line + 1,
                    mark.col,
                    mark.file_path
                ));
            }

            if lines.is_empty() {
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some("No marks set".to_string()),
                    line_count: None,
                })
            } else {
                lines.insert(0, "mark  line   col  file".to_string());
                ApiResponse::Success(SuccessResponse {
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
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some(tab_list.join("\n")),
                line_count: None,
            })
        }
        _ => {
            // Handle :tabnew <filename>, :tabe <filename>, :tabedit <filename>
            if let Some(filename) = command
                .strip_prefix("tabnew ")
                .or_else(|| command.strip_prefix("tabe "))
                .or_else(|| command.strip_prefix("tabedit "))
            {
                // Create new tab and load file (or create if doesn't exist)
                editor.new_tab(None);

                // Try to load the file, if it doesn't exist create an empty buffer
                match editor.load_file(filename) {
                    Ok(_) => {
                        let tab_index = editor.current_tab_index() + 1;
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Opened {} in tab {}", filename, tab_index)),
                            line_count: None,
                        })
                    }
                    Err(e) => {
                        // Check if error is because file doesn't exist
                        if e.to_string().contains("Failed to read file") || e.to_string().contains("No such file") {
                            // Create a new empty buffer with the given filename
                            use crate::buffer::Buffer;
                            let mut new_buffer = Buffer::new();
                            // Normalize the path
                            let absolute_path = std::path::absolute(filename)
                                .unwrap_or_else(|_| std::path::PathBuf::from(filename));
                            let path_str = absolute_path.to_string_lossy().to_string();
                            new_buffer.set_file_path(path_str.clone());
                            editor.add_buffer(new_buffer);
                            editor.mark_dirty();

                            // Update tab title to match the new file
                            editor.update_current_tab_title();

                            // Sync tab's buffer index to match the new buffer
                            editor.sync_current_tab_buffer_index();

                            let tab_index = editor.current_tab_index() + 1;
                            ApiResponse::Success(SuccessResponse {
                                success: true,
                                message: Some(format!("Created new file {} in tab {}", filename, tab_index)),
                                line_count: None,
                            })
                        } else {
                            ApiResponse::Error(ErrorResponse {
                                error: format!("Failed to load file: {}", e),
                            })
                        }
                    }
                }
            // Handle :w <filename>
            } else if let Some(filename) = command
                .strip_prefix("w ")
                .or_else(|| command.strip_prefix("write "))
            {
                editor.buffer_mut().set_file_path(filename.to_string());
                match editor.buffer_mut().save_as(filename) {
                    Ok(_) => {
                        editor.mark_saved();
                        editor.mark_buffer_saved(); // Mark for LSP didSave notification
                        let line_count = editor.buffer().rope().len_lines();
                        let char_count = editor.buffer().rope().len_chars();
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!(
                                "\"{}\" {}L, {}C written",
                                filename, line_count, char_count
                            )),
                            line_count: None,
                        })
                    }
                    Err(e) => ApiResponse::Error(ErrorResponse {
                        error: format!("Failed to save: {}", e),
                    }),
                }
            // Handle :lua <code>
            } else if let Some(_code) = command.strip_prefix("lua ") {
                #[cfg(feature = "lua")]
                {
                    match editor.execute_lua(_code) {
                        Ok(result) => ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(result),
                            line_count: None,
                        }),
                        Err(e) => ApiResponse::Error(ErrorResponse {
                            error: format!("Lua error: {}", e),
                        }),
                    }
                }
                #[cfg(not(feature = "lua"))]
                {
                    ApiResponse::Error(ErrorResponse {
                        error: "Lua support not enabled".to_string(),
                    })
                }
            // Handle :luafile <path>
            } else if let Some(_path) = command.strip_prefix("luafile ") {
                #[cfg(feature = "lua")]
                {
                    match editor.execute_lua_file(_path) {
                        Ok(_) => ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Executed {}", _path)),
                            line_count: None,
                        }),
                        Err(e) => ApiResponse::Error(ErrorResponse {
                            error: format!("Lua error: {}", e),
                        }),
                    }
                }
                #[cfg(not(feature = "lua"))]
                {
                    ApiResponse::Error(ErrorResponse {
                        error: "Lua support not enabled".to_string(),
                    })
                }
            // Handle :colorscheme <name> or :colorscheme (to show current)
            // Also support :colo abbreviation
            } else if command == "colorscheme" || command == "colo" {
                let current = editor.current_color_scheme_name();
                let schemes = editor.list_color_schemes().join(", ");
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!("Current: {}\nAvailable: {}", current, schemes)),
                    line_count: None,
                })
            } else if let Some(scheme_name) = command
                .strip_prefix("colorscheme ")
                .or_else(|| command.strip_prefix("colo "))
            {
                match editor.set_color_scheme(scheme_name.trim()) {
                    Ok(_) => ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("Color scheme set to '{}'", scheme_name.trim())),
                        line_count: None,
                    }),
                    Err(e) => {
                        let available = editor.list_color_schemes().join(", ");
                        ApiResponse::Error(ErrorResponse {
                            error: format!("{}. Available schemes: {}", e, available),
                        })
                    }
                }
            // Handle :set commands
            } else if let Some(set_cmd) = command
                .strip_prefix("set ")
                .or_else(|| command.strip_prefix("se "))
            {
                crate::commands::handle_set_command(editor, set_cmd.trim())
            // Handle split commands
            } else if command == "sp" || command == "split" {
                editor.split_window_horizontal();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!(
                        "Split horizontally ({} windows)",
                        editor.window_count()
                    )),
                    line_count: None,
                })
            } else if command == "vsp" || command == "vsplit" {
                editor.split_window_vertical();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(format!(
                        "Split vertically ({} windows)",
                        editor.window_count()
                    )),
                    line_count: None,
                })
            // Handle config reload
            } else if command == "ConfigReload" || command == "reload" {
                #[cfg(feature = "lua")]
                {
                    match editor.reload_lua_config() {
                        Ok(msg) => ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(msg),
                            line_count: None,
                        }),
                        Err(e) => ApiResponse::Error(ErrorResponse {
                            error: format!("Failed to reload config: {}", e),
                        }),
                    }
                }
                #[cfg(not(feature = "lua"))]
                {
                    ApiResponse::Error(ErrorResponse {
                        error: "Lua support not enabled".to_string(),
                    })
                }
            // Handle :source - load and execute a Lua file
            } else if let Some(file) = command.strip_prefix("source ").or_else(|| command.strip_prefix("so ")) {
                let file = file.trim();
                #[cfg(feature = "lua")]
                {
                    if let Some(ref mut context) = editor.lua_context {
                        let path = std::path::PathBuf::from(file);
                        match context.execute_file(&path) {
                            Ok(_) => {
                                // Process any commands from the sourced file
                                if let Some(ref bridge) = editor.editor_bridge {
                                    let commands = bridge.drain_commands();
                                    for cmd in commands {
                                        let _ = crate::editor::input::InputHandler::execute_command_string(editor, &cmd);
                                    }
                                }
                                ApiResponse::Success(SuccessResponse {
                                    success: true,
                                    message: Some(format!("Sourced: {}", file)),
                                    line_count: None,
                                })
                            }
                            Err(e) => ApiResponse::Error(ErrorResponse {
                                error: format!("Failed to source {}: {}", file, e),
                            }),
                        }
                    } else {
                        ApiResponse::Error(ErrorResponse {
                            error: "Lua not enabled".to_string(),
                        })
                    }
                }
                #[cfg(not(feature = "lua"))]
                {
                    let _ = file;
                    ApiResponse::Error(ErrorResponse {
                        error: "Lua support not enabled".to_string(),
                    })
                }
            // Handle :bn (next buffer)
            } else if command == "bn" || command == "bnext" {
                editor.next_buffer();
                let buf_name = editor
                    .buffer()
                    .file_path()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "[No Name]".to_string());
                ApiResponse::Success(SuccessResponse {
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
                ApiResponse::Success(SuccessResponse {
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
                if editor.buffer().is_modified() {
                    ApiResponse::Error(ErrorResponse {
                        error: "No write since last change (add ! to override)".to_string(),
                    })
                } else {
                    let should_quit = editor.delete_current_buffer();
                    if should_quit {
                        editor.quit();
                        ApiResponse::Success(SuccessResponse {
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
                        ApiResponse::Success(SuccessResponse {
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
                    ApiResponse::Success(SuccessResponse {
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
                    ApiResponse::Success(SuccessResponse {
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
                        let modified =
                            if i < editor.buffer_count() && editor.buffers[i].is_modified() {
                                "+"
                            } else {
                                " "
                            };
                        format!("{} {}  {}", marker, modified, name)
                    })
                    .collect();
                ApiResponse::Success(SuccessResponse {
                    success: true,
                    message: Some(buf_list.join("\n")),
                    line_count: None,
                })
            } else if let Some(filename) = command
                .strip_prefix("e ")
                .or_else(|| command.strip_prefix("edit "))
            {
                // :e <filename> - edit file
                match editor.load_file(filename) {
                    Ok(_) => {
                        let buf_name = editor
                            .buffer()
                            .file_path()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "[No Name]".to_string());
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Editing: {}", buf_name)),
                            line_count: None,
                        })
                    }
                    Err(e) => ApiResponse::Error(ErrorResponse {
                        error: format!("Failed to load file: {}", e),
                    }),
                }
            // Handle :! shell command execution
            } else if let Some(shell_cmd) = command.strip_prefix('!') {
                if shell_cmd.trim().is_empty() {
                    ApiResponse::Error(ErrorResponse {
                        error: "No shell command specified".to_string(),
                    })
                } else {
                    execute_shell_command(shell_cmd.trim())
                }
            } else {
                ApiResponse::Error(ErrorResponse {
                    error: format!("Not an editor command: {}", command),
                })
            }
        }
    }
}

/// Execute a shell command and return the output
fn execute_shell_command(cmd: &str) -> ApiResponse {
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
                    ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some("Command executed successfully".to_string()),
                        line_count: None,
                    })
                } else {
                    ApiResponse::Success(SuccessResponse {
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
                    ApiResponse::Error(ErrorResponse {
                        error: format!("Command failed{}", exit_code),
                    })
                } else {
                    ApiResponse::Error(ErrorResponse {
                        error: format!("{}\n\nCommand failed{}", result, exit_code),
                    })
                }
            }
        }
        Err(e) => ApiResponse::Error(ErrorResponse {
            error: format!("Failed to execute command: {}", e),
        }),
    }
}

/// Handle :set commands for options
pub fn handle_set_command(editor: &mut Editor, args: &str) -> ApiResponse {
    // Handle empty :set (show all options)
    if args.is_empty() {
        let opts = &editor.options;
        let msg = format!(
            "  {}number\n  {}relativenumber\n  {}expandtab\n  tabstop={}\n  shiftwidth={}\n  scroll={}",
            if opts.number { "" } else { "no" },
            if opts.relative_number { "" } else { "no" },
            if opts.expand_tab { "" } else { "no" },
            opts.tab_width,
            opts.shift_width,
            opts.scroll.map(|s| s.to_string()).unwrap_or_else(|| "auto".to_string())
        );
        return ApiResponse::Success(SuccessResponse {
            success: true,
            message: Some(msg),
            line_count: None,
        });
    }

    // Parse option
    let (opt_name, opt_value) = if let Some((name, value)) = args.split_once('=') {
        (name.trim(), Some(value.trim()))
    } else {
        (args, None)
    };

    // Check for query (option?)
    if let Some(query_opt) = opt_name.strip_suffix('?') {
        let opts = &editor.options;
        let msg = match query_opt {
            "number" | "nu" => format!("  {}number", if opts.number { "" } else { "no" }),
            "relativenumber" | "rnu" => format!(
                "  {}relativenumber",
                if opts.relative_number { "" } else { "no" }
            ),
            "expandtab" | "et" => format!("  {}expandtab", if opts.expand_tab { "" } else { "no" }),
            "tabstop" | "ts" => format!("  tabstop={}", opts.tab_width),
            "shiftwidth" | "sw" => format!("  shiftwidth={}", opts.shift_width),
            "scroll" => format!(
                "  scroll={}",
                opts.scroll
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "auto".to_string())
            ),
            "textwidth" | "tw" => format!(
                "  textwidth={}",
                opts.textwidth
                    .map(|w| w.to_string())
                    .unwrap_or_else(|| "0".to_string())
            ),
            "ignorecase" | "ic" => format!("  {}ignorecase", if opts.ignorecase { "" } else { "no" }),
            "smartcase" | "scs" => format!("  {}smartcase", if opts.smartcase { "" } else { "no" }),
            "cursorline" | "cul" => format!("  {}cursorline", if opts.cursorline { "" } else { "no" }),
            "showmatch" | "sm" => format!("  {}showmatch", if opts.showmatch { "" } else { "no" }),
            "swapfile" | "swf" => format!("  {}swapfile", if opts.swapfile { "" } else { "no" }),
            "backup" | "bk" => format!("  {}backup", if opts.backup { "" } else { "no" }),
            _ => {
                return ApiResponse::Error(ErrorResponse {
                    error: format!("Unknown option: {}", query_opt),
                })
            }
        };
        return ApiResponse::Success(SuccessResponse {
            success: true,
            message: Some(msg),
            line_count: None,
        });
    }

    // Handle boolean options
    match opt_name {
        "number" | "nu" => {
            editor.options.number = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  number".to_string()),
                line_count: None,
            });
        }
        "nonumber" | "nonu" => {
            editor.options.number = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  nonumber".to_string()),
                line_count: None,
            });
        }
        "relativenumber" | "rnu" => {
            editor.options.relative_number = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  relativenumber".to_string()),
                line_count: None,
            });
        }
        "norelativenumber" | "nornu" => {
            editor.options.relative_number = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  norelativenumber".to_string()),
                line_count: None,
            });
        }
        "expandtab" | "et" => {
            editor.options.expand_tab = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  expandtab".to_string()),
                line_count: None,
            });
        }
        "noexpandtab" | "noet" => {
            editor.options.expand_tab = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  noexpandtab".to_string()),
                line_count: None,
            });
        }
        "ignorecase" | "ic" => {
            editor.options.ignorecase = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  ignorecase".to_string()),
                line_count: None,
            });
        }
        "noignorecase" | "noic" => {
            editor.options.ignorecase = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  noignorecase".to_string()),
                line_count: None,
            });
        }
        "smartcase" | "scs" => {
            editor.options.smartcase = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  smartcase".to_string()),
                line_count: None,
            });
        }
        "nosmartcase" | "noscs" => {
            editor.options.smartcase = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  nosmartcase".to_string()),
                line_count: None,
            });
        }
        "cursorline" | "cul" => {
            editor.options.cursorline = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  cursorline".to_string()),
                line_count: None,
            });
        }
        "nocursorline" | "nocul" => {
            editor.options.cursorline = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  nocursorline".to_string()),
                line_count: None,
            });
        }
        "showmatch" | "sm" => {
            editor.options.showmatch = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  showmatch".to_string()),
                line_count: None,
            });
        }
        "noshowmatch" | "nosm" => {
            editor.options.showmatch = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  noshowmatch".to_string()),
                line_count: None,
            });
        }
        "swapfile" | "swf" => {
            editor.options.swapfile = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  swapfile".to_string()),
                line_count: None,
            });
        }
        "noswapfile" | "noswf" => {
            editor.options.swapfile = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  noswapfile".to_string()),
                line_count: None,
            });
        }
        "backup" | "bk" => {
            editor.options.backup = true;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  backup".to_string()),
                line_count: None,
            });
        }
        "nobackup" | "nobk" => {
            editor.options.backup = false;
            return ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("  nobackup".to_string()),
                line_count: None,
            });
        }
        _ => {}
    }

    // Handle value-based options
    if let Some(value) = opt_value {
        match opt_name {
            "tabstop" | "ts" => match value.parse::<usize>() {
                Ok(n) if n > 0 && n <= 16 => {
                    editor.options.tab_width = n;
                    ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("  tabstop={}", n)),
                        line_count: None,
                    })
                }
                Ok(_) => ApiResponse::Error(ErrorResponse {
                    error: "tabstop must be between 1 and 16".to_string(),
                }),
                Err(_) => ApiResponse::Error(ErrorResponse {
                    error: format!("Invalid number: {}", value),
                }),
            },
            "shiftwidth" | "sw" => match value.parse::<usize>() {
                Ok(n) if n > 0 && n <= 16 => {
                    editor.options.shift_width = n;
                    ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("  shiftwidth={}", n)),
                        line_count: None,
                    })
                }
                Ok(_) => ApiResponse::Error(ErrorResponse {
                    error: "shiftwidth must be between 1 and 16".to_string(),
                }),
                Err(_) => ApiResponse::Error(ErrorResponse {
                    error: format!("Invalid number: {}", value),
                }),
            },
            "scroll" => match value.parse::<usize>() {
                Ok(n) if n > 0 => {
                    editor.options.scroll = Some(n);
                    ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("  scroll={}", n)),
                        line_count: None,
                    })
                }
                Ok(_) => ApiResponse::Error(ErrorResponse {
                    error: "scroll must be greater than 0".to_string(),
                }),
                Err(_) => ApiResponse::Error(ErrorResponse {
                    error: format!("Invalid number: {}", value),
                }),
            },
            "textwidth" | "tw" => match value.parse::<usize>() {
                Ok(0) => {
                    editor.options.textwidth = None;
                    ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some("  textwidth=0".to_string()),
                        line_count: None,
                    })
                }
                Ok(n) if n >= 20 => {
                    editor.options.textwidth = Some(n);
                    ApiResponse::Success(SuccessResponse {
                        success: true,
                        message: Some(format!("  textwidth={}", n)),
                        line_count: None,
                    })
                }
                Ok(_) => ApiResponse::Error(ErrorResponse {
                    error: "textwidth must be 0 (disabled) or at least 20".to_string(),
                }),
                Err(_) => ApiResponse::Error(ErrorResponse {
                    error: format!("Invalid number: {}", value),
                }),
            },
            _ => ApiResponse::Error(ErrorResponse {
                error: format!("Unknown option: {}", opt_name),
            }),
        }
    } else {
        ApiResponse::Error(ErrorResponse {
            error: format!("Unknown option: {}", opt_name),
        })
    }
}
