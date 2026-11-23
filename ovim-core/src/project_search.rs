//! Project-wide search with replacement previews.
//!
//! This is the engine behind "Replace in files" (`:SearchReplace`, `<Space>sr`)
//! and the quickfix-driven `:grep`. It is deliberately free of editor state:
//! callers hand in the options, the project root and the text of any buffers
//! that are open (so unsaved edits are searched, not the stale disk copy), and
//! get back per-file line matches.
//!
//! Matching is line based (a match never spans a newline), respects
//! `.gitignore`, skips hidden, binary and very large files, and reports
//! columns in `char`s so callers can map them to buffer columns.

use ignore::overrides::OverrideBuilder;
use regex::{Regex, RegexBuilder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Stop collecting after this many matches to keep the review list usable.
pub const MAX_MATCHES: usize = 20_000;
/// Files larger than this are skipped (generated bundles, dumps, ...).
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Full matching context is shared per line, but minified files can still
/// retain megabytes for one match. Bound it across the entire review.
const MAX_REVIEW_SOURCE_BYTES: usize = 16 * 1024 * 1024;
/// Stored line text is capped so a minified one-line file stays cheap.
const MAX_LINE_BYTES: usize = 2_000;

/// What to look for and where.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub pattern: String,
    /// Interpret `pattern` as a regular expression (otherwise it is literal).
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// Comma/space separated globs. A plain glob includes, `!glob` excludes.
    /// Globs without a `/` match at any depth (`*.java`, `!build`).
    pub globs: String,
}

impl SearchOptions {
    /// Compiles the pattern. `Err` carries a message fit for the status line.
    pub fn build_regex(&self) -> Result<Regex, String> {
        if self.pattern.is_empty() {
            return Err("empty pattern".to_string());
        }
        let mut source = if self.regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };
        if self.whole_word {
            source = format!(r"\b(?:{source})\b");
        }
        RegexBuilder::new(&source)
            .case_insensitive(!self.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|error| {
                // regex errors are multi-line; keep the first meaningful line.
                let text = error.to_string();
                text.lines()
                    .rev()
                    .find(|line| line.trim_start().starts_with("error:"))
                    .map(|line| line.trim_start().trim_start_matches("error:").trim())
                    .unwrap_or_else(|| text.lines().next().unwrap_or("invalid pattern"))
                    .to_string()
            })
    }
}

/// One matching span on one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    /// 0-based line.
    pub line: usize,
    /// 0-based char columns, end exclusive.
    pub start_col: usize,
    pub end_col: usize,
    /// Byte offsets of the span in `line_text`.
    pub start_byte: usize,
    pub end_byte: usize,
    /// The whole line (no line terminator), capped for display.
    pub line_text: String,
    /// Complete matching context, shared by matches on the same line. Display
    /// truncation must never change regex semantics or stale-edit validation.
    source_line: Arc<str>,
}

impl LineMatch {
    pub fn source_line(&self) -> &str {
        &self.source_line
    }
}

/// All matches in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatches {
    pub path: PathBuf,
    /// Path relative to the search root, for display.
    pub rel: String,
    pub matches: Vec<LineMatch>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchOutcome {
    pub files: Vec<FileMatches>,
    /// The match-count or retained-source limit was hit; results were omitted.
    pub truncated: bool,
    /// The search was cancelled before it finished.
    pub cancelled: bool,
}

impl SearchOutcome {
    pub fn match_count(&self) -> usize {
        self.files.iter().map(|file| file.matches.len()).sum()
    }
}

/// Splits `globs` into an override matcher. Returns `None` when empty.
fn build_overrides(
    root: &Path,
    globs: &str,
) -> Result<Option<ignore::overrides::Override>, String> {
    let tokens: Vec<&str> = globs
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.is_empty() {
        return Ok(None);
    }
    let mut builder = OverrideBuilder::new(root);
    for token in tokens {
        // In `ignore` overrides a bare glob whitelists and `!glob` ignores,
        // which is exactly the include/exclude contract of the option.
        builder
            .add(token)
            .map_err(|error| format!("bad glob '{token}': {error}"))?;
    }
    builder
        .build()
        .map(Some)
        .map_err(|error| format!("bad glob: {error}"))
}

/// Finds every match under `root`.
///
/// `overlays` maps absolute paths of open buffers to their current text; those
/// files are searched from memory instead of from disk. The result is sorted
/// by path so repeated searches are stable.
pub fn search_project(
    root: &Path,
    options: &SearchOptions,
    overlays: &HashMap<PathBuf, String>,
    cancel: &AtomicBool,
) -> Result<SearchOutcome, String> {
    let regex = options.build_regex()?;
    // Open-buffer overlays are keyed by canonical path; walk canonical paths.
    let root = &root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let overrides = build_overrides(root, &options.globs)?;

    let mut walker = crate::editor::grep::build_walker(root);
    if let Some(overrides) = overrides {
        walker.overrides(overrides);
    }

    let mut outcome = SearchOutcome::default();
    let mut total = 0usize;
    let mut retained_source_bytes = 0usize;

    for entry in walker.build() {
        if cancel.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            break;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path().to_path_buf();
        let content = match overlays.get(&path) {
            Some(text) => text.clone(),
            None => {
                if entry
                    .metadata()
                    .map(|meta| meta.len() > MAX_FILE_BYTES)
                    .unwrap_or(true)
                {
                    continue;
                }
                match std::fs::read(&path) {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(text) => text,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                }
            }
        };
        if content.contains('\0') {
            continue;
        }
        let mut matches = match_content(&regex, &content, &mut total);
        let mut kept = 0;
        let mut last_line = None;
        for found in &matches {
            if last_line != Some(found.line) {
                let bytes = found.source_line.len();
                if bytes > MAX_REVIEW_SOURCE_BYTES.saturating_sub(retained_source_bytes) {
                    outcome.truncated = true;
                    break;
                }
                retained_source_bytes += bytes;
                last_line = Some(found.line);
            }
            kept += 1;
        }
        matches.truncate(kept);
        if !matches.is_empty() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            outcome.files.push(FileMatches { path, rel, matches });
        }
        if total >= MAX_MATCHES || outcome.truncated {
            outcome.truncated = true;
            break;
        }
    }

    // Open buffers that are not on disk under the root walk (new, unsaved
    // files) are intentionally not searched: they have no stable path.
    outcome.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(outcome)
}

/// Line-based matching over `content`. `total` counts matches across files.
pub fn match_content(regex: &Regex, content: &str, total: &mut usize) -> Vec<LineMatch> {
    let mut matches = Vec::new();
    for (line_index, raw) in content.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        // Bound the preview, while matching against the complete line so '$'
        // and word boundaries never treat the preview cutoff as end of text.
        let line_capped = cap_line(line);
        let mut source_line: Option<Arc<str>> = None;
        for found in regex.find_iter(line) {
            if found.end() > line_capped.len() {
                break;
            }
            if found.start() == found.end() {
                continue;
            }
            matches.push(LineMatch {
                line: line_index,
                start_col: line_capped[..found.start()].chars().count(),
                end_col: line_capped[..found.end()].chars().count(),
                start_byte: found.start(),
                end_byte: found.end(),
                line_text: line_capped.to_string(),
                source_line: source_line.get_or_insert_with(|| Arc::from(line)).clone(),
            });
            *total += 1;
            if *total >= MAX_MATCHES {
                return matches;
            }
        }
    }
    matches
}

fn cap_line(line: &str) -> &str {
    if line.len() <= MAX_LINE_BYTES {
        return line;
    }
    let mut end = MAX_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

/// The text that replaces `found` given the replacement template.
///
/// Regex mode expands `$1` / `${name}` capture references; literal mode inserts
/// the replacement verbatim (a `$` is just a `$`).
pub fn expand_replacement(
    regex: &Regex,
    options: &SearchOptions,
    found: &LineMatch,
    replacement: &str,
) -> String {
    if !options.regex {
        return replacement.to_string();
    }
    // Re-run the regex anchored at the match start so capture groups line up
    // with the original match (lookups on the stored line, not the buffer).
    let line = found.source_line();
    if let Some(caps) = regex.captures_at(line, found.start_byte) {
        if caps.get(0).map(|m| m.start()) == Some(found.start_byte) {
            let mut out = String::new();
            caps.expand(&normalize_template(replacement), &mut out);
            return out;
        }
    }
    replacement.to_string()
}

/// Makes replacement templates friendlier than the regex crate's defaults:
/// `$1_x` means group 1 followed by `_x` (not a group named `1_x`), and the
/// vim spelling `\1` works too. `${name}`, `$name` and `$$` are unchanged.
fn normalize_template(template: &str) -> String {
    let mut out = String::with_capacity(template.len() + 4);
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '$' if chars.peek().is_some_and(|next| next.is_ascii_digit()) => {
                out.push_str("${");
                while let Some(digit) = chars.next_if(|next| next.is_ascii_digit()) {
                    out.push(digit);
                }
                out.push('}');
            }
            '$' if chars.peek() == Some(&'$') => {
                out.push_str("$$");
                chars.next();
            }
            '\\' if chars.peek().is_some_and(|next| next.is_ascii_digit()) => {
                out.push_str("${");
                out.push(chars.next().unwrap_or('0'));
                out.push('}');
            }
            '\\' if chars.peek() == Some(&'\\') => {
                out.push('\\');
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// Same-length window of `line_text` around a match for compact display.
pub fn preview_window(found: &LineMatch, width: usize) -> (String, usize, usize) {
    let chars: Vec<char> = found.line_text.chars().collect();
    let start = found.start_col.saturating_sub(width / 3);
    let end = (start + width).min(chars.len());
    let text: String = chars[start..end].iter().collect();
    (
        text,
        found.start_col.saturating_sub(start),
        found.end_col.min(end).saturating_sub(start),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn run(root: &Path, options: &SearchOptions) -> SearchOutcome {
        search_project(root, options, &HashMap::new(), &AtomicBool::new(false)).unwrap()
    }

    fn opts(pattern: &str) -> SearchOptions {
        SearchOptions {
            pattern: pattern.to_string(),
            ..SearchOptions::default()
        }
    }

    #[test]
    fn literal_search_is_case_insensitive_by_default_and_toggles() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "Foo foo FOO\n").unwrap();
        let outcome = run(dir.path(), &opts("foo"));
        assert_eq!(outcome.match_count(), 3);
        let mut sensitive = opts("foo");
        sensitive.case_sensitive = true;
        assert_eq!(run(dir.path(), &sensitive).match_count(), 1);
    }

    #[test]
    fn literal_mode_escapes_regex_metacharacters() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "a.b axb\n").unwrap();
        assert_eq!(run(dir.path(), &opts("a.b")).match_count(), 1);
        let mut regex = opts("a.b");
        regex.regex = true;
        assert_eq!(run(dir.path(), &regex).match_count(), 2);
    }

    #[test]
    fn whole_word_excludes_substrings() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "cat concat cat_x cat\n").unwrap();
        let mut options = opts("cat");
        options.whole_word = true;
        assert_eq!(run(dir.path(), &options).match_count(), 2);
    }

    #[test]
    fn gitignore_hidden_and_globs_are_respected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        fs::create_dir(root.join("ignored")).unwrap();
        fs::write(root.join("ignored/x.java"), "needle\n").unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/A.java"), "needle\n").unwrap();
        fs::write(root.join("src/b.txt"), "needle\n").unwrap();
        fs::write(root.join(".hidden"), "needle\n").unwrap();

        let all = run(root, &opts("needle"));
        let rels: Vec<_> = all.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, vec!["src/A.java", "src/b.txt"]);

        let mut java_only = opts("needle");
        java_only.globs = "*.java".to_string();
        let rels: Vec<_> = run(root, &java_only)
            .files
            .iter()
            .map(|f| f.rel.clone())
            .collect();
        assert_eq!(rels, vec!["src/A.java"]);

        let mut no_txt = opts("needle");
        no_txt.globs = "!*.txt".to_string();
        let rels: Vec<_> = run(root, &no_txt)
            .files
            .iter()
            .map(|f| f.rel.clone())
            .collect();
        assert_eq!(rels, vec!["src/A.java"]);
    }

    #[test]
    fn open_buffer_text_shadows_the_disk_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "old\n").unwrap();
        let mut overlays = HashMap::new();
        overlays.insert(path.canonicalize().unwrap(), "new new\n".to_string());
        let outcome =
            search_project(dir.path(), &opts("new"), &overlays, &AtomicBool::new(false)).unwrap();
        assert_eq!(outcome.match_count(), 2);
        assert_eq!(run(dir.path(), &opts("old")).match_count(), 1);
    }

    #[test]
    fn columns_are_char_based() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "é\u{1F600}x foo\n").unwrap();
        let outcome = run(dir.path(), &opts("foo"));
        let found = &outcome.files[0].matches[0];
        assert_eq!((found.start_col, found.end_col), (4, 7));
    }

    #[test]
    fn regex_replacement_expands_captures_and_literal_does_not() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "get_name();\n").unwrap();
        let mut options = opts(r"get_(\w+)");
        options.regex = true;
        let regex = options.build_regex().unwrap();
        let outcome = run(dir.path(), &options);
        let found = &outcome.files[0].matches[0];
        assert_eq!(
            expand_replacement(&regex, &options, found, "$1_of"),
            "name_of"
        );

        let literal = opts("get_name");
        let regex = literal.build_regex().unwrap();
        let outcome = run(dir.path(), &literal);
        let found = &outcome.files[0].matches[0];
        assert_eq!(expand_replacement(&regex, &literal, found, "$1"), "$1");
    }

    #[test]
    fn invalid_regex_reports_an_error() {
        let mut options = opts("(");
        options.regex = true;
        assert!(options.build_regex().is_err());
    }

    #[test]
    fn crlf_lines_do_not_leak_the_carriage_return() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "foo\r\nbar foo\r\n").unwrap();
        let outcome = run(dir.path(), &opts("foo"));
        assert_eq!(outcome.match_count(), 2);
        assert!(outcome.files[0]
            .matches
            .iter()
            .all(|m| !m.line_text.ends_with('\r')));
    }
    #[test]
    fn preview_cutoff_does_not_create_false_end_anchor_or_word_boundary_matches() {
        let line = format!("{}fooX", "a".repeat(MAX_LINE_BYTES - 3));
        for pattern in [r"foo$", r"foo\b", r"^a+foo$"] {
            let regex = Regex::new(pattern).unwrap();
            assert!(match_content(&regex, &line, &mut 0).is_empty(), "{pattern}");
        }
        assert!(match_content(
            &Regex::new(r"^a+$").unwrap(),
            &format!("{}b", "a".repeat(MAX_LINE_BYTES)),
            &mut 0
        )
        .is_empty());
    }

    #[test]
    fn long_line_matches_share_full_context_for_capture_expansion() {
        let line = format!("{}afooX", "é".repeat((MAX_LINE_BYTES - 4) / 2));
        let options = SearchOptions {
            pattern: "(foo$)|(foo)".into(),
            regex: true,
            ..SearchOptions::default()
        };
        let regex = options.build_regex().unwrap();
        let matches = match_content(&regex, &line, &mut 0);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].source_line(), line);
        assert!(matches[0].line_text.len() <= MAX_LINE_BYTES);
        assert!(matches[0].line_text.len() < matches[0].source_line().len());
        assert_eq!(
            expand_replacement(&regex, &options, &matches[0], "${1}x${2}"),
            "xfoo"
        );

        let matches = match_content(&Regex::new("é").unwrap(), &line, &mut 0);
        assert!(Arc::ptr_eq(
            &matches[0].source_line,
            &matches[1].source_line
        ));
    }
    #[test]
    fn large_line_context_is_bounded_across_the_review() {
        let dir = tempfile::tempdir().unwrap();
        let line = format!("needle{}", "x".repeat(MAX_FILE_BYTES as usize - 6));
        for index in 0..6 {
            fs::write(dir.path().join(format!("{index}.txt")), &line).unwrap();
        }
        let outcome = run(dir.path(), &opts("needle"));
        assert!(outcome.truncated);
        assert_eq!(outcome.match_count(), 4);
        let source_bytes: usize = outcome
            .files
            .iter()
            .flat_map(|file| &file.matches)
            .map(|found| found.source_line.len())
            .sum();
        assert_eq!(source_bytes, MAX_REVIEW_SOURCE_BYTES);
    }
}
