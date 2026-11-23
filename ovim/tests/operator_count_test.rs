//! Counts typed after an operator: `d10j`, `c10l`, and `[n]op[m]motion` (n * m).
//!
//! Every expectation was produced with `nvim --clean --headless` and
//! `normal! {keys}` on the same buffer.

mod helpers;
use helpers::EditorTest;

fn numbered_lines(n: usize) -> String {
    (1..=n).map(|i| format!("l{i}\n")).collect()
}

#[test]
fn zero_continues_a_count_typed_after_the_operator() {
    // nvim: 20 lines, `jjd10j` deletes l3..l13 (11 lines).
    let mut test = EditorTest::new(&numbered_lines(20));
    test.keys("jjd10j");
    assert_eq!(
        test.buffer_content(),
        "l1\nl2\nl14\nl15\nl16\nl17\nl18\nl19\nl20\n"
    );
    test.assert_cursor(2, 0);
}

#[test]
fn zero_after_the_count_works_for_word_and_char_motions() {
    // nvim: `wwd10w` on thirteen words deletes "gamma ... mu ".
    let mut test =
        EditorTest::new("alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu\n");
    test.keys("wwd10w");
    assert_eq!(test.buffer_content(), "alpha beta nu\n");
    assert_eq!(
        test.get_register_content('"').as_deref(),
        Some("gamma delta epsilon zeta eta theta iota kappa lambda mu ")
    );

    // nvim: `c10lX<Esc>` on the alphabet -> "Xklmnopqrstuvwxyz", register "abcdefghij".
    let mut test = EditorTest::new("abcdefghijklmnopqrstuvwxyz\n");
    test.keys("c10lX<Esc>");
    assert_eq!(test.buffer_content(), "Xklmnopqrstuvwxyz\n");
    assert_eq!(
        test.get_register_content('"').as_deref(),
        Some("abcdefghij")
    );

    // nvim: `d10w` on twelve one-letter words -> "k l".
    let mut test = EditorTest::new("a b c d e f g h i j k l\n");
    test.keys("d10w");
    assert_eq!(test.buffer_content(), "k l\n");
}

#[test]
fn zero_right_after_the_operator_is_still_the_line_start_motion() {
    // nvim: `wwh2d0` on "abc def ghi" -> " ghi" (the count does not turn `0` into a digit).
    let mut test = EditorTest::new("abc def ghi\n");
    test.keys("wwh2d0");
    assert_eq!(test.buffer_content(), " ghi\n");
    let mut test = EditorTest::new("abc def ghi\n");
    test.keys("wwhd0");
    assert_eq!(test.buffer_content(), " ghi\n");
}

#[test]
fn counts_before_and_after_the_operator_multiply() {
    // nvim: `2d3w` on a..j deletes six words -> "g h i j".
    let mut test = EditorTest::new("a b c d e f g h i j\n");
    test.keys("2d3w");
    assert_eq!(test.buffer_content(), "g h i j\n");
    assert_eq!(
        test.get_register_content('"').as_deref(),
        Some("a b c d e f ")
    );

    // nvim: `d3d` and `2d2d` delete 3 and 4 lines.
    let mut test = EditorTest::new(&numbered_lines(20));
    test.keys("d3d");
    assert!(test.buffer_content().starts_with("l4\nl5\n"));
    let mut test = EditorTest::new(&numbered_lines(20));
    test.keys("2d2d");
    assert!(test.buffer_content().starts_with("l5\nl6\n"));

    // nvim: `2y3l` yanks six characters.
    let mut test = EditorTest::new("abcdefghijklmnop\n");
    test.keys("2y3l");
    assert_eq!(test.get_register_content('"').as_deref(), Some("abcdef"));
    // The count was consumed: the next `l` moves by one.
    test.keys("l");
    test.assert_cursor(0, 1);
}

#[test]
fn counts_after_the_operator_alone_still_work() {
    // nvim: `y3w` on "a b c d e f" yanks "a b c ".
    let mut test = EditorTest::new("a b c d e f\n");
    test.keys("y3w");
    assert_eq!(test.get_register_content('"').as_deref(), Some("a b c "));
    // nvim: `3dd`/`d2j` unchanged.
    let mut test = EditorTest::new(&numbered_lines(10));
    test.keys("d2j");
    assert!(test.buffer_content().starts_with("l4\n"));
}

#[test]
fn count_prefix_does_not_leak_into_the_next_command() {
    let mut test = EditorTest::new("abcdefghijklmnop\nxyz\n");
    test.keys("2d3lx");
    // `2d3l` deletes six chars, then a plain `x` deletes exactly one more.
    assert_eq!(test.line_text(0).as_deref(), Some("hijklmnop"));
}
