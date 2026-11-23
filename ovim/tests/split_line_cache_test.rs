//! The panes of a split render one after the other each frame. Each pane's
//! rendered lines must survive the others being drawn in between, or an idle
//! split re-renders every visible line of every pane on every frame.

mod helpers;

use helpers::EditorTest;
use ovim::ui::Renderer;
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn an_idle_split_redraws_from_the_line_cache() {
    let text: String = (0..40).map(|n| format!("fn line_{n}() {{}}\n")).collect();
    let mut test = EditorTest::new(&text);
    test.editor.init_window_manager(100, 30);
    test.editor.split_window_vertical();
    // Panes of different widths, so their frames differ.
    test.editor.split_window_horizontal();

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let mut cache = Default::default();
    let mut hits = Vec::new();
    for _ in 0..3 {
        terminal
            .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut cache))
            .unwrap();
        // Statistics describe the pane rendered last.
        hits.push((cache.hits, cache.misses));
    }
    let (first_hits, _) = hits[0];
    let (second_hits, second_misses) = hits[1];
    assert_eq!(first_hits, 0, "a cold cache has nothing to hit");
    assert!(
        second_hits > 0 && second_misses <= 2,
        "the last pane re-rendered from scratch: {hits:?}"
    );
    assert_eq!(hits[1], hits[2], "steady state");
}
