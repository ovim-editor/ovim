//! Tests for multi-line substitute replacements (`\r` in the replacement) and
//! the line-shift bug they used to trigger (OV-00244): the substitution loop
//! iterated top-down, so a replacement that split a line pushed all later
//! lines down and the loop then processed the freshly inserted continuation
//! lines instead of the originally targeted ones.

mod helpers;
use helpers::EditorTest;

// ---- \r and \t replacement escapes ----------------------------------------

#[test]
fn backslash_r_splits_line() {
    let mut test = EditorTest::new("a,b");
    test.command("s/,/\\r/");
    assert_eq!(test.buffer_content(), "a\nb\n");
}

#[test]
fn backslash_t_inserts_tab() {
    let mut test = EditorTest::new("a b");
    test.command("s/ /\\t/");
    assert_eq!(test.buffer_content(), "a\tb\n");
}

#[test]
fn backslash_r_with_backrefs() {
    // Swap around a line break: "key=value" -> "value\nkey".
    let mut test = EditorTest::new("key=value");
    test.command("s/(\\w+)=(\\w+)/\\2\\r\\1/");
    assert_eq!(test.buffer_content(), "value\nkey\n");
}

// ---- OV-00244: multi-line replacement over a range ------------------------

#[test]
fn multiline_replacement_hits_every_ranged_line() {
    // Pre-fix, the top-down loop replaced line 1, which pushed the other foos
    // down; the loop then scanned the new continuation line and skipped the
    // original targets.
    let mut test = EditorTest::new("foo\nfoo\nfoo");
    test.command("%s/foo/bar\\rbaz/");
    assert_eq!(test.buffer_content(), "bar\nbaz\nbar\nbaz\nbar\nbaz\n");
}

#[test]
fn multiline_replacement_with_g_flag_multiple_per_line() {
    let mut test = EditorTest::new("a,b,c\nx,y");
    test.command("%s/,/\\r/g");
    assert_eq!(test.buffer_content(), "a\nb\nc\nx\ny\n");
}

#[test]
fn multiline_replacement_respects_range() {
    let mut test = EditorTest::new("foo\nfoo\nfoo");
    test.command("1,2s/foo/a\\rb/");
    assert_eq!(test.buffer_content(), "a\nb\na\nb\nfoo\n");
}

#[test]
fn global_command_substitute_multiline() {
    // :g routes substitution through its own loop; it must survive line splits
    // on earlier matching lines too.
    let mut test = EditorTest::new("foo 1\nskip\nfoo 2");
    test.command("g/foo/s/foo/x\\ry/");
    assert_eq!(test.buffer_content(), "x\ny 1\nskip\nx\ny 2\n");
}

// ---- interactive /gc with multi-line replacements --------------------------

#[test]
fn confirm_all_multiline_replacements() {
    let mut test = EditorTest::new("foo foo\nfoo");
    test.command("%s/foo/A\\rB/gc");
    test.keys("a"); // confirm all
    assert_eq!(test.buffer_content(), "A\nB A\nB\nA\nB\n");
}

#[test]
fn confirm_then_skip_multiline() {
    let mut test = EditorTest::new("foo foo");
    test.command("s/foo/A\\rB/gc");
    test.keys("y"); // confirm first -> splits the line
    test.keys("n"); // skip second; buffer must not be corrupted afterwards
    assert_eq!(test.buffer_content(), "A\nB foo\n");
}

#[test]
fn skip_then_confirm_multiline() {
    let mut test = EditorTest::new("foo foo");
    test.command("s/foo/A\\rB/gc");
    test.keys("n"); // skip first
    test.keys("y"); // confirm second — offsets still against the original line
    assert_eq!(test.buffer_content(), "foo A\nB\n");
}
