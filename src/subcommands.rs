//! Subcommand implementations for controlling ovim sessions
#![allow(clippy::print_stdout)]

use anyhow::{Context, Result};
use serde_json::Value;

use crate::cli::Command;
use crate::client::OvimClient;
use crate::session::SessionInfo;

/// Helper to resolve session - auto-discover if not provided
fn resolve_session(session_name: Option<String>) -> Result<SessionInfo> {
    match session_name {
        Some(name) => SessionInfo::read(&name)
            .context(format!("Failed to find session '{}'", name)),
        None => SessionInfo::auto_discover()
            .context("Failed to auto-discover session")
    }
}

/// Execute a subcommand
pub fn execute_subcommand(command: Command) -> Result<()> {
    match command {
        Command::Sessions => cmd_sessions(),
        Command::Send { session, keys } => cmd_send(session, &keys),
        Command::Exec { session, command } => cmd_exec(session, &command),
        Command::Snapshot { session, format } => cmd_snapshot(session, &format),
        Command::Buffer { session } => cmd_buffer(session),
        Command::Mcp {
            session,
            method,
            params,
            id,
        } => cmd_mcp(session, &method, &params, id),
        Command::Kill { session } => cmd_kill(session),
        Command::Health { session } => cmd_health(session),
        Command::LspStatus { session } => cmd_lsp_status(session),
        Command::Context { session } => cmd_context(session),
        Command::Install {
            editor,
            show_config,
            workspace,
        } => cmd_install(&editor, show_config, workspace),
        Command::McpServer {
            workspace,
            port,
            session,
        } => cmd_mcp_server(workspace, port, session),
        Command::GotoDefinition { session } => cmd_goto_definition(session),
        Command::FindReferences { session } => cmd_find_references(session),
        Command::Hover { session } => cmd_hover(session),
        Command::Search { pattern, session } => cmd_search(&pattern, session),
        Command::NextMatch { session } => cmd_next_match(session),
        Command::Diagnostics { session } => cmd_diagnostics(session),
        Command::Symbols { session } => cmd_symbols(session),
        Command::ListLanguages { verbose } => cmd_list_languages(verbose),
        Command::CheckLsp { file, verbose } => cmd_check_lsp(&file, verbose),
        Command::WaitLsp { session, timeout } => cmd_wait_lsp(session, timeout),
        Command::Cleanup { max_age, dry_run } => cmd_cleanup(max_age, dry_run),
    }
}

/// List all running sessions
fn cmd_sessions() -> Result<()> {
    let sessions = SessionInfo::list_all().context("Failed to list sessions")?;

    if sessions.is_empty() {
        println!("No running ovim sessions");
        return Ok(());
    }

    println!("Running ovim sessions:\n");
    println!(
        "{:<15} {:<8} {:<10} {:<10} FILE",
        "SESSION", "PID", "PORT", "LSP"
    );
    println!("{}", "─".repeat(80));

    for session in sessions {
        let lsp_status = if session.lsp_ready {
            "\x1b[32mready\x1b[0m"
        } else {
            "\x1b[33mpending\x1b[0m"
        };

        let file = session
            .file
            .as_ref()
            .map(|f| {
                // Show just filename if path is long
                if f.len() > 40 {
                    format!("...{}", &f[f.len() - 37..])
                } else {
                    f.clone()
                }
            })
            .unwrap_or_else(|| "-".to_string());

        println!(
            "{:<15} {:<8} {:<10} {:<17} {}",
            session.session_name, session.pid, session.port, lsp_status, file
        );
    }

    Ok(())
}

/// Send keys to a session
fn cmd_send(session_name: Option<String>, keys: &str) -> Result<()> {
    let session = resolve_session(session_name)?;

    let client = OvimClient::new(&session);
    client
        .send_keys(keys)
        .context("Failed to send keys to session")?;

    println!("Keys sent to session '{}'", session.session_name);
    Ok(())
}

/// Execute ex command in a session
fn cmd_exec(session_name: Option<String>, command: &str) -> Result<()> {
    let session = resolve_session(session_name)?;

    let client = OvimClient::new(&session);
    let result = client
        .execute_command(command)
        .context("Failed to execute command")?;

    println!("{}", result);
    Ok(())
}

/// Get snapshot from a session
fn cmd_snapshot(session_name: Option<String>, format: &str) -> Result<()> {
    let session = resolve_session(session_name)?;

    let client = OvimClient::new(&session);
    let snapshot = client.get_snapshot().context("Failed to get snapshot")?;

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
        "pretty" => {
            println!("Session: {}", session.session_name);
            println!("Mode: {}", snapshot.mode);
            println!("Cursor: line {}, col {}", snapshot.cursor.line, snapshot.cursor.column);
            println!("Buffer: {} lines", snapshot.buffer.line_count);
            if let Some(path) = &snapshot.buffer.file_path {
                println!("File: {}", path);
            }
            if let Some(visual) = &snapshot.visual_selection {
                println!(
                    "Visual: ({}, {}) -> ({}, {})",
                    visual.start.line, visual.start.column, visual.end.line, visual.end.column
                );
            }
            println!("Registers: {}", snapshot.registers.len());
            println!("Marks: {}", snapshot.marks.len());
        }
        _ => anyhow::bail!("Unknown format: {}", format),
    }

    Ok(())
}

/// Get buffer content from a session
fn cmd_buffer(session_name: Option<String>) -> Result<()> {
    let session = resolve_session(session_name)?;

    let client = OvimClient::new(&session);
    let buffer = client.get_buffer().context("Failed to get buffer")?;

    print!("{}", buffer.content);
    Ok(())
}

/// Send MCP request to a session
fn cmd_mcp(session_name: Option<String>, method: &str, params_str: &str, id: i64) -> Result<()> {
    let session = resolve_session(session_name)?;

    // Parse params as JSON
    let params: Value = serde_json::from_str(params_str)
        .context(format!("Failed to parse params as JSON: {}", params_str))?;

    let client = OvimClient::new(&session);
    let response = client
        .send_mcp_request(method, params, id)
        .context("Failed to send MCP request")?;

    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

/// Kill a session
fn cmd_kill(session_name: Option<String>) -> Result<()> {
    let session = resolve_session(session_name)?;

    let client = OvimClient::new(&session);
    client
        .kill_session(&session)
        .context("Failed to kill session")?;

    println!("\x1b[32mSession '{}' (PID: {}) killed\x1b[0m", session.session_name, session.pid);
    Ok(())
}

/// Check health of a session
fn cmd_health(session_name: Option<String>) -> Result<()> {
    let session = resolve_session(session_name)?;

    let client = OvimClient::new(&session);
    let health = client.get_health().context("Failed to get health")?;

    println!("Session: {}", session.session_name);
    println!("Status: {}", health.status);
    println!("Uptime: {} seconds", health.uptime_seconds);
    if let Some(file) = &health.file {
        println!("File: {}", file);
    }
    println!("Ready: {}", health.ready);

    if !health.lsp_servers.is_empty() {
        println!("\nLSP Servers:");
        for (lang, status) in &health.lsp_servers {
            let status_colored = match status.as_str() {
                "ready" => format!("\x1b[32m{}\x1b[0m", status),
                "initializing" => format!("\x1b[33m{}\x1b[0m", status),
                _ => status.clone(),
            };
            println!("  {}: {}", lang, status_colored);
        }
    }

    Ok(())
}

/// Get LSP status from a session
fn cmd_lsp_status(session_name: Option<String>) -> Result<()> {
    let session = resolve_session(session_name)?;

    let client = OvimClient::new(&session);
    let lsp_status = client
        .get_lsp_status()
        .context("Failed to get LSP status")?;

    if lsp_status.servers.is_empty() {
        println!("No LSP servers running");
        return Ok(());
    }

    println!("LSP Servers for session '{}':\n", session.session_name);

    for server in &lsp_status.servers {
        let state_colored = if server.has_capabilities {
            format!("\x1b[32m{}\x1b[0m", server.state)
        } else if server.state.contains("Initializing") {
            format!("\x1b[33m{}\x1b[0m", server.state)
        } else {
            server.state.clone()
        };

        println!("Language: {}", server.language);
        println!("  Command: {}", server.command);
        println!("  State: {}", state_colored);
        println!("  Pending requests: {}", server.pending_requests);
        println!("  Has capabilities: {}", server.has_capabilities);
        println!();
    }

    if let Some(progress) = &lsp_status.progress {
        println!("Progress: {}", progress);
    }

    Ok(())
}

/// Get context window from a session
fn cmd_context(session_name: Option<String>) -> Result<()> {
    let session = resolve_session(session_name)?;

    let client = OvimClient::new(&session);

    // Use send_mcp_request directly to get the raw response
    let response = client
        .send_mcp_request(
            "tools/call",
            serde_json::json!({
                "name": "get_context_window",
                "arguments": {}
            }),
            1,
        )
        .context("Failed to get context window")?;

    // Extract the context string from the MCP response
    // The response structure is: result.content[0].text contains JSON-serialized ContextWindowInfo
    if let Some(text_field) = response.get("result").and_then(|r| r.get("content")).and_then(|c| c.get(0)).and_then(|c| c.get("text")) {
        if let Some(text_str) = text_field.as_str() {
            // Parse the JSON string to extract ContextWindowInfo
            if let Ok(context_info) = serde_json::from_str::<serde_json::Value>(text_str) {
                if let Some(context_text) = context_info.get("context").and_then(|c| c.as_str()) {
                    // Print the context, which has \n escape sequences that will be rendered as newlines
                    println!("{}", context_text);
                    return Ok(());
                }
            }
        }
    }

    anyhow::bail!("Failed to extract context from MCP response")
}

/// Install ovim as MCP server for supported editors
fn cmd_install(editor: &str, show_config: bool, workspace: Option<String>) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    // Determine ovim binary path (canonicalized to avoid symlink/double-slash issues)
    let ovim_path = std::env::current_exe().context("Failed to get current executable path")?;
    let ovim_bin = ovim_path
        .canonicalize()
        .unwrap_or(ovim_path)
        .to_string_lossy()
        .to_string();

    // Determine workspace directory
    let workspace_dir = if let Some(w) = workspace {
        PathBuf::from(w)
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".ovim-workspace")
    };

    // Create workspace if it doesn't exist
    if !show_config {
        fs::create_dir_all(&workspace_dir).context("Failed to create workspace directory")?;
    }

    // Generate MCP configuration (stdio-based server)
    let mcp_config = serde_json::json!({
        "type": "stdio",
        "command": ovim_bin,
        "args": ["mcp-server", "--workspace", workspace_dir.to_string_lossy().to_string()]
    });

    match editor.to_lowercase().as_str() {
        "claude-code" | "code" => install_claude_code(&mcp_config, show_config)?,
        "claude-desktop" | "desktop" => install_claude_desktop(&mcp_config, show_config)?,
        "claude" => {
            // Default: install for both Claude Code and Claude Desktop
            install_claude_code(&mcp_config, show_config)?;
            install_claude_desktop(&mcp_config, show_config)?;
        }
        "cursor" => install_cursor(&mcp_config, show_config)?,
        "all" => {
            install_claude_code(&mcp_config, show_config)?;
            install_claude_desktop(&mcp_config, show_config)?;
            install_cursor(&mcp_config, show_config)?;
        }
        _ => anyhow::bail!(
            "Unknown editor: {}. Supported: claude (both), claude-code, claude-desktop, cursor, all",
            editor
        ),
    }

    if !show_config {
        println!("\n\x1b[32m✓ Installation complete!\x1b[0m");
        println!("\nNext steps:");
        println!("1. Restart the editor to load the new MCP server");
        println!("2. The ovim MCP server will auto-spawn sessions as needed");
        println!("3. Any queries involving your code will automatically use ovim's LSP features");
    }

    Ok(())
}

/// Install for Claude Code (.mcp.json and .claude/settings.json with hooks)
fn install_claude_code(mcp_config: &serde_json::Value, show_config: bool) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let config_path = PathBuf::from(".mcp.json");
    let claude_dir = PathBuf::from(".claude");
    let claude_settings_path = claude_dir.join("settings.json");
    let hook_script_path = claude_dir.join("hooks/inject_context.sh");

    // Get absolute path for display
    let abs_path = std::fs::canonicalize(".")
        .map(|p| p.join(".mcp.json"))
        .unwrap_or_else(|_| config_path.clone());

    // Read existing config or create new one
    let mut config: serde_json::Value = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .context("Failed to read existing .mcp.json")?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Ensure mcpServers object exists
    if config.get("mcpServers").is_none() {
        config["mcpServers"] = serde_json::json!({});
    }

    // Add or update ovim entry (don't override other servers)
    config["mcpServers"]["ovim"] = mcp_config.clone();

    if show_config {
        println!("\n📋 Claude Code config to be added/merged to .mcp.json:");
        println!("{}", serde_json::to_string_pretty(&config["mcpServers"]["ovim"])?);
        println!("\nWill be saved to: {}", abs_path.display());
        println!("\n📋 Claude Code hook config (UserPromptSubmit event):");
        let parent = abs_path.parent().unwrap_or(&abs_path);
        println!("Hooks directory: {}", parent.join(".claude/hooks").display());
        println!("Settings file: {}", parent.join(".claude/settings.json").display());
        println!("Hook script: {}", parent.join(".claude/hooks/inject_context.sh").display());
    } else {
        // Create .claude directory and hooks subdirectory
        fs::create_dir_all(claude_dir.join("hooks"))
            .context("Failed to create .claude/hooks directory")?;

        // Create hook script that auto-injects context
        // This hook receives JSON via stdin (UserPromptSubmit event)
        // and outputs context that Claude Code will inject into the message
        let hook_script = "#!/bin/bash\n\
# Auto-inject ovim context into Claude Code messages\n\
# Runs on UserPromptSubmit event - outputs context to be injected\n\
\n\
# Try to find ovim binary (prefer installed, fallback to local build)\n\
if command -v ovim &>/dev/null; then\n\
  context_output=$(ovim context 2>/dev/null)\n\
elif [ -x ./target/release/ovim ]; then\n\
  context_output=$(./target/release/ovim context 2>/dev/null)\n\
elif [ -x ./target/debug/ovim ]; then\n\
  context_output=$(./target/debug/ovim context 2>/dev/null)\n\
fi\n\
\n\
# Exit code 0 and stdout will be injected as context\n\
if [ -n \"$context_output\" ]; then\n\
  echo \"$context_output\"\n\
fi\n\
";
        fs::write(&hook_script_path, hook_script)
            .context("Failed to write hook script")?;

        // Make hook executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook_script_path, fs::Permissions::from_mode(0o755))
                .context("Failed to make hook executable")?;
        }

        // Create or update .claude/settings.json with hook configuration
        let mut claude_settings: serde_json::Value = if claude_settings_path.exists() {
            let content = fs::read_to_string(&claude_settings_path)
                .context("Failed to read existing .claude/settings.json")?;
            serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        // Ensure hooks object exists
        if claude_settings.get("hooks").is_none() {
            claude_settings["hooks"] = serde_json::json!({});
        }

        // Add UserPromptSubmit hook to auto-inject context (append, don't replace)
        // This hook runs when the user submits a prompt, and its output is injected as context
        let mut hooks = claude_settings["hooks"]["UserPromptSubmit"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        // Only add if not already present
        let new_hook = serde_json::json!({
            "type": "command",
            "command": ".claude/hooks/inject_context.sh"
        });

        let hook_exists = hooks.iter().any(|h| {
            h.get("command").and_then(|c| c.as_str()) == Some(".claude/hooks/inject_context.sh")
        });

        if !hook_exists {
            hooks.push(new_hook);
            claude_settings["hooks"]["UserPromptSubmit"] = serde_json::Value::Array(hooks);
            println!("✓ Added ovim context hook to .claude/settings.json");
        } else {
            println!("✓ Hook already exists in .claude/settings.json (skipped)");
        }

        fs::write(&claude_settings_path, serde_json::to_string_pretty(&claude_settings)?)
            .context("Failed to write .claude/settings.json")?;

        // Write MCP config
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&config)?,
        ).context("Failed to write .mcp.json")?;

        println!("✓ Updated .mcp.json for Claude Code");
        println!("  Location: {}", abs_path.display());
        println!("  Hook script: {}", hook_script_path.display());
        println!("  Context will auto-inject on every message you send!");
        println!("\n⚠️  Important: Files are created in the current directory.");
        println!("  Make sure you run this command from your project root.");
        println!("  If Claude Code isn't finding ovim after restart:");
        println!("  1. Run: cd /path/to/your/project");
        println!("  2. Run: ovim install claude-code");
        println!("  3. Restart Claude Code");
    }

    Ok(())
}

/// Install for Claude Desktop
fn install_claude_desktop(mcp_config: &serde_json::Value, show_config: bool) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let config_path = PathBuf::from(&home).join(".config/Claude/claude_desktop_config.json");

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).context("Failed to create .config/Claude directory")?;
    }

    // Read existing config or create new one
    let mut config: serde_json::Value = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .context("Failed to read existing Claude Desktop config")?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Ensure mcpServers object exists
    if config.get("mcpServers").is_none() {
        config["mcpServers"] = serde_json::json!({});
    }

    // Add or update ovim entry (don't override other servers)
    config["mcpServers"]["ovim"] = mcp_config.clone();

    if show_config {
        println!("\n📋 Claude Desktop config to be added/merged:");
        println!("{}", serde_json::to_string_pretty(&config["mcpServers"]["ovim"])?);
        println!(
            "\nWill be saved to: {}",
            config_path.to_string_lossy()
        );
    } else {
        // Write updated config
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&config)?,
        ).context("Failed to write Claude Desktop config")?;
        println!("✓ Updated Claude Desktop config");
    }

    Ok(())
}

/// Install for Cursor IDE
fn install_cursor(mcp_config: &serde_json::Value, show_config: bool) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let config_path = PathBuf::from(&home).join(".cursor/rules/mcp_config.json");

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).context("Failed to create .cursor/rules directory")?;
    }

    // Read existing config or create new one
    let mut config: serde_json::Value = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .context("Failed to read existing Cursor MCP config")?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Ensure mcpServers object exists
    if config.get("mcpServers").is_none() {
        config["mcpServers"] = serde_json::json!({});
    }

    // Add or update ovim entry
    config["mcpServers"]["ovim"] = mcp_config.clone();

    if show_config {
        println!("\n📋 Cursor IDE config to be added/merged:");
        println!("{}", serde_json::to_string_pretty(&config["mcpServers"]["ovim"])?);
        println!(
            "\nWill be saved to: {}",
            config_path.to_string_lossy()
        );
    } else {
        // Write updated config
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&config)?,
        ).context("Failed to write Cursor MCP config")?;
        println!("✓ Updated Cursor IDE config");
    }

    Ok(())
}

/// Start ovim as a long-running MCP server
fn cmd_mcp_server(
    workspace: Option<String>,
    _port: Option<u16>,
    _session: Option<String>,
) -> Result<()> {
    use std::path::PathBuf;

    // Determine workspace directory
    let workspace_dir = if let Some(w) = workspace {
        PathBuf::from(w)
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".ovim-workspace")
    };

    // Start the MCP stdio server
    crate::mcp_stdio_server::run_mcp_server(workspace_dir)
}

/// Trigger goto-definition and return new location as JSON
fn cmd_goto_definition(session_name: Option<String>) -> Result<()> {
    use std::thread;
    use std::time::Duration;
    use serde_json::json;

    let session = resolve_session(session_name)?;
    let client = OvimClient::new(&session);

    let before = client.get_snapshot().context("Failed to get snapshot before goto-definition")?;
    client.send_keys("gd").context("Failed to send goto-definition keys")?;
    thread::sleep(Duration::from_millis(300));
    let after = client.get_snapshot().context("Failed to get snapshot after goto-definition")?;

    let moved = (before.cursor.line, before.cursor.column) != (after.cursor.line, after.cursor.column)
        || before.buffer.file_path != after.buffer.file_path;

    println!("{}", serde_json::to_string_pretty(&json!({
        "success": moved,
        "file": after.buffer.file_path,
        "line": after.cursor.line + 1,
        "column": after.cursor.column + 1
    }))?);

    Ok(())
}

/// Trigger find-references and return list from picker
fn cmd_find_references(session_name: Option<String>) -> Result<()> {
    use std::thread;
    use std::time::Duration;
    use serde_json::json;

    let session = resolve_session(session_name)?;
    let client = OvimClient::new(&session);

    client.send_keys("gr").context("Failed to send find-references keys")?;
    thread::sleep(Duration::from_millis(500));
    let snapshot = client.get_snapshot().context("Failed to get snapshot after find-references")?;

    let references = if let Some(picker) = &snapshot.picker {
        picker.results.iter().map(|r| {
            json!({
                "display": r.display,
                "file": r.location,
                "line": r.line,
                "column": r.col
            })
        }).collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    println!("{}", serde_json::to_string_pretty(&json!({
        "success": !references.is_empty(),
        "references": references
    }))?);

    Ok(())
}

/// Trigger hover and return hover_info
fn cmd_hover(session_name: Option<String>) -> Result<()> {
    use std::thread;
    use std::time::Duration;
    use serde_json::json;

    let session = resolve_session(session_name)?;
    let client = OvimClient::new(&session);

    client.send_keys("K").context("Failed to send hover keys")?;
    thread::sleep(Duration::from_millis(300));
    let snapshot = client.get_snapshot().context("Failed to get snapshot after hover")?;

    println!("{}", serde_json::to_string_pretty(&json!({
        "success": snapshot.hover_info.is_some(),
        "hover": snapshot.hover_info
    }))?);

    Ok(())
}

/// Search for pattern and jump to first match
fn cmd_search(pattern: &str, session_name: Option<String>) -> Result<()> {
    use std::thread;
    use std::time::Duration;
    use serde_json::json;

    let session = resolve_session(session_name)?;
    let client = OvimClient::new(&session);

    let before = client.get_snapshot().context("Failed to get snapshot before search")?;

    // Enter search mode with / and the pattern, then press Enter
    let search_cmd = format!("/{}<CR>", pattern);
    client.send_keys(&search_cmd).context("Failed to send search keys")?;
    thread::sleep(Duration::from_millis(200));
    let after = client.get_snapshot().context("Failed to get snapshot after search")?;

    let found = (before.cursor.line, before.cursor.column) != (after.cursor.line, after.cursor.column);

    println!("{}", serde_json::to_string_pretty(&json!({
        "success": found,
        "file": after.buffer.file_path,
        "line": after.cursor.line + 1,
        "column": after.cursor.column + 1
    }))?);

    Ok(())
}

/// Jump to next match and return position
fn cmd_next_match(session_name: Option<String>) -> Result<()> {
    use std::thread;
    use std::time::Duration;
    use serde_json::json;

    let session = resolve_session(session_name)?;
    let client = OvimClient::new(&session);

    let before = client.get_snapshot().context("Failed to get snapshot before next match")?;
    client.send_keys("n").context("Failed to send next match keys")?;
    thread::sleep(Duration::from_millis(100));
    let after = client.get_snapshot().context("Failed to get snapshot after next match")?;

    let moved = (before.cursor.line, before.cursor.column) != (after.cursor.line, after.cursor.column);

    println!("{}", serde_json::to_string_pretty(&json!({
        "success": moved,
        "file": after.buffer.file_path,
        "line": after.cursor.line + 1,
        "column": after.cursor.column + 1
    }))?);

    Ok(())
}

/// Return LSP diagnostic info
fn cmd_diagnostics(session_name: Option<String>) -> Result<()> {
    use serde_json::json;

    let session = resolve_session(session_name)?;
    let client = OvimClient::new(&session);

    // Use MCP to call get_diagnostics tool
    let response = client.send_mcp_request(
        "tools/call",
        json!({
            "name": "get_diagnostics",
            "arguments": {}
        }),
        1,
    ).context("Failed to get diagnostics via MCP")?;

    // Extract diagnostics from MCP response
    if let Some(text_field) = response.get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
    {
        if let Some(text_str) = text_field.as_str() {
            if let Ok(diagnostics_info) = serde_json::from_str::<Value>(text_str) {
                println!("{}", serde_json::to_string_pretty(&diagnostics_info)?);
                return Ok(());
            }
        }
    }

    println!("{}", serde_json::to_string_pretty(&json!({
        "success": false,
        "diagnostics": []
    }))?);

    Ok(())
}

/// List document symbols
fn cmd_symbols(session_name: Option<String>) -> Result<()> {
    use serde_json::json;

    let session = resolve_session(session_name)?;
    let client = OvimClient::new(&session);

    // Use MCP to call get_symbols tool
    let response = client.send_mcp_request(
        "tools/call",
        json!({
            "name": "get_symbols",
            "arguments": {}
        }),
        1,
    ).context("Failed to get symbols via MCP")?;

    // Extract symbols from MCP response
    if let Some(text_field) = response.get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
    {
        if let Some(text_str) = text_field.as_str() {
            if let Ok(symbols_info) = serde_json::from_str::<Value>(text_str) {
                println!("{}", serde_json::to_string_pretty(&symbols_info)?);
                return Ok(());
            }
        }
    }

    println!("{}", serde_json::to_string_pretty(&json!({
        "success": false,
        "symbols": []
    }))?);

    Ok(())
}

/// Wait for LSP to be ready (blocks until ready or timeout)
fn cmd_wait_lsp(session_name: Option<String>, timeout_ms: u64) -> Result<()> {
    use std::thread;
    use std::time::{Duration, Instant};
    use serde_json::json;

    let session = resolve_session(session_name)?;
    let client = OvimClient::new(&session);

    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);

    loop {
        if let Ok(lsp_status) = client.get_lsp_status() {
            let all_ready = !lsp_status.servers.is_empty()
                && lsp_status.servers.iter().all(|s| s.has_capabilities);

            if all_ready {
                println!("{}", serde_json::to_string_pretty(&json!({
                    "success": true,
                    "elapsed_ms": start.elapsed().as_millis()
                }))?);
                return Ok(());
            }
        }

        if start.elapsed() >= timeout {
            println!("{}", serde_json::to_string_pretty(&json!({
                "success": false,
                "error": "Timeout waiting for LSP to be ready",
                "elapsed_ms": start.elapsed().as_millis()
            }))?);
            anyhow::bail!("Timeout waiting for LSP");
        }

        thread::sleep(Duration::from_millis(100));
    }
}

/// Clean up stale, expired, and corrupted session files
fn cmd_cleanup(max_age_days: Option<u64>, dry_run: bool) -> Result<()> {
    use crate::session::cleanup_stale_sessions;
    use std::time::Duration;

    // Convert days to Duration
    let max_age = max_age_days.map(|days| Duration::from_secs(days * 24 * 60 * 60));

    println!("Cleaning up session files...\n");

    if dry_run {
        println!("[DRY RUN MODE - no files will be removed]\n");
    }

    if let Some(days) = max_age_days {
        println!("Maximum session age: {} days\n", days);
    }

    let result = cleanup_stale_sessions(max_age, dry_run)
        .context("Failed to clean up sessions")?;

    // Report results
    if result.total_removed() == 0 {
        println!("No stale sessions found. Everything is clean!");
        return Ok(());
    }

    println!("Cleanup Summary:");
    println!("─────────────────────────────────────────────");

    if result.stale_removed > 0 {
        println!("  Stale sessions (dead processes):  {}", result.stale_removed);
    }

    if result.expired_removed > 0 {
        println!("  Expired sessions (too old):       {}", result.expired_removed);
    }

    if result.corrupted_removed > 0 {
        println!("  Corrupted session files:          {}", result.corrupted_removed);
    }

    if result.temp_files_removed > 0 {
        println!("  Orphaned temp files:              {}", result.temp_files_removed);
    }

    println!("─────────────────────────────────────────────");
    println!("  Total removed:                    {}", result.total_removed());

    if !result.removed_sessions.is_empty() {
        println!("\nRemoved sessions:");
        for session in &result.removed_sessions {
            println!("  - {}", session);
        }
    }

    if dry_run {
        println!("\n[DRY RUN] Run without --dry-run to actually remove these files.");
    } else {
        println!("\nCleanup complete!");
    }

    Ok(())
}

/// List all configured languages and their LSP status
///
/// Educational Note: CLI Introspection Patterns
/// This command makes the system inspectable by exposing its configuration.
/// Good CLI tools should answer: "What languages do you support?" without
/// requiring users to dig through source code or documentation.
///
/// Design principles:
/// - Default output is concise (language name + LSP status)
/// - Verbose mode shows configuration details
/// - Exit code reflects success (always 0 unless registry fails)
fn cmd_list_languages(verbose: bool) -> Result<()> {
    use crate::language_config::LanguageRegistry;

    let registry = LanguageRegistry::get();
    let languages = registry.all();

    if languages.is_empty() {
        println!("No languages configured.");
        return Ok(());
    }

    if verbose {
        println!("Configured Languages:\n");
        for lang in languages {
            println!("Language: {} ({})", lang.name, lang.id);
            println!("  Extensions: {}", lang.extensions.join(", "));

            if !lang.filenames.is_empty() {
                println!("  Filenames: {}", lang.filenames.join(", "));
            }

            if let Some(ref syntax) = lang.syntax {
                println!("  Syntax: {}", syntax.grammar);
            } else {
                println!("  Syntax: None");
            }

            if let Some(ref lsp) = lang.lsp {
                println!("  LSP Command: {}", lsp.command);
                if !lsp.args.is_empty() {
                    println!("  LSP Args: {}", lsp.args.join(" "));
                }
                if !lsp.fallback_commands.is_empty() {
                    println!("  Fallbacks: {}", lsp.fallback_commands.join(", "));
                }
                if !lsp.root_markers.is_empty() {
                    println!("  Root Markers: {}", lsp.root_markers.join(", "));
                }
                if let Some(ref hint) = lsp.install_hint {
                    println!("  Install Hint: {}", hint);
                }
                if lsp.auto_install.is_some() {
                    println!("  Auto-Install: Enabled");
                }
            } else {
                println!("  LSP: None");
            }

            println!();
        }
    } else {
        // Concise output: ID, name, LSP status
        println!("{:<15} {:<20} {:<10}", "ID", "Name", "LSP");
        println!("{}", "-".repeat(50));

        for lang in languages {
            let lsp_status = if lang.lsp.is_some() {
                "Configured"
            } else {
                "-"
            };

            println!("{:<15} {:<20} {:<10}", lang.id, lang.name, lsp_status);
        }

        println!("\nUse --verbose for detailed configuration");
    }

    Ok(())
}

/// Check language configuration and LSP status for a file
///
/// Educational Note: Debugging Language Detection
/// This command helps users understand:
/// 1. Which language was detected for a file
/// 2. Which LSP server would be used
/// 3. Whether the LSP server is actually installed
/// 4. What the project root would be
///
/// This is invaluable for debugging "why doesn't LSP work for this file?"
fn cmd_check_lsp(file_path: &str, verbose: bool) -> Result<()> {
    use crate::language_config::{find_lsp_command, find_project_root, LanguageRegistry};
    use std::path::Path;

    let path = Path::new(file_path);
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    println!("File: {}", abs_path.display());
    println!();

    // Detect language
    let registry = LanguageRegistry::get();
    let lang_config = match registry.detect(&abs_path) {
        Some(config) => config,
        None => {
            println!("❌ No language configuration found for this file");
            println!("\nSupported extensions: ");
            for lang in registry.all() {
                if !lang.extensions.is_empty() {
                    println!("  {} → {}", lang.extensions.join(", "), lang.name);
                }
            }
            return Ok(());
        }
    };

    println!("✓ Language Detected: {} ({})", lang_config.name, lang_config.id);
    println!("  Extensions: {}", lang_config.extensions.join(", "));

    // Check syntax highlighting
    if let Some(ref syntax) = lang_config.syntax {
        println!("\n✓ Syntax Highlighting: {} grammar", syntax.grammar);
    } else {
        println!("\n❌ Syntax Highlighting: Not configured");
    }

    // Check LSP configuration
    if let Some(ref lsp) = lang_config.lsp {
        println!("\n✓ LSP Configuration:");
        println!("  Primary Command: {}", lsp.command);

        if !lsp.args.is_empty() {
            println!("  Args: {}", lsp.args.join(" "));
        }

        if !lsp.fallback_commands.is_empty() {
            println!("  Fallback Commands: {}", lsp.fallback_commands.join(", "));
        }

        // Check if LSP server is actually available
        match find_lsp_command(lsp) {
            Some(ref found_command) => {
                println!("\n✓ LSP Server Found: {}", found_command);

                // Try to find project root
                let root_markers = &lsp.root_markers;
                if !root_markers.is_empty() {
                    let project_root = find_project_root(&abs_path, root_markers);
                    println!("✓ Project Root: {}", project_root.display());
                    println!("  (detected using markers: {})", root_markers.join(", "));
                }
            }
            None => {
                println!("\n❌ LSP Server Not Found");
                println!("  Searched for: {}", lsp.command);
                if !lsp.fallback_commands.is_empty() {
                    println!("  Also searched: {}", lsp.fallback_commands.join(", "));
                }

                if let Some(ref hint) = lsp.install_hint {
                    println!("\n  Installation:");
                    println!("  {}", hint);
                }

                if lsp.auto_install.is_some() {
                    println!("\n  Auto-install is configured for this language.");
                    println!("  LSP will be installed automatically when you open a {} file in ovim.", lang_config.id);
                }
            }
        }

        // Show full config in verbose mode
        if verbose {
            println!("\n--- Full LSP Configuration ---");
            println!("{:#?}", lsp);
        }
    } else {
        println!("\n❌ LSP: Not configured for {}", lang_config.name);
        println!("\nYou can add LSP support by creating ~/.config/ovim/languages.toml");
        println!("See: user-docs/LANGUAGE_SUPPORT.md");
    }

    Ok(())
}
