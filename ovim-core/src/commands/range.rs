//! Evaluating parsed ranges against the editor.
//!
//! Line numbers here are 1-based like vim's: 0 means "before the first
//! line" and is only meaningful to commands that accept it (`:0r`, `:t0`).

use super::parse::{Address, Base, RangeSpec};
use crate::editor::Editor;

/// An inclusive range of 1-based line numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

impl LineRange {
    pub fn line(line: usize) -> Self {
        LineRange {
            start: line,
            end: line,
        }
    }

    /// First and last line as 0-based buffer indexes; line 0 counts as the
    /// first line.
    pub fn indexes(self) -> (usize, usize) {
        (self.start.max(1) - 1, self.end.max(1) - 1)
    }
}

/// The cursor line as a 1-based number.
pub fn cursor_line(editor: &Editor) -> usize {
    editor.buffer().cursor().line() + 1
}

pub fn last_line(editor: &Editor) -> usize {
    editor.buffer().line_count()
}

/// Evaluate `spec`. With more than two addresses the last two count (vim).
/// A backwards range is swapped: vim asks first, ovim cannot prompt.
pub fn eval_range(editor: &Editor, spec: &RangeSpec) -> Result<LineRange, String> {
    let mut cursor = cursor_line(editor);
    let mut lines = Vec::with_capacity(spec.addresses.len());
    for (index, (address, _)) in spec.addresses.iter().enumerate() {
        let line = eval_address(editor, address, cursor)?;
        lines.push(line);
        // `;` makes this address the cursor for the next one (vim moves the
        // cursor; here it only affects the rest of the range).
        if spec
            .addresses
            .get(index + 1)
            .is_some_and(|(_, semicolon)| *semicolon)
        {
            cursor = line.max(1);
        }
    }
    let end = *lines.last().expect("a range has at least one address");
    let start = if lines.len() >= 2 {
        lines[lines.len() - 2]
    } else {
        end
    };
    Ok(LineRange {
        start: start.min(end),
        end: start.max(end),
    })
}

/// Evaluate one address relative to `cursor` (1-based). The result may lie
/// past the last line; callers decide whether that is an error.
pub fn eval_address(editor: &Editor, address: &Address, cursor: usize) -> Result<usize, String> {
    let base = match &address.base {
        Base::Current => cursor,
        Base::Last => last_line(editor),
        Base::Number(line) => *line,
        Base::Mark(mark) => mark_line(editor, *mark)?,
        Base::Search { pattern, forward } => search_line(editor, pattern, *forward, cursor)?,
    };
    let line = base as isize + address.offset;
    if line < 0 {
        return Err("E16: Invalid range".to_string());
    }
    Ok(line as usize)
}

fn mark_line(editor: &Editor, mark: char) -> Result<usize, String> {
    if matches!(mark, '<' | '>') {
        if let Some(((start, _), (end, _))) = editor.visual_selection() {
            return Ok(1 + if mark == '<' { start } else { end });
        }
    }
    editor
        .nav
        .marks
        .get_mark(mark)
        .map(|position| position.line + 1)
        .ok_or_else(|| "E20: Mark not set".to_string())
}

/// `/pat/` searches forward from the line after `cursor`, `?pat?` backward
/// from the line before it; both wrap around the buffer (vim 'wrapscan').
fn search_line(
    editor: &Editor,
    pattern: &str,
    forward: bool,
    cursor: usize,
) -> Result<usize, String> {
    let pattern = if pattern.is_empty() {
        let last = editor.registers().get_last_search();
        if last.is_empty() {
            return Err("E35: No previous regular expression".to_string());
        }
        last.to_string()
    } else {
        pattern.to_string()
    };
    let regex = crate::search_pattern::compile(
        &pattern,
        crate::search_pattern::CaseOptions::of(&editor.options),
        None,
    )
    .map_err(|_| format!("E486: Pattern not found: {pattern}"))?;
    let count = last_line(editor);
    if count == 0 {
        return Err(format!("E486: Pattern not found: {pattern}"));
    }
    for step in 1..=count {
        let index = if forward {
            (cursor - 1 + step) % count
        } else {
            (cursor - 1 + count * 2 - step) % count
        };
        if editor
            .buffer()
            .line_text(index)
            .is_some_and(|text| regex.is_match(&text))
        {
            return Ok(index + 1);
        }
    }
    Err(format!("E486: Pattern not found: {pattern}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::parse::parse;

    fn range(editor: &Editor, line: &str) -> Result<LineRange, String> {
        let parsed = parse(line).unwrap();
        eval_range(editor, parsed.range.as_ref().unwrap())
    }

    #[test]
    fn evaluates_numbers_relative_and_reversed_ranges() {
        let mut editor = Editor::with_content("a\nb\nc\nd");
        editor.buffer_mut().cursor_mut().set_line(1);

        assert_eq!(range(&editor, "%d"), Ok(LineRange { start: 1, end: 4 }));
        assert_eq!(range(&editor, "2,4d"), Ok(LineRange { start: 2, end: 4 }));
        assert_eq!(range(&editor, "4,2d"), Ok(LineRange { start: 2, end: 4 }));
        assert_eq!(range(&editor, "+2d"), Ok(LineRange::line(4)));
        assert_eq!(range(&editor, ".-1d"), Ok(LineRange::line(1)));
        assert_eq!(range(&editor, "$-1d"), Ok(LineRange::line(3)));
        assert_eq!(range(&editor, "0d"), Ok(LineRange::line(0)));
    }

    #[test]
    fn searches_and_semicolons_follow_vim() {
        let mut editor = Editor::with_content("x\na\nb\na\n");
        editor.buffer_mut().cursor_mut().set_line(1);
        // Forward search starts on the line after the cursor.
        assert_eq!(range(&editor, "/a/d"), Ok(LineRange::line(4)));
        // Backward search wraps.
        assert_eq!(range(&editor, "?b?d"), Ok(LineRange::line(3)));
        // `;` searches from the first address.
        assert_eq!(range(&editor, "1;/a/d"), Ok(LineRange { start: 1, end: 2 }));
        assert_eq!(
            range(&editor, "/zz/d"),
            Err("E486: Pattern not found: zz".to_string())
        );
    }

    #[test]
    fn missing_marks_are_e20() {
        let editor = Editor::with_content("a\nb\n");
        assert_eq!(
            range(&editor, "'a,'bd"),
            Err("E20: Mark not set".to_string())
        );
    }
}
