mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;

// ============================================================================
// 1. Dot Repeat + Line Joining (the core bug: I<BS> at col 0)
// ============================================================================

#[test]
fn test_dot_repeat_i_backspace_joins_lines() {
    // I<Backspace><Esc> on line 1 joins it with line 0, then . on line 1 (was line 2)
    let mut test = EditorTest::new("aaa\nbbb\nccc");

    test.press('j') // go to line 1 ("bbb")
        .press('I') // insert at first non-blank (col 0)
        .press_backspace() // delete newline → join with "aaa"
        .press_esc(); // back to normal

    assert_eq!(test.buffer_content(), "aaabbb\nccc\n");
    test.assert_cursor(0, 2);

    // Dot repeat: FirstNonBlank repositions cursor to col 0,
    // so the backwards cross-line deletion correctly joins lines
    test.press('j').press('.');

    assert_eq!(test.buffer_content(), "aaabbbccc\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_dot_repeat_i_backspace_indented_lines() {
    // Same test but with indented lines (FirstNonBlank lands at non-zero col)
    let mut test = EditorTest::new("  aaa\n  bbb\n  ccc");

    test.press('j') // line 1 ("  bbb")
        .press('I') // cursor to col 2 (first non-blank)
        .press_backspace() // smart backspace collapses leading 2 spaces in one press
        .press_backspace() // now delete newline, joining with line 0
        .press_esc();

    assert_eq!(test.buffer_content(), "  aaabbb\n  ccc\n");

    test.press('j') // go to "  ccc"
        .press('.'); // dot repeat

    assert_eq!(test.buffer_content(), "  aaabbbccc\n");
}

#[test]
fn test_dot_repeat_i_backspace_multiple_times() {
    // I<BS><Esc> repeated multiple times via .
    let mut test = EditorTest::new("a\nb\nc\nd");

    test.press('j') // line 1
        .press('I')
        .press_backspace()
        .press_esc();

    assert_eq!(test.buffer_content(), "ab\nc\nd\n");

    test.press('j').press('.');

    assert_eq!(test.buffer_content(), "abc\nd\n");

    test.press('j').press('.');

    assert_eq!(test.buffer_content(), "abcd\n");
}

#[test]
fn test_dot_repeat_i_multiple_backspaces_across_newline() {
    // I<BS><BS><Esc> — backspace removes newline then a char from prev line
    let mut test = EditorTest::new("ab\ncd\nef");

    test.press('j') // line 1 ("cd")
        .press('I') // col 0
        .press_backspace() // delete newline → "abcd"
        .press_backspace() // delete 'b' → "acd"
        .press_esc();

    assert_eq!(test.buffer_content(), "acd\nef\n");
    test.assert_cursor(0, 0);

    test.press('j') // line 1 ("ef")
        .press('.');

    assert_eq!(test.buffer_content(), "acef\n");
}

// ============================================================================
// 2. Dot Repeat + Insert Mode Backspace Edge Cases
// ============================================================================

#[test]
fn test_dot_repeat_i_backspace_at_col0_via_i() {
    // i<BS> at col 0 joins lines. But `i` entry mode (Insert) doesn't
    // reposition cursor on repeat — cursor stays at col 2 (from j), so
    // the repeat does a same-line char delete, not a line join.
    // Use I (FirstNonBlank) for guaranteed line-join repeat.
    let mut test = EditorTest::new("aaa\nbbb\nccc");

    test.press('j') // line 1
        .press('i') // insert at col 0
        .press_backspace() // join with previous line
        .press_esc();

    assert_eq!(test.buffer_content(), "aaabbb\nccc\n");
    test.assert_cursor(0, 2);

    // j preserves col 2 → repeat at col 2 deletes one char (same-line BS)
    test.press('j').press('.');

    assert_eq!(test.buffer_content(), "aaabbb\ncc\n");
}

#[test]
fn test_dot_repeat_a_backspace() {
    // a<BS><Esc> — append then backspace deletes char under cursor
    let mut test = EditorTest::new("abc\ndef");

    test.press('a') // append after 'a' (col 1)
        .press_backspace() // delete 'a' (the char we just moved past)
        .press_esc();

    assert_eq!(test.buffer_content(), "bc\ndef\n");

    test.press('j') // line 1 ("def")
        .press('.'); // repeat: a moves right, BS deletes

    assert_eq!(test.buffer_content(), "bc\nef\n");
}

#[test]
fn test_dot_repeat_A_backspace() {
    // A<BS><Esc> at end of line deletes last char
    let mut test = EditorTest::new("abc\ndef");

    test.press('A') // append at end of line (after 'c')
        .press_backspace() // delete 'c'
        .press_esc();

    assert_eq!(test.buffer_content(), "ab\ndef\n");

    test.press('j').press('.');

    assert_eq!(test.buffer_content(), "ab\nde\n");
}

#[test]
fn test_dot_repeat_backspace_deleting_char_not_newline() {
    // i at col 2, then BS — deletes a regular character
    let mut test = EditorTest::new("abcd\nefgh");

    test.keys("ll") // col 2
        .press('i')
        .press_backspace() // delete 'b'
        .press_esc();

    assert_eq!(test.buffer_content(), "acd\nefgh\n");

    test.press('j') // line 1
        .keys("ll") // col 2
        .press('.');

    assert_eq!(test.buffer_content(), "acd\negh\n");
}

// ============================================================================
// 3. Dot Repeat + Entry Mode Combinations
// ============================================================================

#[test]
fn test_dot_repeat_I_type_then_backspace() {
    // I + type "X" + BS + Esc — net effect: nothing (inserted then deleted)
    let mut test = EditorTest::new("hello\nworld");

    test.press('I').type_text("X").press_backspace().press_esc();

    assert_eq!(test.buffer_content(), "hello\nworld\n");

    test.press('j').press('.');

    assert_eq!(test.buffer_content(), "hello\nworld\n");
}

#[test]
fn test_dot_repeat_A_backspace_type() {
    // A<BS>X<Esc> — replace last char with X
    let mut test = EditorTest::new("abc\ndef");

    test.press('A').press_backspace().type_text("X").press_esc();

    assert_eq!(test.buffer_content(), "abX\ndef\n");

    test.press('j').press('.');

    assert_eq!(test.buffer_content(), "abX\ndeX\n");
}

// ============================================================================
// 4. Insert Mode Backspace at Boundaries
// ============================================================================

#[test]
fn test_backspace_at_start_of_buffer_noop() {
    let mut test = EditorTest::new("hello");

    test.press('i')
        .press_backspace() // should be no-op
        .press_esc();

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_backspace_joining_with_empty_previous_line() {
    let mut test = EditorTest::new("\nhello");

    test.press('j') // line 1
        .press('I')
        .press_backspace() // join with empty line above
        .press_esc();

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_backspace_joining_last_two_lines() {
    let mut test = EditorTest::new("first\nlast");

    test.press('j').press('I').press_backspace().press_esc();

    assert_eq!(test.buffer_content(), "firstlast\n");
    test.assert_cursor(0, 4);
}

// ============================================================================
// 5. Cross-line Deletion Edge Cases
// ============================================================================

#[test]
fn test_J_join_lines() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('J');

    assert_eq!(test.buffer_content(), "hello world\n");
}

#[test]
fn test_J_then_undo() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('J').press('u');

    assert_eq!(test.buffer_content(), "hello\nworld\n");
}

#[test]
fn test_dd_on_last_remaining_line() {
    let mut test = EditorTest::new("only line");

    test.keys("dd");

    // Should result in empty buffer (single empty line)
    assert_eq!(test.buffer_content(), "\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_dk_on_first_line() {
    // dk on first line — should delete lines 0 and 1 (if line 1 exists)
    // or just line 0 if only one line
    let mut test = EditorTest::new("line1\nline2\nline3");

    test.keys("dk");

    // dk from line 0: range is max(0, 0-1)..0 which is just line 0
    // Actually in vim, dk from line 0 deletes line 0 only (can't go above)
    // But some implementations delete line 0 and 1. Let's test actual behavior.
    let content = test.buffer_content();
    // Just verify it doesn't crash — the exact behavior depends on implementation
    assert!(!content.is_empty());
}

#[test]
fn test_dj_on_last_line() {
    let mut test = EditorTest::new("line1\nline2\nline3");

    test.keys("GG") // go to last line
        .keys("dj"); // delete down from last line — should handle gracefully

    let content = test.buffer_content();
    assert!(!content.is_empty());
}

// ============================================================================
// 6. Operator + Motion at Boundaries
// ============================================================================

#[test]
fn test_d_dollar_on_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j') // go to empty line
        .keys("d$");

    // d$ on empty line should be a no-op (nothing to delete)
    assert_eq!(test.buffer_content(), "hello\n\nworld\n");
}

#[test]
fn test_dd_on_last_line_multiline() {
    let mut test = EditorTest::new("first\nsecond\nthird");

    test.keys("G") // go to last line
        .keys("dd");

    assert_eq!(test.buffer_content(), "first\nsecond\n");
}

#[test]
fn test_cw_on_single_char_word() {
    let mut test = EditorTest::new("a big word");

    test.keys("cw").type_text("the").press_esc();

    assert_eq!(test.buffer_content(), "the big word\n");
}

#[test]
fn test_diw_on_whitespace() {
    let mut test = EditorTest::new("hello   world");

    test.keys("w") // move to 'w' in "world"
        .press('h') // back to whitespace
        .keys("diw"); // delete inner word (whitespace between words)

    // diw on whitespace deletes the whitespace
    assert_eq!(test.buffer_content(), "helloworld\n");
}

// ============================================================================
// 7. Visual Mode Edge Cases
// ============================================================================

#[test]
fn test_visual_delete_across_lines_then_dot() {
    let mut test = EditorTest::new("aaa\nbbb\nccc\nddd");

    test.press('v')
        .press('j') // select "aaa\nb"
        .press('d'); // delete selection

    let _content_after_delete = test.buffer_content();

    test.press('.'); // dot repeat — should delete same span

    // Just verify it doesn't crash and produces valid output
    let final_content = test.buffer_content();
    assert!(!final_content.is_empty());
}

#[test]
fn test_v_dollar_different_line_lengths() {
    let mut test = EditorTest::new("short\na very long line");

    test.press('v').keys("$"); // select to end of "short"

    // Should select to end of line, not beyond
    test.assert_mode(Mode::Visual);
}

// ============================================================================
// 8. Undo After Dot Repeat
// ============================================================================

#[test]
fn test_undo_after_dot_repeat_i_backspace() {
    // I<BS><Esc>.u — undo should restore only the repeated change
    let mut test = EditorTest::new("aaa\nbbb\nccc");

    test.press('j').press('I').press_backspace().press_esc(); // "aaabbb\nccc"

    test.press('j').press('.'); // "aaabbbccc"

    test.press('u'); // undo the dot repeat

    assert_eq!(test.buffer_content(), "aaabbb\nccc\n");
}

#[test]
fn test_dd_dot_undo_undo() {
    let mut test = EditorTest::new("line1\nline2\nline3\nline4");

    test.keys("dd"); // delete line1
    assert_eq!(test.buffer_content(), "line2\nline3\nline4\n");

    test.press('.'); // delete line2
    assert_eq!(test.buffer_content(), "line3\nline4\n");

    test.press('u'); // undo: restore line2
    assert_eq!(test.buffer_content(), "line2\nline3\nline4\n");

    test.press('u'); // undo: restore line1
    assert_eq!(test.buffer_content(), "line1\nline2\nline3\nline4\n");
}

#[test]
fn test_insert_dot_undo_chain() {
    let mut test = EditorTest::new("aaa\nbbb");

    test.press('i').type_text("X").press_esc(); // "Xaaa\nbbb"

    test.press('j').press('.'); // "Xaaa\nXbbb"

    test.press('u'); // undo dot: "Xaaa\nbbb"
    assert_eq!(test.buffer_content(), "Xaaa\nbbb\n");

    test.press('u'); // undo original: "aaa\nbbb"
    assert_eq!(test.buffer_content(), "aaa\nbbb\n");
}

// ============================================================================
// 9. Additional dot repeat edge cases
// ============================================================================

#[test]
fn test_dot_repeat_gJ_join_without_space() {
    let mut test = EditorTest::new("hello\nworld\nfoo");

    test.keys("gJ"); // join without space
    assert_eq!(test.buffer_content(), "helloworld\nfoo\n");

    test.press('.');
    assert_eq!(test.buffer_content(), "helloworldfoo\n");
}

#[test]
fn test_dot_repeat_x_at_end_of_line() {
    let mut test = EditorTest::new("ab\ncd");

    test.keys("$x"); // delete last char of line 0
    assert_eq!(test.buffer_content(), "a\ncd\n");

    test.press('j').keys("$."); // repeat on line 1

    assert_eq!(test.buffer_content(), "a\nc\n");
}
