mod helpers;

use helpers::EditorTest;
use ovim_core::editor::decoration::{
    Decoration, DecorationPlacement, DecorationSource, DecorationStyle,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create an inline inlay-hint decoration at the given absolute char offset.
fn hint_at(char_offset: usize, text: &str) -> Decoration {
    Decoration {
        placement: DecorationPlacement::Inline { char_offset },
        source: DecorationSource::InlayHint,
        text: text.to_string(),
        display_width: text.len(), // ASCII-safe for test hints
        style: DecorationStyle::new(ovim_core::color::Color::Gray).with_italic(),
        priority: 10,
        source_version: 0,
    }
}

/// Set up an EditorTest with wrap enabled, a fixed viewport, and a wrap map
/// built at the given text width.
fn setup_wrapped(content: &str, text_width: usize) -> EditorTest {
    let mut test = EditorTest::new(content);
    test.editor.options.wrap = true;
    test.editor.options.scrolloff = 0;
    test.editor.set_viewport_height(40);
    test.editor.ensure_wrap_map(text_width);
    test
}

/// Add inlay hints to the editor and rebuild the wrap map.
fn add_hints(test: &mut EditorTest, hints: Vec<Decoration>, text_width: usize) {
    let rope = test.editor.buffer().rope().clone();
    test.editor
        .decorations
        .replace_source(DecorationSource::InlayHint, hints, &rope);
    test.editor.ensure_wrap_map(text_width);
}

// ============================================================================
// Basic cursor motions with inlay hints that cause wrapping
// ============================================================================

#[test]
fn h_l_cursor_stays_in_buffer_coords_with_hint() {
    // "let x = 5;" is 10 chars.  A ": i32" hint (5 cols) at char 5
    // makes the *display* 15 cols, but buffer positions stay 0..9.
    let mut test = setup_wrapped("let x = 5;", 20);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 20);

    // Start at col 0, move right across the line
    test.assert_cursor(0, 0);
    test.keys("$");
    // $ goes to last char (';' at col 9)
    test.assert_cursor(0, 9);

    // Move left back to start
    test.keys("0");
    test.assert_cursor(0, 0);
}

#[test]
fn j_k_across_lines_with_hints_on_first_line() {
    // Line 0: "let x = 5;"  (10 chars)  + ": i32" hint at offset 5
    // Line 1: "let y = 10;" (11 chars)
    let content = "let x = 5;\nlet y = 10;";
    let mut test = setup_wrapped(content, 20);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 20);

    test.set_cursor(0, 4);
    test.keys("j");
    test.assert_cursor(1, 4);

    test.keys("k");
    test.assert_cursor(0, 4);
}

#[test]
fn j_k_across_lines_with_hints_on_both_lines() {
    // Line 0: "let x = 5;"  (10+\n=11 chars) + hint at offset 5
    // Line 1: "let y = 10;" (11 chars)        + hint at offset 16 (11+5)
    let content = "let x = 5;\nlet y = 10;";
    let mut test = setup_wrapped(content, 20);
    add_hints(
        &mut test,
        vec![hint_at(5, ": i32"), hint_at(16, ": i32")],
        20,
    );

    test.set_cursor(0, 4);
    test.keys("j");
    test.assert_cursor(1, 4);
    test.keys("k");
    test.assert_cursor(0, 4);
}

// ============================================================================
// Hints that push lines past textwidth, forcing wrapping
// ============================================================================

#[test]
fn hint_causes_wrap_cursor_on_first_subline() {
    // "let x = 5;" is 10 display cols.
    // With ": i32" (5 cols) at char 5, display = 15 cols.
    // At textwidth=12, this wraps.  Cursor at col 0 should still be on
    // the first sub-line.
    let mut test = setup_wrapped("let x = 5;", 12);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 12);

    test.assert_cursor(0, 0);
    // l moves one buffer char to the right regardless of wrap
    test.keys("l");
    test.assert_cursor(0, 1);
}

#[test]
fn hint_causes_wrap_dollar_finds_eol() {
    let mut test = setup_wrapped("let x = 5;", 12);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 12);

    test.keys("$");
    test.assert_cursor(0, 9);
}

#[test]
fn hint_causes_wrap_j_moves_to_next_logical_line() {
    // Line 0 wraps due to hint, but j should still go to line 1
    let content = "let x = 5;\nlet y = 10;";
    let mut test = setup_wrapped(content, 12);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 12);

    test.keys("j");
    test.assert_cursor(1, 0);

    test.keys("k");
    test.assert_cursor(0, 0);
}

#[test]
fn multiple_hints_cause_wrap_j_k_round_trips() {
    // Two hints on the same line push it well past textwidth.
    // "fn foo(a, b)" = 12 chars
    // Hints: "count: " (7 cols) at char 7, "limit: " (7 cols) at char 10
    // Display: "fn foo(count: a, limit: b)" = 26 cols → wraps at width 15.
    let content = "fn foo(a, b)\nreturn a + b";
    let mut test = setup_wrapped(content, 15);
    add_hints(
        &mut test,
        vec![hint_at(7, "count: "), hint_at(10, "limit: ")],
        15,
    );

    test.set_cursor(0, 5);
    test.keys("j");
    test.assert_cursor(1, 5);
    test.keys("k");
    test.assert_cursor(0, 5);
}

// ============================================================================
// Word motions with inlay hints
// ============================================================================

#[test]
fn w_motion_with_hints() {
    // "let x = 5;" — words: "let", "x", "=", "5", ";"
    // Hint at char 5 (": i32") doesn't affect word boundaries.
    let mut test = setup_wrapped("let x = 5;", 20);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 20);

    test.assert_cursor(0, 0);
    test.keys("w"); // -> "x" at col 4
    test.assert_cursor(0, 4);
    test.keys("w"); // -> "=" at col 6
    test.assert_cursor(0, 6);
    test.keys("w"); // -> "5" at col 8
    test.assert_cursor(0, 8);
}

#[test]
fn b_motion_with_hints() {
    let mut test = setup_wrapped("let x = 5;", 20);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 20);

    test.set_cursor(0, 9);
    test.keys("b"); // -> "5" at col 8
    test.assert_cursor(0, 8);
    test.keys("b"); // -> "=" at col 6
    test.assert_cursor(0, 6);
    test.keys("b"); // -> "x" at col 4
    test.assert_cursor(0, 4);
    test.keys("b"); // -> "let" at col 0
    test.assert_cursor(0, 0);
}

// ============================================================================
// Macros recorded with hints present — cursor should be consistent
// ============================================================================

#[test]
fn macro_j_replayed_with_hints_causing_wrap() {
    // Record a macro that does j (move down), then replay it.
    // With hints causing wrap on line 0, cursor should still land correctly.
    let content = "let x = 5;\nlet y = 10;\nlet z = 15;";
    let mut test = setup_wrapped(content, 12);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 12);

    // Record: q a j q
    test.keys("qajq");
    test.assert_cursor(1, 0);

    // Replay: @a
    test.keys("@a");
    test.assert_cursor(2, 0);
}

#[test]
fn macro_w_replayed_with_hints() {
    let mut test = setup_wrapped("let x = 5;", 20);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 20);

    // Record: q a w q  (move one word forward)
    test.keys("qawq");
    test.assert_cursor(0, 4);

    // Replay 2 more times: 2@a
    test.keys("2@a");
    test.assert_cursor(0, 8);
}

#[test]
fn macro_insert_with_hints_causing_wrap() {
    // Record a macro that inserts text on a line with hints causing wrap.
    // The logical cursor should end up correct after insert + Esc.
    let content = "let x = 5;\nlet y = 10;";
    let mut test = setup_wrapped(content, 12);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 12);

    // Record: q a I // <Space> <Esc> j q  (insert comment prefix, move down)
    test.keys("qaI// <Esc>jq");

    // Line 0 should now be "// let x = 5;"
    assert!(test.buffer_content().starts_with("// let x = 5;"));
    // After I + "// " + Esc, cursor is on last inserted char (col 2 = ' ').
    // Then j moves down keeping goal column 2.
    test.assert_cursor(1, 2);
}

#[test]
fn macro_dd_with_hints_causing_wrap() {
    let content = "let x = 5;\nlet y = 10;\nlet z = 15;";
    let mut test = setup_wrapped(content, 12);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 12);

    // Record: qa dd q  (delete current line)
    test.keys("qaddq");
    test.assert_cursor(0, 0);
    assert_eq!(test.line(0).unwrap().trim_end(), "let y = 10;");

    // Replay
    test.keys("@a");
    test.assert_cursor(0, 0);
    assert_eq!(test.line(0).unwrap().trim_end(), "let z = 15;");
}

// ============================================================================
// Edge cases: hint at line boundary / hint width equals textwidth
// ============================================================================

#[test]
fn hint_at_end_of_line_does_not_affect_dollar() {
    // Hint placed at the very end of the line (after the semicolon).
    // "let x = 5;" is 10 chars; hint at char 10 (after ';').
    let mut test = setup_wrapped("let x = 5;", 20);
    add_hints(&mut test, vec![hint_at(10, ": i32")], 20);

    test.keys("$");
    test.assert_cursor(0, 9);
}

#[test]
fn hint_wider_than_textwidth() {
    // A hint so wide it alone exceeds textwidth.  Cursor motions must
    // still operate in buffer coordinates.
    let mut test = setup_wrapped("ab", 5);
    add_hints(&mut test, vec![hint_at(1, ": SomeVeryLongTypeName")], 5);

    test.assert_cursor(0, 0);
    test.keys("l");
    test.assert_cursor(0, 1);
    test.keys("h");
    test.assert_cursor(0, 0);
}

#[test]
fn hint_at_col_zero() {
    // Parameter-name style hint at the start of the line.
    // "5, 10" — w from '5' jumps to ',' (punctuation word), not '10'.
    let content = "5, 10";
    let mut test = setup_wrapped(content, 20);
    add_hints(&mut test, vec![hint_at(0, "count: ")], 20);

    test.assert_cursor(0, 0);
    test.keys("w"); // -> "," at col 1
    test.assert_cursor(0, 1);
}

// ============================================================================
// Multiple lines each with hints — verifying goal column
// ============================================================================

#[test]
fn goal_column_preserved_across_lines_with_varying_hints() {
    // Line 0: "let x = 5;"  (hint at offset 5)
    // Line 1: "let yy = 10;" (hint at offset 17 = 11 + 6)
    // Line 2: "let z = 15;" (hint at offset 25 = 11 + 14 + ?)
    // Moving down from col 4 should stay at col 4 on each line.
    let content = "let x = 5;\nlet yy = 10;\nlet z = 15;";
    let mut test = setup_wrapped(content, 20);
    // Offsets: line0 = 0..10, line1 = 11..23, line2 = 24..34
    add_hints(
        &mut test,
        vec![
            hint_at(5, ": i32"),
            hint_at(17, ": i64"),
            hint_at(29, ": u8"),
        ],
        20,
    );

    test.set_cursor(0, 4);
    test.keys("j");
    test.assert_cursor(1, 4);
    test.keys("j");
    test.assert_cursor(2, 4);
    test.keys("k");
    test.assert_cursor(1, 4);
    test.keys("k");
    test.assert_cursor(0, 4);
}

#[test]
fn goal_column_clamps_on_short_line_with_hint() {
    // Line 0: "let x = 5;"  (10 chars)
    // Line 1: "ab"          (2 chars, with a hint)
    // Line 2: "let z = 15;"
    let content = "let x = 5;\nab\nlet z = 15;";
    let mut test = setup_wrapped(content, 20);
    // offset of "ab" line starts at 11; hint at offset 12 (char 'b')
    add_hints(&mut test, vec![hint_at(12, ": String")], 20);

    test.set_cursor(0, 9); // col 9 on line 0 (';')
    test.keys("j");
    // Line 1 "ab" only has cols 0..1; cursor clamps to col 1
    test.assert_cursor(1, 1);
    test.keys("j");
    // Goal column is 9, line 2 has enough chars
    test.assert_cursor(2, 9);
}

// ============================================================================
// Editing operations with hints + wrap
// ============================================================================

#[test]
fn x_delete_char_with_hints() {
    let mut test = setup_wrapped("let x = 5;", 12);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 12);

    test.set_cursor(0, 4); // on 'x'
    test.keys("x");
    assert_eq!(test.line(0).unwrap().trim_end(), "let  = 5;");
    test.assert_cursor(0, 4);
}

#[test]
fn ciw_with_hints() {
    let mut test = setup_wrapped("let x = 5;", 12);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 12);

    test.set_cursor(0, 4); // on 'x'
    test.keys("ciwy<Esc>");
    assert_eq!(test.line(0).unwrap().trim_end(), "let y = 5;");
    test.assert_cursor(0, 4);
}

#[test]
fn yy_p_with_hints_causing_wrap() {
    let content = "let x = 5;\nlet y = 10;";
    let mut test = setup_wrapped(content, 12);
    add_hints(&mut test, vec![hint_at(5, ": i32")], 12);

    test.keys("yyp");
    assert_eq!(test.line(0).unwrap().trim_end(), "let x = 5;");
    assert_eq!(test.line(1).unwrap().trim_end(), "let x = 5;");
    assert_eq!(test.line(2).unwrap().trim_end(), "let y = 10;");
    test.assert_cursor(1, 0);
}
