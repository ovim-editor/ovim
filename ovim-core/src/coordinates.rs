//! Type-safe coordinate newtypes.
//!
//! The editor uses multiple coordinate spaces that are all represented as
//! `usize`. Mixing them is a common source of bugs (see OV-00216, OV-00217,
//! OV-00218). These newtypes provide compile-time safety with zero runtime cost.
//!
//! # Coordinate spaces
//!
//! - **[`ByteOffset`]** — a byte offset within a line. This is what tree-sitter
//!   `Point.column` expects, what `str::find()` returns, and what the syntax
//!   highlight cache uses for range boundaries. For ASCII text, bytes == chars,
//!   which is why the confusion goes unnoticed until multi-byte content appears.
//!
//! - **[`CharCol`]** — a Unicode scalar value (char) index within a line (0-based).
//!   This is what `Buffer::insert_text_at()`, `Buffer::delete_range()`, and
//!   `rope.line_to_char()` offsets work with. **Not** what `Cursor::col()` returns
//!   (that's `GraphemeCol`); convert with `grapheme_to_char_col()`.
//!
//! - **[`GraphemeCol`]** — a grapheme cluster index within a line (0-based).
//!   This is what `Cursor::col()` returns and what user-facing column numbers
//!   represent. A grapheme may span multiple chars (e.g., `é` = `e` + `\u{301}`).
//!   Convert to `CharCol` with `grapheme_to_char_col()`.
//!
//! - **[`DisplayCol`]** — a display column (0-based). Accounts for wide
//!   characters (CJK = width 2), tab expansion, and control-character caret
//!   notation. This is what the terminal sees. Convert from `CharCol` with
//!   `char_col_to_display_col()`.
//!
//! - **[`Utf16Col`]** — a UTF-16 code unit offset within a line. This is what
//!   the LSP protocol uses for `Position.character`. Surrogate pairs (emoji,
//!   some CJK) count as 2. Convert with `char_col_to_utf16()` / `utf16_to_char_col()`.
//!
//! # Conversion paths
//!
//! ```text
//! GraphemeCol ←→ CharCol ←→ ByteOffset
//!                  ↕              ↕
//!              DisplayCol    (tree-sitter)
//!                  ↕
//!              Utf16Col (LSP)
//! ```
//!
//! All non-trivial conversions require the line text (and sometimes tab width).

use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};

// ---------------------------------------------------------------------------
// Macro to reduce boilerplate — all newtypes share the same basic impls
// ---------------------------------------------------------------------------

macro_rules! coord_newtype {
    (
        $(#[$meta:meta])*
        $name:ident($inner:ty)
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        #[repr(transparent)]
        pub struct $name(pub $inner);

        impl $name {
            pub const ZERO: Self = Self(0);

            #[inline]
            pub fn as_usize(self) -> usize {
                self.0 as usize
            }

            /// Saturating subtraction.
            #[inline]
            pub fn saturating_sub(self, rhs: $inner) -> Self {
                Self(self.0.saturating_sub(rhs))
            }
        }

        impl From<$inner> for $name {
            #[inline]
            fn from(v: $inner) -> Self {
                Self(v)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl Add<$inner> for $name {
            type Output = Self;
            #[inline]
            fn add(self, rhs: $inner) -> Self {
                Self(self.0 + rhs)
            }
        }

        impl Sub<$inner> for $name {
            type Output = Self;
            #[inline]
            fn sub(self, rhs: $inner) -> Self {
                Self(self.0 - rhs)
            }
        }

        impl Sub for $name {
            type Output = $inner;
            #[inline]
            fn sub(self, rhs: Self) -> $inner {
                self.0 - rhs.0
            }
        }

        impl AddAssign<$inner> for $name {
            #[inline]
            fn add_assign(&mut self, rhs: $inner) {
                self.0 += rhs;
            }
        }

        impl SubAssign<$inner> for $name {
            #[inline]
            fn sub_assign(&mut self, rhs: $inner) {
                self.0 -= rhs;
            }
        }

        impl PartialEq<$inner> for $name {
            #[inline]
            fn eq(&self, other: &$inner) -> bool {
                self.0 == *other
            }
        }

        impl PartialOrd<$inner> for $name {
            #[inline]
            fn partial_cmp(&self, other: &$inner) -> Option<std::cmp::Ordering> {
                self.0.partial_cmp(other)
            }
        }
    };
}

// ---------------------------------------------------------------------------
// ByteOffset
// ---------------------------------------------------------------------------

coord_newtype! {
    /// A byte offset (0-based) within a line of text.
    ///
    /// This is what tree-sitter `Point.column` expects, what `str::find()`
    /// returns, and what `str::len()` measures. The syntax highlight cache
    /// stores ranges in byte offsets.
    ///
    /// For ASCII text, `ByteOffset` == `CharCol`. For multi-byte UTF-8
    /// (CJK, emoji, accented characters), they diverge.
    ///
    /// Convert from `CharCol` via `line_text[..char_col].len()` or
    /// `line_text.char_indices().nth(char_col)`.
    ByteOffset(usize)
}

impl ByteOffset {
    /// Compute the byte offset for a given char index in a line.
    #[inline]
    pub fn from_char_col(line_text: &str, char_col: CharCol) -> Self {
        Self(
            line_text
                .char_indices()
                .nth(char_col.0)
                .map(|(byte, _)| byte)
                .unwrap_or(line_text.len()),
        )
    }

    /// Convert this byte offset back to a char index in a line.
    #[inline]
    pub fn to_char_col(self, line_text: &str) -> CharCol {
        CharCol(
            line_text[..self.0.min(line_text.len())]
                .chars()
                .count(),
        )
    }
}

// ---------------------------------------------------------------------------
// CharCol (formerly CharIdx)
// ---------------------------------------------------------------------------

coord_newtype! {
    /// A character index (0-based) within a line of text.
    ///
    /// Counts Unicode scalar values (`char`). This is what rope operations
    /// like `insert_text_at(line, col, text)` and `delete_range()` expect.
    ///
    /// **Not** the same as `Cursor::col()` which returns a `GraphemeCol`.
    /// Convert with `grapheme_to_char_col()` / `char_to_grapheme_col()`.
    CharCol(usize)
}

/// Preserve the old name as an alias during migration.
pub type CharIdx = CharCol;

impl CharCol {
    /// Convert to display column given line text and tab width.
    #[inline]
    pub fn to_display_col(self, line_text: &str, tab_width: usize) -> DisplayCol {
        DisplayCol(crate::display::char_col_to_display_col(
            line_text, self.0, tab_width,
        ))
    }

    /// Convert to byte offset within the line.
    #[inline]
    pub fn to_byte_offset(self, line_text: &str) -> ByteOffset {
        ByteOffset::from_char_col(line_text, self)
    }

    /// Convert to UTF-16 code unit offset within the line.
    #[inline]
    pub fn to_utf16(self, line_text: &str) -> Utf16Col {
        Utf16Col(crate::lsp::position::char_col_to_utf16(line_text, self.0))
    }
}

// ---------------------------------------------------------------------------
// GraphemeCol
// ---------------------------------------------------------------------------

coord_newtype! {
    /// A grapheme cluster index (0-based) within a line of text.
    ///
    /// This is what `Cursor::col()` returns and what most user-facing
    /// column numbers represent. A grapheme may span multiple Unicode
    /// scalar values (e.g., `é` = `e` + combining accent = 1 grapheme,
    /// 2 chars).
    ///
    /// Convert to `CharCol` with `grapheme_to_char_col()`.
    GraphemeCol(usize)
}

impl GraphemeCol {
    /// Convert to char column using the line text.
    #[inline]
    pub fn to_char_col(self, line_text: &str) -> CharCol {
        CharCol(crate::unicode::grapheme_to_char_col(line_text, crate::unicode::GraphemeCol(self.0)))
    }
}

// ---------------------------------------------------------------------------
// DisplayCol
// ---------------------------------------------------------------------------

coord_newtype! {
    /// A display column (0-based) — the column as it appears in the terminal.
    ///
    /// Accounts for wide characters (CJK, emoji), tab expansion, and
    /// control-character caret notation. Converting to/from `CharCol`
    /// requires the line text and tab width.
    DisplayCol(usize)
}

impl DisplayCol {
    /// Convert to character index given line text and tab width.
    #[inline]
    pub fn to_char_col(self, line_text: &str, tab_width: usize) -> CharCol {
        CharCol(crate::display::display_col_to_char_col(
            line_text, self.0, tab_width,
        ))
    }
}

// ---------------------------------------------------------------------------
// Utf16Col
// ---------------------------------------------------------------------------

coord_newtype! {
    /// A UTF-16 code unit offset (0-based) within a line.
    ///
    /// This is what the LSP protocol uses for `Position.character`.
    /// Characters outside the BMP (emoji, some CJK) consume 2 UTF-16
    /// code units (a surrogate pair).
    ///
    /// Convert with `char_col_to_utf16()` / `utf16_to_char_col()`.
    Utf16Col(u32)
}

impl Utf16Col {
    /// Convert to char column using the line text.
    #[inline]
    pub fn to_char_col(self, line_text: &str) -> CharCol {
        CharCol(crate::lsp::position::utf16_to_char_col(line_text, self.0))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_col_to_display_col_ascii() {
        let text = "hello";
        let col = CharCol(3).to_display_col(text, 4);
        assert_eq!(col, DisplayCol(3));
    }

    #[test]
    fn char_col_to_display_col_wide() {
        let text = "a世b";
        // char 0 = 'a' → display 0
        // char 1 = '世' → display 1
        // char 2 = 'b' → display 3 (after 世 which is width 2)
        let col = CharCol(2).to_display_col(text, 4);
        assert_eq!(col, DisplayCol(3));
    }

    #[test]
    fn display_col_to_char_col_roundtrip() {
        let text = "a世b";
        let original = CharCol(2);
        let display = original.to_display_col(text, 4);
        let back = display.to_char_col(text, 4);
        assert_eq!(back, original);
    }

    #[test]
    fn byte_offset_from_char_col_ascii() {
        let text = "hello";
        assert_eq!(ByteOffset::from_char_col(text, CharCol(3)), ByteOffset(3));
    }

    #[test]
    fn byte_offset_from_char_col_multibyte() {
        let text = "a世b"; // 'a'=1 byte, '世'=3 bytes, 'b'=1 byte
        assert_eq!(ByteOffset::from_char_col(text, CharCol(0)), ByteOffset(0));
        assert_eq!(ByteOffset::from_char_col(text, CharCol(1)), ByteOffset(1)); // start of '世'
        assert_eq!(ByteOffset::from_char_col(text, CharCol(2)), ByteOffset(4)); // start of 'b'
    }

    #[test]
    fn byte_offset_to_char_col_roundtrip() {
        let text = "a世b";
        let char_col = CharCol(2);
        let byte = char_col.to_byte_offset(text);
        assert_eq!(byte, ByteOffset(4));
        let back = byte.to_char_col(text);
        assert_eq!(back, char_col);
    }

    #[test]
    fn byte_offset_past_end_clamps() {
        let text = "ab";
        assert_eq!(ByteOffset::from_char_col(text, CharCol(99)), ByteOffset(2));
        assert_eq!(ByteOffset(99).to_char_col(text), CharCol(2));
    }

    #[test]
    fn display_col_arithmetic() {
        let col = DisplayCol(10);
        assert_eq!((col + 5).as_usize(), 15);
        assert_eq!((col - 3).as_usize(), 7);
        assert_eq!(col.saturating_sub(20), DisplayCol(0));
    }

    #[test]
    fn char_col_arithmetic() {
        let col = CharCol(5);
        assert_eq!((col + 3).as_usize(), 8);
        assert_eq!((col - 2).as_usize(), 3);
    }

    #[test]
    fn cannot_accidentally_mix_types() {
        // The compiler enforces type safety — these are all distinct types.
        let _byte: ByteOffset = ByteOffset(5);
        let _char: CharCol = CharCol(5);
        let _grapheme: GraphemeCol = GraphemeCol(5);
        let _disp: DisplayCol = DisplayCol(5);
        let _utf16: Utf16Col = Utf16Col(5);
        // These would NOT compile:
        // let _bad: CharCol = ByteOffset(5);     // type mismatch
        // let _bad: ByteOffset = CharCol(5);     // type mismatch
        // let _bad: DisplayCol = CharCol(5);     // type mismatch
        // let _bad: GraphemeCol = CharCol(5);    // type mismatch
    }
}
