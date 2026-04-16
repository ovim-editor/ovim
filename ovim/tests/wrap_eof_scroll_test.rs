use ovim::editor::Editor;

#[test]
fn wrap_allows_scrolling_to_reveal_final_logical_lines() {
    // Minimal repro:
    // - Small viewport (3 rows)
    // - A wrapped line near EOF consumes 2 visual rows
    // - Without wrap-aware max-scroll, the viewport cannot advance far enough
    //   to show the final logical lines.
    let content = [
        "line0",
        "line1",
        // 20 chars → wraps into 2 rows at width=10
        "0123456789abcdefghij",
        "line3",
        "line4",
    ]
    .join("\n");

    let mut editor = Editor::with_content(&content);
    editor.options.wrap = true;
    editor.options.scrolloff = 0;
    editor.set_viewport_height(3);
    editor.ensure_wrap_map(10);

    let last_line = editor.buffer().line_count().saturating_sub(1);
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(last_line, ovim::unicode::GraphemeCol::ZERO);

    editor.update_scroll_offset();

    // With a 3-row viewport, starting at logical line 2 would render:
    // - line2 row1
    // - line2 row2
    // - line3
    // making it impossible to see line4. We must allow scrolling to line3.
    assert_eq!(editor.scroll_offset(), 3);
}
