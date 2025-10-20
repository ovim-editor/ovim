mod helpers;
use helpers::EditorTest;

// ============================================================================
// '/' command - Forward search
// ============================================================================

#[test]
fn test_forward_search_basic() {
    let mut test = EditorTest::new("hello world hello");

    test.press('/').type_text("world").press_enter();

    assert_eq!(test.buffer_content(), "hello world hello\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_forward_search_from_middle() {
    let mut test = EditorTest::new("hello world hello test");

    test.keys("w") // Move to "world"
        .press('/')
        .type_text("hello")
        .press_enter(); // Should find second "hello"

    assert_eq!(test.buffer_content(), "hello world hello test\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_forward_search_wrap_around() {
    let mut test = EditorTest::new("start middle end");

    test.keys("$") // Go to end
        .press('/')
        .type_text("start")
        .press_enter(); // Should wrap to beginning

    assert_eq!(test.buffer_content(), "start middle end\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_forward_search_multiline() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\ntarget here");

    test.press('/').type_text("target").press_enter();

    assert_eq!(
        test.buffer_content(),
        "line 1\nline 2\nline 3\ntarget here\n"
    );
    test.assert_cursor(3, 0);
}

#[test]
fn test_forward_search_not_found() {
    let mut test = EditorTest::new("hello world");

    test.press('/').type_text("nothere").press_enter(); // Should not move cursor

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_forward_search_regex() {
    let mut test = EditorTest::new("test123 hello test456");

    test.press('/').type_text("test[0-9]+").press_enter();

    assert_eq!(test.buffer_content(), "test123 hello test456\n");
    test.assert_cursor(0, 14);
}

#[test]
fn test_forward_search_case_sensitive() {
    let mut test = EditorTest::new("hello Hello HELLO");

    test.press('/').type_text("Hello").press_enter();

    assert_eq!(test.buffer_content(), "hello Hello HELLO\n");
    test.assert_cursor(0, 6);
}

// ============================================================================
// '?' command - Backward search
// ============================================================================

#[test]
fn test_backward_search_basic() {
    let mut test = EditorTest::new("hello world hello");

    test.keys("$") // Go to end
        .press('?')
        .type_text("hello")
        .press_enter(); // Should find second "hello"

    assert_eq!(test.buffer_content(), "hello world hello\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_backward_search_from_middle() {
    let mut test = EditorTest::new("hello world hello test");

    test.keys("$").press('?').type_text("world").press_enter();

    assert_eq!(test.buffer_content(), "hello world hello test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_backward_search_wrap_around() {
    let mut test = EditorTest::new("start middle end");

    test.press('?') // At beginning
        .type_text("end")
        .press_enter(); // Should wrap to end

    assert_eq!(test.buffer_content(), "start middle end\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_backward_search_multiline() {
    let mut test = EditorTest::new("target here\nline 2\nline 3\nline 4");

    test.keys("G") // Go to last line
        .press('?')
        .type_text("target")
        .press_enter();

    assert_eq!(
        test.buffer_content(),
        "target here\nline 2\nline 3\nline 4\n"
    );
    test.assert_cursor(0, 0);
}

// ============================================================================
// 'n' command - Repeat search forward
// ============================================================================

#[test]
fn test_n_after_forward_search() {
    let mut test = EditorTest::new("hello world hello test hello");

    test.press('/')
        .type_text("hello")
        .press_enter() // First match
        .press('n'); // Next match

    assert_eq!(test.buffer_content(), "hello world hello test hello\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_n_multiple_times() {
    let mut test = EditorTest::new("a b a c a d");

    test.press('/')
        .type_text("a")
        .press_enter()
        .press('n')
        .press('n'); // Third 'a'

    assert_eq!(test.buffer_content(), "a b a c a d\n");
    test.assert_cursor(0, 8);
}

#[test]
fn test_n_wrap_around() {
    let mut test = EditorTest::new("hello world hello");

    test.press('/')
        .type_text("hello")
        .press_enter() // First
        .press('n') // Second
        .press('n'); // Should wrap to first again

    assert_eq!(test.buffer_content(), "hello world hello\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_n_after_backward_search() {
    let mut test = EditorTest::new("hello world hello test");

    test.keys("$")
        .press('?')
        .type_text("hello")
        .press_enter() // Second hello
        .press('n'); // Continue backward to first

    assert_eq!(test.buffer_content(), "hello world hello test\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// 'N' command - Repeat search backward
// ============================================================================

#[test]
fn test_N_after_forward_search() {
    let mut test = EditorTest::new("hello world hello test hello");

    test.press('/')
        .type_text("hello")
        .press_enter() // First
        .press('n') // Second
        .press('N'); // Back to first

    assert_eq!(test.buffer_content(), "hello world hello test hello\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_N_after_backward_search() {
    let mut test = EditorTest::new("hello world hello test");

    test.keys("$")
        .press('?')
        .type_text("hello")
        .press_enter()
        .press('N'); // Reverse direction (forward)

    assert_eq!(test.buffer_content(), "hello world hello test\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_N_wrap_around() {
    let mut test = EditorTest::new("hello world hello");

    test.press('/').type_text("hello").press_enter().press('N'); // Backward wrap to last

    assert_eq!(test.buffer_content(), "hello world hello\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Search with operators
// ============================================================================

#[test]
fn test_delete_to_search() {
    let mut test = EditorTest::new("hello world test");

    test.keys("d/test").press_enter();

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_change_to_search() {
    let mut test = EditorTest::new("hello world test");

    test.keys("c/test").press_enter().type_text("X").press_esc();

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_yank_to_search() {
    let mut test = EditorTest::new("hello world test");

    test.keys("y/test").press_enter().keys("$").press('p');

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 15);
}

// ============================================================================
// Search highlighting (if implemented)
// ============================================================================

#[test]
fn test_search_shows_all_matches() {
    let mut test = EditorTest::new("hello world hello test hello");

    test.press('/').type_text("hello").press_enter();

    // All "hello" instances should be highlighted
    assert_eq!(test.buffer_content(), "hello world hello test hello\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_noh_clears_highlight() {
    let mut test = EditorTest::new("hello world hello");

    test.press('/')
        .type_text("hello")
        .press_enter()
        .press(':')
        .type_text("noh")
        .press_enter();

    assert_eq!(test.buffer_content(), "hello world hello\n");
    test.assert_cursor(0, 12);
}

// ============================================================================
// Search edge cases
// ============================================================================

#[test]
fn test_search_empty_pattern() {
    let mut test = EditorTest::new("hello world");

    test.press('/').press_enter(); // Empty search

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_search_cancel_with_esc() {
    let mut test = EditorTest::new("hello world");

    test.press('/').type_text("world").press_esc(); // Cancel search

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_search_single_char() {
    let mut test = EditorTest::new("a b c d e");

    test.press('/').type_text("c").press_enter();

    assert_eq!(test.buffer_content(), "a b c d e\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_search_at_end_of_line() {
    let mut test = EditorTest::new("hello world");

    test.press('/')
        .type_text("d") // Last char
        .press_enter();

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_search_special_chars() {
    let mut test = EditorTest::new("hello.world test.case");

    test.press('/')
        .type_text("\\.") // Escaped dot
        .press_enter();

    assert_eq!(test.buffer_content(), "hello.world test.case\n");
    test.assert_cursor(0, 5);
}

// ============================================================================
// Star (*) and hash (#) - Search word under cursor
// ============================================================================

#[test]
fn test_star_search_word_forward() {
    let mut test = EditorTest::new("hello world hello test");

    test.press('*'); // Search for "hello" forward

    assert_eq!(test.buffer_content(), "hello world hello test\n");
    test.assert_cursor(0, 12);
}

#[test]
fn test_star_multiple_matches() {
    let mut test = EditorTest::new("test one test two test");

    test.press('*') // First to second
        .press('n'); // Second to third

    assert_eq!(test.buffer_content(), "test one test two test\n");
    test.assert_cursor(0, 18);
}

#[test]
fn test_hash_search_word_backward() {
    let mut test = EditorTest::new("hello world hello test");

    test.keys("$") // Go to end
        .press('#'); // Search backward for "test"

    assert_eq!(test.buffer_content(), "hello world hello test\n");
    test.assert_cursor(0, 21);
}

// ============================================================================
// Search in visual mode
// ============================================================================

#[test]
fn test_search_in_visual_mode() {
    let mut test = EditorTest::new("hello world test");

    test.press('v')
        .press('/') // Start search in visual
        .type_text("test")
        .press_enter();

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 4);
}

// ============================================================================
// Search with multiline patterns
// ============================================================================

#[test]
fn test_search_across_lines() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('/').type_text("world").press_enter();

    assert_eq!(test.buffer_content(), "hello\nworld\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_search_beginning_of_line() {
    let mut test = EditorTest::new("  hello\n  world\ntest");

    test.press('/')
        .type_text("^test") // Start of line
        .press_enter();

    assert_eq!(test.buffer_content(), "  hello\n  world\ntest\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_search_end_of_line() {
    let mut test = EditorTest::new("hello world\ntest case\nend");

    test.press('/').type_text("world$").press_enter();

    assert_eq!(test.buffer_content(), "hello world\ntest case\nend\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Search history (if implemented)
// ============================================================================

#[test]
fn test_search_history_up() {
    let mut test = EditorTest::new("hello world");

    test.press('/')
        .type_text("hello")
        .press_enter()
        .press('/') // New search
        .press_key(crossterm::event::KeyCode::Up); // Should recall "hello"

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// f/F/t/T commands - Character search on line
// ============================================================================

#[test]
fn test_f_find_char() {
    let mut test = EditorTest::new("hello world");

    test.press('f').press('w'); // Find 'w'

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_f_multiple_occurrences() {
    let mut test = EditorTest::new("a b a c a d");

    test.press('f').press('a'); // Find first 'a' after cursor

    assert_eq!(test.buffer_content(), "a b a c a d\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_f_not_found() {
    let mut test = EditorTest::new("hello world");

    test.press('f').press('x'); // Not found

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_F_find_backward() {
    let mut test = EditorTest::new("hello world");

    test.keys("$") // End of line
        .press('F')
        .press('h'); // Find 'h' backward

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_t_till_char() {
    let mut test = EditorTest::new("hello world");

    test.press('t').press('w'); // Till 'w' (one char before)

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_T_till_backward() {
    let mut test = EditorTest::new("hello world");

    test.keys("$").press('T').press('h'); // Till 'h' backward

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 1);
}

// ============================================================================
// Semicolon and comma - Repeat f/F/t/T
// ============================================================================

#[test]
fn test_semicolon_repeat_f() {
    let mut test = EditorTest::new("a b a c a d");

    test.press('f')
        .press('a') // Find first 'a'
        .press(';') // Repeat
        .press(';'); // Repeat again

    assert_eq!(test.buffer_content(), "a b a c a d\n");
    test.assert_cursor(0, 8);
}

#[test]
fn test_comma_reverse_f() {
    let mut test = EditorTest::new("a b a c a d");

    test.press('f')
        .press('a')
        .press(';') // Forward
        .press(','); // Reverse direction

    assert_eq!(test.buffer_content(), "a b a c a d\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_semicolon_with_t() {
    let mut test = EditorTest::new("a b a c a d");

    test.press('t').press('a').press(';').press(';');

    assert_eq!(test.buffer_content(), "a b a c a d\n");
    test.assert_cursor(0, 3);
}

// ============================================================================
// Delete/change with f/F/t/T
// ============================================================================

#[test]
fn test_df_delete_to_char() {
    let mut test = EditorTest::new("hello world");

    test.keys("dfw"); // Delete to 'w'

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_dt_delete_till_char() {
    let mut test = EditorTest::new("hello world");

    test.keys("dtw"); // Delete till 'w'

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_cf_change_to_char() {
    let mut test = EditorTest::new("hello world");

    test.keys("cfw").type_text("X").press_esc();

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_ct_change_till_char() {
    let mut test = EditorTest::new("hello world");

    test.keys("ctw").type_text("goodbye ").press_esc();

    assert_eq!(test.buffer_content(), "hello world\nodbye \n");
    test.assert_cursor(1, 5);
}
