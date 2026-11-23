//! Exercise clipboard synchronization through real core command entrypoints.
use super::{
    clipboard::ClipboardBackend, input::InputHandler, Editor, RegisterManager, RegisterType,
};
use crate::buffer::Buffer;
use crate::{KeyCode, KeyEvent, Modifiers};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
struct ClipboardState {
    content: String,
    reads: usize,
    writes: usize,
}

#[derive(Debug, Default)]
struct FakeClipboard {
    state: Mutex<ClipboardState>,
    write_delay: Duration,
}

impl ClipboardBackend for FakeClipboard {
    fn read(&self) -> Option<String> {
        let mut state = self.state.lock().unwrap();
        state.reads += 1;
        Some(state.content.clone())
    }

    fn write(&self, text: &str) -> bool {
        if !self.write_delay.is_zero() {
            std::thread::sleep(self.write_delay);
        }
        let mut state = self.state.lock().unwrap();
        state.writes += 1;
        state.content = text.to_owned();
        true
    }
}

impl FakeClipboard {
    fn counts(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap();
        (state.reads, state.writes)
    }

    fn external_write(&self, text: &str) {
        self.state.lock().unwrap().content = text.to_owned();
    }

    fn content(&self) -> String {
        self.state.lock().unwrap().content.clone()
    }
}

fn setup(text: &str, clipboard: &str) -> (Editor, Arc<FakeClipboard>) {
    let backend = Arc::new(FakeClipboard::default());
    backend.external_write(clipboard);
    let mut editor = Editor::new();
    editor.registers = RegisterManager::with_clipboard_backend(backend.clone());
    *editor.buffer_mut() = Buffer::new_from_str(text);
    (editor, backend)
}

fn event(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), Modifiers::NONE)
}

fn keys(editor: &mut Editor, sequence: &str) {
    for ch in sequence.chars() {
        InputHandler::handle_key_event(editor, event(ch)).unwrap();
    }
}

fn record(editor: &mut Editor, register: char, sequence: &str) {
    assert!(editor.start_macro_recording(register));
    for ch in sequence.chars() {
        editor.record_macro_event(event(ch));
    }
    editor.stop_macro_recording();
}

#[test]
fn counted_macro_reads_its_own_deletes_without_external_reads() {
    let (mut editor, backend) = setup("abc\n", "external");
    record(&mut editor, 'a', "xP");
    keys(&mut editor, "50@a");
    assert_eq!(editor.buffer().rope().to_string(), "abc\n");
    assert_eq!(backend.counts(), (0, 1));
    assert_eq!(backend.content(), "a");
    assert_eq!(editor.registers.get(Some('-')), "a");
}

#[test]
fn paste_first_macro_reads_once_then_uses_local_writes() {
    let (mut editor, backend) = setup("A\n", "B");
    record(&mut editor, 'a', "px");
    keys(&mut editor, "20@a");
    assert_eq!(editor.buffer().rope().to_string(), "A\n");
    assert_eq!(backend.counts(), (1, 1));
    assert_eq!(backend.content(), "B");
}

#[test]
fn nested_macros_and_repeat_share_the_outer_scope() {
    let (mut editor, backend) = setup("abc\n", "external");
    record(&mut editor, 'a', "xP");
    record(&mut editor, 'b', "3@a");
    keys(&mut editor, "4@b");
    assert_eq!(backend.counts(), (0, 1));
    keys(&mut editor, "@@");
    assert_eq!(backend.counts(), (0, 2));
    assert_eq!(editor.buffer().rope().to_string(), "abc\n");
}

#[test]
fn failed_motion_publishes_completed_macro_edits() {
    let (mut editor, backend) = setup("abc\n", "external");
    record(&mut editor, 'a', "xj");
    keys(&mut editor, "20@a");
    assert_eq!(editor.buffer().rope().to_string(), "bc\n");
    assert_eq!(backend.content(), "a");
    assert_eq!(backend.counts(), (0, 1));
    keys(&mut editor, "x");
    assert_eq!(backend.content(), "b");
    assert_eq!(backend.counts(), (0, 2));
}

#[test]
fn macro_preserves_linewise_delete_and_paste() {
    let (mut editor, backend) = setup("first\nsecond\n", "external");
    record(&mut editor, 'a', "ddP");
    keys(&mut editor, "10@a");
    assert_eq!(editor.buffer().rope().to_string(), "first\nsecond\n");
    assert_eq!(
        editor.registers.get_default_with_type(),
        ("first\n", RegisterType::Line)
    );
    assert_eq!(backend.counts(), (0, 1));
}

#[test]
fn explicit_clipboard_register_joins_batch_without_duplicate_writes() {
    let (mut editor, backend) = setup("abc\n", "external");
    editor.options.clipboard.clear();
    record(&mut editor, 'a', "\"+x\"+P");
    keys(&mut editor, "10@a");
    assert_eq!(editor.buffer().rope().to_string(), "abc\n");
    assert_eq!(backend.counts(), (0, 1));
    keys(&mut editor, "\"+x");
    assert_eq!(backend.counts(), (0, 2));
}

#[test]
fn blackhole_and_named_register_macros_preserve_routing() {
    let (mut editor, backend) = setup("abcdef\n", "external");
    record(&mut editor, 'a', "\"_x");
    keys(&mut editor, "3@a");
    assert_eq!(editor.buffer().rope().to_string(), "def\n");
    assert_eq!(backend.counts(), (0, 0));
    editor.options.clipboard.clear();
    record(&mut editor, 'b', "\"cx\"cP");
    keys(&mut editor, "10@b");
    assert_eq!(editor.buffer().rope().to_string(), "def\n");
    assert_eq!(editor.registers.get(Some('c')), "d");
    assert_eq!(backend.content(), "external");
    assert_eq!(backend.counts(), (0, 0));
}

#[test]
fn independent_commands_observe_external_clipboard_changes() {
    let (mut editor, backend) = setup("A\n", "B");
    keys(&mut editor, "p");
    backend.external_write("C");
    keys(&mut editor, "p");
    assert_eq!(editor.buffer().rope().to_string(), "ABC\n");
    assert_eq!(backend.counts(), (2, 0));
}

#[test]
fn returned_error_flushes_completed_effects_and_closes_scope() {
    let (mut editor, backend) = setup("abc\n", "external");
    let result: Result<(), &str> = editor.with_execution_scope(|editor| {
        keys(editor, "x");
        assert_eq!(backend.counts(), (0, 0));
        Err("command failed")
    });
    assert_eq!(result, Err("command failed"));
    assert_eq!(backend.content(), "a");
    keys(&mut editor, "x");
    assert_eq!(backend.counts(), (0, 2));
    assert_eq!(backend.content(), "b");
}

#[test]
fn unwinding_flushes_completed_effects_and_closes_scope() {
    let (mut editor, backend) = setup("abc\n", "external");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        editor.with_execution_scope(|editor| {
            keys(editor, "x");
            panic!("simulated command panic");
        });
    }));
    assert!(result.is_err());
    assert_eq!(backend.content(), "a");
    keys(&mut editor, "x");
    assert_eq!(backend.counts(), (0, 2));
    assert_eq!(backend.content(), "b");
}

#[test]
fn external_boundary_flushes_before_execution_and_refreshes_afterward() {
    let (mut editor, backend) = setup("abc\n", "external");
    editor.with_execution_scope(|editor| {
        keys(editor, "x");
        assert_eq!(backend.counts(), (0, 0));
        editor.with_external_effects(|_| {
            assert_eq!(backend.content(), "a");
            assert_eq!(backend.counts(), (0, 1));
            backend.external_write("Z");
        });
        keys(editor, "P");
        assert_eq!(editor.buffer().rope().to_string(), "Zbc\n");
    });
    assert_eq!(backend.counts(), (1, 1));
    assert_eq!(backend.content(), "Z");
}

#[test]
fn direct_ex_commands_publish_and_nested_ex_commands_join_scope() {
    let (mut editor, backend) = setup("first\nsecond\nthird\n", "external");
    InputHandler::execute_command_string(&mut editor, "1y").unwrap();
    assert_eq!(backend.content(), "first\n");
    assert_eq!(backend.counts(), (0, 1));
    editor.with_execution_scope(|editor| {
        InputHandler::execute_command_string(editor, "1d").unwrap();
        InputHandler::execute_command_string(editor, "1y").unwrap();
        assert_eq!(backend.counts(), (0, 1));
        assert_eq!(editor.registers.get(Some('0')), "second\n");
        assert_eq!(editor.registers.get(Some('1')), "first\n");
    });
    assert_eq!(backend.content(), "second\n");
    assert_eq!(backend.counts(), (0, 2));
    assert_eq!(editor.buffer().rope().to_string(), "second\nthird\n");
}

#[test]
#[cfg(unix)]
fn synchronous_shell_entrypoints_flush_inside_the_outer_scope() {
    for command in ["r !printf shell", ".!cat"] {
        let (mut editor, backend) = setup("abc\n", "external");
        editor.with_execution_scope(|editor| {
            keys(editor, "x");
            assert_eq!(backend.counts(), (0, 0));
            InputHandler::execute_command_string(editor, command).unwrap();
            assert_eq!(backend.counts(), (0, 1), "{command}");
            assert_eq!(backend.content(), "a");
            backend.external_write("fresh");
            assert_eq!(editor.registers.get_clipboard(), "fresh", "{command}");
        });
        assert_eq!(backend.counts(), (1, 1));
    }
}

#[test]
#[ignore = "diagnostic timing with controlled clipboard write latency; run with --ignored --nocapture"]
fn diagnostic_clipboard_batching_cost() {
    const REPEATS: usize = 200;
    fn timed(batched: bool) -> (Duration, String, (usize, usize)) {
        let backend = Arc::new(FakeClipboard {
            write_delay: Duration::from_micros(100),
            ..FakeClipboard::default()
        });
        let mut editor = Editor::new();
        editor.registers = RegisterManager::with_clipboard_backend(backend.clone());
        *editor.buffer_mut() = Buffer::new_from_str("abc\n");
        record(&mut editor, 'a', "xP");
        let start = Instant::now();
        if batched {
            keys(&mut editor, &format!("{REPEATS}@a"));
        } else {
            for _ in 0..REPEATS {
                keys(&mut editor, "xP");
            }
        }
        (
            start.elapsed(),
            editor.buffer().rope().to_string(),
            backend.counts(),
        )
    }
    let physical = timed(false);
    let macros = timed(true);
    assert_eq!(physical.1, macros.1);
    assert_eq!(physical.2, (REPEATS, REPEATS));
    assert_eq!(macros.2, (0, 1));
    eprintln!("Controlled 100us/write backend, {REPEATS} xP iterations: physical {:?} {:?}; macro {:?} {:?} (reads, writes)", physical.0, physical.2, macros.0, macros.2);
}

#[test]
fn mapping_rhs_shares_one_execution_scope() {
    let (mut editor, backend) = setup("abc\n", "external");
    InputHandler::execute_command_string(&mut editor, "nnoremap Q xPxP").unwrap();
    keys(&mut editor, "Q");
    assert_eq!(editor.buffer().rope().to_string(), "abc\n");
    assert_eq!(backend.counts(), (0, 1));
}

#[test]
fn replacing_register_state_preserves_external_write_order() {
    let (mut editor, backend) = setup("abc\n", "external");
    editor.with_execution_scope(|editor| {
        keys(editor, "x");
        *editor.registers_mut() = RegisterManager::with_clipboard_backend(backend.clone());
        assert_eq!(backend.content(), "a");
        keys(editor, "x");
    });
    assert_eq!(backend.content(), "b");
    assert_eq!(backend.counts(), (0, 2));
}
