/// Result of trying to locate a search string in buffer text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchResult {
    /// Exact byte-for-byte match.
    Exact { byte_offset: usize },
    /// Matched after normalizing trailing whitespace and line endings.
    WhitespaceNormalized { byte_offset: usize },
    /// No match found.
    NotFound(MatchError),
}

/// Diagnostic information when a match fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchError {
    /// Line number (0-indexed) of the closest match for the first line of the needle.
    pub closest_line: Option<usize>,
    /// The text of the closest matching line, truncated for display.
    pub closest_snippet: Option<String>,
    /// Human-readable error message.
    pub message: String,
}

/// Find `needle` in `haystack` using a 2-layer matching strategy:
/// 1. Exact substring match
/// 2. Whitespace-normalized match (trailing whitespace per line, CRLF → LF)
pub fn find_match(haystack: &str, needle: &str) -> MatchResult {
    // Layer 1: exact match
    if let Some(offset) = haystack.find(needle) {
        return MatchResult::Exact {
            byte_offset: offset,
        };
    }

    // Layer 2: whitespace-normalized
    let norm_haystack = normalize_whitespace(haystack);
    let norm_needle = normalize_whitespace(needle);

    if let Some(norm_offset) = norm_haystack.find(&norm_needle) {
        let original_offset =
            map_normalized_offset_to_original(haystack, &norm_haystack, norm_offset);
        return MatchResult::WhitespaceNormalized {
            byte_offset: original_offset,
        };
    }

    // Not found — build diagnostics
    let first_needle_line = needle.lines().next().unwrap_or("").trim();
    let (closest_line, closest_snippet) = if !first_needle_line.is_empty() {
        find_closest_line(haystack, first_needle_line)
    } else {
        (None, None)
    };

    MatchResult::NotFound(MatchError {
        closest_line,
        closest_snippet,
        message: format!(
            "search text not found in buffer (first line: {:?})",
            truncate(first_needle_line, 60),
        ),
    })
}

/// Normalize whitespace: trim trailing whitespace per line, convert CRLF → LF.
fn normalize_whitespace(text: &str) -> String {
    text.replace("\r\n", "\n")
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        // Preserve trailing newline if original had one
        + if text.ends_with('\n') || text.ends_with("\r\n") {
            "\n"
        } else {
            ""
        }
}

/// Map a byte offset in the normalized string back to the original string.
/// Uses a line-by-line offset mapping approach.
fn map_normalized_offset_to_original(
    original: &str,
    normalized: &str,
    norm_offset: usize,
) -> usize {
    // Find which line and column the normalized offset falls on
    let norm_prefix = &normalized[..norm_offset];
    let norm_line_num = norm_prefix.matches('\n').count();
    let norm_col = norm_prefix.len() - norm_prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);

    // Walk original lines to find the corresponding byte offset
    let mut orig_offset = 0;
    for (i, line) in original.split('\n').enumerate() {
        if i == norm_line_num {
            return orig_offset + norm_col.min(line.len());
        }
        orig_offset += line.len() + 1; // +1 for the '\n'
    }

    // Fallback: clamp to end
    original.len()
}

/// Find the closest matching line in haystack for a needle line.
fn find_closest_line(haystack: &str, needle_line: &str) -> (Option<usize>, Option<String>) {
    let needle_lower = needle_line.to_lowercase();
    let mut best_line = None;
    let mut best_snippet = None;
    let mut best_score = 0usize;

    for (i, line) in haystack.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let trimmed_lower = trimmed.to_lowercase();

        // Exact case-insensitive match
        if trimmed_lower == needle_lower {
            return (Some(i), Some(truncate(trimmed, 80).to_string()));
        }

        // Containment score
        let score = if trimmed_lower.contains(&needle_lower) {
            needle_lower.len() * 2
        } else if needle_lower.contains(&trimmed_lower) {
            trimmed_lower.len()
        } else {
            // Count matching words
            let needle_words: Vec<&str> = needle_lower.split_whitespace().collect();
            let hay_words: Vec<&str> = trimmed_lower.split_whitespace().collect();
            needle_words
                .iter()
                .filter(|w| hay_words.contains(w))
                .count()
        };

        if score > best_score {
            best_score = score;
            best_line = Some(i);
            best_snippet = Some(truncate(trimmed, 80).to_string());
        }
    }

    if best_score > 0 {
        (best_line, best_snippet)
    } else {
        (None, None)
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find a char boundary
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        let haystack = "fn main() {\n    println!(\"hello\");\n}\n";
        let needle = "    println!(\"hello\");\n";
        match find_match(haystack, needle) {
            MatchResult::Exact { byte_offset } => {
                assert_eq!(&haystack[byte_offset..byte_offset + needle.len()], needle);
            }
            other => panic!("expected Exact, got {:?}", other),
        }
    }

    #[test]
    fn trailing_whitespace_match() {
        let haystack = "fn foo() {  \n    bar();  \n}\n";
        let needle = "fn foo() {\n    bar();\n}\n";
        match find_match(haystack, needle) {
            MatchResult::WhitespaceNormalized { byte_offset } => {
                assert_eq!(byte_offset, 0);
            }
            other => panic!("expected WhitespaceNormalized, got {:?}", other),
        }
    }

    #[test]
    fn crlf_normalization() {
        let haystack = "fn foo() {\r\n    bar();\r\n}\r\n";
        let needle = "fn foo() {\n    bar();\n}\n";
        match find_match(haystack, needle) {
            MatchResult::WhitespaceNormalized { byte_offset } => {
                assert_eq!(byte_offset, 0);
            }
            other => panic!("expected WhitespaceNormalized, got {:?}", other),
        }
    }

    #[test]
    fn not_found_diagnostics() {
        let haystack = "fn main() {\n    println!(\"hello\");\n}\n";
        let needle = "fn nonexistent() {\n";
        match find_match(haystack, needle) {
            MatchResult::NotFound(err) => {
                assert!(err.message.contains("not found"));
                // Should find "fn main()" as closest since it shares "fn" prefix
                assert!(err.closest_line.is_some() || err.closest_snippet.is_some());
            }
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn empty_needle() {
        let haystack = "some text\n";
        let needle = "";
        match find_match(haystack, needle) {
            MatchResult::Exact { byte_offset } => {
                assert_eq!(byte_offset, 0);
            }
            other => panic!("expected Exact for empty needle, got {:?}", other),
        }
    }

    #[test]
    fn normalize_whitespace_preserves_trailing_newline() {
        assert_eq!(normalize_whitespace("foo  \nbar  \n"), "foo\nbar\n");
        assert_eq!(normalize_whitespace("foo\nbar"), "foo\nbar");
        assert_eq!(normalize_whitespace("foo\r\nbar\r\n"), "foo\nbar\n");
    }

    #[test]
    fn offset_mapping_simple() {
        let original = "hello  \nworld  \n";
        let normalized = normalize_whitespace(original);
        // normalized is "hello\nworld\n"
        // "world" starts at offset 6 in normalized
        let mapped = map_normalized_offset_to_original(original, &normalized, 6);
        // In original, "world" starts at offset 8 (after "hello  \n")
        assert_eq!(mapped, 8);
    }
}
