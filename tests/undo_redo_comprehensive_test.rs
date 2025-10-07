mod helpers;
use helpers::EditorTest;
use insta::assert_snapshot;

// ============================================================================
// Basic undo/redo
// ============================================================================

#[test]
fn test_undo_single_change() {
    let mut test = EditorTest::new("hello");

    test.press('x')       // Delete
        .press('u');      // Undo

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_redo_single_change() {
    let mut test = EditorTest::new("hello");

    test.press('x')       // Delete
        .press('u')       // Undo
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        );                // Redo

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_multiple_changes() {
    let mut test = EditorTest::new("hello");

    test.press('x')       // Delete 'h'
        .press('x')       // Delete 'e'
        .press('x')       // Delete 'l'
        .press('u')       // Undo last
        .press('u');      // Undo second

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_redo_multiple_changes() {
    let mut test = EditorTest::new("hello");

    test.press('x')
        .press('x')
        .press('u')
        .press('u')
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        )
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        );

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo tree branches
// ============================================================================

#[test]
fn test_undo_branch_new_change() {
    let mut test = EditorTest::new("hello");

    test.press('x')       // Delete 'h' (change 1)
        .press('u')       // Undo
        .press('x')       // Delete 'h' again (change 2 - creates branch)
        .press('x');      // Delete 'e' (change 3)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_redo_branch() {
    let mut test = EditorTest::new("hello world");

    test.press('x')       // Change 1
        .press('x')       // Change 2
        .press('u')       // Undo change 2
        .keys("dw")       // New branch
        .press('u')       // Undo dw
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        );                // Redo dw

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo with different operation types
// ============================================================================

#[test]
fn test_undo_insert() {
    let mut test = EditorTest::new("hello");

    test.press('i')
        .type_text("START ")
        .press_esc()
        .press('u');      // Undo insert

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_append() {
    let mut test = EditorTest::new("hello");

    test.press('a')
        .type_text(" END")
        .press_esc()
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_delete_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("dw")
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_delete_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("dd")
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_change_word() {
    let mut test = EditorTest::new("hello world");

    test.keys("ciw")
        .type_text("goodbye")
        .press_esc()
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_visual_delete() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("e")
        .press('d')
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_line_join() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('J')
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with paste
// ============================================================================

#[test]
fn test_undo_paste() {
    let mut test = EditorTest::new("hello world");

    test.keys("yiw")      // Yank
        .keys("$")
        .press('p')       // Paste
        .press('u');      // Undo paste

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_paste_before() {
    let mut test = EditorTest::new("hello world");

    test.keys("yiw")
        .keys("$")
        .press('P')       // Paste before
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_redo_paste() {
    let mut test = EditorTest::new("hello world");

    test.keys("yiw")
        .keys("$")
        .press('p')
        .press('u')
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        );

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with o and O commands
// ============================================================================

#[test]
fn test_undo_o_command() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('o')
        .type_text("new line")
        .press_esc()
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_O_command() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('O')
        .type_text("new line")
        .press_esc()
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with complex sequences
// ============================================================================

#[test]
fn test_undo_sequence_of_different_operations() {
    let mut test = EditorTest::new("hello world");

    test.press('x')       // Delete char
        .keys("dw")       // Delete word
        .press('i')
        .type_text("NEW ")
        .press_esc()
        .press('u')       // Undo insert
        .press('u')       // Undo dw
        .press('u');      // Undo x

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_redo_sequence() {
    let mut test = EditorTest::new("hello");

    test.press('x')
        .press('x')
        .press('x')
        .press('u')
        .press('u')
        .press('u')
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        )
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        )
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        );

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo at limits
// ============================================================================

#[test]
fn test_undo_at_beginning() {
    let mut test = EditorTest::new("hello");

    test.press('u');      // Nothing to undo

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_redo_at_end() {
    let mut test = EditorTest::new("hello");

    test.press_with(
        crossterm::event::KeyCode::Char('r'),
        crossterm::event::KeyModifiers::CONTROL
    );                    // Nothing to redo

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_all_then_redo_all() {
    let mut test = EditorTest::new("hello");

    test.press('x')
        .press('x')
        .press('x')
        .press('u')
        .press('u')
        .press('u')       // Undo all
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        )
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        )
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        );                // Redo all

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with counts
// ============================================================================

#[test]
fn test_undo_with_count() {
    let mut test = EditorTest::new("hello");

    test.press('x')
        .press('x')
        .press('x')
        .keys("3u");      // Undo 3 times

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_redo_with_count() {
    let mut test = EditorTest::new("hello");

    test.press('x')
        .press('x')
        .press('x')
        .keys("3u")
        .keys("3")
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        );                // Redo 3 times

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_count_exceeds_history() {
    let mut test = EditorTest::new("hello");

    test.press('x')
        .press('x')
        .keys("99u");     // Try to undo 99 times

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with visual mode
// ============================================================================

#[test]
fn test_undo_visual_change() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("e")
        .press('c')
        .type_text("HI")
        .press_esc()
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_visual_line_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j')
        .press('d')
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with macros
// ============================================================================

#[test]
fn test_undo_after_macro() {
    let mut test = EditorTest::new("hello\nworld\ntest");

    test.press('q')
        .press('a')
        .press('x')       // Record delete
        .press('j')
        .press('q')
        .press('@')
        .press('a')       // Play macro
        .press('u');      // Undo macro

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_macro_multiple_times() {
    let mut test = EditorTest::new("abc\ndef\nghi");

    test.press('q')
        .press('a')
        .press('x')
        .press('j')
        .press('q')
        .keys("3@a")      // Play 3 times
        .press('u');      // Should undo all 3?

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with text objects
// ============================================================================

#[test]
fn test_undo_diw() {
    let mut test = EditorTest::new("hello world");

    test.keys("diw")
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_daw() {
    let mut test = EditorTest::new("hello world");

    test.keys("daw")
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_ci_quote() {
    let mut test = EditorTest::new(r#""hello world""#);

    test.keys("f\"")
        .keys("ci\"")
        .type_text("goodbye")
        .press_esc()
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo state after movements
// ============================================================================

#[test]
fn test_undo_after_movement() {
    let mut test = EditorTest::new("hello world");

    test.press('x')       // Change
        .keys("w")        // Move
        .press('u');      // Should undo x, not movement

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_after_search() {
    let mut test = EditorTest::new("hello world hello");

    test.press('x')
        .press('/')
        .type_text("world")
        .press_enter()
        .press('u');      // Should undo x

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with marks
// ============================================================================

#[test]
fn test_undo_preserves_marks() {
    let mut test = EditorTest::new("hello world");

    test.press('m')
        .press('a')       // Set mark
        .press('x')       // Change
        .press('u')       // Undo change
        .press('`')
        .press('a');      // Jump to mark (should still work)

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with replace mode
// ============================================================================

#[test]
fn test_undo_replace_char() {
    let mut test = EditorTest::new("hello");

    test.press('r')
        .press('X')
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_replace_mode() {
    let mut test = EditorTest::new("hello world");

    test.press('R')
        .type_text("HELLO")
        .press_esc()
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with indentation
// ============================================================================

#[test]
fn test_undo_indent() {
    let mut test = EditorTest::new("hello");

    test.keys(">>")       // Indent
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_dedent() {
    let mut test = EditorTest::new("    hello");

    test.keys("<<")       // Dedent
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_visual_indent() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j')
        .press('>')
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo granularity
// ============================================================================

#[test]
fn test_insert_mode_undo_granularity() {
    let mut test = EditorTest::new("hello");

    test.press('i')
        .type_text("one")
        .press(' ')       // Space might break undo
        .type_text("two")
        .press_esc()
        .press('u');      // Should undo entire insert?

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_multiple_insert_sessions() {
    let mut test = EditorTest::new("hello");

    test.press('i')
        .type_text("A")
        .press_esc()
        .press('i')
        .type_text("B")
        .press_esc()
        .press('u')       // Undo B
        .press('u');      // Undo A

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Undo/redo with search and replace
// ============================================================================

#[test]
fn test_undo_substitute() {
    let mut test = EditorTest::new("hello world");

    test.press(':')
        .type_text("s/hello/goodbye/")
        .press_enter()
        .press('u');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// U command - Undo all changes on line
// ============================================================================

#[test]
fn test_U_undo_line() {
    let mut test = EditorTest::new("hello world");

    test.press('x')       // Change 1
        .press('x')       // Change 2
        .press('x')       // Change 3
        .press('U');      // Undo all changes on line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_U_different_line() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('x')       // Change on line 1
        .press('j')       // Move to line 2
        .press('x')       // Change on line 2
        .press('U');      // Should only undo line 2

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_U_after_leaving_line() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('x')       // Change line 1
        .press('j')       // Leave line
        .press('k')       // Return to line 1
        .press('U');      // Should still work?

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Redo variations
// ============================================================================

#[test]
fn test_ctrl_r_redo() {
    let mut test = EditorTest::new("hello");

    test.press('x')
        .press('u')
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        );

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Complex undo/redo scenarios
// ============================================================================

#[test]
fn test_complex_undo_redo_sequence() {
    let mut test = EditorTest::new("one two three four");

    test.press('x')       // 1. Delete 'o'
        .keys("dw")       // 2. Delete "ne "
        .press('i')       // 3. Insert
        .type_text("START ")
        .press_esc()
        .press('u')       // Undo 3
        .press('u')       // Undo 2
        .keys("ciw")      // New change (branches)
        .type_text("NEW")
        .press_esc()
        .press('u')       // Undo new change
        .press_with(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL
        );                // Redo

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_undo_after_dot_repeat() {
    let mut test = EditorTest::new("hello world test");

    test.press('x')       // Delete
        .press('w')
        .press('.')       // Repeat
        .press('u')       // Undo repeat
        .press('u');      // Undo original

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_very_long_undo_history() {
    let mut test = EditorTest::new(&"x".repeat(50));

    // Make 50 changes
    for _ in 0..50 {
        test.press('x');
    }

    // Undo 50 times
    for _ in 0..50 {
        test.press('u');
    }

    assert_snapshot!(test.snapshot_state());
}
