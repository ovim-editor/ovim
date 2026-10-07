//! A burst of server messages while the editor loop is not running (`:terminal`
//! and `:!cmd` park it) must neither lose the latest diagnostics nor leave a
//! server's request unanswered.

mod helpers;

use helpers::lsp_harness::FakeLsp;
use serde_json::{json, Value};
use std::time::Duration;

const DOCUMENTS: usize = 5;
const ROUNDS: usize = 800;

fn diagnostic(message: &str) -> Value {
    json!({
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
        "severity": 1,
        "message": message,
    })
}

fn document_uri(index: usize) -> String {
    format!("file:///burst/doc{index}.fk")
}

/// `ROUNDS` publications for each of `DOCUMENTS` documents, round after round,
/// then a request the server expects an answer to.
fn burst() -> Value {
    let mut messages = Vec::new();
    for round in 0..ROUNDS {
        for index in 0..DOCUMENTS {
            messages.push(json!({
                "method": "textDocument/publishDiagnostics",
                "params": {
                    "uri": document_uri(index),
                    "diagnostics": [diagnostic(&format!("round {round}"))],
                },
            }));
        }
    }
    messages.push(json!({
        "id": 555,
        "method": "workspace/configuration",
        "params": {"items": [{"section": "fake"}]},
    }));
    Value::Array(messages)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_burst_overflowing_the_channel_keeps_the_latest_diagnostics_and_answers_requests() {
    let mut lsp = FakeLsp::start("one\n").await;
    lsp.script(0, "push-after-textDocument_hover.json", burst());
    lsp.test.keys("K");
    // One tick sends the hover; then the editor loop stalls.
    lsp.tick().await;

    // The server's request is answered although nothing is draining the
    // notification channel (4000 messages against its 1000 slots).
    let answered = tokio::time::timeout(Duration::from_secs(10), async {
        while lsp.reply_to(0, 555).is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        answered.is_ok(),
        "workspace/configuration was not answered while the editor loop was stalled"
    );

    // The loop resumes: every document ends with its latest publication.
    lsp.settle().await;
    let manager = lsp.manager();
    for index in 0..DOCUMENTS {
        let uri: lsp_types::Uri = document_uri(index).parse().unwrap();
        let diagnostics = manager.get_diagnostics(&uri).await;
        let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            messages,
            vec![format!("round {}", ROUNDS - 1)],
            "doc{index} lost its latest diagnostics"
        );
    }
    lsp.stop().await;
}
