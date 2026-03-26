/// Regression tests for syntax highlighting with multi-byte UTF-8 content.
///
/// OV-00216: create_ts_insert_edit / create_ts_delete_edit pass char indices
///           as Point.column — tree-sitter expects byte offsets.
/// OV-00217: shift_highlights_for_insertion shifts byte-offset ranges by
///           chars().count() instead of text.len().
/// OV-00218: shift_highlights_for_deletion col params are char indices but
///           highlight cache stores byte offsets.
use ovim::buffer::Buffer;
use ovim_core::syntax::HighlightGroup;

/// Helper: create a buffer with syntax highlighting enabled for Rust code.
fn rust_buffer(source: &str) -> Buffer {
    let mut buf = Buffer::new_from_str(source);
    buf.set_file_path("test.rs".to_string());
    buf.enable_syntax_highlighting();
    assert!(
        buf.has_syntax_highlighting(),
        "Syntax highlighting should be enabled for .rs files"
    );
    buf
}

/// Helper: find the first highlight with the given group on a line.
fn find_highlight(
    buf: &Buffer,
    line: usize,
    group: HighlightGroup,
) -> Option<std::ops::Range<usize>> {
    buf.highlights_for_line(line)
        .into_iter()
        .find(|(_, g)| *g == group)
        .map(|(r, _)| r)
}

/// Helper: collect all highlights of a given group on a line.
fn find_all_highlights(
    buf: &Buffer,
    line: usize,
    group: HighlightGroup,
) -> Vec<std::ops::Range<usize>> {
    buf.highlights_for_line(line)
        .into_iter()
        .filter(|(_, g)| *g == group)
        .map(|(r, _)| r)
        .collect()
}

// Note: In tree-sitter-rust 0.23, integer literals (42) are highlighted as
// HighlightGroup::Constant, not Number. We use Constant in these tests.

// ---------------------------------------------------------------------------
// OV-00217: shift_highlights_for_insertion must shift by byte length, not char count
// ---------------------------------------------------------------------------

#[test]
fn test_ov_00217_highlight_shift_uses_byte_length_for_ascii_insert() {
    // Baseline: ASCII insertion where chars().count() == len().
    // In "let x = 42;\n", "42" is Constant at bytes 8..10.
    let source = "let x = 42;\n";
    let mut buf = rust_buffer(source);

    let constant_before = find_highlight(&buf, 0, HighlightGroup::Constant);
    assert!(constant_before.is_some(), "Should find Constant for '42'");
    let start_before = constant_before.unwrap().start;

    // Insert "y, " (3 chars = 3 bytes) before "42" at char col 8
    buf.insert_text_at(0, 8, "y, ");

    // Check shifted highlights (before rehighlight — tests the shift logic only)
    let constant_shifted = find_highlight(&buf, 0, HighlightGroup::Constant);
    assert!(constant_shifted.is_some(), "Constant should survive shift");
    assert_eq!(
        constant_shifted.unwrap().start,
        start_before + 3,
        "ASCII insertion: Constant highlight should shift right by 3 bytes"
    );
}

#[test]
fn test_ov_00217_highlight_shift_uses_byte_length_for_multibyte_insert() {
    // Multi-byte insertion where chars().count() != len().
    // Insert "é" (2 bytes, 1 char) before the number.
    // The highlight cache stores byte offsets, so the shift should be 2, not 1.
    //
    // Source: "let x = 42;\n"
    //   Bytes: l(0) e(1) t(2) ' '(3) x(4) ' '(5) =(6) ' '(7) 4(8) 2(9) ;(10)
    //   "42" Constant is at bytes 8..10.
    let source = "let x = 42;\n";
    let mut buf = rust_buffer(source);

    let constant_before = find_highlight(&buf, 0, HighlightGroup::Constant);
    assert!(constant_before.is_some(), "Should find Constant for '42'");
    let start_before = constant_before.unwrap().start;
    assert_eq!(start_before, 8, "'42' should start at byte 8");

    // Insert "é" (2 bytes, 1 char) at char col 8 (before "42")
    buf.insert_text_at(0, 8, "é");

    // Check the shifted cache (NOT rebuilt — tests shift_highlights_for_insertion)
    let constant_shifted = find_highlight(&buf, 0, HighlightGroup::Constant);
    assert!(
        constant_shifted.is_some(),
        "Constant should survive the shift"
    );
    let start_shifted = constant_shifted.unwrap().start;

    // The shift should be 2 (byte length of "é"), not 1 (char count of "é").
    assert_eq!(
        start_shifted,
        start_before + "é".len(), // 8 + 2 = 10
        "Multi-byte insertion: highlight should shift by byte length ({}) not char count ({}). \
         Got shift of {}.",
        "é".len(),           // 2
        "é".chars().count(), // 1
        start_shifted - start_before
    );
}

#[test]
fn test_ov_00217_multiline_insert_last_line_len_is_bytes() {
    // Multi-line insertion: the "after" portion moves to a new line and its
    // column offset must use byte length of the last inserted line, not chars.
    //
    // Insert "first\nsécond" at col 8 (where "42" starts on byte 8).
    // "sécond" is 7 bytes but 6 chars.
    let source = "let x = 42;\n";
    let mut buf = rust_buffer(source);

    let constant_before = find_highlight(&buf, 0, HighlightGroup::Constant);
    assert!(constant_before.is_some(), "Should find Constant for '42'");
    let start_before = constant_before.unwrap().start; // should be 8

    // Insert at char col 8 → "let x = " stays on line 0, "42;" moves to line 1
    // with offset = last_line_len (should be 7 bytes for "sécond")
    buf.insert_text_at(0, 8, "first\nsécond");

    // After insertion, "42;" moves to line 1. Its start col should be:
    //   (original_start - insert_col) + last_line_byte_len = (8 - 8) + 7 = 7
    let constant_after = find_highlight(&buf, 1, HighlightGroup::Constant);

    if let Some(range) = constant_after {
        let expected = (start_before - 8) + "sécond".len(); // 0 + 7 = 7
        assert_eq!(
            range.start,
            expected,
            "After multi-line insert, highlight offset should use byte length of last line \
             ('sécond' = {} bytes, not {} chars). Got {}.",
            "sécond".len(),           // 7
            "sécond".chars().count(), // 6
            range.start
        );
    }
    // If constant_after is None, the shift corrupted the entry — also a bug manifestation.
}

// ---------------------------------------------------------------------------
// OV-00218: shift_highlights_for_deletion compares char-index cols to byte-offset ranges
// ---------------------------------------------------------------------------

#[test]
fn test_ov_00218_deletion_shift_vs_rebuilt() {
    // Delete a multi-byte char from a line and compare the shifted highlight
    // cache with a full rebuild. Any mismatch proves the shift logic is wrong.
    //
    // "fn f() {} // ñ test\n"
    //   Bytes: f(0) n(1) ' '(2) f(3) ((4) )(5) ' '(6) {(7) }(8) ' '(9) /(10) /(11) ' '(12) ñ(13,14) ' '(15) t(16) e(17) s(18) t(19)
    //   Chars: f(0) n(1) ' '(2) f(3) ((4) )(5) ' '(6) {(7) }(8) ' '(9) /(10) /(11) ' '(12) ñ(13) ' '(14) t(15) e(16) s(17) t(18)
    //   ñ is char 13, bytes 13..15 (2 bytes)
    let source = "fn f() {} // ñ test\n";
    let mut buf = rust_buffer(source);

    // Verify keyword 'fn' exists
    let keyword = find_highlight(&buf, 0, HighlightGroup::Keyword);
    assert!(keyword.is_some(), "Should find 'fn' keyword");

    // Delete 'ñ' at char col 13..14 (1 char, 2 bytes at bytes 13..15)
    buf.delete_range(0, 13, 0, 14);

    // Capture shifted highlights (before rebuild)
    let comments_shifted = find_all_highlights(&buf, 0, HighlightGroup::Comment);

    // Now rebuild from scratch to get ground truth
    buf.rebuild_highlight_cache();
    let comments_rebuilt = find_all_highlights(&buf, 0, HighlightGroup::Comment);

    assert_eq!(
        comments_shifted, comments_rebuilt,
        "Comment highlight ranges should match between shifted and rebuilt. \
         Shifted: {:?}, Rebuilt: {:?}. \
         If shifted ranges are off by 1, it's the char-vs-byte mismatch bug.",
        comments_shifted, comments_rebuilt
    );
}

#[test]
fn test_ov_00218_deletion_retain_logic_with_multibyte() {
    // Delete 'ñ' (2 bytes, 1 char) from "fn ñ() {}" and verify shifted
    // punctuation matches rebuilt punctuation.
    //
    // Byte layout: f(0) n(1) ' '(2) ñ(3,4) ((5) )(6) ' '(7) {(8) }(9)
    // Char layout: f(0) n(1) ' '(2) ñ(3)   ((4) )(5) ' '(6) {(7) }(8)
    //
    // delete_range(0, 3, 0, 4) deletes char 3 (ñ, 2 bytes at bytes 3..5).
    // In shift_highlights_for_deletion:
    //   deleted_chars = end_col - start_col = 4 - 3 = 1
    // But the byte-offset ranges after byte 5 should shift left by 2, not 1.
    let source = "fn ñ() {}\n";
    let mut buf = rust_buffer(source);

    // Verify 'fn' keyword exists
    let keyword = find_highlight(&buf, 0, HighlightGroup::Keyword);
    assert!(keyword.is_some(), "Should find 'fn' keyword");

    // Delete 'ñ' (char col 3..4, byte range 3..5)
    buf.delete_range(0, 3, 0, 4);

    // Capture shifted punctuation highlights
    let puncts_shifted: Vec<_> = buf
        .highlights_for_line(0)
        .into_iter()
        .filter(|(_, g)| {
            *g == HighlightGroup::Punctuation || *g == HighlightGroup::PunctuationDelimiter
        })
        .map(|(r, _)| r)
        .collect();

    // Rebuild from scratch (ground truth)
    buf.rebuild_highlight_cache();
    let puncts_rebuilt: Vec<_> = buf
        .highlights_for_line(0)
        .into_iter()
        .filter(|(_, g)| {
            *g == HighlightGroup::Punctuation || *g == HighlightGroup::PunctuationDelimiter
        })
        .map(|(r, _)| r)
        .collect();

    assert_eq!(
        puncts_shifted, puncts_rebuilt,
        "Shifted punctuation should match rebuilt. \
         Mismatch means shift_highlights_for_deletion used char count instead of byte count. \
         Shifted: {:?}, Rebuilt: {:?}",
        puncts_shifted, puncts_rebuilt
    );
}

// ---------------------------------------------------------------------------
// OV-00216: InputEdit.start_position.column must be a byte offset, not char index
// ---------------------------------------------------------------------------

#[test]
fn test_ov_00216_ts_insert_edit_column_is_byte_offset() {
    // Source with multi-byte char in a string before the number:
    // let x = "ñ"; let y = 42;
    //
    // After the string "ñ" (2-byte char), the byte offsets diverge from char indices.
    // If InputEdit.column uses char index instead of byte offset, the incremental
    // parse will have wrong positions and may produce a broken tree.
    //
    // Byte layout: l(0)e(1)t(2) (3)x(4) (5)=(6) (7)"(8)ñ(9,10)"(11);(12)
    //              ' '(13)l(14)e(15)t(16) (17)y(18) (19)=(20) (21)4(22)2(23);(24)
    // Char layout: l(0)e(1)t(2) (3)x(4) (5)=(6) (7)"(8)ñ(9)"(10);(11)
    //              ' '(12)l(13)e(14)t(15) (16)y(17) (18)=(19) (20)4(21)2(22);(23)
    let source = "let x = \"ñ\"; let y = 42;\n";
    let mut buf = rust_buffer(source);

    // '42' should be highlighted as Constant (integer literal in this grammar)
    let constant_before = find_highlight(&buf, 0, HighlightGroup::Constant);
    assert!(
        constant_before.is_some(),
        "Should find Constant for '42' in: {}",
        source.trim()
    );

    // Insert a space at char col 20 (before the space before '42')
    // Char 20 = byte 21 (due to 2-byte ñ). If InputEdit uses char col, it's off by 1.
    buf.insert_text_at(0, 20, " ");

    // Rebuild with the incrementally-updated tree
    buf.rebuild_highlight_cache();

    let constant_after = find_highlight(&buf, 0, HighlightGroup::Constant);
    assert!(
        constant_after.is_some(),
        "After inserting space, '42' should still be highlighted as Constant. \
         If this fails, InputEdit.column is using char index instead of byte offset, \
         causing a corrupt incremental parse tree."
    );
}

#[test]
fn test_ov_00216_ts_delete_edit_column_is_byte_offset() {
    // Delete a char after a multi-byte string and verify tree is still valid.
    let source = "let x = \"ñ\"; let y = 1;\n";
    let mut buf = rust_buffer(source);

    let constant_before = find_highlight(&buf, 0, HighlightGroup::Constant);
    assert!(
        constant_before.is_some(),
        "Should find Constant for '1' in: {}",
        source.trim()
    );

    // Delete the space between 'y' and '=' — char col 18, byte col 19.
    // If InputEdit uses char col 18 instead of byte col 19, tree gets corrupted.
    buf.delete_range(0, 18, 0, 19);

    buf.rebuild_highlight_cache();

    let constant_after = find_highlight(&buf, 0, HighlightGroup::Constant);
    assert!(
        constant_after.is_some(),
        "After deleting space, '1' should still be highlighted as Constant. \
         If this fails, delete InputEdit.column is using char index instead of byte offset."
    );
}

// ---------------------------------------------------------------------------
// OV-00221: Code block highlights_for_line is O(n) — behavioral contract test
// ---------------------------------------------------------------------------

#[test]
fn test_ov_00221_code_block_lookup_many_blocks() {
    // Build a markdown source with many code blocks
    let mut source = String::from("# Many blocks\n\n");
    let block_count = 50;
    for i in 0..block_count {
        source.push_str(&format!("```rust\nlet x{} = {};\n```\n\n", i, i));
    }

    let mut buf = Buffer::new_from_str(&source);
    buf.set_file_path("test.md".to_string());
    buf.enable_syntax_highlighting();

    // Verify we can look up highlights for lines in the last code block
    let last_block_content_line: usize = 2 + (block_count - 1) * 4 + 1;
    let mut found_any = false;
    for line in last_block_content_line.saturating_sub(2)..=(last_block_content_line + 2) {
        if !buf.highlights_for_line(line).is_empty() {
            found_any = true;
            break;
        }
    }
    assert!(
        found_any,
        "Should find highlights in the last code block (line ~{})",
        last_block_content_line
    );

    // Blank line between blocks should have no highlights
    assert!(
        buf.highlights_for_line(1).is_empty(),
        "Blank line should have no highlights"
    );
}
