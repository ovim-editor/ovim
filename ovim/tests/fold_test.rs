//! Folding (OV-00455). Expectations for manual folds were cross-checked
//! against `nvim --headless` (`2Gzf3j` creates a closed fold over lines 2-5).

mod helpers;

use helpers::EditorTest;

fn ten_lines() -> EditorTest {
    let text: String = (1..=10).map(|n| format!("line {n}\n")).collect();
    EditorTest::new(&text)
}

fn hidden(test: &EditorTest) -> Vec<usize> {
    (0..test.line_count())
        .filter(|&line| test.editor.buffer().is_line_folded(line))
        .collect()
}

#[test]
fn zf_creates_a_closed_fold_that_hides_its_body() {
    let mut test = ten_lines();
    test.keys("2Gzf3j");
    // Lines 2-5 (1-based): header stays, 3..=5 hidden (0-based 2..=4).
    assert_eq!(hidden(&test), vec![2, 3, 4]);
    test.assert_cursor(1, 0);
}

#[test]
fn j_and_k_treat_a_closed_fold_as_one_line() {
    let mut test = ten_lines();
    test.keys("2Gzf3jgg");
    test.keys("j");
    test.assert_cursor(1, 0); // onto the fold header (line 2)
    test.keys("j");
    test.assert_cursor(5, 0); // over the fold to line 6
    test.keys("k");
    test.assert_cursor(1, 0);
    test.keys("gg3j");
    test.assert_cursor(6, 0); // 1 -> fold -> 6 -> 7, like Vim's `3j`
}

#[test]
fn zo_zc_za_open_and_close_the_fold_under_the_cursor() {
    let mut test = ten_lines();
    test.keys("2Gzf3j");
    test.keys("zo");
    assert!(hidden(&test).is_empty());
    test.keys("4G");
    test.keys("zc");
    assert_eq!(hidden(&test), vec![2, 3, 4]);
    test.assert_cursor(1, 0); // cursor shown on the header
    test.keys("za");
    assert!(hidden(&test).is_empty());
    test.keys("za");
    assert_eq!(hidden(&test), vec![2, 3, 4]);
}

#[test]
fn zr_zm_and_ze_act_on_all_folds() {
    let mut test = ten_lines();
    test.keys("2Gzf3j7Gzf2j");
    assert_eq!(hidden(&test), vec![2, 3, 4, 7, 8]);
    test.keys("zR");
    assert!(hidden(&test).is_empty());
    test.keys("zM");
    assert_eq!(hidden(&test), vec![2, 3, 4, 7, 8]);
    test.keys("zE");
    assert!(hidden(&test).is_empty());
}

#[test]
fn dd_on_a_closed_fold_deletes_the_whole_fold() {
    let mut test = ten_lines();
    test.keys("2Gzf3jdd");
    test.assert_line_count(6);
    assert_eq!(test.line_text(0).as_deref(), Some("line 1"));
    assert_eq!(test.line_text(1).as_deref(), Some("line 6"));
    test.assert_cursor(1, 0);
    // The deleted text is one linewise register of four lines.
    let register = test.get_register_content('"').unwrap();
    assert_eq!(register.lines().count(), 4);
    // Undo brings all of it back.
    test.keys("u");
    test.assert_line_count(10);
}

#[test]
fn moving_horizontally_onto_a_closed_header_opens_it() {
    let mut test = ten_lines();
    test.keys("2Gzf3j");
    test.keys("l");
    assert!(hidden(&test).is_empty());
}

#[test]
fn zj_and_zk_jump_between_folds() {
    let mut test = ten_lines();
    test.keys("2Gzf3j7Gzf2jzRgg");
    test.keys("zj");
    test.assert_cursor(1, 0);
    test.keys("zj");
    test.assert_cursor(6, 0);
    test.keys("zk");
    test.assert_cursor(4, 0); // end of the first fold
}

#[test]
fn closed_folds_take_no_visual_rows_in_the_wrap_map() {
    let mut test = ten_lines();
    test.editor.options.wrap = true;
    test.editor.ensure_wrap_map(80);
    let before = test.editor.wrap_map().unwrap().total_visual_lines();
    test.keys("2Gzf3j");
    test.editor.ensure_wrap_map(80);
    let map = test.editor.wrap_map().unwrap();
    assert_eq!(map.total_visual_lines(), before - 3);
    // Visual row 2 is line 6 (index 5): the fold body is skipped.
    assert_eq!(map.visual_to_logical(2).0, 5);
    test.keys("zo");
    test.editor.ensure_wrap_map(80);
    assert_eq!(test.editor.wrap_map().unwrap().total_visual_lines(), before);
}

#[test]
fn lsp_folding_ranges_replace_indentation_folds_and_keep_state() {
    let mut test = EditorTest::new("a {\n  b {\n    c\n  }\n}\nd\n");
    test.editor
        .buffer_mut()
        .set_file_path("/tmp/fold_test.txt".to_string());
    test.keys("zM");
    // Indentation folds: headers `a {` (0..3) and `b {` (1..2).
    assert!(test.editor.buffer().is_line_folded(1));
    let version = test.editor.buffer().version();
    let range = |start, end| lsp_types::FoldingRange {
        start_line: start,
        end_line: end,
        start_character: None,
        end_character: None,
        kind: None,
        collapsed_text: None,
    };
    let applied = test.editor.apply_lsp_folding_ranges(
        "/tmp/fold_test.txt",
        version,
        &[range(0, 3), range(1, 2)],
    );
    assert!(applied);
    // `a {` stayed closed across the recompute (same header line).
    assert!(test.editor.buffer().is_line_folded(1));
    // Stale answers are ignored.
    assert!(!test.editor.apply_lsp_folding_ranges(
        "/tmp/fold_test.txt",
        version + 5,
        &[range(0, 1)]
    ));
}

// ---------------------------------------------------------------------------
// Commands on a closed fold (`2Gzf3j`: lines 2-5 folded). Every expectation
// below was produced by `nvim --headless -u NONE` on the same ten lines with
// `shiftwidth=2 expandtab` (OV-00473): a linewise command, a characterwise
// command on the fold header, and a Visual selection reaching into a closed
// fold all cover the WHOLE fold.
// ---------------------------------------------------------------------------

fn folded(keys: &str) -> Vec<String> {
    let mut test = ten_lines();
    test.editor.options.shift_width = 2;
    test.editor.options.tab_width = 2;
    test.editor.options.expand_tab = true;
    test.keys("2Gzf3j");
    test.keys(keys);
    test.buffer_content().lines().map(String::from).collect()
}

fn lines(spec: &str) -> Vec<String> {
    spec.split('|').map(String::from).collect()
}

#[test]
fn shift_operators_cover_the_whole_closed_fold() {
    // vim: `>>` on the header shifts lines 2-5.
    assert_eq!(
        folded(">>"),
        lines("line 1|  line 2|  line 3|  line 4|  line 5|line 6|line 7|line 8|line 9|line 10")
    );
    // vim: `V>` too.
    assert_eq!(
        folded("V>"),
        lines("line 1|  line 2|  line 3|  line 4|  line 5|line 6|line 7|line 8|line 9|line 10")
    );
    // vim: `>j` from the header covers fold + the next visible line.
    assert_eq!(
        folded(">j"),
        lines("line 1|  line 2|  line 3|  line 4|  line 5|  line 6|line 7|line 8|line 9|line 10")
    );
}

/// Found hands-on: `>>` moves the cursor to the first non-blank, which the
/// "horizontal movement opens a closed fold" rule took for a motion. Vim keeps
/// the fold closed after an edit.
#[test]
fn an_operator_that_moves_the_cursor_does_not_open_the_fold() {
    let mut test = ten_lines();
    test.editor.options.shift_width = 2;
    test.keys("2Gzf3j>>");
    assert_eq!(hidden(&test), vec![2, 3, 4], "still closed after >>");
    test.keys("gUU");
    assert_eq!(hidden(&test), vec![2, 3, 4], "still closed after gUU");
}

#[test]
fn case_and_yank_operators_cover_the_whole_closed_fold() {
    // vim: `gUU`.
    assert_eq!(
        folded("gUU"),
        lines("line 1|LINE 2|LINE 3|LINE 4|LINE 5|line 6|line 7|line 8|line 9|line 10")
    );
    // vim: `yyP` yanks all four lines.
    assert_eq!(
        folded("yyP"),
        lines("line 1|line 2|line 3|line 4|line 5|line 2|line 3|line 4|line 5|line 6|line 7|line 8|line 9|line 10")
    );
    // vim: `yyp` pastes below the fold, not inside it.
    assert_eq!(
        folded("yyp"),
        lines("line 1|line 2|line 3|line 4|line 5|line 2|line 3|line 4|line 5|line 6|line 7|line 8|line 9|line 10")
    );
}

/// A `"a` prefix must survive the closed-fold check `p` does (reading the
/// register consumes the pending name; it once pasted the wrong register).
#[test]
fn a_named_register_paste_below_a_closed_fold_keeps_its_register() {
    assert_eq!(
        folded("\"ayyG\"ap"),
        lines("line 1|line 2|line 3|line 4|line 5|line 6|line 7|line 8|line 9|line 10|line 2|line 3|line 4|line 5")
    );
    assert_eq!(
        folded("\"ayy7Gzf1j\"ap"),
        lines("line 1|line 2|line 3|line 4|line 5|line 6|line 7|line 8|line 2|line 3|line 4|line 5|line 9|line 10")
    );
}

#[test]
fn change_commands_replace_the_whole_closed_fold_with_one_line() {
    let expected = lines("line 1|X|line 6|line 7|line 8|line 9|line 10");
    for keys in [
        "ccX<Esc>", "CX<Esc>", "SX<Esc>", "VcX<Esc>", "clX<Esc>", "sX<Esc>",
    ] {
        assert_eq!(folded(keys), expected, "{keys}");
    }
    // vim: `cj` covers the fold and the next visible line.
    assert_eq!(
        folded("cjX<Esc>"),
        lines("line 1|X|line 7|line 8|line 9|line 10")
    );
}

#[test]
fn deleting_characterwise_on_a_closed_fold_deletes_the_fold() {
    let expected = lines("line 1|line 6|line 7|line 8|line 9|line 10");
    for keys in ["x", "dl", "D", "d$", "Vd", "vd", "v$d"] {
        assert_eq!(folded(keys), expected, "{keys}");
    }
    assert_eq!(
        folded("dj"),
        lines("line 1|line 7|line 8|line 9|line 10"),
        "vim: dj covers fold + next line"
    );
    assert_eq!(folded("Vjd"), lines("line 1|line 7|line 8|line 9|line 10"));
    // vim: `vjd` starts at the fold's first column and ends on line 6's
    // first character (inclusive).
    assert_eq!(
        folded("vjd"),
        lines("line 1|ine 6|line 7|line 8|line 9|line 10")
    );
}

#[test]
fn open_line_and_commands_that_are_not_fold_aware_in_vim() {
    // vim: `o` opens the line below the whole fold.
    assert_eq!(
        folded("oX<Esc>"),
        lines("line 1|line 2|line 3|line 4|line 5|X|line 6|line 7|line 8|line 9|line 10")
    );
    // vim: `O` opens above the header.
    assert_eq!(
        folded("OX<Esc>"),
        lines("line 1|X|line 2|line 3|line 4|line 5|line 6|line 7|line 8|line 9|line 10")
    );
    // vim: `J` joins only the header line and the next physical line.
    assert_eq!(
        folded("J"),
        lines("line 1|line 2 line 3|line 4|line 5|line 6|line 7|line 8|line 9|line 10")
    );
    // vim: `~` and `r` touch the header character only.
    assert_eq!(
        folded("~"),
        lines("line 1|Line 2|line 3|line 4|line 5|line 6|line 7|line 8|line 9|line 10")
    );
    assert_eq!(
        folded("rx"),
        lines("line 1|xine 2|line 3|line 4|line 5|line 6|line 7|line 8|line 9|line 10")
    );
}

// ---------------------------------------------------------------------------
// zr / zm / zx / zX and [z ]z (OV-00473), expectations from nvim.
// ---------------------------------------------------------------------------

/// 0 a / 1 b / 2 c / 3 d / 4 d2 / 5 c2 / 6 b2 / 7 e / 8 f / 9 a2 with
/// tab indentation: three nesting levels.
fn nested() -> EditorTest {
    let mut test =
        EditorTest::new("a\n\tb\n\t\tc\n\t\t\td\n\t\t\td2\n\t\tc2\n\tb2\n\t\te\n\t\t\tf\na2\n");
    test.editor.options.tab_width = 8;
    test
}

#[test]
fn zm_and_zr_change_the_fold_level_one_step_at_a_time() {
    let mut test = nested();
    test.keys("zM");
    assert_eq!(hidden(&test), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    test.keys("zr");
    // foldlevel 1: only the outermost fold is open.
    assert_eq!(hidden(&test), vec![2, 3, 4, 5, 7, 8]);
    test.keys("zr");
    assert_eq!(hidden(&test), vec![3, 4, 8]);
    test.keys("zr");
    assert!(hidden(&test).is_empty());
    test.keys("zm");
    assert_eq!(hidden(&test), vec![3, 4, 8], "zm closes the deepest level");
    test.keys("2zm");
    assert_eq!(hidden(&test), vec![1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn zx_reapplies_the_fold_level_and_reveals_the_cursor_line() {
    let mut test = nested();
    test.keys("zM");
    test.keys("4Gzo");
    assert!(!hidden(&test).is_empty());
    // vim: zx forgets the manual zo, then opens what hides line 4.
    test.keys("zx");
    assert!(!test.editor.buffer().is_line_folded(3), "cursor line shown");
    assert!(
        test.editor.buffer().is_line_folded(8),
        "unrelated fold closed"
    );
    test.keys("zX");
    assert_eq!(hidden(&test), vec![1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn bracket_z_moves_to_the_edges_of_the_open_fold() {
    // nvim: [z from `c` (3rd line) goes to the enclosing fold's start; from
    // the fold start it goes to the fold around it; ]z mirrors that; counts
    // repeat; the cursor lands in the first column.
    let mut test = nested();
    test.keys("zR");
    // (ovim folds start at the header line, so the line numbers are those of
    // its own fold ranges: (0,8) (1,5) (2,4) (6,8) (7,8), 0-based.)
    test.keys("5G[z");
    test.assert_cursor(2, 0);
    test.keys("[z");
    test.assert_cursor(1, 0);
    test.keys("[z");
    test.assert_cursor(0, 0);
    test.keys("[z");
    test.assert_cursor(0, 0); // no enclosing fold: stays
    test.keys("4G]z");
    test.assert_cursor(4, 0);
    test.keys("]z");
    test.assert_cursor(5, 0);
    test.keys("4G2]z");
    test.assert_cursor(5, 0);
    test.keys("5G3[z");
    test.assert_cursor(0, 0);
}

// ---------------------------------------------------------------------------
// Tree-sitter as a fold source (OV-00473)
// ---------------------------------------------------------------------------

const JAVA: &str = "package a;\n\npublic class A {\n    /* first\n       second */\n    int f(int x) {\n        if (x > 0) {\n            return 1;\n        }\n        return 0;\n    }\n}\n";

/// Without a language server the folds come from the syntax tree, not from
/// indentation: the multi-line block comment (no indentation change) folds,
/// the closing braces stay visible, methods and nested blocks nest.
#[test]
fn syntax_tree_supplies_folds_when_there_is_no_language_server() {
    let mut test = EditorTest::new(JAVA);
    test.editor
        .buffer_mut()
        .set_file_path("/tmp/A.java".to_string());
    test.editor.buffer_mut().enable_syntax_highlighting();
    assert!(test.editor.buffer().syntax_tree().is_some());
    test.keys("zR");
    let manager = test.editor.buffer().fold_manager();
    assert_eq!(manager.source(), ovim_core::fold::FoldSource::Syntax);
    let ranges: Vec<(usize, usize)> = manager
        .folds()
        .iter()
        .map(|fold| (fold.start_line(), fold.end_line()))
        .collect();
    // class (2..10: `}` on line 11 stays), block comment (3..4), method
    // (5..9), the `if` body (6..7 with its closing brace kept visible).
    assert!(ranges.contains(&(2, 10)), "class: {ranges:?}");
    assert!(ranges.contains(&(3, 4)), "block comment: {ranges:?}");
    assert!(ranges.contains(&(5, 9)), "method: {ranges:?}");
    assert!(ranges.contains(&(6, 7)), "if body: {ranges:?}");
    // Closing the method hides its body but leaves the header and `}`.
    test.keys("6Gzc");
    let hidden_lines = hidden(&test);
    assert!(hidden_lines.contains(&6) && hidden_lines.contains(&9));
    assert!(!hidden_lines.contains(&10), "closing brace stays visible");
}

// ---------------------------------------------------------------------------
// Fold gutter (`foldcolumn`, OV-00473)
// ---------------------------------------------------------------------------

fn cells(test: &EditorTest, line: usize, width: usize) -> String {
    test.editor
        .fold_gutter_cells(line, width)
        .into_iter()
        .map(|mark| mark.glyph())
        .collect()
}

/// Vim's foldcolumn glyphs: `-` heads an open fold, `+` a closed one, `|`
/// inside an open fold. Default `auto:1`: absent without folds.
#[test]
fn the_fold_column_marks_headers_and_bodies_and_hides_without_folds() {
    let mut test = nested();
    assert_eq!(
        test.editor.fold_column_width(),
        0,
        "no folds yet: no column"
    );
    test.keys("zR");
    assert_eq!(test.editor.fold_column_width(), 1, "auto:1 caps the width");
    // Folds (0-based): (0,8) (1,5) (2,4) (6,8) (7,8).
    assert_eq!(cells(&test, 0, 1), "-", "outer header");
    assert_eq!(cells(&test, 1, 1), "-", "innermost header wins");
    assert_eq!(cells(&test, 3, 1), "|", "inside the innermost fold");
    assert_eq!(cells(&test, 9, 1), " ", "outside every fold");
    // Wider columns show the enclosing levels, outermost first (line 3,
    // 0-based 2, heads the innermost fold inside two others).
    assert_eq!(cells(&test, 2, 3), "||-");
    assert_eq!(cells(&test, 2, 2), "|-", "the innermost levels are kept");
    test.keys("2Gzc");
    assert_eq!(cells(&test, 1, 1), "+", "closed header");
    test.command("set foldcolumn=0");
    assert_eq!(test.editor.fold_column_width(), 0);
    test.command("set foldcolumn=3");
    assert_eq!(test.editor.fold_column_width(), 3, "fixed width");
    test.command("set fdc=auto:4");
    assert_eq!(test.editor.fold_column_width(), 3, "auto: deepest nesting");
}

#[test]
fn clicking_a_fold_mark_toggles_that_fold() {
    let mut test = nested();
    test.keys("zR");
    assert!(test.editor.toggle_fold_at_gutter(1));
    assert!(test.editor.buffer().is_line_folded(2), "fold closed");
    assert!(test.editor.toggle_fold_at_gutter(1));
    assert!(!test.editor.buffer().is_line_folded(2), "fold opened again");
    assert!(
        !test.editor.toggle_fold_at_gutter(9),
        "a line without a fold header does nothing"
    );
}

#[test]
fn the_tui_draws_the_fold_column_left_of_the_sign_and_number_columns() {
    use ovim::ui::Renderer;
    use ratatui::{backend::TestBackend, Terminal};
    let mut test = nested();
    test.editor.options.number = true;
    test.editor.set_viewport_height(11);
    test.keys("zR2Gzc");
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = |y: u16| -> String { (0..8).map(|x| buffer[(x, y)].symbol()).collect() };

    // Line 1 heads the outer open fold, line 2 the closed one (its body is
    // not drawn), line 7 heads the next open fold, line 9 is inside it and
    // line 10 is outside every fold.
    assert!(row(0).starts_with('-'), "{:?}", row(0));
    assert!(row(1).starts_with('+'), "{:?}", row(1));
    assert!(
        row(2).starts_with('-') && row(2).contains('7'),
        "{:?}",
        row(2)
    );
    assert!(row(4).starts_with('|'), "{:?}", row(4));
    assert!(
        row(5).starts_with(' ') && row(5).contains("10"),
        "{:?}",
        row(5)
    );
    // The numbers moved one column right of where they were without folds.
    assert!(row(0).contains('1'), "{:?}", row(0));
    assert_eq!(test.editor.render_cache.last_gutter_width, 1 + 2 + 3 + 1);
}
// ---------------------------------------------------------------------------
// Cost per key on a big file (fold bookkeeping must not scale with fold count)
// ---------------------------------------------------------------------------

/// About 47k lines and 9.5k functions, all folded by `zM`.
fn big_rust_file() -> String {
    (0..9_500)
        .map(|n| format!("fn f{n}() {{\n    let x = {n};\n    x + 1\n}}\n\n"))
        .collect()
}

/// A key press used to rescan every fold several times (140-160 ms per `j`
/// on a 48k-line Rust file in release). With the fold index it is a few
/// binary searches; the bound is orders of magnitude above that so debug
/// builds and slow CI machines cannot trip it.
#[test]
fn moving_over_thousands_of_closed_folds_stays_fast() {
    let mut test = EditorTest::new(&big_rust_file());
    test.editor
        .buffer_mut()
        .set_file_path("/tmp/many_folds.rs".to_string());
    test.editor.buffer_mut().enable_syntax_highlighting();
    test.keys("zM");
    assert!(test.editor.buffer().fold_manager().folds().len() > 9_000);

    let started = std::time::Instant::now();
    for _ in 0..30 {
        test.keys("j");
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(1500),
        "30 `j` over 9.5k closed folds took {elapsed:?}"
    );
    // Each function shows three lines (header, `}`, blank): 30 `j` cover ten.
    test.assert_cursor(50, 0);
}
