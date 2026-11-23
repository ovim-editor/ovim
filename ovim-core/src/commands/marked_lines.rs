//! The lines `:g` still has to visit, followed through the edits each visit
//! makes (vim marks the lines; the mark moves with its line and is lost with
//! it).
//!
//! A line is its start, a char offset into the rope. The edits a command made
//! are replayed over the starts in order, using the buffer's edit log:
//!
//! - text inserted before a start moves it; text inserted exactly at a start
//!   moves it only when the text ends with a newline (new lines went in above
//!   it, as `P` and `O` do) and otherwise stays in the line (`I`, `cc`);
//! - a deletion that ends before a start moves it back;
//! - a deletion that swallows a start loses the line, and so does one that
//!   begins exactly there and ends with a newline (`dd`): the whole line went;
//! - a start left in the middle of a line is a line that was joined into the
//!   one before (`J`); [`MarkedLines::next`] skips it.

use std::collections::VecDeque;

use ropey::Rope;

use crate::edit::Edit;

pub(super) struct MarkedLines {
    /// Line starts in ascending order, as `actual - shift`.
    starts: VecDeque<isize>,
    /// Added to every entry of `starts`. Most edits happen before all the
    /// lines still to visit, so moving them all is one addition.
    shift: isize,
}

impl MarkedLines {
    /// Mark the lines (0-based, ascending) of `rope`.
    pub(super) fn new(rope: &Rope, lines: &[usize]) -> Self {
        Self {
            starts: lines
                .iter()
                .map(|&line| rope.line_to_char(line) as isize)
                .collect(),
            shift: 0,
        }
    }

    /// The next marked line (0-based) that is still a line of its own.
    pub(super) fn next(&mut self, rope: &Rope) -> Option<usize> {
        while let Some(stored) = self.starts.pop_front() {
            let start = stored + self.shift;
            if start < 0 || start as usize > rope.len_chars() {
                continue;
            }
            let start = start as usize;
            if start == 0 || rope.char(start - 1) == '\n' {
                return Some(rope.char_to_line(start));
            }
        }
        None
    }

    /// Follow the edits a command made.
    pub(super) fn follow(&mut self, edits: &[&Edit]) {
        for edit in edits {
            match edit {
                Edit::Insert { offset, text } => self.insert(*offset, text),
                Edit::Delete { offset, text } => self.delete(*offset, text),
            }
        }
    }

    /// The edits are unknown (the log lost them): assume they all happened
    /// before the lines still to visit and moved them by `delta` chars.
    pub(super) fn follow_unknown(&mut self, delta: isize) {
        self.shift += delta;
    }

    /// How many entries satisfy `before`, which must hold for a prefix.
    fn count(&self, before: impl Fn(isize) -> bool) -> usize {
        self.starts
            .partition_point(|&stored| before(stored + self.shift))
    }

    /// Move the entries from `index` on by `delta`, touching the shorter side.
    fn move_from(&mut self, index: usize, delta: isize) {
        let len = self.starts.len();
        if index >= len {
            return;
        }
        if len - index <= index {
            for stored in self.starts.iter_mut().skip(index) {
                *stored += delta;
            }
        } else {
            self.shift += delta;
            for stored in self.starts.iter_mut().take(index) {
                *stored -= delta;
            }
        }
    }

    fn insert(&mut self, offset: usize, text: &str) {
        let offset = offset as isize;
        let length = text.chars().count() as isize;
        let unmoved = if text.ends_with('\n') {
            self.count(|start| start < offset)
        } else {
            self.count(|start| start <= offset)
        };
        self.move_from(unmoved, length);
    }

    fn delete(&mut self, offset: usize, text: &str) {
        let offset = offset as isize;
        let length = text.chars().count() as isize;
        if length == 0 {
            return;
        }
        let end = offset + length;
        let whole_lines = text.ends_with('\n');
        let kept = self.count(|start| start < offset || (start == offset && !whole_lines));
        let moved = self.count(|start| start < end);
        self.starts.drain(kept..moved);
        self.move_from(kept, -length);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(offset: usize, text: &str) -> Edit {
        Edit::Insert {
            offset,
            text: text.to_string(),
        }
    }

    fn delete(offset: usize, text: &str) -> Edit {
        Edit::Delete {
            offset,
            text: text.to_string(),
        }
    }

    /// Lines 0, 2 and 4 of "a\nb\nc\nd\ne\n" are at offsets 0, 4 and 8.
    fn marked() -> (Rope, MarkedLines) {
        let rope = Rope::from_str("a\nb\nc\nd\ne\n");
        let marked = MarkedLines::new(&rope, &[0, 2, 4]);
        (rope, marked)
    }

    fn starts(marked: &MarkedLines) -> Vec<isize> {
        marked.starts.iter().map(|s| s + marked.shift).collect()
    }

    #[test]
    fn lines_inserted_above_push_the_marks_down() {
        let (_, mut marked) = marked();
        // `P` of a line at the start of line 2 ("c").
        marked.follow(&[&insert(4, "x\n")]);
        assert_eq!(starts(&marked), vec![0, 6, 10]);
    }

    #[test]
    fn text_inserted_at_a_line_start_stays_in_that_line() {
        let (_, mut marked) = marked();
        // `I` typing "xy" on line 2.
        marked.follow(&[&insert(4, "xy")]);
        assert_eq!(starts(&marked), vec![0, 4, 10]);
    }

    #[test]
    fn deleting_a_marked_line_drops_its_mark_only() {
        let (_, mut marked) = marked();
        // `dd` on line 2 ("c\n").
        marked.follow(&[&delete(4, "c\n")]);
        assert_eq!(starts(&marked), vec![0, 6]);
    }

    #[test]
    fn emptying_a_marked_line_keeps_its_mark() {
        let (_, mut marked) = marked();
        // `cc` deletes the text of line 2, then types.
        marked.follow(&[&delete(4, "c"), &insert(4, "new")]);
        assert_eq!(starts(&marked), vec![0, 4, 10]);
    }

    #[test]
    fn deleting_across_marks_drops_them() {
        let (_, mut marked) = marked();
        // Lines 1 to 3 deleted ("b\nc\nd\n"): the mark on line 2 is inside.
        marked.follow(&[&delete(2, "b\nc\nd\n")]);
        assert_eq!(starts(&marked), vec![0, 2]);
    }

    #[test]
    fn edits_after_every_mark_leave_them_alone() {
        let (_, mut marked) = marked();
        // `Gp`: a line appended at the end.
        marked.follow(&[&insert(10, "x\n")]);
        assert_eq!(starts(&marked), vec![0, 4, 8]);
    }

    #[test]
    fn a_joined_line_is_skipped() {
        let (rope, mut marked) = marked();
        // `J` on line 1 joins line 2 into it: "b c" at offset 2.
        let rope = {
            let mut rope = rope;
            rope.remove(3..4);
            rope.insert(3, " ");
            rope
        };
        marked.follow(&[&delete(3, "\n"), &insert(3, " ")]);
        assert_eq!(marked.next(&rope), Some(0));
        // The mark of line 2 points into the middle of "b c" now and is
        // skipped; line 4 ("e") is line 3 of the new text.
        assert_eq!(marked.next(&rope), Some(3));
        assert_eq!(marked.next(&rope), None);
    }

    #[test]
    fn unknown_edits_shift_every_mark() {
        let (_, mut marked) = marked();
        marked.follow_unknown(3);
        assert_eq!(starts(&marked), vec![3, 7, 11]);
    }
}
