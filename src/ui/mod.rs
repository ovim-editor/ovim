mod terminal;
mod renderer;
mod ansi;

pub use terminal::Terminal;
pub use renderer::Renderer;
pub use ansi::buffer_to_ansi;

use anyhow::Result;

/// UI manager that handles terminal and rendering
pub struct UI {
    terminal: Terminal,
    renderer: Renderer,
}

impl UI {
    /// Creates a new UI instance
    pub fn new() -> Result<Self> {
        Self::with_dimensions(None)
    }

    /// Creates a new UI instance with optional custom dimensions
    pub fn with_dimensions(dimensions: Option<(u16, u16)>) -> Result<Self> {
        let terminal = Terminal::new(dimensions)?;
        let renderer = Renderer::new();
        Ok(Self { terminal, renderer })
    }

    /// Gets a reference to the terminal
    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    /// Gets a mutable reference to the terminal
    pub fn terminal_mut(&mut self) -> &mut Terminal {
        &mut self.terminal
    }

    /// Gets a reference to the renderer
    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// Gets a mutable reference to the renderer
    pub fn renderer_mut(&mut self) -> &mut Renderer {
        &mut self.renderer
    }
}

impl Drop for UI {
    fn drop(&mut self) {
        // Terminal cleanup will be handled by Terminal's Drop
    }
}
