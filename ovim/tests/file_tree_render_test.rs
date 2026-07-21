use ovim::editor::{Editor, FileTreeAction};
use ovim::mode::Mode;
use ovim::ui::{render_editor_to_ansi, strip_ansi};

#[test]
fn directory_explorer_renders_files_and_discovery_hints() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("src")).unwrap();
    std::fs::write(directory.path().join("Cargo.toml"), "[package]").unwrap();
    std::fs::write(directory.path().join("src/main.rs"), "fn main() {}").unwrap();
    let mut editor = Editor::new();
    editor.open_directory(directory.path()).unwrap();

    let ansi = render_editor_to_ansi(&mut editor, 100, 24).unwrap();
    let plain = strip_ansi(&ansi);

    assert!(plain.contains("Files"), "render was:\n{plain}");
    assert!(plain.contains("Cargo.toml"), "render was:\n{plain}");
    assert!(plain.contains("src"), "render was:\n{plain}");
    assert!(plain.contains("? help"), "render was:\n{plain}");
    assert_eq!(editor.mode(), Mode::FileTree);
}

#[test]
fn file_tree_text_prompt_renders_at_the_bottom_of_the_panel() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("page.astro"), "---\n---").unwrap();
    let mut editor = Editor::new();
    editor.open_directory(directory.path()).unwrap();
    editor
        .file_tree_mut()
        .set_pending_action(FileTreeAction::Filter {
            input: "astro".to_string(),
            cursor: 5,
        });

    let ansi = render_editor_to_ansi(&mut editor, 80, 16).unwrap();
    let plain = strip_ansi(&ansi);

    assert!(plain.contains("filter: astro"), "render was:\n{plain}");
}
