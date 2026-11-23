//! Paragraph motions: {, }

use super::Motions;
use crate::buffer::Buffer;
use crate::unicode::GraphemeCol;

impl Motions {
    /// `}`: forward to the next empty line, `count` times. With no empty line left
    /// it goes to the last character of the buffer (and the operator range then
    /// includes it, see [`Self::paragraph_end_is_inclusive`]); asking for more
    /// paragraphs than there are fails and leaves the cursor alone.
    pub fn paragraph_forward(buffer: &mut Buffer, count: usize) -> bool {
        Self::find_paragraph(buffer, count, true)
    }

    /// `{`: backward to the previous empty line, `count` times; the first line
    /// when there is none.
    pub fn paragraph_backward(buffer: &mut Buffer, count: usize) -> bool {
        Self::find_paragraph(buffer, count, false)
    }

    /// Whether a `}` that has just moved the cursor ended on the last character of
    /// the buffer, which an operator takes along (the motion is inclusive there).
    pub fn paragraph_end_is_inclusive(buffer: &Buffer) -> bool {
        let line = buffer.cursor().line();
        line + 1 >= buffer.line_count() && buffer.line_len(line) > 0
    }

    /// Vim's `findpar`: only empty lines (or a form feed) separate paragraphs.
    fn find_paragraph(buffer: &mut Buffer, count: usize, forward: bool) -> bool {
        let last = buffer.line_count().saturating_sub(1);
        let separates = |buffer: &Buffer, line: usize| {
            buffer.line_len(line) == 0
                || buffer
                    .line_text(line)
                    .is_some_and(|text| text.starts_with('\x0c'))
        };
        let mut line = buffer.cursor().line();
        for remaining in (0..count).rev() {
            let mut skipped_text = false;
            let mut first = true;
            loop {
                if buffer.line_len(line) != 0 {
                    skipped_text = true;
                }
                if !first && skipped_text && separates(buffer, line) {
                    break;
                }
                let next = if forward {
                    (line < last).then_some(line + 1)
                } else {
                    line.checked_sub(1)
                };
                match next {
                    Some(next) => line = next,
                    // Out of lines: fine for the last paragraph asked for.
                    None if remaining == 0 => break,
                    None => return false,
                }
                first = false;
            }
        }
        if forward && line == last {
            let len = buffer.line_index(line).grapheme_count();
            buffer
                .cursor_mut()
                .set_position(line, GraphemeCol(len.saturating_sub(1)));
        } else {
            buffer.cursor_mut().set_position(line, GraphemeCol::ZERO);
        }
        true
    }
}
