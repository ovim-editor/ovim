//! `ip` and `ap` on the last paragraph include its line break (no empty line is left behind
//! and the register ends in a newline), and yanking any text object leaves the cursor at the
//! start of what was yanked. Every row was produced with `nvim --clean --headless -s` (the keys
//! typed from a script file): the buffer, the cursor as (line, column) and the unnamed register.

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
fn paragraph_objects_on_the_last_paragraph_take_its_line_break() {
    check(&[
        (
            "a\nb\n\nc\nd",
            "Gyip",
            "a\nb\n\nc\nd\n",
            (3, 0),
            Some("c\nd\n"),
        ),
        (
            "a\nb\n\nc\nd",
            "GyipP",
            "a\nb\n\nc\nd\nc\nd\n",
            (3, 0),
            Some("c\nd\n"),
        ),
        (
            "a\nb\n\nc\nd",
            "Gyap",
            "a\nb\n\nc\nd\n",
            (2, 0),
            Some("\nc\nd\n"),
        ),
        ("a\nb\n\nc\nd", "Gdip", "a\nb\n\n", (2, 0), Some("c\nd\n")),
        (
            "a\nb\n\nc\nd",
            "Gdipp",
            "a\nb\n\nc\nd\n",
            (3, 0),
            Some("c\nd\n"),
        ),
        ("a\nb\n\nc\nd", "Gdap", "a\nb\n", (1, 0), Some("\nc\nd\n")),
        (
            "a\nb\n\nc\nd",
            "Gdapp",
            "a\nb\n\nc\nd\n",
            (2, 0),
            Some("\nc\nd\n"),
        ),
        (
            "a\nb\n\nc\nd",
            "GcipX<Esc>",
            "a\nb\n\nX\n",
            (3, 0),
            Some("c\nd\n"),
        ),
        (
            "a\nb\n\nc\nd",
            "yipGp",
            "a\nb\n\nc\nd\na\nb\n",
            (5, 0),
            Some("a\nb\n"),
        ),
    ]);
}

#[test]
fn yanking_an_object_leaves_the_cursor_at_its_start() {
    check(&[
        ("a\n{\nb\n}", "Gyi{", "a\n{\nb\n}\n", (2, 0), Some("b\n")),
        (
            "  ab\n  cd\n  ef\n\ngh",
            "jllyip",
            "  ab\n  cd\n  ef\n\ngh\n",
            (0, 0),
            Some("  ab\n  cd\n  ef\n"),
        ),
        (
            "  ab\n  cd\n  ef\n\ngh",
            "jllyap",
            "  ab\n  cd\n  ef\n\ngh\n",
            (0, 0),
            Some("  ab\n  cd\n  ef\n\n"),
        ),
        (
            "ab\n\n  cd\n  ef\n  gh",
            "Gllyip",
            "ab\n\n  cd\n  ef\n  gh\n",
            (2, 0),
            Some("  cd\n  ef\n  gh\n"),
        ),
        (
            "x {\n  a\n  b\n}",
            "jllyi{",
            "x {\n  a\n  b\n}\n",
            (1, 0),
            Some("  a\n  b\n"),
        ),
        (
            "x {\n  a\n  b\n}",
            "jjlyi{",
            "x {\n  a\n  b\n}\n",
            (1, 0),
            Some("  a\n  b\n"),
        ),
        ("  ab cd", "wyiw", "  ab cd\n", (0, 2), Some("ab")),
        ("ab cd", "$yiw", "ab cd\n", (0, 3), Some("cd")),
        ("ab cd ef", "wlyaw", "ab cd ef\n", (0, 3), Some("cd ")),
    ]);
}
