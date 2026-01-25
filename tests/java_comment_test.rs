use ovim::buffer::Buffer;
use ovim::syntax::{HighlightGroup, Language, LanguageRegistry, SyntaxHighlighter};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

/// Tests that Buffer.highlights_for_line returns correct Comment highlights for Java block comments
/// This is an end-to-end test through the actual Buffer interface
#[test]
fn test_buffer_java_block_comment_highlighting() {
    let source = "/*\n * License\n */\npublic class Test {}";

    // Create a buffer with a Java file path to trigger language detection
    let mut buffer = Buffer::new_from_str(source);
    buffer.set_file_path("/tmp/Test.java".to_string());
    buffer.enable_syntax_highlighting();

    // Verify syntax highlighting was enabled
    assert!(
        buffer.has_syntax_highlighting(),
        "Buffer should have syntax highlighting enabled for .java files"
    );

    // Check that block comment lines have Comment highlight
    for line_idx in 0..3 {
        let highlights = buffer.highlights_for_line(line_idx);
        let has_comment = highlights.iter().any(|(_, g)| *g == HighlightGroup::Comment);
        assert!(
            has_comment,
            "Line {} should have Comment highlight via Buffer.highlights_for_line(), got: {:?}",
            line_idx, highlights
        );
    }
}

#[test]
fn test_java_license_header_block_comment() {
    // Mimic a real license header like in KeyUse.java
    let source = r#"/*
 * Copyright (c) 2012-2017 The ANTLR Project. All rights reserved.
 * Use of this file is governed by the BSD 3-clause license that
 * can be found in the LICENSE.txt file in the project root.
 */
package org.antlr.v4.runtime.atn;

public enum KeyUse {
    sig("sig"),
    enc("enc");
}"#;

    let mut highlighter = SyntaxHighlighter::new(Language::Java).unwrap();
    highlighter.parse(source);
    let highlights = highlighter.highlights_for_all_lines(source);

    // Debug: print all highlights
    for (line_idx, line_highlights) in highlights.iter().enumerate() {
        let line_text: &str = source.lines().nth(line_idx).unwrap_or("");
        println!(
            "Line {}: {:?} -> {:?}",
            line_idx, line_text, line_highlights
        );
    }

    // Lines 0-4 are the block comment
    for line_idx in 0..5 {
        let has_comment = highlights[line_idx]
            .iter()
            .any(|(_, g)| *g == HighlightGroup::Comment);
        assert!(
            has_comment,
            "Line {} should have Comment highlight, got: {:?}",
            line_idx, highlights[line_idx]
        );
    }
}

#[test]
fn test_java_block_comment_highlighting() {
    let source = "/* block comment */\npublic class Test { }";

    // Step 1: Verify tree-sitter produces block_comment node
    let ts_lang = LanguageRegistry::get_tree_sitter_language(Language::Java);
    let mut parser = Parser::new();
    parser.set_language(&ts_lang).unwrap();
    let tree = parser.parse(source, None).unwrap();

    println!("Parse tree: {}", tree.root_node().to_sexp());

    // Step 2: Verify query captures block_comment
    let query_source = LanguageRegistry::get_highlight_query(Language::Java);
    let query = Query::new(&ts_lang, query_source).unwrap();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    let mut found_block_comment = false;
    while let Some(m) = matches.next() {
        for capture in m.captures {
            let name = &query.capture_names()[capture.index as usize];
            println!(
                "Capture: {} -> {:?} @ {}-{}",
                name,
                capture.node.kind(),
                capture.node.start_byte(),
                capture.node.end_byte()
            );
            if *name == "comment" && capture.node.kind() == "block_comment" {
                found_block_comment = true;
            }
        }
    }

    assert!(
        found_block_comment,
        "Query should capture block_comment as @comment"
    );

    // Step 3: Verify SyntaxHighlighter distributes to lines
    let mut highlighter = SyntaxHighlighter::new(Language::Java).unwrap();
    highlighter.parse(source);
    let highlights = highlighter.highlights_for_all_lines(source);

    let line0_has_comment = highlights[0]
        .iter()
        .any(|(_, g)| *g == HighlightGroup::Comment);
    assert!(
        line0_has_comment,
        "Line 0 should have Comment highlight, got: {:?}",
        highlights[0]
    );
}

#[test]
fn test_java_multiline_block_comment() {
    let source = "/*\n * Multi-line\n */\nclass Test {}";

    let mut highlighter = SyntaxHighlighter::new(Language::Java).unwrap();
    highlighter.parse(source);
    let highlights = highlighter.highlights_for_all_lines(source);

    // All 3 comment lines should have Comment highlight
    for line_idx in 0..3 {
        let has_comment = highlights[line_idx]
            .iter()
            .any(|(_, g)| *g == HighlightGroup::Comment);
        assert!(
            has_comment,
            "Line {} should have Comment highlight, got: {:?}",
            line_idx, highlights[line_idx]
        );
    }
}
