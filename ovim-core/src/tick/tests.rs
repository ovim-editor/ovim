use std::time::{Duration, Instant};

use super::{
    process_syntax_highlighting, process_yank_flash, tick_transient_ui, TerminalRequest,
    TickReport, TickState,
};
use crate::ai::chat_types::ChatOpts;
use crate::dap::PendingDebugAction;
use crate::editor::{Editor, InputHandler, PendingShellCommand, PendingTerminalSession};
use crate::unicode::CharCol;

async fn tick(editor: &mut Editor, state: &mut TickState) -> TickReport {
    editor.tick(state).await
}

#[test]
fn working_animation_tick_invalidates_the_render_without_input() {
    let mut editor = Editor::with_content("hello\n");
    editor.open_ai_chat(ChatOpts::default()).unwrap();
    editor.ai_state.chat.as_mut().unwrap().waiting = true;
    editor.render_cache.ai_chat_working_animation_tick = u128::MAX;
    editor.mark_clean();

    tick_transient_ui(&mut editor);

    assert!(editor.is_dirty());
}

#[test]
fn yank_flash_defers_slow_work_until_its_clear_frame_can_paint() {
    let mut editor = Editor::with_content("copy me\n");
    editor.set_yank_flash_lines(0, 0);

    assert!(process_yank_flash(&mut editor));
    assert!(editor.yank_flash().is_some());

    std::thread::sleep(Duration::from_millis(175));
    editor.mark_clean();

    assert!(process_yank_flash(&mut editor));
    assert!(editor.yank_flash().is_none());
    assert!(editor.is_dirty());
    assert!(
        !process_yank_flash(&mut editor),
        "slow work may start only after the clear frame has been deferred once"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn yaml_syntax_gets_a_paint_tick_before_lsp_initialization() {
    let mut editor = Editor::with_content("name: ovim\nenabled: true\n");
    editor.set_file_path("config.yaml".to_string());
    let mut state = TickState::new();

    assert!(process_syntax_highlighting(&mut editor, &mut state));

    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(5)).await;
        let defer_lsp = process_syntax_highlighting(&mut editor, &mut state);
        if editor.buffer().has_syntax_highlighting() {
            assert!(
                defer_lsp,
                "the completion tick must still defer LSP so the frontend can paint"
            );
            assert!(!editor.buffer().highlights_for_line(0).is_empty());
            assert!(
                !process_syntax_highlighting(&mut editor, &mut state),
                "LSP may initialize on the tick after syntax is ready"
            );
            return;
        }
    }

    panic!("YAML syntax highlighting did not finish");
}

/// Tick until background syntax lands, asserting LSP init waits for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tick_starts_lsp_only_after_syntax_has_had_a_paint_tick() {
    let mut editor = Editor::with_content("name: ovim\n");
    editor.set_file_path("config.yaml".to_string());
    editor.request_lsp_init();
    let mut state = TickState::new();

    let _ = tick(&mut editor, &mut state).await;
    assert!(editor.needs_lsp_init().is_some(), "syntax is still loading");

    for _ in 0..200 {
        if editor.buffer().has_syntax_highlighting() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
        let _ = tick(&mut editor, &mut state).await;
        if !editor.buffer().has_syntax_highlighting() {
            assert!(editor.needs_lsp_init().is_some());
        }
    }
    assert!(editor.buffer().has_syntax_highlighting());
    let _ = tick(&mut editor, &mut state).await;
    assert!(
        editor.needs_lsp_init().is_none(),
        "LSP init runs on the tick after syntax is ready"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tick_defers_lsp_init_while_a_yank_flash_is_visible() {
    let mut editor = Editor::with_content("copy me\n");
    editor.set_file_path("notes.no-such-language".to_string());
    editor.request_lsp_init();
    editor.set_yank_flash_lines(0, 0);
    let mut state = TickState::new();

    let _ = tick(&mut editor, &mut state).await;
    assert!(editor.needs_lsp_init().is_some());

    tokio::time::sleep(Duration::from_millis(175)).await;
    let _ = tick(&mut editor, &mut state).await;
    assert!(editor.yank_flash().is_none());
    assert!(
        editor.needs_lsp_init().is_some(),
        "the tick that clears the flash still defers LSP"
    );

    let _ = tick(&mut editor, &mut state).await;
    assert!(editor.needs_lsp_init().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tick_runs_the_queued_debug_action_and_marks_dirty() {
    let mut editor = Editor::with_content("x\n");
    editor
        .dap_manager_mut()
        .queue(PendingDebugAction::Evaluate {
            expression: "x".to_string(),
        });
    editor.mark_clean();
    let mut state = TickState::new();

    let _ = tick(&mut editor, &mut state).await;

    assert_eq!(editor.dap_manager().queued().count(), 0);
    assert!(
        editor.status_message().starts_with("Eval error:"),
        "{}",
        editor.status_message()
    );
    assert!(editor.is_dirty());
}

/// Edits that do not come through a key (the API, LSP, Lua) still move the
/// breakpoints of the file; the keys cover the rest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tick_carries_breakpoints_along_with_edits_made_outside_the_keys() {
    let mut editor = Editor::with_content("a\nb\nc\n");
    editor.set_file_path("/nonexistent/Main.java".to_string());
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(2, crate::unicode::GraphemeCol::ZERO);
    editor.toggle_breakpoint();
    let mut state = TickState::new();
    let _ = tick(&mut editor, &mut state).await;
    assert_eq!(editor.current_file_breakpoint_lines(), vec![3]);

    editor.buffer_mut().insert_text_at(0, CharCol(0), "new\n");
    let _ = tick(&mut editor, &mut state).await;

    assert_eq!(editor.current_file_breakpoint_lines(), vec![4]);
}

/// The GUI and TUI drained picker results after the tick while the headless
/// loop only received them in its own select arms; the tick now delivers them
/// for every frontend.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tick_delivers_picker_previews_and_marks_dirty() {
    let mut editor = Editor::with_content("x\n");
    let mut state = TickState::new();
    state
        .preview_tx
        .send((
            "/tmp/previewed.txt".to_string(),
            crate::editor::PreviewCache {
                content: "preview".to_string(),
                highlighted_lines: Default::default(),
                language: None,
            },
        ))
        .await
        .unwrap();
    editor.mark_clean();

    let _ = tick(&mut editor, &mut state).await;

    assert!(editor.get_preview_cache("/tmp/previewed.txt").is_some());
    assert!(editor.is_dirty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tick_reloads_a_clean_buffer_changed_on_disk_every_half_second() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("watched.txt");
    std::fs::write(&path, "before\n").unwrap();
    let mut editor = Editor::new();
    editor.load_file(&path).unwrap();
    let mut state = TickState::new();
    let start = Instant::now();
    std::fs::write(&path, "after\n").unwrap();
    editor
        .buffer_mut()
        .set_file_mtime(Some(std::time::SystemTime::UNIX_EPOCH));

    let _ = editor
        .tick_at(&mut state, start + Duration::from_millis(100))
        .await;
    assert_eq!(
        editor.buffer().rope().to_string(),
        "before\n",
        "the disk is not polled on every tick"
    );

    editor.mark_clean();
    let _ = editor
        .tick_at(&mut state, start + Duration::from_millis(600))
        .await;
    assert_eq!(editor.buffer().rope().to_string(), "after\n");
    assert_eq!(
        editor.status_message(),
        "File reloaded after external change"
    );
    assert!(editor.is_dirty());
}

/// The GUI used to rehighlight the whole buffer on every tick while the TUI
/// and headless loops waited 200ms after the last edit. All frontends now get
/// the debounce, with edits observed by the tick itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_rehighlight_waits_until_edits_settle() {
    let mut editor = Editor::with_content("fn main() {}\n");
    editor.set_file_path("main.rs".to_string());
    editor.buffer_mut().enable_syntax_highlighting();
    let mut state = TickState::new();
    let start = Instant::now();

    editor.buffer_mut().insert_text_at(0, CharCol(0), "// ");
    assert!(editor.buffer().needs_rehighlight());
    let _ = editor.tick_at(&mut state, start).await;
    assert!(editor.buffer().needs_rehighlight(), "edit just happened");

    editor.buffer_mut().insert_text_at(0, CharCol(0), "x");
    let _ = editor
        .tick_at(&mut state, start + Duration::from_millis(150))
        .await;
    let _ = editor
        .tick_at(&mut state, start + Duration::from_millis(300))
        .await;
    assert!(
        editor.buffer().needs_rehighlight(),
        "the second edit restarted the debounce"
    );

    editor.mark_clean();
    let _ = editor
        .tick_at(&mut state, start + Duration::from_millis(360))
        .await;
    assert!(!editor.buffer().needs_rehighlight());
    assert!(editor.is_dirty());
}

/// `:!cmd` and `:terminal` used to sit on the editor until each frontend
/// remembered to take them (the headless loop never did). The tick now hands
/// them over, shell commands first, one per tick.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tick_hands_terminal_requests_to_the_frontend() {
    let mut editor = Editor::with_content("x\n");
    InputHandler::execute_command_string(&mut editor, "terminal htop").unwrap();
    InputHandler::execute_command_string(&mut editor, "!echo hi").unwrap();
    let mut state = TickState::new();

    let first = tick(&mut editor, &mut state).await.terminal_request;
    let second = tick(&mut editor, &mut state).await.terminal_request;
    let third = tick(&mut editor, &mut state).await.terminal_request;

    assert_eq!(
        first,
        Some(TerminalRequest::Shell(PendingShellCommand {
            command: "echo hi".to_string()
        }))
    );
    assert_eq!(first.unwrap().describe(), ":!echo hi");
    assert_eq!(
        second,
        Some(TerminalRequest::Session(PendingTerminalSession {
            command: Some("htop".to_string())
        }))
    );
    assert_eq!(third, None);
}

/// Every public `poll_*` in ovim-core must be driven by the tick, or listed
/// here with the reason it is not. Non-public polls cannot escape: an
/// uncalled private fn is a dead-code error under clippy.
#[test]
fn every_public_poll_is_driven_by_the_tick() {
    const NOT_TICKED: &[(&str, &str)] = &[
        (
            "poll_pending_completion_response",
            "called by poll_pending_lsp_responses",
        ),
        (
            "poll_pending_diagnostic_refresh_response",
            "called by sync_lsp_and_refresh_diagnostics",
        ),
        (
            "poll_with_timeout",
            "LspSlot primitive used by the Editor polls, not a poll of its own",
        ),
    ];

    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let tick_dir = src.join("tick");
    let mut tick_source = String::new();
    for entry in std::fs::read_dir(&tick_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|name| name != "tests.rs") {
            tick_source.push_str(&std::fs::read_to_string(path).unwrap());
        }
    }

    let mut public_polls = Vec::new();
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                for line in std::fs::read_to_string(&path).unwrap().lines() {
                    let line = line.trim_start();
                    let Some(rest) = line
                        .strip_prefix("pub fn poll_")
                        .or_else(|| line.strip_prefix("pub async fn poll_"))
                    else {
                        continue;
                    };
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    public_polls.push(format!("poll_{name}"));
                }
            }
        }
    }

    assert!(public_polls.len() > 10, "found {public_polls:?}");
    let missing: Vec<_> = public_polls
        .iter()
        .filter(|name| !tick_source.contains(&format!(".{name}(")))
        .filter(|name| !NOT_TICKED.iter().any(|(allowed, _)| allowed == name))
        .collect();
    assert!(
        missing.is_empty(),
        "public polls nothing drives; call them from Editor::tick (ovim-core/src/tick) \
         or add them to NOT_TICKED with a reason: {missing:?}"
    );
    for (allowed, _) in NOT_TICKED {
        assert!(
            public_polls.iter().any(|name| name == allowed),
            "stale NOT_TICKED entry {allowed}"
        );
    }
}
