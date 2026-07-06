//! Regression tests for ex-command bugs found in the bug hunt:
//!  - :t/:m to address 0 insert at the top of the file
//!  - :t/:m without a space (:3t0) are recognized
//!  - ranges beginning with a mark (:'a,'bd) parse correctly
//!  - empty search pattern repeats the last search

#![allow(non_snake_case)]

mod helpers;
use helpers::EditorTest;

#[test]
fn test_copy_to_address_zero_inserts_at_top() {
    let mut test = EditorTest::new("a\nb\nc\n");
    test.command("3t0"); // copy line 3 ("c") to the very top
    assert_eq!(test.buffer_content(), "c\na\nb\nc\n");
}

#[test]
fn test_move_to_address_zero_inserts_at_top() {
    let mut test = EditorTest::new("a\nb\nc\nd\n");
    test.command("3m0"); // move line 3 ("c") to the very top
    assert_eq!(test.buffer_content(), "c\na\nb\nd\n");
}

#[test]
fn test_copy_no_space_form() {
    let mut test = EditorTest::new("a\nb\nc\n");
    test.command("1t2"); // copy line 1 to after line 2
    assert_eq!(test.buffer_content(), "a\nb\na\nc\n");
}

#[test]
fn test_copy_with_space_still_works() {
    let mut test = EditorTest::new("a\nb\nc\n");
    test.command("1t 2");
    assert_eq!(test.buffer_content(), "a\nb\na\nc\n");
}

#[test]
fn test_move_no_space_form() {
    let mut test = EditorTest::new("a\nb\nc\nd\n");
    test.command("1m2"); // move line 1 ("a") to after line 2 ("b") -> b,a,c,d
    assert_eq!(test.buffer_content(), "b\na\nc\nd\n");
}

#[test]
fn test_range_starting_with_mark_delete() {
    let mut test = EditorTest::new("l0\nl1\nl2\nl3\nl4\n");
    // set mark a on line 1, mark b on line 3
    test.keys("j");
    test.keys("ma");
    test.keys("jj");
    test.keys("mb");
    // :'a,'bd deletes lines 1..3 (l1,l2,l3)
    test.command("'a,'bd");
    assert_eq!(test.buffer_content(), "l0\nl4\n");
}

#[test]
fn test_address_single_mark_jumps() {
    let mut test = EditorTest::new("l0\nl1\nl2\nl3\n");
    test.keys("jjj"); // cursor line 3
    test.keys("ma");
    test.keys("gg"); // back to top
    test.command("'a"); // jump to mark a
    assert_eq!(test.cursor().0, 3, "':a should jump to mark a's line");
}

#[test]
fn test_interactive_substitute_same_line_length_change() {
    // :s/a/XX/gc on "aaa" — confirming all three matches must not corrupt the
    // buffer when the replacement is longer than the match.
    let mut test = EditorTest::new("aaa");
    test.command("s/a/XX/gc");
    test.keys("a"); // confirm all remaining
    assert_eq!(test.buffer_content(), "XXXXXX\n");
}

#[test]
fn test_interactive_substitute_shorter_replacement() {
    let mut test = EditorTest::new("aa aa aa");
    test.command("s/aa/b/gc");
    test.keys("a");
    assert_eq!(test.buffer_content(), "b b b\n");
}

#[test]
fn test_interactive_substitute_individual_confirm() {
    // Confirm first, skip second, confirm third with differing length.
    let mut test = EditorTest::new("a a a");
    test.command("s/a/XX/gc");
    test.keys("y"); // first -> XX
    test.keys("n"); // skip second
    test.keys("y"); // third -> XX
    assert_eq!(test.buffer_content(), "XX a XX\n");
}

#[test]
fn test_empty_search_repeats_last() {
    let mut test = EditorTest::new("b x b x b");
    test.keys("/b<CR>"); // search for b, lands on next b
    // Now an empty search should repeat, not wipe the search.
    test.keys("/<CR>");
    // n should still work: find another b
    let before = test.cursor();
    test.keys("n");
    assert_ne!(test.cursor(), before, "n should still advance after an empty-pattern search");
}
