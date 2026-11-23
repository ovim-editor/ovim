mod helpers;
use helpers::EditorTest;
use ovim_core::command_result::CommandResult;
use ovim_core::commands::execute_command;
use ovim_core::unicode::{CharCol, GraphemeCol};
use ovim_core::KeyCode;

fn java() -> EditorTest {
    let mut test = EditorTest::new("public class Demo {\n    public int length(String name) {\n        return name.length();\n    }\n}\n");
    test.editor.set_file_path("/example/Demo.java".into());
    test
}

fn command(test: &mut EditorTest, cmd: &str) {
    let result = execute_command(&mut test.editor, cmd);
    assert!(
        matches!(result, CommandResult::Success(_)),
        "{cmd}: {result:?}"
    );
}

#[test]
fn option_forms_are_reversible_and_do_not_change_source_or_undo() {
    for on in [
        "set pseudo",
        "set pseudocode",
        "set pseudo=on",
        "set pseudocode=true",
        "set pseudo=1",
        "set pseudo!",
    ] {
        for off in [
            "set nopseudo",
            "set nopseudocode",
            "set pseudo=off",
            "set pseudocode=false",
            "set pseudo=0",
            "set pseudo&",
            "set pseudo!",
        ] {
            let mut test = java();
            test.editor
                .buffer_mut()
                .insert_text_at(2, CharCol(8), "// note\n        ");
            let source_id = test.editor.buffer().id();
            let source = test.editor.buffer().rope().to_string();
            command(&mut test, on);
            assert!(test.editor.is_pseudocode_buffer());
            assert!(!test.editor.buffer().is_modified());
            assert!(test.editor.buffer().is_read_only());
            assert!(test.editor.buffer().file_path().is_none());
            assert!(test.editor.buffer().has_forced_highlights());
            assert!(!test.editor.buffer().rope().to_string().contains("public"));
            let query = execute_command(&mut test.editor, "set pseudo?");
            assert!(
                matches!(query, CommandResult::Success(ref response) if response.message.as_deref() == Some("  pseudocode"))
            );
            command(&mut test, off);
            assert_eq!(test.editor.buffer().id(), source_id);
            assert_eq!(test.editor.buffer().rope().to_string(), source);
            assert!(test.editor.buffer().is_modified());
        }
    }
}

#[test]
fn enter_maps_the_selected_identifier_to_source_and_reuses_the_view() {
    let mut test = java();
    let source_id = test.editor.buffer().id();
    command(&mut test, "set pseudo");
    let view_id = test.editor.buffer().id();
    test.editor
        .buffer_mut()
        .cursor_mut()
        .set_position(1, GraphemeCol(4));
    test.press_key(KeyCode::Enter);
    assert_eq!(test.editor.buffer().id(), source_id);
    assert_eq!(test.editor.buffer().cursor().line(), 1);
    assert_eq!(test.editor.buffer().cursor().col(), GraphemeCol(15));
    command(&mut test, "set pseudo");
    assert_eq!(test.editor.buffer().id(), view_id);
    assert_eq!(test.editor.buffer().cursor().line(), 1);
    assert_eq!(test.editor.buffer_count(), 2);
}

#[test]
fn reading_view_blocks_mutations_and_forced_writes_but_allows_search() {
    let mut test = java();
    command(&mut test, "set pseudo");
    let expected = test.editor.buffer().rope().to_string();
    for keys in [
        "dd", "cc", "i", "a", "o", "x", "p", "u", "gU", "gw", "g?", "v",
    ] {
        test.keys(keys);
        assert_eq!(test.editor.buffer().rope().to_string(), expected, "{keys}");
    }
    test.editor.handle_paste_event("overwrite").unwrap();
    assert_eq!(test.editor.buffer().rope().to_string(), expected);
    for cmd in [
        "w",
        "w!",
        "w! /tmp/pseudocode-must-not-write.java",
        "%s/name/other/g",
        "normal dd",
    ] {
        assert!(
            matches!(
                execute_command(&mut test.editor, cmd),
                CommandResult::Error(_)
            ),
            "{cmd}"
        );
    }
    test.keys("/return").press_key(KeyCode::Enter);
    assert_eq!(test.editor.buffer().cursor().line(), 2);
    assert_eq!(test.editor.buffer().rope().to_string(), expected);
}

#[test]
fn unsupported_files_and_invalid_values_leave_current_buffer_unchanged() {
    let mut test = java();
    let id = test.editor.buffer().id();
    for cmd in ["set pseudo=maybe", "set pseudocode=", "set pseudo=2"] {
        assert!(matches!(
            execute_command(&mut test.editor, cmd),
            CommandResult::Error(_)
        ));
        assert_eq!(test.editor.buffer().id(), id);
    }
    test.editor.set_file_path("/example/code.rs".into());
    assert!(matches!(
        execute_command(&mut test.editor, "set pseudo"),
        CommandResult::Error(_)
    ));
    assert_eq!(test.editor.buffer().id(), id);
}

#[test]
fn changed_source_refreshes_before_using_stale_positions() {
    let mut test = java();
    let source_id = test.editor.buffer().id();
    command(&mut test, "set pseudo");
    test.editor
        .get_buffer_by_id_mut(source_id)
        .unwrap()
        .insert_text_at(0, CharCol::ZERO, "// added\n");
    test.press_key(KeyCode::Enter);
    assert!(test.editor.is_pseudocode_buffer());
    assert!(test.editor.buffer().rope().to_string().contains("// added"));
    assert!(test.editor.status_message().contains("Source changed"));
    test.press_key(KeyCode::Enter);
    assert_eq!(test.editor.buffer().id(), source_id);
}

#[test]
fn markdown_view_uses_the_same_java_projection_and_renders_compacted_rows() {
    use ovim::ui::Renderer;
    use ratatui::{backend::TestBackend, Terminal};
    let mut test = EditorTest::new("# Sample\n\n```java\npublic int size(String text) {\n    return text.length();\n}\n```\n\n\nEnd.\n");
    test.editor.set_file_path("/example/README.md".into());
    command(&mut test, "set pseudo");
    assert_eq!(
        test.editor.buffer().rope().to_string(),
        "Sample\n\nsize(text):\n    return text.length()\n\nEnd.\n"
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("size(text):"));
    assert!(!screen.contains("public int"));
    assert!(!screen.contains("```java"));
}

#[cfg(feature = "lua")]
#[test]
fn lua_uses_boolean_options_for_both_namespaces() {
    let mut test = java();
    test.editor.enable_lua().unwrap();
    for namespace in ["ovim", "vim"] {
        test.editor
            .execute_lua(&format!("{namespace}.opt.pseudo = true"))
            .unwrap();
        test.editor.process_lua_commands().unwrap();
        assert!(test.editor.is_pseudocode_buffer());
        test.editor
            .execute_lua(&format!("{namespace}.opt.pseudocode = false"))
            .unwrap();
        test.editor.process_lua_commands().unwrap();
        assert!(!test.editor.is_pseudocode_buffer());
    }
}

#[test]
fn explicit_off_returns_to_source_even_when_the_view_is_stale() {
    let mut test = java();
    let id = test.editor.buffer().id();
    command(&mut test, "set pseudo");
    test.editor
        .get_buffer_by_id_mut(id)
        .unwrap()
        .insert_text_at(0, CharCol::ZERO, "// new\n");
    command(&mut test, "set nopseudo");
    assert_eq!(test.editor.buffer().id(), id);
}

#[test]
fn wrapped_mouse_navigation_maps_to_the_original_unicode_identifier() {
    use ovim::editor::handle_mouse_event;
    use ovim::ui::Renderer;
    use ovim_core::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{backend::TestBackend, Terminal};
    let mut test = EditorTest::new(
        "class A {\n public String こんにちは(String 名前) {\n  return 名前;\n }\n}\n",
    );
    test.editor.set_file_path("/example/A.java".into());
    let source_id = test.editor.buffer().id();
    command(&mut test, "set pseudo");
    test.editor.options.textwidth = None;
    let mut terminal = Terminal::new(TestBackend::new(24, 12)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    let area = test.editor.render_cache.last_buffer_area.unwrap();
    let col = area.x + test.editor.render_cache.last_gutter_width as u16 + 1;
    handle_mouse_event(
        &mut test.editor,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row: area.y + 1,
        },
    )
    .unwrap();
    test.press_key(KeyCode::Enter);
    assert_eq!(test.editor.buffer().id(), source_id);
    assert_eq!(test.editor.buffer().cursor().line(), 1);
    assert_eq!(test.editor.buffer().cursor().col(), GraphemeCol(15));
}

#[tokio::test(flavor = "multi_thread")]
async fn reading_view_preserves_real_file_contents_and_edit_undo_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Demo.java");
    let original = "class Demo {\n int value = 1;\n}\n";
    std::fs::write(&path, original).unwrap();
    let mut test = EditorTest::new("");
    test.editor.open_file(&path).unwrap();
    test.keys("ggI// note").press_enter().press_esc();
    let edited = test.editor.buffer().rope().to_string();
    assert_ne!(edited, original);
    command(&mut test, "set pseudo");
    assert!(matches!(
        execute_command(&mut test.editor, "w!"),
        CommandResult::Error(_)
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    command(&mut test, "set nopseudo");
    assert_eq!(test.editor.buffer().rope().to_string(), edited);
    test.keys("u");
    assert_eq!(test.editor.buffer().rope().to_string(), original);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn a_view_can_be_disabled_after_its_source_buffer_is_deleted() {
    let mut test = java();
    command(&mut test, "set pseudo");
    let view_id = test.editor.buffer().id();
    command(&mut test, "set nopseudo");
    command(&mut test, "bd!");
    assert_eq!(test.editor.buffer().id(), view_id);
    test.press_key(KeyCode::Enter);
    assert!(test
        .editor
        .status_message()
        .contains("Source buffer closed"));
    command(&mut test, "set nopseudo");
    assert!(!test.editor.is_pseudocode_buffer());
    assert!(!test.editor.buffer().is_read_only());
    assert!(test.editor.buffer().rope().to_string().is_empty());
}

#[test]
fn rejected_commands_do_not_leave_pending_prefixes_or_counts() {
    for keys in ["zxj", "2g?j"] {
        let mut test = java();
        command(&mut test, "set pseudo");
        test.keys(keys);
        assert_eq!(test.editor.buffer().cursor().line(), 1, "{keys}");
    }
}

#[test]
fn markdown_document_is_cached_refreshed_and_scoped_to_the_reading_buffer() {
    let mut test = EditorTest::new("# Heading\n\nA **bold** word.\n");
    test.editor.set_file_path("/example/readme.md".into());
    let source_id = test.editor.buffer().id();
    command(&mut test, "set pseudo");
    let view_id = test.editor.buffer().id();
    let document = test.editor.pseudocode_markdown(view_id).unwrap();
    assert!(document.text.contains("# Heading"));
    assert!(document.text.contains("**bold**"));
    assert_eq!(document.view_lines, vec![0, 1, 2]);
    assert!(test.editor.pseudocode_markdown(source_id).is_none());
    command(&mut test, "set nopseudo");
    test.editor
        .buffer_mut()
        .insert_text_at(0, CharCol(2), "Updated ");
    command(&mut test, "set pseudo");
    assert_eq!(test.editor.buffer().id(), view_id);
    assert!(test
        .editor
        .pseudocode_markdown(view_id)
        .unwrap()
        .text
        .starts_with("# Updated Heading"));
}

#[test]
fn terminal_markdown_retains_bold_and_italic_after_concealing_delimiters() {
    use ovim::ui::Renderer;
    use ratatui::{backend::TestBackend, style::Modifier, Terminal};
    let mut test = EditorTest::new("# Title\n\n**Bold** and *Italic*.\n");
    test.editor.set_file_path("/example/readme.md".into());
    command(&mut test, "set pseudo");
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    terminal
        .draw(|frame| Renderer::render_to_frame(frame, &mut test.editor, &mut Default::default()))
        .unwrap();
    let cells = &terminal.backend().buffer().content;
    assert!(cells
        .iter()
        .any(|cell| cell.symbol() == "B" && cell.modifier.contains(Modifier::BOLD)));
    assert!(cells
        .iter()
        .any(|cell| cell.symbol() == "I" && cell.modifier.contains(Modifier::ITALIC)));
}
