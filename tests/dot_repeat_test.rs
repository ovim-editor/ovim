mod helpers;
use helpers::EditorTest;
use insta::assert_snapshot;

// ============================================================================
// Dot (.) command - Repeat last change
// ============================================================================

#[test]
fn test_dot_repeat_delete_char() {
    let mut test = EditorTest::new("hello");

    test.press('x')       // Delete 'h'
        .press('.');      // Repeat (delete 'e')

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_delete_word() {
    let mut test = EditorTest::new("one two three four");

    test.keys("dw")       // Delete "one "
        .press('.');      // Repeat (delete "two ")

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_delete_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("dd")       // Delete line 1
        .press('.');      // Repeat (delete line 2)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_insert() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('i')
        .type_text("PREFIX:")
        .press_esc()
        .press('j')       // Next line
        .press('.');      // Repeat insert

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_append() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('a')
        .type_text("!")
        .press_esc()
        .press('j')       // Next line
        .press('.');      // Repeat append

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_change_word() {
    let mut test = EditorTest::new("one two three");

    test.keys("ciw")      // Change "one"
        .type_text("X")
        .press_esc()
        .keys("w")        // Move to "two"
        .press('.');      // Repeat change

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_substitute() {
    let mut test = EditorTest::new("hello world");

    test.press('s')       // Substitute 'h'
        .type_text("H")
        .press_esc()
        .keys("w")        // Move to 'w'
        .press('.');      // Repeat substitute

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot with counts
// ============================================================================

#[test]
fn test_dot_with_count() {
    let mut test = EditorTest::new("abcdefgh");

    test.press('x')       // Delete one char
        .keys("3.");      // Repeat 3 times

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_original_count_vs_repeat_count() {
    let mut test = EditorTest::new("one two three four five six");

    test.keys("2dw")      // Delete 2 words
        .press('.');      // Repeat (should delete 2 more words)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_override_original_count() {
    let mut test = EditorTest::new("one two three four five six");

    test.keys("2dw")      // Delete 2 words
        .keys("3.");      // Repeat with different count

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_counted_insert() {
    let mut test = EditorTest::new("line");

    test.press('i')
        .type_text("X")
        .press_esc()
        .keys("3.");      // Repeat 3 times

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot with various operators
// ============================================================================

#[test]
fn test_dot_repeat_yank_then_change() {
    let mut test = EditorTest::new("one two three");

    test.keys("yiw")      // Yank doesn't count as change
        .keys("ciw")      // Change word
        .type_text("X")
        .press_esc()
        .keys("w")
        .press('.');      // Should repeat change, not yank

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_d_dollar() {
    let mut test = EditorTest::new("hello world\ntest case");

    test.keys("d$")       // Delete to end of line
        .press('j')       // Next line
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_c_dollar() {
    let mut test = EditorTest::new("hello world\ntest case");

    test.keys("c$")       // Change to end
        .type_text("NEW")
        .press_esc()
        .press('j')
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_upper_case_X() {
    let mut test = EditorTest::new("hello");

    test.keys("$")        // End
        .press('X')       // Delete char before
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_J_join() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('J')       // Join line 1 and 2
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot with text objects
// ============================================================================

#[test]
fn test_dot_repeat_diw() {
    let mut test = EditorTest::new("one two three four");

    test.keys("diw")      // Delete inner word
        .press('w')       // Move to next word
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_daw() {
    let mut test = EditorTest::new("one two three four");

    test.keys("daw")
        .press('.');      // Repeat on next word

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_ci_quote() {
    let mut test = EditorTest::new(r#""hello" and "world""#);

    test.keys("f\"")      // Find first quote
        .keys("ci\"")     // Change inside quotes
        .type_text("X")
        .press_esc()
        .keys("f\"")      // Find next quote
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_di_paren() {
    let mut test = EditorTest::new("func(arg1) and func(arg2)");

    test.keys("f(")
        .keys("di(")      // Delete inside parens
        .keys("f(")       // Next parens
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot with visual mode operations
// ============================================================================

#[test]
fn test_dot_after_visual_delete() {
    let mut test = EditorTest::new("hello world test");

    test.press('v')
        .keys("e")        // Select word
        .press('d')       // Delete
        .press('w')       // Move to next word
        .press('.');      // Repeat (should work?)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_after_visual_change() {
    let mut test = EditorTest::new("one two three");

    test.press('v')
        .keys("e")        // Select "one"
        .press('c')       // Change
        .type_text("X")
        .press_esc()
        .press('w')
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_after_visual_line_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V')       // Visual line
        .press('d')       // Delete
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot with motion variations
// ============================================================================

#[test]
fn test_dot_repeat_dw_different_positions() {
    let mut test = EditorTest::new("one two three four five");

    test.keys("dw")       // Delete "one "
        .keys("w")        // Move to "three"
        .press('.');      // Delete "three "

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_cw_at_different_word_lengths() {
    let mut test = EditorTest::new("a really long short");

    test.keys("cw")       // Change "a"
        .type_text("X")
        .press_esc()
        .keys("w")        // Move to "really" (longer word)
        .press('.');      // Repeat (should change "really")

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot repeat edge cases
// ============================================================================

#[test]
fn test_dot_without_previous_change() {
    let mut test = EditorTest::new("hello");

    test.press('.');      // No previous change

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_after_movement_only() {
    let mut test = EditorTest::new("hello world");

    test.keys("w")        // Just move
        .press('.');      // No change to repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_across_lines() {
    let mut test = EditorTest::new("hello\nworld\ntest");

    test.press('x')       // Delete char
        .press('j')       // Next line
        .press('.')       // Repeat
        .press('j')       // Next line
        .press('.');      // Repeat again

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_at_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$")        // End
        .press('x')       // Delete last char
        .press('.');      // Repeat (nothing to delete)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_after_failed_operation() {
    let mut test = EditorTest::new("x");

    test.press('x')       // Delete 'x'
        .press('.');      // Try to repeat (nothing to delete)

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot with insert mode variations
// ============================================================================

#[test]
fn test_dot_repeat_o_command() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('o')       // Open line below
        .type_text("new")
        .press_esc()
        .press('j')       // Move down
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_O_command() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('O')       // Open line above
        .type_text("new")
        .press_esc()
        .press('j')
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_A_command() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('A')       // Append at end
        .type_text("!")
        .press_esc()
        .press('j')
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_I_command() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('I')       // Insert at beginning
        .type_text("START:")
        .press_esc()
        .press('j')
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot with replace mode
// ============================================================================

#[test]
fn test_dot_repeat_r_command() {
    let mut test = EditorTest::new("hello world");

    test.press('r')       // Replace char
        .press('X')
        .press('l')       // Move right
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_R_command() {
    let mut test = EditorTest::new("hello\nworld");

    test.press('R')       // Replace mode
        .type_text("HI")
        .press_esc()
        .press('j')       // Next line
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Complex dot repeat scenarios
// ============================================================================

#[test]
fn test_dot_repeat_multiple_times() {
    let mut test = EditorTest::new("abcdefghijkl");

    test.press('x')       // Delete 'a'
        .press('.')       // Delete 'b'
        .press('.')       // Delete 'c'
        .press('.')       // Delete 'd'
        .press('.');      // Delete 'e'

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_changes_after_different_operation() {
    let mut test = EditorTest::new("one two three four");

    test.press('x')       // Delete char
        .keys("dw")       // Delete word (new change)
        .press('.');      // Should repeat dw, not x

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_with_undo() {
    let mut test = EditorTest::new("hello");

    test.press('x')       // Delete
        .press('u')       // Undo
        .press('.');      // Repeat (should work)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_after_undo_redo() {
    let mut test = EditorTest::new("hello");

    test.press('x')       // Delete 'h'
        .press('u')       // Undo
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Redo
        .press('.');      // Repeat

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot with search motions
// ============================================================================

#[test]
fn test_dot_with_search_motion() {
    let mut test = EditorTest::new("hello world hello test");

    test.keys("d/world")  // Delete to "world"
        .press_enter()
        .press('.');      // Repeat (delete to next match?)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_with_f_motion() {
    let mut test = EditorTest::new("a b c d e f");

    test.keys("dfc")      // Delete to 'c'
        .press('.');      // Repeat (delete to next 'c'?)

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Dot preserving count
// ============================================================================

#[test]
fn test_dot_preserves_original_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    test.keys("2dd")      // Delete 2 lines
        .press('.');      // Should delete 2 more lines

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_with_multichar_insert() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('i')
        .type_text("LONG TEXT ")
        .press_esc()
        .press('j')
        .press('.');      // Should insert same text

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_dot_repeat_complex_change() {
    let mut test = EditorTest::new("one two\nthree four\nfive six");

    test.keys("ciw")      // Change word
        .type_text("REPLACED")
        .press_esc()
        .press('j')       // Next line
        .press('w')       // Second word
        .press('.');      // Repeat change

    assert_snapshot!(test.snapshot_state());
}
