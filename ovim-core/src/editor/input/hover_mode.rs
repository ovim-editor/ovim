//! Hover mode input handling
//!
//! Two modes for hover windows:
//! - HoverPreview: Quick peek, any key dismisses (except K to navigate)
//! - HoverNavigate: Scrollable view with j/k navigation

use crate::editor::Editor;
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

/// Handles input in HoverPreview mode (quick peek, any key dismisses except K)
///
/// Returns `Some(key_event)` if the key should be re-processed in normal mode,
/// or `None` if the key was fully handled.
pub fn handle_hover_preview_mode(
    editor: &mut Editor,
    key_event: KeyEvent,
) -> Result<Option<KeyEvent>> {
    match key_event.code {
        // K - enter navigate mode (KK to navigate within hover)
        KeyCode::Char('K') => {
            editor.set_mode(Mode::HoverNavigate);
            Ok(None)
        }
        // Any other key dismisses the hover and returns to normal
        _ => {
            editor.clear_hover();
            editor.set_mode(Mode::Normal);
            // Re-process the key in normal mode (so it's not "eaten")
            // This makes ESC just close, but j/k/etc actually perform their normal action
            if key_event.code != KeyCode::Esc && key_event.code != KeyCode::Char('q') {
                Ok(Some(key_event))
            } else {
                Ok(None)
            }
        }
    }
}

/// Handles input in HoverNavigate mode (scrollable, shows raw text)
pub fn handle_hover_navigate_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    match key_event.code {
        // Esc or q - close hover window
        KeyCode::Esc | KeyCode::Char('q') => {
            editor.clear_hover();
            editor.set_mode(Mode::Normal);
        }
        // j or Down - scroll down
        KeyCode::Char('j') | KeyCode::Down => {
            editor.scroll_hover_down(1);
        }
        // k or Up - scroll up
        KeyCode::Char('k') | KeyCode::Up => {
            editor.scroll_hover_up(1);
        }
        // Ctrl-D - scroll down half page
        KeyCode::Char('d') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.scroll_hover_down(10);
        }
        // Ctrl-U - scroll up half page
        KeyCode::Char('u') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.scroll_hover_up(10);
        }
        // Ctrl-F or PageDown - scroll down full page
        KeyCode::Char('f') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.scroll_hover_down(20);
        }
        KeyCode::PageDown => {
            editor.scroll_hover_down(20);
        }
        // Ctrl-B or PageUp - scroll up full page
        KeyCode::Char('b') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.scroll_hover_up(20);
        }
        KeyCode::PageUp => {
            editor.scroll_hover_up(20);
        }
        // g - go to top
        KeyCode::Char('g') => {
            editor.scroll_hover_up(usize::MAX); // Scroll to top
        }
        // G - go to bottom
        KeyCode::Char('G') => {
            editor.scroll_hover_down(usize::MAX); // Scroll to bottom
        }
        _ => {
            // Other keys close the hover
            editor.clear_hover();
            editor.set_mode(Mode::Normal);
        }
    }
    Ok(())
}
