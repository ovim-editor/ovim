//! Operators over motions: counts, `%`, `.`, and where puts leave the cursor.
//!
//! Every row was produced by `nvim --clean --noplugin` (keys fed without
//! mappings, `startofline expandtab shiftwidth=4`): the text and the
//! zero-based cursor it ended with.

mod helpers;
use helpers::EditorTest;

const A: &str = "one two three four five\nsecond (a, b) line\nthird\n";
const B: &str = "fn main() {\n    let x = foo(bar, \"baz qux\");\n    if x { y(); }\n}\n\nlast paragraph here.\nSecond sentence.  Third one? Yes.\n";
const D: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
const E: &str = "foo.bar(baz[1], {k: v});\n<div class=\"x\">text</div>\n'single' `tick` \"dq\"\n";

#[test]
fn matches_nvim() {
    let cases: &[(&str, &str, &str, (usize, usize))] = &[
        // A count on `$`, `D` and `C` reaches into the following lines.
        (A, "wd2$", "one \nthird\n", (0, 3)),
        (A, "w2D", "one \nthird\n", (0, 3)),
        (A, "w2CX<Esc>", "one X\nthird\n", (0, 4)),
        (A, "wc2$X<Esc>", "one X\nthird\n", (0, 4)),
        (A, "wg~2$", "one TWO THREE FOUR FIVE\nSECOND (A, B) LINE\nthird\n", (0, 4)),
        (D, "8Gd4$", "a\nb\nc\nd\ne\nf\ng\n", (6, 0)),
        // ... and fails on the last line, where there is no line below.
        (A, "Gd2$", "one two three four five\nsecond (a, b) line\nthird\n", (2, 0)),
        (A, "G2D", "one two three four five\nsecond (a, b) line\nthird\n", (2, 0)),
        (A, "w2Dj.", "one \nthird\n", (1, 3)),
        // `%` after an operator looks for a bracket further along the line.
        (A, "jd%", "one two three four five\n line\nthird\n", (1, 0)),
        (A, "jc%X<Esc>", "one two three four five\nX line\nthird\n", (1, 0)),
        (A, "jy%P", "one two three four five\nsecond (a, b)second (a, b) line\nthird\n", (1, 12)),
        (E, "$F1d%", "foo.bar(baz], {k: v});\n<div class=\"x\">text</div>\n'single' `tick` \"dq\"\n", (0, 11)),
        (E, "f(c%X<Esc>f{.", "foo.barX;\n<div class=\"x\">text</div>\n'single' `tick` \"dq\"\n", (0, 7)),
        // `.` repeats an operator over any motion.
        (A, "wdgej.", "onwo three four fivond (a, b) line\nthird\n", (0, 19)),
        (A, "d3|j.", "e two three four five\ncond (a, b) line\nthird\n", (1, 0)),
        (A, "fod;.", "one twur five\nsecond (a, b) line\nthird\n", (0, 6)),
        (B, "jwc}X<Esc>", "fn main() {\n    X\n\nlast paragraph here.\nSecond sentence.  Third one? Yes.\n", (1, 4)),
        (A, "wcgeX<Esc>w.", "onXwXhree four five\nsecond (a, b) line\nthird\n", (0, 4)),
        (A, "c3|X<Esc>j.", "Xe two three four five\nXcond (a, b) line\nthird\n", (1, 0)),
        (A, "w2CX<Esc>gg.", "X\n", (0, 0)),
        // A delete over several lines that starts in the indent takes them whole.
        (A, "jd2$", "one two three four five\n", (0, 0)),
        (B, "jd/y<CR>", "fn main() {\ny(); }\n}\n\nlast paragraph here.\nSecond sentence.  Third one? Yes.\n", (1, 0)),
        // `gp` and `gP` leave the cursor after the text.
        (A, "wyiwjgp", "one two three four five\nsecontwod (a, b) line\nthird\n", (1, 8)),
        (A, "yyjgp", "one two three four five\nsecond (a, b) line\none two three four five\nthird\n", (3, 0)),
        (A, "yyjgP", "one two three four five\none two three four five\nsecond (a, b) line\nthird\n", (2, 0)),
        (A, "ywgP", "one one two three four five\nsecond (a, b) line\nthird\n", (0, 4)),
        (A, "yyGgp", "one two three four five\nsecond (a, b) line\nthird\none two three four five\n", (3, 0)),
        (A, "ywgp.", "oone none e two three four five\nsecond (a, b) line\nthird\n", (0, 10)),
        (A, "yyjgp.", "one two three four five\nsecond (a, b) line\none two three four five\nthird\none two three four five\n", (4, 0)),
        // Characterwise text that spans lines: the cursor goes to its start.
        (A, "wy2$jp", "one two three four five\nsecontwo three four five\nsecond (a, b) lined (a, b) line\nthird\n", (1, 5)),
        (A, "wy2$P", "one two three four five\nsecond (a, b) linetwo three four five\nsecond (a, b) line\nthird\n", (0, 4)),
        (A, "vjyjp", "one two three four five\nsone two three four five\nsecond (a, b) line\nthird\n", (1, 1)),
        // `ge` in the first word goes to the start of the buffer.
        (A, "wlge", "one two three four five\nsecond (a, b) line\nthird\n", (0, 2)),
        (A, "wdge.", "o three four five\nsecond (a, b) line\nthird\n", (0, 0)),
    ];
    let mut failures = Vec::new();
    for &(text, keys, expected, cursor) in cases {
        let mut test = EditorTest::new(text);
        test.keys(keys);
        let got = (test.buffer_content(), test.cursor());
        if got != (expected.to_string(), cursor) {
            failures.push(format!(
                "{keys}\n  nvim: {expected:?} {cursor:?}\n  ovim: {:?} {:?}",
                got.0, got.1
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
