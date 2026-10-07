//! The document notifications a server asked for in its `textDocumentSync`
//! capability are the ones it gets, and no others.

mod helpers;

use helpers::lsp_harness::FakeLsp;
use serde_json::json;

fn sync(options: serde_json::Value) -> serde_json::Value {
    json!({ "textDocumentSync": options })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_wants_no_changes_gets_no_changes() {
    let mut lsp = FakeLsp::start_with(
        "one\n",
        1,
        sync(json!({"openClose": true, "change": 0, "save": {}})),
    )
    .await;
    lsp.test.keys("Atwo<Esc>");
    lsp.settle().await;
    lsp.settle().await;
    assert!(lsp.events(0, "textDocument/didChange").is_empty());
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_wants_no_open_close_hears_neither_nor_changes() {
    let mut lsp = FakeLsp::start_unannounced("one\n", sync(json!({"change": 2, "save": {}}))).await;
    lsp.test.keys("Atwo<Esc>");
    lsp.settle().await;
    lsp.settle().await;
    assert!(lsp.events(0, "textDocument/didOpen").is_empty());
    assert!(lsp.events(0, "textDocument/didChange").is_empty());
    assert_eq!(lsp.violations(0), "");
    lsp.stop().await;
}

async fn saved_notification(options: serde_json::Value) -> Vec<serde_json::Value> {
    let mut lsp = FakeLsp::start_with("one\n", 1, sync(options)).await;
    lsp.test.keys("Atwo<Esc>:w<CR>");
    lsp.settle().await;
    lsp.settle().await;
    let saves = lsp.events(0, "textDocument/didSave");
    lsp.stop().await;
    saves
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn didsave_goes_only_to_servers_that_want_it_and_carries_text_only_on_request() {
    // No `save` option: no notification.
    let saves = saved_notification(json!({"openClose": true, "change": 2})).await;
    assert!(saves.is_empty(), "{saves:?}");

    // `save: {}`: a notification without the text.
    let saves = saved_notification(json!({"openClose": true, "change": 2, "save": {}})).await;
    assert_eq!(saves.len(), 1);
    assert!(saves[0]["params"].get("text").is_none(), "{saves:?}");

    // `includeText`: the text comes along.
    let saves =
        saved_notification(json!({"openClose": true, "change": 2, "save": {"includeText": true}}))
            .await;
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0]["params"]["text"], "onetwo\n");
}
