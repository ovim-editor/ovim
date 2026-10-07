//! Case operations (toggle, upper, lower)
//!
//! Handles case transformations for characters and text ranges.

use crate::editor::Editor;
use crate::repeat_action::{CaseTarget, CaseTransform, RepeatAction};
use anyhow::Result;

/// Toggle case of character at cursor position (~)
/// Returns true if the cursor advanced (more chars available).
pub fn toggle_case_at_cursor(editor: &mut Editor) -> Result<bool> {
    let cursor_before = editor.cursor_position();

    let (advanced, edits) = editor
        .buffer_mut()
        .record(|buf| buf.toggle_char_at_cursor());

    let cursor_after = editor.cursor_position();
    editor.push_recorded_undo(edits, cursor_before, cursor_after);
    editor.set_repeat_action(RepeatAction::ToggleCase { count: 1 });

    Ok(advanced)
}

/// Applies `gu` / `gU` / `g~` over `target` from the cursor and makes it the
/// dot-repeat. The cursor stays where it was.
pub fn change_case(
    editor: &mut Editor,
    transform: CaseTransform,
    target: CaseTarget,
) -> Result<()> {
    let cursor_before = editor.cursor_position();
    let ((), edits) = editor
        .buffer_mut()
        .record(|buf| crate::repeat_action::change_case(buf, transform, target));
    if !edits.is_empty() {
        editor.push_recorded_undo(edits, cursor_before, cursor_before);
    }
    editor.set_repeat_action(RepeatAction::ChangeCase { transform, target });
    Ok(())
}
