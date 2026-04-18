mod helpers;
use helpers::EditorTest;

// ============================================================================
// 'm' command - Set mark
// ============================================================================

#[test]
fn test_m_set_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("w") // Move to position
        .press('m') // Set mark
        .press('a'); // Mark 'a'

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(0, 5);
}

#[test]
fn test_m_set_multiple_marks() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('m')
        .press('a') // Mark 'a' at line 0
        .press('j')
        .press('m')
        .press('b') // Mark 'b' at line 1
        .keys("jj")
        .press('m')
        .press('c'); // Mark 'c' at line 3

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(3, 0);
}

#[test]
fn test_m_overwrite_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('m')
        .press('a') // Set mark 'a'
        .keys("jj") // Move
        .press('m')
        .press('a'); // Overwrite mark 'a'

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(2, 0);
}

// ============================================================================
// '`' command - Jump to mark (exact position)
// ============================================================================

#[test]
fn test_backtick_jump_to_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("w") // Move to column 5
        .press('m')
        .press('a') // Set mark
        .keys("G") // Go to end
        .press('`') // Jump to mark
        .press('a'); // Mark 'a'

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
"
    );
    test.assert_cursor(0, 5);
}

#[test]
fn test_backtick_exact_position() {
    let mut test = EditorTest::new("hello world test");

    test.keys("www") // Move to "test"
        .press('m')
        .press('a')
        .keys("gg") // Go to beginning
        .press('`')
        .press('a'); // Should return to "test"

    assert_eq!(
        test.buffer_content(),
        "hello world test
"
    );
    test.assert_cursor(0, 12);
}

#[test]
fn test_backtick_multiple_marks() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    // Set marks
    test.press('m')
        .press('a')
        .keys("jw")
        .press('m')
        .press('b')
        .keys("jw")
        .press('m')
        .press('c');

    // Jump around
    test.press('`')
        .press('a') // Back to mark a
        .press('`')
        .press('c') // To mark c
        .press('`')
        .press('b'); // To mark b

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
line 4
"
    );
    test.assert_cursor(1, 5);
}

// ============================================================================
// "'" command - Jump to mark (beginning of line)
// ============================================================================

#[test]
fn test_quote_jump_to_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("jw") // Line 1, some column
        .press('m')
        .press('a')
        .keys("G") // Go to end
        .press('\'') // Jump to line of mark
        .press('a');

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
"
    );
    test.assert_cursor(1, 0);
}

#[test]
fn test_quote_vs_backtick() {
    let mut test = EditorTest::new("hello world\ntest line");

    test.keys("w") // Move to "world"
        .press('m')
        .press('a')
        .keys("G") // Go to last line
        .press('\'') // ' jumps to first non-blank
        .press('a');

    assert_eq!(
        test.buffer_content(),
        "hello world
test line
"
    );
    test.assert_cursor(0, 0);
}

// ============================================================================
// Special marks
// ============================================================================

#[test]
fn test_backtick_backtick_previous_position() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G") // Go to line 4
        .keys("gg") // Go back to line 1
        .press('`')
        .press('`'); // Jump to previous position (line 4)

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
line 4
"
    );
    test.assert_cursor(3, 0);
}

#[test]
fn test_quote_quote_previous_line() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("G").keys("gg").press('\'').press('\''); // Jump to previous line

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
"
    );
    test.assert_cursor(2, 0);
}

#[test]
fn test_backtick_dot_last_change() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('i')
        .type_text("CHANGED ")
        .press_esc()
        .keys("G") // Move away
        .press('`')
        .press('.'); // Jump to last change position

    assert_eq!(
        test.buffer_content(),
        "CHANGED line 1
line 2
line 3
"
    );
    test.assert_cursor(0, 7);
}

#[test]
#[ignore = "TODO: Implement `[ and `] special marks (yank boundaries)"]
fn test_backtick_bracket_last_yank() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("yy") // Yank line
        .keys("G") // Move away
        .press('`')
        .press('['); // Jump to start of last yank

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
"
    );
    test.assert_cursor(0, 0);
}

#[test]
fn test_backtick_caret_insert_exit() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('i')
        .type_text("text")
        .press_esc()
        .keys("G") // Move away
        .press('`')
        .press('^'); // Jump to last insert position

    assert_eq!(
        test.buffer_content(),
        "textline 1
line 2
"
    );
    test.assert_cursor(0, 3);
}

// ============================================================================
// Marks with operations
// ============================================================================

#[test]
fn test_delete_to_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("jj") // Line 2
        .press('m')
        .press('a')
        .keys("gg") // Go to top
        .keys("d`a"); // Delete to mark

    assert_eq!(
        test.buffer_content(),
        "line 3
line 4
"
    );
    test.assert_cursor(0, 0);
}

#[test]
fn test_yank_to_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("j").press('m').press('a').keys("gg").keys("y`a"); // Yank to mark (characterwise exclusive: "line 1\n")

    // Cursor stays at original position after yank
    test.assert_cursor(0, 0);

    // Verify yanked content by pasting on an empty line at end
    test.keys("Go") // Open new line at end
        .press_esc()
        .press('P'); // Paste before cursor

    // Characterwise paste of "line 1\n" inserts inline
    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 1\n\n");
}

#[test]
fn test_change_to_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("j")
        .press('m')
        .press('a')
        .keys("gg")
        .keys("c`a") // Change to mark (characterwise exclusive: deletes "line 1\n")
        .type_text("CHANGED")
        .press_esc();

    // "line 1\n" deleted, "CHANGED" inserted, "line 2\nline 3\n" remains
    assert_eq!(test.buffer_content(), "CHANGEDline 2\nline 3\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_visual_to_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("jj")
        .press('m')
        .press('a')
        .keys("gg")
        .press('v') // Visual mode
        .press('`')
        .press('a'); // Select to mark

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
line 4
"
    );
    test.assert_cursor(2, 0);
}

// ============================================================================
// Jump list with Ctrl-O and Tab
// ============================================================================

#[test]
fn test_ctrl_o_jump_back() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G") // Jump 1
        .keys("gg") // Jump 2
        .keys("G") // Jump 3
        .press_with(ovim_core::KeyCode::Char('o'), ovim_core::Modifiers::CONTROL); // Jump back

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
line 4
"
    );
    test.assert_cursor(0, 0);
}

#[test]
fn test_tab_jump_forward() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G")
        .keys("gg")
        .press_with(ovim_core::KeyCode::Char('o'), ovim_core::Modifiers::CONTROL) // Back
        .keys("<Tab>"); // Forward

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
line 4
"
    );
    test.assert_cursor(0, 0);
}

#[test]
#[ignore = "TODO: Verify jump list behavior with marks"]
fn test_jump_list_multiple() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    // Create jump list
    test.keys("j").keys("j").keys("j").keys("j");

    // Jump back multiple times
    test.press_with(ovim_core::KeyCode::Char('o'), ovim_core::Modifiers::CONTROL)
        .press_with(ovim_core::KeyCode::Char('o'), ovim_core::Modifiers::CONTROL);

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
line 4
line 5
"
    );
    test.assert_cursor(2, 0);
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
        .press('a') // Mark after insert
        .keys("$") // Move away
        .press('`')
        .press('a'); // Jump back

    assert_eq!(
        test.buffer_content(),
        "INSERTED line 1
"
    );
    test.assert_cursor(0, 8);
}

#[test]
fn test_mark_after_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("dd") // Delete line
        .press('m')
        .press('a')
        .keys("j")
        .press('`')
        .press('a');

    assert_eq!(
        test.buffer_content(),
        "line 2
line 3
"
    );
    test.assert_cursor(0, 0);
}

#[test]
fn test_mark_in_visual_mode() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('v')
        .keys("j") // Visual selection
        .press('m') // Try to set mark in visual mode
        .press('a');

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
"
    );
    test.assert_cursor(1, 0);
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_jump_to_nonexistent_mark() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('`').press('z'); // Jump to mark that doesn't exist

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
"
    );
    test.assert_cursor(0, 0);
}

#[test]
fn test_mark_on_empty_line() {
    let mut test = EditorTest::new("line 1\n\nline 3");

    test.press('j') // Move to empty line
        .press('m')
        .press('a')
        .keys("k")
        .press('`')
        .press('a');

    assert_eq!(
        test.buffer_content(),
        "line 1

line 3
"
    );
    test.assert_cursor(1, 0);
}

#[test]
fn test_mark_at_eof() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.keys("G$") // End of file
        .press('m')
        .press('a')
        .keys("gg")
        .press('`')
        .press('a');

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
"
    );
    test.assert_cursor(1, 5);
}

#[test]
fn test_all_lowercase_marks() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    // Set marks a-z (just a few)
    test.press('m')
        .press('a')
        .keys("j")
        .press('m')
        .press('b')
        .keys("j")
        .press('m')
        .press('c')
        .press('m')
        .press('z'); // Mark z

    // Jump to various marks
    test.press('`')
        .press('a')
        .press('`')
        .press('z')
        .press('`')
        .press('b');

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
"
    );
    test.assert_cursor(1, 0);
}

// ============================================================================
// Marks with undo/redo
// ============================================================================

#[test]
fn test_mark_survives_undo() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('m')
        .press('a') // Set mark
        .press('i')
        .type_text("text")
        .press_esc()
        .press('u') // Undo
        .press('`')
        .press('a'); // Jump to mark (should still exist)

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
"
    );
    test.assert_cursor(0, 0);
}

#[test]
#[ignore = "TODO: Implement mark line number adjustment on buffer modifications"]
fn test_mark_after_line_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("jj") // Line 2
        .press('m')
        .press('a') // Mark at line 2
        .keys("gg")
        .keys("dd") // Delete line 0
        .press('`')
        .press('a'); // Mark should adjust?

    assert_eq!(
        test.buffer_content(),
        "line 2
line 3
line 4
"
    );
    test.assert_cursor(1, 0);
}

// ============================================================================
// List all marks
// ============================================================================

#[test]
fn test_marks_command() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('m')
        .press('a')
        .keys("j")
        .press('m')
        .press('b')
        .keys("j")
        .press('m')
        .press('c')
        .press(':')
        .type_text("marks")
        .press_enter();

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
"
    );
    test.assert_cursor(2, 0);
}

// ============================================================================
// Delete marks
// ============================================================================

#[test]
fn test_delmarks() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('m')
        .press('a')
        .press(':')
        .type_text("delmarks a")
        .press_enter()
        .press('`')
        .press('a'); // Should fail - mark deleted

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
"
    );
    test.assert_cursor(0, 0);
}

// ============================================================================
// Global marks (uppercase A-Z)
// ============================================================================

#[test]
fn test_global_mark() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('m')
        .press('A') // Global mark (uppercase)
        .keys("G")
        .press('`')
        .press('A');

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
"
    );
    test.assert_cursor(0, 0);
}

// ============================================================================
// Marks with line numbers
// ============================================================================

#[test]
#[ignore = "TODO: Implement mark line number adjustment on buffer modifications"]
fn test_mark_line_number_changes() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("j") // Line 1
        .press('m')
        .press('a')
        .keys("gg") // Line 0
        .press('o') // Insert line above mark
        .type_text("new")
        .press_esc()
        .press('`')
        .press('a'); // Mark should have moved down

    assert_eq!(
        test.buffer_content(),
        "line 1
new
line 2
line 3
"
    );
    test.assert_cursor(2, 0);
}

// ============================================================================
// Complex mark navigation
// ============================================================================

#[test]
fn test_complex_mark_navigation() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4\nline 5");

    // Set multiple marks
    test.press('m')
        .press('a') // Line 0
        .keys("jj")
        .press('m')
        .press('b') // Line 2
        .keys("jj")
        .press('m')
        .press('c'); // Line 4

    // Navigate: a -> c -> b -> a
    test.press('`')
        .press('a')
        .press('`')
        .press('c')
        .press('`')
        .press('b')
        .press('`')
        .press('a');

    assert_eq!(
        test.buffer_content(),
        "line 1
line 2
line 3
line 4
line 5
"
    );
    test.assert_cursor(0, 0);
}

// ============================================================================
// OV-00190: local mark jump must clamp to current buffer bounds
// ============================================================================

/// Set a local mark on the last line, delete that line, jump back. Pre-fix
/// the cursor would land at `mark.line` (past EOF). Post-fix it clamps to
/// the new last line.
///
/// Calls `jump_to_mark` directly so the test isn't masked by the
/// `handle_key_event` post-input `validate_cursor_position` safety net —
/// the bug lives in the per-API contract, not just at the input boundary,
/// and any future caller that bypasses the safety net (e.g. an LSP-driven
/// jump or scripted automation) would still corrupt the cursor.
#[test]
fn test_ov00190_backtick_local_mark_clamps_to_eof_after_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    // Mark the last line.
    test.keys("G").press('m').press('a');
    assert_eq!(
        test.editor.marks().get_mark('a').expect("mark set").line,
        2,
        "test setup: mark recorded on the original line 2"
    );

    // Now delete the last two lines so the marked line no longer exists.
    test.keys("gg").keys("dd").keys("dd");
    let line_count_after = test.editor.buffer().line_count();
    assert!(
        line_count_after <= 2,
        "test setup: dd dd should reduce to <=2 lines, got {line_count_after}"
    );

    // Direct call — bypasses the input-dispatch safety net so we observe
    // the contract `jump_to_mark` itself enforces.
    let did_jump = test.editor.jump_to_mark('a');
    assert!(did_jump, "mark `a` was set, jump should report success");

    let cursor_line = test.editor.buffer().cursor().line();
    let max_valid_line = line_count_after.saturating_sub(1);
    assert!(
        cursor_line <= max_valid_line,
        "local mark jump must clamp to a valid line (max {max_valid_line}), got {cursor_line}"
    );
    assert!(
        test.editor.buffer().line(cursor_line).is_some(),
        "cursor must land on a line that actually exists"
    );
}

/// Same shape with the apostrophe (linewise) variant of mark jump. Direct
/// `jump_to_mark_line` call to bypass the input-handler safety net — see
/// the rationale on `test_ov00190_backtick_local_mark_clamps_to_eof_after_delete`.
#[test]
fn test_ov00190_apostrophe_local_mark_clamps_to_eof_after_delete() {
    let mut test = EditorTest::new("line 1\n  indented\nlast");

    test.keys("G").press('m').press('a');
    test.keys("gg").keys("dd").keys("dd");
    let line_count_after = test.editor.buffer().line_count();

    let did_jump = test.editor.jump_to_mark_line('a');
    assert!(did_jump, "mark `a` was set, jump should report success");

    let cursor_line = test.editor.buffer().cursor().line();
    let max_valid_line = line_count_after.saturating_sub(1);
    assert!(
        cursor_line <= max_valid_line,
        "linewise local mark jump must clamp to a valid line (max {max_valid_line}), got {cursor_line}"
    );
}

/// Marked column is past the new line's last grapheme. Without clamping, the
/// cursor would sit past end-of-line; clamping snaps it to the last
/// grapheme (cursor-on-char semantics). Direct call bypasses the input
/// safety net — see rationale on the EOF-clamp test above.
#[test]
fn test_ov00190_local_mark_clamps_column_when_line_shrinks() {
    let mut test = EditorTest::new("hello world\nshort");

    // Mark at column 10 ('d' in "world") on line 0.
    test.keys("$").press('m').press('a');
    test.assert_cursor(0, 10);

    // Use ex command :1s to deterministically shrink line 0 to "hi" (2 graphemes).
    test.press(':').type_text("1s/hello world/hi/").press_enter();

    // Move away, then jump back via direct API to observe the contract.
    test.keys("G");
    let did_jump = test.editor.jump_to_mark('a');
    assert!(did_jump, "mark `a` was set, jump should report success");

    let cursor = test.editor.buffer().cursor();
    assert_eq!(cursor.line(), 0, "jumped to marked line");
    assert!(
        cursor.col().0 <= 1,
        "column must be clamped into 'hi' (last grapheme col 1), got {}",
        cursor.col().0
    );
}
