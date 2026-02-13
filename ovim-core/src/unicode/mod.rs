//! Unicode helpers for proper grapheme cluster handling
//!
//! This module provides utilities for working with Unicode text in a grapheme-aware way.
//! A grapheme cluster is what a user perceives as a single character, even if it's composed
//! of multiple Unicode code points (e.g., emojis like 👨‍👩‍👧‍👦, flags like 🇺🇸, or accented characters like é).
//!
//! # Key Functions
//!
//! - `grapheme_count(s)` - Count grapheme clusters (user-perceived characters)
//! - `grapheme_indices(s)` - Iterate over grapheme clusters with byte positions
//! - `grapheme_at_index(s, n)` - Get the nth grapheme cluster
//! - `byte_offset_for_grapheme(s, n)` - Get byte offset for nth grapheme
//!
//! # Example
//!
//! ```
//! use ovim_core::unicode::grapheme_count;
//!
//! // Family emoji (7 code points, 1 grapheme)
//! assert_eq!(grapheme_count("👨‍👩‍👧‍👦"), 1);
//!
//! // Regular ASCII
//! assert_eq!(grapheme_count("hello"), 5);
//!
//! // Mixed content
//! assert_eq!(grapheme_count("a👨‍👩‍👧‍👦b"), 3);
//! ```

use unicode_segmentation::UnicodeSegmentation;

/// Count the number of grapheme clusters in a string
///
/// This is the correct way to count "characters" as users perceive them.
/// Use this instead of `str::chars().count()` for cursor movement and display.
#[inline]
pub fn grapheme_count(s: &str) -> usize {
    s.graphemes(true).count()
}

/// Get an iterator over grapheme clusters with their byte offsets
///
/// Returns `(byte_offset, grapheme_str)` pairs.
/// Useful for mapping between grapheme index and byte offset.
#[inline]
pub fn grapheme_indices(s: &str) -> impl Iterator<Item = (usize, &str)> {
    s.grapheme_indices(true)
}

/// Get the nth grapheme cluster from a string
///
/// Returns `None` if the index is out of bounds.
#[inline]
pub fn grapheme_at_index(s: &str, index: usize) -> Option<&str> {
    s.graphemes(true).nth(index)
}

/// Get the byte offset for the start of the nth grapheme cluster
///
/// Returns `None` if the index is out of bounds.
/// Returns `Some(s.len())` if index equals the grapheme count (end of string).
#[inline]
pub fn byte_offset_for_grapheme(s: &str, grapheme_index: usize) -> Option<usize> {
    if grapheme_index == 0 {
        return Some(0);
    }
    let mut count = 0;
    for (byte_offset, _) in s.grapheme_indices(true) {
        if count == grapheme_index {
            return Some(byte_offset);
        }
        count += 1;
    }
    // Handle end-of-string case
    if count == grapheme_index {
        return Some(s.len());
    }
    None
}

/// Get the byte range for the nth grapheme cluster
///
/// Returns `(start_byte, end_byte)` or `None` if out of bounds.
#[inline]
pub fn byte_range_for_grapheme(s: &str, grapheme_index: usize) -> Option<(usize, usize)> {
    let iter = s.grapheme_indices(true);

    for (count, (start, grapheme)) in iter.enumerate() {
        if count == grapheme_index {
            return Some((start, start + grapheme.len()));
        }
    }
    None
}

/// Convert a byte offset to a grapheme index
///
/// If the byte offset lands in the middle of a grapheme, returns the grapheme it's part of.
#[inline]
pub fn grapheme_index_for_byte(s: &str, byte_offset: usize) -> usize {
    s.grapheme_indices(true)
        .take_while(|(offset, _)| *offset < byte_offset)
        .count()
}

/// Truncate string to at most `max_graphemes` grapheme clusters
///
/// Returns a string slice containing at most `max_graphemes` grapheme clusters.
#[inline]
pub fn truncate_graphemes(s: &str, max_graphemes: usize) -> &str {
    match byte_offset_for_grapheme(s, max_graphemes) {
        Some(end) => &s[..end],
        None => s, // String has fewer graphemes than max
    }
}

/// Convert a grapheme cluster index to a char (Unicode scalar value) index.
///
/// The cursor stores positions as grapheme indices (what users perceive as characters),
/// but ropey's rope operations use char indices (Unicode scalar values / code points).
/// A single grapheme like 👨‍👩‍👧‍👦 is 1 grapheme but 7 chars.
///
/// Use this at the boundary where a cursor position (grapheme) must be passed to
/// rope operations like `insert_text_at` or `delete_range` (which expect char indices).
///
/// Returns the total char count if `grapheme_col` is past the end.
///
/// # Example
/// ```
/// use ovim_core::unicode::grapheme_to_char_col;
///
/// // ASCII: 1 grapheme = 1 char, so indices match
/// assert_eq!(grapheme_to_char_col("hello", 2), 2);
///
/// // "a👨‍👩‍👧‍👦b": grapheme 0='a'(1 char), grapheme 1=emoji(7 chars), grapheme 2='b'(1 char)
/// assert_eq!(grapheme_to_char_col("a👨‍👩‍👧‍👦b", 0), 0);
/// assert_eq!(grapheme_to_char_col("a👨‍👩‍👧‍👦b", 1), 1);
/// assert_eq!(grapheme_to_char_col("a👨‍👩‍👧‍👦b", 2), 8);
/// ```
#[inline]
pub fn grapheme_to_char_col(s: &str, grapheme_col: usize) -> usize {
    let mut char_offset = 0;
    for (i, grapheme) in s.graphemes(true).enumerate() {
        if i == grapheme_col {
            return char_offset;
        }
        char_offset += grapheme.chars().count();
    }
    char_offset // past-the-end
}

/// Convert a char (Unicode scalar value) index to a grapheme cluster index.
///
/// Rope operations return char-based positions, but cursor positions are stored
/// as grapheme indices. Use this to convert rope results into cursor-compatible values.
///
/// If `char_col` falls in the middle of a multi-char grapheme (e.g., between the
/// code points of 👨‍👩‍👧‍👦), returns the index of that grapheme.
///
/// Returns the total grapheme count if `char_col` is past the end.
///
/// # Example
/// ```
/// use ovim_core::unicode::char_to_grapheme_col;
///
/// // ASCII: identity
/// assert_eq!(char_to_grapheme_col("hello", 2), 2);
///
/// // "a👨‍👩‍👧‍👦b": char 0='a', chars 1-7=emoji, char 8='b'
/// assert_eq!(char_to_grapheme_col("a👨‍👩‍👧‍👦b", 0), 0);
/// assert_eq!(char_to_grapheme_col("a👨‍👩‍👧‍👦b", 1), 1);
/// assert_eq!(char_to_grapheme_col("a👨‍👩‍👧‍👦b", 8), 2);
/// ```
#[inline]
pub fn char_to_grapheme_col(s: &str, char_col: usize) -> usize {
    let mut chars_seen = 0;
    for (i, grapheme) in s.graphemes(true).enumerate() {
        let grapheme_chars = grapheme.chars().count();
        if char_col < chars_seen + grapheme_chars {
            return i;
        }
        chars_seen += grapheme_chars;
    }
    // Past the end: return total grapheme count
    s.graphemes(true).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grapheme_count_ascii() {
        assert_eq!(grapheme_count("hello"), 5);
        assert_eq!(grapheme_count(""), 0);
        assert_eq!(grapheme_count("a"), 1);
    }

    #[test]
    fn test_grapheme_count_emoji() {
        // Single emoji
        assert_eq!(grapheme_count("👍"), 1);
        // Family emoji (ZWJ sequence: 7 code points, 1 grapheme)
        assert_eq!(grapheme_count("👨‍👩‍👧‍👦"), 1);
        // Flag emoji (2 code points, 1 grapheme)
        assert_eq!(grapheme_count("🇺🇸"), 1);
        // Skin tone modifier (2 code points, 1 grapheme)
        assert_eq!(grapheme_count("👋🏽"), 1);
    }

    #[test]
    fn test_grapheme_count_mixed() {
        assert_eq!(grapheme_count("a👨‍👩‍👧‍👦b"), 3);
        assert_eq!(grapheme_count("Hello 🌍!"), 8);
    }

    #[test]
    fn test_grapheme_count_combining() {
        // e with combining acute accent (2 code points, 1 grapheme)
        assert_eq!(grapheme_count("e\u{0301}"), 1);
        // Precomposed é (1 code point, 1 grapheme)
        assert_eq!(grapheme_count("é"), 1);
    }

    #[test]
    fn test_byte_offset_for_grapheme() {
        let s = "a👨‍👩‍👧‍👦b";
        assert_eq!(byte_offset_for_grapheme(s, 0), Some(0)); // 'a'
        assert_eq!(byte_offset_for_grapheme(s, 1), Some(1)); // emoji starts at byte 1
        assert_eq!(byte_offset_for_grapheme(s, 2), Some(26)); // 'b' starts after emoji
        assert_eq!(byte_offset_for_grapheme(s, 3), Some(27)); // end of string
        assert_eq!(byte_offset_for_grapheme(s, 4), None); // out of bounds
    }

    #[test]
    fn test_grapheme_at_index() {
        let s = "a👨‍👩‍👧‍👦b";
        assert_eq!(grapheme_at_index(s, 0), Some("a"));
        assert_eq!(grapheme_at_index(s, 1), Some("👨‍👩‍👧‍👦"));
        assert_eq!(grapheme_at_index(s, 2), Some("b"));
        assert_eq!(grapheme_at_index(s, 3), None);
    }

    #[test]
    fn test_truncate_graphemes() {
        let s = "a👨‍👩‍👧‍👦b";
        assert_eq!(truncate_graphemes(s, 1), "a");
        assert_eq!(truncate_graphemes(s, 2), "a👨‍👩‍👧‍👦");
        assert_eq!(truncate_graphemes(s, 3), "a👨‍👩‍👧‍👦b");
        assert_eq!(truncate_graphemes(s, 10), "a👨‍👩‍👧‍👦b");
    }

    #[test]
    fn test_grapheme_to_char_col_ascii() {
        // For ASCII, grapheme index == char index
        assert_eq!(grapheme_to_char_col("hello", 0), 0);
        assert_eq!(grapheme_to_char_col("hello", 2), 2);
        assert_eq!(grapheme_to_char_col("hello", 5), 5); // past-the-end
        assert_eq!(grapheme_to_char_col("", 0), 0);
    }

    #[test]
    fn test_grapheme_to_char_col_emoji() {
        // "a👨‍👩‍👧‍👦b": 3 graphemes, 9 chars (a=1, family_emoji=7, b=1)
        let s = "a👨‍👩‍👧‍👦b";
        assert_eq!(grapheme_to_char_col(s, 0), 0); // 'a' at char 0
        assert_eq!(grapheme_to_char_col(s, 1), 1); // emoji at char 1
        assert_eq!(grapheme_to_char_col(s, 2), 8); // 'b' at char 8 (1 + 7)
        assert_eq!(grapheme_to_char_col(s, 3), 9); // past-the-end
    }

    #[test]
    fn test_grapheme_to_char_col_combining() {
        // "e\u{0301}x" = é + x: 2 graphemes, 3 chars
        let s = "e\u{0301}x";
        assert_eq!(grapheme_to_char_col(s, 0), 0); // é at char 0
        assert_eq!(grapheme_to_char_col(s, 1), 2); // 'x' at char 2 (e + combining = 2 chars)
        assert_eq!(grapheme_to_char_col(s, 2), 3); // past-the-end
    }

    #[test]
    fn test_grapheme_to_char_col_flags() {
        // "🇺🇸x": flag=1 grapheme (2 chars), x=1 grapheme (1 char)
        let s = "🇺🇸x";
        assert_eq!(grapheme_to_char_col(s, 0), 0); // flag at char 0
        assert_eq!(grapheme_to_char_col(s, 1), 2); // 'x' at char 2
    }

    #[test]
    fn test_char_to_grapheme_col_ascii() {
        assert_eq!(char_to_grapheme_col("hello", 0), 0);
        assert_eq!(char_to_grapheme_col("hello", 2), 2);
        assert_eq!(char_to_grapheme_col("hello", 5), 5); // past-the-end
        assert_eq!(char_to_grapheme_col("", 0), 0);
    }

    #[test]
    fn test_char_to_grapheme_col_emoji() {
        // "a👨‍👩‍👧‍👦b": chars 0='a', 1-7=emoji, 8='b'
        let s = "a👨‍👩‍👧‍👦b";
        assert_eq!(char_to_grapheme_col(s, 0), 0); // char 0 = grapheme 0 ('a')
        assert_eq!(char_to_grapheme_col(s, 1), 1); // char 1 = grapheme 1 (emoji start)
        assert_eq!(char_to_grapheme_col(s, 4), 1); // char 4 = still inside emoji = grapheme 1
        assert_eq!(char_to_grapheme_col(s, 7), 1); // char 7 = still inside emoji = grapheme 1
        assert_eq!(char_to_grapheme_col(s, 8), 2); // char 8 = grapheme 2 ('b')
        assert_eq!(char_to_grapheme_col(s, 9), 3); // past-the-end
    }

    #[test]
    fn test_char_to_grapheme_col_combining() {
        // "e\u{0301}x": chars 0='e', 1=combining, 2='x'
        let s = "e\u{0301}x";
        assert_eq!(char_to_grapheme_col(s, 0), 0); // 'e' = grapheme 0 (é)
        assert_eq!(char_to_grapheme_col(s, 1), 0); // combining accent = still grapheme 0
        assert_eq!(char_to_grapheme_col(s, 2), 1); // 'x' = grapheme 1
    }

    #[test]
    fn test_roundtrip_grapheme_char_col() {
        // Roundtrip: grapheme → char → grapheme should be identity
        let s = "Hello 👨‍👩‍👧‍👦 world 🇺🇸!";
        let grapheme_len = grapheme_count(s);
        for g in 0..grapheme_len {
            let char_col = grapheme_to_char_col(s, g);
            let back = char_to_grapheme_col(s, char_col);
            assert_eq!(back, g, "roundtrip failed for grapheme {g}: char_col={char_col}");
        }
    }
}
