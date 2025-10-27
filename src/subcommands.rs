///! Subcommand implementations for controlling ovim sessions
use anyhow::{Context, Result};
use serde_json::Value;

use crate::cli::Command;
use crate::client::OvimClient;
use crate::session::SessionInfo;

/// Execute a subcommand
pub fn execute_subcommand(command: Command) -> Result<()> {
    match command {
        Command::Sessions => cmd_sessions(),
        Command::Send { session, keys } => cmd_send(&session, &keys),
        Command::Exec { session, command } => cmd_exec(&session, &command),
        Command::Snapshot { session, format } => cmd_snapshot(&session, &format),
        Command::Buffer { session } => cmd_buffer(&session),
        Command::Mcp {
            session,
            method,
            params,
            id,
        } => cmd_mcp(&session, &method, &params, id),
        Command::Kill { session } => cmd_kill(&session),
        Command::Health { session } => cmd_health(&session),
        Command::LspStatus { session } => cmd_lsp_status(&session),
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
fn cmd_send(session_name: &str, keys: &str) -> Result<()> {
    let session = SessionInfo::read(session_name)
        .context(format!("Failed to find session '{}'", session_name))?;

    let client = OvimClient::new(&session);
    client
        .send_keys(keys)
        .context("Failed to send keys to session")?;

    println!("Keys sent to session '{}'", session_name);
    Ok(())
}

/// Execute ex command in a session
fn cmd_exec(session_name: &str, command: &str) -> Result<()> {
    let session = SessionInfo::read(session_name)
        .context(format!("Failed to find session '{}'", session_name))?;

    let client = OvimClient::new(&session);
    let result = client
        .execute_command(command)
        .context("Failed to execute command")?;

    println!("{}", result);
    Ok(())
}

/// Get snapshot from a session
fn cmd_snapshot(session_name: &str, format: &str) -> Result<()> {
    let session = SessionInfo::read(session_name)
        .context(format!("Failed to find session '{}'", session_name))?;

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
fn cmd_buffer(session_name: &str) -> Result<()> {
    let session = SessionInfo::read(session_name)
        .context(format!("Failed to find session '{}'", session_name))?;

    let client = OvimClient::new(&session);
    let buffer = client.get_buffer().context("Failed to get buffer")?;

    print!("{}", buffer.content);
    Ok(())
}

/// Send MCP request to a session
fn cmd_mcp(session_name: &str, method: &str, params_str: &str, id: i64) -> Result<()> {
    let session = SessionInfo::read(session_name)
        .context(format!("Failed to find session '{}'", session_name))?;

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
fn cmd_kill(session_name: &str) -> Result<()> {
    let session = SessionInfo::read(session_name)
        .context(format!("Failed to find session '{}'", session_name))?;

    let client = OvimClient::new(&session);
    client
        .kill_session(&session)
        .context("Failed to kill session")?;

    println!("\x1b[32mSession '{}' (PID: {}) killed\x1b[0m", session_name, session.pid);
    Ok(())
}

/// Check health of a session
fn cmd_health(session_name: &str) -> Result<()> {
    let session = SessionInfo::read(session_name)
        .context(format!("Failed to find session '{}'", session_name))?;

    let client = OvimClient::new(&session);
    let health = client.get_health().context("Failed to get health")?;

    println!("Session: {}", session_name);
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
fn cmd_lsp_status(session_name: &str) -> Result<()> {
    let session = SessionInfo::read(session_name)
        .context(format!("Failed to find session '{}'", session_name))?;

    let client = OvimClient::new(&session);
    let lsp_status = client
        .get_lsp_status()
        .context("Failed to get LSP status")?;

    if lsp_status.servers.is_empty() {
        println!("No LSP servers running");
        return Ok(());
    }

    println!("LSP Servers for session '{}':\n", session_name);

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

/// Install ovim as MCP server for supported editors
fn cmd_install(editor: &str, show_config: bool, workspace: Option<String>) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    // Determine ovim binary path
    let ovim_path = std::env::current_exe().context("Failed to get current executable path")?;
    let ovim_bin = ovim_path.to_string_lossy().to_string();

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

    // Generate MCP configuration
    let mcp_config = serde_json::json!({
        "type": "command",
        "command": ovim_bin,
        "args": ["mcp-server", "--workspace", workspace_dir.to_string_lossy().to_string()]
    });

    match editor.to_lowercase().as_str() {
        "claude" => install_claude_desktop(&mcp_config, show_config)?,
        "cursor" => install_cursor(&mcp_config, show_config)?,
        "all" => {
            install_claude_desktop(&mcp_config, show_config)?;
            install_cursor(&mcp_config, show_config)?;
        }
        _ => anyhow::bail!("Unknown editor: {}. Supported: claude, cursor, all", editor),
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
    port: Option<u16>,
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

    println!("Starting ovim MCP server...");
    println!("Workspace: {}", workspace_dir.display());
    if let Some(p) = port {
        println!("Port: {}", p);
    } else {
        println!("Port: auto (from OS)");
    }

    println!(
        "\n\x1b[33mNote:\x1b[0m This command should be run by editors via the MCP configuration."
    );
    println!("If you're testing, ovim is already running in headless mode on demand.");
    println!("\nTo use with Claude Desktop, run: ovim install claude");

    Ok(())
}
