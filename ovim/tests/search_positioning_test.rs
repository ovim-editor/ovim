//! Where `/`, `?`, `n`, `N`, `*` and `#` put the cursor, including counts and
//! wrap-around. Every expectation was produced with `nvim --clean --headless`
//! (`normal! {keys}`; cursor reported as (line - 1, column)).

mod helpers;
use helpers::EditorTest;

fn at(content: &str, keys: &str) -> (usize, usize) {
    let mut test = EditorTest::new(content);
    test.keys(keys);
    test.cursor()
}

#[test]
fn slash_goes_to_the_next_match_even_when_on_a_match() {
    // nvim: "foo x foo y foo": `/foo<CR>` from the match at 0 -> 6; again -> 12.
    assert_eq!(at("foo x foo y foo\n", "/foo<CR>"), (0, 6));
    assert_eq!(at("foo x foo y foo\n", "/foo<CR>/foo<CR>"), (0, 12));
    // Incremental preview must not shift the start: `/fo<CR>` is the same.
    assert_eq!(at("foo x foo y foo\n", "/fo<CR>"), (0, 6));
    // `n` after it goes on, wrapping to the first match.
    assert_eq!(at("foo x foo y foo\n", "/foo<CR>n"), (0, 12));
    assert_eq!(at("foo x foo y foo\n", "/foo<CR>nn"), (0, 0));
    // nvim: `/<CR>` repeats the last pattern.
    assert_eq!(at("foo x foo y foo\n", "/foo<CR>/<CR>"), (0, 12));
}

#[test]
fn search_wraps_onto_the_cursor_line() {
    // nvim: the only match is earlier on the cursor line -> wrap to it.
    assert_eq!(at("foo bar\n", "w/foo<CR>"), (0, 0));
    assert_eq!(at("a foo b c d\n", "$/foo<CR>"), (0, 2));
    // nvim: `?` wraps to a match later on the cursor line.
    assert_eq!(at("x foo\ny\n", "?foo<CR>"), (0, 2));
    assert_eq!(at("x foo\ny\n", "$?foo<CR>"), (0, 2));
    // nvim: the only match is under the cursor -> it wraps to itself.
    assert_eq!(at("foo x\n", "/foo<CR>n"), (0, 0));
}

#[test]
fn question_mark_searches_back_from_where_it_started() {
    // nvim: `G?foo<CR>` lands on the last match before the cursor, not the one before that.
    assert_eq!(at("a foo\nb\nc foo\nd foo e\nlast\n", "G?foo<CR>"), (3, 2));
    // nvim: `$?fo<CR>` -> 12, `$?foo<BS><CR>` -> 12.
    assert_eq!(at("foo x foo y foo\n", "$?fo<CR>"), (0, 12));
    assert_eq!(at("foo x foo y foo\n", "$?foo<BS><CR>"), (0, 12));
    // nvim: on a match, `?foo` goes to the previous one.
    assert_eq!(at("foo x foo y foo\n", "$b?foo<CR>"), (0, 6));
    assert_eq!(at("foo x foo y foo\n", "wwl?foo<CR>"), (0, 6));
    // nvim: a match that starts before the cursor counts even if it extends past it.
    assert_eq!(at("xfoobar y\n", "llllll?foobar<CR>"), (0, 1));
    // nvim: `?<CR>` repeats backward from the cursor, `/<CR>` forward after a `?`.
    assert_eq!(at("foo x foo y foo\n", "$?foo<CR>?<CR>"), (0, 6));
    assert_eq!(at("foo x foo y foo\n", "$?foo<CR>/<CR>"), (0, 0));
}

#[test]
fn n_and_n_step_over_adjacent_matches_and_follow_the_direction() {
    // nvim: "aaaa": `$?a<CR>` -> 2, then `n` -> 1; `/a<CR>n` -> 2.
    assert_eq!(at("aaaa\n", "$?a<CR>"), (0, 2));
    assert_eq!(at("aaaa\n", "$?a<CR>n"), (0, 1));
    assert_eq!(at("aaaa\n", "/a<CR>n"), (0, 2));
    // nvim: after `?`, `n` goes backward and `N` forward.
    assert_eq!(at("foo x foo y foo\n", "$?foo<CR>n"), (0, 6));
    assert_eq!(at("foo x foo y foo\n", "$?foo<CR>N"), (0, 0));
    assert_eq!(at("foo x foo y foo\n", "/foo<CR>N"), (0, 0));
}

#[test]
fn counts_work_for_search_commands_and_do_not_leak() {
    let content = "foo a foo b foo c foo\n";
    // nvim: `2N` after `$?foo` -> 6; `3N` after `/foo` -> 12.
    assert_eq!(at(content, "$?foo<CR>2N"), (0, 6));
    assert_eq!(at(content, "/foo<CR>3N"), (0, 12));
    // nvim: `3*` -> 18; `$2#` -> 6; `*`,`**`,`#`,`##`.
    assert_eq!(at(content, "3*"), (0, 18));
    assert_eq!(at(content, "$2#"), (0, 6));
    assert_eq!(at(content, "*"), (0, 6));
    assert_eq!(at(content, "**"), (0, 12));
    assert_eq!(at(content, "#"), (0, 18));
    assert_eq!(at(content, "##"), (0, 12));
    // nvim: `2/foo<CR>` and `$2?foo<CR>` -> 12.
    assert_eq!(at(content, "2/foo<CR>"), (0, 12));
    assert_eq!(at(content, "$2?foo<CR>"), (0, 12));

    // nvim: `3n` consumes its count: the following `x` deletes one character.
    let mut test = EditorTest::new("one two\nthree two\nfour two\nfive two\n");
    test.keys("/two<CR>3nx");
    assert_eq!(test.cursor(), (3, 5));
    assert_eq!(
        test.buffer_content(),
        "one two\nthree two\nfour two\nfive wo\n"
    );
    let mut test = EditorTest::new(content);
    test.keys("3*x");
    assert_eq!(test.buffer_content(), "foo a foo b foo c oo\n");
}

#[test]
fn star_and_hash_start_from_the_word_under_or_after_the_cursor() {
    // nvim: `#` from the middle of a word goes to the other occurrence.
    assert_eq!(at("a foo b foo\n", "wl#"), (0, 8));
    // nvim: on blanks `*` takes the next word on the line: "   foo y foo", `*` -> 9.
    assert_eq!(at("   foo y foo\n", "*"), (0, 9));
}
