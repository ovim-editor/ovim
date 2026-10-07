use crate::buffer::Buffer;
use crate::unicode::{char_to_grapheme_col, grapheme_to_char_col, CharCol, GraphemeCol};
use regex::Regex;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};

type CachedLineMatches = (Weak<crate::text_index::LineIndex>, Arc<Vec<(usize, usize)>>);

/// Represents a search query with its direction
#[derive(Clone, Debug)]
pub struct Search {
    /// The search pattern (regex)
    pattern: String,
    /// Compiled regex
    regex: Option<Regex>,
    /// Search direction: true for forward (/), false for backward (?)
    forward: bool,
    /// Last match position (line, col)
    last_match: Option<(usize, usize)>,
    /// Share exact regex results across frontend projections of immutable lines.
    line_matches: Arc<Mutex<VecDeque<CachedLineMatches>>>,
}

impl Search {
    /// Creates a new search with a pattern
    pub fn new(pattern: String, forward: bool) -> Self {
        Self::new_with_options(pattern, forward, false, false)
    }

    /// Creates a new search with case sensitivity options
    pub fn new_with_options(
        pattern: String,
        forward: bool,
        ignorecase: bool,
        smartcase: bool,
    ) -> Self {
        let regex = crate::search_pattern::compile(
            &pattern,
            crate::search_pattern::CaseOptions::new(ignorecase, smartcase),
            None,
        )
        .ok();

        Self {
            pattern,
            regex,
            forward,
            last_match: None,
            line_matches: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Gets the search pattern
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Returns true if search is forward (/)
    pub fn is_forward(&self) -> bool {
        self.forward
    }

    /// Finds the next match starting from the given position.
    ///
    /// `from_col` is a **grapheme** index (matching cursor.col() semantics).
    /// The returned `col` is also a **grapheme** index suitable for cursor positioning.
    pub fn find_next(
        &mut self,
        buffer: &Buffer,
        from_line: usize,
        from_col: GraphemeCol,
    ) -> Option<(usize, usize, String)> {
        let regex = self.regex.as_ref()?;
        let forward = self.forward;

        // Convert from_col from grapheme to char for internal search
        let from_col_char = if let Some(line_text) = buffer.line_text(from_line) {
            grapheme_to_char_col(&line_text, from_col)
        } else {
            CharCol(from_col.0)
        };

        let result = if forward {
            self.find_forward(buffer, regex, from_line, from_col_char)
        } else {
            self.find_backward(buffer, regex, from_line, from_col_char)
        };

        // Convert returned col from char to grapheme
        let result = result.map(|(line, char_col, match_text)| {
            let grapheme_col = if let Some(line_text) = buffer.line_text(line) {
                char_to_grapheme_col(&line_text, char_col).0
            } else {
                char_col.0
            };
            (line, grapheme_col, match_text)
        });

        if let Some((line, col, _)) = result {
            self.last_match = Some((line, col));
        }

        result
    }

    /// Finds next match in forward direction
    fn find_forward(
        &self,
        buffer: &Buffer,
        regex: &Regex,
        from_line: usize,
        from_col: CharCol,
    ) -> Option<(usize, CharCol, String)> {
        let line_count = buffer.line_count();

        // Start from the current position
        for line_idx in from_line..line_count {
            if let Some(line_text) = buffer.line_text(line_idx) {
                let search_from = if line_idx == from_line { from_col.0 } else { 0 };

                // Convert character index to byte offset for regex.find_at()
                let search_from_bytes = line_text
                    .char_indices()
                    .nth(search_from)
                    .map(|(byte_idx, _)| byte_idx)
                    .unwrap_or(line_text.len());

                // Search in this line starting from search_from_bytes
                if let Some(mat) = regex.find_at(&line_text, search_from_bytes) {
                    let col = line_text[..mat.start()].chars().count();
                    let match_text = mat.as_str().to_string();
                    return Some((line_idx, CharCol(col), match_text));
                }
            }
        }

        // Wrap around to beginning, including the part of the starting line before
        // `from_col` (any match there lies before the cursor: `find_at` found none after).
        for line_idx in 0..=from_line.min(line_count.saturating_sub(1)) {
            if let Some(line_text) = buffer.line_text(line_idx) {
                if let Some(mat) = regex.find(&line_text) {
                    let col = line_text[..mat.start()].chars().count();
                    let match_text = mat.as_str().to_string();
                    return Some((line_idx, CharCol(col), match_text));
                }
            }
        }

        None
    }

    /// Finds next match in backward direction
    fn find_backward(
        &self,
        buffer: &Buffer,
        regex: &Regex,
        from_line: usize,
        from_col: CharCol,
    ) -> Option<(usize, CharCol, String)> {
        // Search backward from current position
        // First, search the current line for the last match that starts before
        // `from_col` (it may extend past it: the cursor can be inside a match).
        if let Some(line_text) = buffer.line_text(from_line) {
            let from_byte = line_text
                .char_indices()
                .nth(from_col.0)
                .map_or(line_text.len(), |(byte_idx, _)| byte_idx);
            if let Some(mat) = regex
                .find_iter(&line_text)
                .take_while(|mat| mat.start() < from_byte)
                .last()
            {
                let col = line_text[..mat.start()].chars().count();
                let match_text = mat.as_str().to_string();
                return Some((from_line, CharCol(col), match_text));
            }
        }

        // Search previous lines
        if from_line > 0 {
            for line_idx in (0..from_line).rev() {
                if let Some(line_text) = buffer.line_text(line_idx) {
                    if let Some(mat) = regex.find_iter(&line_text).last() {
                        let col = line_text[..mat.start()].chars().count();
                        let match_text = mat.as_str().to_string();
                        return Some((line_idx, CharCol(col), match_text));
                    }
                }
            }
        }

        // Wrap around to end, including the part of the starting line from
        // `from_col` on (the part before it had no match).
        let line_count = buffer.line_count();
        for line_idx in (from_line..line_count).rev() {
            if let Some(line_text) = buffer.line_text(line_idx) {
                if let Some(mat) = regex.find_iter(&line_text).last() {
                    let col = line_text[..mat.start()].chars().count();
                    let match_text = mat.as_str().to_string();
                    return Some((line_idx, CharCol(col), match_text));
                }
            }
        }

        None
    }

    /// Gets the last match position
    pub fn last_match(&self) -> Option<(usize, usize)> {
        self.last_match
    }

    /// Finds all matches in a given line text
    /// Returns a vector of (start_col, end_col) tuples
    pub fn find_all_in_line(&self, line_text: &str) -> Vec<(usize, usize)> {
        let mut matches = Vec::new();
        let mut previous_byte = 0;
        let mut previous_char = 0;
        if let Some(ref regex) = self.regex {
            for mat in regex.find_iter(line_text) {
                // Matches are ordered and nonoverlapping. Convert each portion
                // once instead of recounting the whole prefix for every match.
                let start_col =
                    previous_char + line_text[previous_byte..mat.start()].chars().count();
                let end_col = start_col + line_text[mat.start()..mat.end()].chars().count();
                matches.push((start_col, end_col));
                previous_byte = mat.end();
                previous_char = end_col;
            }
        }
        matches
    }
    /// Exact whole-line regex results cached by immutable text identity. This
    /// preserves anchors and context while making repeated viewport renders cheap.
    pub fn find_all_in_index(
        &self,
        index: &Arc<crate::text_index::LineIndex>,
    ) -> Arc<Vec<(usize, usize)>> {
        let mut cache = self
            .line_matches
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(position) = cache
            .iter()
            .position(|(line, _)| line.upgrade().is_some_and(|line| Arc::ptr_eq(&line, index)))
        {
            let entry = cache.remove(position).unwrap();
            let result = entry.1.clone();
            cache.push_back(entry);
            return result;
        }
        let text = index.slice_chars(0..index.len_chars());
        let matches = Arc::new(self.find_all_in_line(&text));
        cache.retain(|(line, _)| line.strong_count() > 0);
        // Retain enough distinct lines for a tall viewport and split panes;
        // a cache smaller than the viewport would thrash on every frame.
        if cache.len() == 256 {
            cache.pop_front();
        }
        cache.push_back((Arc::downgrade(index), matches.clone()));
        matches
    }
}

#[cfg(test)]
mod indexed_search_tests {
    use super::*;
    use crate::text_index::LineIndex;

    #[test]
    fn dense_unicode_matches_keep_character_offsets() {
        let search = Search::new("é".into(), true);
        assert_eq!(
            search.find_all_in_line("éé x é"),
            vec![(0, 1), (1, 2), (5, 6)]
        );
        let empty = Search::new("".into(), true);
        assert_eq!(empty.find_all_in_line("éx"), vec![(0, 0), (1, 1), (2, 2)]);
    }

    #[test]
    fn immutable_line_cache_preserves_anchors_and_reuses_results() {
        let search = Search::new("^a+$".into(), true);
        let line = LineIndex::from_text("aaaa");
        let first = search.find_all_in_index(&line);
        assert_eq!(*first, vec![(0, 4)]);
        assert!(Arc::ptr_eq(
            &first,
            &search.clone().find_all_in_index(&line)
        ));
        let edited = LineIndex::from_text("baaaa");
        assert!(search.find_all_in_index(&edited).is_empty());
        assert_eq!(*search.find_all_in_index(&line), vec![(0, 4)]);
    }
}
