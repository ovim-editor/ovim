//! Type conversions and helpers for LSP
//!
//! Provides conversions between ovim's internal types and LSP types.

use lsp_types::{Position, Range, Uri};
use std::path::Path;

/// Converts a file path to an LSP Uri
/// Uses URL file-path encoding, then escapes the additional characters that
/// RFC 3986 (used by LSP's Uri) forbids in paths but WHATWG URLs allow.
pub fn uri_from_file_path<P: AsRef<Path>>(path: P) -> Option<Uri> {
    let mut url = url::Url::from_file_path(path).ok()?;
    // Only rewrite the path: brackets can be valid in an authority. set_path
    // preserves existing percent escapes, including literal '%' in filenames.
    let path = url
        .path()
        .replace('[', "%5B")
        .replace(']', "%5D")
        .replace('^', "%5E")
        .replace('|', "%7C");
    url.set_path(&path);
    url.as_str().parse().ok()
}

/// Converts an LSP Uri to a file path
/// Returns None if the Uri is not a file:// URI or cannot be converted
pub fn uri_to_file_path(uri: &Uri) -> Option<std::path::PathBuf> {
    let uri_str = uri.as_str();
    let url = url::Url::parse(uri_str).ok()?;
    url.to_file_path().ok()
}

/// Reject reversed ranges at the diagnostic ingestion boundary.
pub(crate) fn diagnostic_range_is_valid(range: &Range) -> bool {
    (range.start.line, range.start.character) <= (range.end.line, range.end.character)
}

/// Whether a diagnostic covers a line. Ends are exclusive; an empty range
/// remains reachable on its start line. No expansion by span length is needed.
pub fn diagnostic_covers_line(diagnostic: &lsp_types::Diagnostic, line: usize) -> bool {
    let range = &diagnostic.range;
    diagnostic_range_is_valid(range)
        && line >= range.start.line as usize
        && (line < range.end.line as usize
            || (line == range.end.line as usize
                && (range.end.character > 0 || range.start.line == range.end.line)))
}

/// Clip a diagnostic to a line's visible content, converting its UTF-16
/// endpoints to a half-open scalar range. Continuation lines start at zero
/// and end at the line length. Out-of-bounds columns clamp to the content.
pub fn diagnostic_char_range(
    diagnostic: &lsp_types::Diagnostic,
    line: usize,
    text: &str,
) -> Option<std::ops::Range<usize>> {
    if !diagnostic_covers_line(diagnostic, line) {
        return None;
    }
    let start = if line == diagnostic.range.start.line as usize {
        super::utf16_to_char_col(text, diagnostic.range.start.character)
    } else {
        0
    };
    let end = if line == diagnostic.range.end.line as usize {
        super::utf16_to_char_col(text, diagnostic.range.end.character)
    } else {
        text.chars().count()
    };
    Some(start..end)
}

/// LSP Position wrapper for easier construction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

impl LspPosition {
    pub fn new(line: usize, character: usize) -> Self {
        Self {
            line: line as u32,
            character: character as u32,
        }
    }

    pub fn to_lsp(&self) -> Position {
        Position {
            line: self.line,
            character: self.character,
        }
    }

    pub fn from_lsp(pos: Position) -> Self {
        Self {
            line: pos.line,
            character: pos.character,
        }
    }
}

/// LSP Range wrapper for easier construction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

impl LspRange {
    pub fn new(start: LspPosition, end: LspPosition) -> Self {
        Self { start, end }
    }

    pub fn to_lsp(&self) -> Range {
        Range {
            start: self.start.to_lsp(),
            end: self.end.to_lsp(),
        }
    }

    pub fn from_lsp(range: Range) -> Self {
        Self {
            start: LspPosition::from_lsp(range.start),
            end: LspPosition::from_lsp(range.end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_round_trips_route_and_special_character_paths() {
        let root = std::env::temp_dir();
        for relative in [
            "app/[id]/page.tsx",
            "app/[[...slug]]/page.tsx",
            "app/(group)/café space #100%/page.tsx",
            "app/caret^pipe|/page.tsx",
            "app/literal%5Bid%5D/page.tsx",
        ] {
            let path = root.join(relative);
            let uri = uri_from_file_path(&path)
                .unwrap_or_else(|| panic!("Cannot convert {} to LSP URI", path.display()));
            assert_eq!(uri_to_file_path(&uri).as_deref(), Some(path.as_path()));
            // The document identifier must survive the JSON-RPC wire format.
            let json = serde_json::to_string(&uri).unwrap();
            assert_eq!(serde_json::from_str::<Uri>(&json).unwrap(), uri);
        }
    }

    #[test]
    fn test_position_conversion() {
        let pos = LspPosition::new(10, 5);
        assert_eq!(pos.line, 10);
        assert_eq!(pos.character, 5);

        let lsp_pos = pos.to_lsp();
        assert_eq!(lsp_pos.line, 10);
        assert_eq!(lsp_pos.character, 5);

        let back = LspPosition::from_lsp(lsp_pos);
        assert_eq!(back, pos);
    }

    #[test]
    fn test_range_conversion() {
        let start = LspPosition::new(1, 2);
        let end = LspPosition::new(3, 4);
        let range = LspRange::new(start, end);

        let lsp_range = range.to_lsp();
        assert_eq!(lsp_range.start.line, 1);
        assert_eq!(lsp_range.start.character, 2);
        assert_eq!(lsp_range.end.line, 3);
        assert_eq!(lsp_range.end.character, 4);

        let back = LspRange::from_lsp(lsp_range);
        assert_eq!(back, range);
    }
}
