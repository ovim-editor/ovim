//! A companion server (Tailwind next to the CSS server...) joins a document
//! the primary already has open, and a restarted primary re-opens documents
//! a companion still holds.

mod helpers;

use helpers::lsp_harness::{events_in, FakeLsp};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn companion_started_after_the_document_opened_gets_a_did_open_not_a_did_change() {
    let mut lsp = FakeLsp::start("one\n").await;
    let companion = lsp.start_companion(0).await;

    lsp.test.keys("Atwo<Esc>");
    lsp.pump_until("the companion's didOpen", |_| {
        !events_in(&companion, "textDocument/didOpen").is_empty()
    })
    .await;
    lsp.settle().await;

    let opens = events_in(&companion, "textDocument/didOpen");
    assert_eq!(opens.len(), 1, "{opens:?}");
    // The document's language, not the companion's server id.
    assert_eq!(opens[0]["params"]["textDocument"]["languageId"], "fakels");
    assert_eq!(
        std::fs::read_to_string(companion.join("sync-errors.log")).unwrap_or_default(),
        "",
        "the companion saw a protocol violation"
    );
    // It ends up with the current text, and the primary is undisturbed.
    let mirror = std::fs::read_dir(companion.join("docs"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().ends_with("main.fk.txt"))
        .expect("the companion mirrors the document");
    assert_eq!(std::fs::read_to_string(mirror.path()).unwrap(), "onetwo\n");
    assert_eq!(lsp.events(0, "textDocument/didOpen").len(), 1);
    assert_eq!(lsp.violations(0), "");
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flushing_a_change_to_a_server_without_the_document_opens_it_there() {
    let lsp = FakeLsp::start("one\n").await;
    let companion = lsp.start_companion(0).await;
    let manager = lsp.manager();
    let uri = lsp.uri(0);

    manager
        .did_change(uri.clone(), "fakels", "one two\n".into(), None)
        .await
        .unwrap();
    manager
        .flush_pending_changes_broadcast(&uri, "fakels")
        .await
        .unwrap();
    for _ in 0..50 {
        if !events_in(&companion, "textDocument/didOpen").is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let opens = events_in(&companion, "textDocument/didOpen");
    assert_eq!(opens.len(), 1);
    assert_eq!(opens[0]["params"]["textDocument"]["text"], "one two\n");
    assert_eq!(opens[0]["params"]["textDocument"]["languageId"], "fakels");
    assert!(events_in(&companion, "textDocument/didChange").is_empty());
    // The primary, which had the document, got the change normally.
    assert_eq!(lsp.events(0, "textDocument/didChange").len(), 1);
    lsp.stop().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restarted_primary_reopens_documents_while_a_companion_still_holds_them() {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let mut lsp = FakeLsp::start("one\n").await;
    let companion = lsp.start_companion(0).await;
    lsp.pump_until("the companion's didOpen", |_| {
        !events_in(&companion, "textDocument/didOpen").is_empty()
    })
    .await;
    lsp.manager()
        .set_restart_base_backoff(Duration::from_millis(20));

    let pid = lsp.events(0, "initialize")[0]["pid"].as_i64().unwrap();
    kill(Pid::from_raw(pid as i32), Signal::SIGKILL).unwrap();
    lsp.pump_until("the restarted primary's didOpen", |lsp| {
        lsp.events(0, "textDocument/didOpen").len() >= 2
    })
    .await;
    lsp.settle().await;

    let reopened = lsp.events(0, "textDocument/didOpen");
    assert_ne!(reopened[1]["pid"].as_i64().unwrap(), pid);
    assert_eq!(
        events_in(&companion, "textDocument/didOpen").len(),
        1,
        "the companion already holds the document and must not get it twice"
    );
    assert_eq!(lsp.violations(0), "");
    lsp.stop().await;
}
