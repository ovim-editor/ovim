//! Git operations behind the editor's staging, commit and history commands.
//!
//! Everything goes through libgit2, so it needs no `git` binary and behaves the
//! same in the TUI, the GUI and headless sessions. Functions take any path
//! inside the repository and work on the repository's working tree.

use anyhow::{anyhow, bail, Context, Result};
use chrono::{TimeZone, Utc};
use git2::{
    BlameOptions, DiffOptions, ErrorCode, IndexAddOption, Oid, Repository, Status, StatusOptions,
};
use std::path::{Path, PathBuf};

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
    let workdir = repo
        .workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("bare repositories have no working tree"))?;
    Ok(workdir.canonicalize().unwrap_or(workdir))
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
    let mut entries = Vec::new();
    for entry in statuses.iter() {
        let flags = entry.status();
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
    let mut index = repo.index()?;
    let exists = repo
        .workdir()
        .map(|workdir| workdir.join(&relative).exists())
        .unwrap_or(false);
    if exists {
        index.add_path(&relative)?;
    } else {
        index.remove_path(&relative)?;
    }
    index.write()?;
    Ok(())
}

/// Stages every change in the working tree (`git add -A`).
pub fn stage_all(path: &Path) -> Result<()> {
    let (repo, _) = open(path)?;
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
    index.update_all(["*"].iter(), None)?;
    index.write()?;
    Ok(())
}

/// Removes the file from the index, restoring the HEAD version there.
pub fn unstage_file(path: &Path) -> Result<()> {
    let (repo, relative) = open(path)?;
    let tree = match repo.head().and_then(|head| head.peel_to_tree()) {
        Ok(tree) => Some(tree),
        Err(error) if error.code() == ErrorCode::UnbornBranch => None,
        Err(error) => return Err(error.into()),
    };
    let source = tree.as_ref().and_then(|tree| tree.get_path(&relative).ok());
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
    if let Err(error) = index.remove_path(&relative) {
        if error.code() != ErrorCode::NotFound {
            return Err(error.into());
        }
    }
    if let Some(source) = source {
        let mut entry = empty_index_entry(&relative);
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

/// Replaces the index entry of `relative` with `content`.
fn write_index_text(repo: &Repository, relative: &Path, content: &[u8]) -> Result<()> {
    let mut index = repo.index()?;
    let mut entry = index
        .get_path(relative, 0)
        .unwrap_or_else(|| empty_index_entry(relative));
    entry.file_size = content.len() as u32;
    index.add_frombuffer(&entry, content)?;
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
    let Some((old_start, old_lines, new_start, new_lines)) = hunk_ranges(&diff)?
        .into_iter()
        .find(|(_, _, start, lines)| hunk_covers(*start, *lines, target))
    else {
        return Ok(false);
    };
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow!("bare repositories have no working tree"))?;
    let worktree = std::fs::read(workdir.join(&relative))?;
    let staged = index_text(&repo, &relative)?;
    let merged = splice_hunk(
        &staged,
        side_range(old_start, old_lines),
        &worktree,
        side_range(new_start, new_lines),
    );
    write_index_text(&repo, &relative, &merged)?;
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
        write_index_text(&repo, &relative, &reverted)?;
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
    pub conflicted: Vec<String>,
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
    Ok(CommitContext {
        workdir: workdir_of(path)?,
        branch,
        staged,
        unstaged,
        untracked,
        head_message,
        merging: repo.state() == git2::RepositoryState::Merge,
        conflicted,
    })
}

/// Turns the text of a commit message buffer into the message: `#` lines are
/// comments, trailing blank lines are dropped.
pub fn clean_commit_message(text: &str) -> String {
    let kept: Vec<&str> = text.lines().filter(|line| !line.starts_with('#')).collect();
    kept.join("\n").trim().to_string()
}

/// Commits the index. With `amend` the previous commit is replaced.
///
/// Returns the new commit's short id and subject.
pub fn commit(path: &Path, message: &str, amend: bool) -> Result<(String, String)> {
    let message = clean_commit_message(message);
    if message.is_empty() {
        bail!("Aborting commit: the message is empty");
    }
    let (repo, _) = open(path)?;
    let mut index = repo.index()?;
    if index.has_conflicts() {
        bail!("Cannot commit: unresolved merge conflicts (resolve, then stage the files)");
    }
    let tree = repo.find_tree(index.write_tree()?)?;
    let signature = repo.signature().map_err(|error| {
        anyhow!("Cannot commit: set user.name and user.email in your git config ({error})")
    })?;
    let head = repo.head().ok().and_then(|head| head.peel_to_commit().ok());

    let oid = if amend {
        let head = head.ok_or_else(|| anyhow!("Nothing to amend: there are no commits yet"))?;
        head.amend(
            Some("HEAD"),
            None,
            Some(&signature),
            None,
            Some(&message),
            Some(&tree),
        )?
    } else {
        if let Some(parent) = &head {
            if parent.tree_id() == tree.id() && repo.state() != git2::RepositoryState::Merge {
                bail!("Nothing to commit: no changes are staged");
            }
        } else if tree.is_empty() {
            bail!("Nothing to commit: no changes are staged");
        }
        let mut parents: Vec<git2::Commit> = head.into_iter().collect();
        if repo.state() == git2::RepositoryState::Merge {
            let merge_heads: Vec<Oid> = std::fs::read_to_string(repo.path().join("MERGE_HEAD"))
                .unwrap_or_default()
                .lines()
                .filter_map(|line| Oid::from_str(line.trim()).ok())
                .collect();
            for oid in merge_heads {
                parents.push(repo.find_commit(oid)?);
            }
        }
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            &message,
            &tree,
            &parent_refs,
        )?
    };
    if repo.state() == git2::RepositoryState::Merge {
        repo.cleanup_state()?;
    }
    let subject = message.lines().next().unwrap_or("").to_string();
    Ok((short(oid), subject))
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
        options.pathspec(&relative).disable_pathspec_match(true);
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
        assert_eq!(head.message().unwrap(), "First\n\nbody");

        assert!(commit(&repo.root, "Second", false)
            .unwrap_err()
            .to_string()
            .contains("Nothing to commit"));

        fs::write(&a, "two\n").unwrap();
        stage_file(&a).unwrap();
        commit(&repo.root, "First, amended", true).unwrap();
        let head = repo.repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message().unwrap(), "First, amended");
        assert_eq!(head.parent_count(), 0, "amend replaced the commit");
        assert_eq!(repo.index_text("a.txt"), "two\n");
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
