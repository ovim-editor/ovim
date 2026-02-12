mod helpers;

use helpers::EditorTest;
use ovim::mode::Mode;
use ovim_core::ai::{AiProfileConfig, AiProviderKind, ExtractionStrategy};
use ovim_core::KeyCode;

#[test]
fn test_visual_space_enters_ai_prompt_mode() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("vll<Space>");

    test.assert_mode(Mode::AiPrompt);
    let selection = test
        .editor
        .ai_state
        .active_selection
        .as_ref()
        .expect("expected active AI selection");
    assert_eq!(selection.selected_text, "hel");
    assert_eq!(test.editor.ai_prompt_input(), "");
}

#[test]
fn test_ai_prompt_escape_clears_state() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("vll<Space>");
    test.type_text("rewrite it");
    test.press_esc();

    test.assert_mode(Mode::Normal);
    assert_eq!(test.editor.ai_prompt_input(), "");
    assert!(test.editor.ai_state.active_selection.is_none());
}

#[test]
fn test_ai_prompt_arrow_navigation_edits_prompt() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("vll<Space>");
    test.type_text("abc");
    test.press_key(KeyCode::Left);
    test.press('X');
    test.press_key(KeyCode::Home);
    test.press('!');
    test.press_key(KeyCode::End);
    test.press('?');

    assert_eq!(test.editor.ai_prompt_input(), "!abXc?");
}

#[test]
fn test_ai_lock_blocks_inside_and_allows_outside_edits() {
    let mut test = EditorTest::new("hello world\n");
    test.editor.buffer_mut().add_ai_lock(1, 6, 11); // lock "world"

    test.set_cursor(0, 7);
    test.keys("ix<Esc>");
    assert_eq!(test.buffer_content(), "hello world\n");
    assert_eq!(test.editor.lsp_status(), "AI lock active for selected region");

    test.set_cursor(0, 0);
    test.keys("iX<Esc>");
    assert_eq!(test.buffer_content(), "Xhello world\n");
}

#[tokio::test(flavor = "current_thread")]
async fn test_ai_prompt_submit_creates_lock_and_returns_to_normal() {
    let mut test = EditorTest::new("hello world\n");

    test.editor.ai_state.active_profile = "test".to_string();
    test.editor.ai_state.config.profiles.clear();
    test.editor.ai_state.config.profiles.insert(
        "test".to_string(),
        AiProfileConfig {
            name: "test".to_string(),
            provider: AiProviderKind::OpenAi,
            model: "gpt-4o-mini".to_string(),
            base_url: None,
            api_key_env: Some("OVIM_TEST_AI_KEY_MISSING".to_string()),
            temperature: None,
            max_tokens: None,
            system_prompt: None,
            extraction: ExtractionStrategy::Json,
        },
    );

    test.keys("vll<Space>");
    test.type_text("replace with short word");
    test.press_enter();

    test.assert_mode(Mode::Normal);
    assert_eq!(test.editor.buffer().ai_locks().len(), 1);
    assert_eq!(test.editor.ai_state.pending_jobs.len(), 1);
}
