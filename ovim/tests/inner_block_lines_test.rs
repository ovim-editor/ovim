//! The inner part of a bracket pair that spans lines (`di{`, `ci(`, `yi[`, `>i{`): the line
//! breaks next to the brackets are not part of it, so a block on its own lines is deleted,
//! yanked and changed as whole lines and the brackets stay on theirs; text beside a bracket
//! keeps the old character range. Not inside a pair, the first pair opening later on the
//! line is used. Every row was produced with `nvim --clean --headless -s` (the keys typed from
//! a script file): the buffer, the cursor as (line, column) and the unnamed register.

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
fn inner_block_over_whole_lines_is_linewise() {
    check(&[
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjdiB",
            "fn f() {\n    let a = 1;\n    if a {\n    }\n}\n",
            (3, 4),
            Some("        b();\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjd2iB",
            "fn f() {\n}\n",
            (1, 0),
            Some("    let a = 1;\n    if a {\n        b();\n    }\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjdi{",
            "fn f() {\n}\n",
            (1, 0),
            Some("    let a = 1;\n    if a {\n        b();\n    }\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jj$di{",
            "fn f() {\n    let a = 1;\n    if a {\n    }\n}\n",
            (3, 4),
            Some("        b();\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "ggdi{",
            "fn f() {\n}\n",
            (1, 0),
            Some("    let a = 1;\n    if a {\n        b();\n    }\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjciBX<Esc>",
            "fn f() {\n    let a = 1;\n    if a {\n        X\n    }\n}\n",
            (3, 8),
            Some("        b();\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjciBX<Esc>",
            "fn f() {\n    X\n}\n",
            (1, 4),
            Some("    let a = 1;\n    if a {\n        b();\n    }\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "ggci{X<Esc>",
            "fn f() {\n    X\n}\n",
            (1, 4),
            Some("    let a = 1;\n    if a {\n        b();\n    }\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjyiBGp",
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}\n        b();\n",
            (6, 8),
            Some("        b();\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjyiBP",
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n        b();\n    }\n}\n",
            (3, 8),
            Some("        b();\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjj>iB",
            "fn f() {\n    let a = 1;\n    if a {\n            b();\n    }\n}\n",
            (3, 12),
            None,
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjgUiB",
            "fn f() {\n    let a = 1;\n    if a {\n        B();\n    }\n}\n",
            (3, 0),
            None,
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjdiBkk.",
            "fn f() {\n}\n",
            (1, 0),
            Some("    let a = 1;\n    if a {\n    }\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjdiBu",
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}\n",
            (3, 0),
            Some("        b();\n"),
        ),
        ("{\n}", "jdi{", "{\n}\n", (1, 0), None),
        ("{\n}", "jci{X<Esc>", "{\nX}\n", (1, 0), None),
        ("{\n  a\n}", "jdi{", "{\n}\n", (1, 0), Some("  a\n")),
        (
            "{\n  a\n}",
            "jci{X<Esc>",
            "{\n  X\n}\n",
            (1, 2),
            Some("  a\n"),
        ),
        ("{\na\n}", "jdi{", "{\n}\n", (1, 0), Some("a\n")),
        (
            "a {\n  b {\n    c\n  }\n}",
            "jjd2i{",
            "a {\n}\n",
            (1, 0),
            Some("  b {\n    c\n  }\n"),
        ),
        (
            "a {\n  b {\n    c\n  }\n}",
            "jjc2i{X<Esc>",
            "a {\n  X\n}\n",
            (1, 2),
            Some("  b {\n    c\n  }\n"),
        ),
        ("a [\n  b\n]", "jdi[", "a [\n]\n", (1, 0), Some("  b\n")),
        ("a <\n  b\n>", "jdi<", "a <\n>\n", (1, 0), Some("  b\n")),
        (
            "a {\n  b\n}\nc {\n  d\n}",
            "jci{X<Esc>jjj.",
            "a {\n  X\n}\nc {\n  X\n}\n",
            (4, 2),
            Some("  d\n"),
        ),
        (
            "a {\n  b\n  c\n}\nd",
            "jd1i{",
            "a {\n}\nd\n",
            (1, 0),
            Some("  b\n  c\n"),
        ),
    ]);
}

#[test]
fn inner_block_keeps_the_text_beside_the_brackets() {
    check(&[
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjdaB",
            "fn f() {\n    let a = 1;\n    if a \n}\n",
            (2, 8),
            Some("{\n        b();\n    }"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjviBd",
            "fn f() {\n    let a = 1;\n    if a {\n    }\n}\n",
            (3, 0),
            Some("        b();\n"),
        ),
        (
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}",
            "jjjviBy",
            "fn f() {\n    let a = 1;\n    if a {\n        b();\n    }\n}\n",
            (3, 0),
            Some("        b();\n"),
        ),
        (
            "fn f(a, b) {\n    x\n}",
            "fadi(",
            "fn f() {\n    x\n}\n",
            (0, 5),
            Some("a, b"),
        ),
        (
            "fn f(\n    a,\n    b\n) {\n    x\n}",
            "jdi(",
            "fn f(\n) {\n    x\n}\n",
            (1, 0),
            Some("    a,\n    b\n"),
        ),
        (
            "fn f(\n    a,\n    b\n) {\n    x\n}",
            "jci(X<Esc>",
            "fn f(\n    X\n) {\n    x\n}\n",
            (1, 4),
            Some("    a,\n    b\n"),
        ),
        (
            "fn f(\n    a,\n    b\n)",
            "jyi(P",
            "fn f(\n    a,\n    b\n    a,\n    b\n)\n",
            (1, 4),
            Some("    a,\n    b\n"),
        ),
        (
            "fn f( \n    a\n  ) {}",
            "jdi(",
            "fn f(\n  ) {}\n",
            (0, 4),
            Some(" \n    a"),
        ),
        (
            "fn f( \n    a\n  ) {}",
            "jci(X<Esc>",
            "fn f(X\n  ) {}\n",
            (0, 5),
            Some(" \n    a"),
        ),
        ("{ a\n  b\n}", "jdi{", "{\n}\n", (0, 0), Some(" a\n  b")),
        (
            "{ a\n  b\n}",
            "jci{X<Esc>",
            "{X\n}\n",
            (0, 1),
            Some(" a\n  b"),
        ),
        ("{\n  a\n  b }", "jdi{", "{\n}\n", (1, 0), Some("  a\n  b ")),
        (
            "{\n  a\n  b }",
            "jci{X<Esc>",
            "{\nX}\n",
            (1, 0),
            Some("  a\n  b "),
        ),
        ("x = {a, b};", "fadi{", "x = {};\n", (0, 5), Some("a, b")),
        (
            "a {\n  b\n  c\n}",
            "jvi{d",
            "a {\n}\n",
            (1, 0),
            Some("  b\n  c\n"),
        ),
        (
            "a {\n  b\n  c\n}",
            "jvi{cX<Esc>",
            "a {\nX}\n",
            (1, 0),
            Some("  b\n  c\n"),
        ),
        (
            "a {\n  b\n  c\n}",
            "jvi{y",
            "a {\n  b\n  c\n}\n",
            (1, 0),
            Some("  b\n  c\n"),
        ),
    ]);
}

#[test]
fn an_inner_block_looks_ahead_on_the_line() {
    check(&[
        (
            "foo(bar, baz) qux",
            "di(",
            "foo() qux\n",
            (0, 4),
            Some("bar, baz"),
        ),
        (
            "foo(bar, baz) qux",
            "ci(X<Esc>",
            "foo(X) qux\n",
            (0, 4),
            Some("bar, baz"),
        ),
        ("x foo(bar) y", "wdi(", "x foo() y\n", (0, 6), Some("bar")),
        ("foo(bar) baz", "$di(", "foo(bar) baz\n", (0, 11), None),
        ("a(b(c) d)", "fcdi(", "a(b() d)\n", (0, 4), Some("c")),
        ("a (b (c) d)", "di(", "a ()\n", (0, 3), Some("b (c) d")),
        (
            "foo(bar, baz) qux",
            "da(",
            "foo qux\n",
            (0, 3),
            Some("(bar, baz)"),
        ),
        ("x { a } y", "da{", "x  y\n", (0, 2), Some("{ a }")),
        ("foo bar", "di{", "foo bar\n", (0, 0), None),
        (
            "foo [1, 2] bar",
            "ci[X<Esc>",
            "foo [X] bar\n",
            (0, 5),
            Some("1, 2"),
        ),
        ("foo(bar) x", "vi(d", "foo() x\n", (0, 4), Some("bar")),
        ("f(a) g(b)", "ci(X<Esc>w.", "f(X) g(b)\n", (0, 2), Some("X")),
    ]);
}
