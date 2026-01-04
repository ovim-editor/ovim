use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "ovim")]
#[command(about = "A Neovim clone written in Rust with MCP support", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// File to open (when no subcommand is given)
    #[arg(global = true)]
    pub file: Option<String>,

    /// Run in headless mode with REST API enabled (no TUI)
    #[arg(long, global = true)]
    pub headless: bool,

    /// Session name for headless mode (default: "default")
    #[arg(long, global = true)]
    pub session: Option<String>,

    /// Set viewport dimensions (e.g., 80x24)
    #[arg(long, global = true, value_parser = parse_dimensions)]
    pub dimension: Option<(u16, u16)>,

    /// Render the editor to ANSI and exit (useful for debugging)
    #[arg(long, global = true)]
    pub render: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List all running ovim sessions
    Sessions,

    /// Send key sequence to a session
    Send {
        /// Key sequence (Vim keybindings)
        keys: String,
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Execute an ex command in a session
    Exec {
        /// Ex command (without leading colon)
        command: String,
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Get snapshot of a session's state
    Snapshot {
        /// Output format (json or pretty)
        #[arg(long, default_value = "json")]
        format: String,
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Get buffer content from a session
    Buffer {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Send MCP JSON-RPC request to a session
    Mcp {
        /// MCP method (e.g., initialize, tools/list, tools/call)
        method: String,
        /// JSON parameters (optional, defaults to {})
        #[arg(default_value = "{}")]
        params: String,
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
        /// Request ID (defaults to 1)
        #[arg(long, default_value = "1")]
        id: i64,
    },

    /// Kill a running session
    Kill {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Check health of a session
    Health {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Get LSP status from a session
    LspStatus {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Get 21-line context window around cursor
    Context {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Install ovim as MCP server for supported editors
    Install {
        /// Editor to install for (claude, cursor, or all)
        #[arg(value_name = "EDITOR", default_value = "claude")]
        editor: String,

        /// Show what would be installed without making changes
        #[arg(long)]
        show_config: bool,

        /// Workspace directory for ovim sessions
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Start ovim as a long-running MCP server
    McpServer {
        /// Workspace directory for ovim sessions
        #[arg(long)]
        workspace: Option<String>,

        /// Port to listen on (default: auto)
        #[arg(long)]
        port: Option<u16>,

        /// Session name for this server instance
        #[arg(long)]
        session: Option<String>,
    },

    /// Trigger goto-definition and return new location as JSON
    GotoDefinition {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Trigger find-references and return list from picker
    FindReferences {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Trigger hover and return hover_info
    Hover {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Search for pattern and jump to first match
    Search {
        /// Search pattern
        pattern: String,
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Jump to next match and return position
    NextMatch {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Return LSP diagnostic info
    Diagnostics {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// List document symbols
    Symbols {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Wait for LSP to be ready (blocks until ready)
    WaitLsp {
        /// Session name (auto-discovered if not provided)
        #[arg(short, long)]
        session: Option<String>,
        /// Timeout in milliseconds (default: 30000)
        #[arg(long, default_value = "30000")]
        timeout: u64,
    },

    /// Clean up stale, expired, and corrupted session files
    Cleanup {
        /// Maximum session age in days (sessions older than this will be removed)
        #[arg(long)]
        max_age: Option<u64>,

        /// Show what would be cleaned up without actually removing files
        #[arg(long)]
        dry_run: bool,
    },
}

/// Parse dimension string like "80x24" into (width, height)
fn parse_dimensions(s: &str) -> Result<(u16, u16), String> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid dimension format: '{}'. Expected format: WIDTHxHEIGHT (e.g., 80x24)",
            s
        ));
    }

    let width = parts[0]
        .parse::<u16>()
        .map_err(|_| format!("Invalid width: '{}'", parts[0]))?;
    let height = parts[1]
        .parse::<u16>()
        .map_err(|_| format!("Invalid height: '{}'", parts[1]))?;

    if width == 0 || height == 0 {
        return Err("Width and height must be greater than 0".to_string());
    }

    Ok((width, height))
}

impl Cli {
    pub fn parse_args() -> Self {
        Cli::parse()
    }

    /// Check if running in editor mode (no subcommand)
    pub fn is_editor_mode(&self) -> bool {
        self.command.is_none()
    }

    /// Get editor args (for backward compatibility)
    pub fn editor_args(&self) -> EditorArgs {
        EditorArgs {
            file: self.file.clone(),
            headless: self.headless,
            session: self.session.clone(),
            dimension: self.dimension,
            render: self.render,
        }
    }
}

/// Legacy args structure for editor mode (backward compatibility)
#[derive(Debug, Clone)]
pub struct EditorArgs {
    pub file: Option<String>,
    pub headless: bool,
    pub session: Option<String>,
    pub dimension: Option<(u16, u16)>,
    pub render: bool,
}
