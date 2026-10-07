//! Column allocation when the file tree, test panel, debug panel and docked
//! AI chat compete with the editor for a small terminal.

use ovim::editor::Editor;
use ovim::frontend::handle_viewport_resize;
use ovim::mode::Mode;
use ovim::ui::Renderer;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

/// What the renderer decided for one frame.
struct Frame {
    buffer_x: u16,
    buffer_width: u16,
    chat: Option<(u16, u16)>,
    cursor: (u16, u16),
}

fn render(editor: &mut Editor, width: u16, height: u16) -> Frame {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut cache = Default::default();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, editor, &mut cache))
        .unwrap();
    let buffer = editor.render_cache.last_buffer_area.unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    Frame {
        buffer_x: buffer.x,
        buffer_width: buffer.width,
        chat: editor
            .render_cache
            .last_chat_area
            .map(|area| (area.x, area.width)),
        cursor: (cursor.x, cursor.y),
    }
}

fn editor_with_tree() -> (Editor, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("Cargo.toml"), "[package]").unwrap();
    let mut editor = Editor::with_content("fn main() {}\n");
    editor.open_directory(directory.path()).unwrap();
    editor.set_mode(Mode::Normal);
    // `open_directory` replaces the buffer with the directory listing.
    (editor, directory)
}

#[derive(Clone, Copy, Debug)]
struct Panels {
    tree: bool,
    test: bool,
    debug: bool,
}

fn open(panels: Panels) -> (Editor, Option<tempfile::TempDir>) {
    let (mut editor, directory) = if panels.tree {
        let (editor, directory) = editor_with_tree();
        (editor, Some(directory))
    } else {
        (Editor::with_content("fn main() {}\n"), None)
    };
    if panels.test {
        editor.toggle_test_panel();
    }
    if panels.debug {
        editor.toggle_debug_panels();
    }
    (editor, directory)
}

const COMBINATIONS: [Panels; 4] = [
    Panels {
        tree: true,
        test: false,
        debug: false,
    },
    Panels {
        tree: true,
        test: true,
        debug: false,
    },
    Panels {
        tree: true,
        test: true,
        debug: true,
    },
    Panels {
        tree: false,
        test: true,
        debug: true,
    },
];

/// With the tree at 80x24 the editor was 8 columns wide, at 72x14 a single
/// column with the cursor drawn inside the debug panel.
#[test]
fn side_panels_never_squeeze_the_editor_below_a_usable_width() {
    for (width, height) in [(80, 24), (72, 14), (60, 20), (100, 30), (140, 40)] {
        for panels in COMBINATIONS {
            let (mut editor, _directory) = open(panels);
            let frame = render(&mut editor, width, height);
            assert!(
                frame.buffer_width >= 40,
                "{width}x{height} {panels:?}: editor is {} columns",
                frame.buffer_width
            );
            assert!(
                frame.cursor.0 >= frame.buffer_x
                    && frame.cursor.0 < frame.buffer_x + frame.buffer_width,
                "{width}x{height} {panels:?}: cursor at {:?} outside the editor",
                frame.cursor
            );
        }
    }
}

/// With the test and debug panels open the docked chat got no columns at all
/// while its input kept the keyboard focus.
#[test]
fn the_docked_chat_always_keeps_room_for_its_input() {
    for (width, height) in [(80, 24), (100, 30), (120, 30), (160, 40)] {
        for panels in COMBINATIONS {
            let (mut editor, _directory) = open(panels);
            editor
                .open_ai_chat(ovim_core::ai::ChatOpts::default())
                .unwrap();
            assert_eq!(editor.mode(), Mode::AiChat);
            let frame = render(&mut editor, width, height);
            let (chat_x, chat_width) = frame
                .chat
                .unwrap_or_else(|| panic!("{width}x{height} {panels:?}: no chat area"));
            assert!(
                chat_width >= 30,
                "{width}x{height} {panels:?}: chat is {chat_width} columns"
            );
            assert!(
                frame.cursor.0 >= chat_x && frame.cursor.0 < chat_x + chat_width,
                "{width}x{height} {panels:?}: input cursor {:?} outside the chat",
                frame.cursor
            );
        }
    }
}

/// Plenty of room: every panel shows at its preferred width.
#[test]
fn a_wide_terminal_shows_every_panel() {
    let (mut editor, _directory) = open(Panels {
        tree: true,
        test: true,
        debug: true,
    });
    let frame = render(&mut editor, 200, 40);
    assert!(editor.render_cache.test_panel_area.is_some());
    assert!(frame.buffer_width >= 60, "{}", frame.buffer_width);
    assert!(frame.buffer_x >= 24, "tree sits left of the editor");
}

/// A resize used to size the wrap map for a 50 column tree while the renderer
/// laid out the real tree and the panels, so the first frame after every
/// resize rebuilt the whole map at the correct width.
#[test]
fn a_resize_sizes_the_wrap_map_for_the_width_the_renderer_uses() {
    for panels in COMBINATIONS {
        let (mut editor, _directory) = open(panels);
        editor.options.wrap = true;
        editor.options.number = true;
        let long_lines: String = (0..50)
            .map(|n| format!("line {n}: {}\n", "word ".repeat(40)))
            .collect();
        editor.buffer_mut().replace_content(&long_lines);
        for (width, height) in [(100, 30), (80, 24), (72, 14)] {
            handle_viewport_resize(&mut editor, width, height);
            let at_resize = editor.wrap_map().unwrap().wrap_width();
            render(&mut editor, width, height);
            let map = editor.wrap_map().unwrap();
            assert_eq!(
                map.wrap_width(),
                at_resize,
                "{width}x{height} {panels:?}: the frame re-measured at a different width"
            );
            assert_eq!(
                map.last_recomputed_lines(),
                0,
                "{width}x{height} {panels:?}: the frame rebuilt the wrap map"
            );
        }
    }
}

/// The docked chat takes its default share of the width, or the share the
/// user dragged the separator to, and never starves the editor.
#[test]
fn the_docked_chat_takes_its_default_or_dragged_share() {
    let widths = |allow_edits: bool, drag_to: Option<u16>| {
        let mut editor = Editor::with_content("fn main() {}\n");
        editor
            .open_ai_chat(ovim_core::ai::ChatOpts {
                allow_edits,
                ..Default::default()
            })
            .unwrap();
        if let Some(column) = drag_to {
            let area = ovim_core::Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 24,
            };
            assert!(editor.resize_ai_chat_panel(column, area));
        }
        let frame = render(&mut editor, 100, 24);
        (frame.buffer_width, frame.chat.unwrap().1)
    };
    assert_eq!(widths(true, None), (60, 40));
    assert_eq!(widths(false, None), (65, 35));
    assert_eq!(widths(true, Some(45)), (45, 55));
    assert_eq!(
        widths(true, Some(10)),
        (40, 60),
        "the editor keeps 40 columns"
    );
}

/// A tree the user is typing into stays visible even where it cannot fit
/// beside a comfortable editor.
#[test]
fn the_focused_file_tree_stays_visible_on_a_narrow_grid() {
    for (width, height) in [(60, 20), (50, 20), (30, 12)] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("Cargo.toml"), "[package]").unwrap();
        let mut editor = Editor::new();
        editor.open_directory(directory.path()).unwrap();
        assert_eq!(editor.mode(), Mode::FileTree);
        let frame = render(&mut editor, width, height);
        assert!(
            frame.buffer_x >= 15,
            "{width}x{height}: the tree is {} columns",
            frame.buffer_x
        );
        assert!(
            frame.cursor.0 < frame.buffer_x,
            "{width}x{height}: tree cursor {:?} is not in the tree",
            frame.cursor
        );
    }
}

/// Clicks land in the editor column the frame drew, whatever panels sit
/// beside it.
#[test]
fn clicks_map_through_the_column_the_renderer_allocated() {
    use ovim::editor::handle_mouse_event;
    use ovim_core::{MouseButton, MouseEvent, MouseEventKind};

    for panels in COMBINATIONS {
        let (mut editor, _directory) = open(panels);
        editor.options.number = true;
        editor.buffer_mut().replace_content("abcdefghij\n");
        editor.set_mode(Mode::Normal);
        let frame = render(&mut editor, 100, 24);
        let area = editor.render_cache.last_buffer_area.unwrap();
        let column = frame.buffer_x + editor.render_cache.last_gutter_width as u16 + 4;
        handle_mouse_event(
            &mut editor,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row: area.y,
            },
        )
        .unwrap();
        assert_eq!(
            editor.buffer().cursor().col().0,
            4,
            "{panels:?}: click at column {column} (editor starts at {})",
            frame.buffer_x
        );
    }
}
