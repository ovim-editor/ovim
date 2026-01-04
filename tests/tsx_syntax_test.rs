use ovim::syntax::{Language, LanguageRegistry};

#[test]
fn test_tsx_detection() {
    // Test .tsx extension
    let lang = LanguageRegistry::detect_from_path("Component.tsx");
    assert_eq!(lang, Some(Language::Tsx));

    // Test .ts extension (should be TypeScript, not TSX)
    let lang = LanguageRegistry::detect_from_path("module.ts");
    assert_eq!(lang, Some(Language::TypeScript));

    // Test .mts extension (module TypeScript)
    let lang = LanguageRegistry::detect_from_path("module.mts");
    assert_eq!(lang, Some(Language::TypeScript));
}

#[test]
fn test_tsx_lsp_language_id() {
    // TSX should get "typescriptreact" LSP identifier
    let lsp_id = LanguageRegistry::get_lsp_language_id("Component.tsx");
    assert_eq!(lsp_id, Some("typescriptreact"));

    // Regular TypeScript should get "typescript" LSP identifier
    let lsp_id = LanguageRegistry::get_lsp_language_id("module.ts");
    assert_eq!(lsp_id, Some("typescript"));
}

#[test]
fn test_tsx_tree_sitter_grammar() {
    // Verify TSX uses LANGUAGE_TSX grammar (different from LANGUAGE_TYPESCRIPT)
    let tsx_grammar = LanguageRegistry::get_tree_sitter_language(Language::Tsx);
    let ts_grammar = LanguageRegistry::get_tree_sitter_language(Language::TypeScript);

    // The grammars should be different (TSX has JSX parsing, TypeScript doesn't)
    // We can't directly compare tree_sitter::Language objects, but we can verify they're different
    // by checking their node kind counts or other properties
    let tsx_kind_count = tsx_grammar.node_kind_count();
    let ts_kind_count = ts_grammar.node_kind_count();

    // TSX should have more node kinds than TypeScript due to JSX elements
    assert!(
        tsx_kind_count > ts_kind_count,
        "TSX grammar should have more node kinds than TypeScript grammar due to JSX support"
    );
}

#[test]
fn test_tsx_highlight_query() {
    // Both should use the same highlight query
    let tsx_query = LanguageRegistry::get_highlight_query(Language::Tsx);
    let ts_query = LanguageRegistry::get_highlight_query(Language::TypeScript);

    // The queries should be the same (both use HIGHLIGHTS_QUERY from tree-sitter-typescript)
    assert_eq!(
        tsx_query, ts_query,
        "TSX and TypeScript should share the same highlight query"
    );
}
