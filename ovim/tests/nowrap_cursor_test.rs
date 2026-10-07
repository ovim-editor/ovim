//! With `nowrap`, a scrolled line must draw each character where the cursor
//! and the mouse expect it: the `<` indicator covers the first visible cell
//! instead of pushing the text one cell to the right.

mod helpers;

use helpers::EditorTest;
use ovim::editor::handle_mouse_event;
use ovim::frontend::handle_viewport_resize;
use ovim::ui::Renderer;
use ovim_core::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::TestBackend, Terminal};

const WIDTH: u16 = 60;
const HEIGHT: u16 = 12;

fn line() -> String {
    // 100 distinct-enough characters: digits cycle, tens marked by a letter.
    (0..100)
        .map(|n| {
            if n % 10 == 0 {
                char::from(b'a' + (n / 10) as u8)
            } else {
                char::from(b'0' + (n % 10) as u8)
            }
        })
        .collect()
}

fn editor() -> EditorTest {
    let mut test = EditorTest::new(&format!("{}\n", line()));
    test.editor.options.wrap = false;
    test.editor.options.number = true;
    handle_viewport_resize(&mut test.editor, WIDTH, HEIGHT);
    test
}

/// The symbol under the hardware cursor and the row the line is drawn on.
fn render(test: &mut EditorTest) -> (String, (u16, u16), String) {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    let buffer = terminal.backend().buffer();
    let under_cursor = buffer[(cursor.x, cursor.y)].symbol().to_string();
    let row = (0..WIDTH)
        .map(|x| buffer[(x, cursor.y)].symbol())
        .collect::<String>();
    (under_cursor, (cursor.x, cursor.y), row)
}

/// `:set nowrap`, `$` on a 100 column line in 60 columns drew the cursor on
/// the cell before the last character.
#[test]
fn the_cursor_sits_on_its_character_after_scrolling_right() {
    let mut test = editor();
    test.keys("$");
    let (under, _, row) = render(&mut test);
    assert_eq!(under, "9", "row: {row:?}");
    assert!(
        row.contains('<'),
        "scrolled line shows the indicator: {row:?}"
    );
}

#[test]
fn the_cursor_is_on_its_character_at_every_column() {
    let text = line();
    let mut test = editor();
    for column in 0..100 {
        test.keys("0");
        if column > 0 {
            test.keys(&format!("{column}l"));
        }
        let (under, _, row) = render(&mut test);
        let expected = text.chars().nth(column).unwrap().to_string();
        assert_eq!(under, expected, "column {column}, row {row:?}");
    }
}

#[test]
fn moving_back_left_keeps_the_cursor_on_its_character() {
    let text = line();
    let mut test = editor();
    test.keys("$");
    for column in (0..99).rev() {
        test.keys("h");
        let (under, _, row) = render(&mut test);
        let expected = text.chars().nth(column).unwrap().to_string();
        assert_eq!(under, expected, "column {column}, row {row:?}");
    }
}

#[test]
fn clicking_a_scrolled_character_puts_the_cursor_on_it() {
    let text = line();
    let mut test = editor();
    test.keys("$");
    let (_, (cursor_x, cursor_y), _) = render(&mut test);
    // Click three cells left of the cursor.
    handle_mouse_event(
        &mut test.editor,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: cursor_x - 3,
            row: cursor_y,
        },
    )
    .unwrap();
    assert_eq!(test.editor.buffer().cursor().col().0, 96);
    let (under, _, _) = render(&mut test);
    assert_eq!(under, text.chars().nth(96).unwrap().to_string());
}
