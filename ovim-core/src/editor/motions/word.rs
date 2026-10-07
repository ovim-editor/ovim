//! Word motions: w, W, b, B, e, E, ge, gE

use super::{char_class, CharClass, Motions};
use crate::buffer::Buffer;
use crate::text_index::LineIndex;
use crate::unicode::GraphemeCol;

fn grapheme_char(index: &LineIndex, col: usize) -> Option<char> {
    index.grapheme_first_char(GraphemeCol(col))
}

fn class_at(index: &LineIndex, col: usize) -> Option<CharClass> {
    grapheme_char(index, col).map(char_class)
}

fn is_whitespace_at(index: &LineIndex, col: usize) -> bool {
    grapheme_char(index, col).is_some_and(char::is_whitespace)
}

fn skip_whitespace_forward(index: &LineIndex, start: usize) -> usize {
    index
        .graphemes_from(GraphemeCol(start))
        .find(|grapheme| {
            !grapheme
                .text
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        })
        .map(|grapheme| grapheme.grapheme_col.0)
        .unwrap_or_else(|| index.grapheme_count())
}

impl Motions {
    pub fn word_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_forward_once(buffer, false);
        }
    }

    pub fn word_forward_big(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_forward_once(buffer, true);
        }
    }

    fn word_forward_once(buffer: &mut Buffer, big_word: bool) {
        let line_idx = buffer.cursor().line();
        let col = buffer.cursor().col().0;
        if line_idx >= buffer.line_count() {
            return;
        }

        let index = buffer.line_index(line_idx);
        let line_len = index.grapheme_count();
        if col >= line_len {
            if let Some((next_line, next_col)) = Self::find_next_word_start(buffer, line_idx + 1) {
                buffer.cursor_mut().set_position(next_line, next_col);
            }
            return;
        }

        let mut new_col = col;
        if big_word {
            if !is_whitespace_at(&index, new_col) {
                new_col += 1;
                for grapheme in index.graphemes_from(GraphemeCol(new_col)) {
                    if grapheme
                        .text
                        .chars()
                        .next()
                        .is_some_and(char::is_whitespace)
                    {
                        break;
                    }
                    new_col = grapheme.grapheme_col.0 + 1;
                }
            }
        } else {
            match class_at(&index, new_col).expect("cursor is within the indexed line") {
                CharClass::Cjk => new_col += 1,
                CharClass::Word => {
                    new_col += 1;
                    for grapheme in index.graphemes_from(GraphemeCol(new_col)) {
                        if grapheme.text.chars().next().map(char_class) != Some(CharClass::Word) {
                            break;
                        }
                        new_col = grapheme.grapheme_col.0 + 1;
                    }
                }
                CharClass::Punctuation => {
                    new_col += 1;
                    for grapheme in index.graphemes_from(GraphemeCol(new_col)) {
                        if grapheme.text.chars().next().map(char_class)
                            != Some(CharClass::Punctuation)
                        {
                            break;
                        }
                        new_col = grapheme.grapheme_col.0 + 1;
                    }
                }
                CharClass::Whitespace => {}
            }
        }
        new_col = skip_whitespace_forward(&index, new_col);

        if new_col >= line_len {
            if let Some((next_line, next_col)) = Self::find_next_word_start(buffer, line_idx + 1) {
                buffer.cursor_mut().set_position(next_line, next_col);
            }
        } else {
            buffer.cursor_mut().set_col(GraphemeCol(new_col));
        }
    }

    /// Empty lines are word boundaries; whitespace-only lines are skipped.
    pub(super) fn find_next_word_start(
        buffer: &Buffer,
        start_line: usize,
    ) -> Option<(usize, GraphemeCol)> {
        for line_idx in start_line..buffer.line_count() {
            let index = buffer.line_index(line_idx);
            let line_len = index.grapheme_count();
            if line_len == 0 {
                return Some((line_idx, GraphemeCol::ZERO));
            }
            if let Some(grapheme) = index.graphemes_from(GraphemeCol::ZERO).find(|grapheme| {
                !grapheme
                    .text
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
            }) {
                return Some((line_idx, grapheme.grapheme_col));
            }
        }
        None
    }

    pub fn word_backward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_backward_once(buffer, false);
        }
    }

    pub fn word_backward_big(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_backward_once(buffer, true);
        }
    }

    fn word_backward_once(buffer: &mut Buffer, big_word: bool) {
        let mut line_idx = buffer.cursor().line();
        let mut col = buffer.cursor().col().0;
        if line_idx >= buffer.line_count() {
            return;
        }

        if col == 0 {
            if line_idx == 0 {
                return;
            }
            line_idx -= 1;
            col = buffer.line_index(line_idx).grapheme_count();
            if col == 0 {
                buffer
                    .cursor_mut()
                    .set_position(line_idx, GraphemeCol::ZERO);
                return;
            }
        }

        loop {
            let index = buffer.line_index(line_idx);
            let mut new_col = col.min(index.grapheme_count());
            while new_col > 0 && is_whitespace_at(&index, new_col - 1) {
                new_col -= 1;
            }

            if new_col == 0 {
                if line_idx == 0 {
                    buffer
                        .cursor_mut()
                        .set_position(line_idx, GraphemeCol::ZERO);
                    return;
                }
                line_idx -= 1;
                let previous_len = buffer.line_index(line_idx).grapheme_count();
                if previous_len == 0 {
                    buffer
                        .cursor_mut()
                        .set_position(line_idx, GraphemeCol::ZERO);
                    return;
                }
                col = previous_len;
                continue;
            }

            if big_word {
                while new_col > 0 && !is_whitespace_at(&index, new_col - 1) {
                    new_col -= 1;
                }
            } else {
                match class_at(&index, new_col - 1).expect("cursor is within the indexed line") {
                    CharClass::Cjk => new_col -= 1,
                    CharClass::Word => {
                        while new_col > 0 && class_at(&index, new_col - 1) == Some(CharClass::Word)
                        {
                            new_col -= 1;
                        }
                    }
                    CharClass::Punctuation => {
                        while new_col > 0
                            && class_at(&index, new_col - 1) == Some(CharClass::Punctuation)
                        {
                            new_col -= 1;
                        }
                    }
                    CharClass::Whitespace => {}
                }
            }

            buffer
                .cursor_mut()
                .set_position(line_idx, GraphemeCol(new_col));
            return;
        }
    }

    pub fn word_end_forward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_end_forward_once(buffer, false, false);
        }
    }

    pub fn word_end_forward_prefer_current(buffer: &mut Buffer, count: usize) {
        for i in 0..count {
            Self::word_end_forward_once(buffer, false, i == 0);
        }
    }

    pub fn word_end_forward_big(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_end_forward_once(buffer, true, false);
        }
    }

    pub fn word_end_forward_big_prefer_current(buffer: &mut Buffer, count: usize) {
        for i in 0..count {
            Self::word_end_forward_once(buffer, true, i == 0);
        }
    }

    fn word_end_forward_once(buffer: &mut Buffer, big_word: bool, prefer_current: bool) {
        let line_idx = buffer.cursor().line();
        let col = buffer.cursor().col().0;
        let total_lines = buffer.line_count();
        if line_idx >= total_lines {
            return;
        }

        let index = buffer.line_index(line_idx);
        let line_len = index.grapheme_count();
        if line_len == 0 {
            for next_line in line_idx + 1..total_lines {
                let next_index = buffer.line_index(next_line);
                if let Some(grapheme) = next_index
                    .graphemes_from(GraphemeCol::ZERO)
                    .find(|g| !g.text.chars().next().is_some_and(char::is_whitespace))
                {
                    buffer
                        .cursor_mut()
                        .set_position(next_line, grapheme.grapheme_col);
                    // The first character reached on the new line is a candidate end of word
                    // (a one-character word such as `}` ends where it starts).
                    Self::word_end_forward_once(buffer, big_word, true);
                    return;
                }
            }
            buffer
                .cursor_mut()
                .set_position(total_lines.saturating_sub(1), GraphemeCol::ZERO);
            return;
        }

        if col >= line_len {
            if line_idx + 1 < total_lines {
                buffer
                    .cursor_mut()
                    .set_position(line_idx + 1, GraphemeCol::ZERO);
                Self::word_end_forward_once(buffer, big_word, true);
            }
            return;
        }

        let mut idx = col;
        if is_whitespace_at(&index, idx) {
            idx = skip_whitespace_forward(&index, idx);
            if idx >= line_len {
                if line_idx + 1 < total_lines {
                    buffer
                        .cursor_mut()
                        .set_position(line_idx + 1, GraphemeCol::ZERO);
                    Self::word_end_forward_once(buffer, big_word, true);
                }
                return;
            }
        } else {
            let end_of_current = Self::word_end_at(&index, idx, big_word);
            if prefer_current || idx < end_of_current {
                buffer.cursor_mut().set_col(GraphemeCol(end_of_current));
                return;
            }
            idx += 1;
            idx = skip_whitespace_forward(&index, idx);
            if idx >= line_len {
                if line_idx + 1 < total_lines {
                    buffer
                        .cursor_mut()
                        .set_position(line_idx + 1, GraphemeCol::ZERO);
                    Self::word_end_forward_once(buffer, big_word, true);
                }
                return;
            }
        }

        buffer
            .cursor_mut()
            .set_col(GraphemeCol(Self::word_end_at(&index, idx, big_word)));
    }

    fn word_end_at(index: &LineIndex, start: usize, big_word: bool) -> usize {
        if big_word {
            let mut end = start;
            for grapheme in index.graphemes_from(GraphemeCol(start + 1)) {
                if grapheme
                    .text
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
                {
                    break;
                }
                end = grapheme.grapheme_col.0;
            }
            return end;
        }

        match class_at(index, start).expect("word start is within the indexed line") {
            CharClass::Cjk => start,
            CharClass::Word | CharClass::Punctuation => {
                let class = class_at(index, start).expect("word start is within the indexed line");
                let mut end = start;
                for grapheme in index.graphemes_from(GraphemeCol(start + 1)) {
                    if grapheme.text.chars().next().map(char_class) != Some(class) {
                        break;
                    }
                    end = grapheme.grapheme_col.0;
                }
                end
            }
            CharClass::Whitespace => start,
        }
    }

    pub fn word_end_backward(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_end_backward_once(buffer, false);
        }
    }

    pub fn word_end_backward_big(buffer: &mut Buffer, count: usize) {
        for _ in 0..count {
            Self::word_end_backward_once(buffer, true);
        }
    }

    fn word_end_backward_once(buffer: &mut Buffer, big_word: bool) {
        let original_line = buffer.cursor().line();
        let original_col = buffer.cursor().col().0;
        if original_line >= buffer.line_count() {
            return;
        }

        let original_index = buffer.line_index(original_line);
        let original_class = if original_col < original_index.grapheme_count() {
            if big_word {
                Some(if is_whitespace_at(&original_index, original_col) {
                    CharClass::Whitespace
                } else {
                    CharClass::Word
                })
            } else {
                class_at(&original_index, original_col)
            }
        } else {
            None
        };

        let mut line_idx = original_line;
        let mut col = original_col;
        if col == 0 {
            if line_idx == 0 {
                return;
            }
            line_idx -= 1;
            col = buffer
                .line_index(line_idx)
                .grapheme_count()
                .saturating_sub(1);
        } else {
            col -= 1;
        }

        (line_idx, col) = Self::skip_whitespace_backward(buffer, line_idx, col);
        let index = buffer.line_index(line_idx);
        if index.grapheme_count() == 0 {
            buffer
                .cursor_mut()
                .set_position(line_idx, GraphemeCol::ZERO);
            return;
        }

        let current_class = if big_word {
            CharClass::Word
        } else {
            class_at(&index, col).expect("skip returned a grapheme")
        };
        let crossed_boundary = line_idx != original_line
            || original_class != Some(current_class)
            || col < original_col.saturating_sub(1);
        if crossed_boundary {
            buffer.cursor_mut().set_position(line_idx, GraphemeCol(col));
            return;
        }

        if big_word {
            while col > 0 && !is_whitespace_at(&index, col - 1) {
                col -= 1;
            }
        } else {
            while col > 0 && class_at(&index, col - 1) == Some(current_class) {
                col -= 1;
            }
        }

        if col == 0 {
            if line_idx == 0 {
                return;
            }
            line_idx -= 1;
            col = buffer
                .line_index(line_idx)
                .grapheme_count()
                .saturating_sub(1);
        } else {
            col -= 1;
        }
        let (line_idx, col) = Self::skip_whitespace_backward(buffer, line_idx, col);
        buffer.cursor_mut().set_position(line_idx, GraphemeCol(col));
    }

    fn skip_whitespace_backward(
        buffer: &Buffer,
        mut line_idx: usize,
        mut col: usize,
    ) -> (usize, usize) {
        loop {
            let index = buffer.line_index(line_idx);
            let line_len = index.grapheme_count();
            if line_len == 0 {
                if line_idx == 0 {
                    return (0, 0);
                }
                line_idx -= 1;
                col = buffer
                    .line_index(line_idx)
                    .grapheme_count()
                    .saturating_sub(1);
                continue;
            }

            col = col.min(line_len - 1);
            while col > 0 && is_whitespace_at(&index, col) {
                col -= 1;
            }
            if !is_whitespace_at(&index, col) {
                return (line_idx, col);
            }
            if line_idx == 0 {
                return (0, 0);
            }
            line_idx -= 1;
            col = buffer
                .line_index(line_idx)
                .grapheme_count()
                .saturating_sub(1);
        }
    }
}
