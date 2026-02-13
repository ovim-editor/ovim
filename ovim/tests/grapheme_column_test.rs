//! Integration tests for OV-00138: grapheme vs char column consistency.
//!
//! These tests verify that cursor operations, search, case toggle, number modify,
//! word-under-cursor, and join-lines all work correctly when the buffer contains
//! multi-codepoint grapheme clusters (ZWJ emoji, combining marks, flags).
//!
//! The key invariant: cursor.col() is always a grapheme index, and rope operations
//! (which work in char indices) convert at the boundary.

mod helpers;
use helpers::EditorTest;

// ===========================================================================
// Toggle case (~) with multi-codepoint graphemes
// ===========================================================================

#[test]
fn test_tilde_skips_emoji_advances_past_it() {
    // "a👨‍👩‍👧‍👦b" — 3 graphemes: 'a' at 0, emoji at 1, 'b' at 2
    // Cursor at 0 ('a'): ~ toggles 'a' → 'A', cursor advances to grapheme 1
    let mut t = EditorTest::new("a👨‍👩‍👧‍👦b");
    t.assert_cursor(0, 0);
    t.keys("~");
    assert_eq!(t.cursor(), (0, 1)); // cursor on emoji (grapheme 1)
    // 'a' → 'A', emoji and 'b' unchanged
    assert!(t.buffer_content().starts_with("A👨‍👩‍👧‍👦b"));
}

#[test]
fn test_tilde_on_emoji_advances_cursor() {
    // Emoji has no case, so ~ should still advance cursor past it
    let mut t = EditorTest::new("a👨‍👩‍👧‍👦b");
    t.set_cursor(0, 1); // cursor on emoji (grapheme 1)
    t.keys("~");
    // Emoji is case-neutral — ~ advances to grapheme 2 ('b')
    assert_eq!(t.cursor(), (0, 2));
}

#[test]
fn test_tilde_after_emoji() {
    // Cursor at 'b' after emoji — should toggle 'b' → 'B'
    let mut t = EditorTest::new("a👨‍👩‍👧‍👦b");
    t.set_cursor(0, 2); // cursor on 'b' (grapheme 2)
    t.keys("~");
    assert!(t.buffer_content().starts_with("a👨‍👩‍👧‍👦B"));
}

#[test]
fn test_tilde_with_combining_mark() {
    // "e\u{0301}x" = é + x : 2 graphemes, 3 chars
    // é at grapheme 0 — the 'e' should toggle to 'E' (combining accent stays)
    let mut t = EditorTest::new("e\u{0301}x");
    t.assert_cursor(0, 0);
    t.keys("~");
    // 'e' → 'E', combining accent preserved, cursor advances to grapheme 1 ('x')
    assert_eq!(t.cursor(), (0, 1));
    let content = t.buffer_content();
    assert!(
        content.starts_with("E\u{0301}x") || content.starts_with("É"),
        "Expected toggled combining char, got: {:?}",
        content
    );
}

#[test]
fn test_tilde_count_with_emoji() {
    // "aB👨‍👩‍👧‍👦cD" — 5 graphemes
    // 3~ from pos 0 should toggle 'a','B',emoji → 'A','b',emoji
    let mut t = EditorTest::new("aB👨‍👩‍👧‍👦cD");
    t.assert_cursor(0, 0);
    t.keys("3~");
    // After 3~: cursor at grapheme 3 ('c')
    assert_eq!(t.cursor(), (0, 3));
    assert!(t.buffer_content().starts_with("Ab👨‍👩‍👧‍👦cD"));
}

// ===========================================================================
// Word under cursor with emoji
// ===========================================================================

#[test]
fn test_word_under_cursor_after_emoji() {
    // "👨‍👩‍👧‍👦hello world" — cursor on 'h' (grapheme 1)
    let mut t = EditorTest::new("👨‍👩‍👧‍👦hello world");
    t.set_cursor(0, 1); // grapheme 1 = 'h'
    let word = t.editor.buffer().word_under_cursor();
    assert!(word.is_some(), "Should find word 'hello'");
    let (w, _, _) = word.unwrap();
    assert_eq!(w, "hello");
}

#[test]
fn test_word_under_cursor_on_emoji() {
    // Emoji is not a word char — should return None
    let mut t = EditorTest::new("👨‍👩‍👧‍👦hello");
    t.set_cursor(0, 0); // cursor on emoji
    let word = t.editor.buffer().word_under_cursor();
    assert!(word.is_none(), "Emoji should not be a word character");
}

// ===========================================================================
// Join lines (J) with emoji
// ===========================================================================

#[test]
fn test_join_lines_cursor_after_emoji() {
    // Line 1: "a👨‍👩‍👧‍👦b" (3 graphemes)
    // Line 2: "cd"
    // After J: "a👨‍👩‍👧‍👦b cd" — cursor should be at grapheme 3 (the space/junction)
    let mut t = EditorTest::new("a👨‍👩‍👧‍👦b\ncd");
    t.assert_cursor(0, 0);
    t.keys("J");
    // Junction is at end of first line = grapheme 3
    assert_eq!(t.cursor(), (0, 3));
    assert!(t.buffer_content().starts_with("a👨‍👩‍👧‍👦b cd"));
}

// ===========================================================================
// Modify number (<C-a>/<C-x>) after emoji
// ===========================================================================

#[test]
fn test_increment_number_after_emoji() {
    // "👨‍👩‍👧‍👦42" — 3 graphemes: emoji, '4', '2'
    // Cursor at grapheme 1 ('4') — <C-a> should increment to 43
    let mut t = EditorTest::new("👨‍👩‍👧‍👦42");
    t.set_cursor(0, 1); // grapheme 1 = '4'
    t.keys("<C-a>");
    assert!(t.buffer_content().starts_with("👨‍👩‍👧‍👦43"));
    // Cursor should be on last digit (grapheme 2)
    assert_eq!(t.cursor(), (0, 2));
}

#[test]
fn test_decrement_number_after_emoji() {
    // "👨‍👩‍👧‍👦10" — <C-x> should decrement to 9
    let mut t = EditorTest::new("👨‍👩‍👧‍👦10");
    t.set_cursor(0, 1); // grapheme 1 = '1'
    t.keys("<C-x>");
    assert!(t.buffer_content().starts_with("👨‍👩‍👧‍👦9"));
}

// ===========================================================================
// Search with multi-codepoint graphemes
// ===========================================================================

#[test]
fn test_search_lands_on_correct_grapheme_after_emoji() {
    // "👨‍👩‍👧‍👦hello 👨‍👩‍👧‍👦world"
    // Search for "world" — should land on 'w' at the correct grapheme position
    let mut t = EditorTest::new("👨‍👩‍👧‍👦hello 👨‍👩‍👧‍👦world");
    t.keys("/world<Enter>");
    let (line, col) = t.cursor();
    assert_eq!(line, 0);
    // "👨‍👩‍👧‍👦hello 👨‍👩‍👧‍👦world" → graphemes: emoji,h,e,l,l,o,' ',emoji,w,o,r,l,d = 13
    // "world" starts at grapheme 8
    assert_eq!(col, 8, "Search should land on grapheme 8 ('w')");
}

#[test]
fn test_search_forward_from_emoji_position() {
    // Cursor on emoji, search forward for "b"
    let mut t = EditorTest::new("a👨‍👩‍👧‍👦bc");
    t.set_cursor(0, 1); // on emoji
    t.keys("/b<Enter>");
    assert_eq!(t.cursor(), (0, 2)); // 'b' is at grapheme 2
}

// ===========================================================================
// Case change with motion (gu/gU) across emoji
// ===========================================================================

#[test]
fn test_uppercase_word_after_emoji() {
    // "👨‍👩‍👧‍👦hello world" — gUw on 'hello' should uppercase just that word
    let mut t = EditorTest::new("👨‍👩‍👧‍👦hello world");
    t.set_cursor(0, 1); // 'h' at grapheme 1
    t.keys("gUw");
    assert!(
        t.buffer_content().starts_with("👨‍👩‍👧‍👦HELLO world"),
        "Expected uppercase 'HELLO', got: {:?}",
        t.buffer_content()
    );
}

// ===========================================================================
// Flag emoji (2 codepoints, 1 grapheme)
// ===========================================================================

#[test]
fn test_cursor_movement_across_flag_emoji() {
    // "a🇺🇸b" — 3 graphemes: 'a', flag, 'b'
    // 'l' from 'a' should land on flag (grapheme 1)
    let mut t = EditorTest::new("a🇺🇸b");
    t.assert_cursor(0, 0); // 'a'
    t.keys("l");
    assert_eq!(t.cursor(), (0, 1)); // flag
    t.keys("l");
    assert_eq!(t.cursor(), (0, 2)); // 'b'
}

#[test]
fn test_tilde_on_flag_emoji() {
    // Flag emoji has no case — ~ should advance past it
    let mut t = EditorTest::new("a🇺🇸b");
    t.set_cursor(0, 1); // on flag
    t.keys("~");
    assert_eq!(t.cursor(), (0, 2)); // advanced to 'b'
}

// ===========================================================================
// Skin tone emoji (2 codepoints, 1 grapheme)
// ===========================================================================

#[test]
fn test_movement_across_skin_tone_emoji() {
    // "x👋🏽y" — 3 graphemes
    let mut t = EditorTest::new("x👋🏽y");
    t.assert_cursor(0, 0);
    t.keys("l");
    assert_eq!(t.cursor(), (0, 1)); // wave emoji
    t.keys("l");
    assert_eq!(t.cursor(), (0, 2)); // 'y'
}

// ===========================================================================
// Regression: ASCII-only should be completely unaffected
// ===========================================================================

#[test]
fn test_ascii_operations_unchanged() {
    let mut t = EditorTest::new("hello world");
    t.keys("~");
    assert!(t.buffer_content().starts_with("Hello world"));
    assert_eq!(t.cursor(), (0, 1));
}

#[test]
fn test_ascii_search_unchanged() {
    let mut t = EditorTest::new("foo bar baz");
    t.keys("/bar<Enter>");
    assert_eq!(t.cursor(), (0, 4));
}

#[test]
fn test_ascii_join_unchanged() {
    let mut t = EditorTest::new("hello\nworld");
    t.keys("J");
    assert!(t.buffer_content().starts_with("hello world"));
    assert_eq!(t.cursor(), (0, 5));
}

#[test]
fn test_ascii_increment_unchanged() {
    let mut t = EditorTest::new("42");
    t.keys("<C-a>");
    assert!(t.buffer_content().starts_with("43"));
}
