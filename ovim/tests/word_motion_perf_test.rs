//! Backward word motions must stay linear on very long non-ASCII lines
//! (each step used to re-walk up to a checkpoint stride of graphemes, which made
//! `b`, `B` and `ge` take around a second on a 200k-character line).

mod helpers;
use helpers::EditorTest;
use std::time::{Duration, Instant};

/// A linear pass takes milliseconds even in a debug build; the old per-step walk took
/// about two seconds per motion there.
const LIMIT: Duration = Duration::from_secs(1);

fn long_line() -> String {
    // One 200k-grapheme "word" of non-ASCII letters, then a short tail.
    format!("{} tail\n", "é".repeat(200_000))
}

fn timed(keys: &str, expect_col: usize) {
    let mut test = EditorTest::new(&long_line());
    let started = Instant::now();
    test.keys(keys);
    let elapsed = started.elapsed();
    assert_eq!(test.cursor(), (0, expect_col), "{keys}");
    assert!(
        elapsed < LIMIT,
        "{keys} took {elapsed:?} on a 200k-character non-ASCII line"
    );
}

#[test]
fn backward_word_motions_are_linear_on_a_long_non_ascii_line() {
    // `$` is on the final `l` of "tail"; `b` goes to the start of "tail", the next
    // `b` crosses the whole 200k-character word back to column 0.
    timed("$bb", 0);
    timed("$BB", 0);
    // `ge` from "tail" lands on the last character of the long word; a second `ge`
    // from the start of the word has nowhere to go.
    timed("$bge", 199_999);
    timed("$bgE", 199_999);
}

#[test]
fn backward_word_motion_results_are_unchanged_on_short_non_ascii_text() {
    let mut test = EditorTest::new("héllo wörld ünï\n");
    test.keys("$b");
    test.assert_cursor(0, 12);
    test.keys("b");
    test.assert_cursor(0, 6);
    test.keys("b");
    test.assert_cursor(0, 0);
    test.keys("$ge");
    test.assert_cursor(0, 10);
    test.keys("gE");
    test.assert_cursor(0, 4);
}
