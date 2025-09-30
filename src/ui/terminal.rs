use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Stdout};

/// Manages terminal state and initialization
pub struct Terminal {
    _stdout: Stdout,
}

impl Terminal {
    /// Creates a new terminal instance and initializes it
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Self { _stdout: stdout })
    }

    /// Gets the terminal size (width, height)
    pub fn size() -> Result<(u16, u16)> {
        let size = crossterm::terminal::size()?;
        Ok(size)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Restore terminal state on drop
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}
