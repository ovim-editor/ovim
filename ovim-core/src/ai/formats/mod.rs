pub mod apply_patch;
pub mod matching;
pub mod str_replace;

/// A single search/replace pair within a file edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub search: String,
    pub replace: String,
}

/// Edits targeting one file. `path` is `None` for single-file / inline edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    pub path: Option<String>,
    pub hunks: Vec<Hunk>,
}

/// Result of parsing a chat edit format response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatEditParsed {
    /// Structured hunks (apply_patch, str_replace).
    Hunks(Vec<FileEdit>),
    /// Whole-file replacement (codeblock format).
    WholeFile(String),
}
