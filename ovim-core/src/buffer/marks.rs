//! Buffer-local marks: `a`-`z` and the Visual marks `<` and `>`.
//!
//! They live on the buffer rather than the editor, so switching buffers keeps
//! them, and every edit that goes through the buffer's text primitives (typed,
//! pasted, undone or made by a command) moves them with the text the way Vim's
//! `mark_adjust` does.

use super::Buffer;
use crate::unicode::{CharCol, GraphemeCol};
use std::collections::BTreeMap;

/// Where a mark points: a line and a char column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MarkPos {
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct LocalMarks {
    marks: BTreeMap<char, MarkPos>,
}

impl LocalMarks {
    pub(super) fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    pub(super) fn set(&mut self, name: char, pos: MarkPos) {
        self.marks.insert(name, pos);
    }

    pub(super) fn get(&self, name: char) -> Option<MarkPos> {
        self.marks.get(&name).copied()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (char, MarkPos)> + '_ {
        self.marks.iter().map(|(name, pos)| (*name, *pos))
    }

    /// The marks set on `line`, to put back after an edit that rewrote it.
    pub(super) fn on_line(&self, line: usize) -> Vec<(char, MarkPos)> {
        self.iter().filter(|(_, pos)| pos.line == line).collect()
    }

    /// `breaks` line breaks were inserted. `whole_lines` is set when they
    /// make whole new lines in front of `line` (the text started at column 0
    /// and ended with a line break), which pushes the marks on `line` down as
    /// well. Splitting a line in the middle leaves its marks where they are
    /// (Vim does not adjust them for Enter either).
    pub(super) fn breaks_inserted(&mut self, line: usize, breaks: usize, whole_lines: bool) {
        for pos in self.marks.values_mut() {
            if pos.line > line || (whole_lines && pos.line == line) {
                pos.line += breaks;
            }
        }
    }

    /// The text from `start` to `end` (line, char column; `end` exclusive) was
    /// removed and `end.0 > start.0`. The lengths are those of the two lines
    /// before the edit.
    ///
    /// Marks on removed lines are deleted, except the Visual marks, which
    /// stay at the first removed line. When part of two lines is removed the
    /// rest of the last one is joined to the first and its marks follow.
    pub(super) fn lines_removed(
        &mut self,
        start: (usize, usize),
        end: (usize, usize),
        start_len: usize,
        end_len: usize,
    ) {
        let ((start_line, start_col), (end_line, end_col)) = (start, end);
        let breaks = end_line - start_line;
        // Whole lines went when the range runs from a line start to a line
        // start, or from a line end to a line end (the line break in front
        // of the last line).
        let removed = if start_col == 0 && end_col == 0 {
            Some((start_line, end_line - 1))
        } else if start_col == start_len && end_col == end_len {
            Some((start_line + 1, end_line))
        } else {
            None
        };
        self.marks.retain(|name, pos| {
            let visual = matches!(name, '<' | '>');
            match removed {
                Some((first, last)) if (first..=last).contains(&pos.line) => {
                    pos.line = first;
                    visual
                }
                Some((_, last)) if pos.line > last => {
                    pos.line -= breaks;
                    true
                }
                Some(_) => true,
                None if pos.line > start_line && pos.line < end_line => {
                    pos.line = start_line;
                    visual
                }
                None if pos.line == end_line => {
                    pos.col = start_col + pos.col.saturating_sub(end_col);
                    pos.line = start_line;
                    true
                }
                None if pos.line > end_line => {
                    pos.line -= breaks;
                    true
                }
                None => true,
            }
        });
    }
}

impl Buffer {
    /// Sets the local mark `name` (`a`-`z`, `<` or `>`) at a cursor position.
    pub fn set_local_mark(&mut self, name: char, line: usize, col: GraphemeCol) {
        let col = if line < self.rope.len_lines() {
            self.line_index(line).grapheme_to_char(col).0
        } else {
            col.0
        };
        self.local_marks.set(name, MarkPos { line, col });
    }

    /// Where the local mark `name` points, as a line and grapheme column.
    /// The line may have shrunk or vanished since; callers clamp.
    pub fn local_mark(&self, name: char) -> Option<(usize, GraphemeCol)> {
        self.local_marks.get(name).map(|pos| self.mark_cursor(pos))
    }

    /// All local marks in name order, as (name, line, grapheme column).
    pub fn local_marks(&self) -> Vec<(char, usize, GraphemeCol)> {
        self.local_marks
            .iter()
            .map(|(name, pos)| {
                let (line, col) = self.mark_cursor(pos);
                (name, line, col)
            })
            .collect()
    }

    fn mark_cursor(&self, pos: MarkPos) -> (usize, GraphemeCol) {
        if pos.line < self.rope.len_lines() {
            let col = self.line_index(pos.line).char_to_grapheme(CharCol(pos.col));
            (pos.line, col)
        } else {
            (pos.line, GraphemeCol(pos.col))
        }
    }

    /// Moves the marks for text just inserted at `(line, col)`.
    pub(super) fn adjust_marks_for_insert(
        &mut self,
        line: usize,
        col: usize,
        text: &str,
        lines_before: usize,
    ) {
        if self.local_marks.is_empty() {
            return;
        }
        let breaks = self.rope.len_lines().saturating_sub(lines_before);
        if breaks == 0 {
            return;
        }
        let whole_lines = col == 0 && text.ends_with(['\n', '\r']);
        self.local_marks.breaks_inserted(line, breaks, whole_lines);
    }

    /// Moves the marks for the chars `start_pos..end_pos` that are about to be
    /// removed from the rope.
    pub(super) fn adjust_marks_for_removal(&mut self, start_pos: usize, end_pos: usize) {
        if self.local_marks.is_empty() {
            return;
        }
        let start_line = self.rope.char_to_line(start_pos);
        let end_line = self.rope.char_to_line(end_pos);
        if start_line == end_line {
            return;
        }
        let start_col = start_pos - self.rope.line_to_char(start_line);
        let end_col = end_pos - self.rope.line_to_char(end_line);
        let start_len = self.line_len(start_line);
        let end_len = self.line_len(end_line);
        self.local_marks.lines_removed(
            (start_line, start_col),
            (end_line, end_col),
            start_len,
            end_len,
        );
    }

    /// The marks on `line` as (name, char column), for putting back after an
    /// edit that rewrites the line by deleting and re-inserting it.
    pub(crate) fn marks_on_line(&mut self, line: usize) -> Vec<(char, usize)> {
        self.local_marks
            .on_line(line)
            .into_iter()
            .map(|(name, pos)| (name, pos.col))
            .collect()
    }

    pub(crate) fn restore_marks_on_line(&mut self, line: usize, marks: Vec<(char, usize)>) {
        for (name, col) in marks {
            self.local_marks.set(name, MarkPos { line, col });
        }
    }
}
