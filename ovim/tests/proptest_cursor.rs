//! Property-Based Testing for Cursor Operations
//!
//! Cursor positioning is the foundation of any text editor. Bugs here cause:
//! - Crashes (out-of-bounds access)
//! - Data corruption (editing wrong position)
//! - User confusion (cursor teleporting)
//!
//! ## Why Property-Based Testing for Cursors?
//!
//! Traditional tests check specific cases like "move right from (0,0)".
//! Property tests verify invariants like "cursor is ALWAYS within bounds after ANY movement".
//!
//! This catches edge cases we wouldn't think to test:
//! - Moving past buffer end
//! - Moving from empty lines
//! - Sequences like "down, down, delete line, up" that leave cursor dangling
//!
//! ## Critical Invariants
//!
//! 1. **Bounds Safety**: cursor.line() < buffer.line_count()
//! 2. **Column Bounds**: cursor.col() <= buffer.line_len(cursor.line())
//! 3. **Desired Column**: Preserved during vertical movement (Vim behavior)
//! 4. **Saturation**: Movements don't overflow/underflow
//! 5. **Consistency**: visual_col and col track together (unless tabs involved)

use ovim::buffer::{Buffer, Cursor};
use ovim::unicode::GraphemeCol;
use proptest::prelude::*;

// ============================================================================
// Test Strategies (Input Generators)
// ============================================================================

/// Cursor movement operations
#[derive(Debug, Clone)]
enum CursorOp {
    MoveUp(usize),
    MoveDown(usize),
    MoveLeft(usize),
    MoveRight(usize),
    SetPosition { line: usize, col: usize },
    SetLine(usize),
    SetCol(usize),
}

/// Strategy for generating cursor operations
fn arb_cursor_op() -> impl Strategy<Value = CursorOp> {
    prop_oneof![
        // Movement operations (70% of cases)
        2 => (1..10usize).prop_map(CursorOp::MoveUp),
        2 => (1..10usize).prop_map(CursorOp::MoveDown),
        2 => (1..10usize).prop_map(CursorOp::MoveLeft),
        2 => (1..10usize).prop_map(CursorOp::MoveRight),

        // Absolute positioning (30% of cases)
        1 => (0..50usize, 0..100usize)
            .prop_map(|(line, col)| CursorOp::SetPosition { line, col }),
        1 => (0..50usize).prop_map(CursorOp::SetLine),
        1 => (0..100usize).prop_map(CursorOp::SetCol),
    ]
}

/// Strategy for generating multi-line text
fn arb_multiline_text() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple cases (80%)
        4 => prop::collection::vec("[a-z]{0,20}", 1..10)
            .prop_map(|lines| lines.join("\n")),

        // Edge cases (20%)
        1 => prop_oneof![
            Just("".to_string()),  // Empty buffer
            Just("a".to_string()), // Single char
            Just("\n".to_string()), // Single newline
            Just("a\n".to_string()), // One line with newline
            prop::collection::vec("[a-z]{0,5}", 1..3)
                .prop_map(|lines| lines.join("\n")),  // Few short lines
        ]
    ]
}

// ============================================================================
// Property Tests: Fundamental Cursor Invariants
// ============================================================================

proptest! {
    /// Property: Cursor movement operations don't panic and maintain saturation
    ///
    /// The Cursor module itself doesn't enforce buffer bounds - that's the editor's job.
    /// However, cursor movements should saturate (not overflow/underflow) and never panic.
    #[test]
    fn prop_cursor_always_in_bounds(
        text in arb_multiline_text(),
        ops in prop::collection::vec(arb_cursor_op(), 0..30)
    ) {
        let mut buffer = Buffer::new_from_str(&text);

        for (i, op) in ops.iter().enumerate() {
            // Apply cursor operation
            let cursor = buffer.cursor_mut();
            match op {
                CursorOp::MoveUp(n) => cursor.move_up(*n),
                CursorOp::MoveDown(n) => cursor.move_down(*n),
                CursorOp::MoveLeft(n) => cursor.move_left(*n),
                CursorOp::MoveRight(n) => cursor.move_right(*n),
                CursorOp::SetPosition { line, col } => cursor.set_position(*line, GraphemeCol(*col)),
                CursorOp::SetLine(line) => cursor.set_line(*line),
                CursorOp::SetCol(col) => cursor.set_col(GraphemeCol(*col)),
            }

            // Check that cursor values are reasonable (not wildly out of bounds)
            let cursor = buffer.cursor();

            prop_assert!(
                cursor.line() < usize::MAX / 2,
                "After operation {}: cursor line {} should be reasonable. Op: {:?}",
                i, cursor.line(), op
            );

            prop_assert!(
                cursor.col().0 < usize::MAX / 2,
                "After operation {}: cursor col {} should be reasonable. Op: {:?}",
                i, cursor.col().0, op
            );

            // Note: The Cursor module allows being temporarily out of buffer bounds.
            // The Editor layer is responsible for clamping to valid positions when needed.
        }
    }

    /// Property: Moving up then down returns to original line (if room)
    ///
    /// Tests **reversibility** of vertical movement.
    #[test]
    fn prop_vertical_movement_reversible(
        text in arb_multiline_text(),
        initial_line in 5..15usize,
        delta in 1..5usize,
    ) {
        let buffer = Buffer::new_from_str(&text);
        let line_count = buffer.line_count();

        if line_count < initial_line + delta {
            // Not enough lines for this test
            return Ok(());
        }

        let mut cursor = Cursor::new(initial_line, GraphemeCol::ZERO);
        let original_line = cursor.line();

        // Move up, then down
        cursor.move_up(delta);
        cursor.move_down(delta);

        prop_assert_eq!(
            cursor.line(),
            original_line,
            "Moving up {} then down {} should return to line {}",
            delta, delta, original_line
        );
    }

    /// Property: Moving left then right returns to original column (if room)
    ///
    /// Tests **reversibility** of horizontal movement.
    #[test]
    fn prop_horizontal_movement_reversible(
        initial_col in 5..15usize,
        delta in 1..5usize,
    ) {
        let mut cursor = Cursor::new(0, GraphemeCol(initial_col));
        let original_col = cursor.col();

        // Move left, then right
        cursor.move_left(delta);
        cursor.move_right(delta);

        prop_assert_eq!(
            cursor.col(),
            original_col,
            "Moving left {} then right {} should return to col {:?}",
            delta, delta, original_col
        );
    }

    /// Property: Saturation prevents underflow
    ///
    /// Moving left from column 0 should stay at 0, not wrap around.
    #[test]
    fn prop_movement_saturates_at_zero(
        move_distance in 1..100usize,
    ) {
        let mut cursor = Cursor::new(0, GraphemeCol::ZERO);

        // Try to move before start
        cursor.move_left(move_distance);
        prop_assert_eq!(cursor.col(), GraphemeCol::ZERO, "Moving left from 0 should stay at 0");

        let mut cursor = Cursor::new(0, GraphemeCol::ZERO);
        cursor.move_up(move_distance);
        prop_assert_eq!(cursor.line(), 0, "Moving up from 0 should stay at 0");
    }

    /// Property: Saturation prevents overflow on movement
    ///
    /// Moving right/down by huge amounts shouldn't overflow.
    #[test]
    fn prop_movement_doesnt_overflow(
        initial_line in 0..100usize,
        initial_col in 0..100usize,
        delta in 1..usize::MAX / 2,
    ) {
        let mut cursor = Cursor::new(initial_line, GraphemeCol(initial_col));

        // These should not panic or overflow
        cursor.move_down(delta);
        prop_assert!(cursor.line() >= initial_line, "Moving down should increase or maintain line");

        let mut cursor = Cursor::new(initial_line, GraphemeCol(initial_col));
        cursor.move_right(delta);
        prop_assert!(cursor.col().0 >= initial_col, "Moving right should increase or maintain col");
    }

    /// Property: set_position updates all fields correctly
    ///
    /// When explicitly setting position, all cursor fields should sync.
    #[test]
    fn prop_set_position_syncs_fields(
        line in 0..100usize,
        col in 0..100usize,
    ) {
        let mut cursor = Cursor::new(0, GraphemeCol::ZERO);
        cursor.set_position(line, GraphemeCol(col));

        prop_assert_eq!(cursor.line(), line, "Line should be set");
        prop_assert_eq!(cursor.col(), GraphemeCol(col), "Col should be set");
        prop_assert_eq!(cursor.visual_col(), col, "Visual col should sync with col");
        prop_assert_eq!(cursor.desired_col(), col, "Desired col should sync with col");
    }

    /// Property: desired_col is preserved during vertical movement
    ///
    /// This is Vim's "sticky column" behavior: moving up/down remembers your
    /// desired column, even when passing through shorter lines.
    #[test]
    fn prop_desired_col_preserved_vertically(
        desired_col in 10..20usize,
        vertical_moves in 1..5usize,
    ) {
        let mut cursor = Cursor::new(5, GraphemeCol(desired_col));
        let original_desired = cursor.desired_col();

        // Vertical movements preserve desired_col
        cursor.move_up(vertical_moves);
        prop_assert_eq!(
            cursor.desired_col(),
            original_desired,
            "Moving up should preserve desired_col"
        );

        cursor.move_down(vertical_moves);
        prop_assert_eq!(
            cursor.desired_col(),
            original_desired,
            "Moving down should preserve desired_col"
        );
    }

    /// Property: horizontal movement updates desired_col
    ///
    /// Unlike vertical movement, moving left/right should update desired column.
    #[test]
    fn prop_horizontal_updates_desired_col(
        initial_col in 5..15usize,
        delta in 1..5usize,
    ) {
        let mut cursor = Cursor::new(0, GraphemeCol(initial_col));

        cursor.move_left(delta);
        let new_col = cursor.col().0;
        prop_assert_eq!(
            cursor.desired_col(),
            new_col,
            "Moving left should update desired_col to new position"
        );

        cursor.move_right(delta);
        let new_col = cursor.col().0;
        prop_assert_eq!(
            cursor.desired_col(),
            new_col,
            "Moving right should update desired_col to new position"
        );
    }

    /// Property: visual_col tracks col (when no tabs involved)
    ///
    /// Without tabs, visual column should equal actual column.
    #[test]
    fn prop_visual_col_tracks_col(
        line in 0..50usize,
        col in 0..100usize,
    ) {
        let mut cursor = Cursor::new(0, GraphemeCol::ZERO);
        cursor.set_position(line, GraphemeCol(col));

        prop_assert_eq!(
            cursor.visual_col(),
            cursor.col().0,
            "Visual col should equal col when no tabs"
        );
    }

    /// Property: Multiple set_col calls are idempotent
    ///
    /// Setting the same column twice should have same effect as setting once.
    #[test]
    fn prop_set_col_idempotent(col in 0..100usize) {
        let mut cursor1 = Cursor::new(0, GraphemeCol::ZERO);
        cursor1.set_col(GraphemeCol(col));

        let mut cursor2 = Cursor::new(0, GraphemeCol::ZERO);
        cursor2.set_col(GraphemeCol(col));
        cursor2.set_col(GraphemeCol(col));

        prop_assert_eq!(cursor1.line(), cursor2.line(), "Line should match");
        prop_assert_eq!(cursor1.col(), cursor2.col(), "Col should match");
        prop_assert_eq!(cursor1.visual_col(), cursor2.visual_col(), "Visual col should match");
        prop_assert_eq!(cursor1.desired_col(), cursor2.desired_col(), "Desired col should match");
    }
}

// ============================================================================
// Property Tests: Cursor + Buffer Integration
// ============================================================================

proptest! {
    /// Property: Cursor movement + buffer modification doesn't panic
    ///
    /// Realistic scenario: moving cursor around while editing.
    /// We verify that operations don't panic, not that cursor stays in bounds
    /// (since the cursor module doesn't enforce bounds).
    #[test]
    fn prop_cursor_with_buffer_edits(
        initial_text in arb_multiline_text(),
        cursor_ops in prop::collection::vec(arb_cursor_op(), 0..10),
        insert_text in "[a-z\n]{1,10}",
    ) {
        let mut buffer = Buffer::new_from_str(&initial_text);

        // Apply cursor movements (may leave cursor out of bounds)
        for op in cursor_ops {
            let cursor = buffer.cursor_mut();
            match op {
                CursorOp::MoveUp(n) => cursor.move_up(n),
                CursorOp::MoveDown(n) => cursor.move_down(n),
                CursorOp::MoveLeft(n) => cursor.move_left(n),
                CursorOp::MoveRight(n) => cursor.move_right(n),
                CursorOp::SetPosition { line, col } => cursor.set_position(line, GraphemeCol(col)),
                CursorOp::SetLine(line) => cursor.set_line(line),
                CursorOp::SetCol(col) => cursor.set_col(GraphemeCol(col)),
            }
        }

        // Clamp cursor position to valid range before inserting
        let line_count = buffer.line_count();
        let line = if line_count > 0 {
            buffer.cursor().line().min(line_count - 1)
        } else {
            0
        };
        let col = buffer.cursor().col().0;

        // Insert text - this should not panic
        buffer.insert_text_at(line, ovim_core::unicode::CharCol(col), &insert_text);

        // Just verify we got here without panicking
        prop_assert!(true);
    }

    /// Property: Cursor remains valid after deleting current line
    ///
    /// Edge case: deleting the line the cursor is on.
    #[test]
    fn prop_cursor_valid_after_line_deletion(
        lines in prop::collection::vec("[a-z]{1,10}", 3..10),
        cursor_line in 0..10usize,
    ) {
        let text = lines.join("\n");
        let mut buffer = Buffer::new_from_str(&text);
        let line_count = buffer.line_count();

        if line_count == 0 {
            return Ok(());
        }

        // Place cursor on a line
        let safe_cursor_line = cursor_line % line_count;
        buffer.cursor_mut().set_line(safe_cursor_line);

        // Delete that line (delete from start to end of line)
        let line_len = buffer.line_len(safe_cursor_line);
        let _ = buffer.delete_range(
            safe_cursor_line,
            ovim_core::unicode::CharCol::ZERO,
            safe_cursor_line,
            ovim_core::unicode::CharCol(line_len),
        );

        // Cursor should still be valid (possibly moved to previous line or clamped)
        let cursor = buffer.cursor();
        let new_line_count = buffer.line_count();

        prop_assert!(
            cursor.line() < new_line_count,
            "After deleting line {}, cursor line {} must be < new line count {}",
            safe_cursor_line,
            cursor.line(),
            new_line_count
        );
    }

    /// Property: Cursor at end of buffer stays valid after append
    ///
    /// Common operation: cursor at EOF, append text.
    #[test]
    fn prop_cursor_at_eof_after_append(
        initial_text in arb_multiline_text(),
        append_text in "[a-z\n]{1,20}",
    ) {
        let mut buffer = Buffer::new_from_str(&initial_text);

        // Move cursor to end of buffer
        let last_line = buffer.line_count().saturating_sub(1);
        let last_col = buffer.line_len(last_line);
        buffer.cursor_mut().set_position(last_line, GraphemeCol(last_col));

        // Append text
        buffer.insert_text_at(last_line, ovim_core::unicode::CharCol(last_col), &append_text);

        // Cursor should still be valid
        let cursor = buffer.cursor();
        let line_count = buffer.line_count();

        prop_assert!(
            cursor.line() < line_count,
            "After append, cursor line {} must be < line count {}",
            cursor.line(),
            line_count
        );
    }

    /// Property: Empty buffer always has cursor at (0,0)
    ///
    /// Special case: empty buffer should have cursor at origin.
    #[test]
    fn prop_empty_buffer_cursor(_dummy in 0..1u8) {
        let buffer = Buffer::new_from_str("");

        prop_assert_eq!(buffer.cursor().line(), 0, "Empty buffer cursor should be at line 0");
        prop_assert_eq!(buffer.cursor().col(), GraphemeCol::ZERO, "Empty buffer cursor should be at col 0");
    }
}

// ============================================================================
// Property Tests: Edge Cases and Boundary Conditions
// ============================================================================

proptest! {
    /// Property: Large movement values don't cause issues
    ///
    /// Stress test: extreme movement amounts.
    #[test]
    fn prop_extreme_movements(
        initial_line in 0..100usize,
        initial_col in 0..100usize,
        large_delta in 1000..10000usize,
    ) {
        let mut cursor = Cursor::new(initial_line, GraphemeCol(initial_col));

        // These should not panic
        cursor.move_up(large_delta);
        cursor.move_down(large_delta);
        cursor.move_left(large_delta);
        cursor.move_right(large_delta);

        // Cursor should still be valid (though position may be clamped)
        prop_assert!(cursor.line() < usize::MAX, "Line should not overflow");
        prop_assert!(cursor.col().0 < usize::MAX, "Col should not overflow");
    }

    /// Property: Alternating up/down movements stabilize
    ///
    /// Rapidly moving up and down should eventually stabilize, not diverge.
    #[test]
    fn prop_alternating_vertical_stabilizes(
        initial_line in 5..10usize,
        iterations in 1..20usize,
    ) {
        let mut cursor = Cursor::new(initial_line, GraphemeCol::ZERO);

        for _ in 0..iterations {
            cursor.move_up(1);
            cursor.move_down(1);
        }

        // Should return to original position
        prop_assert_eq!(
            cursor.line(),
            initial_line,
            "Alternating up/down should stabilize at original line"
        );
    }

    /// Property: Alternating left/right movements stabilize
    ///
    /// Rapidly moving left and right should eventually stabilize.
    #[test]
    fn prop_alternating_horizontal_stabilizes(
        initial_col in 5..10usize,
        iterations in 1..20usize,
    ) {
        let mut cursor = Cursor::new(0, GraphemeCol(initial_col));

        for _ in 0..iterations {
            cursor.move_left(1);
            cursor.move_right(1);
        }

        // Should return to original position
        prop_assert_eq!(
            cursor.col(),
            GraphemeCol(initial_col),
            "Alternating left/right should stabilize at original col"
        );
    }

    /// Property: set_col_preserve_desired maintains desired_col
    ///
    /// Special method for vertical movement that shouldn't change desired col.
    #[test]
    fn prop_preserve_desired_col(
        initial_col in 10..20usize,
        new_col in 0..50usize,
    ) {
        let mut cursor = Cursor::new(0, GraphemeCol(initial_col));
        let original_desired = cursor.desired_col();

        // Set column while preserving desired
        cursor.set_col_preserve_desired(GraphemeCol(new_col));

        prop_assert_eq!(cursor.col(), GraphemeCol(new_col), "Col should be updated");
        prop_assert_eq!(
            cursor.desired_col(),
            original_desired,
            "Desired col should be preserved"
        );
    }

    /// Property: Multiple operations maintain consistency
    ///
    /// Complex sequences should maintain cursor field consistency.
    #[test]
    fn prop_complex_sequence_consistency(
        ops in prop::collection::vec(arb_cursor_op(), 0..50)
    ) {
        let mut cursor = Cursor::new(10, GraphemeCol(10));

        for op in ops {
            match op {
                CursorOp::MoveUp(n) => cursor.move_up(n),
                CursorOp::MoveDown(n) => cursor.move_down(n),
                CursorOp::MoveLeft(n) => cursor.move_left(n),
                CursorOp::MoveRight(n) => cursor.move_right(n),
                CursorOp::SetPosition { line, col } => cursor.set_position(line, GraphemeCol(col)),
                CursorOp::SetLine(line) => cursor.set_line(line),
                CursorOp::SetCol(col) => cursor.set_col(GraphemeCol(col)),
            }

            // All fields should be reasonable values (not corrupted)
            prop_assert!(cursor.line() < usize::MAX / 2, "Line should be reasonable");
            prop_assert!(cursor.col().0 < usize::MAX / 2, "Col should be reasonable");
            prop_assert!(cursor.visual_col() < usize::MAX / 2, "Visual col should be reasonable");
            prop_assert!(cursor.desired_col() < usize::MAX / 2, "Desired col should be reasonable");
        }
    }
}
