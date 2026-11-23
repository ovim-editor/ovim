//! Sentence motions `(` and `)`, and the sentence boundaries the `is` / `as`
//! text objects share.
//!
//! `:help sentence`: a sentence ends at `.`, `!` or `?` followed by the end
//! of the line, a space or a tab; any number of `)`, `]`, `"` and `'` may
//! come between. The next sentence starts at the first non-blank after that.
//! An empty line ends a sentence too and is a stop of its own: going forward
//! the cursor stops on the first empty line of a run, going backward on the
//! last. `x.y` and `e.g.` in the middle of a word end nothing.

use super::Motions;
use crate::buffer::Buffer;
use crate::unicode::CharCol;

/// What a place where `(` or `)` stops is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopKind {
    /// The first character of a sentence.
    Sentence,
    /// The first line of a run of empty lines: where `)` stops.
    FirstEmptyLine,
    /// The last line of a run of empty lines: where `(` stops.
    LastEmptyLine,
}

/// A stop at `(line, char column)`.
pub(crate) type Stop = ((usize, usize), StopKind);

fn is_empty_line(buffer: &Buffer, line: usize) -> bool {
    buffer.line_len(line) == 0
}

/// The nearest line at or above `line` where a scan can start knowing that a
/// sentence begins there: an empty line, or the first line.
pub(crate) fn boundary_at_or_above(buffer: &Buffer, mut line: usize) -> usize {
    while line > 0 && !is_empty_line(buffer, line) {
        line -= 1;
    }
    line
}

/// The stops in lines `first..`, in order, until `enough` says so or the
/// buffer ends. `first` must be a line from [`boundary_at_or_above`].
pub(crate) fn stops_from(
    buffer: &Buffer,
    first: usize,
    mut enough: impl FnMut(&Stop) -> bool,
) -> Vec<Stop> {
    let mut stops = Vec::new();
    let mut expect_start = true;
    let mut line = first;
    let line_count = buffer.line_count();
    while line < line_count {
        if is_empty_line(buffer, line) {
            let mut last = line;
            while last + 1 < line_count && is_empty_line(buffer, last + 1) {
                last += 1;
            }
            for stop in [
                ((line, 0), StopKind::FirstEmptyLine),
                ((last, 0), StopKind::LastEmptyLine),
            ] {
                stops.push(stop);
                if enough(&stop) {
                    return stops;
                }
            }
            expect_start = true;
            line = last + 1;
            continue;
        }
        let text = buffer.line_text(line).unwrap_or_default();
        let chars: Vec<char> = text.chars().collect();
        let mut col = 0;
        while col < chars.len() {
            let c = chars[col];
            if expect_start && !c.is_whitespace() {
                expect_start = false;
                let stop = ((line, col), StopKind::Sentence);
                stops.push(stop);
                if enough(&stop) {
                    return stops;
                }
            }
            col += 1;
            if matches!(c, '.' | '!' | '?') {
                let mut after = col;
                while matches!(chars.get(after), Some(')' | ']' | '"' | '\'')) {
                    after += 1;
                }
                if matches!(chars.get(after), None | Some(' ' | '\t')) {
                    expect_start = true;
                    col = after;
                }
            }
        }
        line += 1;
    }
    stops
}

impl Motions {
    /// `)`: forward to the start of the `count`-th next sentence.
    pub fn sentence_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            let cursor = (buffer.cursor().line(), buffer.cursor_char_col().0);
            let ahead = |stop: &Stop| stop.1 != StopKind::LastEmptyLine && stop.0 > cursor;
            let first = boundary_at_or_above(buffer, cursor.0);
            let target = stops_from(buffer, first, ahead)
                .into_iter()
                .find(ahead)
                .map(|stop| stop.0);
            match target {
                Some((line, col)) => buffer.set_cursor_char_col(line, CharCol(col)),
                // No sentence follows: the last character of the buffer.
                None => {
                    let line = buffer.line_count().saturating_sub(1);
                    let col = buffer.line_len(line).saturating_sub(1);
                    buffer.set_cursor_char_col(line, CharCol(col));
                    return;
                }
            }
        }
    }

    /// `(`: backward to the start of the `count`-th previous sentence.
    pub fn sentence_backward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            let cursor = (buffer.cursor().line(), buffer.cursor_char_col().0);
            // Start far enough up to see the stop before the cursor: past the
            // empty lines the cursor may be on and the paragraph above them.
            let mut first = boundary_at_or_above(buffer, cursor.0);
            while first > 0 && is_empty_line(buffer, first - 1) {
                first -= 1;
            }
            let first = boundary_at_or_above(buffer, first.saturating_sub(1));
            let target = stops_from(buffer, first, |stop| stop.0 >= cursor)
                .into_iter()
                .rfind(|stop| stop.1 != StopKind::FirstEmptyLine && stop.0 < cursor)
                .map_or((0, 0), |stop| stop.0);
            buffer.set_cursor_char_col(target.0, CharCol(target.1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // What nvim --clean does on the same text (tools/nvim_parity.py, fixture F).
    const TEXT: &str = "First one. Second one!  Third (really?) yes.\nWraps onto\n\
        the next line. End of para.\n\n\nNew para here.  Last \"quoted.\" Done\n  \
        indented. x.y is not\n";

    fn cursor(buffer: &Buffer) -> (usize, usize) {
        (buffer.cursor().line(), buffer.cursor_char_col().0)
    }

    #[test]
    fn forward_stops_at_sentences_and_at_the_first_empty_line() {
        let mut buffer = Buffer::new_from_str(TEXT);
        let mut seen = Vec::new();
        for _ in 0..11 {
            Motions::sentence_forward(&mut buffer, 1);
            seen.push(cursor(&buffer));
        }
        assert_eq!(
            seen,
            [
                (0, 11),
                (0, 24),
                (0, 40),
                (1, 0),
                (2, 15),
                (3, 0),
                (5, 0),
                (5, 16),
                (5, 31),
                (6, 12),
                // Nothing follows: the last character.
                (6, 21),
            ]
        );
    }

    #[test]
    fn backward_stops_at_sentences_and_at_the_last_empty_line() {
        let mut buffer = Buffer::new_from_str(TEXT);
        buffer.set_cursor_char_col(6, CharCol(21));
        let mut seen = Vec::new();
        for _ in 0..11 {
            Motions::sentence_backward(&mut buffer, 1);
            seen.push(cursor(&buffer));
        }
        assert_eq!(
            seen,
            [
                (6, 12),
                (5, 31),
                (5, 16),
                (5, 0),
                (4, 0),
                (2, 15),
                (1, 0),
                (0, 40),
                (0, 24),
                (0, 11),
                (0, 0),
            ]
        );
    }

    #[test]
    fn counts_and_starting_points_inside_a_sentence() {
        let at = |line: usize, col: usize, forward: bool, count: usize| {
            let mut buffer = Buffer::new_from_str(TEXT);
            buffer.set_cursor_char_col(line, CharCol(col));
            if forward {
                Motions::sentence_forward(&mut buffer, count);
            } else {
                Motions::sentence_backward(&mut buffer, count);
            }
            cursor(&buffer)
        };
        assert_eq!(at(0, 0, true, 3), (0, 40));
        // From the middle of a sentence `(` goes to its start.
        assert_eq!(at(0, 14, false, 1), (0, 11));
        // From between two empty lines.
        assert_eq!(at(3, 0, true, 1), (5, 0));
        assert_eq!(at(4, 0, false, 1), (2, 15));
        assert_eq!(at(5, 0, false, 1), (4, 0));
        // The indent of a continuation line starts nothing.
        assert_eq!(at(6, 2, false, 1), (5, 31));
        assert_eq!(at(6, 2, false, 3), (5, 0));
    }
}
