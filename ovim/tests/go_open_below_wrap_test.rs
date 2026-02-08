mod helpers;

use helpers::EditorTest;

#[test]
fn go_then_open_below_keeps_new_line_visible_with_wrap() {
    // Repro: with wrap enabled, inserting a new line at EOF can make the cursor line
    // fall below the visible region until the next render rebuilds the wrap map.
    // We want scrolling to stay correct immediately after `o`.
    let content = [
        "line0",
        "line1",
        // 20 chars -> wraps into 2 rows at width=10
        "0123456789abcdefghij",
        "line3",
        "line4",
        "",
    ]
    .join("\n");

    let mut test = EditorTest::new(&content);
    test.editor.options.wrap = true;
    test.editor.options.scrolloff = 0;
    test.editor.set_viewport_height(3);
    test.editor.ensure_wrap_map(10);

    // `Go` = jump to EOF then open a new line below (enter insert mode).
    test.keys("Go");

    let cursor_line = test.editor.buffer().cursor().line();
    let last_line = test.editor.buffer().line_count().saturating_sub(1);
    assert_eq!(cursor_line, last_line);

    // With the wrapped line above, the viewport must start on logical line 3 to show:
    // - line3
    // - line4
    // - newly inserted blank line (cursor)
    assert_eq!(test.editor.scroll_offset(), 3);
}
