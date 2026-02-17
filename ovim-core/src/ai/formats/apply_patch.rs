use super::{FileEdit, Hunk};
use anyhow::{anyhow, Result};

/// Parse an apply_patch format response into structured file edits.
///
/// Expected envelope:
/// ```text
/// *** Begin Patch
/// *** Update File: path/to/file.rs
/// @@ context @@
///  context line
/// -removed line
/// +added line
/// *** End Patch
/// ```
pub fn parse_apply_patch(input: &str) -> Result<Vec<FileEdit>> {
    // Find the envelope markers
    let begin = input
        .find("*** Begin Patch")
        .ok_or_else(|| anyhow!("missing *** Begin Patch marker"))?;
    let end = input
        .find("*** End Patch")
        .ok_or_else(|| anyhow!("missing *** End Patch marker"))?;

    if end <= begin {
        return Err(anyhow!("*** End Patch appears before *** Begin Patch"));
    }

    // Extract the patch body (after the Begin Patch line)
    let after_begin = &input[begin..end];
    let body_start = after_begin.find('\n').map(|i| begin + i + 1).unwrap_or(end);
    let body = &input[body_start..end];

    if body.trim().is_empty() {
        return Err(anyhow!("empty patch body"));
    }

    // Split into file sections at *** headers
    let mut file_edits = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_lines: Vec<&str> = Vec::new();

    for line in body.lines() {
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            // Flush previous file section
            if current_path.is_some() || !current_lines.is_empty() {
                let hunks = parse_hunks(&current_lines)?;
                if !hunks.is_empty() {
                    file_edits.push(FileEdit {
                        path: current_path.take(),
                        hunks,
                    });
                }
            }
            current_path = Some(path.trim().to_string());
            current_lines.clear();
        } else if let Some(path) = line.strip_prefix("*** Add File: ") {
            if current_path.is_some() || !current_lines.is_empty() {
                let hunks = parse_hunks(&current_lines)?;
                if !hunks.is_empty() {
                    file_edits.push(FileEdit {
                        path: current_path.take(),
                        hunks,
                    });
                }
            }
            current_path = Some(path.trim().to_string());
            current_lines.clear();
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            if current_path.is_some() || !current_lines.is_empty() {
                let hunks = parse_hunks(&current_lines)?;
                if !hunks.is_empty() {
                    file_edits.push(FileEdit {
                        path: current_path.take(),
                        hunks,
                    });
                }
            }
            // Delete file = empty hunks with path set
            file_edits.push(FileEdit {
                path: Some(path.trim().to_string()),
                hunks: vec![],
            });
            current_path = None;
            current_lines.clear();
        } else {
            current_lines.push(line);
        }
    }

    // Flush final file section
    if current_path.is_some() || !current_lines.is_empty() {
        let hunks = parse_hunks(&current_lines)?;
        if !hunks.is_empty() {
            file_edits.push(FileEdit {
                path: current_path,
                hunks,
            });
        }
    }

    if file_edits.is_empty() {
        return Err(anyhow!("no file edits found in patch"));
    }

    Ok(file_edits)
}

/// Parse hunk blocks from lines within a single file section.
/// Hunks are separated by `@@` lines.
fn parse_hunks(lines: &[&str]) -> Result<Vec<Hunk>> {
    let mut hunks = Vec::new();
    let mut hunk_lines: Vec<&str> = Vec::new();
    let mut in_hunk = false;

    for line in lines {
        if line.starts_with("@@") {
            // Flush previous hunk
            if in_hunk && !hunk_lines.is_empty() {
                hunks.push(build_hunk(&hunk_lines));
                hunk_lines.clear();
            }
            in_hunk = true;
        } else if in_hunk {
            hunk_lines.push(line);
        }
        // Lines before the first @@ are ignored (e.g., file metadata)
    }

    // Flush final hunk
    if in_hunk && !hunk_lines.is_empty() {
        hunks.push(build_hunk(&hunk_lines));
    }

    Ok(hunks)
}

/// Build a Hunk from diff lines. Prefix meanings:
/// ` ` = context (appears in both search and replace)
/// `-` = removal (search only)
/// `+` = addition (replace only)
/// No prefix = treated as context (models sometimes drop the space)
fn build_hunk(lines: &[&str]) -> Hunk {
    let mut search = String::new();
    let mut replace = String::new();

    for (i, line) in lines.iter().enumerate() {
        let is_last = i == lines.len() - 1;

        if let Some(content) = line.strip_prefix('-') {
            search.push_str(content);
            if !is_last {
                search.push('\n');
            }
        } else if let Some(content) = line.strip_prefix('+') {
            replace.push_str(content);
            if !is_last {
                replace.push('\n');
            }
        } else {
            // Context line: strip leading space if present, otherwise treat as-is
            let content = line.strip_prefix(' ').unwrap_or(line);
            search.push_str(content);
            replace.push_str(content);
            if !is_last {
                search.push('\n');
                replace.push('\n');
            }
        }
    }

    // Ensure trailing newlines are consistent
    if !search.is_empty() && !search.ends_with('\n') {
        search.push('\n');
    }
    if !replace.is_empty() && !replace.ends_with('\n') {
        replace.push('\n');
    }

    Hunk { search, replace }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_hunk_update() {
        let input = r#"Some prose before the patch.

*** Begin Patch
*** Update File: src/main.rs
@@ some context @@
 fn main() {
-    println!("hello");
+    println!("world");
 }
*** End Patch
"#;
        let edits = parse_apply_patch(input).expect("should parse");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].path.as_deref(), Some("src/main.rs"));
        assert_eq!(edits[0].hunks.len(), 1);
        assert!(edits[0].hunks[0].search.contains("hello"));
        assert!(edits[0].hunks[0].replace.contains("world"));
        // Context lines should be in both
        assert!(edits[0].hunks[0].search.contains("fn main()"));
        assert!(edits[0].hunks[0].replace.contains("fn main()"));
    }

    #[test]
    fn multi_hunk_single_file() {
        let input = r#"*** Begin Patch
*** Update File: lib.rs
@@ first hunk @@
-old_a
+new_a
@@ second hunk @@
-old_b
+new_b
*** End Patch
"#;
        let edits = parse_apply_patch(input).expect("should parse");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].hunks.len(), 2);
        assert!(edits[0].hunks[0].search.contains("old_a"));
        assert!(edits[0].hunks[1].search.contains("old_b"));
    }

    #[test]
    fn multi_file_patch() {
        let input = r#"*** Begin Patch
*** Update File: a.rs
@@ @@
-old_a
+new_a
*** Update File: b.rs
@@ @@
-old_b
+new_b
*** End Patch
"#;
        let edits = parse_apply_patch(input).expect("should parse");
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].path.as_deref(), Some("a.rs"));
        assert_eq!(edits[1].path.as_deref(), Some("b.rs"));
    }

    #[test]
    fn missing_envelope() {
        let input = "*** Update File: a.rs\n@@ @@\n-old\n+new\n";
        assert!(parse_apply_patch(input).is_err());
    }

    #[test]
    fn no_space_prefix_treated_as_context() {
        let input = r#"*** Begin Patch
*** Update File: test.rs
@@ @@
fn foo() {
-    old();
+    new();
}
*** End Patch
"#;
        let edits = parse_apply_patch(input).expect("should parse");
        let hunk = &edits[0].hunks[0];
        // Lines without prefix should appear in both search and replace
        assert!(hunk.search.contains("fn foo()"));
        assert!(hunk.replace.contains("fn foo()"));
    }

    #[test]
    fn add_file() {
        let input = r#"*** Begin Patch
*** Add File: new.rs
@@ @@
+fn new_fn() {}
*** End Patch
"#;
        let edits = parse_apply_patch(input).expect("should parse");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].path.as_deref(), Some("new.rs"));
        // Add file: search is empty (just newline), replace has content
        assert!(edits[0].hunks[0].replace.contains("new_fn"));
    }
}
