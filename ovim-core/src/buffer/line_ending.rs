use std::borrow::Cow;

/// Normalize line endings for content about to be inserted into the rope.
///
/// The internal rope is LF-only by convention. External text from paste
/// buffers, LSP `TextEdit`s, completion items, and AI tool outputs may
/// contain `\r\n` (Windows) or bare `\r` (Mac-classic, terminal scrollback,
/// mixed-ending files). Without normalization those CRs propagate into the
/// rope and render as `^M` artifacts that also corrupt motion bounds and
/// search/regex matches.
///
/// Rules:
/// - `\r\n` → `\n`
/// - bare `\r` → `\n` (treat as a line break, matching Vim/VS Code paste)
///
/// Returns `Cow::Borrowed` (no allocation) when the input contains no `\r`.
pub fn normalize_for_buffer(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            out.push('\n');
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Line ending style detected for the buffer.
///
/// The rope stores line breaks as LF internally. `Cr` is converted back to bare
/// CR on save for Mac-classic files. `Mixed` cannot be losslessly represented
/// once normalized into the rope, so saving writes LF-only content rather than
/// preserving a corrupt per-line mixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// Unix-style line endings (LF, \n)
    #[default]
    Lf,
    /// Windows-style line endings (CRLF, \r\n)
    Crlf,
    /// Classic Mac-style line endings (CR, \r)
    Cr,
    /// More than one line-ending style was found in the same file.
    Mixed,
}

impl LineEnding {
    /// Detects the line ending style from file content bytes
    pub fn detect(content: &[u8]) -> Self {
        let mut saw_lf = false;
        let mut saw_crlf = false;
        let mut saw_cr = false;

        let mut i = 0;
        while i < content.len() {
            match content[i] {
                b'\r' if content.get(i + 1) == Some(&b'\n') => {
                    saw_crlf = true;
                    i += 2;
                }
                b'\r' => {
                    saw_cr = true;
                    i += 1;
                }
                b'\n' => {
                    saw_lf = true;
                    i += 1;
                }
                _ => i += 1,
            }
        }

        match (saw_lf, saw_crlf, saw_cr) {
            (false, false, false) | (true, false, false) => LineEnding::Lf,
            (false, true, false) => LineEnding::Crlf,
            (false, false, true) => LineEnding::Cr,
            _ => LineEnding::Mixed,
        }
    }

    /// Returns the string representation of this line ending
    pub fn as_str(&self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
            LineEnding::Cr => "\r",
            LineEnding::Mixed => "\n",
        }
    }

    /// Returns a short display name for the status line
    pub fn display_name(&self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::Crlf => "CRLF",
            LineEnding::Cr => "CR",
            LineEnding::Mixed => "MIXED",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_for_buffer, LineEnding};

    #[test]
    fn detect_defaults_to_lf_for_empty_or_no_newlines() {
        assert_eq!(LineEnding::detect(b""), LineEnding::Lf);
        assert_eq!(LineEnding::detect(b"single line"), LineEnding::Lf);
    }

    #[test]
    fn detect_lf_only() {
        assert_eq!(LineEnding::detect(b"a\nb\nc"), LineEnding::Lf);
    }

    #[test]
    fn detect_crlf_only() {
        assert_eq!(LineEnding::detect(b"a\r\nb\r\nc"), LineEnding::Crlf);
    }

    #[test]
    fn detect_bare_cr_only() {
        assert_eq!(LineEnding::detect(b"a\rb\rc"), LineEnding::Cr);
    }

    #[test]
    fn detect_mixed_crlf_and_lf() {
        assert_eq!(LineEnding::detect(b"a\r\nb\nc"), LineEnding::Mixed);
    }

    #[test]
    fn detect_mixed_crlf_and_bare_cr() {
        assert_eq!(LineEnding::detect(b"a\r\nb\rc"), LineEnding::Mixed);
    }

    #[test]
    fn detect_mixed_lf_and_bare_cr() {
        assert_eq!(LineEnding::detect(b"a\nb\rc"), LineEnding::Mixed);
    }

    #[test]
    fn lf_only_input_is_borrowed() {
        let s = "hello\nworld\n";
        let out = normalize_for_buffer(s);
        assert_eq!(out, "hello\nworld\n");
        assert!(matches!(out, std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn crlf_collapses_to_lf() {
        let s = "a\r\nb\r\nc";
        assert_eq!(normalize_for_buffer(s), "a\nb\nc");
    }

    #[test]
    fn bare_cr_becomes_lf() {
        let s = "a\rb\rc";
        assert_eq!(normalize_for_buffer(s), "a\nb\nc");
    }

    #[test]
    fn mixed_endings_all_become_lf() {
        let s = "a\r\nb\rc\nd";
        assert_eq!(normalize_for_buffer(s), "a\nb\nc\nd");
    }

    #[test]
    fn trailing_cr_becomes_lf() {
        // A bare \r at the end is still a line break, not a stray.
        assert_eq!(normalize_for_buffer("hello\r"), "hello\n");
    }

    #[test]
    fn double_cr_produces_two_lfs() {
        // \r\r is two bare CRs, not CRLF — produce two LFs.
        assert_eq!(normalize_for_buffer("a\r\rb"), "a\n\nb");
    }

    #[test]
    fn cr_then_crlf_produces_two_lfs() {
        // First \r is bare (next char is \r), emits LF + skips nothing.
        // Then \r\n is consumed as a unit.
        assert_eq!(normalize_for_buffer("a\r\r\nb"), "a\n\nb");
    }

    #[test]
    fn preserves_unicode_around_cr() {
        // CR is ASCII, never inside a multi-byte UTF-8 sequence — but verify
        // we didn't accidentally byte-iterate and corrupt non-ASCII content.
        assert_eq!(normalize_for_buffer("café\r\ndéjà"), "café\ndéjà");
        assert_eq!(normalize_for_buffer("👨‍👩‍👧‍👦\r\nx"), "👨‍👩‍👧‍👦\nx");
    }

    /// Bug-class regression guard for roadmap 18.
    ///
    /// The migration to `Buffer::line_text` closes the line-ending bug
    /// class structurally — so long as line content is read through that
    /// accessor. Direct `<string>.trim_end_matches('\n')` calls bypass the
    /// canonical seam; each one is a place where a `\r` could leak past.
    ///
    /// This test pins the count of remaining calls. Bumping the bound
    /// requires the dev to look at the new site, decide whether it's
    /// genuinely live (working with raw rope slices, shell output, AI
    /// extraction) or whether it should route through `line_text` instead.
    /// Either choice is fine — the point is to force the conversation.
    #[test]
    fn trim_end_matches_n_count_is_bounded() {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let core_src = manifest.join("src");
        let bin_src = manifest.parent().expect("workspace root").join("ovim/src");

        let mut sites = Vec::new();
        collect_trim_sites(&core_src, &mut sites);
        collect_trim_sites(&bin_src, &mut sites);

        // Update this when adding a genuinely-live call (and document why
        // in the commit / a comment at the call site). Routing through
        // `Buffer::line_text` is preferred for any line-content read.
        const KNOWN_LIVE_BOUND: usize = 10;

        if sites.len() > KNOWN_LIVE_BOUND {
            let mut listing = String::new();
            for (path, line) in &sites {
                listing.push_str(&format!("\n  {}:{}", path.display(), line));
            }
            panic!(
                "trim_end_matches('\\n') count is {} (bound: {}). \
                 Roadmap 18 closed the line-ending bug class via Buffer::line_text \
                 — new direct calls re-open it. Either route through line_text or \
                 bump KNOWN_LIVE_BOUND in this test with a justification.\n\
                 Sites:{}",
                sites.len(),
                KNOWN_LIVE_BOUND,
                listing
            );
        }
    }

    fn collect_trim_sites(root: &std::path::Path, out: &mut Vec<(std::path::PathBuf, usize)>) {
        fn walk(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, usize)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    // Skip this very file — the search string appears in
                    // the test source and would self-match.
                    if path.ends_with("buffer/line_ending.rs") {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        for (line_idx, line) in content.lines().enumerate() {
                            if line.contains("trim_end_matches('\\n')") {
                                out.push((path.clone(), line_idx + 1));
                            }
                        }
                    }
                }
            }
        }
        walk(root, out);
    }
}
