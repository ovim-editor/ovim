//! Acceptance test for the `ovim::frontend` contract documented in
//! `ovim/src/frontend/mod.rs`. It simulates a minimal third frontend (a
//! future GUI) that links `ovim` as a library and drives the editor core
//! through nothing but the public lib API: `use ovim::…`. Binary-target
//! modules (`event_loop`, `api_dispatch`, the crossterm/ratatui-specific
//! bits) are unreachable from an integration test crate regardless — that
//! unreachability is the point this test exists to demonstrate: a GUI does
//! not need to touch the binary to embed the editor.
//!
//! Each test below is named after, and commented with, the contract step(s)
//! from `frontend/mod.rs` it exercises. This is a smoke test of contract
//! steps 1, 2 and 4 (viewport resize, tick, input dispatch), not a feature
//! test suite; the tick's own ordering and cadence policies are tested in
//! `ovim-core/src/tick/tests.rs`, and feature coverage belongs in the other
//! files under `ovim/tests/`.

use ovim::api::parse_key_string;
use ovim::editor::{Editor, InputHandler};
use ovim::frontend::{handle_viewport_resize, refresh_after_input, TickState};
use ovim::mode::Mode;

/// Contract step 2: build a `TickState` and run `Editor::tick`
/// on a periodic interval to drive background work. A tick against a
/// freshly created editor (no open file, no LSP, no picker) must complete
/// promptly rather than panic or hang.
#[tokio::test(flavor = "current_thread")]
async fn tick_completes_on_a_default_editor() {
    let mut editor = Editor::with_content("hello\n");
    let mut state = TickState::new();

    let result =
        tokio::time::timeout(std::time::Duration::from_secs(5), editor.tick(&mut state)).await;

    assert!(
        result.is_ok_and(|report| report.terminal_request.is_none()),
        "Editor::tick did not complete within the timeout"
    );
}

/// Contract steps 1 and 4: resize the viewport, dispatch keys through the
/// same `InputHandler` path the TUI uses (`ovim::api::parse_key_string` ->
/// `InputHandler::handle_key_event_no_dirty`), then call
/// `refresh_after_input`. Mirrors the "jA...<Esc>" sequence from
/// `event_loop.rs`'s `api_keys_match_direct_input_state_and_render` parity
/// test: `j` moves down a line, `A` appends at end of line and enters
/// insert mode, and `<Esc>` returns to normal mode with the cursor pulled
/// back one column onto the last inserted character.
///
/// vim (nvim --clean, "alpha\nbeta\ngamma"): `jA hello<Esc>` -> line 2 is
/// "beta hello", col(".") == 10 (1-indexed), i.e. Escape pulls the cursor
/// back onto the last inserted character.
#[tokio::test(flavor = "current_thread")]
async fn key_dispatch_through_input_handler_updates_buffer_cursor_and_mode() {
    let mut editor = Editor::with_content("alpha\nbeta\ngamma\n");
    handle_viewport_resize(&mut editor, 80, 24);

    for event in parse_key_string("jA hello<Esc>").unwrap() {
        InputHandler::handle_key_event_no_dirty(&mut editor, event).unwrap();
    }
    refresh_after_input(&mut editor);

    assert_eq!(
        editor.buffer().rope().to_string(),
        "alpha\nbeta hello\ngamma\n"
    );
    assert_eq!(editor.buffer().cursor().line(), 1);
    assert_eq!(
        editor.buffer().cursor().col().0,
        9,
        "col(\".\") should be 10 (1-indexed) per the nvim citation above"
    );
    assert_eq!(editor.mode(), Mode::Normal);
}

/// Contract step 1: `handle_viewport_resize` takes raw grid cells and
/// subtracts chrome (tab bar, file tree, LSP progress line, status +
/// OV-00337: vim's special-key notation is case-insensitive for multi-char
/// key names and modifier prefixes (verified `nvim --clean`: `:map <esc>`,
/// `:map <c-w>`, `:map <cr>` all register the same as their canonical
/// spellings). `<esc>` used to be inserted as five literal characters,
/// leaving headless sessions stuck in insert mode. Single-char base keys
/// stay case-sensitive because `<C-a>` and `<C-A>` are distinct chords.
#[test]
fn special_key_notation_is_case_insensitive_for_names_and_modifiers() {
    use ovim_core::key::{KeyCode, Modifiers};

    for spelling in ["<Esc>", "<esc>", "<ESC>"] {
        let events = parse_key_string(spelling).unwrap();
        assert_eq!(events.len(), 1, "{spelling} must parse as one key");
        assert_eq!(events[0].code, KeyCode::Esc, "{spelling}");
    }

    for spelling in ["<CR>", "<cr>", "<Enter>", "<enter>"] {
        let events = parse_key_string(spelling).unwrap();
        assert_eq!(events.len(), 1, "{spelling} must parse as one key");
        assert_eq!(events[0].code, KeyCode::Enter, "{spelling}");
    }

    for spelling in ["<C-w>", "<c-w>", "<ctrl-w>"] {
        let events = parse_key_string(spelling).unwrap();
        assert_eq!(events.len(), 1, "{spelling} must parse as one key");
        assert_eq!(events[0].code, KeyCode::Char('w'), "{spelling}");
        assert!(
            events[0].modifiers.contains(Modifiers::CONTROL),
            "{spelling}"
        );
    }

    // Single-char base keys remain case-sensitive: <C-a> != <C-A>.
    let lower = parse_key_string("<c-a>").unwrap();
    let upper = parse_key_string("<c-A>").unwrap();
    assert_eq!(lower[0].code, KeyCode::Char('a'));
    assert_eq!(upper[0].code, KeyCode::Char('A'));
}

/// command lines) to get the content viewport. A plain single-tab editor
/// with no file tree and no LSP progress line only pays for the status and
/// command lines, so a 24-row window yields a viewport height of 22 (the
/// same "2 rows of chrome" relationship the 20 -> 18 case in
/// `frontend/viewport.rs`'s own resize test asserts).
#[test]
fn viewport_resize_reflects_chrome_subtracted_from_raw_height() {
    let mut editor = Editor::with_content("hello\n");

    handle_viewport_resize(&mut editor, 80, 24);

    assert_eq!(editor.viewport_height(), 22);
}

/// Contract step 2: a tick (which also drains picker results) must be a
/// no-op when the picker was never opened. The tick only drives picker
/// background work while `editor.mode() == Mode::Picker`, and the result
/// channels are simply empty, so it returns without touching editor state.
#[tokio::test(flavor = "current_thread")]
async fn tick_and_picker_drain_are_a_no_op_without_an_open_picker() {
    let mut editor = Editor::with_content("hello\n");
    let mut state = TickState::new();
    let version_before = editor.buffer().version();

    let _report = editor.tick(&mut state).await;

    assert_eq!(editor.mode(), Mode::Normal);
    assert_eq!(
        editor.buffer().version(),
        version_before,
        "a tick and picker drain with no open picker must not mutate the buffer"
    );
    assert!(
        editor.picker().is_none(),
        "no picker was opened, so none should exist after tick + drain"
    );
}
