mod helpers;

use helpers::EditorTest;
use ovim::syntax::{ColorScheme, UiGroup};
use ovim::ui::Renderer;
use ratatui::{backend::TestBackend, Terminal};

/// Inspect the actual cells shared by the TUI, GUI surface and headless render.
fn selected_rows(test: &mut EditorTest, width: u16, rows: usize) -> Vec<String> {
    let scheme = test
        .editor
        .get_color_scheme()
        .cloned()
        .unwrap_or_else(ColorScheme::tokyonight);
    let selection_bg = ovim::key_convert::convert_core_color(scheme.get_ui_color(UiGroup::Visual));
    let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    let cells = terminal.backend().buffer();
    (0..rows as u16)
        .map(|row| {
            (0..width)
                .filter_map(|col| {
                    let cell = &cells[(col, row)];
                    (cell.bg == selection_bg).then(|| cell.symbol())
                })
                .collect()
        })
        .collect()
}

#[test]
fn word_highlight_after_combining_character_matches_the_selected_text() {
    for wrap in [false, true] {
        let mut test = EditorTest::new("é word tail");
        test.editor.options.wrap = wrap;
        test.keys("llviw");
        assert_eq!(selected_rows(&mut test, 40, 1), ["word"], "wrap={wrap}");
    }
}

#[test]
fn quote_highlight_after_emoji_matches_the_selected_text() {
    for wrap in [false, true] {
        let mut test = EditorTest::new("👩‍💻 \"hi\" tail");
        test.editor.options.wrap = wrap;
        test.keys("lllvi\"");
        assert_eq!(selected_rows(&mut test, 40, 1), ["hi"], "wrap={wrap}");
    }
}

#[test]
fn block_highlight_keeps_whole_graphemes_on_every_row() {
    for wrap in [false, true] {
        let mut test = EditorTest::new("aéx\nb👩‍💻y\nczq");
        test.editor.options.wrap = wrap;
        test.keys("l<C-v>jj");
        assert_eq!(
            selected_rows(&mut test, 40, 3),
            ["é", "👩‍💻", "z"],
            "wrap={wrap}"
        );
    }
}

#[test]
fn selected_tab_highlights_its_entire_expansion() {
    let mut test = EditorTest::new("\tx");
    test.keys("v");
    assert_eq!(selected_rows(&mut test, 40, 1), ["    "]);
}

#[test]
fn selection_survives_soft_wrapping_after_a_combining_character() {
    let mut test = EditorTest::new("é 1234567890abcdef tail");
    test.editor.options.wrap = true;
    test.keys("llviw");
    assert_eq!(selected_rows(&mut test, 14, 6).concat(), "1234567890abcdef");
}

#[test]
fn cursorline_keeps_the_visual_selection_visible() {
    let mut test = EditorTest::new("hello world");
    test.editor.options.cursorline = true;
    test.keys("viw");
    assert_eq!(selected_rows(&mut test, 40, 1), ["hello"]);
}

#[test]
fn horizontal_scroll_preserves_selection_after_a_partially_scrolled_wide_character() {
    let mut test = EditorTest::new("界word more text beyond the viewport");
    test.editor.options.wrap = false;
    test.keys("lv3l");
    test.editor.init_window_manager(18, 12);
    test.editor
        .window_manager_mut()
        .unwrap()
        .focused_window_mut()
        .unwrap()
        .set_horizontal_offset(1);
    assert_eq!(selected_rows(&mut test, 18, 1), ["word"]);
}
