mod helpers;

use helpers::EditorTest;
use ovim::mode::Mode;
use ovim_core::ai::{
    AgentLoopConfig, AiProfileConfig, AiProviderKind, ContextGatheringPolicy, EditFormat,
    RetryPolicy,
};

/// Helper to build a test profile with common defaults.
fn test_profile(name: &str, provider: AiProviderKind, model: &str) -> AiProfileConfig {
    AiProfileConfig {
        name: name.to_string(),
        provider,
        model: model.to_string(),
        base_url: None,
        api_key: None,
        api_key_env: None,
        temperature: None,
        max_tokens: None,
        system_prompt: None,
        edit_format: EditFormat::Json,
        chat_edit_format: None,
        context: ContextGatheringPolicy::default(),
        agent_loop: AgentLoopConfig::default(),
        tools: vec![],
        scope: ovim_core::ai::ProfileScope::default(),
        edit_prompt: None,
        chat_prompt: None,
        chat_edit_prompt: None,
        reasoning_effort: None,
        verbosity: None,
        syntax_check: None,
        retry: RetryPolicy::default(),
    }
}

#[test]
fn test_visual_space_space_attaches_selection_to_chat() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("vll<Space><Space>");

    test.assert_mode(Mode::AiChat);
    let attachment = test
        .editor
        .ai_chat_pending_code_attachment()
        .expect("expected attached code selection");
    assert_eq!(attachment.text, "hel");
    assert_eq!(test.editor.ai_chat_input(), "");
}

#[test]
fn test_visual_line_chat_attachment_keeps_indent_and_trailing_newline() {
    let mut test = EditorTest::new("    one\n    two\nnext\n");

    test.keys("Vj<Space><Space>");

    test.assert_mode(Mode::AiChat);
    let attachment = test
        .editor
        .ai_chat_pending_code_attachment()
        .expect("expected attached code selection");
    assert_eq!(attachment.text, "    one\n    two\n");
}

#[test]
fn test_visual_chat_attachment_preserves_existing_draft() {
    let mut test = EditorTest::new("hello world\n");
    test.keys("<Space><Space>");
    test.type_text("Please explain this");
    test.press_esc();

    test.keys("vll<Space><Space>");

    test.assert_mode(Mode::AiChat);
    assert_eq!(test.editor.ai_chat_input(), "Please explain this");
    assert_eq!(
        test.editor
            .ai_chat_pending_code_attachment()
            .expect("expected attached selection")
            .text,
        "hel"
    );
}

#[test]
fn test_visual_space_ai_hotkey_is_removed() {
    let mut test = EditorTest::new("hello world\n");

    test.keys("vll<Space>ai");

    test.assert_mode(Mode::Visual);
    assert!(test.editor.ai_state.active_selection.is_none());
    assert!(test.editor.ai_chat_state().is_none());
}

#[test]
fn test_normal_space_space_opens_ai_chat_with_chat_context_profile() {
    let mut test = EditorTest::new("hello world\n");
    test.editor.ai_state.config.profiles.clear();

    let mut alpha = test_profile("alpha", AiProviderKind::Ollama, "model-a");
    alpha.base_url = Some("http://127.0.0.1:11434".to_string());
    test.editor
        .ai_state
        .config
        .profiles
        .insert("alpha".to_string(), alpha);

    let mut beta = test_profile("beta", AiProviderKind::Ollama, "model-b");
    beta.base_url = Some("http://127.0.0.1:11434".to_string());
    test.editor
        .ai_state
        .config
        .profiles
        .insert("beta".to_string(), beta);

    test.editor.ai_state.active_profile = "alpha".to_string();
    test.editor
        .ai_state
        .config
        .contexts
        .insert("chat".to_string(), "beta".to_string());

    test.keys("<Space><Space>");

    test.assert_mode(Mode::AiChat);
    assert_eq!(test.editor.ai_state.active_profile, "alpha");
    assert_eq!(test.editor.ai_chat_effective_profile(), "beta");
    assert_eq!(
        test.editor
            .ai_state
            .chat
            .as_ref()
            .and_then(|chat| chat.opts.profile.as_deref()),
        Some("beta")
    );
}

#[test]
fn test_edit_selection_compatibility_entry_uses_requested_chat_profile() {
    let mut test = EditorTest::new("hello world\n");
    let profile = test_profile("compat", AiProviderKind::Ollama, "model-compat");
    test.editor
        .ai_state
        .config
        .profiles
        .insert("compat".to_string(), profile);

    test.keys("vll");
    test.editor
        .start_ai_chat_from_visual_with_profile(Some("compat".to_string()))
        .expect("attach selection through compatibility entry point");

    test.assert_mode(Mode::AiChat);
    assert_eq!(test.editor.ai_chat_effective_profile(), "compat");
    assert_eq!(
        test.editor
            .ai_chat_pending_code_attachment()
            .expect("expected attached selection")
            .text,
        "hel"
    );
}

#[test]
fn test_attached_selection_closes_to_normal_and_can_be_reselected() {
    for (keys, mode, text) in [
        ("vll", Mode::Visual, "hel"),
        ("V", Mode::VisualLine, "hello world\n"),
    ] {
        for resume in [false, true] {
            let mut test = EditorTest::new("hello world\nnext\n");
            if resume {
                test.keys("<Space><Space>").press_esc();
            }
            test.keys(keys);
            let selected_text = test.editor.visual_selection_text();
            test.keys("<Space><Space>").press_esc();
            test.assert_mode(Mode::Normal);
            assert!(test.editor.visual_start().is_none());
            assert_eq!(
                test.editor.ai_chat_pending_code_attachment().unwrap().text,
                text
            );
            test.keys("gv");
            test.assert_mode(mode);
            assert_eq!(test.editor.visual_selection_text(), selected_text);
            test.press_esc().keys("0x");
            assert_eq!(test.buffer_content(), "ello world\nnext\n");
            assert!(test.editor.visual_start().is_none());
        }
    }
}

#[test]
fn test_reopening_visible_chat_preserves_return_mode() {
    for replacement in [false, true] {
        let mut test = EditorTest::new("hello world\n");
        test.keys("<Space><Space>");
        test.editor
            .open_ai_chat(ovim_core::ai::ChatOpts {
                name: if replacement { "another" } else { "chat" }.into(),
                ..Default::default()
            })
            .unwrap();
        test.press_esc();
        test.assert_mode(Mode::Normal);
    }
}

#[test]
fn test_direct_chat_open_consumes_all_visual_modes() {
    for mode in [Mode::Visual, Mode::VisualLine, Mode::VisualBlock] {
        let mut test = EditorTest::new("hello world\nnext\n");
        test.editor.set_mode(mode);
        test.editor.set_visual_start(0, 0);
        test.editor
            .open_ai_chat(ovim_core::ai::ChatOpts::default())
            .unwrap();
        test.press_esc();
        test.assert_mode(Mode::Normal);
        assert!(test.editor.visual_start().is_none());
        test.keys("gv");
        test.assert_mode(mode);
        assert!(test.editor.visual_start().is_some());
    }
}

#[test]
fn test_failed_chat_open_preserves_visual_selection() {
    let mut test = EditorTest::new("hello world\n");
    test.keys("vll");
    assert!(test
        .editor
        .open_ai_chat(ovim_core::ai::ChatOpts {
            profile: Some("nonexistent-test-profile".into()),
            ..Default::default()
        })
        .is_err());
    test.assert_mode(Mode::Visual);
    assert_eq!(test.editor.visual_selection_text().as_deref(), Some("hel"));
}
