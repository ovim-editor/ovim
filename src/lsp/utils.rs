//! Utility functions for LSP module

use lsp_types::{Position, Range};

/// Computes a simple diff between old and new content for incremental sync.
///
/// Returns Some((range, new_text)) where range is the old-content region to replace
/// and new_text is the replacement. Returns None if content is identical.
///
/// Strategy:
/// - Find the first differing line from the start
/// - Find the first differing line from the end
/// - The changed region is everything between
/// - Use character-level refinement only when old and new have exactly 1 changed line
///   (the only case where character correspondence is guaranteed)
/// - For all other cases (insertion, deletion, multi-line changes with different counts),
///   use line-level granularity to avoid mismatched line comparisons
pub fn compute_simple_diff(
    old_content: &str,
    new_content: &str,
) -> Option<(lsp_types::Range, String)> {
    if old_content == new_content {
        return None;
    }

    // Use split('\n') instead of .lines() to preserve trailing newline information.
    // "foo\nbar\n" splits into ["foo", "bar", ""] which correctly represents
    // the empty line after the trailing \n (matching LSP's line model).
    let old_lines: Vec<&str> = old_content.split('\n').collect();
    let new_lines: Vec<&str> = new_content.split('\n').collect();

    // Find first differing line from start
    let mut start_line = 0;
    while start_line < old_lines.len() && start_line < new_lines.len() {
        if old_lines[start_line] != new_lines[start_line] {
            break;
        }
        start_line += 1;
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

    // Changed regions:
    // old: start_line..end_line_old
    // new: start_line..end_line_new

    let old_changed = end_line_old - start_line;
    let new_changed = end_line_new - start_line;

    // Single-line change on both sides: use character-level refinement
    if old_changed == 1 && new_changed == 1 {
        let old_line = old_lines[start_line];
        let new_line = new_lines[start_line];

        // Find common prefix
        let mut start_char = 0;
        for (old_ch, new_ch) in old_line.chars().zip(new_line.chars()) {
            if old_ch != new_ch {
                break;
            }
            start_char += 1;
        }

        // Find common suffix (from end of line)
        let old_chars: Vec<char> = old_line.chars().collect();
        let new_chars: Vec<char> = new_line.chars().collect();
        let mut end_char_old = old_chars.len();
        let mut end_char_new = new_chars.len();

        while end_char_old > start_char && end_char_new > start_char {
            if old_chars[end_char_old - 1] != new_chars[end_char_new - 1] {
                break;
            }
            end_char_old -= 1;
            end_char_new -= 1;
        }

        let start_pos = Position {
            line: start_line as u32,
            character: start_char as u32,
        };
        let end_pos = Position {
            line: start_line as u32,
            character: end_char_old as u32,
        };
        let new_text: String = new_chars[start_char..end_char_new].iter().collect();

        return Some((
            Range {
                start: start_pos,
                end: end_pos,
            },
            new_text,
        ));
    }

    // For all other cases (insertion, deletion, multi-line changes),
    // use line-level granularity: replace whole lines.
    //
    // With split('\n'), newlines are separators between lines, not part of them.
    // To replace lines [start_line, end_line_old):
    //   - If there are lines after: range (start_line, 0) to (end_line_old, 0)
    //     covers the lines and their trailing \n separators.
    //   - If at end of file: include the preceding \n by starting at
    //     (start_line - 1, len) to capture the separator before the deleted region.

    let at_end_of_file = end_line_old >= old_lines.len();

    let (start_pos, end_pos) = if !at_end_of_file {
        // Lines exist after the changed region
        let sp = Position {
            line: start_line as u32,
            character: 0,
        };
        let ep = Position {
            line: end_line_old as u32,
            character: 0,
        };
        (sp, ep)
    } else if old_changed > 0 && start_line > 0 {
        // Deleting/replacing lines at end of file — include preceding \n
        let prev_line = old_lines[start_line - 1];
        let sp = Position {
            line: (start_line - 1) as u32,
            character: prev_line.chars().count() as u32,
        };
        let ep = Position {
            line: (end_line_old - 1) as u32,
            character: old_lines[end_line_old - 1].chars().count() as u32,
        };
        (sp, ep)
    } else if start_line > 0 {
        // Insertion or replacement at end of file with preceding content.
        // Anchor at end of the previous line so we capture the \n separator.
        let prev_line = old_lines[start_line - 1];
        let anchor = Position {
            line: (start_line - 1) as u32,
            character: prev_line.chars().count() as u32,
        };
        let ep = if old_changed > 0 {
            Position {
                line: (end_line_old - 1) as u32,
                character: old_lines[end_line_old - 1].chars().count() as u32,
            }
        } else {
            anchor
        };
        (anchor, ep)
    } else {
        // Start of file (start_line == 0), changes at/from beginning
        let sp = Position {
            line: 0,
            character: 0,
        };
        let ep = if old_changed > 0 {
            Position {
                line: (end_line_old - 1) as u32,
                character: old_lines[end_line_old - 1].chars().count() as u32,
            }
        } else {
            sp
        };
        (sp, ep)
    };

    // Build replacement text
    let new_text = if !at_end_of_file {
        // Lines after the region exist: each new line gets a trailing \n
        if new_changed == 0 {
            String::new()
        } else {
            let mut result = String::new();
            for i in start_line..end_line_new {
                result.push_str(new_lines[i]);
                result.push('\n');
            }
            result
        }
    } else if start_line > 0 {
        // End-of-file with anchor at end of previous line
        if new_changed == 0 {
            // Pure deletion — range covers the \n and old content
            String::new()
        } else {
            // Prepend \n since range starts at end of previous line
            let joined = new_lines[start_line..end_line_new].join("\n");
            format!("\n{}", joined)
        }
    } else {
        // Start of file
        if new_changed == 0 {
            String::new()
        } else {
            new_lines[start_line..end_line_new].join("\n")
        }
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

/// Helper: apply an LSP text edit to a string (for testing)
#[cfg(test)]
fn apply_edit(content: &str, range: &Range, new_text: &str) -> String {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut result = String::new();

    // Flatten content into chars with positions
    let mut offset = 0;
    let mut line_offsets = Vec::new();
    for line in &lines {
        line_offsets.push(offset);
        offset += line.len() + 1; // +1 for \n
    }

    // Convert LSP positions to byte offsets
    let start_offset = if (range.start.line as usize) < lines.len() {
        let line = lines[range.start.line as usize];
        let char_offset: usize = line
            .chars()
            .take(range.start.character as usize)
            .map(|c| c.len_utf8())
            .sum();
        line_offsets[range.start.line as usize] + char_offset
    } else {
        content.len()
    };

    let end_offset = if (range.end.line as usize) < lines.len() {
        let line = lines[range.end.line as usize];
        let char_offset: usize = line
            .chars()
            .take(range.end.character as usize)
            .map(|c| c.len_utf8())
            .sum();
        line_offsets[range.end.line as usize] + char_offset
    } else {
        content.len()
    };

    result.push_str(&content[..start_offset]);
    result.push_str(new_text);
    result.push_str(&content[end_offset..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that applying the computed diff to old_content produces new_content
    fn assert_diff_correct(old: &str, new: &str) {
        let result = compute_simple_diff(old, new);
        if old == new {
            assert!(result.is_none(), "Expected None for identical content");
            return;
        }
        let (range, new_text) = result.expect("Expected Some diff");
        let applied = apply_edit(old, &range, &new_text);
        assert_eq!(
            applied, new,
            "\nDiff produced wrong result.\nOld: {:?}\nNew: {:?}\nRange: ({},{}) -> ({},{})\nText: {:?}\nGot: {:?}",
            old, new,
            range.start.line, range.start.character,
            range.end.line, range.end.character,
            new_text, applied
        );
    }

    #[test]
    fn test_no_change() {
        assert_diff_correct("Hello, world!", "Hello, world!");
    }

    #[test]
    fn test_single_line_insert_chars() {
        assert_diff_correct("Hello, world!", "Hello, beautiful world!");
    }

    #[test]
    fn test_single_line_delete_chars() {
        assert_diff_correct("Hello, beautiful world!", "Hello, world!");
    }

    #[test]
    fn test_single_line_replace() {
        assert_diff_correct("Hello, world!", "Goodbye, world!");
    }

    #[test]
    fn test_multiline_single_line_change() {
        assert_diff_correct(
            "Line 1\nLine 2\nLine 3\n",
            "Line 1\nModified Line 2\nLine 3\n",
        );
    }

    #[test]
    fn test_insert_line_middle() {
        assert_diff_correct("Line 1\nLine 3\n", "Line 1\nLine 2\nLine 3\n");
    }

    #[test]
    fn test_insert_multiple_lines() {
        assert_diff_correct("Line 1\nLine 4\n", "Line 1\nLine 2\nLine 3\nLine 4\n");
    }

    #[test]
    fn test_delete_line_middle() {
        assert_diff_correct("Line 1\nLine 2\nLine 3\n", "Line 1\nLine 3\n");
    }

    #[test]
    fn test_delete_multiple_lines() {
        assert_diff_correct("Line 1\nLine 2\nLine 3\nLine 4\n", "Line 1\nLine 4\n");
    }

    #[test]
    fn test_insert_at_start() {
        assert_diff_correct(
            "fn main() {\n    println!(\"Hello\");\n}\n",
            "// Comment\nfn main() {\n    println!(\"Hello\");\n}\n",
        );
    }

    #[test]
    fn test_insert_at_end() {
        assert_diff_correct(
            "fn main() {\n    println!(\"Hello\");\n}\n",
            "fn main() {\n    println!(\"Hello\");\n}\n// Trailing comment\n",
        );
    }

    #[test]
    fn test_replace_with_more_lines() {
        assert_diff_correct(
            "Line 1\nOld line\nLine 3\n",
            "Line 1\nNew line A\nNew line B\nLine 3\n",
        );
    }

    #[test]
    fn test_replace_with_fewer_lines() {
        assert_diff_correct(
            "Line 1\nOld A\nOld B\nLine 4\n",
            "Line 1\nNew line\nLine 4\n",
        );
    }

    #[test]
    fn test_empty_to_content() {
        assert_diff_correct("", "Hello\nWorld\n");
    }

    #[test]
    fn test_content_to_different() {
        assert_diff_correct("foo\n", "bar\n");
    }

    #[test]
    fn test_realistic_code_insert() {
        let old = "fn main() {\n    let result = compute(5);\n    println!(\"{}\", result);\n}\n";
        let new = "fn main() {\n    let x = 10;\n    let result = compute(5);\n    println!(\"{}\", result);\n}\n";
        assert_diff_correct(old, new);
    }

    #[test]
    fn test_realistic_code_delete() {
        let old = "fn main() {\n    let x = 10;\n    let result = compute(5);\n    println!(\"{}\", result);\n}\n";
        let new = "fn main() {\n    let result = compute(5);\n    println!(\"{}\", result);\n}\n";
        assert_diff_correct(old, new);
    }

    #[test]
    fn test_no_trailing_newline() {
        assert_diff_correct("foo\nbar", "foo\nbaz");
    }

    #[test]
    fn test_add_trailing_newline() {
        assert_diff_correct("foo\nbar", "foo\nbar\n");
    }

    #[test]
    fn test_remove_trailing_newline() {
        assert_diff_correct("foo\nbar\n", "foo\nbar");
    }
}
