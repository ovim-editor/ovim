//! Git operations behind the editor's staging, commit and history commands.
//!
//! Everything goes through libgit2, so it needs no `git` binary and behaves the
//! same in the TUI, the GUI and headless sessions. Functions take any path
//! inside the repository and work on the repository's working tree.
//!
//! Committing prefers the `git` binary, which libgit2 cannot stand in for:
//! hooks, signing and the merge / cherry-pick / revert bookkeeping are git's.

use anyhow::{anyhow, bail, Context, Result};
use chrono::{TimeZone, Utc};
use git2::{
    BlameOptions, DiffOptions, ErrorCode, IndexAddOption, Oid, Repository, Status, StatusOptions,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Index flag of files a sparse checkout left out of the working tree.
const SKIP_WORKTREE: u16 = 1 << 14;

/// Opens the repository containing `path` and returns it with the path made
/// relative to the working tree.
fn open(path: &Path) -> Result<(Repository, PathBuf)> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    // Deleted files (and deleted parent directories) are valid Git targets.
    // Discover from the closest surviving ancestor, retaining the missing
    // suffix and canonicalizing any symlink spelling of the repository root.
    let existing = absolute
        .ancestors()
        .find(|ancestor| ancestor.exists())
        .ok_or_else(|| anyhow!("{} has no existing ancestor", path.display()))?;
    let canonical = existing.canonicalize()?;
    let suffix = absolute.strip_prefix(existing)?;
    let absolute = if suffix.as_os_str().is_empty() {
        canonical.clone()
    } else {
        canonical.join(suffix)
    };
    let repo = Repository::discover(&canonical)
        .with_context(|| format!("{} is not inside a Git repository", path.display()))?;
    let workdir = repo
        .workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("bare repositories have no working tree"))?;
    let workdir = workdir.canonicalize().unwrap_or(workdir);
    let relative = absolute
        .strip_prefix(&workdir)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| PathBuf::from(path));
    Ok((repo, relative))
}

/// The repository's working tree root for a path inside it.
pub fn workdir_of(path: &Path) -> Result<PathBuf> {
    let (repo, _) = open(path)?;
    canonical_workdir(&repo)
}

fn canonical_workdir(repo: &Repository) -> Result<PathBuf> {
    let workdir = repo
        .workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("bare repositories have no working tree"))?;
    Ok(workdir.canonicalize().unwrap_or(workdir))
}

/// A path resolved against its repository once: the working tree root and the
/// path relative to it. Picker rows keep one, so acting on a row reopens the
/// repository the row was listed from instead of discovering it again from an
/// absolute path (which finds a submodule's own repository for a submodule
/// row, and nothing at all for a deleted file's directory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitTarget {
    pub root: PathBuf,
    pub relative: PathBuf,
}

impl GitTarget {
    pub fn resolve(path: &Path) -> Result<Self> {
        let (repo, relative) = open(path)?;
        Ok(Self {
            root: canonical_workdir(&repo)?,
            relative,
        })
    }

    pub fn in_root(root: &Path, relative: impl Into<PathBuf>) -> Self {
        Self {
            root: root.to_path_buf(),
            relative: relative.into(),
        }
    }

    pub fn absolute(&self) -> PathBuf {
        self.root.join(&self.relative)
    }

    fn open(&self) -> Result<Repository> {
        Repository::open(&self.root)
            .with_context(|| format!("{} is not a Git working tree", self.root.display()))
    }
}

/// Paths a sparse checkout left out of the working tree. Git treats them as
/// unchanged whatever the working tree holds; libgit2 reports them deleted.
pub(crate) fn skip_worktree_paths(repo: &Repository) -> Result<HashSet<PathBuf>> {
    let index = repo.index()?;
    Ok(index
        .iter()
        .filter(|entry| entry.flags & 0x3000 == 0 && entry.flags_extended & SKIP_WORKTREE != 0)
        .filter_map(|entry| String::from_utf8(entry.path).ok())
        .map(PathBuf::from)
        .collect())
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// One changed path, as `git status --short` would list it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// Path relative to the working tree.
    pub path: String,
    /// Index column: `M`, `A`, `D`, `R`, `T` or a space.
    pub index: char,
    /// Working tree column: `M`, `D`, `T`, `?` (untracked) or a space.
    pub worktree: char,
    pub conflicted: bool,
    /// Original path for renames.
    pub renamed_from: Option<String>,
}

impl StatusEntry {
    pub fn is_staged(&self) -> bool {
        self.index != ' ' && self.index != '?' && !self.conflicted
    }

    pub fn has_unstaged_changes(&self) -> bool {
        self.worktree != ' ' || self.conflicted
    }

    /// `XY` as in `git status --short` (`UU` for conflicts).
    pub fn code(&self) -> String {
        if self.conflicted {
            "UU".to_string()
        } else {
            format!("{}{}", self.index, self.worktree)
        }
    }
}

pub fn status(path: &Path) -> Result<Vec<StatusEntry>> {
    let (repo, _) = open(path)?;
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true)
        .include_ignored(false);
    let statuses = repo.statuses(Some(&mut options))?;
    let sparse = skip_worktree_paths(&repo)?;
    let mut entries = Vec::new();
    for entry in statuses.iter() {
        let mut flags = entry.status();
        if entry
            .path()
            .is_some_and(|path| sparse.contains(Path::new(path)))
        {
            flags -= Status::WT_NEW
                | Status::WT_MODIFIED
                | Status::WT_DELETED
                | Status::WT_TYPECHANGE
                | Status::WT_RENAMED;
        }
        if flags.is_empty() || flags.contains(Status::IGNORED) {
            continue;
        }
        let path = entry
            .head_to_index()
            .and_then(|delta| delta.new_file().path())
            .or_else(|| {
                entry
                    .index_to_workdir()
                    .and_then(|delta| delta.new_file().path())
            })
            .and_then(Path::to_str)
            .or_else(|| entry.path())
            .unwrap_or("")
            .to_string();
        let renamed_from = entry
            .head_to_index()
            .filter(|delta| delta.status() == git2::Delta::Renamed)
            .and_then(|delta| delta.old_file().path())
            .and_then(Path::to_str)
            .map(str::to_string);
        let index = if flags.contains(Status::INDEX_NEW) {
            'A'
        } else if flags.contains(Status::INDEX_MODIFIED) {
            'M'
        } else if flags.contains(Status::INDEX_DELETED) {
            'D'
        } else if flags.contains(Status::INDEX_RENAMED) {
            'R'
        } else if flags.contains(Status::INDEX_TYPECHANGE) {
            'T'
        } else {
            ' '
        };
        let untracked = flags.contains(Status::WT_NEW) && !flags.contains(Status::INDEX_NEW);
        let index = if untracked { '?' } else { index };
        let worktree = if flags.contains(Status::WT_NEW) {
            '?'
        } else if flags.contains(Status::WT_MODIFIED) {
            'M'
        } else if flags.contains(Status::WT_DELETED) {
            'D'
        } else if flags.contains(Status::WT_RENAMED) {
            'R'
        } else if flags.contains(Status::WT_TYPECHANGE) {
            'T'
        } else {
            ' '
        };
        entries.push(StatusEntry {
            path,
            index,
            worktree,
            conflicted: flags.contains(Status::CONFLICTED),
            renamed_from,
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Staging
// ---------------------------------------------------------------------------

/// Stages the whole file (or its deletion).
pub fn stage_file(path: &Path) -> Result<()> {
    let (repo, relative) = open(path)?;
    stage_in(&repo, &relative)
}

/// [`stage_file`] for a path that is already resolved.
pub fn stage_target(target: &GitTarget) -> Result<()> {
    stage_in(&target.open()?, &target.relative)
}

fn stage_in(repo: &Repository, relative: &Path) -> Result<()> {
    let mut index = repo.index()?;
    let exists = repo
        .workdir()
        .map(|workdir| workdir.join(relative).exists())
        .unwrap_or(false);
    if exists {
        index.add_path(relative)?;
    } else {
        index.remove_path(relative)?;
    }
    index.write()?;
    Ok(())
}

/// Stages every change in the working tree (`git add -A`).
pub fn stage_all(path: &Path) -> Result<()> {
    let (repo, _) = open(path)?;
    let sparse = skip_worktree_paths(&repo)?;
    // Positive return values skip a path, so sparse files keep their entries.
    let mut skip_sparse = |path: &Path, _: &[u8]| i32::from(sparse.contains(path));
    let mut index = repo.index()?;
    index.add_all(
        ["*"].iter(),
        IndexAddOption::DEFAULT,
        Some(&mut skip_sparse),
    )?;
    index.update_all(["*"].iter(), Some(&mut skip_sparse))?;
    index.write()?;
    Ok(())
}

/// Removes the file from the index, restoring the HEAD version there.
pub fn unstage_file(path: &Path) -> Result<()> {
    let (repo, relative) = open(path)?;
    unstage_in(&repo, &relative)
}

/// [`unstage_file`] for a path that is already resolved.
pub fn unstage_target(target: &GitTarget) -> Result<()> {
    unstage_in(&target.open()?, &target.relative)
}

fn unstage_in(repo: &Repository, relative: &Path) -> Result<()> {
    let tree = match repo.head().and_then(|head| head.peel_to_tree()) {
        Ok(tree) => Some(tree),
        Err(error) if error.code() == ErrorCode::UnbornBranch => None,
        Err(error) => return Err(error.into()),
    };
    let source = tree.as_ref().and_then(|tree| tree.get_path(relative).ok());
    if source
        .as_ref()
        .is_some_and(|entry| entry.kind() == Some(git2::ObjectType::Tree))
    {
        bail!("{} is a directory, not a file", relative.display());
    }
    let mut index = repo.index()?;
    // reset_default accepts glob pathspecs. Restore exactly this tree entry,
    // including its mode, so names like route/[id].tsx cannot reset siblings.
    // Removing by path first also drops conflict stages, as reset does.
    if let Err(error) = index.remove_path(relative) {
        if error.code() != ErrorCode::NotFound {
            return Err(error.into());
        }
    }
    if let Some(source) = source {
        let mut entry = empty_index_entry(relative);
        entry.id = source.id();
        entry.mode = source.filemode() as u32;
        if source.kind() == Some(git2::ObjectType::Blob) {
            entry.file_size = repo.find_blob(source.id())?.size() as u32;
        }
        index.add(&entry)?;
    }
    index.write()?;
    Ok(())
}

/// A zero-context hunk covers `[start, start + lines)` on the 1-based new
/// side; pure deletions (`lines == 0`) sit between `start` and `start + 1`.
fn hunk_covers(start: u32, lines: u32, line1: u32) -> bool {
    if lines == 0 {
        line1 == start || line1 == start + 1
    } else {
        line1 >= start && line1 < start + lines
    }
}

/// `(old_start, old_lines, new_start, new_lines)` of every zero-context hunk.
type HunkRange = (u32, u32, u32, u32);

fn hunk_ranges(diff: &git2::Diff<'_>) -> Result<Vec<HunkRange>> {
    let mut hunks = Vec::new();
    diff.foreach(
        &mut |_, _| true,
        None,
        Some(&mut |_, hunk| {
            hunks.push((
                hunk.old_start(),
                hunk.old_lines(),
                hunk.new_start(),
                hunk.new_lines(),
            ));
            true
        }),
        None,
    )?;
    Ok(hunks)
}

/// Every zero-context hunk with the text of its new side. The text comes from
/// the diff, which compares the *filtered* working tree (line-ending
/// conversion, ident, ...), so it is what `git add` would store.
fn hunks_with_new_text(diff: &git2::Diff<'_>) -> Result<Vec<(HunkRange, Vec<u8>)>> {
    let mut hunks = Vec::new();
    for delta in 0..diff.deltas().len() {
        let Some(patch) = git2::Patch::from_diff(diff, delta)? else {
            continue;
        };
        for hunk_index in 0..patch.num_hunks() {
            let (hunk, line_count) = patch.hunk(hunk_index)?;
            let mut text = Vec::new();
            for line_index in 0..line_count {
                let line = patch.line_in_hunk(hunk_index, line_index)?;
                if line.origin() == '+' {
                    text.extend_from_slice(line.content());
                }
            }
            let range = (
                hunk.old_start(),
                hunk.old_lines(),
                hunk.new_start(),
                hunk.new_lines(),
            );
            hunks.push((range, text));
        }
    }
    Ok(hunks)
}

/// The 0-based `[start, end)` range a hunk side occupies in its text. An empty
/// side (`lines == 0`) is positioned *after* line `start`.
fn side_range(start: u32, lines: u32) -> (usize, usize) {
    let begin = if lines == 0 {
        start as usize
    } else {
        start as usize - 1
    };
    (begin, begin + lines as usize)
}

fn split_lines(text: &[u8]) -> Vec<&[u8]> {
    text.split_inclusive(|byte| *byte == b'\n').collect()
}

/// `target` with `target_side` replaced by `source_side` of `source`.
fn splice_hunk(
    target: &[u8],
    target_side: (usize, usize),
    source: &[u8],
    source_side: (usize, usize),
) -> Vec<u8> {
    let target_lines = split_lines(target);
    let source_lines = split_lines(source);
    let mut out = Vec::with_capacity(target.len());
    for line in target_lines.iter().take(target_side.0) {
        out.extend_from_slice(line);
    }
    for line in source_lines
        .iter()
        .skip(source_side.0)
        .take(source_side.1 - source_side.0)
    {
        out.extend_from_slice(line);
    }
    for line in target_lines.iter().skip(target_side.1) {
        out.extend_from_slice(line);
    }
    out
}

/// Text of `relative` in the index (empty when it is not there).
fn index_text(repo: &Repository, relative: &Path) -> Result<Vec<u8>> {
    let index = repo.index()?;
    Ok(match index.get_path(relative, 0) {
        Some(entry) => repo.find_blob(entry.id)?.content().to_vec(),
        None => Vec::new(),
    })
}

/// A new index entry without cached worktree stat information.
fn empty_index_entry(relative: &Path) -> git2::IndexEntry {
    git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: 0o100644,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: Oid::zero(),
        flags: 0,
        flags_extended: 0,
        path: relative.as_os_str().as_encoded_bytes().to_vec(),
    }
}

/// The index mode a working tree file gets: executable bit and symlinks.
fn worktree_mode(path: &Path) -> u32 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0o100644;
    };
    if metadata.file_type().is_symlink() {
        return 0o120000;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o100 != 0 {
            return 0o100755;
        }
    }
    0o100644
}

/// Replaces the index entry of `relative` with `content`; a path the index
/// lacks is added with `new_mode`.
fn write_index_text(
    repo: &Repository,
    relative: &Path,
    content: &[u8],
    new_mode: u32,
) -> Result<()> {
    let mut index = repo.index()?;
    let mut entry = index.get_path(relative, 0).unwrap_or_else(|| {
        let mut entry = empty_index_entry(relative);
        entry.mode = new_mode;
        entry
    });
    // Unknown stat data: git and libgit2 then compare the contents when asked
    // whether the working tree file differs. A size taken from the blob would
    // mismatch a file whose line endings are converted on checkout, and the
    // file would show as modified although it matches what was staged.
    index.add_frombuffer(&entry, content)?;
    if let Some(mut written) = index.get_path(relative, 0) {
        written.file_size = 0;
        written.mtime = git2::IndexTime::new(0, 0);
        index.add(&written)?;
    }
    index.write()?;
    Ok(())
}

/// Stages the change hunk of `path` that covers 0-based worktree line `line`.
///
/// The hunk is spliced into the index text directly (zero-context hunks are
/// awkward for libgit2's patch application), leaving the rest of the index
/// entry, including other unstaged changes, untouched. Returns false when no
/// unstaged change covers the line.
pub fn stage_hunk(path: &Path, line: usize) -> Result<bool> {
    let (repo, relative) = open(path)?;
    let mut diff_options = DiffOptions::new();
    diff_options
        .pathspec(&relative)
        .disable_pathspec_match(true)
        .context_lines(0)
        .include_untracked(true)
        .show_untracked_content(true);
    let diff = repo.diff_index_to_workdir(None, Some(&mut diff_options))?;
    let target = line as u32 + 1;
    let Some(((old_start, old_lines, _, new_lines), new_text)) = hunks_with_new_text(&diff)?
        .into_iter()
        .find(|((_, _, start, lines), _)| hunk_covers(*start, *lines, target))
    else {
        return Ok(false);
    };
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow!("bare repositories have no working tree"))?;
    let staged = index_text(&repo, &relative)?;
    let merged = splice_hunk(
        &staged,
        side_range(old_start, old_lines),
        &new_text,
        (0, new_lines as usize),
    );
    let mode = worktree_mode(&workdir.join(&relative));
    write_index_text(&repo, &relative, &merged, mode)?;
    Ok(true)
}

/// Maps a 0-based worktree line to the corresponding 0-based index line, using
/// the unstaged changes above it. `None` when the line itself is unstaged.
fn worktree_line_to_index_line(
    repo: &Repository,
    relative: &Path,
    line: usize,
) -> Result<Option<usize>> {
    let mut diff_options = DiffOptions::new();
    diff_options
        .pathspec(relative)
        .disable_pathspec_match(true)
        .context_lines(0);
    let diff = repo.diff_index_to_workdir(None, Some(&mut diff_options))?;
    let target = line as i64 + 1;
    let mut delta = 0i64;
    let mut inside = false;
    diff.foreach(
        &mut |_, _| true,
        None,
        Some(&mut |_, hunk| {
            let new_start = hunk.new_start() as i64;
            let new_lines = hunk.new_lines() as i64;
            let old_lines = hunk.old_lines() as i64;
            if new_lines > 0 && target >= new_start && target < new_start + new_lines {
                inside = true;
            } else if new_start + new_lines.max(1) <= target
                || (new_lines == 0 && new_start < target)
            {
                delta += old_lines - new_lines;
            }
            true
        }),
        None,
    )?;
    Ok((!inside).then(|| (target + delta - 1).max(0) as usize))
}

/// Unstages the staged hunk of `path` covering 0-based worktree line `line`
/// (the index change is reverted back to the HEAD text).
///
/// Returns false when no staged change covers the line.
pub fn unstage_hunk(path: &Path, line: usize) -> Result<bool> {
    let (repo, relative) = open(path)?;
    let Some(index_line) = worktree_line_to_index_line(&repo, &relative, line)? else {
        return Ok(false);
    };
    let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
    let mut diff_options = DiffOptions::new();
    diff_options
        .pathspec(&relative)
        .disable_pathspec_match(true)
        .context_lines(0);
    let diff = repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut diff_options))?;
    let target = index_line as u32 + 1;
    let Some((old_start, old_lines, new_start, new_lines)) = hunk_ranges(&diff)?
        .into_iter()
        .find(|(_, _, start, lines)| hunk_covers(*start, *lines, target))
    else {
        return Ok(false);
    };
    let head_entry = head_tree
        .as_ref()
        .and_then(|tree| tree.get_path(&relative).ok());
    let head_text = match head_entry.as_ref() {
        Some(entry) => repo.find_blob(entry.id())?.content().to_vec(),
        None => Vec::new(),
    };
    let staged = index_text(&repo, &relative)?;
    let reverted = splice_hunk(
        &staged,
        side_range(new_start, new_lines),
        &head_text,
        side_range(old_start, old_lines),
    );
    if reverted.is_empty() && head_entry.is_none() {
        // Reverting an addition restores absence, not a staged empty file.
        let mut index = repo.index()?;
        index.remove_path(&relative)?;
        index.write()?;
    } else {
        let mode = head_entry
            .as_ref()
            .map_or(0o100644, |entry| entry.filemode() as u32);
        write_index_text(&repo, &relative, &reverted, mode)?;
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// Commit
// ---------------------------------------------------------------------------

/// State a commit message buffer needs.
#[derive(Debug, Clone)]
pub struct CommitContext {
    pub workdir: PathBuf,
    pub branch: Option<String>,
    /// `git status --short` lines of the staged changes.
    pub staged: Vec<String>,
    pub unstaged: Vec<String>,
    pub untracked: Vec<String>,
    /// Subject and body of HEAD (what an amend starts from).
    pub head_message: Option<String>,
    pub merging: bool,
    /// The merge, cherry-pick or revert this commit would finish.
    pub operation: Option<&'static str>,
    /// What git prepared for the commit that finishes `operation`
    /// (`.git/MERGE_MSG`).
    pub prepared_message: Option<String>,
    pub conflicted: Vec<String>,
}

/// The merge, cherry-pick or revert that is waiting for its commit.
fn unfinished_operation(repo: &Repository) -> Option<&'static str> {
    use git2::RepositoryState as State;
    match repo.state() {
        State::Merge => Some("merge"),
        State::Revert | State::RevertSequence => Some("revert"),
        State::CherryPick | State::CherryPickSequence => Some("cherry-pick"),
        _ => None,
    }
}

pub fn commit_context(path: &Path) -> Result<CommitContext> {
    let (repo, _) = open(path)?;
    let entries = status(path)?;
    let describe = |entry: &StatusEntry, code: String| match &entry.renamed_from {
        Some(from) => format!("{code} {from} -> {}", entry.path),
        None => format!("{code} {}", entry.path),
    };
    let staged = entries
        .iter()
        .filter(|entry| entry.is_staged())
        .map(|entry| describe(entry, format!("{}", entry.index)))
        .collect();
    let unstaged = entries
        .iter()
        .filter(|entry| !entry.conflicted && matches!(entry.worktree, 'M' | 'D' | 'T' | 'R'))
        .map(|entry| describe(entry, format!("{}", entry.worktree)))
        .collect();
    let untracked = entries
        .iter()
        .filter(|entry| entry.worktree == '?' && entry.index == '?')
        .map(|entry| entry.path.clone())
        .collect();
    let conflicted = entries
        .iter()
        .filter(|entry| entry.conflicted)
        .map(|entry| entry.path.clone())
        .collect();
    let head_message = repo
        .head()
        .ok()
        .and_then(|head| head.peel_to_commit().ok())
        .and_then(|commit| {
            commit
                .message()
                .map(|message| message.trim_end().to_string())
        });
    let branch = repo
        .head()
        .ok()
        .and_then(|head| head.shorthand().map(str::to_string));
    let operation = unfinished_operation(&repo);
    let prepared_message = operation
        .and_then(|_| repo.message().ok())
        .map(|message| message.trim_end().to_string())
        .filter(|message| !message.is_empty());
    Ok(CommitContext {
        workdir: workdir_of(path)?,
        branch,
        staged,
        unstaged,
        untracked,
        head_message,
        merging: operation == Some("merge"),
        operation,
        prepared_message,
        conflicted,
    })
}

/// Turns the text of a commit message buffer into the message: `#` lines are
/// comments, trailing blank lines are dropped.
pub fn clean_commit_message(text: &str) -> String {
    let kept: Vec<&str> = text.lines().filter(|line| !line.starts_with('#')).collect();
    kept.join("\n").trim().to_string()
}

/// A commit that passed the quick checks and only has to be made. Hooks can
/// take minutes, so callers run it off the UI thread.
#[derive(Debug)]
pub struct PreparedCommit {
    root: PathBuf,
    /// The text of the message buffer, comment lines included.
    message: String,
    amend: bool,
    /// The `git` binary that makes the commit. Without one libgit2 does, which
    /// cannot run hooks or sign.
    git: Option<PathBuf>,
}

/// Commits the index. With `amend` the previous commit is replaced.
///
/// Returns the new commit's short id and subject.
pub fn commit(path: &Path, message: &str, amend: bool) -> Result<(String, String)> {
    prepare_commit(path, message, amend)?.run()
}

/// Checks that the index can be committed with `message` (the text of the
/// message buffer) and returns the commit, ready to [`PreparedCommit::run`].
pub fn prepare_commit(path: &Path, message: &str, amend: bool) -> Result<PreparedCommit> {
    prepare_commit_with(path, message, amend, which::which("git").ok())
}

fn prepare_commit_with(
    path: &Path,
    message: &str,
    amend: bool,
    git: Option<PathBuf>,
) -> Result<PreparedCommit> {
    if clean_commit_message(message).is_empty() {
        bail!("Aborting commit: the message is empty");
    }
    let (repo, _) = open(path)?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
        bail!("Cannot commit: unresolved merge conflicts (resolve, then stage the files)");
    }
    let head = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    if amend {
        if head.is_none() {
            bail!("Nothing to amend: there are no commits yet");
        }
        if let Some(operation) = unfinished_operation(&repo) {
            bail!("You are in the middle of a {operation} -- cannot amend");
        }
    } else {
        let tree = index.write_tree()?;
        let unchanged = match &head {
            Some(head) => head.tree_id() == tree && unfinished_operation(&repo) != Some("merge"),
            None => repo.find_tree(tree)?.is_empty(),
        };
        if unchanged {
            bail!("Nothing to commit: no changes are staged");
        }
    }
    if git.is_none() {
        refuse_what_libgit2_cannot_do(&repo)?;
    }
    Ok(PreparedCommit {
        root: canonical_workdir(&repo)?,
        message: message.to_string(),
        amend,
        git,
    })
}

/// Without a `git` binary hooks cannot run and commits cannot be signed;
/// committing anyway would silently skip checks the repository relies on.
fn refuse_what_libgit2_cannot_do(repo: &Repository) -> Result<()> {
    const HOOKS: [&str; 4] = [
        "pre-commit",
        "prepare-commit-msg",
        "commit-msg",
        "post-commit",
    ];
    let config = repo.config()?;
    if config.get_bool("commit.gpgsign").unwrap_or(false) {
        bail!("Cannot sign the commit: commit.gpgsign is set and no git binary was found");
    }
    let hooks = match config.get_path("core.hooksPath") {
        Ok(path) if path.is_relative() => canonical_workdir(repo)?.join(path),
        Ok(path) => path,
        Err(_) => common_dir(repo).join("hooks"),
    };
    if let Some(hook) = HOOKS.iter().find(|hook| is_executable(&hooks.join(hook))) {
        bail!("Cannot run the {hook} hook: no git binary was found");
    }
    Ok(())
}

/// The directory shared by a repository's linked worktrees (`.git`).
fn common_dir(repo: &Repository) -> PathBuf {
    let git_dir = repo.path();
    match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(relative) => git_dir.join(relative.trim()),
        Err(_) => git_dir.to_path_buf(),
    }
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

impl PreparedCommit {
    /// Makes the commit and returns its short id and subject. Blocks for as
    /// long as the repository's hooks take.
    pub fn run(self) -> Result<(String, String)> {
        match self.git.clone() {
            Some(git) => self.run_git(&git),
            None => self.run_libgit2(),
        }
    }

    /// `git commit`, so hooks, signing and the merge / cherry-pick / revert
    /// bookkeeping all happen as they do on the command line.
    fn run_git(&self, git: &Path) -> Result<(String, String)> {
        use std::io::{Read, Seek, Write};
        use std::process::{Command, Stdio};

        let repo = Repository::open(&self.root)?;
        let mut message = tempfile::Builder::new()
            .prefix("OVIM_COMMIT_MSG")
            .tempfile_in(repo.path())?;
        message.write_all(self.message.as_bytes())?;
        message.flush()?;
        // A file rather than pipes: a hook that leaves a daemon behind would
        // hold a pipe open and keep the read waiting after git has finished.
        let mut output = tempfile::tempfile()?;
        let mut command = Command::new(git);
        command
            .args(["commit", "--cleanup=strip", "-F"])
            .arg(message.path())
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(output.try_clone()?)
            // A curses pinentry would draw over the editor.
            .env_remove("GPG_TTY");
        if self.amend {
            command.arg("--amend");
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // No controlling terminal: hooks and signing programs that open
            // /dev/tty to prompt fail instead of taking over the editor's.
            // SAFETY: setsid is async-signal-safe and touches no shared state.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let status = command
            .status()
            .with_context(|| format!("Could not run {}", git.display()))?;
        if !status.success() {
            let mut text = String::new();
            output.rewind()?;
            output.read_to_string(&mut text)?;
            let text = text.trim();
            if text.is_empty() {
                bail!("git commit failed ({status})");
            }
            bail!("{text}");
        }
        let repo = Repository::open(&self.root)?;
        let head = repo.head()?.peel_to_commit()?;
        Ok((
            short(head.id()),
            head.summary().unwrap_or_default().to_string(),
        ))
    }

    /// Commits through libgit2, finishing the merge, cherry-pick or revert the
    /// way `git commit` does.
    fn run_libgit2(&self) -> Result<(String, String)> {
        let message = clean_commit_message(&self.message);
        let repo = Repository::open(&self.root)?;
        let mut index = repo.index()?;
        let tree = repo.find_tree(index.write_tree()?)?;
        let committer = repo.signature().map_err(|error| {
            anyhow!("Cannot commit: set user.name and user.email in your git config ({error})")
        })?;
        let head = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
        let full_message = format!("{message}\n");

        let oid = if self.amend {
            let head = head.ok_or_else(|| anyhow!("Nothing to amend: there are no commits yet"))?;
            head.amend(
                Some("HEAD"),
                None,
                Some(&committer),
                None,
                Some(&full_message),
                Some(&tree),
            )?
        } else {
            let mut parents: Vec<git2::Commit> = head.into_iter().collect();
            for oid in read_oids(&repo.path().join("MERGE_HEAD")) {
                parents.push(repo.find_commit(oid)?);
            }
            // A resolved cherry-pick keeps the picked commit's author.
            let picked = read_oids(&repo.path().join("CHERRY_PICK_HEAD"))
                .into_iter()
                .next()
                .map(|oid| repo.find_commit(oid))
                .transpose()?;
            let author = match &picked {
                Some(commit) => commit.author(),
                None => committer.clone(),
            };
            let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
            repo.commit(
                Some("HEAD"),
                &author,
                &committer,
                &full_message,
                &tree,
                &parent_refs,
            )?
        };
        if unfinished_operation(&repo).is_some() {
            repo.cleanup_state()?;
        }
        let subject = message.lines().next().unwrap_or("").to_string();
        Ok((short(oid), subject))
    }
}

/// The object ids listed in a git state file such as `MERGE_HEAD`.
fn read_oids(path: &Path) -> Vec<Oid> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| Oid::from_str(line.trim()).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub oid: String,
    pub short: String,
    pub author: String,
    /// `YYYY-MM-DD`
    pub date: String,
    pub subject: String,
    /// Path of the file in that commit (differs after renames).
    pub path: Option<String>,
}

fn short(oid: Oid) -> String {
    oid.to_string()[..7].to_string()
}

fn log_entry(commit: &git2::Commit, path: Option<String>) -> LogEntry {
    let time = commit.author().when();
    LogEntry {
        oid: commit.id().to_string(),
        short: short(commit.id()),
        author: commit.author().name().unwrap_or("?").to_string(),
        date: Utc
            .timestamp_opt(time.seconds(), 0)
            .single()
            .map(|date| date.format("%Y-%m-%d").to_string())
            .unwrap_or_default(),
        subject: commit.summary().unwrap_or("").to_string(),
        path,
    }
}

/// Commits that touched `path` (all commits when `path` is the repository
/// root), newest first.
pub fn file_history(path: &Path, limit: usize) -> Result<Vec<LogEntry>> {
    let (repo, relative) = open(path)?;
    let mut walk = repo.revwalk()?;
    walk.push_head()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
    let mut entries = Vec::new();
    for oid in walk.take(20_000) {
        let commit = repo.find_commit(oid?)?;
        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        let mut options = DiffOptions::new();
        // The repository root has an empty relative path, which as a literal
        // pathspec would match nothing; the whole tree is wanted instead.
        if !relative.as_os_str().is_empty() {
            options.pathspec(&relative).disable_pathspec_match(true);
        }
        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut options))?;
        if diff.deltas().len() > 0 {
            entries.push(log_entry(
                &commit,
                Some(relative.to_string_lossy().to_string()),
            ));
            if entries.len() >= limit {
                break;
            }
        }
    }
    Ok(entries)
}

/// The commits that changed 0-based `line` of `path`, newest first: blame gives
/// the last change, then the line is followed into that commit's parent and
/// blamed again (like `git log -L line,line:path`). Working-tree changes not
/// yet committed show up as an "Uncommitted changes" entry.
pub fn line_history(path: &Path, line: usize, limit: usize) -> Result<Vec<LogEntry>> {
    let (repo, relative) = open(path)?;
    let head = repo.head()?.peel_to_commit()?;
    let workdir = workdir_of(path)?;
    let mut entries = Vec::new();

    let mut current_path = relative;
    let mut current_line = line + 1;
    let mut newest: Option<Oid> = None;

    for _ in 0..limit {
        let mut options = BlameOptions::new();
        options.min_line(current_line).max_line(current_line);
        if let Some(newest) = newest {
            options.newest_commit(newest);
        }
        let committed = repo.blame_file(&current_path, Some(&mut options))?;
        let buffered;
        let blame = match std::fs::read(workdir.join(&current_path)) {
            // Blame the working tree text so uncommitted lines are recognised.
            Ok(content) if newest.is_none() => {
                buffered = committed.blame_buffer(&content)?;
                &buffered
            }
            _ => &committed,
        };
        let Some(hunk) = blame.get_line(current_line) else {
            break;
        };
        let commit_id = hunk.final_commit_id();
        if commit_id.is_zero() {
            // Not committed yet: report it, then follow the line back to HEAD.
            entries.push(LogEntry {
                oid: String::new(),
                short: "-------".to_string(),
                author: "You".to_string(),
                date: String::new(),
                subject: "Uncommitted changes".to_string(),
                path: Some(current_path.to_string_lossy().to_string()),
            });
            let mut diff_options = DiffOptions::new();
            diff_options
                .pathspec(&current_path)
                .disable_pathspec_match(true)
                .context_lines(0);
            let diff =
                repo.diff_tree_to_workdir_with_index(Some(&head.tree()?), Some(&mut diff_options))?;
            match map_line_to_old_side(&diff, current_line)? {
                Some(old_line) => {
                    current_line = old_line;
                    newest = Some(head.id());
                    continue;
                }
                None => break,
            }
        }
        let commit = repo.find_commit(commit_id)?;
        let commit_path = hunk
            .path()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| current_path.clone());
        let commit_line = hunk.orig_start_line().max(1);
        entries.push(log_entry(
            &commit,
            Some(commit_path.to_string_lossy().to_string()),
        ));
        if commit.parent_count() == 0 {
            break;
        }
        let parent = commit.parent(0)?;
        let mut diff_options = DiffOptions::new();
        diff_options
            .pathspec(&commit_path)
            .disable_pathspec_match(true)
            .context_lines(0);
        let mut diff = repo.diff_tree_to_tree(
            Some(&parent.tree()?),
            Some(&commit.tree()?),
            Some(&mut diff_options),
        )?;
        diff.find_similar(Some(git2::DiffFindOptions::new().renames(true)))?;
        let old_path = diff
            .deltas()
            .next()
            .and_then(|delta| delta.old_file().path().map(Path::to_path_buf))
            .unwrap_or_else(|| commit_path.clone());
        match map_line_to_old_side(&diff, commit_line)? {
            Some(old_line) => {
                current_line = old_line;
                current_path = old_path;
                newest = Some(parent.id());
            }
            None => break,
        }
    }
    Ok(entries)
}

/// Line of the old side that the 1-based new-side `line` replaced (or `None`
/// when the line was added rather than modified). Lines outside every hunk
/// shift by the cumulative size change above them.
fn map_line_to_old_side(diff: &git2::Diff<'_>, line: usize) -> Result<Option<usize>> {
    let target = line as i64;
    let mut delta = 0i64;
    let mut result: Option<Option<usize>> = None;
    diff.foreach(
        &mut |_, _| true,
        None,
        Some(&mut |_, hunk| {
            let new_start = hunk.new_start() as i64;
            let new_lines = hunk.new_lines() as i64;
            let old_start = hunk.old_start() as i64;
            let old_lines = hunk.old_lines() as i64;
            if result.is_some() {
                return true;
            }
            if new_lines > 0 && target >= new_start && target < new_start + new_lines {
                let offset = target - new_start;
                result = Some((offset < old_lines).then(|| (old_start + offset) as usize));
            } else if new_start + new_lines <= target {
                delta += old_lines - new_lines;
            }
            true
        }),
        None,
    )?;
    Ok(match result {
        Some(mapped) => mapped,
        None => Some((target + delta).max(1) as usize),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;
    use std::fs;

    struct Repo {
        _dir: tempfile::TempDir,
        root: PathBuf,
        repo: Repository,
    }

    impl Repo {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(dir.path()).unwrap();
            let repo = Repository::init(&root).unwrap();
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "Test").unwrap();
            config.set_str("user.email", "test@example.com").unwrap();
            // Commits run through the git binary: keep the developer's global
            // signing and hook configuration out of the tests.
            config.set_bool("commit.gpgsign", false).unwrap();
            config
                .set_str("core.hooksPath", root.join(".git/hooks").to_str().unwrap())
                .unwrap();
            Self {
                _dir: dir,
                root,
                repo,
            }
        }

        fn write(&self, name: &str, content: &str) -> PathBuf {
            let path = self.root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            path
        }

        fn commit_all(&self, message: &str) -> Oid {
            let mut index = self.repo.index().unwrap();
            index
                .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
                .unwrap();
            index.write().unwrap();
            let tree = self.repo.find_tree(index.write_tree().unwrap()).unwrap();
            let signature = Signature::now("Test", "test@example.com").unwrap();
            let parent = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            let parents: Vec<&git2::Commit> = parent.iter().collect();
            self.repo
                .commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    message,
                    &tree,
                    &parents,
                )
                .unwrap()
        }

        fn index_text(&self, name: &str) -> String {
            let mut index = self.repo.index().unwrap();
            index.read(true).unwrap();
            let entry = index.get_path(Path::new(name), 0).unwrap();
            String::from_utf8(self.repo.find_blob(entry.id).unwrap().content().to_vec()).unwrap()
        }
    }

    const TEN: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";

    #[test]
    fn status_reports_staged_unstaged_and_untracked() {
        let repo = Repo::new();
        let a = repo.write("a.txt", "one\n");
        repo.write("b.txt", "two\n");
        repo.commit_all("init");
        fs::write(&a, "one changed\n").unwrap();
        repo.write("c.txt", "new\n");
        stage_file(&a).unwrap();
        fs::write(&a, "one changed twice\n").unwrap();
        let entries = status(&repo.root).unwrap();
        let by_path = |p: &str| entries.iter().find(|e| e.path == p).unwrap().clone();
        assert_eq!(by_path("a.txt").code(), "MM");
        assert!(by_path("a.txt").is_staged() && by_path("a.txt").has_unstaged_changes());
        assert_eq!(by_path("c.txt").code(), "??");
        assert!(entries.iter().all(|e| e.path != "b.txt"));
    }

    /// Marks `name` as left out of the working tree by a sparse checkout.
    fn mark_skip_worktree(repo: &Repo, name: &str) {
        let mut index = repo.repo.index().unwrap();
        let mut entry = index.get_path(Path::new(name), 0).unwrap();
        entry.flags_extended |= SKIP_WORKTREE;
        index.add(&entry).unwrap();
        index.write().unwrap();
    }

    #[test]
    fn sparse_checkout_files_are_neither_reported_deleted_nor_staged_as_deleted() {
        let repo = Repo::new();
        repo.write("kept.txt", "kept\n");
        let sparse = repo.write("sparse.txt", "sparse\n");
        repo.commit_all("init");
        mark_skip_worktree(&repo, "sparse.txt");
        fs::remove_file(&sparse).unwrap();
        repo.write("kept.txt", "kept, edited\n");

        let entries = status(&repo.root).unwrap();
        assert_eq!(
            entries.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            ["kept.txt"],
            "git status does not list skip-worktree files as deleted"
        );
        stage_all(&repo.root).unwrap();
        assert_eq!(repo.index_text("sparse.txt"), "sparse\n");
        assert_eq!(repo.index_text("kept.txt"), "kept, edited\n");
    }

    #[test]
    fn stage_and_unstage_a_file() {
        let repo = Repo::new();
        let a = repo.write("a.txt", "one\n");
        repo.commit_all("init");
        fs::write(&a, "two\n").unwrap();
        stage_file(&a).unwrap();
        assert_eq!(repo.index_text("a.txt"), "two\n");
        unstage_file(&a).unwrap();
        assert_eq!(repo.index_text("a.txt"), "one\n");
        assert_eq!(
            fs::read_to_string(&a).unwrap(),
            "two\n",
            "worktree untouched"
        );
    }

    #[test]
    fn unstage_refuses_a_directory_and_leaves_the_index_intact() {
        let repo = Repo::new();
        let a = repo.write("dir/a.txt", "one\n");
        repo.commit_all("init");
        fs::write(&a, "two\n").unwrap();
        stage_file(&a).unwrap();
        assert!(unstage_file(&repo.root.join("dir")).is_err());
        assert_eq!(repo.index_text("dir/a.txt"), "two\n");
    }

    #[test]
    fn unstage_clears_conflict_stages_like_git_restore_staged() {
        let repo = Repo::new();
        let a = repo.write("a.txt", "base\n");
        repo.commit_all("init");
        let git = &repo.repo;
        let mut index = git.index().unwrap();
        let mut entry = index.get_path(Path::new("a.txt"), 0).unwrap();
        index.remove_path(Path::new("a.txt")).unwrap();
        for (stage, text) in [(1u16, "base\n"), (2, "ours\n"), (3, "theirs\n")] {
            entry.id = git.blob(text.as_bytes()).unwrap();
            entry.flags = (entry.flags & !0x3000) | (stage << 12);
            index.add(&entry).unwrap();
        }
        index.write().unwrap();
        assert!(index.has_conflicts());
        unstage_file(&a).unwrap();
        index.read(true).unwrap();
        assert!(!index.has_conflicts());
        assert_eq!(repo.index_text("a.txt"), "base\n");
    }

    #[test]
    fn history_of_the_repository_root_lists_every_commit() {
        let repo = Repo::new();
        repo.write("a.txt", "one\n");
        repo.commit_all("first");
        repo.write("dir/b.txt", "two\n");
        repo.commit_all("second");
        let messages: Vec<_> = file_history(&repo.root, 10)
            .unwrap()
            .into_iter()
            .map(|entry| entry.subject)
            .collect();
        assert_eq!(messages, ["second", "first"]);
        assert_eq!(file_history(&repo.root.join("dir"), 10).unwrap().len(), 1);
    }

    #[test]
    fn unstage_works_before_the_first_commit() {
        let repo = Repo::new();
        let a = repo.write("a.txt", "one\n");
        stage_file(&a).unwrap();
        assert_eq!(status(&repo.root).unwrap()[0].code(), "A ");
        unstage_file(&a).unwrap();
        assert_eq!(status(&repo.root).unwrap()[0].code(), "??");
    }

    #[test]
    fn stage_hunk_stages_only_the_hunk_under_the_cursor() {
        let repo = Repo::new();
        let file = repo.write("a.txt", TEN);
        repo.commit_all("init");
        // Two separate edits: line 2 and line 9.
        fs::write(&file, "l1\nL2\nl3\nl4\nl5\nl6\nl7\nl8\nL9\nl10\n").unwrap();

        assert!(stage_hunk(&file, 8).unwrap(), "the change on line 9");
        assert_eq!(
            repo.index_text("a.txt"),
            "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nL9\nl10\n"
        );
        assert!(!stage_hunk(&file, 4).unwrap(), "line 5 has no change");
        let entry = &status(&repo.root).unwrap()[0];
        assert_eq!(entry.code(), "MM", "line 2 is still unstaged");
    }

    #[test]
    fn stage_hunk_handles_added_and_deleted_lines() {
        let repo = Repo::new();
        let file = repo.write("a.txt", TEN);
        repo.commit_all("init");
        // Delete l3, add a line after l7.
        fs::write(&file, "l1\nl2\nl4\nl5\nl6\nl7\nnew\nl8\nl9\nl10\n").unwrap();
        assert!(stage_hunk(&file, 6).unwrap(), "the added line");
        assert_eq!(
            repo.index_text("a.txt"),
            "l1\nl2\nl3\nl4\nl5\nl6\nl7\nnew\nl8\nl9\nl10\n"
        );
        assert!(
            stage_hunk(&file, 2).unwrap(),
            "the deletion, seen from the next line"
        );
        assert_eq!(
            repo.index_text("a.txt"),
            "l1\nl2\nl4\nl5\nl6\nl7\nnew\nl8\nl9\nl10\n"
        );
    }

    #[test]
    fn stage_hunk_can_stage_a_brand_new_file_and_a_last_line_without_newline() {
        let repo = Repo::new();
        repo.write("keep.txt", "x\n");
        repo.commit_all("init");
        let new = repo.write("new.txt", "a\nb\n");
        assert!(stage_hunk(&new, 0).unwrap());
        assert_eq!(repo.index_text("new.txt"), "a\nb\n");

        let tail = repo.write("tail.txt", "one\ntwo\n");
        repo.commit_all("tail");
        fs::write(&tail, "one\ntwo\nthree").unwrap();
        assert!(stage_hunk(&tail, 2).unwrap());
        assert_eq!(repo.index_text("tail.txt"), "one\ntwo\nthree");
    }

    #[test]
    fn stage_hunk_stores_the_clean_filtered_text_not_the_raw_worktree_bytes() {
        let repo = Repo::new();
        repo.write(".gitattributes", "*.txt text eol=crlf\n");
        let file = repo.write("a.txt", "one\r\ntwo\r\nthree\r\n");
        repo.commit_all("init");
        assert_eq!(repo.index_text("a.txt"), "one\ntwo\nthree\n");
        repo.write("a.txt", "one\r\nTWO\r\nthree\r\n");

        assert!(stage_hunk(&file, 1).unwrap());
        assert_eq!(
            repo.index_text("a.txt"),
            "one\nTWO\nthree\n",
            "line endings are normalised like `git add`"
        );
        assert_eq!(
            status(&repo.root).unwrap()[0].code(),
            "M ",
            "nothing is left unstaged"
        );
    }

    #[test]
    fn stage_hunk_of_a_new_executable_file_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let repo = Repo::new();
        repo.write("keep.txt", "x\n");
        repo.commit_all("init");
        let script = repo.write("run.sh", "echo hi\n");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(stage_hunk(&script, 0).unwrap());
        let mut index = repo.repo.index().unwrap();
        index.read(true).unwrap();
        assert_eq!(
            index.get_path(Path::new("run.sh"), 0).unwrap().mode,
            0o100755
        );
    }

    #[test]
    fn unstage_hunk_reverts_only_the_hunk_under_the_cursor() {
        let repo = Repo::new();
        let file = repo.write("a.txt", TEN);
        repo.commit_all("init");
        fs::write(&file, "l1\nL2\nl3\nl4\nl5\nl6\nl7\nl8\nL9\nl10\n").unwrap();
        stage_file(&file).unwrap();

        assert!(unstage_hunk(&file, 8).unwrap());
        assert_eq!(
            repo.index_text("a.txt"),
            "l1\nL2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n",
            "line 9 is unstaged, line 2 stays staged"
        );
        assert!(!unstage_hunk(&file, 4).unwrap(), "nothing staged on line 5");
    }

    #[test]
    fn unstage_hunk_accounts_for_unstaged_lines_above() {
        let repo = Repo::new();
        let file = repo.write("a.txt", TEN);
        repo.commit_all("init");
        // Stage a change to l9, then add two unstaged lines at the top so the
        // worktree line numbers no longer match the index.
        fs::write(&file, "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nL9\nl10\n").unwrap();
        stage_file(&file).unwrap();
        fs::write(
            &file,
            "top1\ntop2\nl1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nL9\nl10\n",
        )
        .unwrap();
        assert!(
            unstage_hunk(&file, 10).unwrap(),
            "L9 is on worktree line 11"
        );
        assert_eq!(repo.index_text("a.txt"), TEN);
    }

    #[test]
    fn commit_creates_amends_and_validates() {
        let repo = Repo::new();
        let a = repo.write("a.txt", "one\n");
        stage_file(&a).unwrap();
        assert!(commit(&repo.root, "  \n# only a comment\n", false)
            .unwrap_err()
            .to_string()
            .contains("empty"));
        let (short, subject) = commit(&repo.root, "First\n\n# comment\nbody", false).unwrap();
        assert_eq!(short.len(), 7);
        assert_eq!(subject, "First");
        let head = repo.repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message().unwrap(), "First\n\nbody\n");

        assert!(commit(&repo.root, "Second", false)
            .unwrap_err()
            .to_string()
            .contains("Nothing to commit"));

        fs::write(&a, "two\n").unwrap();
        stage_file(&a).unwrap();
        commit(&repo.root, "First, amended", true).unwrap();
        let head = repo.repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message().unwrap(), "First, amended\n");
        assert_eq!(head.parent_count(), 0, "amend replaced the commit");
        assert_eq!(repo.index_text("a.txt"), "two\n");
    }

    /// The commit routes under test: the git binary and the libgit2 fallback.
    fn commit_routes() -> [Option<PathBuf>; 2] {
        [
            Some(which::which("git").expect("tests need git on PATH")),
            None,
        ]
    }

    fn commit_via(
        repo: &Repo,
        message: &str,
        amend: bool,
        git: &Option<PathBuf>,
    ) -> Result<(String, String)> {
        prepare_commit_with(&repo.root, message, amend, git.clone())?.run()
    }

    fn head_commit(repo: &Repo) -> git2::Commit<'_> {
        repo.repo.head().unwrap().peel_to_commit().unwrap()
    }

    /// A commit on no branch that adds `name` to the parent's tree.
    fn side_commit(repo: &Repo, parent: Oid, name: &str, author: &Signature) -> Oid {
        let parent = repo.repo.find_commit(parent).unwrap();
        let blob = repo.repo.blob(b"side\n").unwrap();
        let mut builder = repo
            .repo
            .treebuilder(Some(&parent.tree().unwrap()))
            .unwrap();
        builder.insert(name, blob, 0o100644).unwrap();
        let tree = repo.repo.find_tree(builder.write().unwrap()).unwrap();
        repo.repo
            .commit(None, author, author, "Add side file", &tree, &[&parent])
            .unwrap()
    }

    /// Leaves the repository in the middle of merging `other`, as a merge with
    /// conflicts that were all resolved and staged would.
    fn start_merge(repo: &Repo, other: Oid) {
        let git_dir = repo.repo.path();
        fs::write(git_dir.join("MERGE_HEAD"), format!("{other}\n")).unwrap();
        fs::write(git_dir.join("MERGE_MSG"), "Merge branch 'topic'\n").unwrap();
    }

    fn write_hook(repo: &Repo, name: &str, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let hooks = repo.root.join(".git/hooks");
        fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join(name);
        fs::write(&hook, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn commit_messages_end_with_a_newline_on_both_routes() {
        for git in commit_routes() {
            let repo = Repo::new();
            let a = repo.write("a.txt", "one\n");
            stage_file(&a).unwrap();
            let (_, subject) =
                commit_via(&repo, "Subject\n\n# comment\nBody\n\n\n", false, &git).unwrap();
            assert_eq!(subject, "Subject");
            assert_eq!(head_commit(&repo).message().unwrap(), "Subject\n\nBody\n");
        }
    }

    #[test]
    fn amending_in_the_middle_of_a_merge_is_refused() {
        for git in commit_routes() {
            let repo = Repo::new();
            repo.write("a.txt", "one\n");
            let first = repo.commit_all("first");
            repo.write("a.txt", "two\n");
            repo.commit_all("second");
            start_merge(&repo, first);
            let error = commit_via(&repo, "Amended", true, &git).unwrap_err();
            assert!(error.to_string().contains("middle of a merge"), "{error:#}");
            assert_eq!(head_commit(&repo).message().unwrap(), "second");
            assert_eq!(head_commit(&repo).parent_count(), 1, "the merge survives");
            assert_eq!(repo.repo.state(), git2::RepositoryState::Merge);
        }
    }

    #[test]
    fn committing_a_merge_records_both_parents_and_clears_the_merge_state() {
        for git in commit_routes() {
            let repo = Repo::new();
            repo.write("a.txt", "one\n");
            let first = repo.commit_all("first");
            repo.write("a.txt", "two\n");
            repo.commit_all("second");
            let side = side_commit(
                &repo,
                first,
                "side.txt",
                &Signature::now("Test", "test@example.com").unwrap(),
            );
            fs::write(repo.root.join("side.txt"), "side\n").unwrap();
            stage_file(&repo.root.join("side.txt")).unwrap();
            start_merge(&repo, side);
            commit_via(&repo, "Merge branch 'topic'", false, &git).unwrap();
            assert_eq!(head_commit(&repo).parent_count(), 2);
            assert_eq!(repo.repo.state(), git2::RepositoryState::Clean);
            assert!(!repo.repo.path().join("MERGE_MSG").exists());
        }
    }

    #[test]
    fn committing_a_resolved_cherry_pick_keeps_the_author_and_clears_the_state() {
        for git in commit_routes() {
            let repo = Repo::new();
            repo.write("a.txt", "one\n");
            let base = repo.commit_all("base");
            let other = Signature::now("Other", "other@example.com").unwrap();
            let picked = side_commit(&repo, base, "b.txt", &other);
            fs::write(repo.root.join("b.txt"), "side\n").unwrap();
            stage_file(&repo.root.join("b.txt")).unwrap();
            fs::write(
                repo.repo.path().join("CHERRY_PICK_HEAD"),
                format!("{picked}\n"),
            )
            .unwrap();
            assert_eq!(repo.repo.state(), git2::RepositoryState::CherryPick);

            commit_via(&repo, "Add b", false, &git).unwrap();
            let head = head_commit(&repo);
            assert_eq!(head.author().name(), Some("Other"));
            assert_eq!(head.committer().name(), Some("Test"));
            assert_eq!(repo.repo.state(), git2::RepositoryState::Clean);
            assert!(!repo.repo.path().join("CHERRY_PICK_HEAD").exists());
        }
    }

    #[test]
    fn a_failing_hook_prevents_the_commit_and_its_output_is_the_error() {
        let repo = Repo::new();
        let a = repo.write("a.txt", "one\n");
        stage_file(&a).unwrap();
        write_hook(&repo, "pre-commit", "echo 'lint failed' >&2\nexit 1");
        let git = Some(which::which("git").expect("tests need git on PATH"));
        let error = commit_via(&repo, "First", false, &git).unwrap_err();
        assert!(error.to_string().contains("lint failed"), "{error:#}");
        assert!(repo.repo.head().is_err(), "no commit was made");
    }

    #[test]
    fn without_a_git_binary_commits_that_need_hooks_or_signing_are_refused() {
        let repo = Repo::new();
        let a = repo.write("a.txt", "one\n");
        stage_file(&a).unwrap();
        write_hook(&repo, "commit-msg", "exit 0");
        let error = commit_via(&repo, "First", false, &None).unwrap_err();
        assert!(error.to_string().contains("commit-msg hook"), "{error:#}");
        assert!(repo.repo.head().is_err(), "no commit was made");

        fs::remove_file(repo.root.join(".git/hooks/commit-msg")).unwrap();
        repo.repo
            .config()
            .unwrap()
            .set_bool("commit.gpgsign", true)
            .unwrap();
        let error = commit_via(&repo, "First", false, &None).unwrap_err();
        assert!(error.to_string().contains("gpgsign"), "{error:#}");
        assert!(repo.repo.head().is_err(), "no commit was made");
    }

    #[test]
    fn commit_context_reports_the_merge_message_git_prepared() {
        let repo = Repo::new();
        repo.write("a.txt", "one\n");
        let first = repo.commit_all("first");
        repo.write("a.txt", "two\n");
        repo.commit_all("second");
        assert_eq!(commit_context(&repo.root).unwrap().prepared_message, None);
        start_merge(&repo, first);
        let context = commit_context(&repo.root).unwrap();
        assert_eq!(context.operation, Some("merge"));
        assert_eq!(
            context.prepared_message.as_deref(),
            Some("Merge branch 'topic'")
        );
    }

    #[test]
    fn commit_context_lists_staged_unstaged_and_head_message() {
        let repo = Repo::new();
        let a = repo.write("a.txt", "one\n");
        repo.write("b.txt", "b\n");
        repo.commit_all("Initial import");
        fs::write(&a, "two\n").unwrap();
        stage_file(&a).unwrap();
        repo.write("b.txt", "b2\n");
        repo.write("c.txt", "c\n");
        let context = commit_context(&repo.root).unwrap();
        assert_eq!(context.staged, vec!["M a.txt"]);
        assert_eq!(context.unstaged, vec!["M b.txt"]);
        assert_eq!(context.untracked, vec!["c.txt"]);
        assert_eq!(context.head_message.as_deref(), Some("Initial import"));
    }

    #[test]
    fn file_history_lists_only_commits_touching_the_file() {
        let repo = Repo::new();
        let a = repo.write("a.txt", "1\n");
        repo.write("b.txt", "1\n");
        repo.commit_all("add both");
        fs::write(&a, "2\n").unwrap();
        repo.commit_all("change a");
        repo.write("b.txt", "2\n");
        repo.commit_all("change b");
        let history = file_history(&a, 10).unwrap();
        assert_eq!(
            history
                .iter()
                .map(|e| e.subject.as_str())
                .collect::<Vec<_>>(),
            vec!["change a", "add both"]
        );
        assert_eq!(history[0].author, "Test");
    }

    #[test]
    fn line_history_follows_a_line_through_its_edits() {
        let repo = Repo::new();
        let file = repo.write("a.txt", "alpha\nbeta\ngamma\n");
        repo.commit_all("create");
        fs::write(&file, "alpha\nbeta v2\ngamma\n").unwrap();
        repo.commit_all("edit beta");
        fs::write(&file, "intro\nalpha\nbeta v3\ngamma\n").unwrap();
        repo.commit_all("edit beta again and add intro");
        fs::write(&file, "intro\nalpha\nbeta v3\ngamma\ndelta\n").unwrap();
        repo.commit_all("add delta");

        // The "beta" line is line 3 (index 2) now.
        let history = line_history(&file, 2, 10).unwrap();
        assert_eq!(
            history
                .iter()
                .map(|e| e.subject.as_str())
                .collect::<Vec<_>>(),
            vec!["edit beta again and add intro", "edit beta", "create"]
        );

        // A line that was only ever added has a one-entry history.
        let history = line_history(&file, 4, 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].subject, "add delta");
    }

    #[test]
    fn line_history_starts_with_uncommitted_changes() {
        let repo = Repo::new();
        let file = repo.write("a.txt", "one\ntwo\nthree\n");
        repo.commit_all("create");
        fs::write(&file, "one\nTWO\nthree\n").unwrap();
        let history = line_history(&file, 1, 10).unwrap();
        assert_eq!(history[0].subject, "Uncommitted changes");
        assert_eq!(history[1].subject, "create");
    }

    #[test]
    fn clean_message_drops_comments_and_trailing_blank_lines() {
        assert_eq!(clean_commit_message("Fix\n\n# Please\n# enter\n\n"), "Fix");
        assert_eq!(clean_commit_message("# only\n"), "");
        assert_eq!(clean_commit_message("A\n#B\nC\n"), "A\nC");
    }
}
