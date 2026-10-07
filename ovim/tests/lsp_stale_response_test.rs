//! A slow hover or definition answer must not act on what the user has since
//! moved on to: it used to force hover preview over insert mode and jump away
//! mid-typing.

mod helpers;

use helpers::lsp_harness::FakeLsp;
use ovim::mode::Mode;
use serde_json::json;

fn capabilities() -> serde_json::Value {
    json!({
        "textDocumentSync": {"openClose": true, "change": 2},
        "hoverProvider": true,
        "definitionProvider": true,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hover_answer_arriving_in_insert_mode_does_not_change_the_mode() {
    let mut lsp = FakeLsp::start_with("foo bar\n", 1, capabilities()).await;
    lsp.script_text(0, "delay-textDocument_hover.txt", "0.6");
    lsp.script(
        0,
        "response-textDocument_hover.json",
        json!({"result": {"contents": "late hover"}}),
    );

    lsp.test.keys("K");
    lsp.wait_for_event(0, "textDocument/hover").await;
    lsp.test.keys("i");
    lsp.test.assert_mode(Mode::Insert);

    // Wait out the server's delay, with the editor ticking.
    for _ in 0..60 {
        lsp.tick().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    lsp.test.assert_mode(Mode::Insert);
    lsp.test.keys("o<Esc>");
    assert_eq!(lsp.test.buffer_content(), "ofoo bar\n");
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hover_answer_for_the_cursor_it_was_asked_at_still_shows() {
    let mut lsp = FakeLsp::start_with("foo bar\n", 1, capabilities()).await;
    lsp.script(
        0,
        "response-textDocument_hover.json",
        json!({"result": {"contents": "hover text"}}),
    );
    lsp.test.keys("K");
    lsp.pump_until("the hover preview", |lsp| {
        lsp.test.mode() == Mode::HoverPreview
    })
    .await;
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn definition_answer_after_the_user_started_typing_is_dropped() {
    let mut lsp = FakeLsp::start_with("foo bar\n", 1, capabilities()).await;
    let other = lsp.root(0).join("other.fk");
    std::fs::write(&other, "target\n").unwrap();
    let target = ovim::lsp::uri_from_file_path(other.display().to_string()).unwrap();
    lsp.script_text(0, "delay-textDocument_definition.txt", "0.6");
    lsp.script(
        0,
        "response-textDocument_definition.json",
        json!({"result": {"uri": target.as_str(),
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 6}}}}),
    );

    lsp.test.keys("gd");
    lsp.wait_for_event(0, "textDocument/definition").await;
    lsp.test.keys("A");
    for _ in 0..60 {
        lsp.tick().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    lsp.test.keys("typed<Esc>");
    assert_eq!(lsp.test.buffer_content(), "foo bartyped\n");
    assert_eq!(
        lsp.test.editor.buffer().file_path().map(str::to_string),
        Some(lsp.file(0).display().to_string()),
        "the late definition must not switch files"
    );
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn definition_answer_for_an_untouched_editor_still_jumps() {
    let mut lsp = FakeLsp::start_with("foo bar\n", 1, capabilities()).await;
    let other = lsp.root(0).join("other.fk");
    std::fs::write(&other, "target\n").unwrap();
    let target = ovim::lsp::uri_from_file_path(other.display().to_string()).unwrap();
    lsp.script(
        0,
        "response-textDocument_definition.json",
        json!({"result": {"uri": target.as_str(),
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 6}}}}),
    );
    lsp.test.keys("gd");
    lsp.pump_until("the jump", |lsp| {
        lsp.test.editor.buffer().file_path() == Some(other.to_str().unwrap())
    })
    .await;
    lsp.stop().await;
}
