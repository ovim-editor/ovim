//! Property-Based Testing for LSP Position Conversions
//!
//! ## The UTF-16 Problem
//!
//! LSP (Language Server Protocol) uses UTF-16 code units for positions, but Rust text
//! editors typically work with UTF-8 char indices. This mismatch causes bugs like:
//! - Jump to definition landing 1 char off with emoji in the line
//! - Hover tooltips appearing at wrong position
//! - Completions inserted at wrong location
//!
//! ## Why This Needs Property Testing
//!
//! Manual tests check "hello" and "😀" individually. Property tests check:
//! - ALL combinations of ASCII, emoji, CJK, zero-width chars
//! - Round-trip conversions (char → UTF-16 → char should be identity)
//! - Edge cases at line boundaries
//! - Mixed content (ASCII + emoji on same line)
//!
//! These bugs are CRITICAL - they break core LSP features and only appear with
//! specific Unicode characters that developers don't think to test.
//!
//! ## Properties We're Testing
//!
//! 1. **Round-trip Identity**: col → UTF-16 → col should return original
//! 2. **Monotonicity**: Larger char positions → larger UTF-16 positions
//! 3. **Bounds Safety**: Invalid inputs don't panic, return safe defaults
//! 4. **Unicode Correctness**: Emoji (4-byte), CJK (3-byte), zero-width all work
//! 5. **Line Boundary Handling**: Newlines excluded from position calculations

use ovim::buffer::Buffer;
use proptest::prelude::*;

// ============================================================================
// Standalone UTF-16 Conversion Functions (for testing)
// ============================================================================

/// Converts a character column position to UTF-16 code units
///
/// This mirrors the implementation in Editor::col_to_utf16 but is standalone for testing.
fn col_to_utf16(text: &str, col: usize) -> u32 {
    // Take characters up to col, excluding newline
    text.chars()
        .take_while(|&c| c != '\n')
        .take(col)
        .map(|c| c.len_utf16() as u32)
        .sum()
}

/// Converts UTF-16 code units back to character column position
///
/// This mirrors the implementation in Editor::utf16_to_col but is standalone for testing.
fn utf16_to_col(text: &str, utf16_col: u32) -> usize {
    let mut utf16_offset = 0u32;
    let mut char_position = 0usize;

    for ch in text.chars().take_while(|&c| c != '\n') {
        if utf16_offset >= utf16_col {
            break;
        }
        utf16_offset += ch.len_utf16() as u32;
        char_position += 1;
    }

    char_position
}

// ============================================================================
// Test Strategies (Input Generators)
// ============================================================================

/// Strategy for generating text with diverse Unicode content
fn arb_unicode_text() -> impl Strategy<Value = String> {
    prop_oneof![
        // Pure ASCII (50% - most common)
        5 => "[a-zA-Z0-9 ]{0,50}",

        // Emoji (20% - 4-byte UTF-8, 2 UTF-16 code units)
        2 => "[😀😃😄😁😆😅🤣😂🙂🙃😉😊😇🥰😍🤩😘😗]{0,20}",

        // CJK characters (15% - 3-byte UTF-8, 1 UTF-16 code unit)
        2 => "[\\u{4E00}-\\u{9FFF}]{0,20}",  // Chinese characters

        // Mixed content (10% - realistic text with emoji)
        1 => prop::collection::vec(
            prop_oneof![
                "[a-z]{1,10}",
                Just("😀".to_string()),
                Just("你好".to_string()),
            ],
            0..10
        ).prop_map(|parts| parts.join("")),

        // Zero-width characters (5% - edge case)
        1 => "[a-z\u{200B}\u{200C}\u{200D}\u{FEFF}]{5,15}",
    ]
}

/// Strategy for generating single lines (no newlines)
fn arb_single_line() -> impl Strategy<Value = String> {
    arb_unicode_text().prop_map(|s| s.replace('\n', ""))
}

// ============================================================================
// Property Tests: Round-Trip Identity
// ============================================================================

proptest! {
    /// Property: char position → UTF-16 → char position is identity
    ///
    /// This is THE most critical property. If round-trips don't preserve position,
    /// LSP features will be broken.
    #[test]
    fn prop_utf16_round_trip_identity(
        text in arb_single_line(),
        col in 0..50usize,
    ) {
        // Clamp col to valid range
        let char_count = text.chars().take_while(|&c| c != '\n').count();
        let safe_col = col.min(char_count);

        // Convert to UTF-16 and back
        let utf16_pos = col_to_utf16(&text, safe_col);
        let round_trip_col = utf16_to_col(&text, utf16_pos);

        prop_assert_eq!(
            round_trip_col,
            safe_col,
            "Round trip failed: col {} → UTF-16 {} → col {}. Text: {:?}",
            safe_col,
            utf16_pos,
            round_trip_col,
            text
        );
    }

    /// Property: UTF-16 position → char position → UTF-16 rounds to char boundary
    ///
    /// Reverse direction: starting from UTF-16 positions.
    /// Note: If UTF-16 position is in the middle of a surrogate pair (emoji), it will
    /// snap to the start of that character. This is expected behavior.
    #[test]
    fn prop_utf16_reverse_round_trip(
        text in arb_single_line(),
        utf16_col in 0..100u32,
    ) {
        // Clamp to valid UTF-16 range
        let max_utf16 = text.chars()
            .take_while(|&c| c != '\n')
            .map(|c| c.len_utf16() as u32)
            .sum::<u32>();
        let safe_utf16 = utf16_col.min(max_utf16);

        // Convert to char pos and back
        let char_col = utf16_to_col(&text, safe_utf16);
        let round_trip_utf16 = col_to_utf16(&text, char_col);

        // The round trip should either match exactly, or be rounded up to next char boundary
        // (if original UTF-16 position was in middle of surrogate pair, it rounds up to next char)
        prop_assert!(
            round_trip_utf16 >= safe_utf16,
            "Round trip UTF-16 {} should be >= original UTF-16 {} (rounds up to char boundary). Text: {:?}",
            round_trip_utf16,
            safe_utf16,
            text
        );

        // Should be within one character (max 1 UTF-16 unit difference for emoji)
        let diff = round_trip_utf16.saturating_sub(safe_utf16);
        prop_assert!(
            diff <= 1,
            "Round trip diff {} should be at most 1 (within one char). Original: {}, round trip: {}. Text: {:?}",
            diff,
            safe_utf16,
            round_trip_utf16,
            text
        );
    }
}

// ============================================================================
// Property Tests: Monotonicity
// ============================================================================

proptest! {
    /// Property: Larger char positions → larger UTF-16 positions
    ///
    /// Position conversion must be monotonic (preserves ordering).
    #[test]
    fn prop_utf16_monotonic(
        text in arb_single_line(),
        col1 in 0..30usize,
        col2 in 0..30usize,
    ) {
        let char_count = text.chars().take_while(|&c| c != '\n').count();
        let safe_col1 = col1.min(char_count);
        let safe_col2 = col2.min(char_count);

        let utf16_1 = col_to_utf16(&text, safe_col1);
        let utf16_2 = col_to_utf16(&text, safe_col2);

        if safe_col1 <= safe_col2 {
            prop_assert!(
                utf16_1 <= utf16_2,
                "Monotonicity violated: col {} → UTF-16 {}, but col {} → UTF-16 {}. Text: {:?}",
                safe_col1, utf16_1, safe_col2, utf16_2, text
            );
        }
    }

    /// Property: Adjacent char positions have UTF-16 diff matching char's UTF-16 length
    ///
    /// The UTF-16 difference between adjacent positions equals the character's UTF-16 size.
    #[test]
    fn prop_utf16_adjacent_diff(
        text in arb_single_line(),
        col in 0..30usize,
    ) {
        let chars: Vec<char> = text.chars().take_while(|&c| c != '\n').collect();

        if col >= chars.len() {
            return Ok(());
        }

        let utf16_at_col = col_to_utf16(&text, col);
        let utf16_at_next = col_to_utf16(&text, col + 1);

        let expected_diff = chars[col].len_utf16() as u32;
        let actual_diff = utf16_at_next - utf16_at_col;

        prop_assert_eq!(
            actual_diff,
            expected_diff,
            "UTF-16 diff should equal char's UTF-16 length. Char: {:?}, Expected: {}, Got: {}",
            chars[col],
            expected_diff,
            actual_diff
        );
    }
}

// ============================================================================
// Property Tests: Unicode Edge Cases
// ============================================================================

proptest! {
    /// Property: Emoji (4-byte UTF-8, 2 UTF-16 code units) handled correctly
    ///
    /// Emoji are the most common source of LSP positioning bugs.
    #[test]
    fn prop_emoji_utf16_conversion(
        emoji_count in 1..10usize,
        col in 0..10usize,
    ) {
        // Generate string with only emoji
        let emoji_text: String = "😀".repeat(emoji_count);
        let safe_col = col.min(emoji_count);

        let utf16_pos = col_to_utf16(&emoji_text, safe_col);

        // Each emoji is 2 UTF-16 code units (surrogate pair)
        let expected_utf16 = (safe_col * 2) as u32;

        prop_assert_eq!(
            utf16_pos,
            expected_utf16,
            "Emoji UTF-16 conversion failed: {} emoji, col {} should be UTF-16 {}",
            emoji_count,
            safe_col,
            expected_utf16
        );

        // Round trip should work
        let round_trip = utf16_to_col(&emoji_text, utf16_pos);
        prop_assert_eq!(round_trip, safe_col, "Emoji round trip failed");
    }

    /// Property: CJK characters (3-byte UTF-8, 1 UTF-16 code unit) handled correctly
    ///
    /// CJK characters are 1 UTF-16 code unit, same as ASCII.
    #[test]
    fn prop_cjk_utf16_conversion(
        cjk_text in "[\\u{4E00}-\\u{4E10}]{1,20}",
        col in 0..20usize,
    ) {
        let char_count = cjk_text.chars().count();
        let safe_col = col.min(char_count);

        let utf16_pos = col_to_utf16(&cjk_text, safe_col);

        // CJK characters are 1 UTF-16 code unit each (same as char count)
        let expected_utf16 = safe_col as u32;

        prop_assert_eq!(
            utf16_pos,
            expected_utf16,
            "CJK UTF-16 conversion: {} chars, col {} should be UTF-16 {}",
            char_count,
            safe_col,
            expected_utf16
        );
    }

    /// Property: Mixed ASCII + emoji works correctly
    ///
    /// Realistic case: code with emoji in comments or strings.
    #[test]
    fn prop_mixed_ascii_emoji(
        prefix in "[a-z]{0,10}",
        emoji in "[😀😃😄]{0,5}",
        suffix in "[a-z]{0,10}",
        col in 0..30usize,
    ) {
        let text = format!("{}{}{}", prefix, emoji, suffix);
        let char_count = text.chars().count();
        let safe_col = col.min(char_count);

        // Convert and round-trip
        let utf16_pos = col_to_utf16(&text, safe_col);
        let round_trip = utf16_to_col(&text, utf16_pos);

        prop_assert_eq!(
            round_trip,
            safe_col,
            "Mixed ASCII+emoji round trip failed at col {}. Text: {:?}",
            safe_col,
            text
        );
    }

    /// Property: Zero-width characters handled (count as 1 char, 1 UTF-16 unit)
    ///
    /// Zero-width joiners and similar don't render but are valid characters.
    #[test]
    fn prop_zero_width_chars(
        text in "[a-z\u{200B}\u{200C}\u{200D}]{5,15}",
        col in 0..15usize,
    ) {
        let char_count = text.chars().count();
        let safe_col = col.min(char_count);

        // Should not panic or produce invalid results
        let utf16_pos = col_to_utf16(&text, safe_col);
        let round_trip = utf16_to_col(&text, utf16_pos);

        prop_assert_eq!(
            round_trip,
            safe_col,
            "Zero-width char round trip failed at col {}",
            safe_col
        );
    }
}

// ============================================================================
// Property Tests: Boundary Conditions
// ============================================================================

proptest! {
    /// Property: Position at end of line (excluding newline) works
    ///
    /// LSP positions should NOT include the newline character.
    #[test]
    fn prop_end_of_line_position(text in arb_single_line()) {
        let char_count = text.chars().take_while(|&c| c != '\n').count();

        // Position at end of line (excluding newline)
        let utf16_pos = col_to_utf16(&text, char_count);
        let round_trip = utf16_to_col(&text, utf16_pos);

        prop_assert_eq!(
            round_trip,
            char_count,
            "End-of-line position failed. Text: {:?}, chars: {}, UTF-16: {}",
            text,
            char_count,
            utf16_pos
        );
    }

    /// Property: Position 0 always converts to UTF-16 position 0
    ///
    /// Start of line should always be 0.
    #[test]
    fn prop_start_of_line_is_zero(text in arb_single_line()) {
        let utf16_pos = col_to_utf16(&text, 0);
        prop_assert_eq!(utf16_pos, 0, "Start of line should be UTF-16 position 0");

        let char_pos = utf16_to_col(&text, 0);
        prop_assert_eq!(char_pos, 0, "UTF-16 position 0 should be char position 0");
    }

    /// Property: Empty line has position 0
    ///
    /// Edge case: empty lines.
    #[test]
    fn prop_empty_line(_dummy in 0..1u8) {
        let text = "";

        let utf16_pos = col_to_utf16(text, 0);
        prop_assert_eq!(utf16_pos, 0, "Empty line should have UTF-16 position 0");

        let char_pos = utf16_to_col(text, 0);
        prop_assert_eq!(char_pos, 0, "Empty line should have char position 0");
    }

    /// Property: Line with only newline has position 0
    ///
    /// Lines with just '\n' should have no valid positions (newline excluded).
    #[test]
    fn prop_newline_only_line(_dummy in 0..1u8) {
        let text = "\n";

        // Position 0 is valid (before newline)
        let utf16_pos = col_to_utf16(text, 0);
        prop_assert_eq!(utf16_pos, 0, "Newline-only line should have UTF-16 position 0");

        // Position beyond newline should be clamped
        let utf16_pos = col_to_utf16(text, 1);
        prop_assert_eq!(utf16_pos, 0, "Position past newline should be 0 (newline excluded)");
    }

    /// Property: Out-of-bounds positions don't panic
    ///
    /// Invalid positions should be handled gracefully.
    #[test]
    fn prop_out_of_bounds_safe(
        text in arb_single_line(),
        large_col in 100..1000usize,
        large_utf16 in 100..1000u32,
    ) {
        // These should not panic, just clamp/saturate
        let _utf16_pos = col_to_utf16(&text, large_col);
        let _char_pos = utf16_to_col(&text, large_utf16);

        prop_assert!(true); // If we got here, no panic occurred
    }
}

// ============================================================================
// Property Tests: Buffer Integration
// ============================================================================

proptest! {
    /// Property: Multi-line buffer position conversions work per-line
    ///
    /// Each line should have independent position conversion.
    #[test]
    fn prop_multiline_buffer_positions(
        lines in prop::collection::vec(arb_single_line(), 1..10),
        line_idx in 0..10usize,
        col in 0..30usize,
    ) {
        let text = lines.join("\n");
        let buffer = Buffer::new_from_str(&text);

        if line_idx >= buffer.line_count() {
            return Ok(());
        }

        // Get the specific line
        let line_text = buffer.line(line_idx).unwrap_or_default();
        let char_count = line_text.chars().take_while(|&c| c != '\n').count();
        let safe_col = col.min(char_count);

        // Convert on this line
        let utf16_pos = col_to_utf16(&line_text, safe_col);
        let round_trip = utf16_to_col(&line_text, utf16_pos);

        prop_assert_eq!(
            round_trip,
            safe_col,
            "Multi-line buffer round trip failed at line {}, col {}",
            line_idx,
            safe_col
        );
    }

    /// Property: Buffer with mixed Unicode lines
    ///
    /// Realistic buffer with different content per line.
    #[test]
    fn prop_mixed_unicode_buffer(
        ascii_line in "[a-z ]{10,20}",
        emoji_line in "[😀😃😄]{5,10}",
        cjk_line in "[\\u{4E00}-\\u{4E10}]{5,10}",
        col in 0..15usize,
    ) {
        let lines = vec![ascii_line.clone(), emoji_line.clone(), cjk_line.clone()];

        for (idx, line) in lines.iter().enumerate() {
            let char_count = line.chars().count();
            let safe_col = col.min(char_count);

            let utf16_pos = col_to_utf16(line, safe_col);
            let round_trip = utf16_to_col(line, utf16_pos);

            prop_assert_eq!(
                round_trip,
                safe_col,
                "Mixed buffer line {} round trip failed. Line: {:?}",
                idx,
                line
            );
        }
    }
}

// ============================================================================
// Property Tests: Known Edge Cases (Regression Prevention)
// ============================================================================

proptest! {
    /// Property: Consecutive emoji don't cause off-by-one errors
    ///
    /// Common bug: miscounting emoji as 1 UTF-16 unit instead of 2.
    #[test]
    fn prop_consecutive_emoji_no_off_by_one(
        emoji_count in 2..10usize,
    ) {
        let text: String = "😀".repeat(emoji_count);

        // Check each position
        for col in 0..=emoji_count {
            let utf16_pos = col_to_utf16(&text, col);
            let expected_utf16 = (col * 2) as u32;

            prop_assert_eq!(
                utf16_pos,
                expected_utf16,
                "Consecutive emoji off-by-one at position {}",
                col
            );
        }
    }

    /// Property: ASCII followed by single emoji at specific positions
    ///
    /// Catches bugs where conversion is correct until first multi-unit char.
    #[test]
    fn prop_ascii_then_emoji(
        ascii_prefix_len in 1..20usize,
    ) {
        let text = format!("{}😀", "a".repeat(ascii_prefix_len));

        // At the emoji position
        let emoji_col = ascii_prefix_len;
        let utf16_pos = col_to_utf16(&text, emoji_col);

        // Should be: ascii_prefix_len (1 UTF-16 per char)
        prop_assert_eq!(
            utf16_pos,
            ascii_prefix_len as u32,
            "ASCII+emoji boundary failed at emoji position"
        );

        // Past the emoji
        let after_emoji_col = ascii_prefix_len + 1;
        let utf16_after = col_to_utf16(&text, after_emoji_col);

        // Should be: ascii_prefix_len + 2 (emoji is 2 UTF-16 units)
        let expected = (ascii_prefix_len + 2) as u32;
        prop_assert_eq!(
            utf16_after,
            expected,
            "ASCII+emoji failed after emoji position"
        );
    }
}
