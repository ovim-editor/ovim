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

// nvim --clean: `:%s/[0-9]//gn` on "a1b2", "c3", "d" reports
// "3 matches on 2 lines" and leaves the buffer, cursor and modified flag alone.
#[test]
fn n_flag_counts_matches_without_editing() {
    let mut test = EditorTest::new("a1b2\nc3\nd\n");
    test.command("%s/[0-9]//gn");
    assert_eq!(test.buffer_content(), "a1b2\nc3\nd\n");
    assert_eq!(test.editor.status_message(), "3 matches on 2 lines");
    assert!(!test.editor.is_modified());
    test.assert_cursor(0, 0);
}

// nvim --clean: without `g` only the first match of each line counts
// ("2 matches on 2 lines"), a single match reads "1 match on 1 line", and the
// cursor stays where it was.
#[test]
fn n_flag_counts_one_match_per_line_without_g() {
    let mut test = EditorTest::new("a1b2\nc3\nd\n");
    test.keys("j");
    test.command("%s/[0-9]//n");
    assert_eq!(test.editor.status_message(), "2 matches on 2 lines");
    test.assert_cursor(1, 0);
    test.command("%s/1//n");
    assert_eq!(test.editor.status_message(), "1 match on 1 line");
    assert_eq!(test.buffer_content(), "a1b2\nc3\nd\n");
}

// nvim --clean: `:%s/z//n` is "E486: Pattern not found: z"; with the `e`
// flag it is silent. `n` ignores `c` (no prompt) and works in a
// nomodifiable buffer.
#[test]
fn n_flag_reports_e486_unless_e_and_ignores_confirm_and_modifiable() {
    let mut test = EditorTest::new("a1b2\nc3\nd\n");
    test.command("%s/z//n");
    assert_eq!(test.editor.status_message(), "E486: Pattern not found: z");
    test.command("%s/z//ne");
    assert_eq!(test.buffer_content(), "a1b2\nc3\nd\n");
    test.command("set nomodifiable");
    test.command("%s/1//ngc");
    assert_eq!(test.editor.status_message(), "1 match on 1 line");
    test.assert_mode(ovim::mode::Mode::Normal);
}

// nvim --clean: `:%s/,/ | /g` on "a,b,c" gives "a | b | c": a bar inside the
// replacement is text, not a command separator.
#[test]
fn bar_in_the_replacement_is_literal() {
    let mut test = EditorTest::new("a,b,c\n");
    test.command("%s/,/ | /g");
    assert_eq!(test.buffer_content(), "a | b | c\n");
}

// nvim --clean: an unterminated replacement owns the rest of the line:
// `:s/a/X|Y` turns "ab" into "X|Yb".
#[test]
fn unterminated_replacement_owns_the_rest_of_the_line() {
    let mut test = EditorTest::new("ab\n");
    test.command("s/a/X|Y");
    assert_eq!(test.buffer_content(), "X|Yb\n");
}

// nvim --clean: a bar after the closing delimiter still chains commands:
// `:s/a/X/ | s/b/Z/` gives "XZ".
#[test]
fn bar_after_the_closing_delimiter_chains_commands() {
    let mut test = EditorTest::new("ab\n");
    test.command("s/a/X/ | s/b/Z/");
    assert_eq!(test.buffer_content(), "XZ\n");
    let mut test = EditorTest::new("a b\n");
    test.command("s/a/X/g|s/b/Y/");
    assert_eq!(test.buffer_content(), "X Y\n");
}

// nvim --clean: inside `:g/b/`, an empty `:s` pattern is the :g pattern, so
// `:g/b/s//X/` on "abc", "b", "zzz" gives "aXc", "X", "zzz" (not E35).
#[test]
fn global_sets_the_pattern_an_empty_substitute_pattern_reuses() {
    let mut test = EditorTest::new("abc\nb\nzzz\n");
    test.command("g/b/s//X/");
    assert_eq!(test.buffer_content(), "aXc\nX\nzzz\n");
}

// nvim --clean: after `/zzz`, `:2s/b/B/` makes `b` the last search pattern, so
// `n` from line 2 lands on the next `b` (line 3) instead of searching `zzz`.
#[test]
fn substitute_becomes_the_pattern_of_n() {
    let mut test = EditorTest::new("x\nab\nb\nzzz\nb\n");
    test.keys("/zzz<CR>gg");
    test.command("2s/b/B/");
    test.keys("n");
    test.assert_cursor(2, 0);
}

// nvim --clean: `:g/b/y` makes `b` the last search pattern: from the top,
// `n` goes to the first line with a `b`.
#[test]
fn global_becomes_the_pattern_of_n() {
    let mut test = EditorTest::new("x\nb1\nzzz\nb2\n");
    test.keys("/zzz<CR>gg");
    test.command("g/b/y");
    test.keys("n");
    test.assert_cursor(1, 0);
}

// nvim --clean: `:g/b/s/a/X/` on "ab", "b", "b" changes one line, and the
// message counts that line, not the three that matched `b`.
#[test]
fn global_substitute_reports_the_lines_it_changed() {
    let mut test = EditorTest::new("ab\nb\nb\n");
    test.command("g/b/s/a/X/");
    assert_eq!(test.buffer_content(), "Xb\nb\nb\n");
    assert_eq!(test.editor.status_message(), "Substituted on 1 line(s)");
}
