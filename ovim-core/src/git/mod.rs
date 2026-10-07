pub mod conflict;
pub mod ops;

use anyhow::Result;
use chrono::{TimeZone, Utc};
use git2::{DiffOptions, Oid, Repository};
use std::collections::HashMap;
use std::path::Path;

/// Represents the status of a line in the git diff
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStatus {
    /// Line was added
    Added,
    /// Line was modified
    Modified,
    /// Line was deleted (shown on the line before)
    Removed,
}

/// Git status information for a file
#[derive(Debug, Clone)]
pub struct GitStatus {
    /// Map of line number (0-indexed) to status
    line_status: HashMap<usize, LineStatus>,
    // Deleted lines can share one visible anchor; count changes before they
    // are projected onto gutter rows.
    counts: (usize, usize, usize),
}

impl GitStatus {
    /// Creates a new empty git status
    pub fn new() -> Self {
        Self {
            line_status: HashMap::new(),
            counts: (0, 0, 0),
        }
    }

    /// Gets the status for a given line
    pub fn get_line_status(&self, line: usize) -> Option<LineStatus> {
        self.line_status.get(&line).copied()
    }

    /// First line (0-indexed) of every run of consecutive changed lines,
    /// sorted ascending. Used for `]c` / `[c` hunk navigation.
    pub fn hunk_starts(&self) -> Vec<usize> {
        let mut lines: Vec<usize> = self.line_status.keys().copied().collect();
        lines.sort_unstable();
        let mut starts = Vec::new();
        let mut previous: Option<usize> = None;
        for line in lines {
            if previous.is_none_or(|prev| line != prev + 1) {
                starts.push(line);
            }
            previous = Some(line);
        }
        starts
    }

    /// Returns (added, modified, removed) line counts.
    pub fn change_counts(&self) -> (usize, usize, usize) {
        self.counts
    }

    /// Computes git status for a file
    pub fn from_file<P: AsRef<Path>>(file_path: P) -> Result<Self> {
        Self::from_file_with_pullbase(file_path, None)
    }

    /// Compare against HEAD by default, or the configured branch's merge-base.
    pub fn from_file_with_pullbase<P: AsRef<Path>>(
        file_path: P,
        branch: Option<&str>,
    ) -> Result<Self> {
        let file_path = file_path.as_ref();

        // Find the git repository
        let repo = match Repository::discover(file_path) {
            Ok(repo) => repo,
            Err(_) => return Ok(Self::new()), // Not in a git repo
        };

        // Get the workdir
        let workdir = match repo.workdir() {
            Some(dir) => dir,
            None => return Ok(Self::new()), // Bare repo
        };

        // Get relative path from repo root
        let relative_path = match file_path.strip_prefix(workdir) {
            Ok(p) => p,
            Err(_) => return Ok(Self::new()),
        };

        // An unborn repository compares against the empty tree. A configured
        // pull base still needs a real HEAD to resolve its merge-base.
        let head_commit = match repo.head() {
            Ok(head) => Some(head.peel_to_commit()?),
            Err(error) if error.code() == git2::ErrorCode::UnbornBranch => None,
            Err(error) => return Err(error.into()),
        };
        let base_commit = if let Some(branch) = branch {
            let base = crate::native_diff::resolve_pullbase(file_path, Some(branch))?;
            let base_oid = repo
                .revparse_single(base.base_ref().unwrap())?
                .peel_to_commit()?
                .id();
            let head = head_commit.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Cannot resolve a pull base before the first commit")
            })?;
            Some(repo.find_commit(repo.merge_base(head.id(), base_oid)?)?)
        } else {
            head_commit
        };
        let head_tree = base_commit
            .as_ref()
            .map(|commit| commit.tree())
            .transpose()?;

        let mut diff_opts = DiffOptions::new();
        diff_opts
            .pathspec(relative_path)
            .context_lines(0)
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .show_untracked_content(true);
        let diff =
            repo.diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut diff_opts))?;

        // Zero-context hunks supply coordinates in the working file. Old line
        // numbers cannot locate deletions after earlier insertions/removals.
        // Pair replacements within their own hunk, never by proximity to a
        // different change (which also misclassifies isolated additions).
        let mut line_status = HashMap::new();
        let mut counts = (0, 0, 0);
        diff.foreach(
            &mut |_, _| true,
            None,
            Some(&mut |_, hunk| {
                let modified = hunk.old_lines().min(hunk.new_lines());
                counts.0 += (hunk.new_lines() - modified) as usize;
                counts.1 += modified as usize;
                counts.2 += (hunk.old_lines() - modified) as usize;
                let start = hunk.new_start().saturating_sub(1) as usize;
                if hunk.new_lines() == 0 {
                    // A deletion sits after new_start; at BOF/empty files the
                    // first editor row is the only available anchor.
                    line_status.entry(start).or_insert(LineStatus::Removed);
                } else {
                    for offset in 0..hunk.new_lines() {
                        let status = if offset < hunk.old_lines() {
                            LineStatus::Modified
                        } else {
                            LineStatus::Added
                        };
                        line_status.insert(start + offset as usize, status);
                    }
                }
                true
            }),
            None,
        )?;

        Ok(Self {
            line_status,
            counts,
        })
    }
}

impl Default for GitStatus {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Git Blame
// ---------------------------------------------------------------------------

/// Blame information for a single line
#[derive(Debug, Clone)]
pub struct LineBlameInfo {
    /// Full 40-char hex OID
    pub commit_oid: String,
    /// Short commit hash (5 chars)
    pub commit_hash: String,
    /// Author name
    pub author: String,
    /// Commit timestamp (Unix epoch seconds)
    pub timestamp: i64,
}

/// Metadata for a single commit
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub oid_hex: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    pub body: String,
}

/// Git blame data for an entire file
#[derive(Debug, Clone)]
pub struct GitBlame {
    /// Blame info indexed by 0-based line number
    lines: Vec<Option<LineBlameInfo>>,
}

impl GitBlame {
    /// Computes blame for a file using git2
    pub fn from_file<P: AsRef<Path>>(file_path: P) -> Result<Self> {
        let file_path = file_path.as_ref();

        let repo = match Repository::discover(file_path) {
            Ok(repo) => repo,
            Err(_) => return Ok(Self { lines: Vec::new() }),
        };

        let workdir = match repo.workdir() {
            Some(dir) => dir,
            None => return Ok(Self { lines: Vec::new() }),
        };

        let relative_path = match file_path.strip_prefix(workdir) {
            Ok(p) => p,
            Err(_) => return Ok(Self { lines: Vec::new() }),
        };

        let blame = match repo.blame_file(relative_path, None) {
            Ok(b) => b,
            Err(_) => return Ok(Self { lines: Vec::new() }),
        };

        let mut lines = Vec::new();
        for hunk_idx in 0..blame.len() {
            if let Some(hunk) = blame.get_index(hunk_idx) {
                let commit_id = hunk.final_commit_id();
                let oid_hex = format!("{}", commit_id);
                let hash = oid_hex[..5.min(oid_hex.len())].to_string();
                let sig = hunk.final_signature();
                let author = sig.name().unwrap_or("Unknown").to_string();
                let timestamp = sig.when().seconds();
                let start = hunk.final_start_line(); // 1-indexed
                let count = hunk.lines_in_hunk();

                // Ensure vec is large enough
                let end = start + count;
                if end > lines.len() {
                    lines.resize(end, None);
                }

                for i in 0..count {
                    let line_idx = start - 1 + i; // convert to 0-indexed
                    lines[line_idx] = Some(LineBlameInfo {
                        commit_oid: oid_hex.clone(),
                        commit_hash: hash.clone(),
                        author: author.clone(),
                        timestamp,
                    });
                }
            }
        }

        Ok(Self { lines })
    }

    /// Gets blame info for a 0-indexed line
    pub fn get(&self, line: usize) -> Option<&LineBlameInfo> {
        self.lines.get(line).and_then(|o| o.as_ref())
    }

    /// Number of lines with blame data
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Returns true if there is no blame data
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Returns the maximum author name length across all lines
    pub fn max_author_len(&self) -> usize {
        self.lines
            .iter()
            .filter_map(|o| o.as_ref())
            .map(|info| info.author.len())
            .max()
            .unwrap_or(0)
    }
}

/// Returns the current git branch name for a file path.
/// Returns `None` for non-git files. Uses short OID for detached HEAD.
pub fn branch_name<P: AsRef<Path>>(file_path: P) -> Option<String> {
    let repo = Repository::discover(file_path.as_ref()).ok()?;
    let head = repo.head().ok()?;
    if head.is_branch() {
        head.shorthand().map(|s| s.to_string())
    } else {
        // Detached HEAD — show short OID
        head.target()
            .map(|oid| format!("{}", oid))
            .map(|s| s[..7.min(s.len())].to_string())
    }
}

/// Returns true if the OID is all zeros (uncommitted line).
pub fn is_zero_oid(oid_hex: &str) -> bool {
    oid_hex.chars().all(|c| c == '0')
}

/// Looks up commit metadata for a given OID hex string.
pub fn commit_info<P: AsRef<Path>>(file_path: P, oid_hex: &str) -> Result<CommitInfo> {
    if is_zero_oid(oid_hex) {
        return Ok(CommitInfo {
            oid_hex: oid_hex.to_string(),
            author: String::new(),
            date: String::new(),
            subject: "Not yet committed".to_string(),
            body: String::new(),
        });
    }

    let file_path = file_path.as_ref();
    let repo = Repository::discover(file_path)?;
    let oid = Oid::from_str(oid_hex)?;
    let commit = repo.find_commit(oid)?;

    let author = commit.author().name().unwrap_or("Unknown").to_string();
    let time = commit.author().when();
    let dt = Utc.timestamp_opt(time.seconds(), 0).single();
    let date = dt
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default();
    let message = commit.message().unwrap_or("").to_string();
    let mut lines = message.lines();
    let subject = lines.next().unwrap_or("").to_string();
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();

    Ok(CommitInfo {
        oid_hex: oid_hex.to_string(),
        author,
        date,
        subject,
        body,
    })
}

/// Returns a unified diff for a commit (compared to its first parent).
pub fn commit_diff<P: AsRef<Path>>(file_path: P, oid_hex: &str) -> Result<String> {
    if is_zero_oid(oid_hex) {
        return Ok("Not yet committed".to_string());
    }

    let file_path = file_path.as_ref();
    let repo = Repository::discover(file_path)?;
    let oid = Oid::from_str(oid_hex)?;
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;

    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };

    // Without explicit prefixes libgit2 honours the repository's
    // `diff.mnemonicprefix`, which renames `a/` and `b/` after whatever each
    // side is (`c/` for a commit, `i/` for the index, `w/` for the worktree).
    // This patch is shown to the user and highlighted with the diff grammar,
    // so it should read the same on every machine rather than varying with
    // whatever the reader happens to have in their gitconfig.
    // `native_diff::build_diff` pins the prefixes for the same reason.
    let mut options = DiffOptions::new();
    options.old_prefix("a").new_prefix("b");
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut options))?;

    let mut patch = String::new();

    // Header
    let author_sig = commit.author();
    let author = author_sig.name().unwrap_or("Unknown");
    let time = author_sig.when();
    let dt = Utc.timestamp_opt(time.seconds(), 0).single();
    let date = dt
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default();
    let message = commit.message().unwrap_or("");
    let subject = message.lines().next().unwrap_or("");

    patch.push_str(&format!("commit {}\n", oid_hex));
    patch.push_str(&format!("Author: {}\n", author));
    patch.push_str(&format!("Date:   {}\n", date));
    patch.push_str(&format!("\n    {}\n\n", subject));

    // Diff content
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let origin = line.origin();
        match origin {
            '+' | '-' | ' ' => patch.push(origin),
            _ => {}
        }
        if let Ok(content) = std::str::from_utf8(line.content()) {
            patch.push_str(content);
        }
        true
    })?;

    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `diff.mnemonicprefix` replaces `a/` and `b/` with a letter naming each
    /// side (`c/` for a commit, `i/` for the index, `w/` for the worktree).
    /// libgit2 honours it, so before the prefixes were pinned this patch came
    /// out as `diff --git c/a.txt c/a.txt` for anyone who had the option set
    /// globally. The patch is parsed and highlighted as a diff downstream and
    /// is shown to the user, so it has to read the same everywhere.
    #[test]
    fn commit_patch_ignores_the_repository_diff_prefix_config() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp.path()).unwrap();
        repo.config()
            .unwrap()
            .set_bool("diff.mnemonicprefix", true)
            .unwrap();

        let file = temp.path().join("a.txt");
        std::fs::write(&file, "one\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("a.txt")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = git2::Signature::now("Ovim", "ovim@example.com").unwrap();
        let oid = repo
            .commit(Some("HEAD"), &signature, &signature, "c1", &tree, &[])
            .unwrap();

        let patch = commit_diff(&file, &oid.to_string()).unwrap();

        assert!(
            patch.contains("diff --git a/a.txt b/a.txt"),
            "commit patch inherited the repo's diff prefix config: {patch}"
        );
    }

    #[test]
    fn test_empty_status() {
        let status = GitStatus::new();
        assert_eq!(status.get_line_status(0), None);
        assert_eq!(status.get_line_status(10), None);
    }

    #[test]
    fn hunk_starts_groups_consecutive_lines() {
        let mut status = GitStatus::new();
        for line in [3, 4, 5, 9, 12, 13] {
            status.line_status.insert(line, LineStatus::Added);
        }
        assert_eq!(status.hunk_starts(), vec![3, 9, 12]);
        assert!(GitStatus::new().hunk_starts().is_empty());
    }

    #[test]
    fn test_empty_blame() {
        let blame = GitBlame { lines: Vec::new() };
        assert!(blame.is_empty());
        assert_eq!(blame.line_count(), 0);
        assert!(blame.get(0).is_none());
    }

    #[test]
    fn test_is_zero_oid() {
        assert!(is_zero_oid("0000000000000000000000000000000000000000"));
        assert!(is_zero_oid("00000"));
        assert!(!is_zero_oid("abc12"));
        assert!(!is_zero_oid("a000000000000000000000000000000000000000"));
    }
}
