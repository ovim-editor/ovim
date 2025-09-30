mod buffer;
mod editor;
mod ui;
mod mode;

use anyhow::Result;
use crossterm::event::Event;
use editor::{Editor, InputHandler};
use ui::UI;

fn main() -> Result<()> {
    // Create UI and editor
    let mut ui = UI::new()?;

    // Check for command line arguments
    let args: Vec<String> = std::env::args().collect();
    let mut editor = if args.len() > 1 {
        // Load file from command line argument
        let mut ed = Editor::new();
        if let Err(e) = ed.load_file(&args[1]) {
            // If file doesn't exist, create empty buffer with that filename
            ed = Editor::new();
            ed.buffer_mut().set_file_path(args[1].clone());
        }
        ed
    } else {
        // No file specified, show welcome message
        Editor::with_content(
            "Welcome to ovim!\n\nA Neovim clone written in Rust.\n\nPress 'i' to enter Insert mode.\nPress Ctrl+Q to quit.\n"
        )
    };

    // Main event loop
    while !editor.should_quit() {
        // Render the editor
        ui.renderer_mut().render(&editor)?;

        // Poll for events
        if let Some(event) = InputHandler::poll_event()? {
            if let Event::Key(key_event) = event {
                InputHandler::handle_key_event(&mut editor, key_event)?;
            }
        }
    }

    // Cleanup is handled by UI's Drop implementation
    Ok(())
}
