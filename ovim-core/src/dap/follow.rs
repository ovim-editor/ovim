//! Keeps breakpoints on their code while the buffer is edited.
//!
//! A breakpoint is a line number, and line numbers move. The buffer's edit log
//! says what happened to the text, so the span of a breakpoint's line is
//! carried through it the way Neovim carries a sign: lines inserted above push
//! it down, a join pulls it up onto the surviving line, and it is gone when
//! its whole line was deleted (`dd`).

use ropey::Rope;

use crate::edit::Edit;

/// Where the lines of a text went after a run of edits.
pub struct LineProjection<'a> {
    before: Rope,
    after: &'a Rope,
    edits: &'a [&'a Edit],
}

impl<'a> LineProjection<'a> {
    /// `after` is the text `edits` produced. `None` when the edits do not fit
    /// it, i.e. the log cannot be trusted.
    pub fn new(after: &'a Rope, edits: &'a [&'a Edit]) -> Option<Self> {
        let mut before = after.clone();
        for edit in edits.iter().rev() {
            match edit {
                Edit::Insert { offset, text } => {
                    let end = offset + text.chars().count();
                    if before.get_slice(*offset..end)? != text.as_str() {
                        return None;
                    }
                    before.try_remove(*offset..end).ok()?;
                }
                Edit::Delete { offset, text } => before.try_insert(*offset, text).ok()?,
            }
        }
        Some(Self {
            before,
            after,
            edits,
        })
    }

    /// The 0-based line that `line` of the original text is now, or `None`
    /// when its text was deleted. Lines past the original text stay put.
    pub fn line(&self, line: usize) -> Option<usize> {
        let Ok(start) = self.before.try_line_to_char(line) else {
            return Some(line);
        };
        let end = self
            .before
            .try_line_to_char(line + 1)
            .unwrap_or(self.before.len_chars());
        let (mut s, mut e) = (start, end);
        // Set while the line's whole text is deleted and the next edit puts
        // other text in its place: the line's index within the deleted lines.
        let mut replaced = None;
        for (i, edit) in self.edits.iter().enumerate() {
            match edit {
                Edit::Insert { offset, text } => {
                    if let Some(index) = replaced.take() {
                        (s, e) = nth_line_span(text, index, *offset)?;
                        continue;
                    }
                    let len = text.chars().count();
                    // Text typed at the start of the line pushes it along;
                    // text at its end belongs to the next line.
                    if s >= *offset {
                        s += len;
                    }
                    if e > *offset {
                        e += len;
                    }
                }
                Edit::Delete { offset, text } => {
                    let len = text.chars().count();
                    let shrink = |at: usize| {
                        if at >= offset + len {
                            at - len
                        } else if at > *offset {
                            *offset
                        } else {
                            at
                        }
                    };
                    let before_delete = s;
                    s = shrink(s);
                    e = shrink(e);
                    if e <= s && end > start {
                        // Gone, unless the text is being replaced (`J` deletes
                        // the lines it joins and types the joined line back).
                        let replaces = matches!(
                            self.edits.get(i + 1),
                            Some(Edit::Insert { offset: at, .. }) if at == offset
                        );
                        if !replaces {
                            return None;
                        }
                        let skipped = before_delete - offset;
                        replaced = Some(text.chars().take(skipped).filter(|c| *c == '\n').count());
                    }
                }
            }
        }
        Some(self.after.char_to_line(s.min(self.after.len_chars())))
    }
}

/// The char span (in the rope, from `offset`) of the `index`th line of
/// `text`, or of its last line when it has fewer. `None` for empty text.
fn nth_line_span(text: &str, index: usize, offset: usize) -> Option<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    let mut len = 0;
    for c in text.chars() {
        len += 1;
        if c == '\n' {
            spans.push((start, len));
            start = len;
        }
    }
    if start < len {
        spans.push((start, len));
    }
    let (from, to) = *spans.get(index).or(spans.last())?;
    Some((offset + from, offset + to))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies `edits` to `text` and maps the 0-based `line` of the original.
    fn project(text: &str, edits: &[Edit], line: usize) -> Option<usize> {
        let mut rope = Rope::from_str(text);
        for edit in edits {
            match edit {
                Edit::Insert { offset, text } => rope.insert(*offset, text),
                Edit::Delete { offset, text } => {
                    rope.remove(*offset..offset + text.chars().count())
                }
            }
        }
        let refs: Vec<&Edit> = edits.iter().collect();
        LineProjection::new(&rope, &refs).unwrap().line(line)
    }

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

    // Reference for the cases below: the sign (and mark) of `nvim --clean`
    // with lines "l1".."l10" and the sign on line 5. 1-based there, 0-based here.
    const TEXT: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";

    #[test]
    fn lines_inserted_above_push_the_line_down() {
        // `4Go` (nvim: sign 5 -> 6).
        assert_eq!(project(TEXT, &[insert(11, "\n")], 4), Some(5));
        // `P` of a line yanked from the top, above line 4.
        assert_eq!(project(TEXT, &[insert(9, "l1\n")], 4), Some(5));
    }

    #[test]
    fn opening_a_line_above_pushes_the_line_down_and_below_leaves_it() {
        // `5GO` (nvim: sign 5 -> 6): the newline goes in at the line's start.
        assert_eq!(project(TEXT, &[insert(12, "\n")], 4), Some(5));
        // `5Go` (nvim: sign stays on 5): the newline goes in at its end.
        assert_eq!(project(TEXT, &[insert(14, "\n")], 4), Some(4));
    }

    #[test]
    fn typing_on_the_line_keeps_it() {
        assert_eq!(project(TEXT, &[insert(12, "x")], 4), Some(4));
        assert_eq!(project(TEXT, &[insert(13, "x")], 4), Some(4));
        // `5G0i<CR>` splits at the start: nvim's sign follows the text down.
        assert_eq!(project(TEXT, &[insert(12, "\n")], 4), Some(5));
        // Splitting in the middle leaves it on the first half.
        assert_eq!(project(TEXT, &[insert(13, "\n")], 4), Some(4));
    }

    #[test]
    fn deleting_lines_above_pulls_it_up() {
        // `2Gdd` (sign 5 -> 4).
        assert_eq!(project(TEXT, &[delete(3, "l2\n")], 4), Some(3));
    }

    #[test]
    fn deleting_its_line_removes_it() {
        // `5Gdd` (nvim: sign gone).
        assert_eq!(project(TEXT, &[delete(12, "l5\n")], 4), None);
        // `3G3dd` over lines 3-5 (nvim: sign gone).
        assert_eq!(project(TEXT, &[delete(6, "l3\nl4\nl5\n")], 4), None);
    }

    #[test]
    fn the_line_after_a_deleted_one_takes_its_place() {
        // `4Gdd` (nvim: sign 5 -> 4).
        assert_eq!(project(TEXT, &[delete(9, "l4\n")], 4), Some(3));
    }

    #[test]
    fn joining_pulls_the_second_line_up_onto_the_first() {
        // `4GJ` joins l4 and l5 (nvim: sign 5 -> 4).
        assert_eq!(
            project(TEXT, &[delete(11, "\n"), insert(11, " ")], 4),
            Some(3)
        );
        // `5GJ` joins l5 and l6: the first line keeps it (nvim: stays on 5).
        assert_eq!(
            project(TEXT, &[delete(14, "\n"), insert(14, " ")], 4),
            Some(4)
        );
    }

    #[test]
    fn lines_deleted_and_typed_back_in_one_go_keep_their_breakpoints() {
        // ovim's `J` deletes both lines and inserts the joined one.
        let join_4_5 = [delete(9, "l4\nl5\n"), insert(9, "l4 l5\n")];
        assert_eq!(project(TEXT, &join_4_5, 3), Some(3));
        assert_eq!(project(TEXT, &join_4_5, 4), Some(3));
        assert_eq!(
            project(TEXT, &join_4_5, 5),
            Some(4),
            "the line below shifts up"
        );
        let join_5_6 = [delete(12, "l5\nl6\n"), insert(12, "l5 l6\n")];
        assert_eq!(project(TEXT, &join_5_6, 4), Some(4));
        assert_eq!(project(TEXT, &join_5_6, 5), Some(4));
        // `ddP`: the same line comes back.
        assert_eq!(
            project(TEXT, &[delete(12, "l5\n"), insert(12, "l5\n")], 4),
            Some(4)
        );
    }

    #[test]
    fn replacing_the_whole_text_keeps_each_line_in_place() {
        // A formatter that answers with one edit for the whole document.
        let reformatted: String = (1..=10).map(|n| format!("L{n}\n")).collect();
        let edits = [delete(0, TEXT), insert(0, &reformatted)];
        assert_eq!(project(TEXT, &edits, 4), Some(4));
        assert_eq!(project(TEXT, &edits, 9), Some(9));
        // Fewer lines than before: the extra ones pile onto the last.
        let edits = [delete(0, TEXT), insert(0, "only\none\n")];
        assert_eq!(project(TEXT, &edits, 4), Some(1));
        // Nothing typed back: gone.
        assert_eq!(project(TEXT, &[delete(0, TEXT)], 4), None);
    }

    #[test]
    fn replacing_the_text_of_a_line_keeps_the_breakpoint() {
        // `cc`: the text goes, the line stays.
        assert_eq!(
            project(TEXT, &[delete(12, "l5"), insert(12, "new")], 4),
            Some(4)
        );
    }

    #[test]
    fn edits_that_do_not_fit_the_text_are_refused() {
        let rope = Rope::from_str(TEXT);
        let edit = insert(0, "not there");
        assert!(LineProjection::new(&rope, &[&edit]).is_none());
        let past_the_end = delete(1000, "x");
        assert!(LineProjection::new(&rope, &[&past_the_end]).is_none());
    }
}
