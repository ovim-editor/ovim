//! End-to-end completion behaviour against a scripted stdio language server
//! (`helpers/completion_lsp.py`): ordering, fuzzy filtering, `isIncomplete`
//! re-requests, auto-trigger, label details, resolve, snippets, additional
//! edits, insert/replace ranges, commit characters and item commands.
//!
//! The editor is driven through the same shared tick the TUI, headless and GUI
//! frontends run (`Editor::tick`).

mod helpers;

use helpers::EditorTest;
use ovim::frontend::TickState;
use ovim_core::language_catalog::{DynamicLanguageSpec, DynamicLspSpec, RegistrationOwner};
use ovim_core::{KeyCode, Modifiers};
use serde_json::{json, Value};
use std::time::Duration;

struct Session {
    test: EditorTest,
    channels: TickState,
    dir: tempfile::TempDir,
}

fn completion_provider() -> Value {
    json!({"triggerCharacters": ["."]})
}

impl Session {
    async fn new(content: &str) -> Self {
        Self::with_capabilities(
            content,
            json!({"textDocumentSync": 1, "completionProvider": completion_provider()}),
        )
        .await
    }

    async fn with_capabilities(content: &str, capabilities: Value) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let script = root.join("server.py");
        std::fs::write(&script, include_str!("helpers/completion_lsp.py")).unwrap();
        std::fs::write(root.join("capabilities.json"), capabilities.to_string()).unwrap();
        std::fs::write(root.join("completion.json"), "null").unwrap();
        std::fs::write(root.join("project.marker"), "").unwrap();

        let mut test = EditorTest::new(content);
        test.editor.enable_lsp();
        test.editor
            .language_catalog()
            .register_dynamic(
                DynamicLanguageSpec {
                    id: "complang".into(),
                    name: "Completion test language".into(),
                    extensions: vec!["cmp".into()],
                    parser: None,
                    lsp: Some(DynamicLspSpec {
                        command: vec![
                            "python3".into(),
                            script.display().to_string(),
                            root.display().to_string(),
                        ],
                        language_id: "complang".into(),
                        root_markers: vec!["project.marker".into()],
                    }),
                },
                RegistrationOwner::UserConfig {
                    source: root.join("init.lua"),
                },
                std::slice::from_ref(&root),
            )
            .unwrap();
        test.set_file_path(root.join("main.cmp").display().to_string());
        test.editor.request_lsp_init();
        let mut session = Self {
            test,
            channels: TickState::new(),
            dir,
        };
        session.wait_for_event("textDocument/didOpen").await;
        session
    }

    fn root(&self) -> std::path::PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn script(&self, name: &str, value: Value) {
        std::fs::write(self.root().join(name), value.to_string()).unwrap();
    }

    fn set_completion(&self, list: Value) {
        self.script("completion.json", list);
    }

    async fn tick(&mut self) {
        let _report = tokio::time::timeout(
            Duration::from_secs(2),
            self.test.editor.tick(&mut self.channels),
        )
        .await
        .expect("tick blocked");
    }

    fn events(&self, method: &str) -> Vec<Value> {
        std::fs::read_to_string(self.root().join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| event["method"] == method)
            .collect()
    }

    async fn wait_for_event(&mut self, method: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(10), async {
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

    /// Ticks until `done` holds (or fails the test after a few seconds).
    async fn pump_until(&mut self, what: &str, done: impl Fn(&Session) -> bool) {
        let result = tokio::time::timeout(Duration::from_secs(5), async {
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

    async fn pump_menu(&mut self) {
        self.pump_until("the completion menu", |s| {
            s.test.editor.completion_menu().is_visible()
        })
        .await;
    }

    /// Ticks for a while so anything that is going to happen has happened.
    async fn settle(&mut self) {
        for _ in 0..12 {
            self.tick().await;
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    fn labels(&self) -> Vec<String> {
        self.test
            .editor
            .completion_menu()
            .iter()
            .map(|item| item.label.clone())
            .collect()
    }

    fn line(&self, index: usize) -> String {
        self.test
            .editor
            .buffer()
            .line_text(index)
            .unwrap_or_default()
            .to_string()
    }

    fn completion_requests(&self) -> Vec<Value> {
        self.events("textDocument/completion")
    }

    async fn stop(&self) {
        self.test
            .editor
            .lsp_manager()
            .unwrap()
            .stop_server("complang")
            .await
            .unwrap();
    }
}

fn items(labels: &[&str]) -> Value {
    json!(labels
        .iter()
        .map(|label| json!({"label": label}))
        .collect::<Vec<_>>())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialize_advertises_the_completion_features_the_menu_renders() {
    let session = Session::new("").await;
    let init = session.events("initialize");
    let completion = &init[0]["params"]["capabilities"]["textDocument"]["completion"];
    let item = &completion["completionItem"];
    assert_eq!(item["snippetSupport"], true);
    assert_eq!(item["labelDetailsSupport"], true);
    assert_eq!(item["insertReplaceSupport"], true);
    assert_eq!(item["commitCharactersSupport"], true);
    assert_eq!(item["tagSupport"]["valueSet"], json!([1]));
    assert_eq!(item["deprecatedSupport"], true);
    let resolve: Vec<&str> = item["resolveSupport"]["properties"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(resolve.contains(&"documentation"));
    // additionalTextEdits must arrive with the list: accepting is synchronous.
    assert!(!resolve.contains(&"additionalTextEdits"));
    assert!(completion["contextSupport"] == true);
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dot_opens_the_menu_by_itself_in_server_sort_order() {
    let mut session = Session::new("obj").await;
    session.set_completion(json!([
        {"label": "alpha", "sortText": "0002"},
        {"label": "zeta", "sortText": "0001",
         "labelDetails": {"detail": "(int x)", "description": "com.example.Util"}},
        {"label": "beta"},
    ]));
    session.test.keys("A.");
    session.pump_menu().await;

    // sortText first ("0001" < "0002"); an item without sortText sorts by label.
    assert_eq!(session.labels(), vec!["zeta", "alpha", "beta"]);
    let zeta = session.test.editor.completion_menu().get(0).unwrap();
    let details = zeta.label_details.as_ref().unwrap();
    assert_eq!(details.detail.as_deref(), Some("(int x)"));
    assert_eq!(details.description.as_deref(), Some("com.example.Util"));

    let request = &session.completion_requests()[0]["params"]["context"];
    assert_eq!(request["triggerKind"], 2);
    assert_eq!(request["triggerCharacter"], ".");
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typing_filters_locally_camel_hump_and_keeps_server_order_within_a_tier() {
    let mut session = Session::new("").await;
    session.set_completion(items(&[
        "zzz",
        "getEmployee",
        "getEmail",
        "gemstone",
        "getName",
        "other",
    ]));
    session.test.keys("igE");
    session.pump_menu().await;
    // `gE`: a case-insensitive prefix of gemstone, camelHump of the getE...
    // names, and only a loose subsequence of getName.
    assert_eq!(
        session.labels(),
        vec!["gemstone", "getEmail", "getEmployee", "getName"]
    );

    // Keep typing: no new request is needed for a complete list.
    session.test.keys("m");
    assert_eq!(
        session.labels(),
        vec!["gemstone", "getEmail", "getEmployee", "getName"]
    );
    session.test.keys("a");
    assert_eq!(session.labels(), vec!["getEmail"]);
    session.settle().await;
    assert_eq!(
        session.completion_requests().len(),
        1,
        "a complete list must be refined locally, not re-requested"
    );

    // Backspace widens the filter again from the same list.
    session.test.press_backspace();
    session.test.press_backspace();
    assert_eq!(
        session.labels(),
        vec!["gemstone", "getEmail", "getEmployee", "getName"]
    );
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_burst_of_typing_sends_one_request_after_the_pause() {
    let mut session = Session::new("").await;
    session.set_completion(items(&["counter", "count"]));
    session.test.keys("icou");
    assert!(
        session.completion_requests().is_empty(),
        "nothing is sent while the debounce is pending"
    );
    session.pump_menu().await;
    session.test.keys("nt");
    session.settle().await;
    assert_eq!(session.completion_requests().len(), 1);
    assert_eq!(session.labels(), vec!["count", "counter"]);
    session.stop().await;
}

/// rust-analyzer marks every list incomplete, so the list is re-requested on
/// each keystroke: the item the user moved to must still be the one Enter
/// inserts after the new answer arrives.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refreshed_incomplete_list_keeps_the_item_the_user_picked() {
    let mut session = Session::new("").await;
    session.script(
        "completion.json",
        json!({"isIncomplete": true,
               "items": [{"label": "abcdef"}, {"label": "abcxyz"}, {"label": "abcz"}]}),
    );
    session.script(
        "completion-incomplete.json",
        json!({"isIncomplete": true,
               "items": [{"label": "abcdef"}, {"label": "abcxyz"}, {"label": "abczz"}]}),
    );
    session.test.keys("iab");
    session.pump_menu().await;
    assert_eq!(session.labels(), vec!["abcdef", "abcxyz", "abcz"]);

    // Typing `c` re-asks the server; the user moves to abcxyz while the
    // answer is still on its way.
    std::fs::write(session.root().join("completion-delay.txt"), "0.3").unwrap();
    session.test.keys("c");
    session.wait_for_event("textDocument/completion").await;
    session.test.keys("<C-n>");
    session
        .pump_until("the re-requested list", |s| {
            s.labels().contains(&"abczz".to_string())
        })
        .await;
    assert_eq!(
        session
            .test
            .editor
            .completion_menu()
            .selected_item()
            .map(|item| item.label.clone()),
        Some("abcxyz".to_string())
    );

    session.test.press_enter();
    assert_eq!(session.line(0), "abcxyz");
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_identifier_characters_do_not_open_the_menu() {
    let mut session = Session::new("").await;
    session.set_completion(items(&["x1", "x2"]));
    session.test.keys("ix");
    session.settle().await;
    assert!(session.completion_requests().is_empty());
    assert!(!session.test.editor.completion_menu().is_visible());
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incomplete_lists_are_requested_again_as_the_user_types() {
    let mut session = Session::new("").await;
    session.script(
        "completion.json",
        json!({"isIncomplete": true, "items": [{"label": "abc"}, {"label": "abd"}]}),
    );
    session.script(
        "completion-incomplete.json",
        json!({"isIncomplete": false, "items": [{"label": "abcdef"}, {"label": "abcxyz"}]}),
    );
    session.test.keys("iab");
    session.pump_menu().await;
    assert_eq!(session.labels(), vec!["abc", "abd"]);

    session.test.keys("c");
    session
        .pump_until("the re-requested list", |s| {
            s.labels().contains(&"abcdef".to_string())
        })
        .await;
    assert_eq!(session.labels(), vec!["abcdef", "abcxyz"]);
    let requests = session.completion_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["params"]["context"]["triggerKind"], 1);
    assert_eq!(
        requests[1]["params"]["context"]["triggerKind"], 3,
        "TriggerForIncompleteCompletions"
    );

    // Now complete: further typing refines locally.
    session.test.keys("d");
    session.settle().await;
    assert_eq!(session.completion_requests().len(), 2);
    assert_eq!(session.labels(), vec!["abcdef"]);
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_answer_that_arrives_after_more_typing_still_opens_the_menu_and_accepts_correctly() {
    let mut session = Session::new("").await;
    // The server's textEdit replaces the two characters typed when it was asked.
    session.set_completion(json!([{
        "label": "getEmail",
        "textEdit": {"range": {"start": {"line": 0, "character": 0},
                               "end": {"line": 0, "character": 2}},
                     "newText": "getEmail"}
    }]));
    std::fs::write(session.root().join("completion-delay.txt"), "0.4").unwrap();
    session.test.keys("ige");
    session.wait_for_event("textDocument/completion").await;
    // The user types on while the server is still thinking.
    session.test.keys("t");
    session.pump_menu().await;
    assert_eq!(session.labels(), vec!["getEmail"]);

    session.test.press_enter();
    assert_eq!(session.line(0), "getEmail");
    session.test.assert_mode(ovim::mode::Mode::Insert);
    session.stop().await;
}

/// A trigger-character completion answers with a range that starts at the
/// cursor; the filter text typed since is part of the range, not text that
/// goes in front of the inserted item (`foo.` + `ba` + Enter was `foo.babar`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_typed_after_a_trigger_character_is_replaced_by_an_edit_starting_at_the_cursor() {
    let mut session = Session::new("foo").await;
    session.set_completion(json!([{
        "label": "bar",
        "textEdit": {"range": {"start": {"line": 0, "character": 4},
                               "end": {"line": 0, "character": 4}},
                     "newText": "bar"}
    }]));
    session.test.keys("A.");
    session.pump_menu().await;
    session.test.keys("ba");
    session.settle().await;
    assert_eq!(session.labels(), vec!["bar"]);

    session.test.press_enter();
    assert_eq!(session.line(0), "foo.bar");
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insert_and_replace_edits_starting_at_the_cursor_keep_the_text_after_it_correct() {
    for (accept, expected) in [("<CR>", "foo.barrest"), ("<Tab>", "foo.bar")] {
        let mut session = Session::new("foorest").await;
        session.set_completion(json!([{
            "label": "bar",
            "textEdit": {"insert": {"start": {"line": 0, "character": 4},
                                    "end": {"line": 0, "character": 4}},
                         "replace": {"start": {"line": 0, "character": 4},
                                     "end": {"line": 0, "character": 8}},
                         "newText": "bar"}
        }]));
        session.test.keys("03li.");
        session.pump_menu().await;
        session.test.keys("ba");
        session.settle().await;

        session.test.keys(accept);
        assert_eq!(session.line(0), expected, "accepted with {accept}");
        session.stop().await;
    }
}

/// Keys that move the cursor or rewrite the line end the completion session:
/// Enter after `<C-o>j` used to accept the item into a different line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_that_leave_the_word_close_the_menu_instead_of_leaving_it_armed() {
    for keys in ["<C-o>j", "<C-w>", "<C-t>", "<C-u>", "<C-d>"] {
        let mut session = Session::new("foo\nbar\n").await;
        session.set_completion(items(&["alpha"]));
        session.test.keys("A.al");
        session.pump_menu().await;
        assert_eq!(session.labels(), vec!["alpha"]);

        session.test.keys(keys);
        assert!(
            !session.test.editor.completion_menu().is_visible(),
            "{keys} must close the menu"
        );
        session.test.press_enter();
        let text = session.test.buffer_content();
        assert!(
            !text.contains("alpha"),
            "Enter after {keys} accepted a stale completion: {text:?}"
        );
        session.stop().await;
    }
}

/// A cursor that moved without any key reaching insert mode (a mouse click)
/// must not let Enter rewrite the line it landed on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepting_is_refused_when_the_cursor_left_the_completion_line() {
    let mut session = Session::new("foo\nbar\n").await;
    session.set_completion(items(&["alpha"]));
    session.test.keys("A.al");
    session.pump_menu().await;

    session
        .test
        .editor
        .buffer_mut()
        .cursor_mut()
        .set_position(1, ovim_core::unicode::GraphemeCol(2));
    session.test.press_enter();
    assert!(!session.test.buffer_content().contains("alpha"));
    assert!(!session.test.editor.completion_menu().is_visible());
    session.stop().await;
}

/// Typing, accepting a completion and typing on is one insert, as in vim:
/// one undo takes it all back and `.` types the completed text again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepting_a_completion_does_not_split_the_insert() {
    let mut session = Session::new("").await;
    session.set_completion(items(&["getEmail"]));
    session.test.keys("ige");
    session.pump_menu().await;
    session.test.press_enter();
    session.test.keys("!<Esc>");
    assert_eq!(session.line(0), "getEmail!");

    session.test.keys(".");
    assert_eq!(session.line(0), "getEmailgetEmail!!");

    session.test.keys("u");
    assert_eq!(session.line(0), "getEmail!");
    session.test.keys("u");
    assert_eq!(session.line(0), "");
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_answer_for_a_line_the_user_left_is_dropped() {
    let mut session = Session::new("first\nsecond\n").await;
    session.set_completion(items(&["foo", "fob"]));
    std::fs::write(session.root().join("completion-delay.txt"), "0.3").unwrap();
    session.test.keys("A.");
    session.wait_for_event("textDocument/completion").await;
    session.test.keys("<Esc>jA");
    session.settle().await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    session.settle().await;
    assert!(!session.test.editor.completion_menu().is_visible());
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_trigger_characters_open_the_menu_and_other_punctuation_does_not() {
    let mut session = Session::with_capabilities(
        "",
        json!({"textDocumentSync": 1,
               "completionProvider": {"triggerCharacters": ["@"]}}),
    )
    .await;
    session.set_completion(items(&["author", "since"]));
    session.test.keys("i#");
    session.settle().await;
    assert!(
        session.completion_requests().is_empty(),
        "`#` is not a trigger"
    );

    session.test.keys("@");
    session.pump_menu().await;
    let request = &session.completion_requests()[0]["params"]["context"];
    assert_eq!(request["triggerKind"], 2);
    assert_eq!(request["triggerCharacter"], "@");
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn autocomplete_option_off_keeps_ctrl_space_working() {
    let mut session = Session::new("").await;
    session.set_completion(items(&["alpha", "alps"]));
    session.test.command("set noautocomplete");
    session.test.keys("ial.");
    session.settle().await;
    assert!(session.completion_requests().is_empty());

    session
        .test
        .press_with(KeyCode::Char(' '), Modifiers::CONTROL);
    session.pump_menu().await;
    assert_eq!(
        session.completion_requests()[0]["params"]["context"]["triggerKind"],
        1
    );
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_identifier_characters_and_escape_close_the_menu() {
    let mut session = Session::new("").await;
    session.set_completion(items(&["alpha", "alps"]));
    session.test.keys("ial");
    session.pump_menu().await;
    session.test.keys(" ");
    assert!(!session.test.editor.completion_menu().is_visible());

    session.test.keys("al");
    session.pump_menu().await;
    session.test.keys("<Esc>");
    assert!(!session.test.editor.completion_menu().is_visible());
    session.test.assert_mode(ovim::mode::Mode::Normal);
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_item_documentation_is_resolved_once_per_item() {
    let mut session = Session::with_capabilities(
        "",
        json!({"textDocumentSync": 1,
               "completionProvider": {"triggerCharacters": ["."], "resolveProvider": true}}),
    )
    .await;
    session.set_completion(items(&["alpha", "alps"]));
    session.script(
        "resolve.json",
        json!({"documentation": {"kind": "markdown", "value": "Docs for **it**"},
               "detail": "fn resolved()"}),
    );
    session.test.keys("ial");
    session.pump_menu().await;
    session
        .pump_until("documentation of the first item", |s| {
            s.test
                .editor
                .completion_menu()
                .selected_item()
                .is_some_and(|item| item.documentation.is_some())
        })
        .await;
    let item = session
        .test
        .editor
        .completion_menu()
        .selected_item()
        .unwrap();
    assert_eq!(item.detail.as_deref(), Some("fn resolved()"));
    let markdown = ovim_core::editor::completion_documentation_markdown(item).unwrap();
    assert!(markdown.contains("fn resolved()") && markdown.contains("Docs for **it**"));

    // Moving to the next item resolves that one; coming back does not repeat.
    session
        .test
        .press_with(KeyCode::Char('n'), Modifiers::CONTROL);
    session
        .pump_until("documentation of the second item", |s| {
            s.test
                .editor
                .completion_menu()
                .selected_item()
                .is_some_and(|item| item.documentation.is_some())
        })
        .await;
    session
        .test
        .press_with(KeyCode::Char('p'), Modifiers::CONTROL);
    session.settle().await;
    assert_eq!(session.events("completionItem/resolve").len(), 2);
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_resolve_requests_when_the_server_does_not_support_it() {
    let mut session = Session::new("").await;
    session.set_completion(items(&["alpha", "alps"]));
    session.test.keys("ial");
    session.pump_menu().await;
    session.settle().await;
    assert!(session.events("completionItem/resolve").is_empty());
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepting_applies_additional_edits_and_is_one_undo_step() {
    let mut session = Session::new("package p;\n\nnew Ar\n").await;
    session.set_completion(json!([{
        "label": "ArrayList",
        "labelDetails": {"description": "java.util"},
        "kind": 7,
        "textEdit": {"range": {"start": {"line": 2, "character": 4},
                               "end": {"line": 2, "character": 6}},
                     "newText": "ArrayList<>()"},
        "additionalTextEdits": [{
            "range": {"start": {"line": 1, "character": 0},
                      "end": {"line": 1, "character": 0}},
            "newText": "import java.util.ArrayList;\n"}]
    }]));
    session.test.keys("jjA");
    session
        .test
        .press_with(KeyCode::Char(' '), Modifiers::CONTROL);
    session.pump_menu().await;
    session.test.press_enter();

    assert_eq!(
        session.test.buffer_content(),
        "package p;\nimport java.util.ArrayList;\n\nnew ArrayList<>()\n"
    );
    // The cursor follows the text it was placed in, below the inserted import.
    assert_eq!(session.test.cursor(), (3, "new ArrayList<>()".len()));
    session.test.keys("<Esc>u");
    assert_eq!(session.test.buffer_content(), "package p;\n\nnew Ar\n");
    session.stop().await;
}

fn snippet_item(label: &str, snippet: &str) -> Value {
    json!({"label": label, "insertTextFormat": 2, "insertText": snippet})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepting_a_snippet_expands_it_and_tab_walks_the_tab_stops() {
    let mut session = Session::new("").await;
    session.set_completion(json!([snippet_item(
        "call",
        "call(${1:first}, ${2:second})$0"
    )]));
    session.test.keys("ical");
    session.pump_menu().await;
    session.test.press_enter();
    assert_eq!(session.line(0), "call(first, second)");
    // Cursor on the first placeholder; typing replaces its default text.
    assert_eq!(session.test.cursor(), (0, 5));
    assert!(session.test.editor.snippet_active());
    assert_eq!(
        session.test.editor.snippet_placeholder_highlight(),
        Some((0, 5, 10))
    );
    session.test.keys("x");
    assert_eq!(session.line(0), "call(x, second)");

    session.test.press_key(KeyCode::Tab);
    assert_eq!(session.test.cursor(), (0, 8));
    // Shift-Tab goes back, Tab forward again; typing over `second`.
    session.test.press_key(KeyCode::BackTab);
    assert_eq!(session.test.cursor(), (0, 5));
    session.test.press_key(KeyCode::Tab);
    session.test.keys("yz");
    assert_eq!(session.line(0), "call(x, yz)");

    // The last Tab lands on $0 (after the `)`) and ends the session.
    session.test.press_key(KeyCode::Tab);
    assert_eq!(session.test.cursor(), (0, "call(x, yz)".len()));
    assert!(!session.test.editor.snippet_active());
    // ...and the next Tab is an ordinary Tab again.
    session.test.press_key(KeyCode::Tab);
    assert_eq!(session.line(0), "call(x, yz) ", "pads to the next tab stop");
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn escape_leaves_the_snippet_and_insert_mode_and_undo_removes_the_expansion() {
    let mut session = Session::new("").await;
    session.set_completion(json!([snippet_item("call", "call(${1:a})")]));
    session.test.keys("ical");
    session.pump_menu().await;
    session.test.press_enter();
    assert!(session.test.editor.snippet_active());
    session.test.keys("<Esc>");
    assert!(!session.test.editor.snippet_active());
    session.test.assert_mode(ovim::mode::Mode::Normal);
    assert_eq!(session.line(0), "call(a)");
    // Back in insert mode Tab must not jump into the old tab stops.
    session.test.keys("A<Tab>");
    assert_ne!(session.test.cursor().1, 5);
    session.test.keys("<Esc>u");
    assert_eq!(session.line(0), "call(a)");
    // The typed `cal` and the expansion are one insert: one undo takes both.
    session.test.keys("u");
    assert_eq!(session.line(0), "");
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snippet_lines_follow_the_indentation_of_the_line_they_are_inserted_into() {
    let mut session = Session::new("    ").await;
    session.set_completion(json!([snippet_item("if", "if (${1:cond}) {\n\t$0\n}")]));
    session.test.keys("A");
    session.test.type_text("if");
    session.pump_menu().await;
    session.test.press_enter();
    assert_eq!(
        session.test.buffer_content(),
        "    if (cond) {\n    \t\n    }\n"
    );
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mirrors_take_over_the_edited_placeholder_when_it_is_left() {
    let mut session = Session::new("").await;
    session.set_completion(json!([snippet_item("let", "let ${1:v} = ${1:v};$0")]));
    session.test.keys("ilet");
    session.pump_menu().await;
    session.test.press_enter();
    assert_eq!(session.line(0), "let v = v;");
    session.test.keys("count");
    assert_eq!(session.line(0), "let count = v;");
    session.test.press_key(KeyCode::Tab);
    assert_eq!(session.line(0), "let count = count;");
    assert_eq!(session.test.cursor(), (0, "let count = count;".len()));
    // The mirror sync is part of the same insert session: one undo removes
    // the mirror edit, another the typing, another the expansion.
    session.test.keys("<Esc>");
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn choice_and_variable_snippets_insert_their_default_text() {
    let mut session = Session::new("").await;
    session.set_completion(json!([snippet_item(
        "kw",
        "${1|public,private|} ${TM_FILENAME_BASE}"
    )]));
    session.test.keys("ikw");
    session.pump_menu().await;
    session.test.press_enter();
    assert_eq!(session.line(0), "public main");
    session.stop().await;
}

/// OV-00476: a `${1|a,b,c|}` stop opens a small chooser over the inserted
/// first choice; Down/Up move, Enter or Tab picks it over the placeholder,
/// the next Tab moves on. Like every snippet stop it works for later stops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_snippet_choice_stop_opens_a_chooser() {
    let mut session = Session::new("").await;
    session.set_completion(json!([snippet_item(
        "start",
        "start(${1:name}, ${2|fast,slow,auto|})$0"
    )]));
    session.test.keys("ista");
    session.pump_menu().await;
    session.test.press_enter();
    assert_eq!(session.line(0), "start(name, fast)");
    // The first stop is a plain placeholder: no chooser yet.
    assert!(!session.test.editor.completion_menu().is_visible());

    // Tab to the choice stop: the chooser lists the choices in snippet order
    // with the inserted first choice highlighted.
    session.test.press_key(KeyCode::Tab);
    assert_eq!(session.test.cursor(), (0, 12));
    assert!(session.test.editor.completion_menu().is_visible());
    assert_eq!(session.labels(), ["fast", "slow", "auto"]);
    assert_eq!(
        session
            .test
            .editor
            .completion_menu()
            .selected_item()
            .unwrap()
            .label,
        "fast"
    );

    // Down, Down, Enter picks `auto` over `fast`; the stop stays current.
    session.test.press_key(KeyCode::Down);
    session.test.press_key(KeyCode::Down);
    session.test.press_enter();
    assert_eq!(session.line(0), "start(name, auto)");
    assert!(!session.test.editor.completion_menu().is_visible());
    assert!(session.test.editor.snippet_active());
    assert_eq!(session.test.cursor(), (0, 16));

    // The next Tab leaves the stop for $0 (after the `)`) and ends the session.
    session.test.press_key(KeyCode::Tab);
    assert_eq!(session.test.cursor(), (0, "start(name, auto)".len()));
    assert!(!session.test.editor.snippet_active());
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typing_over_a_choice_stop_filters_or_replaces_the_choices() {
    let mut session = Session::new("").await;
    session.set_completion(json!([snippet_item(
        "kw",
        "${1|public,private,protected|} class"
    )]));
    session.test.keys("ikw");
    session.pump_menu().await;
    session.test.press_enter();
    assert_eq!(session.line(0), "public class");
    // The chooser is open at the very first stop; `pri` narrows it to
    // `private`, Tab picks it over whatever the placeholder now holds.
    assert!(session.test.editor.completion_menu().is_visible());
    session.test.keys("pri");
    assert_eq!(session.line(0), "pri class");
    assert_eq!(session.labels(), ["private"]);
    session.test.press_key(KeyCode::Tab);
    assert_eq!(session.line(0), "private class");
    // A key that fits no choice just leaves the typed text.
    session.test.keys("<Esc>");
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_text_items_are_not_parsed_as_snippets() {
    let mut session = Session::new("").await;
    session.set_completion(json!([{"label": "cost", "insertText": "$1 ${2:x}"}]));
    session.test.keys("icos");
    session.pump_menu().await;
    session.test.press_enter();
    assert_eq!(session.line(0), "$1 ${2:x}");
    assert!(!session.test.editor.snippet_active());
    session.stop().await;
}

fn insert_replace_item() -> Value {
    json!([{
        "label": "foobaz",
        "textEdit": {
            "newText": "foobaz",
            "insert": {"start": {"line": 0, "character": 0},
                       "end": {"line": 0, "character": 2}},
            "replace": {"start": {"line": 0, "character": 0},
                        "end": {"line": 0, "character": 6}}}
    }])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enter_uses_the_insert_range_and_tab_the_replace_range() {
    for (key, expected) in [(KeyCode::Enter, "foobazoBar"), (KeyCode::Tab, "foobaz")] {
        let mut session = Session::new("fooBar").await;
        session.set_completion(insert_replace_item());
        // Cursor after `fo`, in the middle of the identifier.
        session.test.keys("0lli");
        session
            .test
            .press_with(KeyCode::Char(' '), Modifiers::CONTROL);
        session.pump_menu().await;
        session.test.press_key(key);
        assert_eq!(session.line(0), expected, "{key:?}");
        session.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn commit_characters_accept_a_chosen_item_but_not_a_partly_typed_one() {
    let mut session = Session::new("").await;
    session.set_completion(json!([
        {"label": "alpha", "commitCharacters": ["("]},
        {"label": "alps", "commitCharacters": ["("]},
    ]));
    session.test.keys("ial");
    session.pump_menu().await;
    // Nothing chosen yet, prefix incomplete: `(` is just typed.
    session.test.keys("(");
    assert_eq!(session.line(0), "al(");
    session.test.keys("<BS><BS><BS>al");
    session.pump_menu().await;

    // Choose the second item explicitly: `(` commits it.
    session
        .test
        .press_with(KeyCode::Char('n'), Modifiers::CONTROL);
    session.test.keys("(");
    assert_eq!(session.line(0), "alps(");
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn item_commands_run_after_the_item_is_accepted() {
    let mut session = Session::with_capabilities(
        "",
        json!({"textDocumentSync": 1,
               "completionProvider": {"triggerCharacters": ["."]},
               "executeCommandProvider": {"commands": ["server.afterInsert"]}}),
    )
    .await;
    session.set_completion(json!([{
        "label": "alpha",
        "command": {"title": "after", "command": "server.afterInsert", "arguments": [7]}
    }]));
    session.test.keys("ial");
    session.pump_menu().await;
    session.test.press_enter();
    let event = session.wait_for_event("workspace/executeCommand").await;
    assert_eq!(event["params"]["command"], "server.afterInsert");
    assert_eq!(event["params"]["arguments"], json!([7]));
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_side_parameter_hints_command_asks_for_signature_help() {
    let mut session = Session::with_capabilities(
        "",
        json!({"textDocumentSync": 1,
               "completionProvider": {"triggerCharacters": ["."]},
               "signatureHelpProvider": {"triggerCharacters": ["("]}}),
    )
    .await;
    session.set_completion(json!([{
        "label": "call",
        "command": {"title": "hints", "command": "editor.action.triggerParameterHints"}
    }]));
    session.test.keys("ical");
    session.pump_menu().await;
    session.test.press_enter();
    session.wait_for_event("textDocument/signatureHelp").await;
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn item_defaults_of_a_completion_list_apply_to_every_item() {
    let mut session = Session::new("").await;
    session.set_completion(json!({
        "isIncomplete": false,
        "itemDefaults": {
            "insertTextFormat": 2,
            "commitCharacters": [";"],
            "editRange": {"start": {"line": 0, "character": 0},
                          "end": {"line": 0, "character": 2}}
        },
        "items": [{"label": "call", "textEditText": "call(${1:x})"}]
    }));
    session.test.keys("ica");
    session.pump_menu().await;
    let item = session.test.editor.completion_menu().get(0).unwrap();
    assert_eq!(item.commit_characters, Some(vec![";".to_string()]));
    session.test.press_enter();
    assert_eq!(session.line(0), "call(x)");
    assert!(session.test.editor.snippet_active());
    session.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deprecated_items_are_flagged_by_tag_and_by_the_legacy_field() {
    let mut session = Session::new("").await;
    session.set_completion(json!([
        {"label": "old_a", "tags": [1]},
        {"label": "old_b", "deprecated": true},
        {"label": "old_c"},
    ]));
    session.test.keys("iol");
    session.pump_menu().await;
    let menu = session.test.editor.completion_menu();
    let flags: Vec<bool> = menu
        .iter()
        .map(ovim_core::editor::completion_item_is_deprecated)
        .collect();
    assert_eq!(flags, vec![true, true, false]);
    session.stop().await;
}
