//! Regression tests for :s substitute parsing bugs found in the bug hunt.

#![allow(non_snake_case)]

mod helpers;
use helpers::EditorTest;

#[test]
fn test_backref_followed_by_word_char() {
    // \1 immediately followed by a word char must not swallow it into the group name.
    // (ovim uses Rust-regex pattern syntax, so capture groups are `(a)`.)
    let mut test = EditorTest::new("abc");
    test.command("s/(a)/\\1x/");
    assert_eq!(test.buffer_content(), "axbc\n", "\\1x should expand group 1 then literal x");
}

#[test]
fn test_backref_followed_by_underscore() {
    let mut test = EditorTest::new("foo");
    test.command("s/(f)/\\1_/");
    assert_eq!(test.buffer_content(), "f_oo\n");
}

#[test]
fn test_literal_dollar_in_replacement() {
    // $5 in the replacement is a literal, not a capture reference.
    let mut test = EditorTest::new("price x here");
    test.command("s/x/$5/");
    assert_eq!(test.buffer_content(), "price $5 here\n");
}

#[test]
fn test_ampersand_whole_match() {
    let mut test = EditorTest::new("cat");
    test.command("s/cat/[&]/");
    assert_eq!(test.buffer_content(), "[cat]\n", "& should expand to the whole match");
}

#[test]
fn test_escaped_delimiter_in_pattern() {
    // :s/a\/b/X/ substitutes the literal text a/b
    let mut test = EditorTest::new("x a/b y");
    test.command("s/a\\/b/X/");
    assert_eq!(test.buffer_content(), "x X y\n");
}

#[test]
fn test_escaped_delimiter_in_replacement() {
    let mut test = EditorTest::new("a-b");
    test.command("s/-/\\//");
    assert_eq!(test.buffer_content(), "a/b\n", "\\/ in replacement is a literal slash");
}

#[test]
fn test_plain_substitute_still_works() {
    let mut test = EditorTest::new("hello world");
    test.command("s/world/there/");
    assert_eq!(test.buffer_content(), "hello there\n");
}

#[test]
fn test_global_flag_still_works() {
    let mut test = EditorTest::new("a a a");
    test.command("s/a/b/g");
    assert_eq!(test.buffer_content(), "b b b\n");
}
