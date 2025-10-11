use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "ovim")]
#[command(about = "A Neovim clone written in Rust", long_about = None)]
pub struct Args {
    /// File to open
    pub file: Option<String>,

    /// Run in headless mode with REST API enabled (no TUI)
    #[arg(long)]
    pub headless: bool,

    /// Session name for headless mode (default: "default")
    #[arg(long)]
    pub session: Option<String>,

    /// Set viewport dimensions (e.g., 80x24)
    #[arg(long, value_parser = parse_dimensions)]
    pub dimension: Option<(u16, u16)>,

    /// Render the editor to ANSI and exit (useful for debugging)
    #[arg(long)]
    pub render: bool,
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

impl Args {
    pub fn parse_args() -> Self {
        Args::parse()
    }
}
