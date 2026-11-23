//! Which directories the editor watches for a server's
//! `workspace/didChangeWatchedFiles` registration.

mod helpers;

use helpers::lsp_harness::{events_in, FakeLsp};
use serde_json::json;

fn register_watchers() -> serde_json::Value {
    json!([{
        "id": 4001,
        "method": "client/registerCapability",
        "params": {"registrations": [{
            "id": "watch-all",
            "method": "workspace/didChangeWatchedFiles",
            "registerOptions": {"watchers": [{"globPattern": "**/*"}]},
        }]},
    }])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_started_without_a_project_root_gets_no_watch() {
    let mut lsp = FakeLsp::start("one\n").await;
    // The project's server registers watchers after the first hover.
    lsp.script(0, "push-after-textDocument_hover.json", register_watchers());
    lsp.test.keys("K");
    lsp.pump_until("the project's watch", |lsp| {
        lsp.manager().watched_file_roots() == vec![lsp.root(0).to_path_buf()]
    })
    .await;

    // A file with no project marker above it starts a server whose root is
    // just the file's directory; its registration must not be honoured.
    let loose = lsp.loose_dir();
    std::fs::write(
        loose.join("push-after-textDocument_hover.json"),
        register_watchers().to_string(),
    )
    .unwrap();
    lsp.test.editor.new_tab();
    lsp.test.editor.open_file(loose.join("main.fk")).unwrap();
    lsp.pump_until("the loose file's server", |_| {
        !events_in(&loose, "textDocument/didOpen").is_empty()
    })
    .await;
    lsp.test.keys("K");
    lsp.pump_until("the loose server's hover", |_| {
        !events_in(&loose, "textDocument/hover").is_empty()
    })
    .await;
    lsp.settle().await;

    assert_eq!(
        lsp.manager().watched_file_roots(),
        vec![lsp.root(0).to_path_buf()],
        "a fallback root must not be watched"
    );
    lsp.stop().await;
}
