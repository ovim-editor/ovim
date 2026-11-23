//! Diagnostics outlive edits the server has not republished for, and a
//! reopened document starts counting versions afresh.

mod helpers;

use helpers::lsp_harness::FakeLsp;
use serde_json::json;

fn publish(lsp: &FakeLsp, message: &str) {
    lsp.script(
        0,
        "auto-diagnostics.json",
        json!({
            "versioned": true,
            "diagnostics": [{
                "range": {"start": {"line": 0, "character": 0},
                          "end": {"line": 0, "character": 3}},
                "severity": 1,
                "message": message,
            }],
        }),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diagnostics_stay_after_an_edit_the_server_does_not_republish_for() {
    let mut lsp = FakeLsp::start("one\n").await;
    publish(&lsp, "published with the change");
    lsp.test.keys("Ax<Esc>");
    lsp.pump_until("the diagnostic", |lsp| {
        !lsp.test.editor.diagnostics_for_line(0).is_empty()
    })
    .await;

    // The server now stays silent (it publishes on save only).
    std::fs::remove_file(lsp.root(0).join("auto-diagnostics.json")).unwrap();
    lsp.test.keys("Ay<Esc>Az<Esc>");
    lsp.settle().await;
    assert_eq!(lsp.test.buffer_content(), "onexyz\n");
    assert!(
        !lsp.test.editor.diagnostics_for_line(0).is_empty(),
        "the diagnostics vanished although the server never retracted them"
    );
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reopened_document_accepts_publications_from_version_one() {
    let mut lsp = FakeLsp::start("one\n").await;
    publish(&lsp, "before closing");
    // Several edits: the document is at a high version when it is closed.
    for edit in ["Aa<Esc>", "Ab<Esc>", "Ac<Esc>", "Ad<Esc>", "Ae<Esc>"] {
        lsp.test.keys(edit);
        lsp.settle().await;
    }
    let uri = lsp.uri(0);
    let manager = lsp.manager();
    lsp.pump_until("the diagnostic", |lsp| {
        lsp.test.editor.diagnostics_for_line(0).len() == 1
    })
    .await;

    // Close the document (another buffer keeps the editor open).
    let other = lsp.root(0).join("other.fk");
    std::fs::write(&other, "other\n").unwrap();
    lsp.test.keys(":w<CR>");
    lsp.test.editor.open_file(&other).unwrap();
    lsp.test.keys(":bprevious<CR>:bdelete!<CR>");
    lsp.wait_for_event(0, "textDocument/didClose").await;
    publish(&lsp, "after reopening");
    lsp.test.keys(&format!(":e {}<CR>", lsp.file(0).display()));
    lsp.pump_until("the second didOpen", |lsp| {
        lsp.events(0, "textDocument/didOpen").len() >= 2
    })
    .await;
    lsp.settle().await;

    let messages: Vec<String> = manager
        .get_diagnostics(&uri)
        .await
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert_eq!(messages, vec!["after reopening".to_string()]);
    lsp.stop().await;
}
