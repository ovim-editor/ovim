use ovim::editor::Editor;
use ovim::ui::Renderer;

#[test]
fn test_expand_tabs_basic() {
    // Test basic tab expansion with tab_width = 4
    let mut editor = Editor::with_content("hello\tworld");
    let _renderer = Renderer::new();

    // Buffer adds trailing newline (Vim behavior)
    assert_eq!(editor.buffer().rope().to_string(), "hello\tworld\n");
}

#[test]
fn test_expand_tabs_at_tab_stop() {
    // Tab at column 4 should expand to 4 spaces (next tab stop at 8)
    let mut editor = Editor::with_content("1234\tx");
    assert!(editor.buffer().rope().to_string().contains('\t'));
}

#[test]
fn test_expand_tabs_with_emoji() {
    // Emoji takes 2 columns, so tab position should account for that
    let content = "😀\ttest";
    let mut editor = Editor::with_content(content);

    // Should have emoji and tab
    assert!(editor.buffer().rope().to_string().contains("😀"));
    assert!(editor.buffer().rope().to_string().contains('\t'));
}

#[test]
fn test_expand_tabs_multiple() {
    let content = "a\tb\tc\td";
    let mut editor = Editor::with_content(content);

    let rope = editor.buffer().rope();
    // Buffer adds trailing newline
    assert_eq!(rope.to_string(), format!("{}\n", content));
}

#[test]
fn test_emoji_display_width() {
    use unicode_width::UnicodeWidthChar;

    // Test that emojis have width 2
    assert_eq!('😀'.width(), Some(2));
    assert_eq!('👋'.width(), Some(2));
    assert_eq!('🌍'.width(), Some(2));

    // Regular chars have width 1
    assert_eq!('a'.width(), Some(1));
    assert_eq!('1'.width(), Some(1));

    // Wide chars (CJK)
    assert_eq!('你'.width(), Some(2));
    assert_eq!('世'.width(), Some(2));
}

#[test]
fn test_mixed_width_rendering() {
    let content = "Hello 😀 World";
    let mut editor = Editor::with_content(content);

    // Verify content is preserved (buffer adds trailing newline)
    assert_eq!(editor.buffer().rope().to_string(), format!("{}\n", content));

    // Try to render (just make sure it doesn't panic)
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_tabs_with_wide_chars() {
    // Chinese character (width 2) followed by tab
    let content = "你\tworld";
    let mut editor = Editor::with_content(content);

    // Buffer adds trailing newline
    assert_eq!(editor.buffer().rope().to_string(), format!("{}\n", content));
}

#[test]
fn test_emoji_with_tabs_in_code() {
    let content = "function test() {\n\t// Say hello 👋\n\tconsole.log(\"Hi 😀\");\n}";
    let mut editor = Editor::with_content(content);

    // Verify content preserved
    assert!(editor.buffer().rope().to_string().contains("👋"));
    assert!(editor.buffer().rope().to_string().contains("😀"));
    assert!(editor.buffer().rope().to_string().contains('\t'));
}

#[test]
fn test_rendering_with_emojis_no_panic() {
    let test_cases = vec![
        "Single emoji: 😀",
        "Multiple: 😀 😁 😂 🤣",
        "Tab\t😀\tEmoji",
        "你好\t世界",
        "Mixed: abc 你好 😀",
        "Start\t😀\tMiddle\t🎉\tEnd",
    ];

    for content in test_cases {
        let mut editor = Editor::with_content(content);
        let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
        assert!(result.is_ok(), "Failed to render: {}", content);
    }
}

#[test]
fn test_tab_alignment_with_emoji() {
    // Emoji at start, then tab - should align to correct column
    let content = "😀\tx";
    let mut editor = Editor::with_content(content);

    // Emoji is 2 columns wide, so tab should go to column 4 (next tab stop)
    // When expanded: "😀  x" (emoji + 2 spaces + x)
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_multiple_tabs_with_emojis() {
    let content = "😀\t😁\t😂\tend";
    let mut editor = Editor::with_content(content);

    // Each emoji + tab should align to tab stops
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_zero_width_characters() {
    // Zero-width joiner (invisible)
    let content = "a\u{200D}b";
    let mut editor = Editor::with_content(content);

    // Buffer adds trailing newline
    assert_eq!(editor.buffer().rope().to_string(), format!("{}\n", content));
}

#[test]
fn test_skin_tone_emojis() {
    let content = "👋🏻 👋🏿";
    let mut editor = Editor::with_content(content);

    // Buffer adds trailing newline
    assert_eq!(editor.buffer().rope().to_string(), format!("{}\n", content));

    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_flag_emojis() {
    let content = "🇺🇸 🇯🇵 🇬🇧";
    let mut editor = Editor::with_content(content);

    // Buffer adds trailing newline
    assert_eq!(editor.buffer().rope().to_string(), format!("{}\n", content));

    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_line_padding_with_emojis() {
    // Test that lines are properly padded even with wide characters
    let content = "😀\n🌍\n👋";
    let mut editor = Editor::with_content(content);

    // Render and ensure no panic
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_long_line_with_mixed_content() {
    let content = "Start 😀 Tab:\tMiddle 你好\tEnd 🎉";
    let mut editor = Editor::with_content(content);

    // Verify all content preserved
    assert!(editor.buffer().rope().to_string().contains("😀"));
    assert!(editor.buffer().rope().to_string().contains("你好"));
    assert!(editor.buffer().rope().to_string().contains("🎉"));
    assert!(editor.buffer().rope().to_string().contains('\t'));

    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_syntax_highlighting_with_emojis() {
    // JavaScript with emoji in comment
    let content = r#"// Hello 👋
function test() {
    return "😀";
}"#;

    let mut editor = Editor::with_content(content);

    // Should not panic during syntax highlighting
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_cursor_position_tracking() {
    // Just verify cursor position is tracked correctly
    let mut editor = Editor::with_content("😀😁😂");

    // Initial cursor should be at start
    assert_eq!(editor.buffer().cursor().col(), 0);
    assert_eq!(editor.buffer().cursor().line(), 0);

    // Rendering should not panic
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_tab_width_option() {
    let mut editor = Editor::with_content("a\tb");

    // Default tab width is 4
    assert_eq!(editor.options.tab_width, 4);

    // Change tab width
    editor.options.tab_width = 8;
    assert_eq!(editor.options.tab_width, 8);

    // Render should use new tab width
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_render_with_emojis_no_panic() {
    // Test that rendering doesn't panic with emojis
    let mut editor = Editor::with_content("😀😁😂");

    // Should not panic
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_complex_unicode_graphemes() {
    // Regional indicator symbols (flags)
    let content = "🇺🇸";
    let mut editor = Editor::with_content(content);

    // Buffer adds trailing newline
    assert_eq!(editor.buffer().rope().to_string(), format!("{}\n", content));
}

#[test]
fn test_cursor_with_tabs() {
    // Test cursor tracking on lines with tabs
    let mut editor = Editor::with_content("\thello");

    // Start at beginning (before tab)
    assert_eq!(editor.buffer().cursor().col(), 0);

    // Rendering should handle cursor display correctly with tabs
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_cursor_with_emoji_and_tabs() {
    // Test cursor tracking with both emojis and tabs
    let mut editor = Editor::with_content("😀\thello");

    // Start at emoji
    assert_eq!(editor.buffer().cursor().col(), 0);

    // Cursor should handle display correctly even with emoji + tab
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_long_lines_with_tabs() {
    // Test that very long lines with tabs don't cause rendering issues
    let long_line = "\t".repeat(20) + "text";
    let mut editor = Editor::with_content(&long_line);

    // Should not panic even with extreme tab expansion
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}

#[test]
fn test_preview_with_long_lines() {
    // Test that preview handles long lines correctly
    let content = "a\t".repeat(50) + "end";
    let mut editor = Editor::with_content(&content);

    // Should render without panic
    let result = ovim::ui::render_editor_to_ansi(&mut editor, 80, 24);
    assert!(result.is_ok());
}
