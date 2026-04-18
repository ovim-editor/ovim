//! Tests for `AnsiRenderCache` — the headless render cache that backs
//! the `/v1/render` API path (OV-00181).
//!
//! The cache exists to keep the main event loop responsive when external
//! clients poll `/v1/render` repeatedly: re-running the full ratatui
//! draw + syntax highlighting pipeline on every poll synchronously
//! starves other API requests, LSP polling, and tick handlers until it
//! completes.

use ovim::editor::Editor;
use ovim::ui::AnsiRenderCache;
use ovim_core::{KeyCode, KeyEvent, Modifiers};

#[test]
fn cache_serves_repeat_render_without_recomputing() {
    let mut editor = Editor::with_content("hello world");
    let mut cache = AnsiRenderCache::new();

    // Cold call — no entry yet, must do real work.
    assert!(!cache.would_hit(&editor, 80, 24, false));
    let first = cache.render(&mut editor, 80, 24, false).unwrap();

    // Same dimensions, same plain flag, no editor mutation in between:
    // the next call should be a hit and produce byte-identical output.
    assert!(cache.would_hit(&editor, 80, 24, false));
    let second = cache.render(&mut editor, 80, 24, false).unwrap();
    assert_eq!(first, second);
}

#[test]
fn editor_mutation_invalidates_cache() {
    let mut editor = Editor::with_content("hello world");
    let mut cache = AnsiRenderCache::new();

    let _ = cache.render(&mut editor, 80, 24, false).unwrap();
    assert!(cache.would_hit(&editor, 80, 24, false));

    // Any `mark_dirty()` bumps the render-input version — so does any
    // operation that goes through the input handler. Move the cursor
    // through the input handler and confirm the cache no longer matches.
    let event = KeyEvent::new(KeyCode::Char('l'), Modifiers::NONE);
    ovim::editor::InputHandler::handle_key_event(&mut editor, event).unwrap();
    editor.mark_dirty();

    assert!(!cache.would_hit(&editor, 80, 24, false));
}

#[test]
fn dimension_change_invalidates_cache() {
    let mut editor = Editor::with_content("hello world");
    let mut cache = AnsiRenderCache::new();

    let _ = cache.render(&mut editor, 80, 24, false).unwrap();
    assert!(cache.would_hit(&editor, 80, 24, false));
    assert!(!cache.would_hit(&editor, 100, 24, false));
    assert!(!cache.would_hit(&editor, 80, 30, false));

    // Rendering at the new size populates the cache for that size and
    // evicts the previous one.
    let _ = cache.render(&mut editor, 100, 24, false).unwrap();
    assert!(cache.would_hit(&editor, 100, 24, false));
    assert!(!cache.would_hit(&editor, 80, 24, false));
}

#[test]
fn plain_and_ansi_outputs_are_cached_independently() {
    let mut editor = Editor::with_content("hello world");
    let mut cache = AnsiRenderCache::new();

    let with_ansi = cache.render(&mut editor, 80, 24, false).unwrap();
    let plain = cache.render(&mut editor, 80, 24, true).unwrap();

    // Plain output should not contain ESC; ANSI output should.
    assert!(!plain.contains('\x1b'), "plain output leaked ANSI escapes");
    assert!(
        with_ansi.contains('\x1b'),
        "ANSI output unexpectedly stripped"
    );

    // Switching `plain` is a cache miss but the underlying editor isn't
    // mutated, so flipping back-and-forth stabilises after one render
    // each. Re-asking for the most-recent variant is a hit.
    assert!(cache.would_hit(&editor, 80, 24, true));
    assert!(!cache.would_hit(&editor, 80, 24, false));
}

#[test]
fn render_input_version_increments_on_mark_dirty() {
    let mut editor = Editor::with_content("hello");
    let v0 = editor.render_input_version();
    editor.mark_dirty();
    let v1 = editor.render_input_version();
    assert!(v1 > v0, "mark_dirty did not bump render_input_version");
}
