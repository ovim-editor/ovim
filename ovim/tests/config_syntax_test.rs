use ovim_core::buffer::Buffer;
use ovim_core::language_catalog::LanguageCatalog;
use ovim_core::syntax::{HighlightGroup, Language, LanguageRegistry, SyntaxHighlighter};

#[test]
fn config_file_detection_and_fences_use_the_same_builtin_grammars() {
    let catalog = LanguageCatalog::built_in();
    for (path, id, language) in [
        ("application.properties", "properties", Language::Properties),
        ("gradle.properties", "properties", Language::Properties),
        ("messages_fr.PROPERTIES", "properties", Language::Properties),
        (
            ".settings/org.eclipse.core.resources.prefs",
            "properties",
            Language::Properties,
        ),
        ("settings.ini", "ini", Language::Ini),
        (".editorconfig", "ini", Language::Ini),
        (".gitconfig", "ini", Language::Ini),
        (".gitmodules", "ini", Language::Ini),
        ("project/.git/config", "ini", Language::Ini),
        ("project/.git/config.worktree", "ini", Language::Ini),
    ] {
        let definition = catalog.detect(path).unwrap();
        assert_eq!(definition.id(), id, "{path}");
        assert!(definition.syntax.is_some());
        assert!(definition.config.lsp.is_none());
        assert_eq!(LanguageRegistry::detect_from_path(path), Some(language));
    }
    for (fence, id) in [
        ("properties", "properties"),
        ("prefs", "properties"),
        ("ini", "ini"),
        ("editorconfig", "ini"),
        ("gitconfig", "ini"),
    ] {
        assert_eq!(catalog.detect_from_info_string(fence).unwrap().id(), id);
    }
    for ambiguous in [
        "application.conf",
        "application.cfg",
        "application.config",
        "config",
    ] {
        assert!(catalog.detect(ambiguous).is_none(), "{ambiguous}");
    }
}

#[test]
fn properties_highlights_comments_keys_values_escapes_and_continuations() {
    let source = "# comment\n! another comment\nservice.name=café\nescaped\\ key : hello\\ world\ncontinued=first\\\n    second\nempty=\nunicode=\\u0041\n";
    let mut syntax = SyntaxHighlighter::new(Language::Properties).unwrap();
    syntax.parse(source);
    assert!(
        !syntax.tree().unwrap().root_node().has_error(),
        "{}",
        syntax.tree().unwrap().root_node()
    );
    let mut buffer = Buffer::new_from_str(source);
    buffer.enable_syntax_highlighting_for_path("application.properties");
    for (line, group) in [
        (0, HighlightGroup::Comment),
        (1, HighlightGroup::Comment),
        (2, HighlightGroup::Property),
        (2, HighlightGroup::String),
        (3, HighlightGroup::Property),
        (5, HighlightGroup::String),
    ] {
        assert!(
            buffer
                .highlights_for_line(line)
                .iter()
                .any(|(_, found)| *found == group),
            "line {line}: {group:?}"
        );
    }
}

#[test]
fn ini_highlights_sections_keys_and_values_including_an_unterminated_last_line() {
    for ending in ["\n", ""] {
        let source = format!("; comment\n# comment\nroot=true\n[*.rs]\nindent_size=4\n[remote \"origin\"]\nurl=https://example.com/repo{ending}");
        let mut buffer = Buffer::new_from_str(&source);
        buffer.enable_syntax_highlighting_for_path(".editorconfig");
        for (line, group) in [
            (0, HighlightGroup::Comment),
            (1, HighlightGroup::Comment),
            (2, HighlightGroup::Property),
            (3, HighlightGroup::Type),
            (4, HighlightGroup::String),
            (5, HighlightGroup::Type),
            (6, HighlightGroup::Property),
            (6, HighlightGroup::String),
        ] {
            assert!(
                buffer
                    .highlights_for_line(line)
                    .iter()
                    .any(|(_, found)| *found == group),
                "ending {ending:?}, line {line}: {group:?}"
            );
        }
    }
}

#[test]
fn markdown_fences_embed_properties_and_ini_highlighting() {
    let source = "```properties\nname=value\n```\n\n```ini\n[section]\nname=value\n```\n";
    let mut buffer = Buffer::new_from_str(source);
    buffer.enable_syntax_highlighting_for_path("README.md");
    assert!(buffer
        .highlights_for_line(1)
        .iter()
        .any(|(_, group)| *group == HighlightGroup::Property));
    assert!(buffer
        .highlights_for_line(5)
        .iter()
        .any(|(_, group)| *group == HighlightGroup::Type));
    assert!(buffer
        .highlights_for_line(6)
        .iter()
        .any(|(_, group)| *group == HighlightGroup::Property));
}
