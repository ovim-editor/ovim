use ovim::editor::{Editor, InputHandler};
use ovim_core::{KeyCode, KeyEvent, Modifiers};

#[test]
fn escape_closes_the_test_panel() {
    let mut editor = Editor::with_content("fn main() {}\n");
    editor.toggle_test_panel();
    assert!(editor.is_test_panel_open());

    InputHandler::handle_key_event(&mut editor, KeyEvent::new(KeyCode::Esc, Modifiers::NONE))
        .unwrap();

    assert!(!editor.is_test_panel_open());
}

#[test]
fn escape_closes_the_test_panel_while_cancelling_a_pending_command() {
    let mut editor = Editor::with_content("fn main() {}\n");
    editor.toggle_test_panel();
    InputHandler::handle_key_event(
        &mut editor,
        KeyEvent::new(KeyCode::Char('g'), Modifiers::NONE),
    )
    .unwrap();

    InputHandler::handle_key_event(&mut editor, KeyEvent::new(KeyCode::Esc, Modifiers::NONE))
        .unwrap();

    assert!(!editor.is_test_panel_open());
    assert!(editor.input_state().is_normal());
}

#[test]
fn escape_dismisses_a_passive_or_focused_console_and_preserves_history() {
    for focused in [false, true] {
        let mut editor = Editor::with_content("hello");
        editor.run_console_mut().start_run(
            "example".into(),
            ovim_core::launch::LaunchMode::Run,
            "/tmp".into(),
        );
        if focused {
            editor.focus_run_console();
        }
        InputHandler::handle_key_event(&mut editor, KeyEvent::new(KeyCode::Esc, Modifiers::NONE))
            .unwrap();
        assert!(!editor.run_console().open);
        assert_eq!(editor.mode(), ovim_core::mode::Mode::Normal);
        assert_eq!(editor.run_console().runs.len(), 1);
    }
}
