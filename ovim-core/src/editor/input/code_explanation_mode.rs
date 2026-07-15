use crate::editor::Editor;
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, MouseEventKind};

/// Restores the mode that owns an active code walkthrough.
///
/// A walkthrough deliberately renders the source buffer while AI chat remains
/// the active interaction. Keeping this invariant in one place prevents a stale
/// editor mode from routing input around the walkthrough controls.
pub(super) fn restore_owning_mode(editor: &mut Editor) -> bool {
    if !editor.ai_chat_has_pending_code_explanation() {
        return false;
    }
    if editor.mode() != Mode::AiChat {
        editor.set_mode(Mode::AiChat);
    }
    true
}

/// Handles the complete keyboard allowlist for an active walkthrough.
/// Returns false when there is no walkthrough to handle.
pub(super) fn handle_key(editor: &mut Editor, key_event: KeyEvent) -> bool {
    if !restore_owning_mode(editor) {
        return false;
    }

    match key_event.code {
        KeyCode::Left | KeyCode::Char('h') => {
            editor.move_code_explanation(false);
        }
        KeyCode::Right | KeyCode::Char('l') => {
            editor.move_code_explanation(true);
        }
        KeyCode::Enter => {
            editor.advance_or_finish_code_explanation();
        }
        KeyCode::Esc => {
            editor.finish_code_explanation(true);
        }
        _ => {}
    }

    // Step navigation positions the viewport explicitly, and ignored keys must
    // leave that position alone. A finishing key no longer owns the viewport.
    if editor.ai_chat_has_pending_code_explanation() {
        editor.preserve_viewport_after_input();
    }
    true
}

/// Applies the pointer allowlist for an active walkthrough.
///
/// Wheel events are allowed through for reading. Every other pointer event is
/// consumed so the visible source cannot be edited or switched to Visual mode.
pub(super) fn blocks_pointer_event(editor: &mut Editor, kind: &MouseEventKind) -> bool {
    if !restore_owning_mode(editor) {
        return false;
    }
    if matches!(kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown) {
        return false;
    }

    editor.render_cache.mouse_state.is_dragging = false;
    editor.render_cache.mouse_state.drag_origin = None;
    true
}
