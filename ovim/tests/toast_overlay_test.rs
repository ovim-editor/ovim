use ovim::editor::{Editor, InputHandler, ToastLevel, ToastRequest, ToastSource};
use ovim_core::{KeyCode, KeyEvent, Modifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, Modifiers::NONE)
}

fn press(editor: &mut Editor, code: KeyCode) {
    InputHandler::handle_key_event(editor, key(code)).unwrap();
}

#[test]
fn test_double_escape_dismisses_top_right_toast_overlay() {
    let mut editor = Editor::with_content("fn main() {}\n");
    editor.push_toast(
        ToastRequest::new(
            ToastSource::System,
            ToastLevel::Warning,
            "Something happened",
        )
        .with_sticky(true)
        .with_dedupe_key("system:warn"),
    );

    assert!(editor.has_top_right_overlay());
    assert!(editor.has_visible_toasts());

    press(&mut editor, KeyCode::Esc);
    assert!(editor.has_visible_toasts());

    press(&mut editor, KeyCode::Esc);
    assert!(!editor.has_visible_toasts());
    assert!(!editor.has_top_right_overlay());
}

#[test]
fn test_lsp_status_error_emits_deduped_toast() {
    let mut editor = Editor::with_content("fn main() {}\n");

    editor.set_lsp_status("Completion failed: No server for language: markdown".to_string());
    editor.set_lsp_status("Completion failed: No server for language: markdown".to_string());

    let toasts = editor.visible_toasts_newest_first(10);
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].source, ToastSource::Lsp);
    assert_eq!(toasts[0].level, ToastLevel::Error);
    assert_eq!(toasts[0].repeat, 2);
}

#[test]
fn test_tick_toasts_prunes_expired_entries() {
    let mut editor = Editor::with_content("fn main() {}\n");
    editor.push_toast(
        ToastRequest::new(ToastSource::System, ToastLevel::Info, "Ephemeral")
            .with_ttl(Some(std::time::Duration::ZERO))
            .with_sticky(false),
    );

    assert!(editor.tick_toasts());
    assert!(!editor.tick_toasts());
}
