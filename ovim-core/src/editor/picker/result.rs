use crate::git::ops::GitTarget;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum PickerMode {
    FindFiles,
    LiveGrep,
    Custom,
    Completion,
    LspLocations,
}

/// Action to execute when a picker result is selected (Enter key).
/// Decouples the selection logic from the mode-switching dispatch.
#[derive(Debug, Clone)]
pub enum PickerAction {
    /// Open a file at a specific position
    OpenFile {
        path: String,
        line: usize,
        col: usize,
    },
    /// Open a file at a specific position and push to the tag stack (Ctrl-T navigation)
    OpenFileWithTag {
        path: String,
        line: usize,
        col: usize,
    },
    /// Apply a code action by index
    ApplyCodeAction { index: usize },
    /// Apply a completion by index
    ApplyCompletion { index: usize },
    /// Select a debug run configuration by index
    SelectDebugConfig { index: usize },
    /// Answer a server `window/showMessageRequest` with action `index`
    MessageRequestAction { index: usize },
    /// Open a git view for a status / history entry
    Git(GitPick),
}

/// What Enter does on a git status / history entry. Entries carry the path
/// itself rather than an ex command line: file names may contain `|` or
/// spaces, which an ex parser would split into further commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitPick {
    /// The file's diff in the review of uncommitted changes.
    DiffFile(GitTarget),
    /// The file itself.
    Edit(GitTarget),
    /// A commit's diff in the review, positioned on `path` when known.
    Show {
        root: PathBuf,
        oid: String,
        path: Option<String>,
    },
    /// The review of all uncommitted changes.
    DiffHead,
}

impl GitPick {
    /// The file the entry stands for, when it is about a single file.
    pub fn target(&self) -> Option<&GitTarget> {
        match self {
            Self::DiffFile(target) | Self::Edit(target) => Some(target),
            Self::Show { .. } | Self::DiffHead => None,
        }
    }
}

/// What a picker is for, when it needs bindings beyond select/cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerRole {
    /// `Ctrl-T` stages/unstages the selected file, `Ctrl-E` opens it.
    GitStatus,
    /// `Ctrl-T` cycles the severity filter.
    Problems,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickerField {
    Query,
    FileFilter,
}

#[derive(Debug, Clone)]
pub struct PickerResult {
    /// Display text for the result
    pub display: String,
    /// File path (for FindFiles) or file:line:col (for LiveGrep)
    pub location: String,
    /// Line number (for LiveGrep, 0 for FindFiles)
    pub line: usize,
    /// Column number (for LiveGrep, 0 for FindFiles)
    pub col: usize,
    /// Character indices in `display` that matched the query
    pub match_positions: Vec<usize>,
    /// Matched content (for LiveGrep) — displayed separately from the location
    pub content: Option<String>,
}
