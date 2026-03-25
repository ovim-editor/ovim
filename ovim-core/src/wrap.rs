//! Shared wrap computation used by both `WrapMap` (core) and the renderer.
//!
//! This module provides a single source of truth for how lines are broken
//! into visual rows when soft-wrapping is enabled. Both the structural
//! mapping (`WrapMap`) and the visual rendering (`split_line_into_rows`)
//! call into these functions, guaranteeing consistent behaviour.

use crate::display::char_display_width;

/// Computes the character indices where a line should wrap.
///
/// Returns a `Vec<usize>` of char indices at which a new visual row begins.
/// For example, if the line "abcdefgh" wraps at width 3, the result would be
/// `[3, 6]` — meaning rows are chars `[0..3)`, `[3..6)`, `[6..8)`.
///
/// Wide characters that don't fit at the end of a row are pushed to the
/// next row (the remaining space is padded), matching terminal and Neovim
/// behaviour.
///
/// # Arguments
/// * `line` — the text of a single line (no trailing newline)
/// * `max_width` — the available width in display columns (must be ≥ 1)
/// * `tab_width` — how many display columns a tab occupies (tab stops)
pub fn compute_wrap_points(line: &str, max_width: usize, tab_width: usize) -> Vec<usize> {
    let max_width = max_width.max(1);
    let mut wrap_points = Vec::new();
    let mut current_width: usize = 0;

    for (char_idx, ch) in line.chars().enumerate() {
        let ch_width = if ch == '\t' {
            let spaces = tab_width - (current_width % tab_width);
            spaces
        } else {
            char_display_width(ch)
        };

        if current_width + ch_width > max_width {
            // This character doesn't fit on the current row → start a new row
            wrap_points.push(char_idx);
            current_width = ch_width;
        } else {
            current_width += ch_width;
        }

        // If we've exactly filled the row and there are more characters coming,
        // the *next* character starts a new row. We don't push a wrap point
        // here — it'll be handled when we see the next character overflow.
        // (If current_width == max_width the next char with width ≥ 1 will
        // trigger the `current_width + ch_width > max_width` branch above.)
    }

    wrap_points
}

/// Returns the number of visual rows a line occupies when wrapped.
///
/// This is the authoritative function both `WrapMap` and the renderer
/// should use. It accounts for wide characters being pushed to the next
/// row, unlike a naïve `display_width / wrap_width` calculation.
pub fn visual_line_count(line: &str, max_width: usize, tab_width: usize) -> usize {
    if line.is_empty() {
        return 1;
    }
    // Number of rows = number of wrap points + 1
    compute_wrap_points(line, max_width, tab_width).len() + 1
}

/// Like [`compute_wrap_points`] but accounts for inline decorations (e.g.
/// inlay hints) that add display width at specific character positions.
///
/// `inline_widths` is a sorted slice of `(char_idx, display_width)` pairs.
/// Each decoration's width is added to the running total just before the
/// character at `char_idx`, matching how the renderer inserts decoration
/// text before splitting into rows.
pub fn compute_wrap_points_with_decorations(
    line: &str,
    max_width: usize,
    tab_width: usize,
    inline_widths: &[(usize, usize)],
) -> Vec<usize> {
    if inline_widths.is_empty() {
        return compute_wrap_points(line, max_width, tab_width);
    }

    let max_width = max_width.max(1);
    let mut wrap_points = Vec::new();
    let mut current_width: usize = 0;
    let mut dec_idx = 0;

    for (char_idx, ch) in line.chars().enumerate() {
        // Add decoration width at this character position.  Decoration text
        // is inserted before the character by the renderer, so its width is
        // accumulated before the character's own width check.
        while dec_idx < inline_widths.len() && inline_widths[dec_idx].0 == char_idx {
            current_width += inline_widths[dec_idx].1;
            dec_idx += 1;
        }

        let ch_width = if ch == '\t' {
            tab_width - (current_width % tab_width)
        } else {
            char_display_width(ch)
        };

        if current_width + ch_width > max_width {
            wrap_points.push(char_idx);
            current_width = ch_width;
        } else {
            current_width += ch_width;
        }
    }

    wrap_points
}

/// Like [`visual_line_count`] but accounts for inline decorations.
pub fn visual_line_count_with_decorations(
    line: &str,
    max_width: usize,
    tab_width: usize,
    inline_widths: &[(usize, usize)],
) -> usize {
    if line.is_empty() && inline_widths.is_empty() {
        return 1;
    }
    compute_wrap_points_with_decorations(line, max_width, tab_width, inline_widths).len() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_line() {
        assert_eq!(visual_line_count("", 80, 4), 1);
        assert_eq!(compute_wrap_points("", 80, 4), Vec::<usize>::new());
    }

    #[test]
    fn line_fits() {
        assert_eq!(visual_line_count("hello", 80, 4), 1);
        assert_eq!(compute_wrap_points("hello", 80, 4), Vec::<usize>::new());
    }

    #[test]
    fn line_exactly_fits() {
        assert_eq!(visual_line_count("abcde", 5, 4), 1);
        assert_eq!(compute_wrap_points("abcde", 5, 4), Vec::<usize>::new());
    }

    #[test]
    fn line_wraps_once() {
        // "abcdef" at width 5 → "abcde" + "f" = 2 rows
        assert_eq!(visual_line_count("abcdef", 5, 4), 2);
        assert_eq!(compute_wrap_points("abcdef", 5, 4), vec![5]);
    }

    #[test]
    fn line_wraps_twice() {
        // "abcdefghijk" (11 chars) at width 5 → 3 rows
        assert_eq!(visual_line_count("abcdefghijk", 5, 4), 3);
        assert_eq!(compute_wrap_points("abcdefghijk", 5, 4), vec![5, 10]);
    }

    #[test]
    fn wide_char_pushed_to_next_row() {
        // Width 3: "aa世" = 2 + 2 = 4 display cols
        // Row 1: "aa" (can't fit 世, width 2+2=4 > 3) → wrap before 世
        // Row 2: "世"
        assert_eq!(visual_line_count("aa世", 3, 4), 2);
        assert_eq!(compute_wrap_points("aa世", 3, 4), vec![2]);
    }

    #[test]
    fn wide_chars_cause_extra_rows() {
        // Width 3: "世世世" = 6 display cols
        // Naïve: div_ceil(6, 3) = 2
        // Actual: 世(2) fits row1 (pad 1), 世(2) fits row2 (pad 1), 世(2) fits row3 (pad 1) = 3 rows
        assert_eq!(visual_line_count("世世世", 3, 4), 3);
        assert_eq!(compute_wrap_points("世世世", 3, 4), vec![1, 2]);
    }

    #[test]
    fn wide_char_exactly_fits() {
        // Width 4: "aa世" = 2 + 2 = 4, fits exactly
        assert_eq!(visual_line_count("aa世", 4, 4), 1);
        assert_eq!(compute_wrap_points("aa世", 4, 4), Vec::<usize>::new());
    }

    #[test]
    fn tab_handling() {
        // Width 8, tab_width 4: "\thello" = 4 + 5 = 9 display cols → 2 rows
        assert_eq!(visual_line_count("\thello", 8, 4), 2);
        // Tab takes 4 cols, then "hell" fills to 8, "o" wraps
        assert_eq!(compute_wrap_points("\thello", 8, 4), vec![5]);
    }

    #[test]
    fn tab_at_boundary() {
        // Width 4, tab_width 4: "\ta" = 4 + 1 = 5 display cols → 2 rows
        assert_eq!(visual_line_count("\ta", 4, 4), 2);
        assert_eq!(compute_wrap_points("\ta", 4, 4), vec![1]);
    }

    #[test]
    fn mixed_wide_and_ascii() {
        // Width 5: "ab世cd" = 1+1+2+1+1 = 6
        // Row 1: "ab世" (1+1+2=4, next 'c' would be 5 → fits), so "ab世c" (5)
        // Row 2: "d"
        assert_eq!(visual_line_count("ab世cd", 5, 4), 2);
        assert_eq!(compute_wrap_points("ab世cd", 5, 4), vec![4]);
    }

    #[test]
    fn width_1() {
        // Each character gets its own row (wide chars also get 1 row since width=max(1,1)=1)
        // But wide chars (width 2) can't fit in width 1... they still need to go somewhere.
        // We put them on their own row (width overflows but it's the minimum).
        assert_eq!(visual_line_count("abc", 1, 4), 3);
        assert_eq!(compute_wrap_points("abc", 1, 4), vec![1, 2]);
    }

    #[test]
    fn control_chars() {
        // Control char \x01 has display width 2 (caret notation ^A)
        // Width 3: "a\x01b" = 1 + 2 + 1 = 4 → 2 rows
        // "a\x01" = 3, then "b" = 1 → wraps after char 2
        assert_eq!(visual_line_count("a\x01b", 3, 4), 2);
        assert_eq!(compute_wrap_points("a\x01b", 3, 4), vec![2]);
    }

    // --- decoration-aware wrap tests ---

    #[test]
    fn decoration_no_effect_when_line_fits() {
        // "hello" (5) + ": i32" (5) = 10, fits in width 20
        let decs = vec![(5, 5)];
        assert_eq!(
            compute_wrap_points_with_decorations("hello", 20, 4, &decs),
            Vec::<usize>::new()
        );
        assert_eq!(visual_line_count_with_decorations("hello", 20, 4, &decs), 1);
    }

    #[test]
    fn decoration_causes_wrap() {
        // "let x = 5" (10 chars) at width 12
        // Without decoration: 10 cols, fits in 1 row
        // With ": i32" (5 cols) at char 5: "let x: i32 = 5" = 15 cols → wraps
        let decs = vec![(5, 5)];
        assert_eq!(
            compute_wrap_points_with_decorations("let x = 5", 12, 4, &decs),
            vec![7] // wrap happens at char 7 (after "let x" + ": i32" = 10, then " =" = 12, " " wraps)
        );
    }

    #[test]
    fn decoration_at_start_of_line() {
        // Decoration at char 0 with width 10, line "ab" at width 8
        // Decoration (10) > width (8) → wraps, then "ab" fits in next row
        let decs = vec![(0, 10)];
        let points = compute_wrap_points_with_decorations("ab", 8, 4, &decs);
        assert!(!points.is_empty(), "should wrap when decoration exceeds width");
        assert_eq!(visual_line_count_with_decorations("ab", 8, 4, &decs), 2);
    }

    #[test]
    fn multiple_decorations_on_same_line() {
        // "a b c" at width 10
        // Dec at char 1: 3 cols, dec at char 3: 3 cols
        // Total: a(1) + dec(3) + " "(1) + b(1) + dec(3) + " "(1) + c(1) = 11 → wraps
        let decs = vec![(1, 3), (3, 3)];
        assert!(visual_line_count_with_decorations("a b c", 10, 4, &decs) >= 2);
    }

    #[test]
    fn empty_decorations_same_as_plain() {
        let line = "abcdefghij";
        for width in [3, 5, 8, 80] {
            assert_eq!(
                compute_wrap_points_with_decorations(line, width, 4, &[]),
                compute_wrap_points(line, width, 4),
                "empty decorations should match plain wrap at width {}",
                width
            );
        }
    }

    /// Cross-validation: visual_line_count should agree with
    /// compute_wrap_points().len() + 1 for all inputs.
    #[test]
    fn count_agrees_with_wrap_points() {
        let cases = [
            ("", 80),
            ("hello", 3),
            ("世世世", 3),
            ("世世世", 4),
            ("世世世", 5),
            ("a\tb\tc", 8),
            ("\t\t\t", 4),
            ("abcdefghij", 3),
        ];
        for (line, width) in cases {
            let points = compute_wrap_points(line, width, 4);
            let count = visual_line_count(line, width, 4);
            assert_eq!(
                count,
                points.len() + 1,
                "mismatch for {:?} at width {}: count={}, points={:?}",
                line,
                width,
                count,
                points
            );
        }
    }
}
