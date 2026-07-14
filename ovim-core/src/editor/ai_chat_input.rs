use crate::display::{char_display_width, display_width};

/// Byte range occupied by one visual row of the AI chat composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatInputRow {
    pub start: usize,
    /// Start of displayed content. Separator whitespace that falls exactly at
    /// a soft-wrap boundary remains in the source range but is visually elided.
    pub visible_start: usize,
    pub end: usize,
}

/// Wrap composer text at word boundaries while preserving byte ranges for
/// cursor movement and mouse hit-testing. A word is split only when no
/// whitespace boundary fits on the current row.
pub fn wrap_chat_input_rows(text: &str, max_width: usize, tab_width: usize) -> Vec<ChatInputRow> {
    if text.is_empty() {
        return vec![ChatInputRow {
            start: 0,
            visible_start: 0,
            end: 0,
        }];
    }

    let width_limit = max_width.max(1);
    let tab_width = tab_width.max(1);
    let mut rows = Vec::new();
    let mut row_start = 0usize;

    while row_start < text.len() {
        let soft_wrapped = row_start > 0 && text.as_bytes().get(row_start - 1) != Some(&b'\n');
        let visible_start = if soft_wrapped {
            let mut visible = row_start;
            for (relative, character) in text[row_start..].char_indices() {
                if character == '\n' || !character.is_whitespace() {
                    break;
                }
                visible = row_start + relative + character.len_utf8();
            }
            visible
        } else {
            row_start
        };
        let mut row_end = visible_start;
        let mut row_width = 0usize;
        let mut last_word_boundary = None;
        let mut newline_at = None;

        for (relative, character) in text[visible_start..].char_indices() {
            let byte_index = visible_start + relative;
            if character == '\n' {
                newline_at = Some(byte_index);
                row_end = byte_index;
                break;
            }

            let character_width = if character == '\t' {
                tab_width - (row_width % tab_width)
            } else {
                char_display_width(character)
            };
            if row_end > visible_start && row_width.saturating_add(character_width) > width_limit {
                if !character.is_whitespace() {
                    if let Some(boundary) = last_word_boundary {
                        row_end = boundary;
                    }
                }
                break;
            }

            row_width = row_width.saturating_add(character_width);
            row_end = byte_index + character.len_utf8();
            if character.is_whitespace() && row_end > visible_start {
                last_word_boundary = Some(row_end);
            }
        }

        if row_end == visible_start {
            match text[visible_start..].chars().next() {
                Some('\n') => {
                    rows.push(ChatInputRow {
                        start: row_start,
                        visible_start: row_start,
                        end: row_start,
                    });
                    row_start += 1;
                    continue;
                }
                Some(character) => row_end = visible_start + character.len_utf8(),
                None if visible_start > row_start => row_end = visible_start,
                None => break,
            }
        }

        rows.push(ChatInputRow {
            start: row_start,
            visible_start,
            end: row_end,
        });
        row_start = if newline_at == Some(row_end) {
            row_end + 1
        } else {
            row_end
        };
    }

    if text.ends_with('\n') {
        rows.push(ChatInputRow {
            start: text.len(),
            visible_start: text.len(),
            end: text.len(),
        });
    }
    if rows.is_empty() {
        rows.push(ChatInputRow {
            start: 0,
            visible_start: 0,
            end: 0,
        });
    }
    rows
}

pub fn chat_input_cursor_row_col(
    text: &str,
    cursor_byte: usize,
    rows: &[ChatInputRow],
    tab_width: usize,
) -> (usize, usize) {
    let safe_cursor = cursor_byte.min(text.len());
    for (row_index, row) in rows.iter().enumerate() {
        if safe_cursor <= row.end {
            return (
                row_index,
                display_width(
                    &text[row.visible_start..safe_cursor.max(row.visible_start)],
                    tab_width,
                ),
            );
        }
    }
    let row_index = rows.len().saturating_sub(1);
    let row = rows.get(row_index).copied().unwrap_or(ChatInputRow {
        start: 0,
        visible_start: 0,
        end: 0,
    });
    (
        row_index,
        display_width(&text[row.visible_start..row.end], tab_width),
    )
}

pub fn chat_input_byte_for_display_column(
    text: &str,
    row: ChatInputRow,
    target_column: usize,
    tab_width: usize,
) -> usize {
    let mut column = 0usize;
    for (relative, character) in text[row.visible_start..row.end].char_indices() {
        let character_width = if character == '\t' {
            let tab_width = tab_width.max(1);
            tab_width - (column % tab_width)
        } else {
            char_display_width(character)
        };
        if column.saturating_add(character_width) > target_column {
            return row.visible_start + relative;
        }
        column = column.saturating_add(character_width);
    }
    row.end
}

pub fn chat_input_visible_start(
    total_rows: usize,
    cursor_row: usize,
    visible_rows: usize,
) -> usize {
    if visible_rows == 0 || total_rows <= visible_rows {
        return 0;
    }
    cursor_row
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(total_rows - visible_rows)
}

pub fn move_chat_input_cursor_vertical(
    text: &str,
    cursor_byte: usize,
    rows: &[ChatInputRow],
    direction: i8,
    tab_width: usize,
) -> Option<usize> {
    let (current_row, column) = chat_input_cursor_row_col(text, cursor_byte, rows, tab_width);
    let target_row = if direction < 0 {
        current_row.checked_sub(1)?
    } else {
        let next = current_row.saturating_add(1);
        (next < rows.len()).then_some(next)?
    };
    Some(chat_input_byte_for_display_column(
        text,
        rows[target_row],
        column,
        tab_width,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[ChatInputRow], input: &str) -> Vec<String> {
        rows.iter()
            .map(|row| input[row.start..row.end].to_string())
            .collect()
    }

    #[test]
    fn wraps_at_words_before_splitting_them() {
        let input = "alpha beta gamma";
        let rows = wrap_chat_input_rows(input, 11, 4);
        assert_eq!(text(&rows, input), vec!["alpha beta ", "gamma"]);
    }

    #[test]
    fn splits_a_word_longer_than_the_row() {
        let input = "extraordinary";
        let rows = wrap_chat_input_rows(input, 5, 4);
        assert_eq!(text(&rows, input), vec!["extra", "ordin", "ary"]);
    }

    #[test]
    fn preserves_explicit_newlines_and_trailing_spaces() {
        let input = "abc \ndef";
        let rows = wrap_chat_input_rows(input, 20, 4);
        assert_eq!(text(&rows, input), vec!["abc ", "def"]);
    }

    #[test]
    fn cursor_viewport_follows_rows_beyond_the_height_cap() {
        assert_eq!(chat_input_visible_start(9, 8, 5), 4);
        assert_eq!(chat_input_visible_start(9, 2, 5), 0);
    }

    #[test]
    fn display_column_maps_back_to_utf8_byte_offset() {
        let input = "a界b";
        let row = ChatInputRow {
            start: 0,
            visible_start: 0,
            end: input.len(),
        };
        assert_eq!(chat_input_byte_for_display_column(input, row, 1, 4), 1);
        assert_eq!(chat_input_byte_for_display_column(input, row, 3, 4), 4);
    }

    #[test]
    fn elides_only_separator_space_at_an_exact_soft_wrap() {
        let input = "hello world";
        let rows = wrap_chat_input_rows(input, 5, 4);
        assert_eq!(text(&rows, input), vec!["hello", " world"]);
        assert_eq!(&input[rows[1].visible_start..rows[1].end], "world");
    }

    #[test]
    fn vertical_movement_uses_soft_wrapped_rows() {
        let input = "alpha beta gamma";
        let rows = wrap_chat_input_rows(input, 7, 4);
        let moved = move_chat_input_cursor_vertical(input, input.len(), &rows, -1, 4).unwrap();
        let (row, _) = chat_input_cursor_row_col(input, moved, &rows, 4);
        assert_eq!(row, rows.len() - 2);
    }

    #[test]
    fn trailing_separator_keeps_an_empty_cursor_row() {
        let input = "hello ";
        let rows = wrap_chat_input_rows(input, 5, 4);
        let (row, column) = chat_input_cursor_row_col(input, input.len(), &rows, 4);
        assert_eq!(row, 1);
        assert_eq!(column, 0);
    }
}
