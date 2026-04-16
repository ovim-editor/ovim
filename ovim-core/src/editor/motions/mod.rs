//! Cursor motion implementations for the editor.
//!
//! Organized by motion category:
//! - `word` — w, W, b, B, e, E, ge, gE
//! - `char_find` — f, F, t, T
//! - `bracket` — %, [{, ]}, [(, ])
//! - `line` — ^, g_, +, -, _
//! - `paragraph` — {, }
//! - `sentence` — (, )
//! - `screen` — H, M, L, Ctrl-D/U/F/B/E/Y
//! - `section` — ]], [[, ][, []
//! - `method` — ]m, [m, ]M, [M

mod bracket;
mod char_find;
mod line;
mod method;
mod paragraph;
mod screen;
mod section;
mod sentence;
mod word;

/// Character classification for word motions.
/// CJK ideographs are treated as individual words (each char = one word),
/// matching Vim's behavior.
#[derive(PartialEq, Eq, Clone, Copy)]
pub(super) enum CharClass {
    Word,        // ASCII alphanumeric + underscore
    Cjk,         // CJK ideographs, Hiragana, Katakana, Hangul, Bopomofo
    Punctuation, // everything else that's not whitespace
    Whitespace,
}

pub(super) fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if is_cjk_ideograph(c) {
        CharClass::Cjk
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

pub(super) fn is_cjk_ideograph(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF       // CJK Unified Ideographs
        | 0x3400..=0x4DBF     // CJK Extension A
        | 0x20000..=0x2A6DF   // CJK Extension B
        | 0x2A700..=0x2B73F   // CJK Extension C
        | 0x2B740..=0x2B81F   // CJK Extension D
        | 0x2B820..=0x2CEAF   // CJK Extension E
        | 0x2CEB0..=0x2EBEF   // CJK Extension F
        | 0x30000..=0x3134F   // CJK Extension G
        | 0x3100..=0x312F     // Bopomofo
        | 0x31A0..=0x31BF     // Bopomofo Extended
        | 0x3040..=0x309F     // Hiragana
        | 0x30A0..=0x30FF     // Katakana
        | 0x31F0..=0x31FF     // Katakana Phonetic Extensions
        | 0xAC00..=0xD7AF     // Hangul Syllables
        | 0x1100..=0x11FF     // Hangul Jamo
    )
}

/// Utilities for cursor motions
pub struct Motions;

impl Motions {
    /// Checks if a character is whitespace
    pub(super) fn is_whitespace(c: char) -> bool {
        c.is_whitespace()
    }

    /// Convert absolute character position to (line, char col).
    pub fn abs_pos_to_line_col(
        rope: &ropey::Rope,
        abs_pos: usize,
    ) -> (usize, crate::unicode::CharCol) {
        let line = rope.char_to_line(abs_pos.min(rope.len_chars().saturating_sub(1)));
        let line_start = rope.line_to_char(line);
        let col = abs_pos.saturating_sub(line_start);
        (line, crate::unicode::CharCol(col))
    }
}
