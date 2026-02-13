//! Property-Based Testing for Motions
//!
//! Motions are the highest-risk code in a Vim editor. A bug in `w`/`b`/`e`
//! doesn't just move the cursor wrong — it corrupts the range that `dw` deletes
//! and `cw` changes. Property-based testing catches edge cases that hand-written
//! tests miss: unusual whitespace runs, CJK mixed with ASCII, empty lines, etc.
//!
//! ## Critical Invariants
//!
//! 1. **Bounds safety**: After any motion, cursor line < line_count and col <= line_len
//! 2. **Forward monotonicity**: `w`/`e` move forward (or stay at buffer end)
//! 3. **Backward monotonicity**: `b`/`ge` move backward (or stay at buffer start)
//! 4. **Count decomposition**: `3w` = `w` + `w` + `w`
//! 5. **Boundary idempotency**: At buffer start, `b` is no-op. At buffer end, `w` is no-op-ish.
//! 6. **No panics**: Any motion on any buffer with any cursor position doesn't crash
//! 7. **Bracket symmetry**: `%` from `(` to `)` then `%` back returns to original

use ovim::buffer::Buffer;
use ovim::editor::Motions;
use proptest::prelude::*;

// ============================================================================
// Test Strategies
// ============================================================================

/// Strategy for multiline text that exercises word-motion edge cases.
fn arb_motion_text() -> impl Strategy<Value = String> {
    prop_oneof![
        // Word-heavy text: words, punctuation, whitespace
        3 => prop::collection::vec(
            prop_oneof![
                3 => "[a-zA-Z_]{1,8}",
                1 => "[!@#$%^&*]{1,3}",
                1 => "[ \t]{1,4}",
            ],
            1..20,
        ).prop_map(|chunks| chunks.join("")),

        // Multiline with varied content
        3 => prop::collection::vec(
            prop_oneof![
                3 => "[a-zA-Z0-9_ .!?]{0,30}",
                1 => Just("".to_string()),  // empty lines
                1 => "[ \t]{1,8}",          // whitespace-only lines
            ],
            1..8,
        ).prop_map(|lines| lines.join("\n")),

        // Edge cases
        1 => prop_oneof![
            Just("".to_string()),
            Just("a".to_string()),
            Just("   ".to_string()),
            Just("hello world".to_string()),
            Just("one\n\n\nfour".to_string()),
            Just("fn foo() {\n  bar();\n}".to_string()),
            Just("word   word".to_string()),
        ],
    ]
}

/// Strategy for text with brackets for % motion testing.
fn arb_bracket_text() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("(hello)".to_string()),
        Just("(a(b)c)".to_string()),
        Just("[1, [2, 3], 4]".to_string()),
        Just("{a {b {c} d} e}".to_string()),
        Just("fn foo(x: i32) {\n  bar(x)\n}".to_string()),
        Just("if (a && (b || c)) {}".to_string()),
        Just("([{nested}])".to_string()),
        Just("no brackets here".to_string()),
        Just("(unmatched".to_string()),
        Just("((()))".to_string()),
    ]
}

/// Helper: absolute cursor position as (line * MAX_COL + col) for ordering comparisons.
fn cursor_ord(buffer: &Buffer) -> (usize, usize) {
    (buffer.cursor().line(), buffer.cursor().col())
}

/// Helper: clamp cursor to valid buffer position before applying motion.
fn clamp_cursor(buffer: &mut Buffer) {
    let line_count = buffer.line_count();
    let cur_line = buffer.cursor().line();
    if cur_line >= line_count {
        let safe_line = line_count.saturating_sub(1);
        buffer.cursor_mut().set_line(safe_line);
    }
    let line = buffer.cursor().line();
    let line_text = buffer
        .line(line)
        .unwrap_or_default();
    let line_len = line_text.trim_end_matches('\n').chars().count();
    let cur_col = buffer.cursor().col();
    if line_len == 0 {
        buffer.cursor_mut().set_col(0);
    } else if cur_col >= line_len {
        buffer
            .cursor_mut()
            .set_col(line_len.saturating_sub(1));
    }
}

/// Helper: check that cursor is within valid buffer bounds.
///
/// Motions must leave cursor in bounds on their own — we do NOT call
/// `validate_cursor_position()` here. This ensures bugs are caught at the
/// motion layer, not masked by the Editor's post-key safety net.
fn assert_cursor_in_bounds(buffer: &Buffer, context: &str) -> Result<(), TestCaseError> {
    let cursor = buffer.cursor();
    let line_count = buffer.line_count();
    prop_assert!(
        cursor.line() < line_count,
        "{}: cursor line {} >= line_count {}",
        context,
        cursor.line(),
        line_count
    );
    let line_text = buffer
        .line(cursor.line())
        .unwrap_or_default();
    let line_len = line_text.trim_end_matches('\n').chars().count();
    prop_assert!(
        cursor.col() <= line_len,
        "{}: cursor col {} > line_len {} on line {}",
        context,
        cursor.col(),
        line_len,
        cursor.line()
    );
    Ok(())
}

// ============================================================================
// Property Tests: Word Motions (w, W, b, B, e, E)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Property: word_forward never panics and cursor stays in bounds.
    #[test]
    fn prop_word_forward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 1..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::word_forward(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "word_forward")?;
    }

    /// Property: word_forward_big never panics and cursor stays in bounds.
    #[test]
    fn prop_word_forward_big_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 1..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::word_forward_big(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "word_forward_big")?;
    }

    /// Property: word_backward never panics and cursor stays in bounds.
    #[test]
    fn prop_word_backward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 1..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::word_backward(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "word_backward")?;
    }

    /// Property: word_backward_big never panics and cursor stays in bounds.
    #[test]
    fn prop_word_backward_big_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 1..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::word_backward_big(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "word_backward_big")?;
    }

    /// Property: word_end_forward never panics and cursor stays in bounds.
    #[test]
    fn prop_word_end_forward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 1..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::word_end_forward(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "word_end_forward")?;
    }

    /// Property: word_end_forward_big never panics and cursor stays in bounds.
    #[test]
    fn prop_word_end_forward_big_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 1..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::word_end_forward_big(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "word_end_forward_big")?;
    }

    /// Property: word_end_backward never panics and cursor stays in bounds.
    #[test]
    fn prop_word_end_backward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 1..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::word_end_backward(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "word_end_backward")?;
    }

    /// Property: word_end_backward_big never panics and cursor stays in bounds.
    #[test]
    fn prop_word_end_backward_big_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 1..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::word_end_backward_big(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "word_end_backward_big")?;
    }

    /// Property: `w` moves forward or stays at end of buffer.
    ///
    /// A forward word motion should never move the cursor backward.
    /// After `w`, cursor position (line, col) should be >= original.
    #[test]
    fn prop_word_forward_monotonic(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        let before = cursor_ord(&buffer);
        Motions::word_forward(&mut buffer, 1);
        let after = cursor_ord(&buffer);

        prop_assert!(
            after >= before,
            "word_forward should not move backward: {:?} -> {:?}",
            before, after
        );
    }

    /// Property: `b` moves backward or stays at start of buffer.
    #[test]
    fn prop_word_backward_monotonic(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        let before = cursor_ord(&buffer);
        Motions::word_backward(&mut buffer, 1);
        let after = cursor_ord(&buffer);

        prop_assert!(
            after <= before,
            "word_backward should not move forward: {:?} -> {:?}",
            before, after
        );
    }

    /// Property: `e` moves forward or stays at end of buffer.
    #[test]
    fn prop_word_end_forward_monotonic(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        let before = cursor_ord(&buffer);
        Motions::word_end_forward(&mut buffer, 1);
        let after = cursor_ord(&buffer);

        prop_assert!(
            after >= before,
            "word_end_forward should not move backward: {:?} -> {:?}",
            before, after
        );
    }

    /// Property: `ge` moves backward or stays at start of buffer.
    #[test]
    fn prop_word_end_backward_monotonic(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        let before = cursor_ord(&buffer);
        Motions::word_end_backward(&mut buffer, 1);
        let after = cursor_ord(&buffer);

        prop_assert!(
            after <= before,
            "word_end_backward should not move forward: {:?} -> {:?}",
            before, after
        );
    }

    /// Property: `3w` = `w` + `w` + `w` (count decomposition).
    ///
    /// This is a fundamental Vim contract: a counted motion is equivalent
    /// to repeating the motion that many times.
    #[test]
    fn prop_word_forward_count_decomposes(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 2..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);
        let clamped_pos = cursor_ord(&buffer);

        // Apply counted motion
        Motions::word_forward(&mut buffer, count);
        let counted_pos = cursor_ord(&buffer);

        // Apply motion N times individually
        buffer.cursor_mut().set_position(clamped_pos.0, clamped_pos.1);
        for _ in 0..count {
            Motions::word_forward(&mut buffer, 1);
        }
        let iterated_pos = cursor_ord(&buffer);

        prop_assert_eq!(
            counted_pos, iterated_pos,
            "{}w should equal w repeated {} times",
            count, count
        );
    }

    /// Property: `3b` = `b` + `b` + `b` (count decomposition).
    #[test]
    fn prop_word_backward_count_decomposes(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 2..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);
        let clamped_pos = cursor_ord(&buffer);

        Motions::word_backward(&mut buffer, count);
        let counted_pos = cursor_ord(&buffer);

        buffer.cursor_mut().set_position(clamped_pos.0, clamped_pos.1);
        for _ in 0..count {
            Motions::word_backward(&mut buffer, 1);
        }
        let iterated_pos = cursor_ord(&buffer);

        prop_assert_eq!(
            counted_pos, iterated_pos,
            "{}b should equal b repeated {} times",
            count, count
        );
    }

    /// Property: `3e` = `e` + `e` + `e` (count decomposition).
    #[test]
    fn prop_word_end_forward_count_decomposes(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 2..5usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);
        let clamped_pos = cursor_ord(&buffer);

        Motions::word_end_forward(&mut buffer, count);
        let counted_pos = cursor_ord(&buffer);

        buffer.cursor_mut().set_position(clamped_pos.0, clamped_pos.1);
        for _ in 0..count {
            Motions::word_end_forward(&mut buffer, 1);
        }
        let iterated_pos = cursor_ord(&buffer);

        prop_assert_eq!(
            counted_pos, iterated_pos,
            "{}e should equal e repeated {} times",
            count, count
        );
    }
}

// ============================================================================
// Property Tests: Char-Find Motions (f, F, t, T)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Property: find_char_forward never panics and stays on same line.
    #[test]
    fn prop_find_char_forward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        ch in prop::char::range('a', 'z'),
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        let line_before = buffer.cursor().line();
        Motions::find_char_forward(&mut buffer, ch, 1);

        // f motion stays on same line
        prop_assert_eq!(
            buffer.cursor().line(),
            line_before,
            "find_char_forward should not change line"
        );
        assert_cursor_in_bounds(&buffer, "find_char_forward")?;
    }

    /// Property: find_char_backward never panics and stays on same line.
    #[test]
    fn prop_find_char_backward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        ch in prop::char::range('a', 'z'),
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        let line_before = buffer.cursor().line();
        Motions::find_char_backward(&mut buffer, ch, 1);

        prop_assert_eq!(
            buffer.cursor().line(),
            line_before,
            "find_char_backward should not change line"
        );
        assert_cursor_in_bounds(&buffer, "find_char_backward")?;
    }

    /// Property: till_char_forward never panics and stays on same line.
    #[test]
    fn prop_till_char_forward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        ch in prop::char::range('a', 'z'),
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        let line_before = buffer.cursor().line();
        Motions::till_char_forward(&mut buffer, ch, 1);

        prop_assert_eq!(
            buffer.cursor().line(),
            line_before,
            "till_char_forward should not change line"
        );
        assert_cursor_in_bounds(&buffer, "till_char_forward")?;
    }

    /// Property: till_char_backward never panics and stays on same line.
    #[test]
    fn prop_till_char_backward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        ch in prop::char::range('a', 'z'),
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        let line_before = buffer.cursor().line();
        Motions::till_char_backward(&mut buffer, ch, 1);

        prop_assert_eq!(
            buffer.cursor().line(),
            line_before,
            "till_char_backward should not change line"
        );
        assert_cursor_in_bounds(&buffer, "till_char_backward")?;
    }

    /// Property: f{ch} only moves forward if it finds the character.
    #[test]
    fn prop_find_char_forward_only_advances(
        text in "[a-z ]{5,30}",
        start_col in 0..10usize,
        ch in prop::char::range('a', 'z'),
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        clamp_cursor(&mut buffer);
        let safe_col = start_col.min(
            text.chars().count().saturating_sub(1)
        );
        buffer.cursor_mut().set_col(safe_col);

        let col_before = buffer.cursor().col();
        let found = Motions::find_char_forward(&mut buffer, ch, 1);

        if found {
            prop_assert!(
                buffer.cursor().col() > col_before,
                "f motion found char but didn't advance: col {} -> {}",
                col_before,
                buffer.cursor().col()
            );
        } else {
            prop_assert_eq!(
                buffer.cursor().col(),
                col_before,
                "f motion didn't find char but moved cursor"
            );
        }
    }
}

// ============================================================================
// Property Tests: Bracket Matching (%)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: jump_to_matching_bracket never panics.
    #[test]
    fn prop_bracket_match_no_panic(
        text in arb_bracket_text(),
        start_line in 0..5usize,
        start_col in 0..20usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        // Should never panic, even on unmatched brackets
        let _ = Motions::jump_to_matching_bracket(&mut buffer);

        assert_cursor_in_bounds(&buffer, "jump_to_matching_bracket")?;
    }

    /// Property: % is an involution when cursor starts on a bracket.
    ///
    /// If cursor is directly on a bracket character and % succeeds, applying
    /// % again should return to the original position. This is Vim's core
    /// bracket-matching contract.
    ///
    /// Note: when cursor is NOT on a bracket, % first searches forward on the
    /// line for one, so `% %` returns to the found bracket, not the original
    /// position. We only test the on-bracket case here.
    #[test]
    fn prop_bracket_match_involution(
        text in arb_bracket_text(),
        start_line in 0..5usize,
        start_col in 0..20usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        // Check if cursor is directly on a bracket character
        let line_text = buffer.line(buffer.cursor().line()).unwrap_or_default();
        let chars: Vec<char> = line_text.trim_end_matches('\n').chars().collect();
        let col = buffer.cursor().col();
        let on_bracket = col < chars.len()
            && matches!(chars[col], '(' | ')' | '[' | ']' | '{' | '}');

        if !on_bracket {
            // Skip — involution only holds when starting on a bracket
            return Ok(());
        }

        let original = cursor_ord(&buffer);
        let first_match = Motions::jump_to_matching_bracket(&mut buffer);

        if first_match {
            let second_match = Motions::jump_to_matching_bracket(&mut buffer);

            if second_match {
                let returned = cursor_ord(&buffer);
                prop_assert_eq!(
                    returned, original,
                    "% twice from bracket should return to original: \
                     original={:?}, returned={:?}",
                    original, returned
                );
            }
        }
    }
}

// ============================================================================
// Property Tests: Paragraph and Sentence Motions
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: paragraph_forward never panics and cursor stays in bounds.
    #[test]
    fn prop_paragraph_forward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        Motions::paragraph_forward(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "paragraph_forward")?;
    }

    /// Property: paragraph_backward never panics and cursor stays in bounds.
    #[test]
    fn prop_paragraph_backward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        Motions::paragraph_backward(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "paragraph_backward")?;
    }

    /// Property: paragraph_forward moves forward (or stays at end).
    #[test]
    fn prop_paragraph_forward_monotonic(
        text in arb_motion_text(),
        start_line in 0..10usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        let before = buffer.cursor().line();
        Motions::paragraph_forward(&mut buffer, 1);
        let after = buffer.cursor().line();

        prop_assert!(
            after >= before,
            "paragraph_forward should not go backward: {} -> {}",
            before, after
        );
    }

    /// Property: paragraph_backward moves backward (or stays at start).
    #[test]
    fn prop_paragraph_backward_monotonic(
        text in arb_motion_text(),
        start_line in 0..10usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        let before = buffer.cursor().line();
        Motions::paragraph_backward(&mut buffer, 1);
        let after = buffer.cursor().line();

        prop_assert!(
            after <= before,
            "paragraph_backward should not go forward: {} -> {}",
            before, after
        );
    }

    /// Property: sentence_forward never panics.
    #[test]
    fn prop_sentence_forward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::sentence_forward(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "sentence_forward")?;
    }

    /// Property: sentence_backward never panics.
    #[test]
    fn prop_sentence_backward_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        start_col in 0..30usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::sentence_backward(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "sentence_backward")?;
    }
}

// ============================================================================
// Property Tests: Line Motions (^, g_, 0, $)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Property: first_non_blank lands on a non-blank char (or col 0 if line is empty/all-blank).
    #[test]
    fn prop_first_non_blank_correct(
        text in arb_motion_text(),
        start_line in 0..10usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        Motions::first_non_blank(&mut buffer);

        let col = buffer.cursor().col();
        let line_text = buffer
            .line(safe_line)
            .unwrap_or_default();
        let trimmed = line_text.trim_end_matches('\n');

        if !trimmed.is_empty() {
            // Col should point to a non-whitespace char (or 0 if all whitespace)
            let chars: Vec<char> = trimmed.chars().collect();
            if col < chars.len() {
                let at_cursor = chars[col];
                if chars.iter().any(|c| !c.is_whitespace()) {
                    prop_assert!(
                        !at_cursor.is_whitespace(),
                        "first_non_blank landed on whitespace '{}' at col {}",
                        at_cursor, col
                    );
                }
            }
        }

        assert_cursor_in_bounds(&buffer, "first_non_blank")?;
    }

    /// Property: last_non_blank lands on a non-blank char (or col 0 if line is empty/all-blank).
    #[test]
    fn prop_last_non_blank_correct(
        text in arb_motion_text(),
        start_line in 0..10usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        Motions::last_non_blank(&mut buffer);

        let col = buffer.cursor().col();
        let line_text = buffer
            .line(safe_line)
            .unwrap_or_default();
        let trimmed = line_text.trim_end_matches('\n');
        let chars: Vec<char> = trimmed.chars().collect();

        if !chars.is_empty() && chars.iter().any(|c| !c.is_whitespace()) {
            // Should land on a non-whitespace char
            if col < chars.len() {
                prop_assert!(
                    !chars[col].is_whitespace(),
                    "last_non_blank landed on whitespace at col {}",
                    col
                );
                // No non-whitespace chars after this position
                let has_nonws_after = chars[col + 1..].iter().any(|c| !c.is_whitespace());
                prop_assert!(
                    !has_nonws_after,
                    "last_non_blank at col {} but found non-ws after it",
                    col
                );
            }
        }

        assert_cursor_in_bounds(&buffer, "last_non_blank")?;
    }

    /// Property: plus_motion moves to next line's first non-blank.
    #[test]
    fn prop_plus_motion_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        Motions::plus_motion(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "plus_motion")?;
        // Should be on a line >= start_line (moved down)
        prop_assert!(
            buffer.cursor().line() >= safe_line,
            "plus_motion should move down: {} -> {}",
            safe_line, buffer.cursor().line()
        );
    }

    /// Property: minus_motion moves to previous line's first non-blank.
    #[test]
    fn prop_minus_motion_bounds_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        Motions::minus_motion(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "minus_motion")?;
        prop_assert!(
            buffer.cursor().line() <= safe_line,
            "minus_motion should move up: {} -> {}",
            safe_line, buffer.cursor().line()
        );
    }
}

// ============================================================================
// Property Tests: Scroll Motions
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: scroll_half_page_down never panics, cursor stays in bounds.
    #[test]
    fn prop_scroll_half_page_down_safe(
        text in arb_motion_text(),
        viewport_start in 0..10usize,
        viewport_height in 5..30usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_start = viewport_start.min(lc.saturating_sub(1));
        buffer.cursor_mut().set_position(safe_start, 0);

        let _ = Motions::scroll_half_page_down(&mut buffer, safe_start, viewport_height);

        assert_cursor_in_bounds(&buffer, "scroll_half_page_down")?;
    }

    /// Property: scroll_half_page_up never panics, cursor stays in bounds.
    #[test]
    fn prop_scroll_half_page_up_safe(
        text in arb_motion_text(),
        viewport_start in 0..10usize,
        viewport_height in 5..30usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_start = viewport_start.min(lc.saturating_sub(1));
        buffer.cursor_mut().set_position(safe_start, 0);

        let _ = Motions::scroll_half_page_up(&mut buffer, safe_start, viewport_height);

        assert_cursor_in_bounds(&buffer, "scroll_half_page_up")?;
    }

    /// Property: scroll_page_down never panics.
    #[test]
    fn prop_scroll_page_down_safe(
        text in arb_motion_text(),
        viewport_start in 0..10usize,
        viewport_height in 5..30usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_start = viewport_start.min(lc.saturating_sub(1));
        buffer.cursor_mut().set_position(safe_start, 0);

        let _ = Motions::scroll_page_down(&mut buffer, safe_start, viewport_height);

        assert_cursor_in_bounds(&buffer, "scroll_page_down")?;
    }

    /// Property: scroll_page_up never panics.
    #[test]
    fn prop_scroll_page_up_safe(
        text in arb_motion_text(),
        viewport_start in 0..10usize,
        viewport_height in 5..30usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_start = viewport_start.min(lc.saturating_sub(1));
        buffer.cursor_mut().set_position(safe_start, 0);

        let _ = Motions::scroll_page_up(&mut buffer, safe_start, viewport_height);

        assert_cursor_in_bounds(&buffer, "scroll_page_up")?;
    }
}

// ============================================================================
// Property Tests: Section and Method Motions
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: section_forward never panics and moves forward.
    #[test]
    fn prop_section_forward_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        let before = buffer.cursor().line();
        Motions::section_forward(&mut buffer, count);
        let after = buffer.cursor().line();

        prop_assert!(after >= before, "section_forward should not go backward");
        assert_cursor_in_bounds(&buffer, "section_forward")?;
    }

    /// Property: section_backward never panics and moves backward.
    #[test]
    fn prop_section_backward_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        let before = buffer.cursor().line();
        Motions::section_backward(&mut buffer, count);
        let after = buffer.cursor().line();

        prop_assert!(after <= before, "section_backward should not go forward");
        assert_cursor_in_bounds(&buffer, "section_backward")?;
    }

    /// Property: method_forward never panics.
    #[test]
    fn prop_method_forward_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        Motions::method_forward(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "method_forward")?;
    }

    /// Property: method_backward never panics.
    #[test]
    fn prop_method_backward_safe(
        text in arb_motion_text(),
        start_line in 0..10usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, 0);

        Motions::method_backward(&mut buffer, count);

        assert_cursor_in_bounds(&buffer, "method_backward")?;
    }
}

// ============================================================================
// Property Tests: Unmatched Brace/Paren Motions ([{, ]}, [(, ]))
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: unmatched brace motions never panic.
    #[test]
    fn prop_unmatched_brace_no_panic(
        text in arb_bracket_text(),
        start_line in 0..5usize,
        start_col in 0..20usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::unmatched_brace_backward(&mut buffer, count);
        assert_cursor_in_bounds(&buffer, "unmatched_brace_backward")?;

        // Reset position for the forward test
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::unmatched_brace_forward(&mut buffer, count);
        assert_cursor_in_bounds(&buffer, "unmatched_brace_forward")?;
    }

    /// Property: unmatched paren motions never panic.
    #[test]
    fn prop_unmatched_paren_no_panic(
        text in arb_bracket_text(),
        start_line in 0..5usize,
        start_col in 0..20usize,
        count in 1..3usize,
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let lc = buffer.line_count();
        let safe_line = start_line % lc;
        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::unmatched_paren_backward(&mut buffer, count);
        assert_cursor_in_bounds(&buffer, "unmatched_paren_backward")?;

        buffer.cursor_mut().set_position(safe_line, start_col);
        clamp_cursor(&mut buffer);

        Motions::unmatched_paren_forward(&mut buffer, count);
        assert_cursor_in_bounds(&buffer, "unmatched_paren_forward")?;
    }
}

// ============================================================================
// Property Tests: Composite / Stress
// ============================================================================

/// Motion operations for stress testing.
#[derive(Debug, Clone)]
enum MotionOp {
    WordForward(usize),
    WordBackward(usize),
    WordForwardBig(usize),
    WordBackwardBig(usize),
    WordEndForward(usize),
    WordEndForwardBig(usize),
    WordEndBackward(usize),
    WordEndBackwardBig(usize),
    FindCharForward(char),
    FindCharBackward(char),
    FirstNonBlank,
    LastNonBlank,
    ParagraphForward(usize),
    ParagraphBackward(usize),
    SentenceForward(usize),
    SentenceBackward(usize),
    JumpToMatchingBracket,
}

fn arb_motion_op() -> impl Strategy<Value = MotionOp> {
    prop_oneof![
        // Word motions (heavy weight - these are the most common and most complex)
        3 => (1..4usize).prop_map(MotionOp::WordForward),
        3 => (1..4usize).prop_map(MotionOp::WordBackward),
        2 => (1..4usize).prop_map(MotionOp::WordForwardBig),
        2 => (1..4usize).prop_map(MotionOp::WordBackwardBig),
        2 => (1..4usize).prop_map(MotionOp::WordEndForward),
        2 => (1..4usize).prop_map(MotionOp::WordEndForwardBig),
        1 => (1..4usize).prop_map(MotionOp::WordEndBackward),
        1 => (1..4usize).prop_map(MotionOp::WordEndBackwardBig),
        // Char-find motions
        1 => prop::char::range('a', 'z').prop_map(MotionOp::FindCharForward),
        1 => prop::char::range('a', 'z').prop_map(MotionOp::FindCharBackward),
        // Line motions
        1 => Just(MotionOp::FirstNonBlank),
        1 => Just(MotionOp::LastNonBlank),
        // Paragraph/sentence motions
        1 => (1..3usize).prop_map(MotionOp::ParagraphForward),
        1 => (1..3usize).prop_map(MotionOp::ParagraphBackward),
        1 => (1..3usize).prop_map(MotionOp::SentenceForward),
        1 => (1..3usize).prop_map(MotionOp::SentenceBackward),
        // Bracket matching
        1 => Just(MotionOp::JumpToMatchingBracket),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property: Arbitrary sequence of motions never panics and cursor stays in bounds.
    ///
    /// This is the ultimate stress test: apply a random sequence of motions to
    /// random text and verify that the cursor never escapes the buffer.
    #[test]
    fn prop_motion_sequence_always_safe(
        text in arb_motion_text(),
        ops in prop::collection::vec(arb_motion_op(), 1..30),
    ) {
        let mut buffer = Buffer::new_from_str(&text);

        for (i, op) in ops.iter().enumerate() {
            match op {
                MotionOp::WordForward(n) => Motions::word_forward(&mut buffer, *n),
                MotionOp::WordBackward(n) => Motions::word_backward(&mut buffer, *n),
                MotionOp::WordForwardBig(n) => Motions::word_forward_big(&mut buffer, *n),
                MotionOp::WordBackwardBig(n) => Motions::word_backward_big(&mut buffer, *n),
                MotionOp::WordEndForward(n) => Motions::word_end_forward(&mut buffer, *n),
                MotionOp::WordEndForwardBig(n) => Motions::word_end_forward_big(&mut buffer, *n),
                MotionOp::WordEndBackward(n) => Motions::word_end_backward(&mut buffer, *n),
                MotionOp::WordEndBackwardBig(n) => Motions::word_end_backward_big(&mut buffer, *n),
                MotionOp::FindCharForward(ch) => { Motions::find_char_forward(&mut buffer, *ch, 1); },
                MotionOp::FindCharBackward(ch) => { Motions::find_char_backward(&mut buffer, *ch, 1); },
                MotionOp::FirstNonBlank => Motions::first_non_blank(&mut buffer),
                MotionOp::LastNonBlank => Motions::last_non_blank(&mut buffer),
                MotionOp::ParagraphForward(n) => Motions::paragraph_forward(&mut buffer, *n),
                MotionOp::ParagraphBackward(n) => Motions::paragraph_backward(&mut buffer, *n),
                MotionOp::SentenceForward(n) => Motions::sentence_forward(&mut buffer, *n),
                MotionOp::SentenceBackward(n) => Motions::sentence_backward(&mut buffer, *n),
                MotionOp::JumpToMatchingBracket => { Motions::jump_to_matching_bracket(&mut buffer); },
            }

            assert_cursor_in_bounds(&buffer, &format!("motion sequence step {}: {:?}", i, op))?;
        }
    }

    /// Property: Buffer content is never modified by motions.
    ///
    /// Motions are read-only operations — they should never alter the buffer text.
    #[test]
    fn prop_motions_dont_modify_buffer(
        text in arb_motion_text(),
        ops in prop::collection::vec(arb_motion_op(), 1..20),
    ) {
        let mut buffer = Buffer::new_from_str(&text);
        let original_content = buffer.rope().to_string();

        for op in &ops {
            match op {
                MotionOp::WordForward(n) => Motions::word_forward(&mut buffer, *n),
                MotionOp::WordBackward(n) => Motions::word_backward(&mut buffer, *n),
                MotionOp::WordForwardBig(n) => Motions::word_forward_big(&mut buffer, *n),
                MotionOp::WordBackwardBig(n) => Motions::word_backward_big(&mut buffer, *n),
                MotionOp::WordEndForward(n) => Motions::word_end_forward(&mut buffer, *n),
                MotionOp::WordEndForwardBig(n) => Motions::word_end_forward_big(&mut buffer, *n),
                MotionOp::WordEndBackward(n) => Motions::word_end_backward(&mut buffer, *n),
                MotionOp::WordEndBackwardBig(n) => Motions::word_end_backward_big(&mut buffer, *n),
                MotionOp::FindCharForward(ch) => { Motions::find_char_forward(&mut buffer, *ch, 1); },
                MotionOp::FindCharBackward(ch) => { Motions::find_char_backward(&mut buffer, *ch, 1); },
                MotionOp::FirstNonBlank => Motions::first_non_blank(&mut buffer),
                MotionOp::LastNonBlank => Motions::last_non_blank(&mut buffer),
                MotionOp::ParagraphForward(n) => Motions::paragraph_forward(&mut buffer, *n),
                MotionOp::ParagraphBackward(n) => Motions::paragraph_backward(&mut buffer, *n),
                MotionOp::SentenceForward(n) => Motions::sentence_forward(&mut buffer, *n),
                MotionOp::SentenceBackward(n) => Motions::sentence_backward(&mut buffer, *n),
                MotionOp::JumpToMatchingBracket => { Motions::jump_to_matching_bracket(&mut buffer); },
            }
        }

        prop_assert_eq!(
            buffer.rope().to_string(),
            original_content,
            "Motions should never modify buffer content"
        );
    }
}
