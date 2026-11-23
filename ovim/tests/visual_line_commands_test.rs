//! Visual-mode `s` is `c`, and `S`, `R`, `C`, `D`, `X` and `Y` act on whole lines (in Visual-block
//! mode `D` and `C` go to the end of the line, and `X` and `Y` are `d` and `y`). Every row was
//! produced with `nvim --clean --headless -s` (the keys typed from a script file): the buffer,
//! the cursor as (line, column) and the unnamed register.

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
fn visual_s_is_c() {
    check(&[
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvesX<Esc>",
            "alpha X\nepsilon zeta\ngamma\n",
            (0, 6),
            Some("beta"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvjsX<Esc>",
            "alpha X zeta\ngamma\n",
            (0, 6),
            Some("beta\nepsilon"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "VsX<Esc>",
            "X\nepsilon zeta\ngamma\n",
            (0, 0),
            Some("alpha beta\n"),
        ),
        (
            "abcd\nabcd\nabcd",
            "l<C-v>jlsX<Esc>",
            "aXd\naXd\nabcd\n",
            (0, 1),
            Some("bc\nbc"),
        ),
    ]);
}

#[test]
fn visual_capital_s_and_r_change_whole_lines() {
    check(&[
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvSX<Esc>",
            "X\nepsilon zeta\ngamma\n",
            (0, 0),
            Some("alpha beta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvjSX<Esc>",
            "X\ngamma\n",
            (0, 0),
            Some("alpha beta\nepsilon zeta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "VjSX<Esc>",
            "X\ngamma\n",
            (0, 0),
            Some("alpha beta\nepsilon zeta\n"),
        ),
        (
            "abcd\nabcd\nabcd",
            "l<C-v>jlSX<Esc>",
            "X\nabcd\n",
            (0, 0),
            Some("abcd\nabcd\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvjRX<Esc>",
            "X\ngamma\n",
            (0, 0),
            Some("alpha beta\nepsilon zeta\n"),
        ),
        (
            "abcd\nabcd\nabcd",
            "l<C-v>jlRX<Esc>",
            "X\nabcd\n",
            (0, 0),
            Some("abcd\nabcd\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvSX<Esc>j.",
            "X\nX\ngamma\n",
            (1, 0),
            Some("epsilon zeta\n"),
        ),
    ]);
}

#[test]
fn visual_capital_c_changes_lines_or_the_block_to_the_end_of_line() {
    check(&[
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvCX<Esc>",
            "X\nepsilon zeta\ngamma\n",
            (0, 0),
            Some("alpha beta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvjCX<Esc>",
            "X\ngamma\n",
            (0, 0),
            Some("alpha beta\nepsilon zeta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "VCX<Esc>",
            "X\nepsilon zeta\ngamma\n",
            (0, 0),
            Some("alpha beta\n"),
        ),
        (
            "abcd\nabcd\nabcd",
            "l<C-v>jCX<Esc>",
            "aX\naX\nabcd\n",
            (0, 1),
            Some("bcd\nbcd"),
        ),
        (
            "abcd\nab\nabcd",
            "l<C-v>jjCX<Esc>",
            "aX\naX\naX\n",
            (0, 1),
            Some("bcd\nb\nbcd"),
        ),
    ]);
}

#[test]
fn visual_capital_d_and_x_delete_lines() {
    check(&[
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvD",
            "epsilon zeta\ngamma\n",
            (0, 0),
            Some("alpha beta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvjD",
            "gamma\n",
            (0, 0),
            Some("alpha beta\nepsilon zeta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "VD",
            "epsilon zeta\ngamma\n",
            (0, 0),
            Some("alpha beta\n"),
        ),
        (
            "abcd\nabcd\nabcd",
            "l<C-v>jD",
            "a\na\nabcd\n",
            (0, 0),
            Some("bcd\nbcd"),
        ),
        (
            "abcd\nab\nabcd",
            "ll<C-v>jjD",
            "ab\nab\nab\n",
            (0, 1),
            Some("cd\n\ncd"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvX",
            "epsilon zeta\ngamma\n",
            (0, 0),
            Some("alpha beta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvjX",
            "gamma\n",
            (0, 0),
            Some("alpha beta\nepsilon zeta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "VX",
            "epsilon zeta\ngamma\n",
            (0, 0),
            Some("alpha beta\n"),
        ),
        (
            "abcd\nabcd\nabcd",
            "l<C-v>jX",
            "acd\nacd\nabcd\n",
            (0, 1),
            Some("b\nb"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvj\"aD\"ap",
            "gamma\nalpha beta\nepsilon zeta\n",
            (1, 0),
            Some("alpha beta\nepsilon zeta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvDj.",
            "epsilon zeta\n",
            (0, 0),
            Some("gamma\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvDgvd",
            "epsilo zeta\ngamma\n",
            (0, 6),
            Some("n"),
        ),
    ]);
}

#[test]
fn visual_capital_y_yanks_lines() {
    check(&[
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvY",
            "alpha beta\nepsilon zeta\ngamma\n",
            (0, 0),
            Some("alpha beta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvjYGp",
            "alpha beta\nepsilon zeta\ngamma\nalpha beta\nepsilon zeta\n",
            (3, 0),
            Some("alpha beta\nepsilon zeta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "VjYGp",
            "alpha beta\nepsilon zeta\ngamma\nalpha beta\nepsilon zeta\n",
            (3, 0),
            Some("alpha beta\nepsilon zeta\n"),
        ),
        (
            "alpha beta\nepsilon zeta\ngamma",
            "wvj\"aY\"aP",
            "alpha beta\nepsilon zeta\nalpha beta\nepsilon zeta\ngamma\n",
            (0, 0),
            Some("alpha beta\nepsilon zeta\n"),
        ),
    ]);
}
