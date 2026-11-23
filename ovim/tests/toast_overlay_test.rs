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
    assert!(!toasts[0].sticky, "routine LSP failures must auto-dismiss");
    assert_eq!(toasts[0].ttl, Some(std::time::Duration::from_secs(8)));
}

#[test]
fn generic_status_message_does_not_impersonate_an_lsp_notification() {
    let mut editor = Editor::with_content("fn main() {}\n");

    editor.set_status_message("Delete failed: permission denied");

    assert_eq!(editor.status_message(), "Delete failed: permission denied");
    assert!(!editor.has_visible_toasts());
}

#[test]
fn generic_feedback_does_not_overwrite_the_lsp_subsystem_status() {
    let mut editor = Editor::with_content("fn main() {}\n");
    editor.set_lsp_status("LSP: rust-analyzer ready".to_owned());

    editor.set_status_message("Saved file");

    assert_eq!(editor.status_message(), "Saved file");
    assert_eq!(editor.lsp_status(), "LSP: rust-analyzer ready");
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

/// Publish diagnostic counts the way a diagnostics refresh does.
fn publish_counts(editor: &mut Editor, errors: usize, warnings: usize) {
    editor.set_test_diagnostic_counts(errors, warnings);
}

// The badge covers the top-right corner of the code and repeats what the
// status line already says. It is a notice: raised by news, gone by itself.
#[test]
fn the_diagnostic_badge_is_raised_by_news_and_expires() {
    let mut editor = Editor::with_content("fn main() {}\n");
    assert!(!editor.diagnostic_badge_visible());

    publish_counts(&mut editor, 0, 3);
    assert!(editor.diagnostic_badge_visible(), "diagnostics appeared");
    assert!(editor.has_top_right_overlay());

    // Its time is up.
    editor.ui_panels.diagnostic_badge_until = Some(std::time::Instant::now());
    assert!(editor.tick_toasts(), "expiry asks for a redraw");
    assert!(!editor.diagnostic_badge_visible());
    assert!(!editor.tick_toasts());

    // Warnings coming and going while typing are not news.
    publish_counts(&mut editor, 0, 4);
    publish_counts(&mut editor, 0, 2);
    assert!(!editor.diagnostic_badge_visible());

    // A new error is.
    publish_counts(&mut editor, 1, 2);
    assert!(editor.diagnostic_badge_visible());

    // Fixing everything takes it down at once.
    publish_counts(&mut editor, 0, 0);
    assert!(!editor.diagnostic_badge_visible());
    assert!(!editor.has_top_right_overlay());
}

#[test]
fn double_escape_dismisses_the_diagnostic_badge_until_there_is_news() {
    let mut editor = Editor::with_content("fn main() {}\n");
    publish_counts(&mut editor, 2, 0);
    assert!(editor.diagnostic_badge_visible());

    press(&mut editor, KeyCode::Esc);
    press(&mut editor, KeyCode::Esc);
    assert!(!editor.diagnostic_badge_visible());

    publish_counts(&mut editor, 1, 0);
    assert!(
        !editor.diagnostic_badge_visible(),
        "fewer errors is not news"
    );
    publish_counts(&mut editor, 3, 0);
    assert!(editor.diagnostic_badge_visible());
}
