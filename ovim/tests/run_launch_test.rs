//! Run / debug launch flow, end to end through the shared frontend tick.
//!
//! A scripted stdio language server plays Hyperion (`hyperion.resolveLaunch`),
//! a scripted stdio debug adapter plays `hyperion-lsp dap`, and the "JDK" is a
//! shell script, so these tests need no Java and no Hyperion binary.

mod helpers;

use helpers::EditorTest;
use ovim::frontend::TickState;
use ovim::mode::Mode;
use ovim_core::language_catalog::{DynamicLanguageSpec, DynamicLspSpec, RegistrationOwner};
use ovim_core::launch::{LineKind, RunOutcome, RunStatus};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `JAVA_HOME` is process-global; tests that fake the JDK take turns.
static JDK_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Session {
    test: EditorTest,
    channels: TickState,
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Session {
    /// A ready language server that advertises `commands`.
    async fn new(commands: &[&str]) -> Self {
        Self::with_capabilities(commands, json!({})).await
    }

    async fn with_capabilities(commands: &[&str], extra: Value) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let script = root.join("server.py");
        std::fs::write(&script, include_str!("helpers/controlled_lsp.py")).unwrap();
        std::fs::write(root.join("project.marker"), "").unwrap();
        std::fs::write(root.join("initialize-response.json"), {
            let mut capabilities = json!({
                "textDocumentSync": 1,
                "executeCommandProvider": {"commands": commands}
            });
            for (key, value) in extra.as_object().unwrap() {
                capabilities[key] = value.clone();
            }
            json!({"result": {"capabilities": capabilities}}).to_string()
        })
        .unwrap();
        let mut test = EditorTest::new("class Main {}\n");
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
        std::fs::write(root.join("Main.controlled"), "class Main {}\n").unwrap();
        test.set_file_path(root.join("Main.controlled").display().to_string());
        test.editor.request_lsp_init();
        let mut session = Self {
            test,
            channels: TickState::new(),
            _dir: dir,
            root,
        };
        session.wait_for_event("textDocument/didOpen").await;
        session
    }

    async fn tick(&mut self) {
        let _report = tokio::time::timeout(
            Duration::from_secs(1),
            self.test.editor.tick(&mut self.channels),
        )
        .await
        .expect("the launch flow blocked the input/render tick");
    }

    fn events_in(&self, file: &str, key: &str, name: &str) -> Vec<Value> {
        std::fs::read_to_string(self.root.join(file))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| event[key] == name)
            .collect()
    }

    fn lsp_events(&self, method: &str) -> Vec<Value> {
        self.events_in("events.jsonl", "method", method)
    }

    /// Every message the server received, requests and responses alike.
    fn lsp_events_raw(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect()
    }

    async fn wait_for_event(&mut self, method: &str) {
        self.until(&format!("LSP event {method}"), |s| {
            !s.lsp_events(method).is_empty()
        })
        .await;
    }

    /// Ticks until `done`, failing (with the console text) after 10 seconds.
    async fn until(&mut self, what: &str, done: impl Fn(&mut Session) -> bool) {
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
        if result.is_err() {
            panic!(
                "timed out waiting for {what}\nstatus: {:?}\nconsole:\n{}",
                self.test.editor.status_message(),
                self.console_text()
            );
        }
    }

    fn script_resolve(&self, result: Value) {
        std::fs::write(
            self.root.join("response-workspace_executeCommand.json"),
            json!({"result": result}).to_string(),
        )
        .unwrap();
    }

    fn console_text(&self) -> String {
        self.test
            .editor
            .run_console()
            .viewed()
            .map(|run| {
                run.lines
                    .iter()
                    .map(|l| format!("[{:?}] {}", l.kind, l.text))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    fn run_finished(&self) -> bool {
        matches!(
            self.test.editor.run_console().viewed().map(|r| &r.status),
            Some(RunStatus::Done(_))
        )
    }

    fn outcome(&self) -> RunOutcome {
        match &self.test.editor.run_console().viewed().unwrap().status {
            RunStatus::Done(outcome) => outcome.clone(),
            other => panic!("run still active: {other:?}"),
        }
    }

    fn write_script(&self, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = self.root.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Installs a fake `$JAVA_HOME/bin/java` running `body`.
    fn fake_java(&self, body: &str) {
        std::fs::create_dir_all(self.root.join("jdk/bin")).unwrap();
        self.write_script("jdk/bin/java", body);
        // SAFETY: serialized by JDK_LOCK; no other thread reads the variable.
        unsafe { std::env::set_var("JAVA_HOME", self.root.join("jdk")) };
    }

    fn main_plan(&self, build: Option<Value>) -> Value {
        json!({
            "name": "Main (app)", "kind": "main", "language": "java",
            "projectRoot": self.root, "moduleDir": self.root, "buildTool": "none",
            "build": build,
            "launch": {
                "mainClass": "com.example.Main", "classpath": "/cp/classes",
                "args": ["one", "two words"], "jvmArgs": ["-Xmx8m"],
                "cwd": self.root, "projectRoot": self.root, "env": {"GREETING": "hi"}
            },
            "warnings": []
        })
    }

    async fn stop_lsp(&self) {
        let _ = self
            .test
            .editor
            .lsp_manager()
            .unwrap()
            .stop_server("controlled")
            .await;
    }
}

fn resolve_commands() -> [&'static str; 2] {
    ["hyperion.resolveLaunch", "hyperion.runConfigurations"]
}

fn process_alive(pid: i64) -> bool {
    // SAFETY: probing for existence only.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

// ---------------------------------------------------------------------------
// Run (no debugger)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_at_cursor_resolves_via_the_server_and_streams_output_into_a_persistent_console() {
    let _jdk = JDK_LOCK.lock().await;
    let mut s = Session::new(&resolve_commands()).await;
    s.fake_java(
        "echo \"args: $@\" > \"$JAVA_HOME/../java-args.txt\"\n\
         echo \"greeting=$GREETING cwd=$(pwd)\"\n\
         echo 'to stdout'\n\
         echo 'to stderr' >&2\n\
         exit 3",
    );
    s.script_resolve(s.main_plan(None));

    s.test.keys(" rr");
    s.until("the run to finish", |s| s.run_finished()).await;

    let resolve = s.lsp_events("workspace/executeCommand");
    assert_eq!(resolve.len(), 1);
    assert_eq!(resolve[0]["params"]["command"], "hyperion.resolveLaunch");
    let args = &resolve[0]["params"]["arguments"][0];
    assert_eq!(args["target"], "auto");
    assert!(args["uri"].as_str().unwrap().ends_with("/Main.controlled"));
    assert_eq!(args["position"]["line"], 0);

    let java_args = std::fs::read_to_string(s.root.join("java-args.txt")).unwrap();
    assert_eq!(
        java_args.trim(),
        "args: -Xmx8m -cp /cp/classes com.example.Main one two words"
    );

    let run = s.test.editor.run_console().viewed().unwrap();
    let kinds: Vec<(LineKind, &str)> = run
        .lines
        .iter()
        .map(|l| (l.kind, l.text.as_str()))
        .collect();
    assert!(
        kinds.contains(&(LineKind::Stdout, "to stdout")),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&(LineKind::Stderr, "to stderr")),
        "{kinds:?}"
    );
    assert!(
        run.lines
            .iter()
            .any(|l| l.text.contains("greeting=hi") && l.text.contains(s.root.to_str().unwrap())),
        "env and cwd from the plan must reach the process: {kinds:?}"
    );
    assert_eq!(run.exit_code, Some(3));
    assert_eq!(s.outcome(), RunOutcome::Failed);
    assert!(run.duration.is_some());
    assert!(
        run.status_text().starts_with("failed (exit 3) in "),
        "{}",
        run.status_text()
    );

    // The console outlives the process.
    s.tick().await;
    assert!(s.test.editor.run_console().open);
    assert_eq!(
        s.test.editor.run_console().viewed().unwrap().lines.len(),
        run_len(&s)
    );
    s.stop_lsp().await;
}

fn run_len(s: &Session) -> usize {
    s.test.editor.run_console().viewed().unwrap().lines.len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_runs_first_without_blocking_the_tick_and_its_failure_aborts_the_launch() {
    let _jdk = JDK_LOCK.lock().await;
    let mut s = Session::new(&resolve_commands()).await;
    s.fake_java("touch \"$JAVA_HOME/../java-ran\"");
    let source = s.root.join("src/A.java");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, "class A {\n    int x\n}\n").unwrap();
    let build = s.write_script(
        "build.sh",
        &format!(
            "sleep 1\n\
             echo '> Task :compileJava FAILED'\n\
             echo '{path}:2: error: cannot find symbol' >&2\n\
             sleep 0.2\n\
             echo 'noise on stdout between the diagnostic lines'\n\
             sleep 0.2\n\
             echo '    int x' >&2\n\
             echo '        ^' >&2\n\
             echo '  symbol:   class Foo' >&2\n\
             echo '  location: class A' >&2\n\
             echo '1 error' >&2\n\
             exit 1",
            path = source.display()
        ),
    );
    s.script_resolve(s.main_plan(Some(json!({"argv": [build], "cwd": s.root}))));

    s.test.keys(" rr");
    // The build is slow: the tick must stay responsive while it runs.
    let started = std::time::Instant::now();
    s.until("the build to start", |s| {
        matches!(
            s.test.editor.run_console().viewed().map(|r| &r.status),
            Some(RunStatus::Active(ovim_core::launch::RunPhase::Building))
        )
    })
    .await;
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "build must not be awaited in the tick"
    );

    s.until("the build to finish", |s| s.run_finished()).await;
    assert_eq!(s.outcome(), RunOutcome::BuildFailed);
    assert!(
        !s.root.join("java-ran").exists(),
        "a failed build must abort the launch"
    );

    let qf = s.test.editor.quickfix_list();
    assert_eq!(qf.len(), 1, "{:?}", qf.entries());
    let entry = &qf.entries()[0];
    assert_eq!(entry.filename.as_deref(), Some(source.as_path()));
    assert_eq!(
        (entry.lnum, entry.col),
        (2, 9),
        "column comes from the caret line\nentries={:?}\nruns={} status={:?}\n{}",
        qf.entries(),
        s.test.editor.run_console().runs.len(),
        s.test
            .editor
            .run_console()
            .viewed()
            .map(|r| r.status.clone()),
        s.console_text()
    );
    assert!(entry.text.contains("symbol: class Foo"), "{}", entry.text);
    assert!(s.test.editor.is_quickfix_window_open());
    assert!(
        s.test.editor.status_message().contains("Build failed"),
        "{}",
        s.test.editor.status_message()
    );
    assert!(s.test.editor.status_message().contains("Launch aborted"));
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_successful_build_is_followed_by_the_launch_and_the_edit_is_saved_first() {
    let _jdk = JDK_LOCK.lock().await;
    let mut s = Session::new(&resolve_commands()).await;
    s.fake_java("test -f \"$JAVA_HOME/../built\" && echo build-was-first");
    let build = s.write_script(
        "build.sh",
        &format!(
            "cp {main} \"{root}/built\"; echo building",
            main = s.root.join("Main.controlled").display(),
            root = s.root.display()
        ),
    );
    s.script_resolve(s.main_plan(Some(json!({"argv": [build], "cwd": s.root}))));

    // An unsaved edit must reach the build.
    s.test.keys("A // edited<Esc>");
    s.test.keys(" rr");
    s.until("the run to finish", |s| s.run_finished()).await;

    assert_eq!(s.outcome(), RunOutcome::Succeeded);
    assert!(
        s.console_text().contains("build-was-first"),
        "{}",
        s.console_text()
    );
    let built = std::fs::read_to_string(s.root.join("built")).unwrap();
    assert!(
        built.contains("// edited"),
        "unsaved edits are saved before the build: {built}"
    );
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_kills_the_program_and_everything_it_started() {
    let _jdk = JDK_LOCK.lock().await;
    let mut s = Session::new(&resolve_commands()).await;
    // java "starts" a helper that would outlive it.
    s.fake_java("sleep 300 &\necho $! > \"$JAVA_HOME/../helper.pid\"\necho started\nwait");
    s.script_resolve(s.main_plan(None));

    s.test.keys(" rr");
    s.until("the program to start", |s| {
        s.console_text().contains("started")
    })
    .await;
    let helper: i64 = std::fs::read_to_string(s.root.join("helper.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(process_alive(helper));

    s.test.keys(" rs");
    s.until("the run to stop", |s| s.run_finished()).await;
    assert_eq!(s.outcome(), RunOutcome::Stopped);
    for _ in 0..100 {
        if !process_alive(helper) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !process_alive(helper),
        "Stop must kill the whole process group"
    );
    assert!(!s.test.editor.is_launch_active());
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coloured_program_output_shows_as_plain_text() {
    let mut s = Session::new(&resolve_commands()).await;
    let script = s.write_script("colours.sh", "printf '\\033[31mred text\\033[0m plain\\n'");
    s.test.command(&format!("set makeprg={}", script.display()));
    s.test.command("make");
    s.until("the run to finish", |s| s.run_finished()).await;
    let lines: Vec<String> = s
        .test
        .editor
        .run_console()
        .viewed()
        .unwrap()
        .lines
        .iter()
        .map(|l| l.text.clone())
        .collect();
    assert!(lines.contains(&"red text plain".to_string()), "{lines:?}");
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rerun_replaces_the_running_program() {
    let _jdk = JDK_LOCK.lock().await;
    let mut s = Session::new(&resolve_commands()).await;
    s.fake_java("echo \"run $$\" >> \"$JAVA_HOME/../runs.txt\"\nsleep 300");
    s.script_resolve(s.main_plan(None));

    s.test.keys(" rr");
    s.until("first run", |s| s.root.join("runs.txt").exists())
        .await;
    s.test.keys(" rl");
    s.until("second run", |s| {
        std::fs::read_to_string(s.root.join("runs.txt"))
            .map(|t| t.lines().count() == 2)
            .unwrap_or(false)
    })
    .await;
    assert_eq!(s.test.editor.run_console().runs.len(), 2);
    assert_eq!(
        s.test.editor.run_console().runs[0].status,
        RunStatus::Done(RunOutcome::Stopped),
        "the replaced run is kept, marked stopped"
    );
    s.test.keys(" rs");
    s.until("stop", |s| s.run_finished()).await;
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_program_flooding_its_output_is_held_back_and_stop_ends_it_promptly() {
    let mut s = Session::new(&resolve_commands()).await;
    s.test.command("set makeprg=yes");
    s.test.command("make");
    s.until("output to flow", |s| run_len(s) > 5_000).await;
    // Many ticks' worth of a program that never stops writing.
    for _ in 0..50 {
        s.tick().await;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let lines = run_len(&s);
    assert!(
        lines <= ovim_core::launch::console::MAX_CONSOLE_LINES,
        "{lines}"
    );

    s.test.keys(" rs");
    s.until("the run to stop", |s| s.run_finished()).await;
    assert_eq!(s.outcome(), RunOutcome::Stopped);
    assert!(!s.test.editor.is_launch_active());
    assert!(!s
        .test
        .editor
        .run_console()
        .viewed()
        .unwrap()
        .status
        .is_active());
    s.stop_lsp().await;
}

// ---------------------------------------------------------------------------
// Resolution fallbacks and messages
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_nothing_runnable_and_no_config_the_message_says_what_to_do() {
    let mut s = Session::new(&resolve_commands()).await;
    s.script_resolve(Value::Null);

    s.test.press_key(ovim_core::KeyCode::F(5));
    s.until("the launch to give up", |s| s.run_finished()).await;
    let status = s.test.editor.status_message().to_string();
    assert!(status.contains("Nothing to debug here"), "{status}");
    assert!(status.contains(".ovim/debug.toml"), "actionable: {status}");
    assert!(!status.contains("configurationDone"), "{status}");
    assert!(!s.test.editor.is_debug_active());
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_without_resolve_launch_falls_back_to_debug_toml_and_says_so() {
    let _jdk = JDK_LOCK.lock().await;
    let mut s = Session::new(&["something.else"]).await;
    s.fake_java("echo from-config");
    std::fs::create_dir_all(s.root.join(".ovim")).unwrap();
    std::fs::write(
        s.root.join(".ovim/debug.toml"),
        "[[config]]\nname = \"App\"\ntype = \"launch\"\nmain_class = \"a.B\"\nclasspath = \"out\"\n",
    )
    .unwrap();

    s.test.keys(" rr");
    s.until("the run to finish", |s| s.run_finished()).await;
    assert_eq!(s.outcome(), RunOutcome::Succeeded);
    assert!(s.console_text().contains("from-config"));
    assert!(
        s.test.editor.lsp_manager().is_some()
            && s.lsp_events("workspace/executeCommand").is_empty(),
        "the server does not list the command, so it must not be sent"
    );
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn several_configs_open_a_picker_and_choosing_one_runs_it() {
    let _jdk = JDK_LOCK.lock().await;
    let mut s = Session::new(&["x"]).await;
    s.fake_java("echo \"ran $@\"");
    std::fs::create_dir_all(s.root.join(".ovim")).unwrap();
    std::fs::write(
        s.root.join(".ovim/debug.toml"),
        "[[config]]\nname = \"First\"\ntype = \"launch\"\nmain_class = \"a.First\"\n\n\
         [[config]]\nname = \"Second\"\ntype = \"launch\"\nmain_class = \"a.Second\"\n\n\
         [[config]]\nname = \"Attach\"\ntype = \"attach\"\nport = 1\n",
    )
    .unwrap();

    s.test.keys(" rc");
    s.until("the picker", |s| s.test.editor.mode() == Mode::Picker)
        .await;
    // Attach configs cannot be run without a debugger and are not offered.
    s.test.type_text("Second");
    s.tick().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    s.tick().await;
    s.test.press_enter();
    s.until("the run", |s| s.run_finished()).await;
    assert!(
        s.console_text().contains("ran a.Second"),
        "{}",
        s.console_text()
    );
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_config_picker_opens_next_to_a_running_program_and_leaves_it_alone() {
    let mut s = Session::new(&["x"]).await;
    std::fs::create_dir_all(s.root.join(".ovim")).unwrap();
    std::fs::write(
        s.root.join(".ovim/debug.toml"),
        "[[config]]\nname = \"First\"\ntype = \"launch\"\nmain_class = \"a.First\"\n\n\
         [[config]]\nname = \"Second\"\ntype = \"launch\"\nmain_class = \"a.Second\"\n",
    )
    .unwrap();
    let script = s.write_script(
        "long.sh",
        "echo $$ > \"$(dirname \"$0\")/long.pid\"\necho started\nexec sleep 300",
    );
    s.test.command(&format!("set makeprg={}", script.display()));
    s.test.command("make");
    s.until("the program to start", |s| {
        s.console_text().contains("started")
    })
    .await;
    let pid: i64 = std::fs::read_to_string(s.root.join("long.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    for keys in [" rc", " rC", " dC"] {
        s.test.press_esc();
        s.test.keys(keys);
        s.until("the picker", |s| s.test.editor.mode() == Mode::Picker)
            .await;
        assert!(process_alive(pid), "{keys} killed the running program");
        assert!(s.test.editor.is_launch_active(), "{keys}");
        assert_eq!(
            s.test.editor.run_console().runs.len(),
            1,
            "{keys}: looking up configurations is not a run of its own"
        );
        assert!(s.test.editor.run_console().runs[0].status.is_active());
        s.test.press_esc();
        s.tick().await;
    }

    // Stop still ends the program that was running.
    s.test.keys(" rs");
    s.until("the run to stop", |s| s.run_finished()).await;
    assert_eq!(s.outcome(), RunOutcome::Stopped);
    for _ in 0..100 {
        if !process_alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!process_alive(pid));
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_cancels_a_configuration_lookup_that_is_still_waiting_for_the_server() {
    let mut s = Session::new(&resolve_commands()).await;
    std::fs::create_dir_all(s.root.join(".ovim")).unwrap();
    std::fs::write(
        s.root.join(".ovim/debug.toml"),
        "[[config]]\nname = \"First\"\ntype = \"launch\"\nmain_class = \"a.First\"\n\n\
         [[config]]\nname = \"Second\"\ntype = \"launch\"\nmain_class = \"a.Second\"\n",
    )
    .unwrap();
    // The lookup is in flight: only a tick hands its result to the editor.
    s.test.keys(" rc");
    assert!(s.test.editor.mode() != Mode::Picker);
    assert!(!s.test.editor.is_launch_active());
    s.test.keys(" rs");
    for _ in 0..30 {
        s.tick().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        s.test.editor.mode() != Mode::Picker,
        "a cancelled lookup must not open the picker later"
    );
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_broken_debug_toml_is_reported_not_silently_ignored() {
    let mut s = Session::new(&["x"]).await;
    std::fs::create_dir_all(s.root.join(".ovim")).unwrap();
    std::fs::write(
        s.root.join(".ovim/debug.toml"),
        "[[config]]\nname = \"Half\"\ntype = \"launch\"\n",
    )
    .unwrap();
    s.test.keys(" rr");
    s.until("give up", |s| s.run_finished()).await;
    assert!(
        s.console_text().contains("Half") && s.console_text().contains("incomplete"),
        "{}",
        s.console_text()
    );
    s.stop_lsp().await;
}

// ---------------------------------------------------------------------------
// Debug
// ---------------------------------------------------------------------------

/// Both fakes log to `events.jsonl`; give the DAP one its own directory.
struct DebugSession {
    inner: Session,
    dap_dir: PathBuf,
    _jdk: tokio::sync::MutexGuard<'static, ()>,
}

/// Port the fake JVM claims to listen on (the fake adapter does not connect).
const FAKE_JDWP_PORT: u16 = 41999;

impl DebugSession {
    /// The debuggee is a fake JVM that announces its JDWP port and then
    /// echoes its stdin until EOF.
    async fn new(commands: &[&str]) -> Self {
        Self::with_jvm(
            commands,
            "while read line; do echo \"in:$line\"; done\nexit 0",
        )
        .await
    }

    /// `after_listening` is the fake JVM's script after it has printed the
    /// `Listening for transport` line.
    async fn with_jvm(commands: &[&str], after_listening: &str) -> Self {
        let jdk = JDK_LOCK.lock().await;
        let inner = Session::new(commands).await;
        inner.fake_java(&format!(
            "echo 'Listening for transport dt_socket at address: {FAKE_JDWP_PORT}'\n{after_listening}"
        ));
        let dap_dir = inner.root.join("dap");
        std::fs::create_dir_all(&dap_dir).unwrap();
        Self {
            inner,
            dap_dir,
            _jdk: jdk,
        }
    }

    fn adapter(&self, scenario: Value) -> (String, Vec<String>) {
        let script = self.dap_dir.join("fake_dap.py");
        std::fs::write(&script, include_str!("helpers/fake_dap.py")).unwrap();
        std::fs::write(self.dap_dir.join("scenario.json"), scenario.to_string()).unwrap();
        (
            "python3".to_string(),
            vec![
                script.display().to_string(),
                self.dap_dir.display().to_string(),
            ],
        )
    }

    fn requests(&self, command: &str) -> Vec<Value> {
        dap_requests(&self.dap_dir, command)
    }

    fn pid(&self) -> Option<i64> {
        std::fs::read_to_string(self.dap_dir.join("events.jsonl"))
            .ok()?
            .lines()
            .find_map(|l| serde_json::from_str::<Value>(l).ok()?["adapterPid"].as_i64())
    }
}

fn dap_requests(dir: &Path, command: &str) -> Vec<Value> {
    std::fs::read_to_string(dir.join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["command"] == command)
        .collect()
}

async fn wait_gone(pid: i64) {
    for _ in 0..200 {
        if !process_alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn debug_at_cursor_starts_the_jvm_itself_attaches_the_adapter_and_keeps_output_after_it_ends()
{
    let mut d = DebugSession::with_jvm(
        &resolve_commands(),
        "echo hello\necho boom >&2\nsleep 0.5\nexit 2",
    )
    .await;
    let adapter = d.adapter(json!({
        "on_configuration_done": [
            {"event": "output", "body": {"category": "stdout", "output": "adapter chatter\n"}, "delay": 0.1},
            {"event": "terminated", "delay": 0.2}
        ]
    }));
    d.inner.script_resolve(d.inner.main_plan(None));

    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    d.inner
        .until("the session to end", |s| s.run_finished())
        .await;

    // ovim owns the JVM; the adapter attaches to the port it announced.
    assert!(
        d.requests("launch").is_empty(),
        "no DAP launch: ovim spawns the JVM"
    );
    let attach = d.requests("attach");
    assert_eq!(attach.len(), 1);
    assert_eq!(attach[0]["arguments"]["port"], FAKE_JDWP_PORT);
    assert_eq!(
        attach[0]["arguments"]["projectRoot"],
        d.inner.root.to_str().unwrap()
    );
    assert!(!d.requests("configurationDone").is_empty());

    let run = d.inner.test.editor.run_console().viewed().unwrap();
    let out: Vec<(LineKind, &str)> = run
        .lines
        .iter()
        .map(|l| (l.kind, l.text.as_str()))
        .collect();
    assert!(out.contains(&(LineKind::Stdout, "hello")), "{out:?}");
    assert!(out.contains(&(LineKind::Stderr, "boom")), "{out:?}");
    assert_eq!(run.exit_code, Some(2), "the JVM's own exit code is shown");
    assert_eq!(d.inner.outcome(), RunOutcome::Failed);

    // Output survives the session; the adapter and UI state are gone.
    assert!(!d.inner.test.editor.is_debug_active());
    assert!(!d.inner.test.editor.debug_state().output_lines.is_empty());
    assert!(d.inner.test.editor.debug_state().execution_line.is_none());
    let pid = d.pid().unwrap();
    wait_gone(pid).await;
    assert!(
        !process_alive(pid),
        "the adapter process must not linger after terminate"
    );
    d.inner.stop_lsp().await;
}

/// OV-00444: a debugged program reads the same stdin as a run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_debugged_program_takes_run_input_and_eof() {
    let mut d = DebugSession::new(&resolve_commands()).await;
    let adapter = d.adapter(json!({}));
    d.inner.script_resolve(d.inner.main_plan(None));
    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    let dap_dir = d.dap_dir.clone();
    d.inner
        .until("configurationDone", |_| {
            !dap_requests(&dap_dir, "configurationDone").is_empty()
        })
        .await;
    d.inner.test.command("RunInput hi there");
    d.inner
        .until("the echo", |s| s.console_text().contains("in:hi there"))
        .await;
    assert!(d.inner.console_text().contains("» hi there"));
    d.inner.test.command("RunEof");
    d.inner
        .until("the program to end", |s| {
            s.console_text().contains("Process finished")
        })
        .await;
    d.inner.test.keys(" ds");
    d.inner.until("stopped", |s| s.run_finished()).await;
    d.inner.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f5_debug_start_and_space_dc_all_resolve_the_cursor_the_same_way() {
    let mut d = DebugSession::new(&resolve_commands()).await;
    d.inner.script_resolve(Value::Null);
    for keys in ["F5", " dc", " rd"] {
        if keys == "F5" {
            d.inner.test.press_key(ovim_core::KeyCode::F(5));
        } else {
            d.inner.test.keys(keys);
        }
        d.inner.until("give up", |s| s.run_finished()).await;
        assert!(d
            .inner
            .test
            .editor
            .status_message()
            .contains("Nothing to debug here"));
        d.inner.test.editor.clear_run_console();
    }
    d.inner.test.command("debug start");
    d.inner.until("give up", |s| s.run_finished()).await;
    let commands = d.inner.lsp_events("workspace/executeCommand");
    let resolves = commands
        .iter()
        .filter(|c| c["params"]["command"] == "hyperion.resolveLaunch")
        .count();
    assert_eq!(
        resolves, 4,
        "each entry point asks resolveLaunch once: {commands:?}"
    );
    assert!(
        !d.dap_dir.join("events.jsonl").exists(),
        "no adapter is started without a target"
    );
    d.inner.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_launch_resets_everything_and_reports_the_reason() {
    let mut d = DebugSession::new(&resolve_commands()).await;
    let adapter =
        d.adapter(json!({"launch_error": "Could not find or load main class com.example.Main"}));
    d.inner.script_resolve(d.inner.main_plan(None));

    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    d.inner
        .until("the launch to fail", |s| s.run_finished())
        .await;

    assert!(
        matches!(d.inner.outcome(), RunOutcome::Error(m) if m.contains("Could not find or load main class"))
    );
    assert!(d
        .inner
        .test
        .editor
        .status_message()
        .contains("Could not find or load main class"));
    assert!(
        !d.inner.test.editor.is_debug_active(),
        "a failed launch must not leave the session active"
    );
    assert!(
        d.requests("configurationDone").is_empty(),
        "no configurationDone after a failed launch"
    );
    let pid = d.pid().unwrap();
    wait_gone(pid).await;
    assert!(
        !process_alive(pid),
        "the adapter is killed after a failed launch"
    );
    assert!(!d.inner.test.editor.is_launch_active());
    d.inner.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_ends_a_live_debug_session_and_kills_an_adapter_that_ignores_disconnect() {
    let mut d = DebugSession::new(&resolve_commands()).await;
    let adapter = d.adapter(json!({"linger_after_disconnect": true}));
    d.inner.script_resolve(d.inner.main_plan(None));

    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    let dap_dir = d.dap_dir.clone();
    d.inner
        .until("configurationDone", |_| {
            !dap_requests(&dap_dir, "configurationDone").is_empty()
        })
        .await;
    d.inner
        .until("debugging phase", |s| {
            matches!(
                s.test.editor.run_console().viewed().map(|r| &r.status),
                Some(RunStatus::Active(ovim_core::launch::RunPhase::Debugging))
            ) && s.test.editor.is_debug_active()
        })
        .await;

    d.inner.test.keys(" ds");
    d.inner.until("stopped", |s| s.run_finished()).await;
    assert_eq!(d.inner.outcome(), RunOutcome::Stopped);
    assert!(!d.inner.test.editor.is_debug_active());
    let pid = d.pid().unwrap();
    wait_gone(pid).await;
    assert!(
        !process_alive(pid),
        "adapter must be killed on Stop even if it lingers"
    );
    assert_eq!(
        d.requests("disconnect")[0]["arguments"]["terminateDebuggee"],
        true,
        "ovim started the JVM, so ending the session ends it"
    );
    d.inner.stop_lsp().await;
}

/// Attaching to a JVM that is already running does not make it ours: Stop
/// detaches and leaves the process alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_an_attach_session_leaves_the_debuggee_running() {
    let mut d = DebugSession::new(&["something.else"]).await;
    std::fs::create_dir_all(d.inner.root.join(".ovim")).unwrap();
    std::fs::write(
        d.inner.root.join(".ovim/debug.toml"),
        "[[config]]\nname = \"Remote\"\ntype = \"attach\"\nport = 5005\n",
    )
    .unwrap();
    let adapter = d.adapter(json!({}));
    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    let dap_dir = d.dap_dir.clone();
    d.inner
        .until("configurationDone", |_| {
            !dap_requests(&dap_dir, "configurationDone").is_empty()
        })
        .await;
    assert_eq!(d.requests("attach")[0]["arguments"]["port"], 5005);

    d.inner.test.keys(" ds");
    d.inner.until("stopped", |s| s.run_finished()).await;

    assert_eq!(
        d.requests("disconnect")[0]["arguments"]["terminateDebuggee"],
        false,
        "a stopped attach session must not kill the user's process"
    );
    d.inner.stop_lsp().await;
}

/// Stop between "the debugger is queued to start" and "it started" used to
/// drop the queued start without anybody noticing: the job stayed on
/// "starting debugger" forever and every later run only said "Stopping the
/// current run...".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_before_the_debugger_has_started_ends_the_run_and_frees_the_launcher() {
    let mut d = DebugSession::new(&["something.else"]).await;
    std::fs::create_dir_all(d.inner.root.join(".ovim")).unwrap();
    std::fs::write(
        d.inner.root.join(".ovim/debug.toml"),
        "[[config]]\nname = \"Remote\"\ntype = \"attach\"\nport = 5005\n",
    )
    .unwrap();
    let adapter = d.adapter(json!({}));
    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter.clone()));
    d.inner
        .until("the debugger to be queued", |s| {
            matches!(
                s.test.editor.dap_manager().pending_action,
                Some(ovim_core::dap::PendingDebugAction::Start { .. })
            )
        })
        .await;

    d.inner.test.keys(" ds");
    d.inner.until("stopped", |s| s.run_finished()).await;
    assert_eq!(d.inner.outcome(), RunOutcome::Stopped);
    assert!(!d.inner.test.editor.is_launch_active());
    assert!(
        d.requests("initialize").is_empty(),
        "the cancelled debugger never started"
    );

    // The launcher is free again.
    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    let dap_dir = d.dap_dir.clone();
    d.inner
        .until("a new debug session", |_| {
            !dap_requests(&dap_dir, "configurationDone").is_empty()
        })
        .await;
    d.inner.test.keys(" ds");
    d.inner.until("stopped again", |s| s.run_finished()).await;
    d.inner.stop_lsp().await;
}

// ---------------------------------------------------------------------------
// Test debugging through the build tool
// ---------------------------------------------------------------------------

fn test_plan(s: &Session, debug_script: &Path) -> Value {
    json!({
        "name": "FooTest", "kind": "test", "language": "java",
        "projectRoot": s.root, "moduleDir": s.root, "buildTool": "gradle",
        "build": null,
        "launch": {"mainClass": "unused", "projectRoot": s.root},
        "test": {
            "className": "com.example.FooTest", "methodName": null,
            "gradleTask": ":test",
            "argv": [debug_script, "plain"],
            "debugArgv": [debug_script, "debug"],
            "cwd": s.root, "reportsDir": s.root.join("reports")
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_debug_waits_for_the_listening_line_on_stdout_or_stderr_and_attaches_to_that_port() {
    for stream in ["stdout", "stderr"] {
        let mut d = DebugSession::new(&resolve_commands()).await;
        let adapter = d.adapter(json!({}));
        let redirect = if stream == "stderr" { " >&2" } else { "" };
        let script = d.inner.write_script(
            "gradlew-fake.sh",
            &format!(
                "echo 'Starting a Gradle Daemon'\nsleep 0.5\necho 'Listening for transport dt_socket at address: 41977'{redirect}\nsleep 300"
            ),
        );
        d.inner.script_resolve(test_plan(&d.inner, &script));

        d.inner
            .test
            .editor
            .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
        let dap_dir = d.dap_dir.clone();
        d.inner
            .until("attach", |_| !dap_requests(&dap_dir, "attach").is_empty())
            .await;
        let attach = &d.requests("attach")[0]["arguments"];
        assert_eq!(
            attach["port"], 41977,
            "the port is parsed from the line ({stream}), not hardcoded"
        );
        assert_eq!(attach["host"], "127.0.0.1");
        d.inner
            .until("configurationDone", |_| {
                !dap_requests(&dap_dir, "configurationDone").is_empty()
            })
            .await;
        assert!(d.inner.console_text().contains("Starting a Gradle Daemon"));

        // Stop kills the debug-jvm child too.
        d.inner.test.keys(" rs");
        d.inner.until("stopped", |s| s.run_finished()).await;
        assert!(!d.inner.test.editor.is_debug_active());
        d.inner.stop_lsp().await;
    }
}

/// OV-00445: surefire swallows the JVM's `Listening for transport` line, so
/// ovim gives the JVM its own address and watches that port instead of
/// waiting for text that never comes (it used to hang for the whole timeout).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maven_test_debug_attaches_when_the_pinned_port_listens_without_any_output() {
    let mut d = DebugSession::new(&resolve_commands()).await;
    let adapter = d.adapter(json!({}));
    // Plays mvn: prints nothing about JDWP, but the "forked JVM" listens on the
    // address handed to -Dmaven.surefire.debug.
    let script = d.inner.write_script(
        "mvn-fake.sh",
        "echo '[INFO] T E S T S'\nport=$(echo \"$2\" | sed 's/.*address=127.0.0.1://')\necho \"$2\" > args.txt\nexec python3 -c \"import socket,time; s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(('127.0.0.1', $port)); s.listen(1); time.sleep(300)\"",
    );
    let mut plan = test_plan(&d.inner, &script);
    plan["test"]["debugArgv"] = json!([script, "-Dtest=FooTest", "-Dmaven.surefire.debug", "test"]);
    d.inner.script_resolve(plan);
    d.inner
        .test
        .editor
        .set_debug_port_timeout(Duration::from_secs(20));
    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    let dap_dir = d.dap_dir.clone();
    d.inner
        .until("attach", |_| !dap_requests(&dap_dir, "attach").is_empty())
        .await;
    let port = d.requests("attach")[0]["arguments"]["port"]
        .as_u64()
        .unwrap();
    assert_ne!(port, 5005, "not the hardcoded surefire default");
    let args = std::fs::read_to_string(d.inner.root.join("args.txt")).unwrap();
    assert!(
        args.contains(&format!("address=127.0.0.1:{port}")),
        "{args}"
    );

    // Stop takes the whole process tree down, including the listener.
    d.inner.test.keys(" rs");
    d.inner.until("stopped", |s| s.run_finished()).await;
    for _ in 0..100 {
        if !ovim_core::launch::process::port_is_listening(port as u16) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!ovim_core::launch::process::port_is_listening(port as u16));
    d.inner.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_debug_that_never_listens_times_out_showing_the_output_and_kills_the_child() {
    let mut d = DebugSession::new(&resolve_commands()).await;
    let adapter = d.adapter(json!({}));
    let script = d.inner.write_script(
        "gradlew-fake.sh",
        "echo $$ > pid.txt\necho 'Downloading gradle-9.zip'\nsleep 300",
    );
    d.inner.script_resolve(test_plan(&d.inner, &script));
    d.inner
        .test
        .editor
        .set_debug_port_timeout(Duration::from_millis(800));

    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    d.inner.until("the timeout", |s| s.run_finished()).await;
    let status = d.inner.test.editor.status_message().to_string();
    assert!(status.contains("Timed out"), "{status}");
    assert!(
        status.contains("Downloading gradle-9.zip"),
        "the actual output is shown: {status}"
    );
    assert!(d.requests("attach").is_empty());
    let pid: i64 = std::fs::read_to_string(d.inner.root.join("pid.txt"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    wait_gone(pid).await;
    assert!(
        !process_alive(pid),
        "the suspended JVM launcher must be killed on failure"
    );
    d.inner.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_run_reads_junit_reports_into_the_console_and_quickfix() {
    let mut s = Session::new(&resolve_commands()).await;
    std::fs::create_dir_all(s.root.join("reports")).unwrap();
    let source = s.root.join("src/test/java/com/example/FooTest.java");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, "class FooTest {}\n").unwrap();
    let script = s.write_script(
        "gradlew-fake.sh",
        &format!(
            "cat > reports/TEST-com.example.FooTest.xml <<'EOF'\n\
             <testsuite name=\"com.example.FooTest\">\n\
             <testcase name=\"ok()\" classname=\"com.example.FooTest\" time=\"0.1\"/>\n\
             <testcase name=\"bad()\" classname=\"com.example.FooTest\" time=\"0.2\">\n\
             <failure message=\"expected 1\" type=\"AssertionError\">AssertionError\n\
             \tat com.example.FooTest.bad(FooTest.java:1)\n\
             </failure></testcase></testsuite>\n\
             EOF\n\
             echo '{}'\n\
             exit 1",
            "tests ran"
        ),
    );
    s.script_resolve(test_plan(&s, &script));
    s.test.keys(" rr");
    s.until("the tests to finish", |s| s.run_finished()).await;
    assert!(
        s.console_text().contains("Tests: 1 passed, 1 failed"),
        "{}",
        s.console_text()
    );
    assert!(s
        .console_text()
        .contains("FAILED com.example.FooTest.bad(): AssertionError: expected 1"));
    let entry = &s.test.editor.quickfix_list().entries()[0];
    assert_eq!(entry.filename.as_deref(), Some(source.as_path()));
    assert_eq!(entry.lnum, 1);
    s.stop_lsp().await;
}

// ---------------------------------------------------------------------------
// Console navigation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn console_focus_scrolls_and_enter_jumps_to_a_stack_frame_in_the_project() {
    let _jdk = JDK_LOCK.lock().await;
    let mut s = Session::new(&resolve_commands()).await;
    let source = s.root.join("src/main/java/com/example/Main.java");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(
        &source,
        "package com.example;\nclass Main {\n  void boom() {}\n}\n",
    )
    .unwrap();
    s.fake_java(
        "echo 'Exception in thread \"main\" java.lang.IllegalStateException: no' >&2\n\
         echo '\tat com.example.Main.boom(Main.java:3)' >&2\n\
         echo '\tat java.base/java.lang.Thread.run(Thread.java:1583)' >&2\n\
         exit 1",
    );
    s.script_resolve(s.main_plan(None));
    s.test.keys(" rr");
    s.until("finish", |s| s.run_finished()).await;

    s.test.keys(" rf");
    assert_eq!(s.test.editor.mode(), Mode::RunConsole);
    let frame_line = s
        .test
        .editor
        .run_console()
        .viewed()
        .unwrap()
        .lines
        .iter()
        .position(|l| l.text.contains("Main.boom"))
        .unwrap();
    s.test.editor.run_console_mut().set_cursor(frame_line);
    s.test.press_enter();
    assert_eq!(s.test.editor.mode(), Mode::Normal);
    assert_eq!(
        s.test.editor.buffer().file_path().map(PathBuf::from),
        Some(source.canonicalize().unwrap())
    );
    assert_eq!(s.test.editor.buffer().cursor().line(), 2);

    // A JDK frame has no project source: say so instead of failing silently.
    s.test.keys(" rf");
    let jdk_line = s
        .test
        .editor
        .run_console()
        .viewed()
        .unwrap()
        .lines
        .iter()
        .position(|l| l.text.contains("Thread.run"))
        .unwrap();
    s.test.editor.run_console_mut().set_cursor(jdk_line);
    s.test.press_enter();
    // The language server is asked (workspace/symbol) before giving up.
    s.until("the lookup to give up", |s| {
        s.test.editor.status_message().contains("Source not found")
    })
    .await;

    // `x` clears finished runs, `q` leaves focus.
    s.test.keys("x");
    assert!(s.test.editor.run_console().runs.is_empty());
    s.test.keys("q");
    assert_eq!(s.test.editor.mode(), Mode::Normal);
    s.stop_lsp().await;
}

// ---------------------------------------------------------------------------
// Code lens
// ---------------------------------------------------------------------------

fn run_lens(line: u32) -> Value {
    json!({
        "range": {"start": {"line": line, "character": 4}, "end": {"line": line, "character": 8}},
        "command": {"title": "▶ Run", "command": "hyperion.run", "arguments": [{"className": "Main"}]}
    })
}

fn lens_events(s: &Session) -> usize {
    s.lsp_events("textDocument/codeLens").len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn code_lenses_are_shown_refreshed_after_edits_and_on_server_request() {
    let mut s = Session::with_capabilities(
        &resolve_commands(),
        json!({"codeLensProvider": {"resolveProvider": false}}),
    )
    .await;
    std::fs::write(
        s.root.join("response-textDocument_codeLens.json"),
        json!({"result": [run_lens(0)]}).to_string(),
    )
    .unwrap();

    s.until("the lens to appear", |s| {
        !s.test.editor.code_lenses().is_empty()
    })
    .await;
    assert_eq!(lens_events(&s), 1);
    let eol = s.test.editor.decorations.eol_for_line(0);
    assert_eq!(eol.len(), 1);
    assert_eq!(eol[0].text, "  ▶ Run │ ▶ Debug");

    // Idle: no request storm.
    for _ in 0..30 {
        s.tick().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        lens_events(&s),
        1,
        "an unchanged buffer is not re-requested"
    );

    // An edit re-requests once it settles, and the new answer replaces the old.
    std::fs::write(
        s.root.join("response-textDocument_codeLens.json"),
        json!({"result": [run_lens(1)]}).to_string(),
    )
    .unwrap();
    s.test.keys("O// new first line<Esc>");
    s.until("the lens to move", |s| {
        s.test
            .editor
            .code_lenses()
            .first()
            .is_some_and(|l| l.line == 1)
    })
    .await;
    assert_eq!(lens_events(&s), 2);
    assert!(s.test.editor.decorations.eol_for_line(0).is_empty());
    assert_eq!(s.test.editor.decorations.eol_for_line(1).len(), 1);

    // workspace/codeLens/refresh makes the editor ask again without an edit.
    std::fs::write(
        s.root.join("push-after-textDocument_codeLens.json"),
        json!([{"id": 777, "method": "workspace/codeLens/refresh"}]).to_string(),
    )
    .unwrap();
    // The push fires after the *next* codeLens answer; provoke one with an edit.
    s.test.keys("A x<Esc>");
    s.until("the third request", |s| lens_events(s) >= 3).await;
    s.until("the refresh-triggered request", |s| lens_events(s) >= 4)
        .await;
    let refresh_reply = s
        .lsp_events_raw()
        .into_iter()
        .any(|e| e["id"] == 777 && e.get("method").is_none());
    assert!(refresh_reply, "the refresh request must be answered");
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_a_hyperion_run_lens_goes_through_resolve_launch_at_the_lens_not_the_servers_vm() {
    let _jdk = JDK_LOCK.lock().await;
    let mut s = Session::with_capabilities(
        &resolve_commands(),
        json!({"codeLensProvider": {"resolveProvider": false}}),
    )
    .await;
    s.test
        .set_buffer_content("class Main {\n  void main() {}\n}\n");
    std::fs::write(
        s.root.join("response-textDocument_codeLens.json"),
        json!({"result": [run_lens(1)]}).to_string(),
    )
    .unwrap();
    s.fake_java("echo real-jvm");
    s.script_resolve(s.main_plan(None));
    s.until("the lens", |s| !s.test.editor.code_lenses().is_empty())
        .await;

    // Not on the lens line: nothing to run.
    s.test.keys("gg");
    s.test.keys(" cl");
    assert_eq!(s.test.editor.status_message(), "No code lens on this line");

    s.test.keys("j");
    s.test.keys(" cl");
    s.until("the run", |s| s.run_finished()).await;
    let commands = s.lsp_events("workspace/executeCommand");
    assert!(
        commands
            .iter()
            .all(|c| c["params"]["command"] != "hyperion.run"),
        "the lens must not execute on the server: {commands:?}"
    );
    let resolve = commands
        .iter()
        .find(|c| c["params"]["command"] == "hyperion.resolveLaunch")
        .expect("resolveLaunch");
    let position = &resolve["params"]["arguments"][0]["position"];
    assert_eq!(
        (position["line"].as_u64(), position["character"].as_u64()),
        (Some(1), Some(4)),
        "resolved at the lens position"
    );
    assert!(s.console_text().contains("real-jvm"));
    s.stop_lsp().await;
}

// ---------------------------------------------------------------------------
// Java tests through the normal test-runner keys (<Space>tn, :TestFile, ...)
// ---------------------------------------------------------------------------

const CALC_TEST: &str = "package com.example.app;\n\
\n\
import org.junit.jupiter.api.Test;\n\
\n\
class CalcTest {\n\
    @Test\n\
    void adds() {\n\
        assertEquals(2, 1 + 1);\n\
    }\n\
\n\
    @Test\n\
    void subtracts() {\n\
        assertEquals(1, 2 - 1);\n\
    }\n\
}\n";

impl Session {
    /// A Gradle project whose `gradlew` is a script: records its arguments and
    /// writes a JUnit report in which `subtracts` fails.
    fn fake_gradle_project(&self) -> PathBuf {
        let app = self.root.join("app");
        let src = app.join("src/test/java/com/example/app");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(self.root.join("gradle/wrapper")).unwrap();
        std::fs::write(self.root.join("gradle/wrapper/gradle-wrapper.jar"), "").unwrap();
        std::fs::write(self.root.join("settings.gradle.kts"), "include(\"app\")\n").unwrap();
        std::fs::write(app.join("build.gradle.kts"), "").unwrap();
        let file = src.join("CalcTest.java");
        std::fs::write(&file, CALC_TEST).unwrap();
        self.write_script(
            "gradlew",
            "echo \"$@\" >> gradle-args.txt\n\
             mkdir -p app/build/test-results/test\n\
             cat > app/build/test-results/test/TEST-com.example.app.CalcTest.xml <<'EOF'\n\
             <testsuite name=\"com.example.app.CalcTest\">\n\
             <testcase name=\"adds()\" classname=\"com.example.app.CalcTest\" time=\"0.01\"/>\n\
             <testcase name=\"subtracts()\" classname=\"com.example.app.CalcTest\" time=\"0.2\">\n\
             <failure message=\"expected: &lt;1&gt; but was: &lt;2&gt;\" type=\"AssertionFailedError\">AssertionFailedError\n\
             \tat org.junit.jupiter.api.Assertions.fail(Assertions.java:1)\n\
             \tat com.example.app.CalcTest.subtracts(CalcTest.java:13)\n\
             </failure></testcase></testsuite>\n\
             EOF\n\
             exit 1",
        );
        file
    }

    fn gradle_invocations(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("gradle-args.txt"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn space_t_n_runs_the_java_test_under_the_cursor_and_fills_the_test_panel() {
    use ovim_core::editor::TestRunStatus;
    let mut s = Session::new(&resolve_commands()).await;
    // The server has no answer for a .java file: the command is composed
    // from the file itself.
    let file = s.fake_gradle_project();
    s.test.editor.load_file(file.display().to_string()).unwrap();
    s.test.set_cursor(12, 8); // inside `subtracts`
    s.test.keys(" tn");
    s.until("the test run to finish", |s| s.run_finished())
        .await;

    assert_eq!(
        s.gradle_invocations(),
        vec![":app:cleanTest :app:test --tests com.example.app.CalcTest.subtracts --console=plain"]
    );
    let panel = s.test.editor.test_panel();
    let run = panel.latest().expect("the test panel records the run");
    assert_eq!(run.status, TestRunStatus::Failed);
    assert_eq!(run.scope_label, "nearest");
    assert_eq!(run.summary.as_deref(), Some("1 passed, 1 failed"));
    let text = run.lines.join("\n");
    assert!(text.contains("✓ CalcTest.adds"), "{text}");
    assert!(text.contains("✗ CalcTest.subtracts"), "{text}");
    assert!(text.contains("expected: <1> but was: <2>"), "{text}");
    assert!(text.contains("at com.example.app.CalcTest.subtracts(CalcTest.java:13)"));
    assert!(
        !text.contains("org.junit"),
        "framework frames are hidden: {text}"
    );
    assert_eq!(run.failures.len(), 1);
    let location = run.failures[0].location.as_ref().unwrap();
    assert!(location.path.ends_with("CalcTest.java"));
    assert_eq!(location.line, 13);
    // Failures reach the quickfix list too.
    let entry = &s.test.editor.quickfix_list().entries()[0];
    assert_eq!(entry.lnum, 13);
    assert!(
        entry.text.contains("CalcTest.subtracts()"),
        "{}",
        entry.text
    );

    // :TestLast replays the same request; :TestFile selects the class.
    s.test.command("TestFile");
    s.until("the file run to finish", |s| {
        s.run_finished() && s.gradle_invocations().len() == 2
    })
    .await;
    assert_eq!(
        s.gradle_invocations()[1],
        ":app:cleanTest :app:test --tests com.example.app.CalcTest --console=plain"
    );
    s.test.command("TestLast");
    s.until("the rerun to finish", |s| {
        s.run_finished() && s.gradle_invocations().len() == 3
    })
    .await;
    assert_eq!(s.gradle_invocations()[2], s.gradle_invocations()[1]);
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_java_file_without_a_build_file_says_why_no_test_can_run() {
    let mut s = Session::new(&resolve_commands()).await;
    let lone = s.root.join("Lone.java");
    std::fs::write(&lone, CALC_TEST).unwrap();
    s.test.editor.load_file(lone.display().to_string()).unwrap();
    s.test.keys(" tn");
    s.until("the failure message", |s| {
        s.test
            .editor
            .status_message()
            .contains("No Gradle or Maven project")
    })
    .await;
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lsp_exec_runs_a_server_command_with_json_arguments_and_reports_the_result() {
    let mut s = Session::new(&resolve_commands()).await;
    s.script_resolve(json!({"reloaded": true}));
    s.test
        .command(r#"LspExec hyperion.resolveLaunch {"flag": 1} "two""#);
    s.until("the server command result", |s| {
        s.test.editor.status_message().contains("reloaded")
    })
    .await;
    let calls = s.lsp_events("workspace/executeCommand");
    let params = &calls.last().expect("the server got the command")["params"];
    assert_eq!(params["command"], "hyperion.resolveLaunch");
    assert_eq!(params["arguments"], json!([{"flag": 1}, "two"]));

    // A command the server does not list is refused with an explanation.
    s.test.command("LspExec no.such.command");
    s.until("the refusal", |s| {
        s.test.editor.status_message().contains("does not provide")
    })
    .await;
    s.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_lsp_settings_reach_the_server_as_initialization_options_and_workspace_settings() {
    ovim_core::lsp::user_settings::configure(
        &["controlled".to_string()],
        ovim_core::lsp::user_settings::UserLspSettings {
            initialization_options: Some(json!({"hyperion": {"buildToolClasspath": true}})),
            settings: Some(json!({"hyperion": {"buildToolClasspath": true}})),
        },
    );
    let mut s = Session::new(&resolve_commands()).await;
    s.wait_for_event("workspace/didChangeConfiguration").await;
    let init = s.lsp_events("initialize");
    assert_eq!(
        init[0]["params"]["initializationOptions"]["hyperion"]["buildToolClasspath"],
        true
    );
    let config = s.lsp_events("workspace/didChangeConfiguration");
    assert_eq!(
        config[0]["params"]["settings"]["hyperion"]["buildToolClasspath"],
        true
    );
    s.stop_lsp().await;
}

// ---------------------------------------------------------------------------
// Debug panel: variables tree, watches, breakpoint list, exception filters
// ---------------------------------------------------------------------------

fn stopped_scenario(root: &Path) -> Value {
    json!({
        "exception_filters": [
            {"filter": "all", "label": "All exceptions", "default": false},
            {"filter": "uncaught", "label": "Uncaught exceptions", "default": true}
        ],
        "on_configuration_done": [
            {"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1, "allThreadsStopped": true}}
        ],
        "frames": [{
            "id": 1, "name": "main", "line": 1, "column": 1,
            "source": {"name": "Main.controlled", "path": root.join("Main.controlled")}
        }],
        "scopes": [{"name": "Locals", "variablesReference": 10}],
        "variables": {
            "10": [
                {"name": "user", "value": "User@1", "type": "User", "variablesReference": 11},
                {"name": "n", "value": "3", "type": "int", "variablesReference": 0}
            ],
            "11": [{"name": "name", "value": "\"Ann\"", "variablesReference": 0}]
        },
        "evaluate": {
            "n * 2": {"result": "6", "type": "int"},
            "Main": {"result": "Main@2", "type": "Main", "variablesReference": 11}
        }
    })
}

fn panel_labels(s: &Session) -> Vec<String> {
    s.test
        .editor
        .debug_panel_rows()
        .into_iter()
        .map(|r| r.label)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn debug_panel_expands_variables_and_manages_watches_breakpoints_and_exceptions() {
    let mut d = DebugSession::new(&resolve_commands()).await;
    let adapter = d.adapter(stopped_scenario(&d.inner.root));
    d.inner.script_resolve(d.inner.main_plan(None));
    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    d.inner
        .until("the stop to be loaded", |s| {
            s.test.editor.debug_state().variables.contains_key(&10)
        })
        .await;
    assert!(panel_labels(&d.inner).contains(&"user".to_string()));

    // Watches are evaluated at the stop, and again when added while stopped.
    d.inner.test.command("DebugWatch n * 2");
    d.inner
        .until("the watch value", |s| {
            s.test
                .editor
                .debug_state()
                .watches
                .first()
                .is_some_and(|w| w.result == Some(Ok("6".to_string())))
        })
        .await;
    d.inner.test.command("DebugWatch nosuch");
    d.inner
        .until("the failing watch", |s| {
            s.test
                .editor
                .debug_state()
                .watches
                .get(1)
                .is_some_and(|w| matches!(w.result, Some(Err(_))))
        })
        .await;

    // Focus the panel; the cursor starts on the selected frame. Move to
    // `user` and expand it with `l`.
    d.inner.test.keys(" df");
    assert_eq!(d.inner.test.editor.mode(), Mode::DebugPanel);
    d.inner.test.keys("j");
    let row = d.inner.test.editor.debug_panel_rows()
        [d.inner.test.editor.debug_state().panel.cursor]
        .clone();
    assert_eq!(row.label, "user");
    d.inner.test.keys("l");
    d.inner
        .until("the children to load", |s| {
            panel_labels(s).contains(&"name".to_string())
        })
        .await;
    assert_eq!(
        d.requests("variables").len(),
        2,
        "children fetched once, on demand"
    );
    d.inner.test.keys("j");
    d.inner.test.keys("h"); // on a child: go to the parent
    assert_eq!(
        d.inner.test.editor.debug_panel_rows()[d.inner.test.editor.debug_state().panel.cursor]
            .label,
        "user"
    );
    d.inner.test.keys("h"); // on the expanded parent: collapse
    assert!(!panel_labels(&d.inner).contains(&"name".to_string()));

    // Breakpoint list: toggle one in the buffer, disable it in the panel, delete it.
    d.inner.test.keys("q");
    d.inner.test.keys(" db");
    d.inner
        .until("setBreakpoints with the new line", |s| {
            s.test
                .editor
                .debug_state()
                .breakpoints
                .values()
                .any(|v| v.iter().any(|b| b.verified))
        })
        .await;
    d.inner.test.keys(" df");
    d.inner.test.keys("G"); // last row: the second exception filter
    d.inner.test.keys("kk"); // over both filters, onto the breakpoint
    let bp = d.inner.test.editor.debug_panel_rows()[d.inner.test.editor.debug_state().panel.cursor]
        .clone();
    assert_eq!(bp.label, "Main.controlled:1", "{bp:?}");
    let sent = d.requests("setBreakpoints").len();
    d.inner.test.keys("e");
    d.inner
        .until("the disabled breakpoint to be synced", |_| {
            dap_requests(&d.dap_dir, "setBreakpoints").len() > sent
        })
        .await;
    let last = d.requests("setBreakpoints").last().unwrap().clone();
    assert_eq!(
        last["arguments"]["breakpoints"],
        json!([]),
        "disabled breakpoints are not sent"
    );
    assert!(
        d.inner.test.editor.debug_state().all_breakpoints().len() == 1,
        "but stay listed"
    );
    d.inner.test.keys("d");
    assert!(d
        .inner
        .test
        .editor
        .debug_state()
        .all_breakpoints()
        .is_empty());

    // Exception filters: defaults come from the adapter; toggling re-sends them.
    let filters = d.inner.test.editor.debug_state().exception_filters.clone();
    assert_eq!(filters.len(), 2);
    assert!(!filters[0].enabled && filters[1].enabled);
    assert_eq!(
        d.requests("setExceptionBreakpoints")[0]["arguments"]["filters"],
        json!(["uncaught"]),
        "the adapter's defaults are sent when the session is configured"
    );
    d.inner.test.keys("q"); // back to the buffer
    d.inner.test.command("DebugException all");
    d.inner
        .until("setExceptionBreakpoints with both filters", |_| {
            dap_requests(&d.dap_dir, "setExceptionBreakpoints")
                .last()
                .is_some_and(|r| r["arguments"]["filters"] == json!(["all", "uncaught"]))
        })
        .await;

    // K evaluates the expression under the cursor into the hover popup.
    d.inner.test.set_cursor(0, 7); // on `Main`
    d.inner.test.press('K');
    d.inner
        .until("the hover", |s| s.test.editor.hover_info().is_some())
        .await;
    let hover = d.inner.test.editor.hover_info().unwrap().to_string();
    assert!(
        hover.contains("Main = Main@2") && hover.contains("name = \"Ann\""),
        "{hover}"
    );
    d.inner.test.press_esc();

    d.inner.test.keys(" ds");
    d.inner.stop_lsp().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_input_feeds_the_programs_stdin_and_eof_ends_it() {
    let _jdk = JDK_LOCK.lock().await;
    let mut s = Session::new(&resolve_commands()).await;
    s.fake_java("echo 'name?'\nread name\necho \"hello:$name\"\nwhile read more; do echo \"more:$more\"; done\necho done");
    s.script_resolve(s.main_plan(None));
    s.test.keys(" rr");
    s.until("the prompt", |s| s.console_text().contains("name?"))
        .await;
    s.test.command("RunInput Ann");
    s.until("the greeting", |s| s.console_text().contains("hello:Ann"))
        .await;
    assert!(s.console_text().contains("» Ann"), "the input is echoed");
    s.test.command("RunInput second line");
    s.until("the echo", |s| {
        s.console_text().contains("more:second line")
    })
    .await;
    s.test.command("RunEof");
    s.until("the program to finish", |s| s.run_finished()).await;
    assert!(s.console_text().contains("done"));
    assert_eq!(s.outcome(), RunOutcome::Succeeded);

    // After the run there is nothing to type into.
    s.test.command("RunInput late");
    assert!(s
        .test
        .editor
        .status_message()
        .contains("No program is running"));
    s.stop_lsp().await;
}

/// Starts a stopped debug session against the scripted adapter and waits for
/// the frame/variables to load. `scenario` gets the project root.
async fn stopped_session(scenario: impl FnOnce(&Path) -> Value) -> DebugSession {
    let mut d = DebugSession::new(&resolve_commands()).await;
    let adapter = d.adapter(scenario(&d.inner.root));
    d.inner.script_resolve(d.inner.main_plan(None));
    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    d.inner
        .until("the stop to be loaded", |s| {
            s.test.editor.debug_state().variables.contains_key(&10)
        })
        .await;
    d
}

/// OV-00441: `:eval` is explicit evaluation ("repl", which may run method
/// calls); only `K` hover is a side-effect-free "hover" evaluation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eval_command_uses_the_repl_context_and_hover_uses_hover() {
    let mut d = stopped_session(stopped_scenario).await;
    d.inner.test.command("eval n * 2");
    d.inner
        .until("the eval", |s| {
            s.test.editor.status_message().contains("= 6")
        })
        .await;
    let contexts: Vec<Value> = d
        .requests("evaluate")
        .iter()
        .map(|r| r["arguments"]["context"].clone())
        .collect();
    assert!(contexts.contains(&json!("repl")), "{contexts:?}");
    assert!(!contexts.contains(&json!("hover")), "{contexts:?}");
    d.inner.stop_lsp().await;
}

/// OV-00442: stopping in a file other than the open buffer opens that file
/// and the execution marker belongs to that buffer only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_in_another_file_opens_it_and_marks_only_that_buffer() {
    let mut d = stopped_session(|root| {
        std::fs::write(root.join("Other.controlled"), "a\nb\nc\n").unwrap();
        let mut scenario = stopped_scenario(root);
        scenario["frames"] = json!([{
            "id": 1, "name": "label", "line": 3, "column": 1,
            "source": {"name": "Other.controlled", "path": root.join("Other.controlled")}
        }]);
        scenario
    })
    .await;
    let other = d.inner.root.join("Other.controlled");
    assert_eq!(
        d.inner.test.editor.buffer().file_path(),
        Some(other.to_str().unwrap()),
        "the stop opens the file"
    );
    assert_eq!(d.inner.test.editor.buffer().cursor().line(), 2);
    assert_eq!(
        d.inner.test.editor.execution_line_in_current_buffer(),
        Some(3)
    );

    // Back in the original file the marker must not show up on line 3.
    let main = d.inner.root.join("Main.controlled");
    d.inner.test.command(&format!("e {}", main.display()));
    d.inner
        .until("Main to be shown", |s| {
            s.test
                .editor
                .buffer()
                .file_path()
                .is_some_and(|p| p.ends_with("Main.controlled"))
        })
        .await;
    assert_eq!(d.inner.test.editor.execution_line_in_current_buffer(), None);
    d.inner.stop_lsp().await;
}

/// OV-00443: an exception stop asks for `exceptionInfo` and shows type and
/// message in the panel, the status line and the console.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exception_stop_shows_the_exception_type_and_message() {
    let mut d = stopped_session(|root| {
        let mut scenario = stopped_scenario(root);
        scenario["on_configuration_done"] = json!([
            {"event": "stopped", "body": {"reason": "exception", "threadId": 1, "allThreadsStopped": true}}
        ]);
        scenario["exception_info"] = json!({
            "exceptionId": "java.lang.ArrayIndexOutOfBoundsException",
            "description": "java.lang.ArrayIndexOutOfBoundsException: Index 5 out of bounds for length 2",
            "breakMode": "unhandled",
            "details": {"message": "Index 5 out of bounds for length 2",
                        "typeName": "java.lang.ArrayIndexOutOfBoundsException"}
        });
        scenario
    })
    .await;
    d.inner
        .until("exception info", |s| {
            s.test.editor.debug_state().exception.is_some()
        })
        .await;
    assert!(!d.requests("exceptionInfo").is_empty());
    let want = "java.lang.ArrayIndexOutOfBoundsException: Index 5 out of bounds for length 2";
    assert!(
        panel_labels(&d.inner).iter().any(|l| l.contains(want)),
        "{:?}",
        panel_labels(&d.inner)
    );
    assert!(d.inner.test.editor.status_message().contains(want));
    d.inner
        .until("the console line", |s| s.console_text().contains(want))
        .await;
    d.inner.stop_lsp().await;
}

/// OV-00446: an adapter that dies mid-session is a failure with its exit
/// status and last words, not a successful "exit 0" ending; the JVM it was
/// driving is stopped too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crashing_adapter_is_reported_as_a_crash_and_takes_the_jvm_down() {
    let mut d = DebugSession::new(&resolve_commands()).await;
    let adapter = d.adapter(json!({
        "on_configuration_done": [
            {"event": "crash", "message": "thread 'main' panicked at jdwp.rs:1", "code": 101, "delay": 0.2}
        ]
    }));
    d.inner.script_resolve(d.inner.main_plan(None));
    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    d.inner
        .until("the session to end", |s| s.run_finished())
        .await;
    match d.inner.outcome() {
        RunOutcome::Error(message) => {
            assert!(message.contains("crashed"), "{message}");
            assert!(message.contains("101"), "{message}");
            assert!(message.contains("panicked at jdwp.rs:1"), "{message}");
        }
        other => panic!("expected an error outcome, got {other:?}"),
    }
    assert!(d.inner.test.editor.status_message().contains("crashed"));
    assert!(!d.inner.test.editor.is_debug_active());
    d.inner.stop_lsp().await;
}

/// OV-00447: the debug function keys work whichever panel has the keyboard
/// focus, and an exception filter toggled at a stop reaches the adapter.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn function_keys_work_from_the_debug_panel_and_the_run_console() {
    let mut d = stopped_session(stopped_scenario).await;
    d.inner.test.keys(" df");
    assert_eq!(d.inner.test.editor.mode(), Mode::DebugPanel);
    d.inner.test.press_key(ovim_core::KeyCode::F(10));
    d.inner
        .until("next", |s| {
            !dap_requests(&s.root.join("dap"), "next").is_empty()
        })
        .await;
    d.inner.test.press_key(ovim_core::KeyCode::F(11));
    d.inner
        .until("stepIn", |s| {
            !dap_requests(&s.root.join("dap"), "stepIn").is_empty()
        })
        .await;
    d.inner.test.keys("q");
    d.inner.test.keys(" rf");
    assert_eq!(d.inner.test.editor.mode(), Mode::RunConsole);
    d.inner.test.press_key(ovim_core::KeyCode::F(5));
    d.inner
        .until("continue", |s| {
            !dap_requests(&s.root.join("dap"), "continue").is_empty()
        })
        .await;
    d.inner.test.keys("q");
    d.inner.stop_lsp().await;
}

/// OV-00449: the panel lists the threads and Enter on one shows its stack.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_panel_lists_threads_and_switches_the_inspected_one() {
    let mut d = stopped_session(|root| {
        std::fs::write(root.join("Other.controlled"), "a\nb\nc\n").unwrap();
        let mut scenario = stopped_scenario(root);
        scenario["threads"] = json!([
            {"id": 1, "name": "main"},
            {"id": 2, "name": "worker-1"},
            {"id": 3, "name": "Reference Handler"}
        ]);
        scenario["frames_by_thread"] = json!({
            "2": [{"id": 7, "name": "runWorker", "line": 2, "column": 1,
                   "source": {"name": "Other.controlled", "path": root.join("Other.controlled")}}]
        });
        scenario
    })
    .await;
    d.inner
        .until("the thread list", |s| {
            panel_labels(s).contains(&"worker-1 (2)".to_string())
        })
        .await;
    let labels = panel_labels(&d.inner);
    assert!(labels.contains(&"main (1)".to_string()), "{labels:?}");
    assert!(
        !labels.iter().any(|l| l.contains("Reference Handler")),
        "JVM housekeeping threads are hidden: {labels:?}"
    );

    d.inner.test.keys(" df");
    for _ in 0..20 {
        let cursor = d.inner.test.editor.debug_state().panel.cursor;
        if d.inner.test.editor.debug_panel_rows()[cursor].label == "worker-1 (2)" {
            break;
        }
        d.inner.test.keys("j");
    }
    d.inner.test.keys("<CR>");
    d.inner
        .until("the other thread's stack", |s| {
            panel_labels(s).contains(&"runWorker Other.controlled:2".to_string())
        })
        .await;
    assert_eq!(d.inner.test.editor.debug_state().stopped_thread, Some(2));
    assert!(d
        .inner
        .test
        .editor
        .buffer()
        .file_path()
        .is_some_and(|p| p.ends_with("Other.controlled")));
    // Stepping now steps the inspected thread.
    d.inner.test.keys("n");
    d.inner
        .until("next on thread 2", |s| {
            dap_requests(&s.root.join("dap"), "next")
                .last()
                .is_some_and(|r| r["arguments"]["threadId"] == 2)
        })
        .await;
    d.inner.test.keys("q");
    d.inner.stop_lsp().await;
}

/// OV-00449: logpoints and hit counts are sent when the adapter supports
/// them; when it does not, the line is not sent bare (it would stop on every
/// hit) and the console says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logpoints_and_hit_counts_follow_the_adapters_capabilities() {
    for supported in [true, false] {
        let mut d = stopped_session(|root| {
            let mut scenario = stopped_scenario(root);
            scenario["capabilities"] = json!({
                "supportsLogPoints": supported,
                "supportsHitConditionalBreakpoints": supported
            });
            scenario
        })
        .await;
        let baseline = d.requests("setBreakpoints").len();
        d.inner.test.set_cursor(0, 0);
        d.inner.test.command("DebugLogpoint value is {n}");
        d.inner
            .until("the logpoint sync", |s| {
                dap_requests(&s.root.join("dap"), "setBreakpoints").len() > baseline
            })
            .await;
        let last = d.requests("setBreakpoints").last().unwrap().clone();
        let sent = &last["arguments"]["breakpoints"];
        if supported {
            assert_eq!(sent[0]["logMessage"], "value is {n}", "{last}");
            d.inner.test.command("DebugHitCount >3");
            d.inner
                .until("the hit count sync", |s| {
                    dap_requests(&s.root.join("dap"), "setBreakpoints")
                        .last()
                        .is_some_and(|r| r["arguments"]["breakpoints"][0]["hitCondition"] == ">3")
                })
                .await;
            let labels = d
                .inner
                .test
                .editor
                .debug_panel_rows()
                .into_iter()
                .filter_map(|r| r.value)
                .collect::<Vec<_>>();
            assert!(labels
                .iter()
                .any(|v| v.contains("hits >3") && v.contains("log")));
        } else {
            assert_eq!(sent, &json!([]), "not sent bare: {last}");
            d.inner
                .until("the explanation", |s| {
                    s.console_text().contains("does not support logpoints")
                })
                .await;
            assert_eq!(
                d.inner.test.editor.debug_state().all_breakpoints().len(),
                1,
                "the breakpoint stays listed"
            );
        }
        d.inner.stop_lsp().await;
    }
}

/// An adapter may put a breakpoint on another line (the next executable one)
/// or leave the line out of an unverified answer. Neither may cost the user
/// the breakpoint or what they attached to it: later syncs still send the
/// line they chose, with its condition.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_breakpoint_the_adapter_moves_keeps_its_condition_in_later_syncs() {
    let mut d = DebugSession::new(&resolve_commands()).await;
    let adapter = d.adapter(json!({
        "move_breakpoints": {"1": 3},
        "unverified_breakpoints": [2]
    }));
    d.inner.script_resolve(d.inner.main_plan(None));
    let file = d.inner.root.join("Main.controlled");
    d.inner.test.set_cursor(0, 0);
    d.inner.test.command("DebugCondition n > 1");
    d.inner.test.editor.toggle_breakpoint_at(&file, 2);
    d.inner
        .test
        .editor
        .launch_at_cursor_with(ovim_core::launch::LaunchMode::Debug, Some(adapter));
    let dap_dir = d.dap_dir.clone();
    d.inner
        .until("configurationDone", |_| {
            !dap_requests(&dap_dir, "configurationDone").is_empty()
        })
        .await;

    let state = d.inner.test.editor.debug_state();
    assert_eq!(
        state.breakpoint_lines(&file),
        vec![2, 3],
        "drawn where the adapter put them"
    );
    let by_line = |request: &Value| {
        let mut sent = request["arguments"]["breakpoints"]
            .as_array()
            .unwrap()
            .clone();
        sent.sort_by_key(|bp| bp["line"].as_u64());
        Value::Array(sent)
    };
    let sent = d.requests("setBreakpoints");
    assert_eq!(
        by_line(sent.last().unwrap()),
        json!([{"line": 1, "condition": "n > 1"}, {"line": 2}])
    );

    d.inner
        .test
        .editor
        .dap_manager_mut()
        .request_breakpoint_sync();
    let baseline = sent.len();
    d.inner
        .until("the second sync", |_| {
            dap_requests(&dap_dir, "setBreakpoints").len() > baseline
        })
        .await;
    let resent = d.requests("setBreakpoints").last().unwrap().clone();
    assert_eq!(
        by_line(&resent),
        json!([{"line": 1, "condition": "n > 1"}, {"line": 2}]),
        "the condition and the requested lines survive the adapter's answer"
    );
    let state = d.inner.test.editor.debug_state();
    assert_eq!(state.breakpoint_lines(&file), vec![2, 3]);
    assert!(state.is_conditional_breakpoint(&file, 3));

    d.inner.test.keys(" ds");
    d.inner.until("stopped", |s| s.run_finished()).await;
    d.inner.stop_lsp().await;
}

/// OV-00449: panels can be resized (`:PanelSize`, `+`/`-` in the console).
#[test]
fn panel_size_command_resizes_the_side_panels_and_the_console() {
    let mut t = EditorTest::new("x\n");
    t.command("PanelSize test +6");
    assert_eq!(t.editor.test_panel().width_delta, 6);
    t.command("PanelSize debug -4");
    assert_eq!(t.editor.debug_state().panel.width_delta, -4);
    t.command("PanelSize console +3");
    assert_eq!(t.editor.run_console().height_delta, 3);
    t.command("PanelSize test reset");
    assert_eq!(t.editor.test_panel().width_delta, 0);
    t.command("PanelSize nonsense +1");
    assert!(t.editor.status_message().contains("Unknown panel"));
}
