//! Tests reproducing bugs for tracking and verification.
//!
//! Bug 1: Welcome screen shows when opening file from CLI
//! Bug 2: `p` paste behavior inconsistency
//! Bug 3: `o<esc>` leaves spaces instead of empty line

mod helpers;
use helpers::EditorTest;
use ovim::editor::Editor;
use ovim::mode::Mode;

// ============================================================================
// Bug 1: Welcome screen showing when opening file from CLI
// ============================================================================
//
// ROOT CAUSE ANALYSIS:
// - `Editor::new()` in src/editor/mod.rs:349 sets `mode: Mode::Dashboard`
// - When a file is loaded from CLI, main.rs calls `Editor::new()` then `load_file()`
// - `load_file_async()` never changes the mode from Dashboard to Normal
// - Result: Dashboard shows even when file is loaded
//
// FIX: In `load_file_async()`, set mode to Normal after successfully loading file
// Or: In main.rs, after `load_file()` succeeds, call `editor.set_mode(Mode::Normal)`
//
// Note: `Editor::with_content()` correctly uses `Mode::default()` which is Normal

#[test]
fn test_editor_new_starts_in_dashboard() {
    // This documents current behavior - Editor::new() starts in Dashboard mode
    let editor = Editor::new();
    assert_eq!(
        editor.mode(),
        Mode::Dashboard,
        "Editor::new() should start in Dashboard mode for welcome screen"
    );
}

#[test]
fn test_editor_with_content_starts_in_normal() {
    // Editor::with_content() correctly starts in Normal mode
    let editor = Editor::with_content("hello world");
    assert_eq!(
        editor.mode(),
        Mode::Normal,
        "Editor::with_content() should start in Normal mode"
    );
}

// Note: test_load_file_should_exit_dashboard requires Tokio runtime
// The bug can be demonstrated by observing that in main.rs:42-57:
//   - Editor::new() is called (starts in Dashboard mode)
//   - load_file() is called (does NOT change mode)
//   - Mode stays Dashboard even with file loaded
//
// The fix is either:
// 1. In load_file_async(), set mode to Normal after successfully loading
// 2. In main.rs, after successful load_file(), call editor.set_mode(Mode::Normal)

// ============================================================================
// Bug 2: `p` paste behavior inconsistency
// ============================================================================
//
// ROOT CAUSE ANALYSIS:
// **FOUND THE BUG!** In src/editor/input/mod.rs lines 646-647:
//
//   let yanked = Operators::yank_line(editor.buffer(), count)?;
//   editor.yank_to_register(yanked);  // <-- BUG HERE
//
// The method `yank_to_register()` defaults to `RegisterType::Character` (mod.rs:871-872):
//
//   pub fn yank_to_register(&mut self, text: String) {
//       self.yank_to_register_with_type(text, RegisterType::Character);  // WRONG!
//   }
//
// So `yy` yanks with RegisterType::Character instead of RegisterType::Line!
//
// When pasting with `p`:
// - For `RegisterType::Character`: inserts at `(line_idx, col + 1)` - INLINE
// - For `RegisterType::Line`: inserts at end of current line - NEW LINE BELOW
//
// With cursor at (0, 0), paste_after for Character type inserts at (0, 1),
// breaking "line 1\n" into "l" + "line 1\n" + "ine 1\n" = "lline 1\nine 1\n"
//
// FIX: Change line 647 to:
//   editor.yank_to_register_with_type(yanked, RegisterType::Line);
//
// Same fix needed for: yj, yk, Y, and other linewise yank operations.

#[test]
fn test_p_linewise_pastes_on_new_line_below() {
    // After yy, p should paste the line BELOW the current line
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("yy") // Yank "line 1\n" with Line register type
        .keys("p"); // Paste after - should create new line below

    // Expected: line 1, then duplicated line 1, then line 2, line 3
    assert_eq!(test.buffer_content(), "line 1\nline 1\nline 2\nline 3\n");

    // Cursor should be on the newly pasted line (line 1, not line 2)
    test.assert_cursor(1, 0);
}

#[test]
fn test_p_on_last_line_creates_new_line() {
    // BUG: When on last line with content, p should paste on NEW line below
    // not concatenate with current line
    let mut test = EditorTest::new("first\nlast");

    test.keys("yy")  // Yank "first\n"
        .keys("G")   // Go to last content line ("last")
        .keys("p");  // Paste after

    // Expected:
    // first
    // last
    // first
    //
    // Current buggy behavior produces "lastfirst" concatenated
    assert_eq!(
        test.buffer_content(),
        "first\nlast\nfirst\n",
        "Linewise paste on last line should create new line, not concatenate"
    );
}

#[test]
fn test_paste_after_uses_register_type_correctly() {
    // Test that character yank pastes inline, line yank pastes on new line
    let mut test = EditorTest::new("hello world");

    // Character yank (yw on word without newline)
    test.keys("yw")   // Yank "hello "
        .keys("$")    // Go to end
        .keys("p");   // Paste after

    // Should paste inline after the 'd' in "world"
    // Result should be "hello worldhello " (with trailing space)
    let content = test.buffer_content();
    assert!(
        content.starts_with("hello world"),
        "Character paste should be inline, got: {}",
        content
    );
}

// ============================================================================
// Bug 3: `o<esc>` leaves spaces instead of empty line
// ============================================================================
//
// ROOT CAUSE ANALYSIS:
// - `insert_line_below()` in src/editor/input/helpers.rs:370-400
// - Creates new line with current line's indentation: `format!("{}\n", indent)`
// - When user presses Esc without typing, the indent whitespace remains
// - Vim behavior: pressing Esc after o without typing removes trailing whitespace
//
// The fix should be in the Insert→Normal mode transition:
// - When exiting insert mode, if current line is only whitespace and was created
//   by o/O, remove the whitespace
// - Or: track if any typing occurred in insert mode since o/O, and clean up if not
//
// Related: `i` should NOT alter whitespace - entering and exiting insert mode
// with `i<Esc>` should leave the line unchanged

#[test]
fn test_o_esc_leaves_empty_line() {
    // After o<Esc> without typing, the new line should be empty
    let mut test = EditorTest::new("    indented line\nnext line");

    // Position on indented line
    test.assert_cursor(0, 0);

    // Press o (opens new line with indent) then Esc (exit without typing)
    test.keys("o");
    test.assert_mode(Mode::Insert);

    // Line 1 should now have the auto-indent "    \n"
    // assert_eq!(test.line(1), Some("    \n".to_string())); // Before Esc

    test.keys("<Esc>");
    test.assert_mode(Mode::Normal);

    // Vim behavior: After Esc without typing, line should be just "\n"
    assert_eq!(
        test.line(1),
        Some("\n".to_string()),
        "After o<Esc> without typing, line should be empty (no indent)"
    );
}

#[test]
fn test_o_esc_removes_whitespace_only_line() {
    // Verify that o<Esc> properly removes the auto-indent whitespace
    let mut test = EditorTest::new("    indented\nnext");

    test.keys("o");      // Open new line with indent
    test.keys("<Esc>");  // Exit without typing

    // Correct behavior: line should be empty (no auto-indent whitespace)
    let line1 = test.line(1).unwrap_or_default();
    assert_eq!(
        line1, "\n",
        "o<Esc> should leave empty line, not indented. Line 1 = {:?}",
        line1
    );
}

#[test]
fn test_i_esc_should_not_alter_whitespace() {
    // `i<Esc>` should NOT change anything about the current line
    let mut test = EditorTest::new("  some text  \nother");

    let original_line = test.line(0).unwrap();

    test.keys("i");       // Enter insert mode
    test.keys("<Esc>");   // Exit immediately

    let after_line = test.line(0).unwrap();

    assert_eq!(
        original_line, after_line,
        "i<Esc> should not alter the line at all"
    );
}

#[test]
fn test_o_with_typing_keeps_content() {
    // When you actually type after o, content should be kept
    let mut test = EditorTest::new("    indented\nnext");

    test.keys("o");
    test.keys("hello");
    test.keys("<Esc>");

    // The line should have indent + "hello"
    let line1 = test.line(1).unwrap();
    assert!(
        line1.contains("hello"),
        "o with typing should keep the typed content. Line 1 = {:?}",
        line1
    );
}

#[test]
fn test_O_esc_leaves_empty_line() {
    // O (insert line above) should also remove whitespace on Esc
    let mut test = EditorTest::new("    indented\nnext");

    test.keys("j");      // Go to "next"
    test.keys("O");      // Open line above with potential indent
    test.keys("<Esc>");  // Exit without typing

    // Line 1 should be empty after Esc without typing
    assert_eq!(
        test.line(1),
        Some("\n".to_string()),
        "After O<Esc> without typing, line should be empty"
    );
}

#[test]
fn test_cc_esc_behavior() {
    // cc (change line) followed by Esc - should this also clean whitespace?
    // This is a related case to document
    let mut test = EditorTest::new("    indented content\nnext");

    test.keys("cc");     // Change entire line (delete content, keep indent, enter insert)
    test.keys("<Esc>");  // Exit without typing

    // Document current behavior
    let line0 = test.line(0).unwrap();
    eprintln!("After cc<Esc>, line 0 = {:?}", line0);
    // Note: Vim's cc keeps indent and Esc keeps it - this is intentional
    // cc is different from o in that the line existed before
}
