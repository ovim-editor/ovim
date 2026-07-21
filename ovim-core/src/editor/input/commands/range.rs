use crate::editor::Editor;

/// Parse an Ex command range (for example `1,5`, `%`, `.`, or `'a,'b`).
///
/// The returned line indexes are zero-based and inclusive.
pub fn parse_range(editor: &Editor, range: &str) -> Option<(usize, usize)> {
    parse_range_internal(editor, range).ok()
}

#[derive(Debug, Clone, Copy)]
enum ParseRangeError {
    MarkNotSet,
    InvalidRange,
}

fn parse_range_internal(editor: &Editor, range: &str) -> Result<(usize, usize), ParseRangeError> {
    let range = range.trim();

    if range.is_empty() {
        let cursor_line = editor.buffer().cursor().line();
        return Ok((cursor_line, cursor_line));
    }

    if range == "%" {
        if editor.buffer().line_count() == 0 {
            return Err(ParseRangeError::InvalidRange);
        }
        return Ok((0, editor.buffer().line_count().saturating_sub(1)));
    }

    if range == "'<,'>" || range.contains("'<") {
        if let Some(((start_line, _), (end_line, _))) = editor.visual_selection() {
            return Ok((start_line, end_line));
        }
        return Err(ParseRangeError::InvalidRange);
    }

    if let Some(comma_index) = range.find(',') {
        let start = parse_range_endpoint_internal(editor, range[..comma_index].trim())?;
        let end = parse_range_endpoint_internal(editor, range[comma_index + 1..].trim())?;
        return Ok((start.min(end), start.max(end)));
    }

    let line = parse_range_endpoint_internal(editor, range)?;
    Ok((line, line))
}

pub(super) fn parse_range_with_status(
    editor: &mut Editor,
    range: &str,
    invalid_status: Option<&str>,
) -> Option<(usize, usize)> {
    match parse_range_internal(editor, range) {
        Ok(range) => Some(range),
        Err(ParseRangeError::MarkNotSet) => {
            editor.set_status_message("E20: Mark not set");
            None
        }
        Err(ParseRangeError::InvalidRange) => {
            if let Some(status) = invalid_status {
                editor.set_status_message(status);
            }
            None
        }
    }
}

pub(super) fn parse_range_endpoint_with_status(
    editor: &mut Editor,
    endpoint: &str,
    invalid_status: Option<&str>,
) -> Option<usize> {
    match parse_range_endpoint_internal(editor, endpoint) {
        Ok(line) => Some(line),
        Err(ParseRangeError::MarkNotSet) => {
            editor.set_status_message("E20: Mark not set");
            None
        }
        Err(ParseRangeError::InvalidRange) => {
            if let Some(status) = invalid_status {
                editor.set_status_message(status);
            }
            None
        }
    }
}

fn parse_range_endpoint_internal(
    editor: &Editor,
    endpoint: &str,
) -> Result<usize, ParseRangeError> {
    let endpoint = endpoint.trim();
    let cursor_line = editor.buffer().cursor().line();
    let last_line = editor.buffer().line_count().saturating_sub(1);

    if endpoint == "." {
        return Ok(cursor_line);
    }

    if endpoint == "$" {
        return Ok(last_line);
    }

    if endpoint.starts_with('\'') && endpoint.len() == 2 {
        let mark = endpoint
            .chars()
            .nth(1)
            .ok_or(ParseRangeError::InvalidRange)?;
        return editor
            .nav
            .marks
            .get_mark(mark)
            .map(|position| position.line)
            .ok_or(ParseRangeError::MarkNotSet);
    }

    if let Some(rest) = endpoint.strip_prefix('+') {
        let offset = rest
            .parse::<usize>()
            .map_err(|_| ParseRangeError::InvalidRange)?;
        return Ok((cursor_line + offset).min(last_line));
    }
    if let Some(rest) = endpoint.strip_prefix('-') {
        let offset = rest
            .parse::<usize>()
            .map_err(|_| ParseRangeError::InvalidRange)?;
        return Ok(cursor_line.saturating_sub(offset));
    }

    if let Ok(line_number) = endpoint.parse::<usize>() {
        if line_number == 0 {
            return Ok(0);
        }
        return Ok(line_number.saturating_sub(1).min(last_line));
    }

    Err(ParseRangeError::InvalidRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_full_numeric_reversed_and_relative_ranges() {
        let mut editor = Editor::with_content("a\nb\nc\nd\n");
        editor.buffer_mut().cursor_mut().set_line(1);

        assert_eq!(parse_range(&editor, ""), Some((1, 1)));
        assert_eq!(parse_range(&editor, "%"), Some((0, 3)));
        assert_eq!(parse_range(&editor, "2,4"), Some((1, 3)));
        assert_eq!(parse_range(&editor, "4,2"), Some((1, 3)));
        assert_eq!(parse_range(&editor, "+2"), Some((3, 3)));
        assert_eq!(parse_range(&editor, "-1"), Some((0, 0)));
    }

    #[test]
    fn status_parser_distinguishes_missing_marks_from_invalid_ranges() {
        let mut editor = Editor::with_content("a\nb\nc\n");

        assert_eq!(parse_range_with_status(&mut editor, "1,'a", None), None);
        assert_eq!(editor.status_message(), "E20: Mark not set");

        editor.clear_status_message();
        assert_eq!(
            parse_range_with_status(&mut editor, "invalid", Some("E14: Invalid address")),
            None
        );
        assert_eq!(editor.status_message(), "E14: Invalid address");
    }
}
