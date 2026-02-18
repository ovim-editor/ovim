//! Dashboard mode input handling.
//!
//! Startup screen should teach normal ovim usage, so the first key press
//! immediately switches to Normal mode and is handled exactly like an empty buffer.

use super::normal;
use crate::editor::Editor;
use crate::mode::Mode;
use crate::KeyEvent;
use anyhow::Result;

/// Handles input in Dashboard mode.
///
/// Any key exits dashboard and is re-processed through normal-mode handlers.
pub fn handle_dashboard_mode(editor: &mut Editor, key_event: KeyEvent) -> Result<()> {
    editor.set_mode(Mode::Normal);
    normal::handle_normal_mode(editor, key_event)
}
