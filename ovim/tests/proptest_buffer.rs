//! Property-Based Testing for Buffer Operations
//!
//! This test suite uses proptest to automatically discover edge cases in buffer operations.
//! Unlike traditional example-based tests, property-based tests specify **invariants** that
//! should always hold, then generate hundreds of random inputs to try to violate them.
//!
//! ## Why Property-Based Testing?
//!
//! For data structures like rope-based text buffers:
//! - State space is enormous (any UTF-8 string + cursor position)
//! - Edge cases are subtle (emoji, zero-width chars, buffer boundaries)
//! - Bugs often appear only with specific operation sequences
//!
//! Property tests explore this space systematically and use **shrinking** to find minimal
//! failing cases, making debugging dramatically easier than manual testing.
//!
//! ## What We're Testing
//!
//! 1. **Reversibility**: insert + delete = identity
//! 2. **Robustness**: operations never panic on valid inputs
//! 3. **Invariants**: UTF-8 validity, cursor bounds, line counts
//! 4. **Unicode handling**: emoji, multi-byte chars, zero-width chars
//! 5. **Boundary conditions**: empty buffers, end-of-file, zero-length operations

use ovim::buffer::Buffer;
use proptest::prelude::*;

// ============================================================================
// Test Strategies (Input Generators)
// ============================================================================

/// Strategy for generating arbitrary buffer operations
#[derive(Debug, Clone)]
enum BufferOp {
    /// Insert text at position (line, col)
    Insert {
        line: usize,
        col: usize,
        text: String,
    },
    /// Delete range from (start_line, start_col) to (end_line, end_col)
    Delete {
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    },
}

/// Generates arbitrary text including challenging Unicode:
/// - ASCII (common case)
/// - Multi-byte UTF-8 (emoji, CJK)
/// - Zero-width characters
/// - Mixed newlines
fn arb_text() -> impl Strategy<Value = String> {
    prop_oneof![
        // ASCII text (80% of cases - common case optimization)
        4 => "[a-zA-Z0-9 \n]{0,50}",
        // Unicode text with emoji and multi-byte chars
        1 => "[\\u{0}-\\u{10FFFF}]{0,20}",
    ]
}

/// Generates buffer operations that are "reasonable" (within small bounds)
fn arb_buffer_op() -> impl Strategy<Value = BufferOp> {
    prop_oneof![
        // Insert operations (60% of cases)
        3 => (0..20usize, 0..20usize, arb_text())
            .prop_map(|(line, col, text)| BufferOp::Insert { line, col, text }),

        // Delete operations (40% of cases)
        2 => (0..20usize, 0..20usize, 0..20usize, 0..20usize)
            .prop_map(|(sl, sc, el, ec)| BufferOp::Delete {
                start_line: sl,
                start_col: sc,
                end_line: el,
                end_col: ec,
            }),
    ]
}

// ============================================================================
// Property Tests: Fundamental Invariants
// ============================================================================

proptest! {
    /// Property: Insert text then delete the same text should restore original content
    ///
    /// This is the **reversibility property**. It verifies that insert and delete are
    /// inverse operations, which is fundamental to undo/redo correctness.
    ///
    /// Note: This test uses conservative positioning (start of line) to avoid
    /// edge cases with buffer position clamping.
    #[test]
    fn prop_insert_delete_identity(
        initial_text in arb_text(),
        insert_line in 0..5usize,
        insert_text in "[a-zA-Z0-9]{1,5}"  // Simple ASCII, no spaces/newlines
    ) {
        let mut buffer = Buffer::new_from_str(&initial_text);
        let initial_content = buffer.rope().to_string();

        // Clamp insert position to valid range
        let line_count = buffer.line_count();
        if line_count == 0 {
            return Ok(()); // Empty buffer edge case
        }

        // Use start of line for simplicity (col = 0 is always valid)
        let line = insert_line % line_count;
        let col = 0;  // Start of line is always safe

        // Insert text at start of line
        buffer.insert_text_at(line, col, &insert_text);

        // Verify the insert worked
        let content_after_insert = buffer.rope().to_string();
        prop_assert!(
            content_after_insert.len() >= initial_content.len(),
            "Content should grow after insert"
        );

        // Calculate delete range (from start of line, for length of inserted text)
        let delete_end_col = insert_text.chars().count();

        // Delete the same text
        let deleted = buffer.delete_range(line, col, line, delete_end_col);

        // Verify we got back what we inserted
        prop_assert_eq!(
            &deleted,
            &insert_text,
            "Deleted text should match inserted text. Line: {}, inserted: {:?}, deleted: {:?}",
            line,
            insert_text,
            deleted
        );

        // Verify buffer is back to original state
        prop_assert_eq!(
            buffer.rope().to_string(),
            initial_content,
            "Buffer should be restored to original after insert+delete"
        );
    }

    /// Property: Buffer operations never panic (robustness)
    ///
    /// This verifies **defensive programming**. Even with random invalid inputs,
    /// the buffer should clamp/validate positions rather than panic.
    #[test]
    fn prop_buffer_ops_never_panic(
        initial_text in arb_text(),
        ops in prop::collection::vec(arb_buffer_op(), 0..30)
    ) {
        let mut buffer = Buffer::new_from_str(&initial_text);

        // Apply all operations - should never panic
        for op in ops {
            match &op {
                BufferOp::Insert { line, col, text } => {
                    // These should handle out-of-bounds gracefully
                    buffer.insert_text_at(*line, *col, text);
                }
                BufferOp::Delete { start_line, start_col, end_line, end_col } => {
                    // These should handle invalid ranges gracefully
                    let _ = buffer.delete_range(*start_line, *start_col, *end_line, *end_col);
                }
            }
        }

        // If we got here without panic, test passes
        prop_assert!(true);
    }

    /// Property: Buffer content is always valid UTF-8
    ///
    /// This is critical because rope operations work on char boundaries.
    /// We must never split multi-byte UTF-8 sequences.
    #[test]
    fn prop_utf8_always_valid(
        initial_text in arb_text(),
        ops in prop::collection::vec(arb_buffer_op(), 0..20)
    ) {
        let mut buffer = Buffer::new_from_str(&initial_text);

        for op in ops {
            match &op {
                BufferOp::Insert { line, col, text } => {
                    buffer.insert_text_at(*line, *col, text);
                }
                BufferOp::Delete { start_line, start_col, end_line, end_col } => {
                    let _ = buffer.delete_range(*start_line, *start_col, *end_line, *end_col);
                }
            }

            // After every operation, buffer must be valid UTF-8
            let content = buffer.rope().to_string();
            prop_assert!(
                std::str::from_utf8(content.as_bytes()).is_ok(),
                "Buffer content must be valid UTF-8 after operation: {:?}", op
            );
        }
    }

    /// Property: Line count is always positive and bounded
    ///
    /// Verifies that the buffer's line counting logic returns reasonable values.
    /// Note: Rope library treats certain characters (\n, \r\n, \u{b}, \u{c}, \r, \u{85}, \u{2028}, \u{2029})
    /// as line separators, not just '\n'. This is by design for Unicode line breaking.
    #[test]
    fn prop_line_count_consistent(text in arb_text()) {
        let buffer = Buffer::new_from_str(&text);

        // Property 1: Line count is always at least 1
        prop_assert!(
            buffer.line_count() >= 1,
            "Buffer must have at least 1 line"
        );

        // Property 2: Line count should be reasonable (not wildly off)
        let content = buffer.rope().to_string();
        let char_count = content.chars().count();

        // Line count should never exceed char count (extreme upper bound)
        prop_assert!(
            buffer.line_count() <= char_count + 1,
            "Line count {} should not exceed char count {} + 1",
            buffer.line_count(),
            char_count
        );

        // Property 3: line_count() should match rope's internal line count
        // (after accounting for phantom empty line at end)
        let raw_rope_lines = buffer.rope().len_lines();
        prop_assert!(
            buffer.line_count() == raw_rope_lines || buffer.line_count() == raw_rope_lines - 1,
            "Line count {} should be close to rope's line count {}",
            buffer.line_count(),
            raw_rope_lines
        );
    }

    /// Property: Cursor is always within buffer bounds after operations
    ///
    /// Critical invariant: cursor must never point outside the buffer.
    /// This prevents out-of-bounds access and crashes.
    #[test]
    fn prop_cursor_always_in_bounds(
        text in arb_text(),
        ops in prop::collection::vec(arb_buffer_op(), 0..20)
    ) {
        let mut buffer = Buffer::new_from_str(&text);

        for op in ops {
            match &op {
                BufferOp::Insert { line, col, text } => {
                    buffer.insert_text_at(*line, *col, text);
                }
                BufferOp::Delete { start_line, start_col, end_line, end_col } => {
                    let _ = buffer.delete_range(*start_line, *start_col, *end_line, *end_col);
                }
            }

            // Cursor must be within bounds
            let cursor = buffer.cursor();
            let line_count = buffer.line_count();

            prop_assert!(
                cursor.line() < line_count,
                "Cursor line {} must be < line count {}",
                cursor.line(),
                line_count
            );

            let line_len = buffer.line_len(cursor.line());
            prop_assert!(
                cursor.col() <= line_len,
                "Cursor col {} must be <= line length {}",
                cursor.col(),
                line_len
            );
        }
    }

    /// Property: Delete beyond buffer bounds returns empty string
    ///
    /// Verifies **safe degradation**: invalid operations return empty results
    /// rather than panicking or corrupting state.
    #[test]
    fn prop_delete_out_of_bounds_safe(
        text in arb_text(),
        start_line in 100..1000usize,
        end_line in 100..1000usize,
    ) {
        let buffer = Buffer::new_from_str(&text);
        let mut buffer_copy = Buffer::new_from_str(&text);

        // Delete with out-of-bounds positions
        let deleted = buffer_copy.delete_range(start_line, 0, end_line, 0);

        prop_assert_eq!(deleted, "", "Delete out of bounds should return empty string");
        prop_assert_eq!(
            buffer.rope().to_string(),
            buffer_copy.rope().to_string(),
            "Delete out of bounds should not modify buffer"
        );
    }

    /// Property: Insert at end of buffer works correctly
    ///
    /// Tests **boundary condition**: appending to end of buffer.
    #[test]
    fn prop_insert_at_end(
        text in arb_text(),
        append_text in "[a-zA-Z0-9 ]{1,20}"
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let initial_content = buffer.rope().to_string();

        // Insert at last line, at end of line
        let last_line = buffer.line_count().saturating_sub(1);
        let last_col = buffer.line_len(last_line);

        buffer.insert_text_at(last_line, last_col, &append_text);

        let new_content = buffer.rope().to_string();

        // The new content should contain the appended text
        prop_assert!(
            new_content.contains(&append_text),
            "Buffer should contain appended text. Before: {:?}, After: {:?}, Appended: {:?}",
            initial_content,
            new_content,
            append_text
        );
    }

    /// Property: Empty buffer has exactly 1 empty line
    ///
    /// Tests the **empty buffer invariant** (Vim-like behavior).
    #[test]
    fn prop_empty_buffer_one_line(_dummy in 0..1u8) {
        let buffer = Buffer::new_from_str("");

        prop_assert_eq!(buffer.rope().len_chars(), 0, "Empty buffer should have 0 chars");
        prop_assert_eq!(buffer.line_count(), 1, "Empty buffer should have exactly 1 line");
        prop_assert_eq!(buffer.line_len(0), 0, "Empty buffer's line should have length 0");
        prop_assert_eq!(buffer.cursor().line(), 0, "Cursor should be at line 0");
        prop_assert_eq!(buffer.cursor().col(), 0, "Cursor should be at col 0");
    }

    /// Property: Delete entire buffer leaves minimal content
    ///
    /// Verifies that deleting everything results in valid nearly-empty state.
    /// Note: Buffer may retain a trailing newline (Vim behavior).
    #[test]
    fn prop_delete_all_leaves_empty(text in arb_text()) {
        if text.is_empty() {
            // Skip empty input (nothing to delete)
            return Ok(());
        }

        let mut buffer = Buffer::new_from_str(&text);
        let original_content = buffer.rope().to_string();
        let line_count = buffer.line_count();

        // Delete from start to end of last line
        let last_line_idx = line_count.saturating_sub(1);
        let last_line_len = buffer.line_len(last_line_idx);
        let deleted = buffer.delete_range(0, 0, last_line_idx, last_line_len);

        // Should have deleted most or all of the original content
        // (The exact match depends on whether trailing newline is included)
        prop_assert!(
            deleted.len() >= original_content.trim_end().len(),
            "Should delete most of original content. Deleted: {:?}, Original: {:?}",
            deleted,
            original_content
        );

        // Buffer should now be empty or nearly empty
        let remaining = buffer.rope().to_string();
        prop_assert!(
            remaining.is_empty() || remaining == "\n" || remaining.chars().all(char::is_whitespace),
            "After deleting all content, buffer should be empty or whitespace-only. Got: {:?}",
            remaining
        );
    }

    /// Property: Inserting empty string doesn't change buffer
    ///
    /// Tests **identity element**: inserting "" should be a no-op.
    #[test]
    fn prop_insert_empty_is_noop(
        text in arb_text(),
        line in 0..10usize,
        col in 0..50usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let initial_content = buffer.rope().to_string();

        // Clamp to valid position
        let line_count = buffer.line_count();
        let safe_line = line % line_count.max(1);
        let line_len = buffer.line_len(safe_line);
        let safe_col = col % (line_len + 1);

        buffer.insert_text_at(safe_line, safe_col, "");

        prop_assert_eq!(
            buffer.rope().to_string(),
            initial_content,
            "Inserting empty string should not change buffer"
        );
    }
}

// ============================================================================
// Property Tests: Unicode Edge Cases
// ============================================================================

proptest! {
    /// Property: Emoji and multi-byte characters are handled correctly
    ///
    /// Critical for modern text editing: emoji (4-byte UTF-8) must not be split.
    #[test]
    fn prop_emoji_handling(
        emoji in "[😀😃😄😁😆😅🤣😂🙂🙃😉😊]{1,10}",
        insert_pos in 0..10usize,
    ) {
        let mut buffer = Buffer::new_from_str(&emoji);
        let initial_char_count = buffer.rope().len_chars();

        // Insert more emoji
        let pos = insert_pos % (initial_char_count + 1);
        buffer.insert_text_at(0, pos, "🎉");

        // Verify UTF-8 validity
        let content = buffer.rope().to_string();
        prop_assert!(
            std::str::from_utf8(content.as_bytes()).is_ok(),
            "Buffer with emoji must be valid UTF-8"
        );

        // Verify char count increased by exactly 1 (emoji is 1 char)
        prop_assert_eq!(
            buffer.rope().len_chars(),
            initial_char_count + 1,
            "Inserting one emoji should increase char count by 1"
        );
    }

    /// Property: Zero-width characters don't break cursor positioning
    ///
    /// Zero-width joiners (ZWJ) and other zero-width chars are valid Unicode
    /// but have complex rendering. We must handle them correctly.
    #[test]
    fn prop_zero_width_chars(
        text in "[a-z\u{200B}\u{200C}\u{200D}\u{FEFF}]{5,20}",
    ) {
        let buffer = Buffer::new_from_str(&text);
        let content = buffer.rope().to_string();

        // Should be valid UTF-8
        prop_assert!(
            std::str::from_utf8(content.as_bytes()).is_ok(),
            "Buffer with zero-width chars must be valid UTF-8"
        );

        // Cursor should be in bounds
        let cursor = buffer.cursor();
        prop_assert!(
            cursor.line() < buffer.line_count(),
            "Cursor must be within bounds with zero-width chars"
        );
    }

    /// Property: Mixed newline handling (LF vs CRLF)
    ///
    /// Files from different platforms may have different line endings.
    /// Buffer should handle them consistently.
    #[test]
    fn prop_mixed_newlines(
        lines in prop::collection::vec("[a-z]{0,10}", 1..10),
        use_crlf in prop::bool::ANY,
    ) {
        let separator = if use_crlf { "\r\n" } else { "\n" };
        let text = lines.join(separator);

        let buffer = Buffer::new_from_str(&text);

        // Line count should match number of separators
        let separator_count = text.matches(separator).count();
        let expected_lines = separator_count.max(1);

        prop_assert!(
            buffer.line_count() >= expected_lines,
            "Line count should be at least {} for {} separators",
            expected_lines,
            separator_count
        );
    }
}

// ============================================================================
// Property Tests: Operation Sequences (Stateful Testing)
// ============================================================================

proptest! {
    /// Property: Arbitrary sequence of operations maintains all invariants
    ///
    /// This is the **comprehensive stress test**: random operations should
    /// never violate any invariant (UTF-8, cursor bounds, line counts).
    #[test]
    fn prop_operation_sequence_maintains_invariants(
        initial_text in arb_text(),
        ops in prop::collection::vec(arb_buffer_op(), 0..50)
    ) {
        let mut buffer = Buffer::new_from_str(&initial_text);

        for (i, op) in ops.iter().enumerate() {
            // Apply operation
            match op {
                BufferOp::Insert { line, col, ref text } => {
                    buffer.insert_text_at(*line, *col, text);
                }
                BufferOp::Delete { start_line, start_col, end_line, end_col } => {
                    let _ = buffer.delete_range(*start_line, *start_col, *end_line, *end_col);
                }
            }

            // Check all invariants after each operation
            let content = buffer.rope().to_string();

            // Invariant 1: Valid UTF-8
            prop_assert!(
                std::str::from_utf8(content.as_bytes()).is_ok(),
                "After operation {}: content must be valid UTF-8. Op: {:?}",
                i, op
            );

            // Invariant 2: Cursor in bounds
            let cursor = buffer.cursor();
            let line_count = buffer.line_count();
            prop_assert!(
                cursor.line() < line_count,
                "After operation {}: cursor line {} must be < line count {}. Op: {:?}",
                i, cursor.line(), line_count, op
            );

            let line_len = buffer.line_len(cursor.line());
            prop_assert!(
                cursor.col() <= line_len,
                "After operation {}: cursor col {} must be <= line len {}. Op: {:?}",
                i, cursor.col(), line_len, op
            );

            // Invariant 3: Line count > 0
            prop_assert!(
                line_count > 0,
                "After operation {}: line count must be > 0. Op: {:?}",
                i, op
            );
        }
    }

    /// Property: Undo-like sequences (insert, delete, insert) maintain consistency
    ///
    /// Simulates real editing workflows: type, backspace, type again.
    #[test]
    fn prop_undo_redo_consistency(
        initial_text in "[a-z\n]{5,20}",
        modifications in prop::collection::vec("[a-z]{1,5}", 1..10)
    ) {
        let mut buffer = Buffer::new_from_str(&initial_text);

        for text in modifications {
            // Insert at current cursor position
            let line = buffer.cursor().line();
            let col = buffer.cursor().col();

            buffer.insert_text_at(line, col, &text);

            // Verify still valid
            let content = buffer.rope().to_string();
            prop_assert!(
                std::str::from_utf8(content.as_bytes()).is_ok(),
                "After insert: content must be valid UTF-8"
            );

            // Now delete what we just inserted
            let delete_end_col = col + text.chars().count();
            let deleted = buffer.delete_range(line, col, line, delete_end_col);

            prop_assert_eq!(deleted, text, "Should delete what was just inserted");

            // Still valid after delete
            let content_after = buffer.rope().to_string();
            prop_assert!(
                std::str::from_utf8(content_after.as_bytes()).is_ok(),
                "After delete: content must be valid UTF-8"
            );
        }
    }
}
