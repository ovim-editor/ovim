use crate::editor::Editor;
use crate::mode::Mode;
use crate::{KeyCode, KeyEvent, Modifiers, MouseEventKind};

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

    let composing = editor.ai_code_explanation_view().is_some_and(|view| {
        matches!(
            view.discussion,
            crate::editor::CodeExplanationDiscussionView::Composing { .. }
        )
    });

    if composing {
        match key_event.code {
            KeyCode::Esc => {
                editor.cancel_code_explanation_question();
            }
            KeyCode::Enter if key_event.modifiers.contains(Modifiers::SHIFT) => {
                editor.insert_code_explanation_question_char('\n');
            }
            KeyCode::Char('j') if key_event.modifiers.contains(Modifiers::CONTROL) => {
                editor.insert_code_explanation_question_char('\n');
            }
            KeyCode::Enter => {
                if let Err(error) = editor.submit_code_explanation_question() {
                    editor.set_status_message(error);
                }
            }
            KeyCode::Backspace => {
                editor.backspace_code_explanation_question();
            }
            KeyCode::Left => {
                editor.move_code_explanation_question_cursor(false);
            }
            KeyCode::Right => {
                editor.move_code_explanation_question_cursor(true);
            }
            KeyCode::Char(character)
                if !key_event.modifiers.contains(Modifiers::CONTROL)
                    && !key_event.modifiers.contains(Modifiers::ALT) =>
            {
                editor.insert_code_explanation_question_char(character);
            }
            _ => {}
        }
    } else {
        match key_event.code {
            KeyCode::Left | KeyCode::Char('h') => {
                editor.move_code_explanation(false);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                editor.move_code_explanation(true);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                editor.scroll_code_explanation_answer(false);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                editor.scroll_code_explanation_answer(true);
            }
            KeyCode::Enter => {
                if editor.ai_code_explanation_answering() {
                    editor.set_status_message("Wait for the walkthrough answer before continuing");
                } else {
                    editor.advance_or_finish_code_explanation();
                }
            }
            KeyCode::Esc => {
                editor.finish_code_explanation(true);
            }
            KeyCode::Char(' ') => {
                if editor.ai_code_explanation_answering() {
                    editor.set_status_message(
                        "The current walkthrough question is still being answered",
                    );
                } else {
                    editor.begin_code_explanation_question();
                }
            }
            _ => {}
        }
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
    match kind {
        MouseEventKind::ScrollUp if editor.render_cache.code_explanation_answer_max_scroll > 0 => {
            editor.scroll_code_explanation_answer(false);
            return true;
        }
        MouseEventKind::ScrollDown
            if editor.render_cache.code_explanation_answer_max_scroll > 0 =>
        {
            editor.scroll_code_explanation_answer(true);
            return true;
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => return false,
        _ => {}
    }

    editor.render_cache.mouse_state.is_dragging = false;
    editor.render_cache.mouse_state.drag_origin = None;
    true
}
