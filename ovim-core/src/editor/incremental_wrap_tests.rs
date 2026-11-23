use super::Editor;
use crate::unicode::{CharCol, GraphemeCol};
use std::sync::Arc;

#[test]
fn same_line_edit_reuses_other_line_geometry() {
    let mut editor = Editor::with_content(&"abcdef\n".repeat(100));
    editor.options.wrap = true;
    editor.ensure_wrap_map(4);
    let unchanged = editor.wrap_map().unwrap().line_layout(80).unwrap().clone();
    editor.buffer_mut().insert_text_at(3, CharCol(1), "12345");
    editor.ensure_wrap_map(4);
    let map = editor.wrap_map().unwrap();
    assert_eq!(map.last_recomputed_lines(), 1);
    assert_eq!(map.visual_lines_for(3), 3);
    assert!(Arc::ptr_eq(&unchanged, map.line_layout(80).unwrap()));
    assert_eq!(map.logical_to_visual(4), 9);
}

#[test]
fn structural_edits_shift_reusable_geometry_and_refresh_touched_lines() {
    let mut editor = Editor::with_content("abcdef\nghijkl\nmnopqr\n");
    editor.options.wrap = true;
    editor.ensure_wrap_map(4);
    let tail = editor.wrap_map().unwrap().line_layout(2).unwrap().clone();
    editor.buffer_mut().insert_text_at(0, CharCol(3), "\n");
    editor.ensure_wrap_map(4);
    let map = editor.wrap_map().unwrap();
    assert_eq!(map.last_recomputed_lines(), 2);
    assert_eq!(map.visual_lines_for(0), 1);
    assert_eq!(map.visual_lines_for(1), 1);
    assert!(Arc::ptr_eq(&tail, map.line_layout(3).unwrap()));
    editor
        .buffer_mut()
        .delete_range(0, CharCol(3), 1, CharCol::ZERO);
    editor.ensure_wrap_map(4);
    let map = editor.wrap_map().unwrap();
    assert_eq!(map.last_recomputed_lines(), 1);
    assert!(Arc::ptr_eq(&tail, map.line_layout(2).unwrap()));
    assert_eq!(map.visual_lines_for(0), 2);
}

#[test]
fn raw_mutation_and_tab_policy_change_rebuild_safely() {
    let mut editor = Editor::with_content("\tx\n");
    editor.options.wrap = true;
    let mut indent = editor.indent_options();
    indent.tab_width = 8;
    editor.set_indent_options(indent);
    editor.ensure_wrap_map(4);
    assert_eq!(editor.wrap_map().unwrap().visual_lines_for(0), 3);
    indent.tab_width = 2;
    editor.set_indent_options(indent);
    editor.ensure_wrap_map(4);
    assert_eq!(editor.wrap_map().unwrap().visual_lines_for(0), 1);
    editor.buffer_mut().rope_mut().insert(0, "abcdefgh");
    editor.ensure_wrap_map(4);
    assert_eq!(editor.wrap_map().unwrap().visual_lines_for(0), 3);
}

#[test]
fn journal_eviction_falls_back_to_current_content() {
    let mut editor = Editor::with_content("x\n");
    editor.options.wrap = true;
    editor.ensure_wrap_map(80);
    for _ in 0..300 {
        editor.buffer_mut().insert_text_at(0, CharCol::ZERO, "x");
    }
    editor.ensure_wrap_map(80);
    assert_eq!(editor.wrap_map().unwrap().visual_lines_for(0), 4);
    assert_eq!(editor.wrap_map().unwrap().last_recomputed_lines(), 2);
}

#[test]
fn structural_edit_reconceals_the_previously_revealed_cursor_line() {
    let mut editor = Editor::with_content(&"[label](https://example.test/path)\n".repeat(12));
    editor.set_file_path("/tmp/incremental-wrap.md".to_string());
    editor.options.wrap = true;

    // The initial map reveals line 5 because it owns the cursor, while
    // every other Markdown link line is concealed.
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(5, GraphemeCol::ZERO);
    editor.ensure_wrap_map(8);
    assert!(editor.wrap_map().unwrap().line_transform(5).is_none());
    assert!(editor.wrap_map().unwrap().line_transform(6).is_some());

    // Move without rendering, then insert a line ahead of both cursor
    // positions. The old revealed row shifts from 5 to 6; the active
    // cursor becomes line 11 in final coordinates.
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(10, GraphemeCol::ZERO);
    editor.buffer_mut().insert_text_at(0, CharCol::ZERO, "\n");
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(11, GraphemeCol::ZERO);
    editor.ensure_wrap_map(8);

    let map = editor.wrap_map().unwrap();
    assert_eq!(map.conceal_cursor_line(), Some(11));
    assert!(
        map.line_transform(6).is_some(),
        "shifted old cursor line must conceal"
    );
    assert!(
        map.line_transform(11).is_none(),
        "current cursor line must reveal"
    );
}

#[test]
fn visual_scroll_refreshes_same_line_edit_before_using_row_counts() {
    let mut editor = Editor::with_content("abcd\nx\n");
    editor.options.wrap = true;
    editor.init_window_manager(4, 1);
    editor.ensure_wrap_map(4);
    assert_eq!(editor.wrap_map().unwrap().visual_lines_for(0), 1);

    // The map still has the old logical row count. Ctrl-E must refresh it
    // before converting one visual-row scroll into (line, subrow).
    editor.buffer_mut().insert_text_at(0, CharCol(4), "efgh");
    editor.scroll_viewport_down(1);

    assert_eq!(editor.wrap_map().unwrap().visual_lines_for(0), 2);
    assert_eq!(editor.scroll_offset(), 0);
    assert_eq!(editor.scroll_subrow(), 1);
}
