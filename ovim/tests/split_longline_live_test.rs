//! Live demo-project regression: splitting at a deep wrapped cursor must
//! preserve visibility in both resulting panes on their first render.
mod helpers;

use helpers::EditorTest;
use ovim::ui::render_editor_to_ansi;

#[test]
fn splitting_at_a_deep_wrapped_cursor_keeps_both_panes_visible() {
    for (split_keys, prepare_frontend) in [
        ("<C-w>v", false),
        ("<C-w>s", false),
        ("<C-w>v", true),
        ("<C-w>s", true),
    ] {
        let text = format!("{}Z\nsecond line\n", "世界 ".repeat(5001));
        let mut test = EditorTest::new(&text);
        test.editor.options.wrap = true;
        test.editor.options.scrolloff = 0;
        ovim::frontend::handle_viewport_resize(&mut test.editor, 100, 30);
        test.keys("$");
        test.editor.update_scroll_offset();
        let _ = render_editor_to_ansi(&mut test.editor, 100, 30).unwrap();
        assert!(test.editor.scroll_subrow() > 0);
        let cursor = *test.editor.buffer().cursor();
        test.keys(split_keys);
        if prepare_frontend {
            ovim::frontend::handle_viewport_resize(&mut test.editor, 100, 30);
        }
        let rendered = render_editor_to_ansi(&mut test.editor, 100, 30).unwrap();
        assert_eq!(ovim::ui::strip_ansi(&rendered).matches("Z").count(), 2,
            "both panes must draw the cursor's source text after {split_keys}, frontend={prepare_frontend}:\n{}", ovim::ui::strip_ansi(&rendered));
        assert_eq!(*test.editor.buffer().cursor(), cursor);
        let wm = test.editor.window_manager().unwrap();
        for pane in 0..2 {
            let window = wm.get_window(pane).unwrap();
            let map = window.wrap_map().unwrap();
            let index = test.editor.buffer().line_index(cursor.line());
            let char_col = index.grapheme_to_char(cursor.col()).0;
            let position = map
                .line_layout(cursor.line())
                .unwrap()
                .position_for_char(char_col);
            let visual = map.logical_to_visual(cursor.line()) + position.row;
            let top = map.viewport_top_visual_row(window.scroll_offset(), window.scroll_subrow());
            assert!(
                visual >= top && visual < top + window.height() as usize,
                "{split_keys} pane {pane}: cursor row {visual}, viewport {top}..{}",
                top + window.height() as usize
            );
        }
    }
}
