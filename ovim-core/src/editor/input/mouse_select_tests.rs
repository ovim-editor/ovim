//! Expected values were recorded from `nvim --clean` (`mouse=a`) by sending SGR
//! mouse sequences to a real terminal nvim over tmux, on
//! `foo bar baz / hello world  x / last line`. Positions below are 0-based
//! (line, grapheme column); nvim's 1-based `getpos('v')`/`getpos('.')` are
//! quoted per test. See notes/TESTING_VIM_SEMANTICS.md.

use super::*;
use crate::key::{KeyCode, KeyEvent, Modifiers};

const TEXT: &str = "foo bar baz\nhello world  x\nlast line\n";

fn editor() -> Editor {
    Editor::with_content(TEXT)
}

fn key(editor: &mut Editor, text: &str) {
    for c in text.chars() {
        let event = if c == '\x1b' {
            KeyEvent::new(KeyCode::Esc, Modifiers::NONE)
        } else {
            KeyEvent::new(KeyCode::Char(c), Modifiers::NONE)
        };
        crate::editor::InputHandler::handle_key_event(editor, event).unwrap();
    }
}

/// `count` consecutive presses (each released) like a multi-click.
fn click(editor: &mut Editor, line: usize, col: usize, count: u8) {
    for n in 1..=count {
        editor.pointer_press(line, col, n, false);
        editor.pointer_release();
    }
}

fn selection(editor: &Editor) -> ((usize, usize), (usize, usize), Mode) {
    (
        editor.visual_start().expect("visual anchor"),
        cursor(editor),
        editor.mode(),
    )
}

fn cursor(editor: &Editor) -> (usize, usize) {
    let cursor = editor.buffer().cursor();
    (cursor.line(), cursor.col().0)
}

#[test]
fn drag_enters_charwise_visual_anchored_at_the_press() {
    // nvim: press (1,3), drag, release at (2,9) -> `v` v=[1,3] cur=[2,9].
    let mut editor = editor();
    editor.pointer_press(0, 2, 1, false);
    assert_eq!(editor.mode(), Mode::Normal);
    editor.pointer_drag(0, 4);
    editor.pointer_drag(1, 8);
    editor.pointer_release();
    assert_eq!(selection(&editor), ((0, 2), (1, 8), Mode::Visual));
}

#[test]
fn drag_backwards_keeps_the_press_as_anchor() {
    // nvim: press (2,9), drag to (1,3) -> `v` v=[2,9] cur=[1,3].
    let mut editor = editor();
    editor.pointer_press(1, 8, 1, false);
    editor.pointer_drag(0, 2);
    editor.pointer_release();
    assert_eq!(selection(&editor), ((1, 8), (0, 2), Mode::Visual));
}

#[test]
fn drag_inside_the_press_cell_does_not_start_visual_mode() {
    // nvim: press (1,3), drag to the same cell, release -> mode `n`.
    let mut editor = editor();
    editor.pointer_press(0, 2, 1, false);
    editor.pointer_drag(0, 2);
    editor.pointer_release();
    assert_eq!(editor.mode(), Mode::Normal);
    assert_eq!(cursor(&editor), (0, 2));
    // Once Visual, returning to the press cell keeps it (one-character selection).
    editor.pointer_press(0, 2, 1, false);
    editor.pointer_drag(0, 3);
    editor.pointer_drag(0, 2);
    assert_eq!(selection(&editor), ((0, 2), (0, 2), Mode::Visual));
}

#[test]
fn drag_past_the_end_or_below_the_text_clamps_to_the_buffer() {
    // nvim: dragging below the last line lands on (3,3); a far-right drag lands on
    // the line's end (nvim also reaches the newline cell there; ovim's Visual
    // cursor stops on the last character, as the TUI always has).
    let mut editor = editor();
    editor.pointer_press(0, 2, 1, false);
    editor.pointer_drag(1, 400);
    assert_eq!(selection(&editor), ((0, 2), (1, 13), Mode::Visual));
    editor.pointer_drag(99, 2);
    assert_eq!(selection(&editor), ((0, 2), (2, 2), Mode::Visual));
}

#[test]
fn double_click_selects_the_word_under_the_pointer() {
    // nvim: `bar` double-clicked at (1,6) -> `v` v=[1,5] cur=[1,7].
    let mut editor = editor();
    click(&mut editor, 0, 5, 2);
    assert_eq!(selection(&editor), ((0, 4), (0, 6), Mode::Visual));
    // First word, nvim: v=[1,1] cur=[1,3].
    let mut editor = self::editor();
    click(&mut editor, 0, 0, 2);
    assert_eq!(selection(&editor), ((0, 0), (0, 2), Mode::Visual));
}

#[test]
fn double_click_on_whitespace_or_punctuation_selects_that_run() {
    // nvim: second of two spaces at (2,13) -> `v` v=[2,12] cur=[2,13].
    let mut editor = editor();
    click(&mut editor, 1, 12, 2);
    assert_eq!(selection(&editor), ((1, 11), (1, 12), Mode::Visual));
    // Punctuation run: same character-class rule as `iw` (`a ==> b`, click `=`).
    let mut editor = Editor::with_content("a ==> b\n");
    click(&mut editor, 0, 3, 2);
    assert_eq!(selection(&editor), ((0, 2), (0, 4), Mode::Visual));
}

#[test]
fn double_click_past_the_end_selects_the_last_word() {
    // nvim: double-click right of `x` at (2,40) -> `v` v=[2,14] cur=[2,14].
    let mut editor = editor();
    click(&mut editor, 1, 400, 2);
    assert_eq!(selection(&editor), ((1, 13), (1, 13), Mode::Visual));
}

#[test]
fn double_click_on_an_empty_line_enters_visual_mode_there() {
    let mut editor = Editor::with_content("a\n\nb\n");
    click(&mut editor, 1, 0, 2);
    assert_eq!(selection(&editor), ((1, 0), (1, 0), Mode::Visual));
}

#[test]
fn double_click_while_visual_restarts_the_selection_on_the_word() {
    // nvim: `vl` then double-click `bar` -> `v` v=[1,5] cur=[1,7].
    let mut editor = editor();
    key(&mut editor, "vl");
    click(&mut editor, 0, 5, 2);
    assert_eq!(selection(&editor), ((0, 4), (0, 6), Mode::Visual));
}

#[test]
fn dragging_after_a_double_click_extends_by_words() {
    // nvim: double-click `bar`, drag to (2,9) inside `world` -> v=[1,5] cur=[2,11].
    let mut editor = editor();
    click(&mut editor, 0, 5, 1);
    editor.pointer_press(0, 5, 2, false);
    editor.pointer_drag(1, 8);
    editor.pointer_release();
    assert_eq!(selection(&editor), ((0, 4), (1, 10), Mode::Visual));
    // nvim: double-click `world` at (2,10), drag back to (1,3) -> v=[2,11] cur=[1,1]:
    // the anchor flips to the end of the original word.
    let mut editor = self::editor();
    click(&mut editor, 1, 9, 1);
    editor.pointer_press(1, 9, 2, false);
    editor.pointer_drag(0, 2);
    assert_eq!(selection(&editor), ((1, 10), (0, 0), Mode::Visual));
    // nvim: double-click `world` at (2,8), drag left to (2,7) -> v=[2,11] cur=[2,7].
    let mut editor = self::editor();
    click(&mut editor, 1, 7, 1);
    editor.pointer_press(1, 7, 2, false);
    editor.pointer_drag(1, 6);
    assert_eq!(selection(&editor), ((1, 10), (1, 6), Mode::Visual));
}

#[test]
fn triple_click_selects_the_line_and_drag_extends_by_lines() {
    // nvim: triple-click (2,4) -> `V` v=[2,4] cur=[2,4].
    let mut editor = editor();
    click(&mut editor, 1, 3, 3);
    assert_eq!(selection(&editor), ((1, 3), (1, 3), Mode::VisualLine));
    // nvim: triple-click (1,6), drag to (2,9) -> `V` v=[1,6] cur=[2,9].
    let mut editor = self::editor();
    click(&mut editor, 0, 5, 2);
    editor.pointer_press(0, 5, 3, false);
    editor.pointer_drag(1, 8);
    editor.pointer_release();
    assert_eq!(selection(&editor), ((0, 5), (1, 8), Mode::VisualLine));
}

#[test]
fn shift_click_in_normal_mode_selects_from_the_old_cursor() {
    // nvim: cursor (1,3), shift-click (2,6) -> `v` v=[1,3] cur=[2,6].
    let mut editor = editor();
    key(&mut editor, "ll");
    editor.pointer_press(1, 5, 1, true);
    editor.pointer_release();
    assert_eq!(selection(&editor), ((0, 2), (1, 5), Mode::Visual));
}

#[test]
fn shift_click_in_visual_mode_moves_the_end_nearest_the_click() {
    // nvim, selection (2,3)-(2,5) (`jllvll`): shift-click (1,1) -> v=[2,5] cur=[1,1].
    let mut editor = editor();
    key(&mut editor, "jllvll");
    editor.pointer_press(0, 0, 1, true);
    assert_eq!(selection(&editor), ((1, 4), (0, 0), Mode::Visual));
    // nvim, selection (2,3)-(2,8) (`jllvlllll`): shift-click (2,6) -> v=[2,3] cur=[2,6].
    let mut editor = self::editor();
    key(&mut editor, "jllvlllll");
    editor.pointer_press(1, 5, 1, true);
    assert_eq!(selection(&editor), ((1, 2), (1, 5), Mode::Visual));
    // ... and shift-click (2,4) -> v=[2,8] cur=[2,4].
    let mut editor = self::editor();
    key(&mut editor, "jllvlllll");
    editor.pointer_press(1, 3, 1, true);
    assert_eq!(selection(&editor), ((1, 7), (1, 3), Mode::Visual));
}

#[test]
fn shift_click_keeps_linewise_visual() {
    // nvim: `Vj` then shift-click (3,9) -> `V` v=[1,1] cur=[3,9].
    let mut editor = editor();
    key(&mut editor, "Vj");
    editor.pointer_press(2, 8, 1, true);
    assert_eq!(selection(&editor), ((0, 0), (2, 8), Mode::VisualLine));
}

#[test]
fn plain_click_while_visual_leaves_visual_mode() {
    // nvim: `llvl`, click (2,9) -> mode `n`, cursor (2,9).
    let mut editor = editor();
    key(&mut editor, "llvl");
    editor.pointer_press(1, 8, 1, false);
    assert_eq!(editor.mode(), Mode::Normal);
    assert_eq!(cursor(&editor), (1, 8));
}

#[test]
fn starting_a_selection_from_insert_mode_ends_the_insert_session() {
    // nvim: from Insert, dragging (1,3)->(1,8) gives `v` v=[1,3] cur=[1,8]; a
    // double-click selects the word (`v` v=[1,5] cur=[1,7]); shift-click gives
    // `v` v=[1,1] cur=[1,8].
    let mut editor = editor();
    key(&mut editor, "iXY");
    editor.pointer_press(0, 4, 1, false);
    assert_eq!(editor.mode(), Mode::Insert);
    editor.pointer_drag(0, 8);
    editor.pointer_release();
    assert_eq!(selection(&editor), ((0, 4), (0, 8), Mode::Visual));
    // The typed text became its own undo step.
    key(&mut editor, "\x1b");
    assert_eq!(editor.buffer().rope().to_string(), format!("XY{TEXT}"));
    editor.undo();
    assert_eq!(editor.buffer().rope().to_string(), TEXT);

    let mut editor = self::editor();
    key(&mut editor, "i");
    click(&mut editor, 0, 5, 2);
    assert_eq!(selection(&editor), ((0, 4), (0, 6), Mode::Visual));

    let mut editor = self::editor();
    key(&mut editor, "i");
    editor.pointer_press(0, 7, 1, true);
    assert_eq!(selection(&editor), ((0, 0), (0, 7), Mode::Visual));
}

#[test]
fn drag_without_a_press_does_nothing() {
    let mut editor = editor();
    editor.pointer_drag(1, 3);
    assert_eq!(editor.mode(), Mode::Normal);
    assert_eq!(cursor(&editor), (0, 0));
}
