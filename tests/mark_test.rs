mod helpers;
use helpers::EditorTest;
use insta::assert_snapshot;

// ============================================================================
// 'm' command - Set mark
// ============================================================================

#[test]
fn test_m_set_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("w")        // Move to position
        .press('m')       // Set mark
        .press('a');      // Mark 'a'

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_m_set_multiple_marks() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('m')
        .press('a')       // Mark 'a' at line 0
        .press('j')
        .press('m')
        .press('b')       // Mark 'b' at line 1
        .keys("jj")
        .press('m')
        .press('c');      // Mark 'c' at line 3

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_m_overwrite_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('m')
        .press('a')       // Set mark 'a'
        .keys("jj")       // Move
        .press('m')
        .press('a');      // Overwrite mark 'a'

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// '`' command - Jump to mark (exact position)
// ============================================================================

#[test]
fn test_backtick_jump_to_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("w")        // Move to column 5
        .press('m')
        .press('a')       // Set mark
        .keys("G")        // Go to end
        .press('`')       // Jump to mark
        .press('a');      // Mark 'a'

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_backtick_exact_position() {
    let mut test = EditorTest::new("hello world test");

    test.keys("www")      // Move to "test"
        .press('m')
        .press('a')
        .keys("gg")       // Go to beginning
        .press('`')
        .press('a');      // Should return to "test"

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_backtick_multiple_marks() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    // Set marks
    test.press('m').press('a')
        .keys("jw").press('m').press('b')
        .keys("jw").press('m').press('c');

    // Jump around
    test.press('`').press('a')  // Back to mark a
        .press('`').press('c')  // To mark c
        .press('`').press('b'); // To mark b

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// "'" command - Jump to mark (beginning of line)
// ============================================================================

#[test]
fn test_quote_jump_to_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("jw")       // Line 1, some column
        .press('m')
        .press('a')
        .keys("G")        // Go to end
        .press('\'')      // Jump to line of mark
        .press('a');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_quote_vs_backtick() {
    let mut test = EditorTest::new("hello world\ntest line");

    test.keys("w")        // Move to "world"
        .press('m')
        .press('a')
        .keys("G")        // Go to last line
        .press('\'')      // ' jumps to first non-blank
        .press('a');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Special marks
// ============================================================================

#[test]
fn test_backtick_backtick_previous_position() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G")        // Go to line 4
        .keys("gg")       // Go back to line 1
        .press('`')
        .press('`');      // Jump to previous position (line 4)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_quote_quote_previous_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("G")
        .keys("gg")
        .press('\'')
        .press('\'');     // Jump to previous line

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_backtick_dot_last_change() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('i')
        .type_text("CHANGED ")
        .press_esc()
        .keys("G")        // Move away
        .press('`')
        .press('.');      // Jump to last change position

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_backtick_bracket_last_yank() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("yy")       // Yank line
        .keys("G")        // Move away
        .press('`')
        .press('[');      // Jump to start of last yank

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_backtick_caret_insert_exit() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('i')
        .type_text("text")
        .press_esc()
        .keys("G")        // Move away
        .press('`')
        .press('^');      // Jump to last insert position

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Marks with operations
// ============================================================================

#[test]
fn test_delete_to_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("jj")       // Line 2
        .press('m')
        .press('a')
        .keys("gg")       // Go to top
        .keys("d`a");     // Delete to mark

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_yank_to_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("j")
        .press('m')
        .press('a')
        .keys("gg")
        .keys("y`a")      // Yank to mark
        .keys("G")
        .press('p');      // Paste

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_change_to_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("j")
        .press('m')
        .press('a')
        .keys("gg")
        .keys("c`a")      // Change to mark
        .type_text("CHANGED")
        .press_esc();

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_visual_to_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("jj")
        .press('m')
        .press('a')
        .keys("gg")
        .press('v')       // Visual mode
        .press('`')
        .press('a');      // Select to mark

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Jump list with Ctrl-O and Ctrl-I
// ============================================================================

#[test]
fn test_ctrl_o_jump_back() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G")        // Jump 1
        .keys("gg")       // Jump 2
        .keys("G")        // Jump 3
        .press_with(
            crossterm::event::KeyCode::Char('o'),
            crossterm::event::KeyModifiers::CONTROL
        );                // Jump back

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_ctrl_i_jump_forward() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G")
        .keys("gg")
        .press_with(
            crossterm::event::KeyCode::Char('o'),
            crossterm::event::KeyModifiers::CONTROL
        )                 // Back
        .press_with(
            crossterm::event::KeyCode::Char('i'),
            crossterm::event::KeyModifiers::CONTROL
        );                // Forward

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_jump_list_multiple() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    // Create jump list
    test.keys("j")
        .keys("j")
        .keys("j")
        .keys("j");

    // Jump back multiple times
    test.press_with(
        crossterm::event::KeyCode::Char('o'),
        crossterm::event::KeyModifiers::CONTROL
    )
    .press_with(
        crossterm::event::KeyCode::Char('o'),
        crossterm::event::KeyModifiers::CONTROL
    );

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Marks in different contexts
// ============================================================================

#[test]
fn test_mark_after_insert() {
    let mut test = EditorTest::new("line 1");

    test.press('i')
        .type_text("INSERTED ")
        .press_esc()
        .press('m')
        .press('a')       // Mark after insert
        .keys("$")        // Move away
        .press('`')
        .press('a');      // Jump back

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_mark_after_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("dd")       // Delete line
        .press('m')
        .press('a')
        .keys("j")
        .press('`')
        .press('a');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_mark_in_visual_mode() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('v')
        .keys("j")        // Visual selection
        .press('m')       // Try to set mark in visual mode
        .press('a');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_jump_to_nonexistent_mark() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('`')
        .press('z');      // Jump to mark that doesn't exist

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_mark_on_empty_line() {
    let mut test = EditorTest::new("line 1\n\nline 3");

    test.press('j')       // Move to empty line
        .press('m')
        .press('a')
        .keys("k")
        .press('`')
        .press('a');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_mark_at_eof() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("G$")       // End of file
        .press('m')
        .press('a')
        .keys("gg")
        .press('`')
        .press('a');

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_all_lowercase_marks() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    // Set marks a-z (just a few)
    test.press('m').press('a')
        .keys("j").press('m').press('b')
        .keys("j").press('m').press('c')
        .press('m').press('z');  // Mark z

    // Jump to various marks
    test.press('`').press('a')
        .press('`').press('z')
        .press('`').press('b');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Marks with undo/redo
// ============================================================================

#[test]
fn test_mark_survives_undo() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('m')
        .press('a')       // Set mark
        .press('i')
        .type_text("text")
        .press_esc()
        .press('u')       // Undo
        .press('`')
        .press('a');      // Jump to mark (should still exist)

    assert_snapshot!(test.snapshot_state());
}

#[test]
fn test_mark_after_line_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("jj")       // Line 2
        .press('m')
        .press('a')       // Mark at line 2
        .keys("gg")
        .keys("dd")       // Delete line 0
        .press('`')
        .press('a');      // Mark should adjust?

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// List all marks
// ============================================================================

#[test]
fn test_marks_command() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('m').press('a')
        .keys("j").press('m').press('b')
        .keys("j").press('m').press('c')
        .press(':')
        .type_text("marks")
        .press_enter();

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Delete marks
// ============================================================================

#[test]
fn test_delmarks() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('m').press('a')
        .press(':')
        .type_text("delmarks a")
        .press_enter()
        .press('`')
        .press('a');      // Should fail - mark deleted

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Global marks (uppercase A-Z)
// ============================================================================

#[test]
fn test_global_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('m')
        .press('A')       // Global mark (uppercase)
        .keys("G")
        .press('`')
        .press('A');

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Marks with line numbers
// ============================================================================

#[test]
fn test_mark_line_number_changes() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("j")        // Line 1
        .press('m')
        .press('a')
        .keys("gg")       // Line 0
        .press('o')       // Insert line above mark
        .type_text("new")
        .press_esc()
        .press('`')
        .press('a');      // Mark should have moved down

    assert_snapshot!(test.snapshot_state());
}

// ============================================================================
// Complex mark navigation
// ============================================================================

#[test]
fn test_complex_mark_navigation() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    // Set multiple marks
    test.press('m').press('a')        // Line 0
        .keys("jj").press('m').press('b')   // Line 2
        .keys("jj").press('m').press('c');  // Line 4

    // Navigate: a -> c -> b -> a
    test.press('`').press('a')
        .press('`').press('c')
        .press('`').press('b')
        .press('`').press('a');

    assert_snapshot!(test.snapshot_state());
}
