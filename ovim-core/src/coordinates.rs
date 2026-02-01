//! Type-safe coordinate newtypes.
//!
//! The editor uses multiple coordinate spaces that are all represented as
//! `usize`. Mixing them is a common source of bugs (see OV-00050, OV-00051).
//! These newtypes provide compile-time safety with zero runtime cost.
//!
//! # Coordinate spaces
//!
//! - **`CharIdx`** — a character index within a line (0-based). This is what
//!   `Cursor::col()` returns. It counts Unicode scalar values (grapheme-naive).
//!
//! - **`DisplayCol`** — a display column (0-based). Accounts for wide
//!   characters (width 2) and tab expansion. This is what the terminal sees.
//!
//! Converting between them requires the line text and tab width — use
//! [`crate::display::char_col_to_display_col`] and
//! [`crate::display::display_col_to_char_col`].

use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};

// ---------------------------------------------------------------------------
// CharIdx
// ---------------------------------------------------------------------------

/// A character index (0-based) within a line of text.
///
/// This counts Unicode scalar values. Converting to/from `DisplayCol`
/// requires the line text and tab width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CharIdx(pub usize);

impl CharIdx {
    pub const ZERO: Self = Self(0);

    #[inline]
    pub fn as_usize(self) -> usize {
        self.0
    }

    /// Convert to display column given line text and tab width.
    #[inline]
    pub fn to_display_col(self, line_text: &str, tab_width: usize) -> DisplayCol {
        DisplayCol(crate::display::char_col_to_display_col(
            line_text, self.0, tab_width,
        ))
    }
}

impl From<usize> for CharIdx {
    #[inline]
    fn from(v: usize) -> Self {
        Self(v)
    }
}

impl fmt::Display for CharIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Add<usize> for CharIdx {
    type Output = Self;
    #[inline]
    fn add(self, rhs: usize) -> Self {
        Self(self.0 + rhs)
    }
}

impl Sub<usize> for CharIdx {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: usize) -> Self {
        Self(self.0 - rhs)
    }
}

impl AddAssign<usize> for CharIdx {
    #[inline]
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl SubAssign<usize> for CharIdx {
    #[inline]
    fn sub_assign(&mut self, rhs: usize) {
        self.0 -= rhs;
    }
}

// ---------------------------------------------------------------------------
// DisplayCol
// ---------------------------------------------------------------------------

/// A display column (0-based) — the column as it appears in the terminal.
///
/// Accounts for wide characters (CJK, emoji), tab expansion, and
/// control-character caret notation. Converting to/from `CharIdx`
/// requires the line text and tab width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DisplayCol(pub usize);

impl DisplayCol {
    pub const ZERO: Self = Self(0);

    #[inline]
    pub fn as_usize(self) -> usize {
        self.0
    }

    /// Convert to character index given line text and tab width.
    #[inline]
    pub fn to_char_idx(self, line_text: &str, tab_width: usize) -> CharIdx {
        CharIdx(crate::display::display_col_to_char_col(
            line_text, self.0, tab_width,
        ))
    }

    /// Saturating subtraction.
    #[inline]
    pub fn saturating_sub(self, rhs: usize) -> Self {
        Self(self.0.saturating_sub(rhs))
    }
}

impl From<usize> for DisplayCol {
    #[inline]
    fn from(v: usize) -> Self {
        Self(v)
    }
}

impl fmt::Display for DisplayCol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Add<usize> for DisplayCol {
    type Output = Self;
    #[inline]
    fn add(self, rhs: usize) -> Self {
        Self(self.0 + rhs)
    }
}

impl Sub<usize> for DisplayCol {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: usize) -> Self {
        Self(self.0 - rhs)
    }
}

impl Sub for DisplayCol {
    type Output = usize;
    #[inline]
    fn sub(self, rhs: Self) -> usize {
        self.0 - rhs.0
    }
}

impl AddAssign<usize> for DisplayCol {
    #[inline]
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl SubAssign<usize> for DisplayCol {
    #[inline]
    fn sub_assign(&mut self, rhs: usize) {
        self.0 -= rhs;
    }
}

impl PartialEq<usize> for DisplayCol {
    #[inline]
    fn eq(&self, other: &usize) -> bool {
        self.0 == *other
    }
}

impl PartialOrd<usize> for DisplayCol {
    #[inline]
    fn partial_cmp(&self, other: &usize) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_idx_to_display_col_ascii() {
        let text = "hello";
        let idx = CharIdx(3);
        let col = idx.to_display_col(text, 4);
        assert_eq!(col, DisplayCol(3));
    }

    #[test]
    fn char_idx_to_display_col_wide() {
        let text = "a世b";
        // char 0 = 'a' → display 0
        // char 1 = '世' → display 1
        // char 2 = 'b' → display 3 (after 世 which is width 2)
        let col = CharIdx(2).to_display_col(text, 4);
        assert_eq!(col, DisplayCol(3));
    }

    #[test]
    fn display_col_to_char_idx_roundtrip() {
        let text = "a世b";
        let original = CharIdx(2);
        let display = original.to_display_col(text, 4);
        let back = display.to_char_idx(text, 4);
        assert_eq!(back, original);
    }

    #[test]
    fn display_col_arithmetic() {
        let col = DisplayCol(10);
        assert_eq!((col + 5).as_usize(), 15);
        assert_eq!((col - 3).as_usize(), 7);
        assert_eq!(col.saturating_sub(20), DisplayCol(0));
    }

    #[test]
    fn char_idx_arithmetic() {
        let idx = CharIdx(5);
        assert_eq!((idx + 3).as_usize(), 8);
        assert_eq!((idx - 2).as_usize(), 3);
    }

    #[test]
    fn cannot_accidentally_mix_types() {
        // This test documents the type safety: you can't pass a DisplayCol
        // where a CharIdx is expected (or vice versa) without explicit
        // conversion. The compiler enforces this.
        let _char: CharIdx = CharIdx(5);
        let _disp: DisplayCol = DisplayCol(5);
        // These would NOT compile:
        // let _bad: CharIdx = DisplayCol(5);  // type mismatch
        // let _bad: DisplayCol = CharIdx(5);  // type mismatch
    }
}
