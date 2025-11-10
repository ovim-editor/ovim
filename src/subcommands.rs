///! Subcommand implementations for controlling ovim sessions
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
        "{:<15} {:<8} {:<10} {:<10} {}",
        "SESSION", "PID", "PORT", "LSP", "FILE"
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
    if !config.get("mcpServers").is_some() {
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
    if !config.get("mcpServers").is_some() {
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
    if !config.get("mcpServers").is_some() {
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
