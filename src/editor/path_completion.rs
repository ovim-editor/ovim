use std::path::{Path, PathBuf};

/// A single filesystem entry returned by path completion.
#[derive(Debug, Clone)]
pub struct PathEntry {
    /// Display name (file or directory name, no trailing slash).
    pub name: String,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// Whether this entry is hidden (starts with `.`).
    pub is_hidden: bool,
}

/// Completes a partial path against the filesystem.
///
/// Returns entries matching `partial` resolved relative to `cwd`.
/// Results are sorted: directories before files, non-hidden before hidden,
/// then alphabetically within each group.
pub fn complete(partial: &str, cwd: &Path) -> Vec<PathEntry> {
    let expanded = expand_tilde(partial);
    let partial_path = Path::new(&expanded);

    // Determine the directory to list and the prefix to filter by.
    let (dir, prefix) = if expanded.ends_with('/') || expanded.ends_with(std::path::MAIN_SEPARATOR)
    {
        // Trailing slash: list directory contents, no prefix filter.
        (resolve(partial_path, cwd), String::new())
    } else if let Some(parent) = partial_path.parent() {
        let file_part = partial_path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        if parent.as_os_str().is_empty() {
            // No directory component — list cwd.
            (cwd.to_path_buf(), file_part)
        } else {
            (resolve(parent, cwd), file_part)
        }
    } else {
        (cwd.to_path_buf(), String::new())
    };

    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(&dir) else {
        return entries;
    };

    let prefix_lower = prefix.to_lowercase();

    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !prefix.is_empty() && !name.to_lowercase().starts_with(&prefix_lower) {
            continue;
        }
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        let is_hidden = name.starts_with('.');
        entries.push(PathEntry {
            name,
            is_dir,
            is_hidden,
        });
    }

    entries.sort_by(|a, b| {
        // Directories first, then non-hidden before hidden, then alphabetical.
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.is_hidden.cmp(&b.is_hidden))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    entries
}

/// Expands a leading `~` to the user's home directory.
fn expand_tilde(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
}

/// Resolves a path relative to `cwd`, handling absolute paths correctly.
fn resolve(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// State for the command-line path completion popup.
#[derive(Debug, Clone)]
pub struct PathCompletionState {
    /// Completion entries for the current directory/prefix.
    entries: Vec<PathEntry>,
    /// Currently selected index (wraps around).
    selected: usize,
    /// Whether the popup is visible.
    visible: bool,
    /// The base directory path (everything before the filename prefix).
    /// Used to reconstruct the full path when accepting a completion.
    base_path: String,
    /// The original partial input that triggered completion.
    /// Used to restore when cycling past the end.
    original_input: String,
    /// Whether Tab has accepted an entry since the last `update()`.
    /// When false, the first Tab accepts the current selection without cycling.
    tab_accepted: bool,
}

impl PathCompletionState {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            selected: 0,
            visible: false,
            base_path: String::new(),
            original_input: String::new(),
            tab_accepted: false,
        }
    }

    /// Update completions for the given command line.
    /// `path_portion` is just the file path part (after `:e `).
    pub fn update(&mut self, path_portion: &str, cwd: &Path) {
        self.original_input = path_portion.to_string();
        self.entries = complete(path_portion, cwd);

        // Compute base_path: the directory portion (up to and including last `/`).
        let expanded = expand_tilde(path_portion);
        if expanded.ends_with('/') || expanded.ends_with(std::path::MAIN_SEPARATOR) {
            self.base_path = path_portion.to_string();
        } else if let Some(idx) = path_portion.rfind('/') {
            self.base_path = path_portion[..=idx].to_string();
        } else {
            self.base_path = String::new();
        }

        self.selected = 0;
        self.visible = !self.entries.is_empty();
        self.tab_accepted = false;
    }

    /// Accept the currently selected entry and return the new path portion.
    /// Appends `/` for directories.
    pub fn accept(&self) -> Option<String> {
        let entry = self.entries.get(self.selected)?;
        let mut path = format!("{}{}", self.base_path, entry.name);
        if entry.is_dir {
            path.push('/');
        }
        Some(path)
    }

    pub fn select_next(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1) % self.entries.len();
        }
    }

    pub fn select_previous(&mut self) {
        if !self.entries.is_empty() {
            if self.selected == 0 {
                self.selected = self.entries.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    pub fn tab_accepted(&self) -> bool {
        self.tab_accepted
    }

    pub fn set_tab_accepted(&mut self) {
        self.tab_accepted = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.entries.clear();
        self.selected = 0;
    }

    pub fn is_visible(&self) -> bool {
        self.visible && !self.entries.is_empty()
    }

    /// Returns whether the currently selected entry is a directory.
    pub fn selected_is_dir(&self) -> bool {
        self.entries
            .get(self.selected)
            .map(|e| e.is_dir)
            .unwrap_or(false)
    }

    pub fn entries(&self) -> &[PathEntry] {
        &self.entries
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }
}

impl Default for PathCompletionState {
    fn default() -> Self {
        Self::new()
    }
}

/// Extracts the path portion from a command line, if the command takes a file argument.
///
/// Returns `Some(path_portion)` for commands like `:e foo`, `:tabe bar`, etc.
/// Returns `None` if the command doesn't take a file argument or hasn't reached
/// the path portion yet.
pub fn extract_path_from_command(command_line: &str) -> Option<&str> {
    let trimmed = command_line.trim_start();

    // Commands that take file path arguments.
    const FILE_COMMANDS: &[&str] = &[
        "e ", "edit ", "tabe ", "tabedit ", "w ", "write ", "saveas ", "source ", "sp ", "split ",
        "vsp ", "vsplit ",
    ];

    for cmd in FILE_COMMANDS {
        if let Some(rest) = trimmed.strip_prefix(cmd) {
            return Some(rest);
        }
    }

    // Also handle bare command with no space yet — don't trigger completion.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_complete_sorts_dirs_first() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("afile.txt"), "").unwrap();
        fs::create_dir(tmp.path().join("bdir")).unwrap();
        fs::write(tmp.path().join("cfile.rs"), "").unwrap();
        fs::create_dir(tmp.path().join("ddir")).unwrap();

        let entries = complete("", tmp.path());
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        // Dirs first (bdir, ddir), then files (afile.txt, cfile.rs)
        assert_eq!(names, vec!["bdir", "ddir", "afile.txt", "cfile.rs"]);
    }

    #[test]
    fn test_complete_hidden_after_visible() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("visible.txt"), "").unwrap();
        fs::write(tmp.path().join(".hidden"), "").unwrap();
        fs::create_dir(tmp.path().join(".hidden_dir")).unwrap();
        fs::create_dir(tmp.path().join("visible_dir")).unwrap();

        let entries = complete("", tmp.path());
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        // dirs first: visible_dir, .hidden_dir; then files: visible.txt, .hidden
        assert_eq!(
            names,
            vec!["visible_dir", ".hidden_dir", "visible.txt", ".hidden"]
        );
    }

    #[test]
    fn test_complete_prefix_filter() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("main.rs"), "").unwrap();
        fs::write(tmp.path().join("mod.rs"), "").unwrap();
        fs::write(tmp.path().join("other.rs"), "").unwrap();

        let entries = complete("m", tmp.path());
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["main.rs", "mod.rs"]);
    }

    #[test]
    fn test_complete_directory_trailing_slash() {
        let tmp = tempfile::tempdir().unwrap();
        let subdir = tmp.path().join("src");
        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("lib.rs"), "").unwrap();
        fs::write(subdir.join("main.rs"), "").unwrap();

        let entries = complete("src/", tmp.path());
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["lib.rs", "main.rs"]);
    }

    #[test]
    fn test_extract_path_from_command() {
        assert_eq!(extract_path_from_command("e src/main.rs"), Some("src/main.rs"));
        assert_eq!(extract_path_from_command("edit foo.txt"), Some("foo.txt"));
        assert_eq!(extract_path_from_command("tabe bar"), Some("bar"));
        assert_eq!(extract_path_from_command("w new_file.txt"), Some("new_file.txt"));
        assert_eq!(extract_path_from_command("saveas backup.rs"), Some("backup.rs"));
        assert_eq!(extract_path_from_command("sp file"), Some("file"));
        assert_eq!(extract_path_from_command("vsp file"), Some("file"));
        // No match
        assert_eq!(extract_path_from_command("set number"), None);
        assert_eq!(extract_path_from_command("q"), None);
        // Command typed but no space yet
        assert_eq!(extract_path_from_command("e"), None);
    }

    #[test]
    fn test_expand_tilde() {
        let expanded = expand_tilde("~/foo");
        assert!(expanded.ends_with("/foo"));
        assert!(!expanded.starts_with('~'));

        // Non-tilde path unchanged
        assert_eq!(expand_tilde("./bar"), "./bar");
        assert_eq!(expand_tilde("/absolute"), "/absolute");
    }

    #[test]
    fn test_state_accept_appends_slash_for_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("src")).unwrap();
        fs::write(tmp.path().join("main.rs"), "").unwrap();

        let mut state = PathCompletionState::new();
        state.update("", tmp.path());

        // First entry should be the dir "src"
        assert!(state.entries[0].is_dir);
        let accepted = state.accept().unwrap();
        assert!(accepted.ends_with('/'));
    }

    #[test]
    fn test_nonexistent_path_empty() {
        let entries = complete("nonexistent_dir_12345/", Path::new("/"));
        assert!(entries.is_empty());
    }
}
