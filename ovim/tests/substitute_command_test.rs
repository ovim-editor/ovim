mod helpers;
use helpers::EditorTest;

#[test]
fn substitute_with_s_in_replacement() {
    // Bug: rfind('s') found the 's' in "best" instead of the command 's'.
    let mut test = EditorTest::new("test line");
    test.command(":%s/test/best/g");
    assert_eq!(test.buffer_content(), "best line\n");
}

#[test]
fn substitute_with_s_in_pattern() {
    let mut test = EditorTest::new("stars are bright");
    test.command(":%s/stars/bars/g");
    assert_eq!(test.buffer_content(), "bars are bright\n");
}

#[test]
fn substitute_multiple_s_in_content() {
    let mut test = EditorTest::new("sys systems system");
    test.command(":%s/sys/os/g");
    assert_eq!(test.buffer_content(), "os ostems ostem\n");
}

#[test]
fn substitute_on_current_line() {
    let mut test = EditorTest::new("foo\nbar\nfoo");
    test.keys("j"); // move to line 2
    test.command(":s/bar/baz/");
    assert_eq!(test.buffer_content(), "foo\nbaz\nfoo\n");
}

#[test]
fn substitute_with_range() {
    let mut test = EditorTest::new("aaa\nbbb\nccc\nddd");
    test.command(":2,3s/[bc]/x/g");
    assert_eq!(test.buffer_content(), "aaa\nxxx\nxxx\nddd\n");
}

#[test]
fn substitute_empty_pattern_reuses_last_search() {
    let mut test = EditorTest::new("hello world\nhello there");
    // First, do a search for "hello"
    test.keys("/hello<CR>");
    // Now substitute with empty pattern — should reuse "hello"
    test.command(":%s//goodbye/g");
    assert_eq!(test.buffer_content(), "goodbye world\ngoodbye there\n");
}

#[test]
fn substitute_empty_pattern_without_prior_search() {
    let mut test = EditorTest::new("hello world");
    // No prior search — should show error, buffer unchanged
    test.command(":%s//bar/g");
    assert_eq!(test.buffer_content(), "hello world\n");
}

#[test]
fn global_substitute_converts_backrefs() {
    let mut test = EditorTest::new("foo123\nbar456\nbaz789");
    // Use :g with substitute that has a capture group. Vim capture refs in the
    // replacement are `\1`/`\2` (a literal `$` is literal text, e.g. dollar
    // amounts), so use the Vim spelling here.
    test.command(":g/[0-9]/s/([a-z]+)([0-9]+)/\\2\\1/");
    assert_eq!(test.buffer_content(), "123foo\n456bar\n789baz\n");
}

#[test]
fn substitute_without_g_replaces_first_only() {
    let mut test = EditorTest::new("aaa bbb aaa");
    test.command(":%s/aaa/xxx/");
    assert_eq!(test.buffer_content(), "xxx bbb aaa\n");
}

#[test]
fn substitute_with_g_replaces_all() {
    let mut test = EditorTest::new("aaa bbb aaa");
    test.command(":%s/aaa/xxx/g");
    assert_eq!(test.buffer_content(), "xxx bbb xxx\n");
}

#[test]
fn substitute_empty_replacement() {
    let mut test = EditorTest::new("hello world");
    test.command(":%s/hello //");
    assert_eq!(test.buffer_content(), "world\n");
}

#[test]
fn substitute_case_insensitive() {
    let mut test = EditorTest::new("Hello HELLO hello");
    test.command(":%s/hello/hi/gi");
    assert_eq!(test.buffer_content(), "hi hi hi\n");
}
