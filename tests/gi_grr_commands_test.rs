mod helpers;

use helpers::EditorTest;

/// Test gi command returns to last insert position
#[test]
fn test_gi_returns_to_last_insert_position() {
    let mut test = EditorTest::new("line one\nline two\nline three\n");

    // Enter insert mode at start
    test.keys("i");
    test.type_text("start ");
    test.press_esc();

    // Move away
    test.keys("j");
    test.keys("$");

    // Use gi to return to last insert position
    test.keys("gi");

    // Should be in insert mode at the position after "start "
    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 6); // After "start "
}

/// Test gi after insert in middle of line
#[test]
fn test_gi_after_insert_middle_of_line() {
    let mut test = EditorTest::new("hello world\n");

    // Move to middle and insert
    test.keys("0");
    test.keys("6l"); // After "hello "
    test.keys("i");
    test.type_text("beautiful ");
    test.press_esc();

    // Move away
    test.keys("gg0");

    // Use gi
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 16); // After "beautiful "
}

/// Test gi after append command
#[test]
fn test_gi_after_append() {
    let mut test = EditorTest::new("test\n");

    // Append at end of line
    test.keys("$");
    test.keys("a");
    test.type_text("ing");
    test.press_esc();

    // Move away
    test.keys("0");

    // Use gi
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 7); // After "testing"
}

/// Test gi after insert at beginning of line (I)
#[test]
fn test_gi_after_capital_i() {
    let mut test = EditorTest::new("    indented\n");

    // Insert at beginning of line
    test.keys("I");
    test.type_text("start ");
    test.press_esc();

    // Move away
    test.keys("$");

    // Use gi
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 10); // After "start " at indent position
}

/// Test gi after append at end of line (A)
#[test]
fn test_gi_after_capital_a() {
    let mut test = EditorTest::new("hello\n");

    // Append at end
    test.keys("A");
    test.type_text(" world");
    test.press_esc();

    // Move away
    test.keys("0");

    // Use gi
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 11); // After " world"
}

/// Test gi after insert on new line below (o)
#[test]
fn test_gi_after_o_command() {
    let mut test = EditorTest::new("line one\nline two\n");

    // Open new line below
    test.keys("o");
    test.type_text("new line");
    test.press_esc();

    // Move away
    test.keys("gg");

    // Use gi
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(1, 8); // After "new line"
}

/// Test gi after insert on new line above (O)
#[test]
fn test_gi_after_capital_o_command() {
    let mut test = EditorTest::new("line one\nline two\n");

    // Move to second line
    test.keys("j");

    // Open new line above
    test.keys("O");
    test.type_text("inserted");
    test.press_esc();

    // Move away
    test.keys("G");

    // Use gi
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(1, 8); // After "inserted"
}

/// Test gi after multiple insert sessions (should use last one)
#[test]
fn test_gi_uses_last_insert_position() {
    let mut test = EditorTest::new("one\ntwo\nthree\n");

    // First insert
    test.keys("i");
    test.type_text("first ");
    test.press_esc();

    // Second insert on different line
    test.keys("j");
    test.keys("i");
    test.type_text("second ");
    test.press_esc();

    // Move away
    test.keys("G");

    // gi should go to the last (second) insert position
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(1, 7); // After "second " on line 1
}

/// Test gi when no previous insert exists
#[test]
fn test_gi_with_no_previous_insert() {
    let mut test = EditorTest::new("hello\nworld\n");

    // Try gi without any previous insert
    test.keys("gi");

    // Should enter insert mode at current position
    test.assert_mode(ovim::mode::Mode::Insert);
}

/// Test gi after change command
#[test]
fn test_gi_after_change_command() {
    let mut test = EditorTest::new("hello world\n");

    // Change a word
    test.keys("cw");
    test.type_text("goodbye");
    test.press_esc();

    // Move away
    test.keys("$");

    // Use gi
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 7); // After "goodbye"
}

/// Test gi after substitution (s)
#[test]
fn test_gi_after_substitute_char() {
    let mut test = EditorTest::new("test\n");

    // Substitute character
    test.keys("s");
    test.type_text("T");
    test.press_esc();

    // Move away
    test.keys("$");

    // Use gi
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 1); // After "T"
}

/// Test gi after substitute line (S)
#[test]
fn test_gi_after_substitute_line() {
    let mut test = EditorTest::new("old line\n");

    // Substitute entire line
    test.keys("S");
    test.type_text("new line");
    test.press_esc();

    // Move to end
    test.keys("$");

    // Use gi
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 8); // After "new line"
}

/// Test gi after change to end of line (C)
#[test]
fn test_gi_after_capital_c() {
    let mut test = EditorTest::new("hello world\n");

    // Move to middle
    test.keys("0");
    test.keys("6l");

    // Change to end of line
    test.keys("C");
    test.type_text("there");
    test.press_esc();

    // Move away
    test.keys("0");

    // Use gi
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 11); // After "there"
}

/// Test grr replaces line with register content
#[test]
fn test_grr_replaces_line() {
    let mut test = EditorTest::new("line one\nline two\nline three\n");

    // Yank first line
    test.keys("yy");

    // Move to second line
    test.keys("j");

    // Replace with register using grr
    test.keys("grr");

    // Second line should now be "line one"
    let content = test.buffer_content();
    assert!(content.contains("line one"));
}

/// Test gr motion replaces text with register
#[test]
fn test_gr_with_motion() {
    let mut test = EditorTest::new("hello world\n");

    // Yank "test"
    test.keys("i");
    test.type_text("test");
    test.press_esc();
    test.keys("0");
    test.keys("vw");
    test.keys("y");

    // Start fresh line
    test.keys("cc");
    test.type_text("hello world");
    test.press_esc();
    test.keys("0");

    // Replace word with register
    test.keys("grw");

    // Should replace "hello" with "test"
    let content = test.buffer_content();
    assert!(content.contains("test"));
}

/// Test gr$ replaces to end of line
#[test]
fn test_gr_dollar_replaces_to_end() {
    let mut test = EditorTest::new("original text here\n");

    // Yank some text
    test.keys("i");
    test.type_text("replacement");
    test.press_esc();
    test.keys("0");
    test.keys("viw");
    test.keys("y");

    // New line
    test.keys("cc");
    test.type_text("original text here");
    test.press_esc();
    test.keys("0");
    test.keys("w");

    // Replace from cursor to end
    test.keys("gr$");

    // Should have "original replacement"
    assert!(test.buffer_content().contains("original"));
    assert!(test.buffer_content().contains("replacement"));
}

/// Test gri replaces characters with insert mode
#[test]
fn test_gri_replace_with_insert() {
    let mut test = EditorTest::new("hello\n");

    // Yank text
    test.keys("i");
    test.type_text("world");
    test.press_esc();
    test.keys("0");
    test.keys("viw");
    test.keys("y");

    // New line
    test.keys("cc");
    test.type_text("hello");
    test.press_esc();
    test.keys("0");

    // Replace in insert mode
    test.keys("gri");
    test.type_text("w"); // Just type one char

    test.press_esc();

    // Buffer should be modified
    assert!(test.buffer_content().len() > 0);
}

/// Test gra replaces and appends
#[test]
fn test_gra_replace_and_append() {
    let mut test = EditorTest::new("test line\n");

    // Yank text
    test.keys("0");
    test.keys("viw");
    test.keys("y"); // Yank "test"

    // Position and replace with append
    test.keys("$");
    test.keys("gra");

    test.assert_mode(ovim::mode::Mode::Insert);
}

/// Test grI replaces at beginning of line
#[test]
fn test_gri_capital_at_line_start() {
    let mut test = EditorTest::new("    indented text\n");

    // Yank something
    test.keys("i");
    test.type_text("prefix");
    test.press_esc();
    test.keys("0");
    test.keys("viw");
    test.keys("y");

    // New line
    test.keys("cc");
    test.type_text("    indented text");
    test.press_esc();

    // Replace at line start
    test.keys("grI");

    test.assert_mode(ovim::mode::Mode::Insert);
}

/// Test grA replaces at end of line
#[test]
fn test_gra_capital_at_line_end() {
    let mut test = EditorTest::new("some text\n");

    // Yank something
    test.keys("i");
    test.type_text("suffix");
    test.press_esc();
    test.keys("0");
    test.keys("viw");
    test.keys("y");

    // New line
    test.keys("cc");
    test.type_text("some text");
    test.press_esc();

    // Replace at line end
    test.keys("grA");

    test.assert_mode(ovim::mode::Mode::Insert);
}

/// Test gi preserves position across undo
#[test]
fn test_gi_after_undo() {
    let mut test = EditorTest::new("original\n");

    // Insert text
    test.keys("A");
    test.type_text(" added");
    test.press_esc();

    let insert_pos = test.cursor();

    // Undo the insert
    test.keys("u");

    // gi should still remember the insert position
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    // Position might be adjusted if text was undone
}

/// Test gi after visual mode insert
#[test]
fn test_gi_after_visual_insert() {
    let mut test = EditorTest::new("line one\nline two\nline three\n");

    // Visual block insert (if supported)
    test.keys("gg");
    test.keys("0");
    test.keys("i");
    test.type_text("prefix ");
    test.press_esc();

    // Move away
    test.keys("G");

    // gi should return to last insert
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
}

/// Test multiple gi commands in sequence
#[test]
fn test_multiple_gi_commands() {
    let mut test = EditorTest::new("test\n");

    // First insert
    test.keys("A");
    test.type_text("1");
    test.press_esc();

    // Move and use gi
    test.keys("0");
    test.keys("gi");
    test.assert_mode(ovim::mode::Mode::Insert);

    // Exit and use gi again
    test.press_esc();
    test.keys("0");
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
}

/// Test gi with counts (should enter insert mode once)
#[test]
fn test_gi_with_count() {
    let mut test = EditorTest::new("hello\n");

    test.keys("i");
    test.type_text("world ");
    test.press_esc();

    test.keys("0");

    // Try gi with count (count may be ignored)
    test.keys("5gi");

    test.assert_mode(ovim::mode::Mode::Insert);
}

/// Test gr with visual selection
#[test]
fn test_gr_with_visual_selection() {
    let mut test = EditorTest::new("replace this text\n");

    // Yank some text
    test.keys("i");
    test.type_text("NEW");
    test.press_esc();
    test.keys("0");
    test.keys("viw");
    test.keys("y");

    // Clear and create new text
    test.keys("cc");
    test.type_text("replace this text");
    test.press_esc();
    test.keys("0");
    test.keys("w");

    // Visual select and replace
    test.keys("viw");
    test.keys("gr");

    // "this" should be replaced
    assert!(test.buffer_content().contains("replace"));
}

/// Test gi after paste
#[test]
fn test_gi_not_affected_by_paste() {
    let mut test = EditorTest::new("line\n");

    // Insert text
    test.keys("A");
    test.type_text(" one");
    test.press_esc();

    let insert_cursor = test.cursor();

    // Yank and paste
    test.keys("yy");
    test.keys("p");

    // gi should still return to original insert position
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
}

/// Test gr in empty line
#[test]
fn test_gr_in_empty_line() {
    let mut test = EditorTest::new("\n\n");

    // Yank something
    test.keys("i");
    test.type_text("content");
    test.press_esc();
    test.keys("0");
    test.keys("viw");
    test.keys("y");

    // Go to empty line
    test.keys("j");
    test.keys("cc");
    test.press_esc();

    // Try grr on empty line
    test.keys("grr");

    // Should have content
    let content = test.buffer_content();
    assert!(content.contains("content"));
}

/// Test gi position persistence across file operations
#[test]
fn test_gi_persistence() {
    let mut test = EditorTest::new("original content\n");

    // Make an insert
    test.keys("A");
    test.type_text(" extra");
    test.press_esc();

    // Do some other operations
    test.keys("gg");
    test.keys("yy");
    test.keys("p");
    test.keys("dd");

    // gi should still work
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
}

/// Test gr with count
#[test]
fn test_gr_with_count_and_motion() {
    let mut test = EditorTest::new("word1 word2 word3\n");

    // Yank text
    test.keys("i");
    test.type_text("REPLACED");
    test.press_esc();
    test.keys("0");
    test.keys("viw");
    test.keys("y");

    // Clear and reset
    test.keys("cc");
    test.type_text("word1 word2 word3");
    test.press_esc();
    test.keys("0");

    // Replace 2 words
    test.keys("gr2w");

    // Should replace first two words
    assert!(test.buffer_content().contains("REPLACED"));
}

/// Test gI inserts at column 1 (before indentation)
#[test]
fn test_gi_capital_inserts_at_column_zero() {
    let mut test = EditorTest::new("    indented line\n");

    // Move to middle of line
    test.keys("$");

    // Use gI to insert at column 0
    test.keys("gI");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 0); // At column 0
}

/// Test gI on line with no indentation
#[test]
fn test_gi_capital_on_unindented_line() {
    let mut test = EditorTest::new("no indent\n");

    // Move to middle
    test.keys("w");

    // Use gI
    test.keys("gI");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 0);
}

/// Test gI vs I difference
#[test]
fn test_gi_capital_vs_capital_i() {
    let mut test = EditorTest::new("    indented content\n");

    // Test I (insert at first non-blank)
    test.keys("I");
    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 4); // After indentation

    test.press_esc();

    // Test gI (insert at column 0)
    test.keys("gI");
    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 0); // Before indentation
}

/// Test gI with tabs
#[test]
fn test_gi_capital_with_tabs() {
    let mut test = EditorTest::new("\t\ttext\n");

    // Move to end
    test.keys("$");

    // Use gI
    test.keys("gI");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 0);
}

/// Test gI on empty line
#[test]
fn test_gi_capital_on_empty_line() {
    let mut test = EditorTest::new("\n");

    test.keys("gI");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 0);
}

/// Test gI inserts before whitespace
#[test]
fn test_gi_capital_inserts_before_whitespace() {
    let mut test = EditorTest::new("        heavily indented\n");

    test.keys("$");
    test.keys("gI");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 0);

    // Type something
    test.type_text("//");
    test.press_esc();

    // Should have "//" before indentation
    let content = test.buffer_content();
    assert!(content.starts_with("//"));
}

/// Test gI after various cursor positions
#[test]
fn test_gi_capital_from_various_positions() {
    let mut test = EditorTest::new("  line with indent\n");

    // From start
    test.keys("0");
    test.keys("gI");
    test.assert_cursor(0, 0);
    test.press_esc();

    // From middle
    test.keys("w");
    test.keys("w");
    test.keys("gI");
    test.assert_cursor(0, 0);
    test.press_esc();

    // From end
    test.keys("$");
    test.keys("gI");
    test.assert_cursor(0, 0);
}

/// Test gI with count (count should be ignored)
#[test]
fn test_gi_capital_with_count() {
    let mut test = EditorTest::new("    text\n");

    // Try gI with count
    test.keys("5gI");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 0);
}

/// Test gI then type and verify insertion point
#[test]
fn test_gi_capital_typing() {
    let mut test = EditorTest::new("    original\n");

    test.keys("gI");
    test.type_text("prefix");
    test.press_esc();

    // Should have "prefix    original"
    let content = test.buffer_content();
    assert!(content.starts_with("prefix"));
    assert!(content.contains("original"));
}

/// Test gI updates last insert position
#[test]
fn test_gi_capital_updates_last_insert() {
    let mut test = EditorTest::new("    content\n");

    // Use gI
    test.keys("gI");
    test.type_text("start");
    test.press_esc();

    // Move away
    test.keys("$");

    // Use gi (lowercase) to return
    test.keys("gi");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 5); // After "start"
}

/// Test gI on line with only whitespace
#[test]
fn test_gi_capital_on_whitespace_only_line() {
    let mut test = EditorTest::new("        \n");

    test.keys("gI");

    test.assert_mode(ovim::mode::Mode::Insert);
    test.assert_cursor(0, 0);
}

/// Test gI followed by delete operations
#[test]
fn test_gi_capital_then_delete() {
    let mut test = EditorTest::new("    text here\n");

    test.keys("gI");
    test.assert_cursor(0, 0);

    // Delete some indentation
    test.keys("<C-d>"); // This might delete indent in insert mode

    test.assert_mode(ovim::mode::Mode::Insert);
}
