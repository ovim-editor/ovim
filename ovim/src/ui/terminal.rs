use anyhow::Result;
use crossterm::{
    cursor::SetCursorStyle,
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen, SetTitle,
    },
};
use std::io::{self, Stdout};

/// Manages terminal state and initialization
pub struct Terminal {
    _stdout: Stdout,
    override_size: Option<(u16, u16)>,
    keyboard_enhancement_enabled: bool,
}

impl Terminal {
    /// Creates a new terminal instance and initializes it
    pub fn new(override_size: Option<(u16, u16)>) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange,
            EnableMouseCapture
        )?;

        // Enable Kitty keyboard protocol if the terminal supports it.
        // This lets us detect Super/Cmd modifier (e.g. Cmd+1 on macOS in Ghostty).
        let keyboard_enhancement_enabled = if supports_keyboard_enhancement().unwrap_or(false) {
            execute!(
                stdout,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )
            .is_ok()
        } else {
            false
        };

        Ok(Self {
            _stdout: stdout,
            override_size,
            keyboard_enhancement_enabled,
        })
    }

    /// Leave alternate screen and disable raw mode so a child process
    /// can use the terminal normally. Call `resume()` afterward.
    pub fn suspend(&mut self) -> Result<()> {
        let _ = disable_raw_mode();
        if self.keyboard_enhancement_enabled {
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        }
        execute!(
            io::stdout(),
            DisableMouseCapture,
            DisableFocusChange,
            DisableBracketedPaste,
            LeaveAlternateScreen,
        )?;
        Ok(())
    }

    /// Re-enter alternate screen and raw mode after `suspend()`.
    pub fn resume(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange,
            EnableMouseCapture,
        )?;
        if self.keyboard_enhancement_enabled {
            let _ = execute!(
                io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
        Ok(())
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
        if self.keyboard_enhancement_enabled {
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        }
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            DisableFocusChange,
            DisableBracketedPaste,
            LeaveAlternateScreen,
            SetCursorStyle::DefaultUserShape,
            SetTitle("")
        );
    }
}
