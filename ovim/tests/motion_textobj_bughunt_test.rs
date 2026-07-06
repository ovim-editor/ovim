//! Regression tests for motion / text-object / macro bugs found in the bug hunt.

#![allow(non_snake_case)]

mod helpers;
use helpers::EditorTest;

// ---- { paragraph-backward onto the target blank line --------------------

#[test]
fn test_paragraph_backward_from_first_content_line_stops_on_blank() {
    // Cursor on the first content line of the 2nd paragraph; { moves to the
    // blank line just above it, not past the whole previous paragraph.
    let mut test = EditorTest::new("a1\na2\n\nb1\nb2\n");
    test.set_cursor(3, 0); // "b1"
    test.keys("{");
    assert_eq!(test.cursor().0, 2, "{{ should land on the blank line above the paragraph");
}

#[test]
fn test_paragraph_backward_from_blank_still_skips_run() {
    // Starting on a blank line, { skips the current blank run to the previous
    // paragraph boundary (unchanged behavior).
    let mut test = EditorTest::new("a1\na2\n\n\nb1\n");
    test.set_cursor(3, 0); // second blank line
    test.keys("{");
    // should move up past the blank run (to line 2 blank or the a-paragraph)
    assert!(test.cursor().0 < 3);
}

// ---- ( sentence-backward at a sentence start ----------------------------

#[test]
fn test_sentence_backward_at_sentence_start() {
    let mut test = EditorTest::new("One. Two. Three.");
    test.set_cursor(0, 5); // start of "Two"
    test.keys("(");
    assert_eq!(test.cursor(), (0, 0), "( at a sentence start goes to the previous sentence");
}

#[test]
fn test_sentence_backward_from_mid_sentence() {
    let mut test = EditorTest::new("One. Two. Three.");
    test.set_cursor(0, 12); // inside "Three"
    test.keys("(");
    assert_eq!(test.cursor(), (0, 10), "( from mid-sentence goes to that sentence's start");
}

// ---- ci"/di" forward search ---------------------------------------------

#[test]
fn test_di_quote_before_quotes_searches_forward() {
    // Cursor before the quotes on the line: di" should still find them forward.
    let mut test = EditorTest::new("foo = \"bar\"");
    test.set_cursor(0, 0); // on 'f', before the quotes
    test.keys("di\"");
    assert_eq!(test.buffer_content(), "foo = \"\"\n", "di\" should delete inside the forward quotes");
}

#[test]
fn test_ci_quote_inside_quotes_still_works() {
    let mut test = EditorTest::new("say \"hi\" now");
    test.set_cursor(0, 6); // inside "hi"
    test.keys("di\"");
    assert_eq!(test.buffer_content(), "say \"\" now\n");
}

// ---- macro recording keeps 'q' typed as an argument ---------------------

#[test]
fn test_macro_records_q_as_find_target() {
    // qa  fq  q  records "fq"; replaying with @a should jump to the next 'q'.
    let mut test = EditorTest::new("aqbqc");
    test.keys("qa"); // start recording into register a
    test.keys("fq"); // find 'q' — cursor moves to first 'q' (col 1)
    test.keys("q"); // stop recording
    assert_eq!(test.cursor(), (0, 1), "fq should have moved to the first q");
    // Move to start and replay
    test.keys("0");
    test.keys("@a");
    assert_eq!(test.cursor(), (0, 1), "replaying macro must repeat fq, landing on a q");
}

#[test]
fn test_macro_records_r_replacement_q() {
    // qa rq q  records "rq" (replace char with 'q'); replay must replace again.
    let mut test = EditorTest::new("abcd");
    test.keys("qa"); // record
    test.keys("rq"); // replace 'a' with 'q'
    test.keys("q"); // stop
    assert_eq!(test.buffer_content(), "qbcd\n");
    test.keys("l"); // move to 'b'
    test.keys("@a"); // replay -> replace 'b' with 'q'
    assert_eq!(test.buffer_content(), "qqcd\n", "macro replay must re-run rq");
}
