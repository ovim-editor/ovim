//! Named registers: `"A` appends to `"a` (a linewise side makes the result linewise), the
//! unnamed register then holds the whole register, and a yank into a named register leaves
//! the yank register `0` alone. Every row was produced with `nvim --clean --headless`
//! (`normal! {keys}`): the buffer, the cursor as (line, column) and the unnamed register.

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
fn named_and_appended_registers_follow_vim() {
    check(&[
        (
            "alpha\nepsilon",
            "\"ayiwj\"Ayiw\"ap",
            "alpha\nealphaepsilonpsilon\n",
            (1, 12),
            Some("alphaepsilon"),
        ),
        (
            "alpha\nepsilon\nzeta",
            "\"ayyj\"Ayy\"ap",
            "alpha\nepsilon\nalpha\nepsilon\nzeta\n",
            (2, 0),
            Some("alpha\nepsilon\n"),
        ),
        (
            "alpha\nepsilon\nzeta",
            "\"ayiwj\"Ayy\"ap",
            "alpha\nepsilon\nalpha\nepsilon\nzeta\n",
            (2, 0),
            Some("alpha\nepsilon\n"),
        ),
        (
            "alpha\nepsilon\nzeta",
            "\"ayyj\"Ayiw\"ap",
            "alpha\nepsilon\nalpha\nepsilon\nzeta\n",
            (2, 0),
            Some("alpha\nepsilon\n"),
        ),
        (
            "alpha beta\nepsilon",
            "\"adwj\"Adw\"ap",
            "beta\nalpha epsilon\n",
            (1, 12),
            Some("alpha epsilon"),
        ),
        (
            "alpha\nepsilon\nzeta",
            "\"addj\"Add\"ap",
            "epsilon\nalpha\nzeta\n",
            (1, 0),
            Some("alpha\nzeta\n"),
        ),
        (
            "alpha\nepsilon",
            "\"Ayiw\"ap",
            "aalphalpha\nepsilon\n",
            (0, 5),
            Some("alpha"),
        ),
        (
            "alpha beta\nepsilon",
            "\"Acwxx<Esc>\"ap",
            "xxalpha beta\nepsilon\n",
            (0, 6),
            Some("alpha"),
        ),
        (
            "alpha beta\nepsilon",
            "\"ayiwj\"Avey\"ap",
            "alpha beta\nealphapsilon\n",
            (1, 5),
            Some("epsilon"),
        ),
        (
            "alpha\nepsilon",
            "\"ayiw\"Ap",
            "aalphalpha\nepsilon\n",
            (0, 5),
            Some("alpha"),
        ),
        (
            "alpha\nepsilon",
            "\"axj\"Ax\"ap",
            "lpha\npaesilon\n",
            (1, 2),
            Some("ae"),
        ),
        (
            "alpha\nepsilon",
            "\"ayiwj\"Ayiwp",
            "alpha\nealphaepsilonpsilon\n",
            (1, 12),
            Some("alphaepsilon"),
        ),
        (
            "alpha\nepsilon",
            "\"Ayiw\"0p",
            "alpha\nepsilon\n",
            (0, 0),
            Some("alpha"),
        ),
        (
            "alpha\nepsilon",
            "\"ayiwj\"Ayiw\"Ap",
            "alpha\nealphaepsilonpsilon\n",
            (1, 12),
            Some("alphaepsilon"),
        ),
        (
            "alpha\nepsilon",
            "\"ayiw\"0p",
            "alpha\nepsilon\n",
            (0, 0),
            Some("alpha"),
        ),
        (
            "alpha\nepsilon",
            "yiwj\"ayiw\"0p",
            "alpha\nealphapsilon\n",
            (1, 5),
            Some("epsilon"),
        ),
        (
            "alpha\nepsilon",
            "\"ayiwjp",
            "alpha\nealphapsilon\n",
            (1, 5),
            Some("alpha"),
        ),
        (
            "alpha\nepsilon",
            "\"ayy\"0p",
            "alpha\nepsilon\n",
            (0, 0),
            Some("alpha\n"),
        ),
        (
            "alpha beta\nepsilon",
            "\"adwj\"-p",
            "beta\nepsilon\n",
            (1, 0),
            Some("alpha "),
        ),
        (
            "alpha beta\nepsilon",
            "yiwj\"adw\"0p",
            "alpha beta\nalpha\n",
            (1, 4),
            Some("epsilon"),
        ),
        (
            "alpha beta\nepsilon",
            "\"addj\"1p",
            "epsilon\n",
            (0, 0),
            Some("alpha beta\n"),
        ),
        (
            "alpha beta\nepsilon",
            "\"axj\"-p",
            "lpha beta\nepsilon\n",
            (1, 0),
            Some("a"),
        ),
        (
            "alpha\nepsilon",
            "\"ayyj\"Ayyp",
            "alpha\nepsilon\nalpha\nepsilon\n",
            (2, 0),
            Some("alpha\nepsilon\n"),
        ),
        (
            "alpha\nepsilon\nzeta",
            "\"ayiwj\"Ayy\"ap",
            "alpha\nepsilon\nalpha\nepsilon\nzeta\n",
            (2, 0),
            Some("alpha\nepsilon\n"),
        ),
        (
            "alpha\nepsilon\nzeta",
            "\"ayyj\"Ayiw\"ap",
            "alpha\nepsilon\nalpha\nepsilon\nzeta\n",
            (2, 0),
            Some("alpha\nepsilon\n"),
        ),
        (
            "alpha\nepsilon\nzeta",
            "\"ayiwj\"Ayy\"ayy",
            "alpha\nepsilon\nzeta\n",
            (1, 0),
            Some("epsilon\n"),
        ),
        (
            "alpha\nepsilon\nzeta",
            "\"ayiwj\"Add\"ap",
            "alpha\nzeta\nalpha\nepsilon\n",
            (2, 0),
            Some("alpha\nepsilon\n"),
        ),
        (
            "alpha\nepsilon\nzeta",
            "\"ayiwj\"Addp",
            "alpha\nzeta\nalpha\nepsilon\n",
            (2, 0),
            Some("alpha\nepsilon\n"),
        ),
        (
            "alpha beta",
            "\"aywe\"Acwxx<Esc>\"ap",
            "alphxxalpha a beta\n",
            (0, 12),
            Some("alpha a"),
        ),
    ]);
}
