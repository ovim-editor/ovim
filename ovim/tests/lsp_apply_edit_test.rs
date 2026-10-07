//! A server's `workspace/applyEdit` request is answered with what really
//! happened to the edit, after the editor decided, not with "queued".

mod helpers;

use helpers::lsp_harness::FakeLsp;
use serde_json::{json, Value};

fn capabilities() -> Value {
    json!({
        "textDocumentSync": {"openClose": true, "change": 2},
        "hoverProvider": true,
    })
}

fn replace(uri: &str, version: Value, new_text: &str) -> Value {
    json!({
        "textDocument": {"uri": uri, "version": version},
        "edits": [{
            "range": {"start": {"line": 0, "character": 0},
                      "end": {"line": 0, "character": 3}},
            "newText": new_text,
        }],
    })
}

/// Has the server send `workspace/applyEdit` (id 777) with `document_changes`
/// right after answering a hover, and returns the editor's reply.
async fn apply_from_server(lsp: &mut FakeLsp, document_changes: Value) -> Value {
    lsp.script(
        0,
        "push-after-textDocument_hover.json",
        json!([{
            "id": 777,
            "method": "workspace/applyEdit",
            "params": {"edit": {"documentChanges": document_changes}},
        }]),
    );
    lsp.test.keys("K");
    lsp.pump_until("the editor's reply", |lsp| lsp.reply_to(0, 777).is_some())
        .await;
    lsp.reply_to(0, 777).unwrap()["result"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_applied_edit_is_reported_as_applied() {
    let mut lsp = FakeLsp::start_with("foo bar\n", 1, capabilities()).await;
    let uri = lsp.uri(0);
    let reply =
        apply_from_server(&mut lsp, json!([replace(uri.as_str(), json!(null), "baz")])).await;
    assert_eq!(reply["applied"], true, "{reply}");
    assert_eq!(lsp.test.buffer_content(), "baz bar\n");
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_edit_is_refused_and_says_why() {
    let mut lsp = FakeLsp::start_with("foo bar\n", 1, capabilities()).await;
    let uri = lsp.uri(0);
    let reply = apply_from_server(&mut lsp, json!([replace(uri.as_str(), json!(99), "baz")])).await;
    assert_eq!(reply["applied"], false, "{reply}");
    assert!(
        reply["failureReason"]
            .as_str()
            .is_some_and(|reason| reason.contains("document changed")),
        "{reply}"
    );
    assert_eq!(lsp.test.buffer_content(), "foo bar\n");
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_stale_document_keeps_the_whole_edit_from_being_applied() {
    let mut lsp = FakeLsp::start_with("foo bar\n", 1, capabilities()).await;
    let uri = lsp.uri(0);
    let reply = apply_from_server(
        &mut lsp,
        json!([
            replace(uri.as_str(), json!(null), "baz"),
            replace(uri.as_str(), json!(99), "qux"),
        ]),
    )
    .await;
    assert_eq!(reply["applied"], false, "{reply}");
    assert_eq!(reply["failedChange"], 1, "{reply}");
    assert_eq!(
        lsp.test.buffer_content(),
        "foo bar\n",
        "the first document must not be edited when the second one is stale"
    );
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_edit_that_cannot_be_applied_is_reported_as_failed() {
    let mut lsp = FakeLsp::start_with("foo bar\n", 1, capabilities()).await;
    let missing = lsp.root(0).join("no-such-dir").join("x.fk");
    // A directory cannot be written as a file: creating the target fails.
    std::fs::create_dir_all(&missing).unwrap();
    let uri = ovim::lsp::uri_from_file_path(missing.display().to_string()).unwrap();
    let reply = apply_from_server(&mut lsp, json!([replace(uri.as_str(), json!(null), "x")])).await;
    assert_eq!(reply["applied"], false, "{reply}");
    assert!(reply["failureReason"].is_string(), "{reply}");
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn several_edits_to_one_document_at_its_current_version_all_apply() {
    let mut lsp = FakeLsp::start_with("foo bar\n", 1, capabilities()).await;
    let uri = lsp.uri(0);
    let version =
        lsp.events(0, "textDocument/didOpen")[0]["params"]["textDocument"]["version"].clone();
    let second = json!({
        "textDocument": {"uri": uri.as_str(), "version": version},
        "edits": [{"range": {"start": {"line": 0, "character": 4},
                             "end": {"line": 0, "character": 7}},
                   "newText": "qux"}],
    });
    let reply = apply_from_server(
        &mut lsp,
        json!([replace(uri.as_str(), version, "baz"), second]),
    )
    .await;
    assert_eq!(reply["applied"], true, "{reply}");
    assert_eq!(lsp.test.buffer_content(), "baz qux\n");
    lsp.stop().await;
}
