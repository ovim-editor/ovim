//! Parsers for build output, turned into quickfix entries: JVM builds
//! (javac, kotlinc plain and via Gradle, Maven) with correct file/line/
//! column, then rustc, gcc/clang, tsc and Rust panics
//! ([`parse_compiler_output_in`]).
//!
//! Formats understood:
//!
//! - javac (also printed verbatim by Gradle):
//!   ```text
//!   /abs/Foo.java:12: error: cannot find symbol
//!           foo.bar();
//!              ^
//!     symbol:   method bar()
//!     location: variable foo of type Foo
//!   ```
//!   The column comes from the caret line; `symbol:`/`location:` detail lines
//!   are folded into the entry text.
//! - kotlinc: `e: file:///abs/Foo.kt:12:5 message`, the older
//!   `e: /abs/Foo.kt: (12, 5): message` and `w:` warnings.
//! - Maven: `[ERROR] /abs/Foo.java:[12,5] message` followed by
//!   `[ERROR]   symbol:` / `[ERROR]   location:` detail lines, and the Kotlin
//!   Maven plugin's `[ERROR] /abs/Foo.kt: (12, 5) message`.
//!
//! The same error frequently appears twice in build output (Gradle repeats
//! compiler output when a task is retried, Maven prints both the compiler
//! block and a summary), so results are de-duplicated.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::editor::{QuickfixEntry, QuickfixEntryType};

/// Result of scanning build output for JVM compiler diagnostics.
#[derive(Debug, Default)]
pub struct JvmDiagnostics {
    /// Diagnostics in output order, de-duplicated.
    pub entries: Vec<QuickfixEntry>,
    /// `consumed[i]` is true when output line `i` was part of a recognised
    /// diagnostic (header, snippet, caret or detail line). Generic parsers
    /// should skip these lines to avoid double-reporting.
    pub consumed: Vec<bool>,
}

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new("\x1b\\[[0-9;?]*[A-Za-z]").unwrap());

static JAVAC_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<indent>[ \t]*)(?P<path>.+?\.(?:java|kt|kts|scala|groovy)):(?P<line>\d+): (?P<kind>error|warning|note): ?(?P<msg>.*)$",
    )
    .unwrap()
});

static KOTLIN_URI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<kind>[ewi]): (?P<path>(?:file://)?.+?):(?P<line>\d+):(?P<col>\d+)[: ]\s*(?P<msg>.*)$")
        .unwrap()
});

static KOTLIN_PAREN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?P<kind>[ewi]): (?P<path>.+?): ?\((?P<line>\d+), ?(?P<col>\d+)\):? ?(?P<msg>.*)$",
    )
    .unwrap()
});

static KOTLIN_BARE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<kind>[ew]): (?P<msg>\S.*)$").unwrap());

static MAVEN_BRACKET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\[(?P<kind>ERROR|WARNING)\]\s+(?P<path>.+?):\[(?P<line>\d+),(?P<col>\d+)\]\s*(?P<msg>.*)$",
    )
    .unwrap()
});

static MAVEN_PAREN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\[(?P<kind>ERROR|WARNING)\]\s+(?P<path>.+?\.(?:kt|kts|java|scala|groovy)): ?\((?P<line>\d+), ?(?P<col>\d+)\):? ?(?P<msg>.*)$",
    )
    .unwrap()
});

static CARET_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[ \t]*\^[ \t]*$").unwrap());

static DETAIL_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s+(?P<key>symbol|location|required|found|reason|expected|actual)\s*:\s*(?P<val>.*)$",
    )
    .unwrap()
});

static MAVEN_DETAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\[(?:ERROR|WARNING)\]\s+(?P<key>symbol|location|required|found|reason|expected|actual)\s*:\s*(?P<val>.*)$",
    )
    .unwrap()
});

/// Removes ANSI colour/cursor escapes so coloured Maven/Gradle output parses.
pub fn strip_ansi(line: &str) -> std::borrow::Cow<'_, str> {
    if line.contains('\x1b') {
        ANSI.replace_all(line, "")
    } else {
        std::borrow::Cow::Borrowed(line)
    }
}

/// Decodes `%XX` escapes in a `file://` URI path.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn resolve_path(raw: &str, base_dir: Option<&Path>) -> PathBuf {
    let raw = raw.trim();
    let path = if let Some(rest) = raw.strip_prefix("file://") {
        // `file:///abs` -> `/abs`; `file:///C:/x` -> `C:/x`.
        let decoded = percent_decode(rest);
        if cfg!(windows)
            && decoded.len() > 2
            && decoded.as_bytes()[0] == b'/'
            && decoded.as_bytes()[2] == b':'
        {
            decoded[1..].to_string()
        } else {
            decoded
        }
    } else {
        raw.to_string()
    };
    let path = PathBuf::from(path);
    match base_dir {
        Some(base) if path.is_relative() => base.join(path),
        _ => path,
    }
}

fn kind_type(kind: &str) -> QuickfixEntryType {
    match kind {
        "error" | "ERROR" | "e" => QuickfixEntryType::Error,
        "warning" | "WARNING" | "w" => QuickfixEntryType::Warning,
        "note" => QuickfixEntryType::Note,
        _ => QuickfixEntryType::Info,
    }
}

fn is_header(line: &str) -> bool {
    JAVAC_HEADER.is_match(line)
        || KOTLIN_URI.is_match(line)
        || KOTLIN_PAREN.is_match(line)
        || MAVEN_BRACKET.is_match(line)
        || MAVEN_PAREN.is_match(line)
}

fn is_summary(line: &str) -> bool {
    let t = line.trim();
    let mut parts = t.split_whitespace();
    matches!(
        (
            parts.next().and_then(|n| n.parse::<u32>().ok()),
            parts.next()
        ),
        (Some(_), Some("error" | "errors" | "warning" | "warnings"))
    )
}

fn fold_details(text: &mut String, details: &[(String, String)]) {
    if details.is_empty() {
        return;
    }
    let joined = details
        .iter()
        .map(|(key, val)| format!("{key}: {val}"))
        .collect::<Vec<_>>()
        .join("; ");
    text.push_str(&format!(" ({joined})"));
}

/// Scans `output` for javac / kotlinc / Gradle / Maven diagnostics.
///
/// `base_dir` is the directory relative paths in the output are relative to
/// (the working directory of the build).
pub fn parse_jvm_diagnostics(output: &str, base_dir: Option<&Path>) -> JvmDiagnostics {
    let cleaned: Vec<std::borrow::Cow<'_, str>> = output.lines().map(strip_ansi).collect();
    let lines: Vec<&str> = cleaned.iter().map(|l| l.trim_end_matches('\r')).collect();
    let mut consumed = vec![false; lines.len()];
    let mut entries: Vec<QuickfixEntry> = Vec::new();
    let mut seen: HashSet<(PathBuf, usize, usize, u8, String)> = HashSet::new();

    let mut push = |entry: QuickfixEntry, entries: &mut Vec<QuickfixEntry>| {
        let key = (
            entry.filename.clone().unwrap_or_default(),
            entry.lnum,
            entry.col,
            entry.entry_type as u8,
            entry.text.clone(),
        );
        if seen.insert(key) {
            entries.push(entry);
        }
    };

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // ---- javac ----
        if let Some(caps) = JAVAC_HEADER.captures(line) {
            let lnum: usize = caps["line"].parse().unwrap_or(0);
            let mut text = caps["msg"].trim().to_string();
            let entry_type = kind_type(&caps["kind"]);
            let path = resolve_path(&caps["path"], base_dir);
            // Gradle repeats compiler output indented inside its failure
            // report; the caret column is relative to that indentation.
            let indent = caps["indent"].chars().count();
            consumed[i] = true;
            let mut col = 0usize;
            let mut next = i + 1;

            // Source snippet + caret. javac always prints both together; a
            // caret line directly after the header (empty snippet) is also
            // tolerated.
            if next + 1 < lines.len()
                && CARET_LINE.is_match(lines[next + 1])
                && !is_header(lines[next])
            {
                col = (lines[next + 1].chars().take_while(|c| *c != '^').count() + 1)
                    .saturating_sub(indent)
                    .max(1);
                consumed[next] = true;
                consumed[next + 1] = true;
                next += 2;
            } else if next < lines.len() && CARET_LINE.is_match(lines[next]) {
                col = (lines[next].chars().take_while(|c| *c != '^').count() + 1)
                    .saturating_sub(indent)
                    .max(1);
                consumed[next] = true;
                next += 1;
            }

            let mut details: Vec<(String, String)> = Vec::new();
            while next < lines.len() {
                let l = lines[next];
                if l.trim().is_empty() || is_header(l) || is_summary(l) {
                    break;
                }
                if let Some(d) = DETAIL_LINE.captures(l) {
                    details.push((d["key"].to_string(), d["val"].trim().to_string()));
                    consumed[next] = true;
                    next += 1;
                    continue;
                }
                // Indented continuation lines of a multi-line message
                // ("method X is not applicable", "where T is a type-variable")
                // belong to the diagnostic but are too noisy for quickfix.
                if l.starts_with(' ') || l.starts_with('\t') {
                    consumed[next] = true;
                    next += 1;
                    continue;
                }
                break;
            }
            fold_details(&mut text, &details);
            push(
                QuickfixEntry::new(Some(path), lnum, col, entry_type, text),
                &mut entries,
            );
            i = next;
            continue;
        }

        // ---- kotlinc ----
        if let Some(caps) = KOTLIN_URI
            .captures(line)
            .or_else(|| KOTLIN_PAREN.captures(line))
        {
            let lnum: usize = caps["line"].parse().unwrap_or(0);
            let col: usize = caps["col"].parse().unwrap_or(0);
            consumed[i] = true;
            push(
                QuickfixEntry::new(
                    Some(resolve_path(&caps["path"], base_dir)),
                    lnum,
                    col,
                    kind_type(&caps["kind"]),
                    caps["msg"].trim().to_string(),
                ),
                &mut entries,
            );
            i += 1;
            continue;
        }

        // ---- Maven ----
        if let Some(caps) = MAVEN_BRACKET
            .captures(line)
            .or_else(|| MAVEN_PAREN.captures(line))
        {
            let lnum: usize = caps["line"].parse().unwrap_or(0);
            let col: usize = caps["col"].parse().unwrap_or(0);
            let mut text = caps["msg"].trim().to_string();
            let entry_type = kind_type(&caps["kind"]);
            let path = resolve_path(&caps["path"], base_dir);
            consumed[i] = true;
            let mut next = i + 1;
            let mut details: Vec<(String, String)> = Vec::new();
            while next < lines.len() {
                if let Some(d) = MAVEN_DETAIL.captures(lines[next]) {
                    details.push((d["key"].to_string(), d["val"].trim().to_string()));
                    consumed[next] = true;
                    next += 1;
                } else {
                    break;
                }
            }
            fold_details(&mut text, &details);
            push(
                QuickfixEntry::new(Some(path), lnum, col, entry_type, text),
                &mut entries,
            );
            i = next;
            continue;
        }

        // ---- kotlinc message without a location ----
        if let Some(caps) = KOTLIN_BARE.captures(line) {
            // Avoid swallowing unrelated "e: ..." prose: only accept it when
            // it is the Kotlin compiler's own prefix at column 0.
            consumed[i] = true;
            push(
                QuickfixEntry::new(
                    None,
                    0,
                    0,
                    kind_type(&caps["kind"]),
                    caps["msg"].trim().to_string(),
                ),
                &mut entries,
            );
        }
        i += 1;
    }

    JvmDiagnostics { entries, consumed }
}

/// One-line explanation of why a build failed, for status messages.
///
/// Prefers the first diagnostic; otherwise Gradle's "What went wrong" block or
/// Maven's `Failed to execute goal` line; otherwise the last non-empty output
/// line.
pub fn summarize_build_failure(output: &str, diagnostics: &[QuickfixEntry]) -> Option<String> {
    let errors = diagnostics
        .iter()
        .filter(|e| e.entry_type == QuickfixEntryType::Error)
        .count();
    if let Some(first) = diagnostics
        .iter()
        .find(|e| e.entry_type == QuickfixEntryType::Error)
    {
        let file = first
            .filename
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let head = first.text.split(" (").next().unwrap_or(&first.text);
        let more = if errors > 1 {
            format!(" (+{} more)", errors - 1)
        } else {
            String::new()
        };
        return Some(if file.is_empty() {
            format!("{head}{more}")
        } else {
            format!("{file}:{}: {head}{more}", first.lnum)
        });
    }
    let lines: Vec<String> = output
        .lines()
        .map(|l| strip_ansi(l).trim().to_string())
        .collect();
    if let Some(pos) = lines
        .iter()
        .position(|l| l.starts_with("* What went wrong"))
    {
        if let Some(msg) = lines[pos + 1..].iter().find(|l| !l.is_empty()) {
            return Some(msg.clone());
        }
    }
    if let Some(l) = lines
        .iter()
        .find(|l| l.starts_with("[ERROR] Failed to execute goal"))
    {
        return Some(l.trim_start_matches("[ERROR] ").to_string());
    }
    lines.into_iter().rev().find(|l| !l.is_empty())
}

// ---- rustc / gcc / tsc / Rust panics (everything that is not JVM) ----

/// Parse compiler output for file:line:col: error/warning patterns
pub fn parse_compiler_output(output: &str) -> Vec<QuickfixEntry> {
    parse_compiler_output_in(output, None)
}

/// Like [`parse_compiler_output`], resolving relative paths against `base_dir`
/// (the directory the build ran in). javac, kotlinc, Gradle and Maven output
/// is handled by [`crate::launch::diagnostics`] first (with columns from
/// javac's caret line and `symbol:`/`location:` details); the remaining lines
/// go through the rustc / gcc / tsc patterns below.
pub fn parse_compiler_output_in(
    output: &str,
    base_dir: Option<&std::path::Path>,
) -> Vec<QuickfixEntry> {
    let jvm = parse_jvm_diagnostics(output, base_dir);
    let mut entries = parse_generic_compiler_output(output, &jvm.consumed);
    if let Some(base) = base_dir {
        for entry in &mut entries {
            if let Some(path) = entry.filename.as_mut() {
                if path.is_relative() {
                    *path = base.join(&*path);
                }
            }
        }
    }
    let mut all = jvm.entries;
    all.append(&mut entries);
    all
}

/// rustc / gcc / tsc / panic patterns. Lines flagged in `skip` were already
/// claimed by a more specific parser.
fn parse_generic_compiler_output(output: &str, skip: &[bool]) -> Vec<QuickfixEntry> {
    use crate::editor::{QuickfixEntry, QuickfixEntryType};
    use std::path::PathBuf;

    let mut entries = Vec::new();

    // Regex-free parsing for common patterns:
    // - rustc/cargo:  "  --> file.rs:line:col"
    // - gcc/clang:    "file.rs:line:col: error: message"
    // - typescript:    "file.ts(line,col): error TS1234: message"
    for (line_index, line) in output.lines().enumerate() {
        if skip.get(line_index).copied().unwrap_or(false) {
            continue;
        }
        let trimmed = line.trim();

        // Rust/cargo style: "  --> file.rs:42:10"
        if let Some(rest) = trimmed.strip_prefix("--> ") {
            // Split from the right: col, then line, rest is file path
            let parts: Vec<&str> = rest.rsplitn(3, ':').collect();
            if parts.len() == 3 {
                if let (Ok(col), Ok(lnum)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>())
                {
                    entries.push(QuickfixEntry::new(
                        Some(PathBuf::from(parts[2])),
                        lnum,
                        col,
                        QuickfixEntryType::Error,
                        String::new(), // Will be filled from context
                    ));
                }
            }
            continue;
        }

        // Rust panic style: "thread 'name' panicked at 'msg', file.rs:42:5"
        // Also: "thread 'name' panicked at file.rs:42:5:" (Rust 2024+ format)
        if trimmed.starts_with("thread '") && trimmed.contains("panicked at") {
            if let Some(entry) = parse_panic_line(trimmed) {
                entries.push(entry);
                continue;
            }
        }

        // gcc/clang/generic style: "file:line:col: error/warning: message"
        // Also matches: "file:line: error: message" (no col)
        if let Some(entry) = parse_gcc_style_line(trimmed) {
            entries.push(entry);
        }
    }

    // Second pass: fill in error messages from cargo/rustc output
    // Cargo errors look like:
    //   error[E0425]: cannot find value `foo` in this scope
    //     --> file.rs:42:10
    // So we look for "error" or "warning" lines preceding "-->" lines
    let lines: Vec<&str> = output.lines().collect();
    let mut entry_idx = 0;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("--> ") && entry_idx < entries.len() {
            // Look backward for the error/warning message
            if i > 0 {
                let prev = lines[i - 1].trim();
                if prev.starts_with("error") || prev.starts_with("warning") {
                    entries[entry_idx].text = prev.to_string();
                    if prev.starts_with("warning") {
                        entries[entry_idx].entry_type = QuickfixEntryType::Warning;
                    }
                }
            }
            entry_idx += 1;
        }
    }

    entries
}

/// Parse a gcc/clang-style error line: "file:line:col: error: message"
fn parse_gcc_style_line(line: &str) -> Option<QuickfixEntry> {
    use crate::editor::{QuickfixEntry, QuickfixEntryType};
    use std::path::PathBuf;

    // Skip lines that don't look like file:line patterns
    // Must contain at least one ':' and not start with whitespace
    if line.is_empty() || line.starts_with(' ') || !line.contains(':') {
        return None;
    }

    // Try to match file:line:col: type: message
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() < 3 {
        return None;
    }

    let file = parts[0];
    let lnum: usize = parts[1].trim().parse().ok()?;

    // Check if parts[2] is a column number or the error type
    let (col, rest) = if let Ok(c) = parts[2].trim().parse::<usize>() {
        let rest = if parts.len() > 3 { parts[3] } else { "" };
        (c, rest)
    } else {
        // No column — parts[2] is the type/message
        let rest = if parts.len() > 3 {
            &line[parts[0].len() + 1 + parts[1].len() + 1..]
        } else {
            parts[2]
        };
        (0, rest)
    };

    let rest = rest.trim();
    let entry_type = if rest.starts_with("error") {
        QuickfixEntryType::Error
    } else if rest.starts_with("warning") {
        QuickfixEntryType::Warning
    } else if rest.starts_with("note") {
        QuickfixEntryType::Note
    } else if rest.starts_with("info") {
        QuickfixEntryType::Info
    } else {
        // Not a recognizable error pattern — could be just a file path with colons
        // Only include if it looks like an error (has some message text)
        if rest.is_empty() {
            return None;
        }
        QuickfixEntryType::Error
    };

    // Don't include entries for paths that don't look like files
    if !file.contains('.') && !file.contains('/') {
        return None;
    }

    Some(QuickfixEntry::new(
        Some(PathBuf::from(file)),
        lnum,
        col,
        entry_type,
        rest.to_string(),
    ))
}

/// Parse Rust panic lines from test output.
///
/// Formats:
/// - `thread 'test_name' panicked at 'assertion message', src/file.rs:42:5`
/// - `thread 'test_name' panicked at src/file.rs:42:5:` (Rust 2024+)
fn parse_panic_line(line: &str) -> Option<QuickfixEntry> {
    // Extract the panic message and location
    let after_panicked = line.split("panicked at").nth(1)?.trim();

    // Try old format: 'message', file:line:col
    if after_panicked.starts_with('\'') {
        // Find closing quote + comma
        if let Some(comma_pos) = after_panicked.rfind("', ") {
            let message = &after_panicked[1..comma_pos];
            let location = &after_panicked[comma_pos + 3..];
            return parse_file_line_col(location, message);
        }
    }

    // Try new format: file:line:col:
    // or: file:line:col:\nmessage
    let location = after_panicked.trim_end_matches(':');
    parse_file_line_col(location, "panicked")
}

/// Parse a `file:line:col` string into a QuickfixEntry.
fn parse_file_line_col(location: &str, message: &str) -> Option<QuickfixEntry> {
    use crate::editor::{QuickfixEntry, QuickfixEntryType};

    let parts: Vec<&str> = location.rsplitn(3, ':').collect();
    if parts.len() >= 2 {
        let col: usize = parts[0].trim().parse().ok().unwrap_or(0);
        let lnum: usize = parts[1].trim().parse().ok()?;
        let file = if parts.len() == 3 {
            parts[2]
        } else {
            return None;
        };

        if !file.contains('.') && !file.contains('/') {
            return None;
        }

        return Some(QuickfixEntry::new(
            Some(std::path::PathBuf::from(file)),
            lnum,
            col,
            QuickfixEntryType::Error,
            format!("PANIC: {}", message),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(e: &QuickfixEntry) -> (String, usize, usize, QuickfixEntryType, &str) {
        (
            e.filename
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            e.lnum,
            e.col,
            e.entry_type,
            e.text.as_str(),
        )
    }

    #[test]
    fn javac_error_gets_column_from_caret_and_symbol_details() {
        let out = "\
> Task :compileJava FAILED
/p/src/main/java/com/x/Foo.java:12: error: cannot find symbol
        foo.bar();
           ^
  symbol:   method bar()
  location: variable foo of type Foo
1 error
";
        let parsed = parse_jvm_diagnostics(out, None);
        assert_eq!(parsed.entries.len(), 1);
        let e = &parsed.entries[0];
        assert_eq!(
            e.filename.as_deref(),
            Some(Path::new("/p/src/main/java/com/x/Foo.java"))
        );
        assert_eq!(e.lnum, 12);
        assert_eq!(e.col, 12, "caret sits under the '.' before bar");
        assert!(e.text.starts_with("cannot find symbol"));
        assert!(e.text.contains("symbol: method bar()"), "{}", e.text);
        assert!(
            e.text.contains("location: variable foo of type Foo"),
            "{}",
            e.text
        );
        assert!(parsed.consumed[1] && parsed.consumed[2] && parsed.consumed[3]);
        assert!(parsed.consumed[4] && parsed.consumed[5]);
        assert!(!parsed.consumed[0]);
        assert!(
            !parsed.consumed[6],
            "the '1 error' summary is not a diagnostic line"
        );
    }

    #[test]
    fn gradle_repeats_javac_output_indented_and_the_copy_is_deduplicated() {
        let out = "\
/p/A.java:58: error: incompatible types: String cannot be converted to int
        int total = \"x\";
                    ^
1 error

* What went wrong:
> Compilation failed; see the compiler output below.
  /p/A.java:58: error: incompatible types: String cannot be converted to int
          int total = \"x\";
                      ^
  1 error
";
        let parsed = parse_jvm_diagnostics(out, None);
        assert_eq!(parsed.entries.len(), 1, "{:?}", parsed.entries);
        assert_eq!(parsed.entries[0].col, 21);
    }

    #[test]
    fn javac_relative_paths_resolve_against_build_cwd() {
        let out = "src/A.java:3: error: ';' expected\n  int x\n       ^\n";
        let parsed = parse_jvm_diagnostics(out, Some(Path::new("/work")));
        assert_eq!(
            entry(&parsed.entries[0]),
            (
                "/work/src/A.java".into(),
                3,
                8,
                QuickfixEntryType::Error,
                "';' expected"
            )
        );
    }

    #[test]
    fn javac_multiline_message_before_details_keeps_first_line() {
        let out = "\
A.java:5: error: no suitable method found for println(int,int)
        System.out.println(1, 2);
                  ^
    method PrintStream.println() is not applicable
      (actual and formal argument lists differ in length)
A.java:9: warning: [deprecation] foo() in Bar has been deprecated
    bar.foo();
       ^
";
        let parsed = parse_jvm_diagnostics(out, None);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].col, 19);
        assert_eq!(
            parsed.entries[0].text,
            "no suitable method found for println(int,int)"
        );
        assert_eq!(parsed.entries[1].entry_type, QuickfixEntryType::Warning);
        assert_eq!(parsed.entries[1].lnum, 9);
    }

    #[test]
    fn kotlin_uri_and_legacy_paren_and_warning_forms() {
        let out = "\
e: file:///p/src/main/kotlin/My%20App/Main.kt:12:5 Unresolved reference: foo
w: file:///p/src/Main.kt:3:1 Parameter 'x' is never used
e: /p/src/Old.kt: (7, 9): Type mismatch: inferred type is String but Int was expected
e: Some global failure
";
        let parsed = parse_jvm_diagnostics(out, None);
        let got: Vec<_> = parsed.entries.iter().map(entry).collect();
        assert_eq!(
            got[0],
            (
                "/p/src/main/kotlin/My App/Main.kt".to_string(),
                12,
                5,
                QuickfixEntryType::Error,
                "Unresolved reference: foo"
            )
        );
        assert_eq!(got[1].3, QuickfixEntryType::Warning);
        assert_eq!(got[2].0, "/p/src/Old.kt");
        assert_eq!((got[2].1, got[2].2), (7, 9));
        assert_eq!(
            got[3],
            (
                String::new(),
                0,
                0,
                QuickfixEntryType::Error,
                "Some global failure"
            )
        );
    }

    #[test]
    fn maven_bracket_form_with_detail_lines() {
        let out = "\
[INFO] Compiling 3 source files
[ERROR] /p/src/main/java/A.java:[12,5] cannot find symbol
[ERROR]   symbol:   variable x
[ERROR]   location: class A
[ERROR] /p/src/main/java/B.java:[3,1] class, interface, enum, or record expected
[ERROR] Failed to execute goal org.apache.maven.plugins:maven-compiler-plugin:3.11.0:compile
";
        let parsed = parse_jvm_diagnostics(out, None);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!((parsed.entries[0].lnum, parsed.entries[0].col), (12, 5));
        assert!(parsed.entries[0].text.contains("symbol: variable x"));
        assert!(parsed.entries[0].text.contains("location: class A"));
        assert_eq!((parsed.entries[1].lnum, parsed.entries[1].col), (3, 1));
        assert!(!parsed.consumed[5], "goal failure line is not a diagnostic");
    }

    #[test]
    fn maven_kotlin_paren_form() {
        let out = "[ERROR] /p/src/main/kotlin/M.kt: (4, 13) Unresolved reference: zed\n";
        let parsed = parse_jvm_diagnostics(out, None);
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!((parsed.entries[0].lnum, parsed.entries[0].col), (4, 13));
    }

    #[test]
    fn duplicates_are_removed_and_ansi_is_stripped() {
        let out = "\
\x1b[1;31m[ERROR]\x1b[m /p/A.java:[1,2] boom
[ERROR] /p/A.java:[1,2] boom
/p/A.java:1: error: boom
  x
   ^
/p/A.java:1: error: boom
  x
   ^
";
        let parsed = parse_jvm_diagnostics(out, None);
        // Maven and javac forms differ in column (2 vs 4) so they survive as
        // distinct entries, but exact repeats collapse.
        assert_eq!(parsed.entries.len(), 2, "{:?}", parsed.entries);
    }

    #[test]
    fn non_jvm_output_is_left_alone() {
        let out =
            "error[E0425]: cannot find value `x`\n --> src/main.rs:4:5\nsrc/a.c:3:4: error: bad\n";
        let parsed = parse_jvm_diagnostics(out, None);
        assert!(parsed.entries.is_empty());
        assert!(parsed.consumed.iter().all(|c| !c));
    }

    #[test]
    fn failure_summary_prefers_first_error_then_gradle_block() {
        let out = "/p/A.java:4: error: boom\n  x\n  ^\n/p/A.java:5: error: bang\n  y\n  ^\n";
        let parsed = parse_jvm_diagnostics(out, None);
        assert_eq!(
            summarize_build_failure(out, &parsed.entries).as_deref(),
            Some("A.java:4: boom (+1 more)")
        );
        let gradle = "FAILURE: Build failed with an exception.\n\n* What went wrong:\nExecution failed for task ':compileJava'.\n> Compilation failed\n";
        assert_eq!(
            summarize_build_failure(gradle, &[]).as_deref(),
            Some("Execution failed for task ':compileJava'.")
        );
    }
}
