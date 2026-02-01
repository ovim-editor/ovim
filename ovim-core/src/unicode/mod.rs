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
//! use ovim::unicode::grapheme_count;
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
}
