//! Type conversions and helpers for LSP
//!
//! Provides conversions between ovim's internal types and LSP types.

use lsp_types::{Position, Range, Uri};
use std::path::Path;

/// Converts a file path to an LSP Uri
/// Uses url crate for proper encoding, then parses as lsp_types::Uri
pub fn uri_from_file_path<P: AsRef<Path>>(path: P) -> Option<Uri> {
    let url = url::Url::from_file_path(path).ok()?;
    url.as_str().parse().ok()
}

/// Converts an LSP Uri to a file path
/// Returns None if the Uri is not a file:// URI or cannot be converted
pub fn uri_to_file_path(uri: &Uri) -> Option<std::path::PathBuf> {
    let uri_str = uri.as_str();
    let url = url::Url::parse(uri_str).ok()?;
    url.to_file_path().ok()
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
