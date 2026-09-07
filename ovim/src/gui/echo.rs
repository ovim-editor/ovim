//! The one promise a frontend needs before it can speak for the editor.
//!
//! A remote frontend that wants to echo a keystroke locally has to know what
//! the keystroke will do, and in a modal editor almost nothing about that is
//! visible in a [`GuiSnapshot`](super::GuiSnapshot). A pending `i_CTRL-R`
//! turns the next character into a register name; an `imap` whose first key is
//! printable turns it into a mapping lookup; the LSP-install and sign-in
//! dialogs swallow it entirely. None of those change a single rendered cell,
//! so a client staring at the projection cannot tell them apart from ordinary
//! typing.
//!
//! [`predictable_insert`] is therefore computed here, where the editor is, and
//! travels on the snapshot as one boolean. It is the only thing this chunk
//! adds to the server: the speculation itself, and every rule about when to
//! give up on it, live in the frontend's `predictiveEcho.ts`, next to the
//! rendering whose latency is the reason any of this exists.

use crate::editor::{Editor, MapMode};
use crate::mode::Mode;

/// Whether a plain printable character typed now is certain to be inserted
/// literally at the cursor, and to do nothing else.
///
/// Deliberately a conjunction of cheap, individually obvious clauses rather
/// than anything clever. Each one names a state in which a character means
/// something other than itself; a clause that is wrong costs the user text
/// that was never typed, so "probably" is not good enough for any of them.
pub(super) fn predictable_insert(editor: &Editor) -> bool {
    editor.mode() == Mode::Insert
        // `i_CTRL-R` has been pressed and the next character names a register.
        && !editor.editing.pending_register_insert
        // Half a mapping has been typed; the next character extends or
        // resolves it rather than reaching the buffer.
        && !editor.has_pending_mapping()
        // Modal consent dialogs intercept every key ahead of the mode handler.
        && !editor.has_pending_lsp_install()
        && !editor.has_codex_auth_dialog()
        // A visible completion menu is excluded even though a printable
        // character still inserts itself through it: the menu is a piece of
        // state the client can watch changing, and a prediction made against
        // state that is moving underneath it is the kind this feature must not
        // make.
        && !editor.completion_menu().is_visible()
        // A visual-block insert replays the typed run onto sibling lines when
        // it ends, so the buffer it produces is not the one the keystrokes
        // describe.
        && editor.visual_block_insert_state().is_none()
        && !editor.buffer().is_read_only()
        && !insert_mapping_starts_with_a_printable_key(editor)
}

/// Whether any insert-applicable mapping begins with a character a user would
/// type as text.
///
/// `imap jk <Esc>` is the canonical example: with it defined, `j` is not an
/// insertion at all but the first half of a lookup, and the editor deliberately
/// shows nothing until the second key decides. Mappings whose first key is a
/// control code (`<C-x>`, `<Esc>`, `<Tab>`) cannot collide with a printable
/// character, so they leave prediction alone.
fn insert_mapping_starts_with_a_printable_key(editor: &Editor) -> bool {
    [MapMode::Insert, MapMode::All].iter().any(|mode| {
        editor
            .keymaps()
            .list_mappings(Some(*mode))
            .iter()
            .filter_map(|(_, mapping)| mapping.lhs.chars().next())
            .any(|first| !first.is_control())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ovim_core::{KeyCode, KeyEvent, Modifiers};

    fn typing(content: &str) -> Editor {
        let mut editor = Editor::with_content(content);
        editor.set_mode(Mode::Insert);
        editor
    }

    fn press(editor: &mut Editor, code: KeyCode, modifiers: Modifiers) {
        crate::editor::InputHandler::handle_key_event(editor, KeyEvent::new(code, modifiers))
            .expect("the key should be handled");
    }

    #[test]
    fn plain_insert_mode_is_the_state_a_frontend_may_speak_for() {
        assert!(predictable_insert(&typing("fn main() {}\n")));
    }

    #[test]
    fn every_other_mode_is_left_to_the_editor() {
        let mut editor = Editor::with_content("fn main() {}\n");
        for mode in [
            Mode::Normal,
            Mode::Visual,
            Mode::VisualLine,
            Mode::VisualBlock,
            Mode::Replace,
            Mode::Command,
            Mode::Search,
            Mode::Picker,
            Mode::FileTree,
            Mode::AiChat,
        ] {
            editor.set_mode(mode);
            assert!(
                !predictable_insert(&editor),
                "{mode:?} should not be predictable"
            );
        }
    }

    #[test]
    fn a_half_typed_register_insert_is_not_predictable() {
        let mut editor = typing("fn main() {}\n");
        press(&mut editor, KeyCode::Char('r'), Modifiers::CONTROL);

        assert!(editor.editing.pending_register_insert);
        assert!(!predictable_insert(&editor));
    }

    #[test]
    fn an_insert_mapping_that_starts_with_a_letter_stops_prediction_entirely() {
        let mut editor = typing("fn main() {}\n");
        assert!(predictable_insert(&editor));

        editor.keymaps_mut().add_mapping(
            MapMode::Insert,
            "jk".to_string(),
            "\x1b".to_string(),
            true,
        );

        assert!(!predictable_insert(&editor));
    }

    #[test]
    fn a_mapping_whose_first_key_is_a_control_code_leaves_prediction_alone() {
        let mut editor = typing("fn main() {}\n");
        editor.keymaps_mut().add_mapping(
            MapMode::Insert,
            "\x0c".to_string(),
            "x".to_string(),
            true,
        );

        assert!(predictable_insert(&editor));
    }

    #[test]
    fn a_read_only_buffer_accepts_no_speculation() {
        let mut editor = typing("fn main() {}\n");
        editor.buffer_mut().set_read_only(true);

        assert!(!predictable_insert(&editor));
    }
}
