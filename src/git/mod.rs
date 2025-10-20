use anyhow::{Context, Result};
use git2::{Diff, DiffOptions, Repository};
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
}

impl GitStatus {
    /// Creates a new empty git status
    pub fn new() -> Self {
        Self {
            line_status: HashMap::new(),
        }
    }

    /// Gets the status for a given line
    pub fn get_line_status(&self, line: usize) -> Option<LineStatus> {
        self.line_status.get(&line).copied()
    }

    /// Computes git status for a file
    pub fn from_file<P: AsRef<Path>>(file_path: P) -> Result<Self> {
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

        // Get HEAD tree
        let head = match repo.head() {
            Ok(head) => head,
            Err(_) => return Ok(Self::new()), // No HEAD (empty repo)
        };

        let head_commit = match head.peel_to_commit() {
            Ok(commit) => commit,
            Err(_) => return Ok(Self::new()),
        };

        let head_tree = match head_commit.tree() {
            Ok(tree) => tree,
            Err(_) => return Ok(Self::new()),
        };

        // Create diff between HEAD and working directory
        let mut diff_opts = DiffOptions::new();
        diff_opts.pathspec(relative_path);
        diff_opts.context_lines(0); // We only need the changed lines

        let diff =
            match repo.diff_tree_to_workdir_with_index(Some(&head_tree), Some(&mut diff_opts)) {
                Ok(diff) => diff,
                Err(_) => return Ok(Self::new()),
            };

        // Parse the diff
        let mut line_status = HashMap::new();

        diff.foreach(
            &mut |_, _| true,
            None,
            None,
            Some(&mut |_delta, _hunk, line| {
                // Get the new line number (in the working copy)
                let line_num = line.new_lineno();

                match line.origin() {
                    '+' => {
                        // Added line
                        if let Some(num) = line_num {
                            line_status.insert(num as usize - 1, LineStatus::Added);
                        }
                    }
                    '-' => {
                        // Deleted line - mark the line before it
                        if let Some(num) = line.old_lineno() {
                            // Show deletion marker on the previous line in the new file
                            let old_line = num as usize - 1;
                            line_status.insert(old_line, LineStatus::Removed);
                        }
                    }
                    ' ' => {
                        // Context line - check if surrounded by changes
                        // This is a heuristic for "modified" lines
                    }
                    _ => {}
                }
                true
            }),
        )
        .ok();

        // Detect modified lines (lines that have both additions and deletions nearby)
        // This is a simple heuristic - in a real implementation you'd want more sophisticated detection
        let keys: Vec<usize> = line_status.keys().copied().collect();
        for &line in &keys {
            if let Some(status) = line_status.get(&line) {
                if *status == LineStatus::Added {
                    // Check if there's a removal nearby
                    for offset in 1..=3 {
                        if line >= offset {
                            if line_status.get(&(line - offset)) == Some(&LineStatus::Removed) {
                                // Likely a modification
                                line_status.insert(line, LineStatus::Modified);
                                break;
                            }
                        }
                        if line_status.get(&(line + offset)) == Some(&LineStatus::Removed) {
                            line_status.insert(line, LineStatus::Modified);
                            break;
                        }
                    }
                }
            }
        }

        Ok(Self { line_status })
    }
}

impl Default for GitStatus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_status() {
        let status = GitStatus::new();
        assert_eq!(status.get_line_status(0), None);
        assert_eq!(status.get_line_status(10), None);
    }
}
