//! Hover mode input handling
//!
//! Two modes for hover windows:
//! - HoverPreview: Styled markdown, scrollable with j/k/h/l, Esc/q dismisses
//! - HoverNavigate: Raw text view, scrollable with j/k/h/l, Esc/q dismisses

use crate::editor::Editor;
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, Modifiers};
use anyhow::Result;

/// Handles input in HoverPreview mode (styled markdown, scrollable)
///
/// Returns `Some(key_event)` if the key should be re-processed in normal mode,
/// or `None` if the key was fully handled.
pub fn handle_hover_preview_mode(
    editor: &mut Editor,
    key_event: KeyEvent,
) -> Result<Option<KeyEvent>> {
    match key_event.code {
        // K - enter navigate mode (raw text view)
        KeyCode::Char('K') => {
            editor.set_mode(Mode::HoverNavigate);
            Ok(None)
        }
        // Esc or q - dismiss hover
        KeyCode::Esc | KeyCode::Char('q') => {
            editor.clear_hover();
            editor.set_mode(Mode::Normal);
            Ok(None)
        }
        // j or Down - scroll down
        KeyCode::Char('j') | KeyCode::Down => {
            editor.scroll_hover_down(1);
            Ok(None)
        }
        // k or Up - scroll up
        KeyCode::Char('k') | KeyCode::Up => {
            editor.scroll_hover_up(1);
            Ok(None)
        }
        // h or Left - scroll left
        KeyCode::Char('h') | KeyCode::Left => {
            editor.scroll_hover_left(4);
            Ok(None)
        }
        // l or Right - scroll right
        KeyCode::Char('l') | KeyCode::Right => {
            editor.scroll_hover_right(4);
            Ok(None)
        }
        // Ctrl-D - scroll down half page
        KeyCode::Char('d') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.scroll_hover_down(10);
            Ok(None)
        }
        // Ctrl-U - scroll up half page
        KeyCode::Char('u') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.scroll_hover_up(10);
            Ok(None)
        }
        // Ctrl-F or PageDown - scroll down full page
        KeyCode::Char('f') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.scroll_hover_down(20);
            Ok(None)
        }
        KeyCode::PageDown => {
            editor.scroll_hover_down(20);
            Ok(None)
        }
        // Ctrl-B or PageUp - scroll up full page
        KeyCode::Char('b') if key_event.modifiers.contains(Modifiers::CONTROL) => {
            editor.scroll_hover_up(20);
            Ok(None)
        }
        KeyCode::PageUp => {
            editor.scroll_hover_up(20);
            Ok(None)
        }
        // g - go to top
        KeyCode::Char('g') => {
            editor.scroll_hover_up(usize::MAX);
            editor.scroll_hover_left(usize::MAX);
            Ok(None)
        }
        // G - go to bottom
        KeyCode::Char('G') => {
            editor.scroll_hover_down(usize::MAX);
            Ok(None)
        }
        // 0 - scroll to left edge
        KeyCode::Char('0') => {
            editor.scroll_hover_left(usize::MAX);
            Ok(None)
        }
        // $ - scroll to right edge
        KeyCode::Char('$') => {
            editor.scroll_hover_right(usize::MAX);
            Ok(None)
        }
        // Any other key dismisses the hover and returns to normal
        _ => {
            editor.clear_hover();
            editor.set_mode(Mode::Normal);
            // Re-process the key in normal mode (so it's not "eaten")
            Ok(Some(key_event))
        }
    }
}

/// Handles input in HoverNavigate mode (raw text, scrollable)
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
        // h or Left - scroll left
        KeyCode::Char('h') | KeyCode::Left => {
            editor.scroll_hover_left(4);
        }
        // l or Right - scroll right
        KeyCode::Char('l') | KeyCode::Right => {
            editor.scroll_hover_right(4);
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
            editor.scroll_hover_up(usize::MAX);
            editor.scroll_hover_left(usize::MAX);
        }
        // G - go to bottom
        KeyCode::Char('G') => {
            editor.scroll_hover_down(usize::MAX);
        }
        // 0 - scroll to left edge
        KeyCode::Char('0') => {
            editor.scroll_hover_left(usize::MAX);
        }
        // $ - scroll to right edge
        KeyCode::Char('$') => {
            editor.scroll_hover_right(usize::MAX);
        }
        _ => {
            // Other keys close the hover
            editor.clear_hover();
            editor.set_mode(Mode::Normal);
        }
    }
    Ok(())
}
