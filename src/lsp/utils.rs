//! Utility functions for LSP module

use lsp_types::{Position, Range};

/// Computes a simple diff between old and new content for incremental sync
/// Returns Some((range, new_text)) if a single contiguous change is found,
/// or None if the change is too complex (fallback to full sync)
pub fn compute_simple_diff(
    old_content: &str,
    new_content: &str,
) -> Option<(lsp_types::Range, String)> {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    // Find first differing line from start
    let mut start_line = 0;
    while start_line < old_lines.len() && start_line < new_lines.len() {
        if old_lines[start_line] != new_lines[start_line] {
            break;
        }
        start_line += 1;
    }

    // If all old lines match prefix of new lines, it's just appending
    if start_line == old_lines.len() {
        if new_lines.len() > old_lines.len() {
            // Lines were appended
            let start_pos = Position {
                line: old_lines.len() as u32,
                character: 0,
            };
            let new_text = new_lines[old_lines.len()..].join("\n");
            let new_text = if !old_content.is_empty() {
                format!("\n{}", new_text)
            } else {
                new_text
            };

            return Some((
                Range {
                    start: start_pos,
                    end: start_pos,
                },
                new_text,
            ));
        }
        // Contents are identical
        return None;
    }

    // Find first differing line from end
    let mut end_line_old = old_lines.len();
    let mut end_line_new = new_lines.len();

    while end_line_old > start_line && end_line_new > start_line {
        if old_lines[end_line_old - 1] != new_lines[end_line_new - 1] {
            break;
        }
        end_line_old -= 1;
        end_line_new -= 1;
    }

    // Now we have a contiguous changed region:
    // old: start_line..end_line_old
    // new: start_line..end_line_new

    // Find character-level start position within the first changed line
    let start_char = if end_line_old == start_line {
        // Pure insertion: no old lines in the changed region
        // Start at beginning of the line
        0
    } else if start_line < old_lines.len() && start_line < new_lines.len() {
        let old_line = old_lines[start_line];
        let new_line = new_lines[start_line];
        let mut char_pos = 0;

        for (old_ch, new_ch) in old_line.chars().zip(new_line.chars()) {
            if old_ch != new_ch {
                break;
            }
            char_pos += 1;
        }
        char_pos
    } else {
        0
    };

    // Find character-level end position within the last changed line
    let end_char = if end_line_old > 0 && end_line_old <= old_lines.len() {
        old_lines[end_line_old - 1].chars().count()
    } else {
        0
    };

    let start_pos = Position {
        line: start_line as u32,
        character: start_char as u32,
    };

    let end_pos = Position {
        line: (end_line_old.saturating_sub(1)) as u32,
        character: end_char as u32,
    };

    // Extract the new text for the changed region
    let new_text = if start_line < new_lines.len() {
        if end_line_new > start_line {
            // Multiple lines changed
            let mut result = String::new();

            // First line: from start_char onwards
            if let Some(first_line) = new_lines.get(start_line) {
                if start_char < first_line.chars().count() {
                    result.push_str(&first_line.chars().skip(start_char).collect::<String>());
                }
            }

            // Middle lines
            for line in &new_lines[start_line + 1..end_line_new] {
                result.push('\n');
                result.push_str(line);
            }

            result
        } else {
            // Single line partial change
            new_lines[start_line]
                .chars()
                .skip(start_char)
                .collect::<String>()
        }
    } else {
        String::new()
    };

    Some((
        Range {
            start: start_pos,
            end: end_pos,
        },
        new_text,
    ))
}

/// Converts a MarkedString to plain text
pub(crate) fn marked_string_to_text(marked: lsp_types::MarkedString) -> String {
    match marked {
        lsp_types::MarkedString::String(s) => s,
        lsp_types::MarkedString::LanguageString(ls) => ls.value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_simple_diff_no_change() {
        let old = "Hello, world!";
        let new = "Hello, world!";
        let result = compute_simple_diff(old, new);
        assert!(result.is_none(), "No diff expected for identical content");
    }

    #[test]
    fn test_compute_simple_diff_single_line_insert() {
        let old = "Hello, world!";
        let new = "Hello, beautiful world!";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for inserted text");

        let (range, new_text) = result.unwrap();
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 7);
        assert_eq!(range.end.line, 0);
        assert_eq!(range.end.character, 13);
        assert_eq!(new_text, "beautiful world!");
    }

    #[test]
    fn test_compute_simple_diff_single_line_delete() {
        let old = "Hello, beautiful world!";
        let new = "Hello, world!";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for deleted text");

        let (range, new_text) = result.unwrap();
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 7);
        assert_eq!(range.end.line, 0);
        assert_eq!(range.end.character, 23);
        assert_eq!(new_text, "world!");
    }

    #[test]
    fn test_compute_simple_diff_multiline_change() {
        let old = "Line 1\nLine 2\nLine 3\n";
        let new = "Line 1\nModified Line 2\nLine 3\n";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for modified line");

        let (range, new_text) = result.unwrap();
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 0);
        assert_eq!(range.end.line, 1);
        assert_eq!(range.end.character, 6);
        assert_eq!(new_text, "Modified Line 2");
    }

    #[test]
    fn test_compute_simple_diff_insert_line() {
        let old = "Line 1\nLine 3\n";
        let new = "Line 1\nLine 2\nLine 3\n";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for inserted line");

        let (range, new_text) = result.unwrap();
        assert_eq!(range.start.line, 1);
        // The diff algorithm should include "Line 2\nLine 3" as the new text
        assert!(
            new_text.contains("Line 2") || new_text.contains("Line 3"),
            "Expected new_text to contain inserted content, got: {:?}",
            new_text
        );
    }

    #[test]
    fn test_compute_simple_diff_delete_line() {
        let old = "Line 1\nLine 2\nLine 3\n";
        let new = "Line 1\nLine 3\n";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for deleted line");

        let (_range, _new_text) = result.unwrap();
        assert_eq!(_range.start.line, 1);
    }

    #[test]
    fn test_compute_simple_diff_start_of_file() {
        let old = "fn main() {\n    println!(\"Hello\");\n}\n";
        let new = "// Comment\nfn main() {\n    println!(\"Hello\");\n}\n";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for content added at start");

        let (range, new_text) = result.unwrap();
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
        assert!(new_text.starts_with("// Comment"));
    }

    #[test]
    fn test_compute_simple_diff_end_of_file() {
        let old = "fn main() {\n    println!(\"Hello\");\n}\n";
        let new = "fn main() {\n    println!(\"Hello\");\n}\n// Trailing comment\n";
        let result = compute_simple_diff(old, new);
        assert!(result.is_some(), "Expected diff for content added at end");

        let (_range, new_text) = result.unwrap();
        assert!(new_text.contains("// Trailing comment"));
    }
}
