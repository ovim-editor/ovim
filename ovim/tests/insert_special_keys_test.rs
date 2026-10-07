//! <Del>, <Home> and <End> in Insert, Normal and Visual mode, Ctrl chords that are not bound in
//! Insert mode (they insert nothing), `<C-a>` (insert the previous insert) and `<C-v>` literal insert.
//! Every row was produced with `nvim --clean --headless` (`normal! {keys}`): the buffer, the cursor
//! as (line, column) and the unnamed register.

mod helpers;
use helpers::EditorTest;

/// (content, keys, buffer, cursor, unnamed register)
type Row = (
    &'static str,
    &'static str,
    &'static str,
    (usize, usize),
    Option<&'static str>,
);

fn check(rows: &[Row]) {
    for &(content, keys, buffer, cursor, register) in rows {
        let mut test = EditorTest::new(&format!("{content}\n"));
        test.keys(keys);
        let what = format!("{keys} on {content:?}");
        assert_eq!(test.buffer_content(), buffer, "buffer after {what}");
        assert_eq!(test.cursor(), cursor, "cursor after {what}");
        if let Some(register) = register {
            assert_eq!(
                test.get_register_content('"').as_deref(),
                Some(register),
                "register after {what}"
            );
        }
    }
}

#[test]
fn del_home_end() {
    check(&[
        ("hello", "i<Del>X<Esc>", "Xello\n", (0, 0), None),
        ("hello", "li<Del>X<Esc>", "hXllo\n", (0, 1), None),
        (
            "hello\nworld",
            "A<Del>X<Esc>",
            "helloXworld\n",
            (0, 5),
            None,
        ),
        ("hello", "A<Del>X<Esc>", "helloX\n", (0, 5), None),
        (
            "a👨\u{200d}👩\u{200d}👧\u{200d}👦b",
            "li<Del>X<Esc>",
            "aXb\n",
            (0, 1),
            None,
        ),
        ("hello", "A<Home>X<Esc>", "Xhello\n", (0, 0), None),
        ("hello", "i<End>X<Esc>", "helloX\n", (0, 5), None),
        (
            "hello world",
            "wiab<Home>X<Esc>",
            "Xhello abworld\n",
            (0, 0),
            None,
        ),
        ("hello", "<Del>", "ello\n", (0, 0), Some("h")),
        ("hello", "3<Del>", "hello\n", (0, 0), None),
        ("hello", "$<Home>", "hello\n", (0, 0), None),
        ("hello", "<End>", "hello\n", (0, 4), None),
        ("hello\nworld", "2<End>", "hello\nworld\n", (1, 4), None),
        ("hello", "<Del>p", "ehllo\n", (0, 1), Some("h")),
        ("hello", "lvl<Del>", "hlo\n", (0, 1), Some("el")),
        ("hello", "$v<Home>d", "\n", (0, 0), Some("hello")),
        ("hello world", "vl<End>d", "\n", (0, 0), Some("hello world")),
        ("hello", "i<Del><Del>X<Esc>u", "hello\n", (0, 0), None),
    ]);
}

#[test]
fn unbound_ctrl_chords_insert_nothing() {
    check(&[
        ("hello", "A<C-j>X<Esc>", "hello\nX\n", (1, 0), None),
        ("hello", "A<C-m>X<Esc>", "hello\nX\n", (1, 0), None),
        ("hello", "i<C-i>X<Esc>", "    Xhello\n", (0, 4), None),
    ]);
}

#[test]
fn ctrl_v_inserts_literals() {
    check(&[
        ("hello", "i<C-v>065<Esc>", "Ahello\n", (0, 0), None),
        ("hello", "i<C-v>0651<Esc>", "A1hello\n", (0, 1), None),
        ("hello", "i<C-v>256<Esc>", "ÿhello\n", (0, 0), None),
        ("hello", "i<C-v>x41<Esc>", "Ahello\n", (0, 0), None),
        ("hello", "i<C-v>x4g<Esc>", "ghello\n", (0, 1), None),
        ("hello", "i<C-v>xg<Esc>", "ghello\n", (0, 0), None),
        ("hello", "i<C-v>u20ac<Esc>", "€hello\n", (0, 0), None),
        ("hello", "i<C-v>u20a<Esc>", "Ȋhello\n", (0, 0), None),
        ("hello", "i<C-v>U0001f600<Esc>", "😀hello\n", (0, 0), None),
        ("hello", "i<C-v>o101<Esc>", "Ahello\n", (0, 0), None),
        ("hello", "i<C-v><Tab><Esc>", "\thello\n", (0, 0), None),
        ("hello", "i<C-v>a<Esc>", "ahello\n", (0, 0), None),
        ("hello", "i<C-v><C-a><Esc>", "hello\n", (0, 0), None),
        ("hello", "i<C-v><Esc>X<Esc>", "\x1bXhello\n", (0, 1), None),
        ("hello", "i<C-v>$<Esc>", "$hello\n", (0, 0), None),
    ]);
}

#[test]
fn ctrl_a_inserts_the_previous_insert() {
    check(&[
        (
            "test",
            "ifirst<Esc>i<C-a><Esc>",
            "firsfirstttest\n",
            (0, 8),
            None,
        ),
        ("abc", "iXY<Esc>a<C-a>Z<Esc>", "XYXYZabc\n", (0, 4), None),
    ]);
}
