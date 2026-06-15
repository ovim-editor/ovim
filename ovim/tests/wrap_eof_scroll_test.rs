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

    // The final logical line must be visible. With a visual-row sub-offset the
    // viewport scrolls *partway into* the wrapped line2 (offset 2, sub-row 1)
    // rather than jumping to a logical-line boundary, placing the cursor exactly
    // at the bottom edge while still revealing line2's tail. Assert the invariant
    // (final line on screen), not the exact offset mechanics.
    let height = 3;
    let map = editor.wrap_map().expect("wrap map");
    let top_visual = map.logical_to_visual(editor.scroll_offset()) + editor.scroll_subrow();
    let line_text = editor.buffer().line_text(last_line).unwrap_or_default();
    let (cursor_visual, _) = map.cursor_to_visual(last_line, 0, &line_text);
    assert!(
        cursor_visual >= top_visual && cursor_visual < top_visual + height,
        "final logical line must be visible: cursor visual row {cursor_visual}, \
         viewport visual rows {top_visual}..{}",
        top_visual + height
    );
}
