use super::{FileEdit, Hunk};
use anyhow::{anyhow, Result};

/// Parse a str_replace format response into structured file edits.
///
/// Expected format:
/// ```text
/// <<<<<<< SEARCH
/// old code here
/// =======
/// new code here
/// >>>>>>> REPLACE
/// ```
pub fn parse_str_replace(input: &str) -> Result<Vec<FileEdit>> {
    let mut hunks = Vec::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        // Look for SEARCH marker (trim trailing whitespace from marker lines)
        if lines[i].trim().starts_with("<<<<<<< SEARCH") {
            let search_start = i + 1;

            // Find the separator
            let sep = (search_start..lines.len())
                .find(|&j| lines[j].trim() == "=======")
                .ok_or_else(|| {
                    anyhow!(
                        "missing ======= separator after SEARCH marker at line {}",
                        i + 1
                    )
                })?;

            // Find the REPLACE marker
            let replace_end = (sep + 1..lines.len())
                .find(|&j| lines[j].trim().starts_with(">>>>>>> REPLACE"))
                .ok_or_else(|| {
                    anyhow!(
                        "missing >>>>>>> REPLACE marker after separator at line {}",
                        sep + 1
                    )
                })?;

            let search_text = join_lines(&lines[search_start..sep]);
            let replace_text = join_lines(&lines[sep + 1..replace_end]);

            if search_text.is_empty() {
                return Err(anyhow!(
                    "empty SEARCH block at line {} — use a non-empty search pattern",
                    i + 1
                ));
            }

            hunks.push(Hunk {
                search: search_text,
                replace: replace_text,
            });

            i = replace_end + 1;
        } else {
            i += 1;
        }
    }

    if hunks.is_empty() {
        return Err(anyhow!("no SEARCH/REPLACE blocks found"));
    }

    Ok(vec![FileEdit { path: None, hunks }])
}

/// Join lines with newlines, preserving a trailing newline if non-empty.
fn join_lines(lines: &[&str]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut result = lines.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_block() {
        let input = r#"Here's the fix:

<<<<<<< SEARCH
fn old() {}
=======
fn new() {}
>>>>>>> REPLACE
"#;
        let edits = parse_str_replace(input).expect("should parse");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].hunks.len(), 1);
        assert!(edits[0].hunks[0].search.contains("fn old()"));
        assert!(edits[0].hunks[0].replace.contains("fn new()"));
    }

    #[test]
    fn multiple_blocks() {
        let input = r#"<<<<<<< SEARCH
old_a()
=======
new_a()
>>>>>>> REPLACE

<<<<<<< SEARCH
old_b()
=======
new_b()
>>>>>>> REPLACE
"#;
        let edits = parse_str_replace(input).expect("should parse");
        assert_eq!(edits[0].hunks.len(), 2);
        assert!(edits[0].hunks[0].search.contains("old_a"));
        assert!(edits[0].hunks[1].search.contains("old_b"));
    }

    #[test]
    fn deletion_block() {
        let input = r#"<<<<<<< SEARCH
fn to_delete() {}
=======
>>>>>>> REPLACE
"#;
        let edits = parse_str_replace(input).expect("should parse");
        assert_eq!(edits[0].hunks.len(), 1);
        assert!(edits[0].hunks[0].search.contains("to_delete"));
        assert!(edits[0].hunks[0].replace.is_empty());
    }

    #[test]
    fn empty_search_is_error() {
        let input = r#"<<<<<<< SEARCH
=======
fn new() {}
>>>>>>> REPLACE
"#;
        assert!(parse_str_replace(input).is_err());
    }

    #[test]
    fn malformed_missing_separator() {
        let input = r#"<<<<<<< SEARCH
fn old() {}
>>>>>>> REPLACE
"#;
        assert!(parse_str_replace(input).is_err());
    }

    #[test]
    fn trailing_whitespace_on_markers() {
        let input = "<<<<<<< SEARCH   \nfn old() {}\n=======   \nfn new() {}\n>>>>>>> REPLACE   \n";
        let edits = parse_str_replace(input).expect("should parse with trailing ws");
        assert_eq!(edits[0].hunks.len(), 1);
    }

    #[test]
    fn nested_markers_in_search_text() {
        // Search text itself contains marker-like strings
        let input = r#"<<<<<<< SEARCH
let marker = "<<<<<<< SEARCH";
=======
let marker = "fixed";
>>>>>>> REPLACE
"#;
        let edits = parse_str_replace(input).expect("should parse");
        assert_eq!(edits[0].hunks.len(), 1);
        assert!(edits[0].hunks[0].search.contains("<<<<<<< SEARCH"));
    }

    #[test]
    fn multiline_search_and_replace() {
        let input = r#"<<<<<<< SEARCH
fn foo() {
    let x = 1;
    let y = 2;
    x + y
}
=======
fn foo() {
    let x = 10;
    let y = 20;
    x * y
}
>>>>>>> REPLACE
"#;
        let edits = parse_str_replace(input).expect("should parse");
        let hunk = &edits[0].hunks[0];
        assert!(hunk.search.contains("let x = 1;"));
        assert!(hunk.search.contains("let y = 2;"));
        assert!(hunk.replace.contains("let x = 10;"));
        assert!(hunk.replace.contains("x * y"));
    }

    #[test]
    fn no_blocks_found_is_error() {
        let input = "Here is some prose with no search/replace blocks.";
        assert!(parse_str_replace(input).is_err());
    }

    #[test]
    fn whitespace_only_search_parses() {
        let input = "<<<<<<< SEARCH\n   \n=======\nfn new() {}\n>>>>>>> REPLACE\n";
        // Whitespace-only search is allowed (not treated as empty)
        let edits = parse_str_replace(input).expect("should parse");
        assert_eq!(edits[0].hunks.len(), 1);
    }

    #[test]
    fn insertion_block_with_empty_replace() {
        let input = r#"<<<<<<< SEARCH
fn old() {
    // remove everything
}
=======
>>>>>>> REPLACE
"#;
        let edits = parse_str_replace(input).expect("should parse");
        assert!(edits[0].hunks[0].replace.is_empty());
        assert!(edits[0].hunks[0].search.contains("remove everything"));
    }

    #[test]
    fn separator_in_search_text() {
        // The ======= separator could appear in code (e.g., merge conflict markers)
        // The parser should use the first ======= as the separator
        let input = r#"<<<<<<< SEARCH
let divider = "=======";
=======
let divider = "---";
>>>>>>> REPLACE
"#;
        let edits = parse_str_replace(input).expect("should parse");
        assert!(edits[0].hunks[0].search.contains("======="));
        assert!(edits[0].hunks[0].replace.contains("---"));
    }
}
