mod helpers;

use helpers::EditorTest;
use ovim::frontend::TickState;
use ovim_core::language_catalog::{DynamicLanguageSpec, DynamicLspSpec, RegistrationOwner};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

/// A real stdio peer holds initialize until the test releases it. Tests drive
/// the shared frontend tick, so a blocked startup fails before that release.
struct StartupSession {
    test: EditorTest,
    channels: TickState,
    dir: tempfile::TempDir,
}

impl StartupSession {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        // The editor canonicalizes paths it opens. On macOS the temp dir
        // sits behind the /var -> /private/var symlink, so every path the
        // harness hands the editor must already be canonical or the second
        // file lands in a "different" workspace root and starts a second
        // server.
        let root = dir.path().canonicalize().unwrap();
        let script = root.join("server.py");
        std::fs::write(&script, include_str!("helpers/controlled_lsp.py")).unwrap();
        let mut test = EditorTest::new("original\n");
        test.editor.enable_lsp();
        test.editor
            .language_catalog()
            .register_dynamic(
                DynamicLanguageSpec {
                    id: "controlled".into(),
                    name: "Controlled LSP".into(),
                    extensions: vec!["controlled".into()],
                    parser: None,
                    lsp: Some(DynamicLspSpec {
                        command: vec![
                            "python3".into(),
                            script.display().to_string(),
                            root.display().to_string(),
                        ],
                        language_id: "controlled".into(),
                        root_markers: vec!["project.marker".into()],
                    }),
                },
                RegistrationOwner::UserConfig {
                    source: root.join("init.lua"),
                },
                std::slice::from_ref(&root),
            )
            .unwrap();
        std::fs::write(root.join("project.marker"), "").unwrap();
        test.set_file_path(root.join("first.controlled").display().to_string());
        test.editor.request_lsp_init();
        Self {
            test,
            channels: TickState::new(),
            dir,
        }
    }

    async fn tick(&mut self) {
        let _report = tokio::time::timeout(
            Duration::from_secs(1),
            self.test.editor.tick(&mut self.channels),
        )
        .await
        .expect("LSP startup blocked the input/render tick");
    }

    fn events(&self, method: &str) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| event["method"] == method)
            .collect()
    }

    async fn wait_for_event(&mut self, method: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                self.tick().await;
                if let Some(event) = self.events(method).last() {
                    return event.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("server did not receive {method}"))
    }

    fn initialize_response(&self, payload: Value) {
        // Publish the complete response atomically; the peer may read immediately.
        let pending = self.dir.path().join("response.pending");
        std::fs::write(&pending, payload.to_string()).unwrap();
        std::fs::rename(pending, self.dir.path().join("initialize-response.json")).unwrap();
    }

    fn ready(&self) {
        self.initialize_response(json!({"result": {"capabilities": {"textDocumentSync": 1}}}));
    }

    async fn stop(&self) {
        self.test
            .editor
            .lsp_manager()
            .unwrap()
            .stop_server("controlled")
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typing_during_startup_is_responsive_and_did_open_uses_latest_text() {
    let mut session = StartupSession::new();
    session.wait_for_event("initialize").await;

    session.test.keys("A edited<Esc>");
    session.tick().await;
    assert_eq!(session.test.buffer_content(), "original edited\n");
    assert!(session.events("textDocument/didOpen").is_empty());

    session.ready();
    let opened = session.wait_for_event("textDocument/didOpen").await;
    assert_eq!(
        opened["params"]["textDocument"]["text"],
        "original edited\n"
    );
    assert_eq!(session.events("initialize").len(), 1);
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_open_buffer_is_opened_once_the_shared_server_is_ready() {
    let mut session = StartupSession::new();
    session.wait_for_event("initialize").await;
    let first = session
        .test
        .editor
        .buffer()
        .file_path()
        .unwrap()
        .to_string();
    let second: PathBuf = session
        .dir
        .path()
        .canonicalize()
        .unwrap()
        .join("second.controlled");

    std::fs::write(&second, "second file\n").unwrap();
    session.test.editor.new_tab();
    session.test.editor.open_file(&second).unwrap();
    session.tick().await;
    session.ready();

    // Both the current buffer and the buffer left behind in the first tab
    // must reach the server; only one server is started for the shared root.
    tokio::time::timeout(Duration::from_secs(5), async {
        while session.events("textDocument/didOpen").len() < 2 {
            session.tick().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both open buffers must be didOpen'd");
    let opened = session.events("textDocument/didOpen");
    let text_for = |path: &str| {
        let uri = ovim::lsp::uri_from_file_path(path).unwrap();
        opened
            .iter()
            .find(|event| event["params"]["textDocument"]["uri"] == uri.as_str())
            .unwrap_or_else(|| panic!("no didOpen for {path}"))["params"]["textDocument"]["text"]
            .clone()
    };
    assert_eq!(text_for(&second.display().to_string()), "second file\n");
    assert_eq!(text_for(&first), "original\n");
    assert_eq!(session.events("initialize").len(), 1);
    session.stop().await;
}

/// Two files closed between two ticks (a workspace edit renaming both) must
/// both be reported: the close notification used to live in one slot, so the
/// second close overwrote the first and the server kept a stale document.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_file_closed_in_one_tick_is_reported_to_the_server() {
    let mut session = StartupSession::new();
    session.wait_for_event("initialize").await;
    session.ready();
    let root = session.dir.path().canonicalize().unwrap();
    let files: Vec<PathBuf> = ["old-a.controlled", "old-b.controlled"]
        .iter()
        .map(|name| root.join(name))
        .collect();
    for file in &files {
        std::fs::write(file, "text\n").unwrap();
        session.test.editor.new_tab();
        session.test.editor.open_file(file).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while session.events("textDocument/didOpen").len() < 3 {
            session.tick().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("all three buffers must be didOpen'd");

    let uri = |file: &PathBuf| ovim::lsp::uri_from_file_path(file.display().to_string()).unwrap();
    let rename = |file: &PathBuf| {
        let renamed = file.with_file_name(
            file.file_name()
                .unwrap()
                .to_string_lossy()
                .replace("old", "new"),
        );
        lsp_types::DocumentChangeOperation::Op(lsp_types::ResourceOp::Rename(
            lsp_types::RenameFile {
                old_uri: uri(file),
                new_uri: uri(&renamed),
                options: None,
                annotation_id: None,
            },
        ))
    };
    let edit = lsp_types::WorkspaceEdit {
        changes: None,
        document_changes: Some(lsp_types::DocumentChanges::Operations(
            files.iter().map(rename).collect(),
        )),
        change_annotations: None,
    };
    session.test.editor.apply_workspace_edit(edit).unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        while session.events("textDocument/didClose").len() < 2 {
            session.tick().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "expected a didClose per renamed file, got {:?}",
            session.events("textDocument/didClose")
        )
    });
    let closed: Vec<String> = session
        .events("textDocument/didClose")
        .iter()
        .map(|event| {
            event["params"]["textDocument"]["uri"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    for file in &files {
        assert!(
            closed.contains(&uri(file).as_str().to_string()),
            "{closed:?}"
        );
    }
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_opened_in_a_split_after_startup_is_opened_on_the_server() {
    let mut session = StartupSession::new();
    session.ready();
    session.wait_for_event("textDocument/didOpen").await;

    let second: PathBuf = session
        .dir
        .path()
        .canonicalize()
        .unwrap()
        .join("split.controlled");
    std::fs::write(&second, "in a split\n").unwrap();
    session.test.keys(":vsplit<CR>");
    session.test.keys(&format!(":e {}<CR>", second.display()));
    tokio::time::timeout(Duration::from_secs(5), async {
        while session.events("textDocument/didOpen").len() < 2 {
            session.tick().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("split buffer must be didOpen'd");
    session.test.keys("<C-w>w");
    session.tick().await;
    // Switching focus between windows must not close either document.
    for _ in 0..5 {
        session.tick().await;
    }
    assert!(session.events("textDocument/didClose").is_empty());
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_failure_is_reported_without_disabling_editing() {
    let mut session = StartupSession::new();
    session.wait_for_event("initialize").await;
    session.initialize_response(
        json!({"error": {"code": -32603, "message": "deliberate startup failure"}}),
    );

    tokio::time::timeout(Duration::from_secs(5), async {
        while !session
            .test
            .editor
            .lsp_status()
            .contains("deliberate startup failure")
        {
            session.tick().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("startup failure was not reported");
    session.test.keys("ccstill editable<Esc>");
    session.tick().await;
    assert_eq!(session.test.buffer_content(), "still editable\n");
    assert!(session.events("textDocument/didOpen").is_empty());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_startup_reaps_its_process_and_can_be_retried() {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let mut session = StartupSession::new();
    let initialization = session.wait_for_event("initialize").await;
    let pid = Pid::from_raw(initialization["peerPid"].as_i64().unwrap() as i32);

    // Closing the frontend aborts its pending startup without a server response.
    session.channels = TickState::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while kill(pid, None).is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancelled startup left its language server running");

    session.test.editor.request_lsp_init();
    session.ready();
    let opened = session.wait_for_event("textDocument/didOpen").await;
    assert_eq!(opened["params"]["textDocument"]["text"], "original\n");
    assert_eq!(session.events("initialize").len(), 2);
    session.stop().await;
}

#[cfg(unix)]
fn sigkill(pid: i64) {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), Signal::SIGKILL).unwrap();
}

#[cfg(unix)]
impl StartupSession {
    async fn wait_until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                self.tick().await;
                if done(self) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out waiting for {what}; status: {}",
                self.test.editor.lsp_status()
            )
        });
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killed_server_is_reaped_restarted_and_documents_reopened() {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let mut session = StartupSession::new();
    session.ready();
    let manager = session.test.editor.lsp_manager().unwrap();
    manager.set_restart_base_backoff(Duration::from_millis(20));
    let first_open = session.wait_for_event("textDocument/didOpen").await;
    let pid = session.events("initialize")[0]["peerPid"].as_i64().unwrap();

    sigkill(pid);
    session
        .wait_until("crash to be reported", |s| {
            s.test.editor.lsp_status().contains("crashed")
        })
        .await;
    let status = session.test.editor.lsp_status().to_string();
    assert!(
        status.contains("signal 9"),
        "status should say why: {status}"
    );
    // A zombie still answers signal 0; a reaped process does not.
    assert!(
        kill(Pid::from_raw(pid as i32), None).is_err(),
        "the killed server must be reaped, not left as a zombie"
    );

    session
        .wait_until("second didOpen after restart", |s| {
            s.events("textDocument/didOpen").len() >= 2
        })
        .await;
    assert_eq!(session.events("initialize").len(), 2);
    let reopened = session.events("textDocument/didOpen");
    assert_eq!(
        reopened[1]["params"]["textDocument"]["uri"],
        first_open["params"]["textDocument"]["uri"]
    );
    assert_ne!(
        reopened[1]["peerPid"].as_i64().unwrap(),
        pid,
        "didOpen must reach the replacement process"
    );

    // Explicit restart of a healthy server, through the ex command.
    session.test.keys(":LspRestart<CR>");
    session
        .wait_until("third didOpen after :LspRestart", |s| {
            s.events("textDocument/didOpen").len() >= 3
        })
        .await;
    assert_eq!(session.events("initialize").len(), 3);
    session.stop().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_attempts_are_bounded_until_lsp_restart() {
    let mut session = StartupSession::new();
    session.ready();
    let manager = session.test.editor.lsp_manager().unwrap();
    manager.set_restart_base_backoff(Duration::from_millis(5));
    session.wait_for_event("textDocument/didOpen").await;
    let pid = session.events("initialize")[0]["peerPid"].as_i64().unwrap();

    // Every replacement fails its handshake.
    session.initialize_response(json!({"error": {"code": -32603, "message": "boom"}}));
    sigkill(pid);
    session
        .wait_until("giving up", |s| {
            s.test.editor.lsp_status().contains("giving up")
        })
        .await;
    let attempts = session.events("initialize").len();
    assert_eq!(attempts, 1 + ovim::lsp::MAX_AUTO_RESTARTS as usize);
    for _ in 0..30 {
        session.tick().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        session.events("initialize").len(),
        attempts,
        "no further automatic restarts after giving up"
    );

    // :LspRestart resets the budget.
    session.ready();
    session.test.keys(":LspRestart<CR>");
    session
        .wait_until("manual restart", |s| {
            s.events("initialize").len() > attempts
        })
        .await;
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_request_shows_the_servers_own_error_message() {
    let mut session = StartupSession::new();
    session.initialize_response(
        json!({"result": {"capabilities": {"textDocumentSync": 1, "renameProvider": true}}}),
    );
    session.wait_for_event("textDocument/didOpen").await;
    std::fs::write(
        session.dir.path().join("response-textDocument_rename.json"),
        json!({"error": {"code": -32602, "message": "Cannot rename: contains an unresolved refactoring candidate 'Circle'"}}).to_string(),
    )
    .unwrap();

    session.test.editor.request_rename("Ring".to_string());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !session
            .test
            .editor
            .status_message()
            .contains("Rename request failed")
        {
            session.tick().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("rename failure must be reported");
    let status = session.test.editor.status_message().to_string();
    assert!(
        status.contains("Cannot rename: contains an unresolved refactoring candidate 'Circle'"),
        "server message must reach the user: {status}"
    );
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registered_file_watchers_receive_changes_to_unopened_files() {
    let mut session = StartupSession::new();
    let root = session.dir.path().canonicalize().unwrap();
    std::fs::write(
        root.join("register-capability.json"),
        json!([{
            "id": "watch-1",
            "method": "workspace/didChangeWatchedFiles",
            "registerOptions": {"watchers": [{"globPattern": "**/*.controlled"}]}
        }])
        .to_string(),
    )
    .unwrap();
    session.ready();
    session.wait_for_event("textDocument/didOpen").await;
    // The client acknowledged the registration (the fake logs the response).
    session
        .wait_until("registration ack", |s| {
            std::fs::read_to_string(s.dir.path().join("events.jsonl"))
                .unwrap_or_default()
                .contains("\"id\": 9001")
        })
        .await;
    // Let the watcher attach before touching files.
    for _ in 0..20 {
        session.tick().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let unopened = root.join("unopened.controlled");
    let uri = ovim::lsp::uri_from_file_path(&unopened).unwrap();
    let change_types = |s: &StartupSession| -> Vec<i64> {
        s.events("workspace/didChangeWatchedFiles")
            .iter()
            .flat_map(|event| event["params"]["changes"].as_array().unwrap().clone())
            .filter(|change| change["uri"] == uri.as_str())
            .map(|change| change["type"].as_i64().unwrap())
            .collect()
    };

    std::fs::write(&unopened, "one\n").unwrap();
    std::fs::write(root.join("notes.txt"), "ignored\n").unwrap();
    session
        .wait_until("created event", |s| change_types(s).contains(&1))
        .await;

    // Distinct mtime/batch so the change is not merged with the create.
    tokio::time::sleep(Duration::from_millis(400)).await;
    std::fs::write(&unopened, "two\n").unwrap();
    session
        .wait_until("changed event", |s| change_types(s).contains(&2))
        .await;

    tokio::time::sleep(Duration::from_millis(400)).await;
    std::fs::remove_file(&unopened).unwrap();
    session
        .wait_until("deleted event", |s| change_types(s).contains(&3))
        .await;

    let notes_uri = ovim::lsp::uri_from_file_path(root.join("notes.txt")).unwrap();
    assert!(
        !session
            .events("workspace/didChangeWatchedFiles")
            .iter()
            .any(|event| event.to_string().contains(notes_uri.as_str())),
        "files that match no registered glob must not be forwarded"
    );
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explorer_rename_asks_will_rename_first_and_reports_did_rename() {
    let mut session = StartupSession::new();
    let root = session.dir.path().canonicalize().unwrap();
    let filters = json!({"filters": [{"pattern": {"glob": "**/*.controlled"}}]});
    session.initialize_response(json!({"result": {"capabilities": {
        "textDocumentSync": 1,
        "workspace": {"fileOperations": {"willRename": filters, "didRename": filters}}
    }}}));
    let first = PathBuf::from(session.test.editor.buffer().file_path().unwrap());
    let other = root.join("other.controlled");
    std::fs::write(&first, "original\n").unwrap();
    std::fs::write(&other, "uses first\n").unwrap();
    session.wait_for_event("textDocument/didOpen").await;

    // The server wants `other` rewritten when `first` is renamed.
    let other_uri = ovim::lsp::uri_from_file_path(&other).unwrap();
    std::fs::write(
        root.join("response-workspace_willRenameFiles.json"),
        json!({"result": {"changes": {other_uri.as_str(): [
            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}},
             "newText": "renamed"}
        ]}}})
        .to_string(),
    )
    .unwrap();

    session.test.editor.file_tree_mut().set_root(&root);
    session
        .test
        .editor
        .request_explorer_rename(first.clone(), "second.controlled".to_string());
    session
        .wait_until("didRenameFiles", |s| {
            !s.events("workspace/didRenameFiles").is_empty()
        })
        .await;

    let will = session.events("workspace/willRenameFiles");
    assert_eq!(will.len(), 1);
    let old_uri = ovim::lsp::uri_from_file_path(&first).unwrap();
    let new_path = root.join("second.controlled");
    let new_uri = ovim::lsp::uri_from_file_path(&new_path).unwrap();
    assert_eq!(will[0]["params"]["files"][0]["oldUri"], old_uri.as_str());
    assert_eq!(will[0]["params"]["files"][0]["newUri"], new_uri.as_str());
    let did = session.events("workspace/didRenameFiles");
    assert_eq!(did[0]["params"]["files"][0]["newUri"], new_uri.as_str());

    // The server's willRename edit landed, the file moved, the buffer followed.
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "renamed first\n");
    assert!(!first.exists() && new_path.exists());
    assert!(session
        .test
        .editor
        .buffer()
        .file_path()
        .unwrap()
        .ends_with("second.controlled"));

    // The document is re-announced under its new URI.
    session
        .wait_until("didOpen for new uri", |s| {
            s.events("textDocument/didOpen")
                .iter()
                .any(|e| e["params"]["textDocument"]["uri"] == new_uri.as_str())
        })
        .await;
    session.stop().await;
}

fn hierarchy_item(name: &str, detail: &str, uri: &str, line: u32) -> Value {
    json!({
        "name": name, "kind": 5, "detail": detail, "uri": uri,
        "range": {"start": {"line": line, "character": 0}, "end": {"line": line + 3, "character": 1}},
        "selectionRange": {"start": {"line": line, "character": 6}, "end": {"line": line, "character": 12}}
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn type_hierarchy_uses_dynamic_registration_and_drills_down_with_names() {
    let mut session = StartupSession::new();
    let root = session.dir.path().canonicalize().unwrap();
    // Like Hyperion: no static typeHierarchyProvider, registered dynamically.
    std::fs::write(
        root.join("register-capability.json"),
        json!([{
            "id": "th", "method": "textDocument/prepareTypeHierarchy",
            "registerOptions": {"documentSelector": [{"language": "controlled"}]}
        }])
        .to_string(),
    )
    .unwrap();
    session.ready();
    session.wait_for_event("textDocument/didOpen").await;
    session
        .wait_until("registration ack", |s| {
            std::fs::read_to_string(s.dir.path().join("events.jsonl"))
                .unwrap_or_default()
                .contains("\"id\": 9001")
        })
        .await;
    // The client advertised dynamic registration (what Hyperion checks).
    let init = &session.events("initialize")[0];
    assert_eq!(
        init["params"]["capabilities"]["textDocument"]["typeHierarchy"]["dynamicRegistration"],
        true
    );

    let uri =
        ovim::lsp::uri_from_file_path(session.test.editor.buffer().file_path().unwrap()).unwrap();
    let uri = uri.as_str();
    let script = |method: &str, result: Value| {
        std::fs::write(
            root.join(format!("response-{method}.json")),
            json!({"result": result}).to_string(),
        )
        .unwrap();
    };
    script(
        "textDocument_prepareTypeHierarchy",
        json!([hierarchy_item("Circle", "demo.core", uri, 2)]),
    );
    script(
        "typeHierarchy_supertypes",
        json!([hierarchy_item("Shape", "demo.core", uri, 10)]),
    );
    script(
        "typeHierarchy_subtypes",
        json!([hierarchy_item("Ring", "demo.core", uri, 20)]),
    );

    session.test.keys(" th");
    session
        .wait_until("hierarchy picker", |s| {
            s.test
                .editor
                .picker()
                .is_some_and(|p| !p.filtered_results().is_empty())
        })
        .await;
    let rows: Vec<String> = session
        .test
        .editor
        .picker()
        .unwrap()
        .filtered_results()
        .iter()
        .map(|r| r.display.clone())
        .collect();
    assert!(
        rows.iter()
            .any(|r| r.contains("Shape") && r.contains("demo.core")),
        "{rows:?}"
    );
    assert!(rows.iter().any(|r| r.contains("Ring")), "{rows:?}");
    assert!(
        rows.iter().all(|r| !r.starts_with("first.controlled")),
        "names, not file:line: {rows:?}"
    );

    // Drill into the first row (Shape, a supertype): its own supertypes.
    script(
        "typeHierarchy_supertypes",
        json!([hierarchy_item("Object", "java.lang", uri, 30)]),
    );
    session.test.keys("<Tab>");
    session
        .wait_until("second level", |s| {
            s.test.editor.picker().is_some_and(|p| {
                p.filtered_results()
                    .iter()
                    .any(|r| r.display.contains("Object"))
            })
        })
        .await;
    // ... and back up.
    session.test.press_key(ovim_core::KeyCode::BackTab);
    session
        .wait_until("back to first level", |s| {
            s.test.editor.picker().is_some_and(|p| {
                p.filtered_results()
                    .iter()
                    .any(|r| r.display.contains("Shape"))
            })
        })
        .await;
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_messages_are_shown_and_message_requests_get_the_users_choice() {
    let mut session = StartupSession::new();
    let root = session.dir.path().canonicalize().unwrap();
    std::fs::write(
        root.join("outbox.json"),
        json!([
            {"method": "window/showMessage", "params": {"type": 2, "message": "Index is incomplete"}},
            {"id": 7001, "method": "window/showMessageRequest",
             "params": {"type": 1, "message": "Build failed", "actions": [{"title": "Retry"}, {"title": "Ignore"}]}}
        ])
        .to_string(),
    )
    .unwrap();
    session.ready();
    session.wait_for_event("textDocument/didOpen").await;

    session
        .wait_until("showMessage on the status line", |s| {
            s.test
                .editor
                .status_message()
                .contains("Index is incomplete")
                || s.test
                    .editor
                    .visible_toasts_newest_first(5)
                    .iter()
                    .any(|t| t.message.contains("Index is incomplete"))
        })
        .await;
    session
        .wait_until("action picker", |s| {
            s.test
                .editor
                .picker()
                .is_some_and(|p| p.is_message_action_picker())
        })
        .await;
    let rows: Vec<String> = session
        .test
        .editor
        .picker()
        .unwrap()
        .filtered_results()
        .iter()
        .map(|r| r.display.clone())
        .collect();
    assert_eq!(rows, ["Retry", "Ignore"]);

    session.test.press_key(ovim_core::KeyCode::Down);
    session.test.keys("<CR>");
    session
        .wait_until("reply", |s| {
            std::fs::read_to_string(s.dir.path().join("events.jsonl"))
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .any(|e| e["id"] == 7001 && e["result"]["title"] == "Ignore")
        })
        .await;
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dismissing_a_message_request_answers_null() {
    let mut session = StartupSession::new();
    let root = session.dir.path().canonicalize().unwrap();
    std::fs::write(
        root.join("outbox.json"),
        json!([{"id": 7002, "method": "window/showMessageRequest",
                "params": {"type": 3, "message": "Pick", "actions": [{"title": "A"}]}}])
        .to_string(),
    )
    .unwrap();
    session.ready();
    session
        .wait_until("action picker", |s| {
            s.test
                .editor
                .picker()
                .is_some_and(|p| p.is_message_action_picker())
        })
        .await;
    session.test.keys("<Esc>");
    session
        .wait_until("null reply", |s| {
            std::fs::read_to_string(s.dir.path().join("events.jsonl"))
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .any(|e| e["id"] == 7002 && e["result"].is_null() && e.get("method").is_none())
        })
        .await;
    session.stop().await;
}
