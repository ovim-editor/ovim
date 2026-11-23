//! `}` and `{`: only empty lines separate paragraphs (not lines of blanks), `}` on the last
//! paragraph goes to the last character of the buffer (and `d}` there takes it), asking for
//! more paragraphs than there are fails, and both work in Visual mode. Every row was produced
//! with `nvim --clean --headless -s` (the keys typed from a script file): the buffer, the
//! cursor as (line, column) and the unnamed register.

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
fn paragraph_motions_at_the_ends_of_the_buffer() {
    check(&[
        ("a\nb\n\nc\nd", "G}", "a\nb\n\nc\nd\n", (4, 0), None),
        ("a\nb\n\nc\nd", "G}x", "a\nb\n\nc\n\n", (4, 0), Some("d")),
        ("a\nb\n\nc\nd", "Gd}", "a\nb\n\nc\n\n", (4, 0), Some("d")),
        ("a\nb\n\nc\nd", "jjjd}", "a\nb\n\n", (2, 0), Some("c\nd\n")),
        (
            "a\nb\n\nc\nd",
            "jjjc}X<Esc>",
            "a\nb\n\nX\n",
            (3, 0),
            Some("c\nd"),
        ),
        ("a\nb\n\nc\nd", "{", "a\nb\n\nc\nd\n", (0, 0), None),
        ("a\nb\n\nc\nd", "jd{", "b\n\nc\nd\n", (0, 0), Some("a\n")),
        ("a\nb\n\nc\nd", "ld{", "a\nb\n\nc\nd\n", (0, 0), None),
        ("a\n\nb\n\nc", "5}", "a\n\nb\n\nc\n", (0, 0), None),
        ("a\n\nb\n\nc", "d5}", "a\n\nb\n\nc\n", (0, 0), None),
        ("a\n\nb\n\n", "G}", "a\n\nb\n\n\n", (4, 0), None),
        ("a\n\nb\n\n", "jjd}", "a\n\n\n\n", (2, 0), Some("b\n")),
        ("a\nb\n\nc\nd", "jjj}x", "a\nb\n\nc\n\n", (4, 0), Some("d")),
        ("a\nb\n\nc dd", "jjj}x", "a\nb\n\nc d\n", (3, 2), Some("d")),
    ]);
}

#[test]
fn paragraphs_are_separated_by_empty_lines_only() {
    check(&[
        ("a\n  \nb\n\nc", "}", "a\n  \nb\n\nc\n", (3, 0), None),
        ("a\n  \nb\n\nc", "}}", "a\n  \nb\n\nc\n", (4, 0), None),
        ("a\n  \nb\n\nc", "G{", "a\n  \nb\n\nc\n", (3, 0), None),
        ("a\n  \nb\n\nc", "jj{", "a\n  \nb\n\nc\n", (0, 0), None),
        ("a\n  \nb\n\nc", "d}", "\nc\n", (0, 0), Some("a\n  \nb\n")),
        ("a\n  \nb\n\nc", "jdip", "a\nb\n\nc\n", (1, 0), Some("  \n")),
        ("\n\na\nb\n\nc", "}", "\n\na\nb\n\nc\n", (4, 0), None),
        ("\n\na\nb\n\nc", "jj}", "\n\na\nb\n\nc\n", (4, 0), None),
        ("\n\na\nb\n\nc", "jjj{", "\n\na\nb\n\nc\n", (1, 0), None),
        ("a\n\n\nb", "jj{", "a\n\n\nb\n", (0, 0), None),
        ("a\n\n\nb", "jjj{", "a\n\n\nb\n", (2, 0), None),
        (
            "a\n\nb\n\nc\n\nd",
            "G2{",
            "a\n\nb\n\nc\n\nd\n",
            (3, 0),
            None,
        ),
        ("a\n\nb\n\nc", "G9{", "a\n\nb\n\nc\n", (4, 0), None),
        ("a\n\nb\n\nc\n\nd", "2}", "a\n\nb\n\nc\n\nd\n", (3, 0), None),
        ("a\n\nb\n\nc", "4}", "a\n\nb\n\nc\n", (0, 0), None),
        ("a\n\nb\n\nc", "5}", "a\n\nb\n\nc\n", (0, 0), None),
        (
            "a\n\nb\n\nc\n\nd",
            "d}.",
            "\nc\n\nd\n",
            (0, 0),
            Some("\nb\n"),
        ),
        ("a\n\nb\n\nc", "jjd}j.", "a\n\n\n\n", (3, 0), Some("c")),
        ("a\nb\n\nc\nd", "Gd{", "a\nb\nd\n", (2, 0), Some("\nc\n")),
        ("a\nb\n\nc\nd", "Gld{", "a\nb\nd\n", (2, 0), Some("\nc\n")),
        (
            "a\nb\n\nc\nd",
            "jyyGyip{P",
            "a\nb\nc\nd\n\nc\nd\n",
            (2, 0),
            Some("c\nd\n"),
        ),
        ("a\nb\n\nc\nd", "gU}", "A\nB\n\nc\nd\n", (0, 0), None),
        ("a\nb\n\nc\nd", "jjjgU}", "a\nb\n\nC\nD\n", (3, 0), None),
        ("a\nb\n\nc\nd", ">}", "    a\n    b\n\nc\nd\n", (0, 4), None),
        (
            "a\nb\n\nc\nd",
            "jjj>}",
            "a\nb\n\n    c\n    d\n",
            (3, 4),
            None,
        ),
    ]);
}

#[test]
fn paragraph_motions_in_visual_mode() {
    check(&[
        ("a\nb\n\nc\nd", "v}d", "c\nd\n", (0, 0), Some("a\nb\n\n")),
        ("a\nb\n\nc\nd", "jjjv}d", "a\nb\n\n\n", (3, 0), Some("c\nd")),
        ("a\nb\n\nc\nd", "Gv{d", "a\nb\n\n", (2, 0), Some("\nc\nd")),
    ]);
}
