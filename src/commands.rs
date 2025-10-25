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
        }
        "q!" | "quit!" => {
            editor.quit();
            ApiResponse::Success(SuccessResponse {
                success: true,
                message: Some("Quitting (forced)".to_string()),
                line_count: None,
            })
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
                // Create new tab and load file
                editor.new_tab(None);
                match editor.load_file(filename) {
                    Ok(_) => {
                        let tab_index = editor.current_tab_index() + 1;
                        ApiResponse::Success(SuccessResponse {
                            success: true,
                            message: Some(format!("Opened {} in tab {}", filename, tab_index)),
                            line_count: None,
                        })
                    }
                    Err(e) => ApiResponse::Error(ErrorResponse {
                        error: format!("Failed to load file: {}", e),
                    }),
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
            } else {
                ApiResponse::Error(ErrorResponse {
                    error: format!("Not an editor command: {}", command),
                })
            }
        }
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
