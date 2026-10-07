//! A scriptable fake language server (`scripted_lsp.py`) wired into an
//! [`EditorTest`], shared by the LSP regression tests.
//!
//! Every project root is a directory holding a `project.marker` and a
//! `main.fk` file; the server started for it uses that directory as its
//! control directory (see the script for the files it reads and writes), so a
//! test can tell the servers of two roots apart.
#![allow(dead_code)]

use super::EditorTest;
use ovim::frontend::TickState;
use ovim_core::language_catalog::{DynamicLanguageSpec, DynamicLspSpec, RegistrationOwner};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct FakeLsp {
    pub test: EditorTest,
    pub channels: TickState,
    dir: tempfile::TempDir,
    script: PathBuf,
    roots: Vec<PathBuf>,
}

pub fn default_capabilities() -> Value {
    json!({
        "textDocumentSync": {"openClose": true, "change": 2, "save": {"includeText": false}},
        "completionProvider": {"triggerCharacters": ["."]},
        "hoverProvider": true,
    })
}

impl FakeLsp {
    /// One project root whose `main.fk` is the current buffer.
    pub async fn start(content: &str) -> Self {
        Self::start_with(content, 1, default_capabilities()).await
    }

    /// `roots` project roots; the first root's `main.fk` is the current buffer
    /// and has been opened on its server when this returns.
    pub async fn start_with(content: &str, roots: usize, capabilities: Value) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let script = base.join("server.py");
        std::fs::write(&script, include_str!("scripted_lsp.py")).unwrap();
        let roots: Vec<PathBuf> = (0..roots)
            .map(|index| {
                let root = base.join(format!("root{index}"));
                std::fs::create_dir_all(&root).unwrap();
                std::fs::write(root.join("project.marker"), "").unwrap();
                std::fs::write(root.join("capabilities.json"), capabilities.to_string()).unwrap();
                std::fs::write(root.join("main.fk"), if index == 0 { content } else { "" })
                    .unwrap();
                root
            })
            .collect();

        let mut test = EditorTest::new(content);
        test.editor.enable_lsp();
        test.editor
            .language_catalog()
            .register_dynamic(
                DynamicLanguageSpec {
                    id: "fakels".into(),
                    name: "Fake language".into(),
                    extensions: vec!["fk".into()],
                    parser: None,
                    lsp: Some(DynamicLspSpec {
                        command: vec![
                            "python3".into(),
                            script.display().to_string(),
                            "@root".into(),
                        ],
                        language_id: "fakels".into(),
                        root_markers: vec!["project.marker".into()],
                    }),
                },
                RegistrationOwner::UserConfig {
                    source: base.join("init.lua"),
                },
                std::slice::from_ref(&base),
            )
            .unwrap();
        test.set_file_path(roots[0].join("main.fk").display().to_string());
        test.editor.request_lsp_init();
        let mut lsp = Self {
            test,
            channels: TickState::new(),
            dir,
            script,
            roots,
        };
        lsp.wait_for_event(0, "textDocument/didOpen").await;
        lsp
    }

    pub fn root(&self, root: usize) -> &Path {
        &self.roots[root]
    }

    pub fn file(&self, root: usize) -> PathBuf {
        self.roots[root].join("main.fk")
    }

    pub fn uri(&self, root: usize) -> lsp_types::Uri {
        ovim::lsp::uri_from_file_path(self.file(root).display().to_string()).unwrap()
    }

    /// Scripts the server of `root`: writes `name` in its control directory.
    pub fn script(&self, root: usize, name: &str, value: Value) {
        self.script_text(root, name, &value.to_string());
    }

    pub fn script_text(&self, root: usize, name: &str, text: &str) {
        std::fs::write(self.roots[root].join(name), text).unwrap();
    }

    pub async fn tick(&mut self) {
        let _report = tokio::time::timeout(
            Duration::from_secs(5),
            self.test.editor.tick(&mut self.channels),
        )
        .await
        .expect("tick blocked");
    }

    /// Every message of `method` the server of `root` has received.
    pub fn events(&self, root: usize, method: &str) -> Vec<Value> {
        std::fs::read_to_string(self.roots[root].join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| event["method"] == method)
            .collect()
    }

    /// The client's reply to the request with `id` the server of `root` sent.
    pub fn reply_to(&self, root: usize, id: i64) -> Option<Value> {
        std::fs::read_to_string(self.roots[root].join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|event| event["id"] == id && event.get("method").is_none())
    }

    /// Protocol violations the server noticed (`didChange` before `didOpen`...).
    pub fn violations(&self, root: usize) -> String {
        std::fs::read_to_string(self.roots[root].join("sync-errors.log")).unwrap_or_default()
    }

    /// Ticks until `done` holds (or fails the test after a few seconds).
    pub async fn pump_until(&mut self, what: &str, done: impl Fn(&FakeLsp) -> bool) {
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                self.tick().await;
                if done(self) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(result.is_ok(), "timed out waiting for {what}");
    }

    pub async fn wait_for_event(&mut self, root: usize, method: &str) -> Value {
        self.pump_until(&format!("{method} on root {root}"), |lsp| {
            !lsp.events(root, method).is_empty()
        })
        .await;
        self.events(root, method).last().unwrap().clone()
    }

    /// Ticks for a while so anything that is going to happen has happened.
    pub async fn settle(&mut self) {
        for _ in 0..12 {
            self.tick().await;
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    /// Opens `root`'s `main.fk` in a new tab and waits until its server has it.
    pub async fn open_in_tab(&mut self, root: usize) {
        self.test.editor.new_tab();
        self.test.editor.open_file(&self.file(root)).unwrap();
        self.wait_for_event(root, "textDocument/didOpen").await;
    }

    /// Starts a companion server (`fakels:comp`) for `root`'s project. It
    /// logs into its own control directory, which is returned.
    pub async fn start_companion(&self, root: usize) -> PathBuf {
        let control = self
            .dir
            .path()
            .canonicalize()
            .unwrap()
            .join("companion-ctl");
        std::fs::create_dir_all(&control).unwrap();
        std::fs::write(
            control.join("capabilities.json"),
            default_capabilities().to_string(),
        )
        .unwrap();
        let manager = self.manager();
        let server_id = ovim_core::lsp::companion_server_id("fakels", "comp");
        manager
            .start_companion_server(
                &server_id,
                "python3",
                vec![
                    self.script.display().to_string(),
                    control.display().to_string(),
                ],
                &self.roots[root],
            )
            .await
            .unwrap();
        manager.start_notification_listener(server_id).await;
        control
    }

    pub fn manager(&self) -> std::sync::Arc<ovim_core::lsp::LspManager> {
        self.test.editor.lsp_manager().unwrap()
    }

    pub async fn stop(&self) {
        let manager = self.manager();
        for server in manager.get_active_servers().await {
            let _ = manager.stop_server(&server).await;
        }
    }
}

/// Every message of `method` the fake server logging into `control` received.
pub fn events_in(control: &Path, method: &str) -> Vec<Value> {
    std::fs::read_to_string(control.join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["method"] == method)
        .collect()
}
