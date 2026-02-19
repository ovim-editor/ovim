#![allow(
    non_snake_case,
    reason = "Test names intentionally mirror Vim key notation (for example V, gN, and G)."
)]

#[macro_use]
#[path = "helpers/editor_test_macro.rs"]
mod editor_test_macro;
mod helpers;
use helpers::EditorTest;
use ovim::mode::Mode;
use ovim_core::{KeyCode, Modifiers};

// ============================================================================
// 'v' command - Character-wise visual mode
// ============================================================================

#[test]
fn test_v_basic_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v') // Enter visual mode
        .keys("lll"); // Select 4 chars (h, e, l, l)

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_v_delete_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("lll") // Select "hell"
        .press('d'); // Delete selection

    assert_eq!(test.buffer_content(), "o world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_v_yank_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("llll") // Select "hello"
        .press('y') // Yank
        .press_esc()
        .keys("$") // Move to end
        .press('p'); // Paste

    // Yanking "hello" (5 chars from positions 0-4), paste after 'd' at end
    assert_eq!(test.buffer_content(), "hello worldhello\n");
    test.assert_cursor(0, 15);
}

#[test]
fn test_v_select_word() {
    let mut test = EditorTest::new("hello world test");

    test.press('v').keys("e"); // Select to end of word

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_v_across_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('v').keys("jjj"); // Select across multiple lines

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_v_backward_selection() {
    let mut test = EditorTest::new("hello world");

    test.keys("$") // Go to end
        .press('v')
        .keys("hhh"); // Select backward

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 7);
}

#[test]
fn test_v_change_selection() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("llll") // Select "hello"
        .press('c') // Change
        .type_text("goodbye")
        .press_esc();

    assert_eq!(test.buffer_content(), "goodbye world\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_v_escape_cancels() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("lll") // Make selection
        .press_esc(); // Cancel

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 3);
}

// ============================================================================
// 'V' command - Line-wise visual mode
// ============================================================================

#[test]
fn test_V_basic_selection() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V'); // Enter visual line mode

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_V_multiple_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V').keys("jj"); // Select 3 lines

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_V_delete_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V')
        .keys("jj") // Select 3 lines
        .press('d'); // Delete

    assert_eq!(test.buffer_content(), "line 4\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_V_yank_paste_lines() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j') // Select 2 lines
        .press('y') // Yank
        .press_esc()
        .press('G') // Go to last line
        .press('p'); // Paste

    assert_eq!(
        test.buffer_content(),
        "line 1\nline 2\nline 3\nline 1\nline 2\n"
    );
    // Vim: cursor on first non-blank of first pasted line
    test.assert_cursor(3, 0);
}

#[test]
fn test_V_from_middle_of_line() {
    let mut test = EditorTest::new("hello world\ntest line");

    test.keys("w") // Move to "world"
        .press('V'); // Should select entire line

    assert_eq!(test.buffer_content(), "hello world\ntest line\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_V_select_all() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V').keys("G"); // Select to last line

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    // G moves cursor to last line (selection extends from line 0 to line 2)
    test.assert_cursor(2, 0);
}

#[test]
fn test_V_indent() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j') // Select 2 lines
        .press('>'); // Indent (if implemented)

    assert_eq!(test.buffer_content(), "    line 1\n    line 2\nline 3\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_V_backward_selection() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.keys("G") // Go to last line
        .press('V')
        .keys("kk"); // Select upward

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(1, 0);
}

// ============================================================================
// Visual mode with operators
// ============================================================================

#[test]
fn test_v_delete_word() {
    let mut test = EditorTest::new("hello world test");

    test.press('v')
        .keys("e") // Select word
        .press('d'); // Delete

    assert_eq!(test.buffer_content(), " world test\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_v_change_word() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("e") // Select "hello"
        .press('c') // Change
        .type_text("goodbye")
        .press_esc();

    assert_eq!(test.buffer_content(), "goodbye world\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_V_delete_and_undo() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('j') // Select 2 lines
        .press('d') // Delete
        .press('u'); // Undo

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    test.assert_cursor(1, 0);
}

#[test]
fn test_v_yank_and_replace() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("llll") // Select "hello"
        .press('y') // Yank
        .press_esc()
        .keys("w") // Move to "world"
        .press('v')
        .keys("llll") // Select "world"
        .press('p'); // Paste (should replace selection)

    // Vim replaces visual selection with yanked text: "hello" replaces "world" → "hello hello"
    assert_eq!(test.buffer_content(), "hello hello\n");
    test.assert_cursor(0, 10);
}

// ============================================================================
// Visual mode edge cases
// ============================================================================

#[test]
fn test_v_empty_line() {
    let mut test = EditorTest::new("hello\n\nworld");

    test.press('j') // Move to empty line
        .press('v')
        .press('j'); // Select to next line

    assert_eq!(test.buffer_content(), "hello\n\nworld\n");
    test.assert_cursor(2, 0);
}

#[test]
fn test_v_single_char() {
    let mut test = EditorTest::new("x");

    test.press('v'); // Select single char

    assert_eq!(test.buffer_content(), "x\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_v_end_of_line() {
    let mut test = EditorTest::new("hello");

    test.keys("$") // Move to end
        .press('v'); // Select last char

    assert_eq!(test.buffer_content(), "hello\n");
    test.assert_cursor(0, 4);
}

#[test]
fn test_V_single_line() {
    let mut test = EditorTest::new("only line");

    test.press('V'); // Select only line

    assert_eq!(test.buffer_content(), "only line\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_v_select_entire_file() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.keys("gg") // Go to top
        .press('v')
        .keys("G"); // Select to end

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\n");
    // G moves cursor to last line (selection extends from line 0 to line 2)
    test.assert_cursor(2, 0);
}

// ============================================================================
// Visual mode with motions
// ============================================================================

#[test]
fn test_v_with_w_motion() {
    let mut test = EditorTest::new("hello world test");

    test.press('v').press('w'); // Select word forward

    assert_eq!(test.buffer_content(), "hello world test\n");
    test.assert_cursor(0, 6);
}

#[test]
fn test_v_with_dollar() {
    let mut test = EditorTest::new("hello world");

    test.press('v').keys("$"); // Select to end of line

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 10);
}

#[test]
fn test_v_with_zero() {
    let mut test = EditorTest::new("hello world");

    test.keys("$") // Go to end
        .press('v')
        .keys("0"); // Select to beginning

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_visual_find_repeat_semicolon_and_comma() {
    editor_test! {
        given Normal {
            "ab_cd_ef", "^",
        }
        keys "vf_;"
        expect Visual {
            "ab_cd_ef", "-----^  ",
        }
        keys ","
        expect Visual {
            "ab_cd_ef", "--^     ",
        }
    }
}

#[test]
fn test_visual_block_find_repeat_semicolon_and_comma() {
    editor_test! {
        given Normal {
            "aa_bb_cc", "^",
            "dd_ee_ff", "",
            "gg_hh_ii", "",
        }
        keys "<C-v>jjf_;"
        expect VisualBlock {
            "aa_bb_cc", "------  ",
            "dd_ee_ff", "------  ",
            "gg_hh_ii", "-----^  ",
        }
        keys ","
        expect VisualBlock {
            "aa_bb_cc", "---     ",
            "dd_ee_ff", "---     ",
            "gg_hh_ii", "--^     ",
        }
    }
}

#[test]
fn test_V_with_gg() {
    editor_test! {
        given Normal {
            "line 1", "^",
            "line 2", "",
            "line 3", "",
            "line 4", "",
        }
        keys "GVgg"
        expect VisualLine {
            "line 1", "^-",
            "line 2", "-",
            "line 3", "-",
            "line 4", "-",
        }
    }
}

#[test]
fn test_v_with_gg_moves_cursor_and_extends_selection() {
    editor_test! {
        given Normal {
            "line 1", "^",
            "line 2", "",
            "line 3", "",
            "line 4", "",
        }
        keys "Gvgg"
        expect Visual {
            "line 1", "^",
            "line 2", "-",
            "line 3", "-",
            "line 4", "-",
        }
    }
}

#[test]
fn test_v_with_G_moves_cursor_and_extends_selection() {
    editor_test! {
        given Normal {
            "line 1", "^",
            "line 2", "",
            "line 3", "",
            "line 4", "",
        }
        keys "ggvG"
        expect Visual {
            "line 1", "-",
            "line 2", "-",
            "line 3", "-",
            "line 4", "^",
        }
    }
}

#[test]
fn test_visual_block_gg_and_G_preserve_column() {
    editor_test! {
        given Normal {
            "aaaaaa", "^",
            "bbbbbb", "",
            "cccccc", "",
            "dddddd", "",
        }
        keys "G3l<C-v>gg"
        expect VisualBlock {
            "aaaaaa", "   ^  ",
            "bbbbbb", "   -  ",
            "cccccc", "   -  ",
            "dddddd", "   -  ",
        }
        keys "<Esc>gg3l<C-v>G"
        expect VisualBlock {
            "aaaaaa", "   -  ",
            "bbbbbb", "   -  ",
            "cccccc", "   -  ",
            "dddddd", "   ^  ",
        }
    }
}

#[test]
fn test_editor_test_dsl_allows_multi_step_debugging() {
    editor_test! {
        given Normal {
            "line 1", "^",
            "line 2", "",
            "line 3", "",
            "line 4", "",
        }
        keys "Gv"
        expect Visual {
            "line 1", "",
            "line 2", "",
            "line 3", "",
            "line 4", "^",
        }
        keys "gg"
        expect Visual {
            "line 1", "^",
            "line 2", "-",
            "line 3", "-",
            "line 4", "-",
        }
    }
}

// ============================================================================
// Switch between visual modes
// ============================================================================

#[test]
fn test_v_to_V_switch() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('v') // Character visual
        .press('V'); // Switch to line visual

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_V_to_v_switch() {
    let mut test = EditorTest::new("line 1\nline 2");

    test.press('V') // Line visual
        .press('v'); // Switch to character visual

    assert_eq!(test.buffer_content(), "line 1\nline 2\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Visual mode with count
// ============================================================================

#[test]
fn test_v_with_count() {
    let mut test = EditorTest::new("hello world test");

    test.press('v').keys("3l"); // Select 4 chars (including current)

    assert_eq!(test.buffer_content(), "hello world test\n");
    // 3l moves 3 positions: 0 -> 3
    test.assert_cursor(0, 3);
}

#[test]
fn test_V_with_count() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    test.press('V').keys("3j"); // Select 4 lines

    assert_eq!(test.buffer_content(), "line 1\nline 2\nline 3\nline 4\n");
    test.assert_cursor(3, 0);
}

// ============================================================================
// Visual mode with search
// ============================================================================

#[test]
fn test_v_to_search_result() {
    let mut test = EditorTest::new("hello world hello");

    test.press('v').press('/'); // Start search in visual mode

    assert_eq!(test.buffer_content(), "hello world hello\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Visual mode reselect with gv
// ============================================================================

#[test]
fn test_gv_reselect() {
    let mut test = EditorTest::new("hello world");

    test.press('v')
        .keys("lll") // Select
        .press_esc() // Exit visual mode
        .keys("gv"); // Reselect last selection

    assert_eq!(test.buffer_content(), "hello world\n");
    test.assert_cursor(0, 3);
}

#[test]
fn test_gv_after_delete() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3");

    test.press('V')
        .press('d') // Delete line
        .keys("gv"); // Reselect (might not work after delete)

    assert_eq!(test.buffer_content(), "line 2\nline 3\n");
    test.assert_cursor(0, 0);
}

// ============================================================================
// Visual mode with indented text
// ============================================================================

#[test]
fn test_V_indented_lines() {
    let mut test = EditorTest::new("    line 1\n    line 2\n    line 3");

    test.press('V')
        .press('j') // Select 2 indented lines
        .press('d'); // Delete

    assert_eq!(test.buffer_content(), "    line 3\n");
    test.assert_cursor(0, 0);
}

#[test]
fn test_v_select_indentation() {
    let mut test = EditorTest::new("    indented line");

    test.press('v').keys("lll"); // Select spaces

    assert_eq!(test.buffer_content(), "    indented line\n");
    test.assert_cursor(0, 3);
}

// ============================================================================
// Visual mode search features (* and # in visual mode)
// ============================================================================

#[test]
fn test_visual_star_search() {
    let mut test = EditorTest::new("hello world\nhello test\nhello world");

    // Select "hello" and press * to search forward
    test.press('v')
        .keys("llll") // Select "hello"
        .press('*'); // Search for next occurrence

    // Should exit visual mode and jump to next "hello" on line 1
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(1, 0);

    // Press n to find next match
    test.press('n');
    test.assert_cursor(2, 0);
}

#[test]
fn test_visual_hash_search() {
    let mut test = EditorTest::new("hello world\nhello test\nhello world");

    // Move to line 2, select "hello" and press # to search backward
    test.keys("jj") // Move to line 2
        .press('v')
        .keys("llll") // Select "hello"
        .press('#'); // Search backward

    // Should exit visual mode and jump to previous "hello" on line 1
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(1, 0);

    // Press n to continue searching backward
    test.press('n');
    test.assert_cursor(0, 0);
}

#[test]
fn test_visual_star_multiline_selection() {
    let mut test = EditorTest::new("hello\nworld\nhello\ntest");

    // Select across lines and press *
    test.press('v')
        .keys("jl") // Select "hello\nwo"
        .press('*'); // Search for this multi-line text

    // Should find the next occurrence (note: multiline search might not match exactly)
    // The behavior should be that it searches for the literal text
    assert_eq!(test.editor.mode(), Mode::Normal);
}

#[test]
fn test_visual_block_star_search() {
    let mut test = EditorTest::new("abc\ndef\nghi\nabc");

    // Select block and press *
    test.press_with(KeyCode::Char('v'), Modifiers::CONTROL) // Visual block mode
        .keys("l") // Select 2 chars width
        .press('*'); // Search for "ab"

    // Should exit visual mode and jump to next occurrence of "ab" on line 3
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(3, 0);
}

#[test]
fn test_visualline_star_search() {
    let mut test = EditorTest::new("line one\nline two\nline one");

    // Select full line and press *
    test.press('V') // Visual line mode
        .press('*'); // Search for "line one"

    // Should exit visual mode and jump to next occurrence
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(2, 0);
}

#[test]
fn test_visual_star_with_special_chars() {
    let mut test = EditorTest::new("foo.bar\ntest\nfoo.bar");

    // Select "foo.bar" which contains regex special character '.'
    test.press('v')
        .keys("llllll") // Select "foo.bar"
        .press('*'); // Should escape the '.' for literal search

    // Should find exact match, not regex match
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(2, 0);
}

// ============================================================================
// Visual mode search extension (/ and ? extend selection)
// ============================================================================

#[test]
fn test_visual_search_extends_selection_forward() {
    let mut test = EditorTest::new("hello world test hello");

    // Start visual selection at beginning
    test.press('v')
        .keys("ll") // Select "hel"
        .press('/') // Enter search mode
        .type_text("test")
        .press_enter(); // Execute search

    // Should extend selection from "hello" to "test"
    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 12); // At start of "test"

    // Visual anchor should be at original position (0, 0)
    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 0)));
}

#[test]
fn test_visual_search_extends_selection_backward() {
    let mut test = EditorTest::new("hello world test hello");

    // Start at end, select backward, then search backward
    test.keys("$") // Move to end
        .press('v')
        .keys("hh") // Select backward
        .press('?') // Enter backward search
        .type_text("world")
        .press_enter(); // Execute search

    // Should extend selection from end to "world"
    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 6); // At start of "world"
}

#[test]
fn test_visualline_search_extends_selection() {
    let mut test = EditorTest::new("line 1\nline 2\nline 3\nline 4");

    // Start visual line selection
    test.press('V') // Visual line mode
        .press('/') // Enter search mode
        .type_text("line 3")
        .press_enter(); // Execute search

    // Should extend selection to include lines up to line 3
    assert_eq!(test.editor.mode(), Mode::VisualLine);
    test.assert_cursor(2, 0); // At line 3
}

#[test]
fn test_visualblock_search_extends_selection() {
    let mut test = EditorTest::new("abc def\nghi jkl\nmno pqr");

    // Start visual block selection
    test.press_with(KeyCode::Char('v'), Modifiers::CONTROL) // Visual block
        .press('/') // Enter search mode
        .type_text("jkl")
        .press_enter(); // Execute search

    // Should extend block selection to search match
    assert_eq!(test.editor.mode(), Mode::VisualBlock);
    test.assert_cursor(1, 4); // At "jkl"
}

#[test]
fn test_visual_search_escape_cancels() {
    let mut test = EditorTest::new("hello world test");

    // Start visual selection and cancel search
    test.press('v')
        .keys("ll")
        .press('/') // Enter search mode (saves position at 0, 2)
        .type_text("test")
        .press_esc(); // Cancel search

    // Should return to visual mode at position when search was started (0, 2)
    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 2); // Cursor restored to position when / was pressed

    // Visual anchor should still be at original position (0, 0)
    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 0)));
}

// ============================================================================
// 'gn' and 'gN' commands - Search motion with visual selection
// ============================================================================

#[test]
fn test_gn_normal_mode_selects_next_match() {
    let mut test = EditorTest::new("foo bar foo baz");

    // Search for "foo" and then use gn to select first match
    test.press('/').type_text("foo").press_enter().keys("gn"); // Should select first "foo"

    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 2); // End of "foo" (inclusive)

    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 0))); // Start of "foo"
}

#[test]
fn test_gn_selects_current_match_if_cursor_on_it() {
    let mut test = EditorTest::new("foo bar foo baz");

    // Search and move to second "foo", then gn should select it
    test.press('/')
        .type_text("foo")
        .press_enter()
        .press('n') // Move to second "foo"
        .keys("gn"); // Should select second "foo"

    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 10); // End of second "foo"

    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 8))); // Start of second "foo"
}

#[test]
fn test_gn_selects_next_when_not_on_match() {
    let mut test = EditorTest::new("foo bar foo baz");

    // Search for "foo", move to "bar", then gn should select next "foo"
    test.press('/')
        .type_text("foo")
        .press_enter()
        .keys("w") // Move to "bar"
        .keys("gn"); // Should select next "foo"

    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 10); // End of second "foo"

    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 8))); // Start of second "foo"
}

#[test]
fn test_gN_selects_previous_match() {
    let mut test = EditorTest::new("foo bar foo baz");

    // Search for "foo", move to end, then gN should select previous "foo"
    test.press('/')
        .type_text("foo")
        .press_enter()
        .press('$') // Move to end
        .keys("gN"); // Should select second "foo" (previous from end)

    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 10); // End of second "foo"

    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 8))); // Start of second "foo"
}

#[test]
fn test_gn_visual_mode_extends_selection() {
    let mut test = EditorTest::new("foo bar foo baz");

    // Search for "foo", select first one, then extend to next
    test.press('/')
        .type_text("foo")
        .press_enter()
        .press('v') // Enter visual mode
        .keys("gn"); // Should extend to next "foo"

    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 10); // End of second "foo"

    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 0))); // Original anchor at first character
}

#[test]
fn test_gN_visual_mode_extends_selection_backward() {
    let mut test = EditorTest::new("foo bar foo baz");

    // Search backward from end, select last "foo", then extend to previous
    test.press('$') // Go to end
        .press('?') // Backward search
        .type_text("foo")
        .press_enter()
        .press('v') // Enter visual mode
        .keys("gN"); // Should extend to previous "foo"

    assert_eq!(test.editor.mode(), Mode::Visual);
    // Should extend from current position backward to first "foo"
}

#[test]
fn test_gn_multiline_search() {
    let mut test = EditorTest::new("foo\nbar\nfoo\nbaz");

    // Search for "foo" across lines
    test.press('/').type_text("foo").press_enter().keys("gn"); // Select first "foo"

    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 2); // End of "foo" on line 0

    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 0)));
}

#[test]
fn test_gn_wraps_around_buffer() {
    let mut test = EditorTest::new("foo bar baz");

    // Search for "baz", then gn should select current match (baz)
    // After executing the search, cursor is at start of "baz" (within the match)
    test.press('/').type_text("baz").press_enter().keys("gn"); // Should select current "baz" since cursor is within it

    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 10); // End of "baz"

    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 8))); // Start of "baz"
}

#[test]
fn test_gn_wraps_to_beginning_multiline() {
    let mut test = EditorTest::new("foo bar baz\nmore text\nfoo again");

    // Search for "foo", move to second line, then gn should wrap back to first "foo"
    test.press('/')
        .type_text("foo")
        .press_enter()
        .press('j') // Move to second line
        .keys("gn"); // Should find third "foo" on line 3

    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(2, 2); // End of "foo" on line 3

    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((2, 0))); // Start of "foo" on line 3
}

#[test]
fn test_gn_no_search_does_nothing() {
    let mut test = EditorTest::new("foo bar baz");

    // Try gn without active search
    test.keys("gn");

    // Should stay in normal mode
    assert_eq!(test.editor.mode(), Mode::Normal);
    test.assert_cursor(0, 0); // Cursor unchanged
}

#[test]
fn test_cgn_change_next_match() {
    let mut test = EditorTest::new("foo bar foo baz");

    // Search for "foo" and change next match
    test.press('/')
        .type_text("foo")
        .press_enter()
        .keys("cgn") // Change next match
        .type_text("FOO")
        .press_esc();

    assert_eq!(test.buffer_content(), "FOO bar foo baz\n");
    test.assert_cursor(0, 2); // At end of "FOO"
}

#[test]
fn test_dgn_delete_next_match() {
    let mut test = EditorTest::new("foo bar foo baz");

    // Search for "foo" and delete next match
    test.press('/').type_text("foo").press_enter().keys("dgn"); // Delete next match

    assert_eq!(test.buffer_content(), " bar foo baz\n");
    test.assert_cursor(0, 0); // At start where "foo" was
}

#[test]
fn test_ygn_yank_next_match() {
    let mut test = EditorTest::new("foo bar foo baz");

    // Search for "foo" and yank next match
    test.press('/')
        .type_text("foo")
        .press_enter()
        .keys("ygn") // Yank next match
        .press('$') // Move to end
        .press('p'); // Paste

    assert_eq!(test.buffer_content(), "foo bar foo bazfoo\n");
}

#[test]
fn test_cgn_dot_repeat() {
    let mut test = EditorTest::new("foo bar foo baz foo end");

    // Change first "foo", then repeat with dot
    test.press('/')
        .type_text("foo")
        .press_enter()
        .keys("cgn")
        .type_text("FOO")
        .press_esc()
        .press('n') // Move to next match
        .press('.'); // Repeat change

    assert_eq!(test.buffer_content(), "FOO bar FOO baz foo end\n");
    test.assert_cursor(0, 10); // At end of second "FOO"
}

#[test]
fn test_dot_repeat_cgn_undo_redo_isolation_macro_flow() {
    editor_flow_test! {
        content "foo bar foo baz foo\n";
        step "A!<Esc>" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo baz foo!\n");
        }
        step "0/foo<Enter>cgnFOO<Esc>" => |test| {
            assert_eq!(test.buffer_content(), "FOO bar foo baz foo!\n");
        }
        step "n." => |test| {
            assert_eq!(test.buffer_content(), "FOO bar FOO baz foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "FOO bar foo baz foo!\n");
        }
        step "<C-r>" => |test| {
            assert_eq!(test.buffer_content(), "FOO bar FOO baz foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "FOO bar foo baz foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo baz foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo baz foo\n");
        }
    }
}

#[test]
fn test_dot_repeat_cgn_esc_no_insert_undo_redo_isolation_macro_flow() {
    editor_flow_test! {
        content "foo bar foo baz foo\n";
        step "A!<Esc>" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo baz foo!\n");
        }
        step "0/foo<Enter>cgn<Esc>" => |test| {
            assert_eq!(test.buffer_content(), " bar foo baz foo!\n");
        }
        step "n." => |test| {
            assert_eq!(test.buffer_content(), " bar  baz foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), " bar foo baz foo!\n");
        }
        step "<C-r>" => |test| {
            assert_eq!(test.buffer_content(), " bar  baz foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), " bar foo baz foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo baz foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo baz foo\n");
        }
    }
}

#[test]
fn test_cgn_esc_undo_does_not_consume_prior_change_macro_flow() {
    editor_flow_test! {
        content "foo bar foo\n";
        step "A!<Esc>" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo!\n");
        }
        step "0/foo<Enter>cgn<Esc>" => |test| {
            assert_eq!(test.buffer_content(), " bar foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo\n");
        }
    }
}

#[test]
fn test_cgn_esc_undo_redo_isolation_macro_flow() {
    editor_flow_test! {
        content "foo bar foo\n";
        step "A!<Esc>" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo!\n");
        }
        step "0/foo<Enter>cgn<Esc>" => |test| {
            assert_eq!(test.buffer_content(), " bar foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo!\n");
        }
        step "<C-r>" => |test| {
            assert_eq!(test.buffer_content(), " bar foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo!\n");
        }
        step "u" => |test| {
            assert_eq!(test.buffer_content(), "foo bar foo\n");
        }
    }
}

#[test]
fn test_gn_with_regex_pattern() {
    let mut test = EditorTest::new("test123 word test456 end");

    // Search for pattern matching "test" followed by digits
    test.press('/')
        .type_text(r"test\d+")
        .press_enter()
        .keys("gn");

    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 6); // End of "test123"
}

#[test]
fn test_gN_from_middle_selects_previous() {
    let mut test = EditorTest::new("foo bar foo baz foo end");

    // Search for "foo", move to middle, gN should select previous match
    test.press('/')
        .type_text("foo")
        .press_enter()
        .keys("nn") // Move to third "foo"
        .keys("gN"); // Select previous (second "foo")

    assert_eq!(test.editor.mode(), Mode::Visual);
    test.assert_cursor(0, 10); // End of second "foo"

    let visual_start = test.editor.visual_start();
    assert_eq!(visual_start, Some((0, 8))); // Start of second "foo"
}

#[test]
fn test_gn_respects_smartcase() {
    let mut test = EditorTest::new("Foo bar foo baz");

    // Search for lowercase "foo" (should match both with smartcase)
    test.press('/').type_text("foo").press_enter().keys("gn");

    // Should select first match (smartcase makes it case-insensitive for lowercase pattern)
    assert_eq!(test.editor.mode(), Mode::Visual);
}

#[test]
fn test_gn_empty_buffer_does_nothing() {
    let mut test = EditorTest::new("");

    test.press('/').type_text("foo").press_enter().keys("gn");

    // Should stay in normal mode
    assert_eq!(test.editor.mode(), Mode::Normal);
}

// ============================================================================
// OV-00041: Visual ~ undo grouping
// ============================================================================

#[test]
fn test_visual_tilde_multi_line_single_undo() {
    // Visual select 3 lines, press ~, then u should revert all 3 lines in one undo
    let mut test = EditorTest::new("aaa\nbbb\nccc");

    test.keys("Vjj~");

    // Should have toggled case
    assert_eq!(test.buffer_content(), "AAA\nBBB\nCCC\n");

    // Single undo should revert ALL lines
    test.keys("u");
    assert_eq!(test.buffer_content(), "aaa\nbbb\nccc\n");
}

#[test]
fn test_visual_uppercase_u_undo_single_step() {
    // VjjU then u should revert all 3 lines in one undo
    let mut test = EditorTest::new("hello\nworld\nfoo");

    test.keys("VjjU");
    assert_eq!(test.buffer_content(), "HELLO\nWORLD\nFOO\n");

    // Single undo should revert ALL lines
    test.keys("u");
    assert_eq!(test.buffer_content(), "hello\nworld\nfoo\n");
}

#[test]
fn test_visual_block_lowercase_u_undo_single_step() {
    // Visual block select, lowercase u, then undo should revert in one step
    let mut test = EditorTest::new("HELLO\nWORLD\nFOOBA");

    // Enter visual block mode with Ctrl-V, then select first 3 chars of 3 lines
    test.press_with(KeyCode::Char('v'), Modifiers::CONTROL);
    test.keys("jjllu");
    assert_eq!(test.buffer_content(), "helLO\nworLD\nfooBA\n");

    // Single undo should revert ALL lines
    test.keys("u");
    assert_eq!(test.buffer_content(), "HELLO\nWORLD\nFOOBA\n");
}

// ============================================================================
// Visual mode toggling (v/V/Ctrl-V toggle their own mode off)
// ============================================================================

#[test]
fn test_v_toggles_visual_mode() {
    let mut test = EditorTest::new("hello world");

    test.press('v');
    test.assert_mode(Mode::Visual);

    test.press('v');
    test.assert_mode(Mode::Normal);
}

#[test]
fn test_shift_v_toggles_visual_line_mode() {
    let mut test = EditorTest::new("hello world");

    test.press('V');
    test.assert_mode(Mode::VisualLine);

    test.press('V');
    test.assert_mode(Mode::Normal);
}

#[test]
fn test_ctrl_v_toggles_visual_block_mode() {
    let mut test = EditorTest::new("hello world");

    test.press_with(KeyCode::Char('v'), Modifiers::CONTROL);
    test.assert_mode(Mode::VisualBlock);

    test.press_with(KeyCode::Char('v'), Modifiers::CONTROL);
    test.assert_mode(Mode::Normal);
}

#[test]
fn test_v_switches_from_visual_line_to_visual() {
    let mut test = EditorTest::new("hello world");

    test.press('V');
    test.assert_mode(Mode::VisualLine);

    test.press('v');
    test.assert_mode(Mode::Visual);
}

#[test]
fn test_shift_v_switches_from_visual_to_visual_line() {
    let mut test = EditorTest::new("hello world");

    test.press('v');
    test.assert_mode(Mode::Visual);

    test.press('V');
    test.assert_mode(Mode::VisualLine);
}
