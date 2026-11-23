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

/// Write the DEC private modes that the heartbeat reasserts.
///
/// Split out from `Terminal::ensure_interaction_modes` so a test can assert
/// the exact bytes rather than trusting the argument list to stay correct.
fn write_interaction_modes(out: &mut impl io::Write) -> io::Result<()> {
    execute!(out, EnableBracketedPaste, EnableMouseCapture)
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

    /// Reassert terminal modes needed for mouse and drag/drop input.
    ///
    /// These DEC private modes are terminal-global and can be cleared by a
    /// child process, terminal integration, or a partially failed suspend.
    /// Re-enabling them is idempotent, so the event loop uses this as a small
    /// self-healing heartbeat.
    ///
    /// Focus reporting (`CSI ? 1004 h`) is deliberately *not* reasserted here.
    /// Ghostty on the tip channel answers every write of it with a focus
    /// event, so a heartbeat that includes it feeds itself: each write yields
    /// an input event, and each input event yields another write. That loop
    /// pins the CPU and starves the tick branch that delivers syntax, picker
    /// and grep results. Focus reporting is enabled once at setup and again
    /// after `suspend()`, which is all the protocol actually needs.
    pub fn ensure_interaction_modes(&mut self) -> Result<()> {
        write_interaction_modes(&mut io::stdout())?;
        Ok(())
    }

    /// Re-enter alternate screen and raw mode after `suspend()`.
    pub fn resume(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableFocusChange)?;
        self.ensure_interaction_modes()?;
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

#[cfg(test)]
mod tests {
    use super::write_interaction_modes;

    /// Ghostty on the tip channel (1.3.2-main) answers *every* write of
    /// `CSI ? 1004 h` with a focus event, where release Ghostty and other
    /// terminals stay silent. The heartbeat therefore must not contain it:
    /// the reply arrives as terminal input, the input branch writes the
    /// heartbeat again, and the TUI spins at ~96% CPU while the tick branch
    /// that delivers syntax, picker and grep results never runs.
    #[test]
    fn interaction_mode_heartbeat_omits_focus_reporting() {
        let mut out = Vec::new();
        write_interaction_modes(&mut out).expect("write modes");
        let written = String::from_utf8(out).expect("utf-8");

        assert!(
            !written.contains("?1004h"),
            "heartbeat re-enables focus reporting: {written:?}"
        );
        assert!(
            written.contains("?2004h"),
            "heartbeat lost bracketed paste: {written:?}"
        );
        assert!(
            written.contains("?1000h"),
            "heartbeat lost mouse capture: {written:?}"
        );
    }
}
