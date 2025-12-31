use crate::buffer::Buffer;
use regex::{Regex, RegexBuilder};

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
}

impl Search {
    /// Creates a new search with a pattern
    pub fn new(pattern: String, forward: bool) -> Self {
        let regex = Regex::new(&pattern).ok();
        Self {
            pattern,
            regex,
            forward,
            last_match: None,
        }
    }

    /// Creates a new search with case sensitivity options
    pub fn new_with_options(pattern: String, forward: bool, ignorecase: bool, smartcase: bool) -> Self {
        // Determine if we should be case-insensitive
        let case_insensitive = if ignorecase {
            // If smartcase is on and pattern has uppercase, be case-sensitive
            if smartcase && pattern.chars().any(|c| c.is_uppercase()) {
                false
            } else {
                true
            }
        } else {
            false
        };

        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(case_insensitive)
            .build()
            .ok();

        Self {
            pattern,
            regex,
            forward,
            last_match: None,
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

    /// Finds the next match starting from the given position
    /// Returns (line, col, match_text) if found
    pub fn find_next(
        &mut self,
        buffer: &Buffer,
        from_line: usize,
        from_col: usize,
    ) -> Option<(usize, usize, String)> {
        let regex = self.regex.as_ref()?;
        let forward = self.forward;

        let result = if forward {
            self.find_forward(buffer, regex, from_line, from_col)
        } else {
            self.find_backward(buffer, regex, from_line, from_col)
        };

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
        from_col: usize,
    ) -> Option<(usize, usize, String)> {
        let line_count = buffer.line_count();

        // Start from the current position
        for line_idx in from_line..line_count {
            if let Some(line_text) = buffer.line(line_idx) {
                let search_from = if line_idx == from_line { from_col } else { 0 };

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
                    return Some((line_idx, col, match_text));
                }
            }
        }

        // Wrap around to beginning
        for line_idx in 0..from_line {
            if let Some(line_text) = buffer.line(line_idx) {
                if let Some(mat) = regex.find(&line_text) {
                    let col = line_text[..mat.start()].chars().count();
                    let match_text = mat.as_str().to_string();
                    return Some((line_idx, col, match_text));
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
        from_col: usize,
    ) -> Option<(usize, usize, String)> {
        // Search backward from current position
        // First, search the current line up to from_col
        if let Some(line_text) = buffer.line(from_line) {
            // Use character-based slicing to avoid UTF-8 boundary panics
            let search_text: String = line_text.chars().take(from_col).collect();
            if let Some(mat) = regex.find_iter(&search_text).last() {
                let col = search_text[..mat.start()].chars().count();
                let match_text = mat.as_str().to_string();
                return Some((from_line, col, match_text));
            }
        }

        // Search previous lines
        if from_line > 0 {
            for line_idx in (0..from_line).rev() {
                if let Some(line_text) = buffer.line(line_idx) {
                    if let Some(mat) = regex.find_iter(&line_text).last() {
                        let col = line_text[..mat.start()].chars().count();
                        let match_text = mat.as_str().to_string();
                        return Some((line_idx, col, match_text));
                    }
                }
            }
        }

        // Wrap around to end
        let line_count = buffer.line_count();
        for line_idx in (from_line + 1..line_count).rev() {
            if let Some(line_text) = buffer.line(line_idx) {
                if let Some(mat) = regex.find_iter(&line_text).last() {
                    let col = line_text[..mat.start()].chars().count();
                    let match_text = mat.as_str().to_string();
                    return Some((line_idx, col, match_text));
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

        if let Some(ref regex) = self.regex {
            for mat in regex.find_iter(line_text) {
                let start_col = line_text[..mat.start()].chars().count();
                let end_col = line_text[..mat.end()].chars().count();
                matches.push((start_col, end_col));
            }
        }

        matches
    }
}
