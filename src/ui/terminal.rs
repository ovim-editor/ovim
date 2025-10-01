use anyhow::Result;
use crossterm::{
    cursor::SetCursorStyle,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Stdout};

/// Manages terminal state and initialization
pub struct Terminal {
    _stdout: Stdout,
    override_size: Option<(u16, u16)>,
}

impl Terminal {
    /// Creates a new terminal instance and initializes it
    pub fn new(override_size: Option<(u16, u16)>) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Self {
            _stdout: stdout,
            override_size,
        })
    }

    /// Gets the terminal size (width, height)
    /// If override_size was set, returns that instead of actual terminal size
    pub fn size(&self) -> Result<(u16, u16)> {
        if let Some(size) = self.override_size {
            Ok(size)
        } else {
            let size = crossterm::terminal::size()?;
            Ok(size)
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Restore terminal state on drop
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            SetCursorStyle::DefaultUserShape
        );
    }
}
