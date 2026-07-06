//! Regression tests for linewise-register and count-paste bugs found in the
//! bug hunt:
//!  - yj/yk/y}/y{ must keep line terminators (else pasted lines glue together)
//!  - `[count]p` of a linewise register must produce `count` separate lines
//!  - blockwise `[count]p` repeats each row horizontally, not vertically
//!  - dip/dap store a linewise register

#![allow(non_snake_case)]

mod helpers;
use helpers::EditorTest;

#[test]
fn test_yj_register_keeps_newlines() {
    let mut test = EditorTest::new("foo\nbar\nbaz");
    test.keys("yj");
    assert_eq!(
        test.get_register_content('"').as_deref(),
        Some("foo\nbar\n"),
        "yj should yank two whole lines with terminators"
    );
}

#[test]
fn test_yj_then_paste_inserts_two_lines() {
    let mut test = EditorTest::new("foo\nbar\nbaz");
    test.keys("yjGp");
    assert_eq!(test.buffer_content(), "foo\nbar\nbaz\nfoo\nbar\n");
}

#[test]
fn test_yk_register_keeps_newlines() {
    let mut test = EditorTest::new("foo\nbar\nbaz");
    test.keys("jyk"); // cursor on "bar", yank up to "foo"
    assert_eq!(
        test.get_register_content('"').as_deref(),
        Some("foo\nbar\n")
    );
}

#[test]
fn test_y_paragraph_forward_keeps_newlines() {
    let mut test = EditorTest::new("a\nb\nc\n\nd");
    test.keys("y}"); // yank paragraph forward from line 0
    let reg = test.get_register_content('"').unwrap();
    // Must contain the internal separators, not "abc"
    assert!(reg.contains("a\nb\nc"), "y}} register was {reg:?}");
    assert!(
        reg.ends_with('\n'),
        "linewise register must end with newline"
    );
}

#[test]
fn test_count_paste_linewise_no_trailing_newline_splits_lines() {
    // `S` leaves a linewise register without a trailing newline; 2p must still
    // produce two separate lines, not one merged "NEWNEW".
    let mut test = EditorTest::new("hello\nworld");
    test.keys("SNEW<Esc>"); // substitute line 0 -> "NEW", register = "hello" linewise
    test.keys("2p");
    assert_eq!(
        test.buffer_content(),
        "NEW\nhello\nhello\nworld\n",
        "2p of a linewise register must paste two separate lines"
    );
}

#[test]
fn test_count_paste_yy_two_copies() {
    let mut test = EditorTest::new("one\ntwo");
    test.keys("yy2p");
    assert_eq!(test.buffer_content(), "one\none\none\ntwo\n");
}

#[test]
fn test_dap_stores_linewise_register() {
    let mut test = EditorTest::new("foo\nbar\n\nqux");
    test.keys("dap"); // delete a paragraph (first two lines + blank)
                      // Now paste it back below — it should come back as whole lines, not spliced
    let reg = test.get_register_content('"').unwrap();
    assert!(reg.starts_with("foo\nbar"), "dap register was {reg:?}");
    // Paste and confirm it lands as separate lines, not spliced into "qux"
    test.keys("p");
    let content = test.buffer_content();
    assert!(
        content.contains("foo\nbar"),
        "dap then p should restore whole lines, got {content:?}"
    );
    assert!(
        !content.contains("qfoo"),
        "paragraph must not splice into the middle of a line, got {content:?}"
    );
}

#[test]
fn test_blockwise_count_paste_repeats_horizontally() {
    // Yank a 2x2 block "ab"/"de", then 3p repeats each row 3x horizontally.
    let mut test = EditorTest::new("abc\ndef\nghi");
    test.keys("<C-v>jly"); // block select cols 0..1 over 2 rows -> "ab","de"
    assert_eq!(test.get_register_content('"').as_deref(), Some("ab\nde"));
    // cursor back at 0,0; paste-after with count 2
    test.set_cursor(0, 0);
    test.keys("2p");
    // Row 0: "abc" with "abab" inserted after col 0 -> "aababbc"
    // Row 1: "def" with "dede" inserted after col 0 -> "ddedeef"
    // Row 2 unchanged.
    let content = test.buffer_content();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "block paste must not add extra lines, got {content:?}"
    );
    assert_eq!(lines[0], "aababbc", "got {content:?}");
    assert_eq!(lines[1], "ddedeef", "got {content:?}");
    assert_eq!(lines[2], "ghi");
}
