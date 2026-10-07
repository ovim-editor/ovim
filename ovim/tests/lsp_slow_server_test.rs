//! A language server that answers slowly must not hold up the editor loop:
//! outline/symbol/trace API queries and explorer renames used to await it
//! inline, stalling every tick (and so every keypress) for the whole delay.

mod helpers;

use helpers::lsp_harness::FakeLsp;
use serde_json::json;
use std::time::{Duration, Instant};

fn capabilities() -> serde_json::Value {
    let filters = json!({"filters": [{"pattern": {"glob": "**/*.fk"}}]});
    json!({
        "textDocumentSync": {"openClose": true, "change": 2},
        "documentSymbolProvider": true,
        "workspaceSymbolProvider": true,
        "callHierarchyProvider": true,
        "workspace": {"fileOperations": {"willRename": filters, "didRename": filters}},
    })
}

const SERVER_DELAY: &str = "1.5";

/// The slowest a single tick may take while the server is stalling.
const TICK_BUDGET: Duration = Duration::from_millis(500);

fn symbols() -> serde_json::Value {
    json!({"result": [{
        "name": "main", "kind": 12,
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
        "selectionRange": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
    }]})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outline_requests_leave_the_loop_free_while_the_server_thinks() {
    let mut lsp = FakeLsp::start_with("one\n", 1, capabilities()).await;
    lsp.script(0, "response-textDocument_documentSymbol.json", symbols());
    lsp.script_text(0, "delay-textDocument_documentSymbol.txt", SERVER_DELAY);

    let started = Instant::now();
    let pending = lsp.test.editor.begin_outline().await;
    assert!(
        started.elapsed() < TICK_BUDGET,
        "starting the outline waited {:?} for the server",
        started.elapsed()
    );

    // The loop keeps running: typing and ticks are not held up.
    lsp.test.keys("Ax<Esc>");
    let tick = Instant::now();
    lsp.tick().await;
    assert!(tick.elapsed() < TICK_BUDGET, "{:?}", tick.elapsed());
    assert_eq!(lsp.test.buffer_content(), "onex\n");

    let outline = pending.await.unwrap();
    assert_eq!(outline.source, "lsp");
    assert_eq!(outline.symbol_count, 1);
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn symbol_search_and_trace_leave_the_loop_free_too() {
    let mut lsp = FakeLsp::start_with("one\n", 1, capabilities()).await;
    lsp.script_text(0, "delay-workspace_symbol.txt", SERVER_DELAY);
    lsp.script_text(
        0,
        "delay-textDocument_prepareCallHierarchy.txt",
        SERVER_DELAY,
    );

    let started = Instant::now();
    let search = lsp.test.editor.begin_symbol_search("main").await;
    let trace = lsp.test.editor.begin_trace().await;
    assert!(
        started.elapsed() < TICK_BUDGET,
        "starting the queries waited {:?} for the server",
        started.elapsed()
    );
    assert_eq!(search.await.unwrap().query, "main");
    trace.await.unwrap();
    lsp.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explorer_rename_does_not_stall_ticks_while_will_rename_is_pending() {
    let mut lsp = FakeLsp::start_with("one\n", 1, capabilities()).await;
    lsp.script_text(0, "delay-workspace_willRenameFiles.txt", SERVER_DELAY);
    let root = lsp.root(0).to_path_buf();
    let original = lsp.file(0);
    lsp.test.editor.file_tree_mut().set_root(&root);
    lsp.test
        .editor
        .request_explorer_rename(original.clone(), "renamed.fk".to_string());

    // Ticks keep flowing while the server is asked.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut slowest = Duration::ZERO;
    while !root.join("renamed.fk").exists() {
        assert!(Instant::now() < deadline, "the rename never finished");
        let tick = Instant::now();
        lsp.tick().await;
        slowest = slowest.max(tick.elapsed());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        slowest < TICK_BUDGET,
        "a tick took {slowest:?} while willRenameFiles was pending"
    );
    assert!(!original.exists());
    assert!(lsp
        .test
        .editor
        .buffer()
        .file_path()
        .is_some_and(|path| path.ends_with("renamed.fk")));
    lsp.pump_until("didRenameFiles", |lsp| {
        !lsp.events(0, "workspace/didRenameFiles").is_empty()
    })
    .await;
    lsp.stop().await;
}
