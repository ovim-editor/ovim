mod terminal;
mod renderer;

pub use terminal::Terminal;
pub use renderer::Renderer;

use anyhow::Result;

/// UI manager that handles terminal and rendering
pub struct UI {
    terminal: Terminal,
    renderer: Renderer,
}

impl UI {
    /// Creates a new UI instance
    pub fn new() -> Result<Self> {
        let terminal = Terminal::new()?;
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
