//! Git from the editor: stage/unstage hunks and files, commit with a message
//! buffer (or amend), file and line history, a status list that opens diffs,
//! and merge conflict resolution.
//!
//! The heavy lifting is in [`crate::git::ops`] (libgit2, and `git commit`); this module wires it
//! to buffers, pickers and the existing diff review.

use super::picker::{GitPick, Picker, PickerRole};
use super::{Editor, ToastLevel, ToastRequest, ToastSource};
use crate::git::conflict::{self, Resolution};
use crate::git::ops::{self, GitTarget, LogEntry};
use crate::mode::Mode;
use crate::unicode::{CharCol, GraphemeCol};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// Display name of the commit message buffer.
const COMMIT_BUFFER: &str = "[COMMIT_EDITMSG]";

/// The short id and subject of a commit, or what git said when it failed.
type CommitOutcome = Result<(String, String), String>;

/// An open commit message buffer.
pub struct CommitSession {
    pub buffer_id: crate::buffer::BufferId,
    pub amend: bool,
    /// A path inside the repository being committed to.
    pub anchor: PathBuf,
    /// The commit running in the background since the message was written.
    pending: Option<Receiver<CommitOutcome>>,
}

impl Editor {
    /// A path inside the repository the git commands act on: the current file,
    /// else the project directory.
    fn git_anchor(&self) -> PathBuf {
        if let Some(session) = self.ui_panels.commit.as_ref() {
            if session.buffer_id == self.buffer().id() {
                return session.anchor.clone();
            }
        }
        match self.buffer().file_path() {
            Some(path) if !super::buffer_manager::is_scratch_path(path) => {
                let path = Path::new(path);
                path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
            }
            _ => self.picker_base_dir(),
        }
    }

    /// The root of the repository the git commands act on.
    pub fn git_root(&self) -> anyhow::Result<PathBuf> {
        ops::workdir_of(&self.git_anchor())
    }

    /// Writes the buffer when it has unsaved changes, so git sees what the
    /// user sees (and hunk line numbers line up).
    fn write_buffer_for_git(&mut self) -> Result<(), String> {
        // `:w` in the commit message buffer commits what is half written.
        if self.is_commit_message_buffer()
            || self.buffer().file_path().is_none()
            || !self.current_buffer_needs_write()
        {
            return Ok(());
        }
        match crate::commands::execute_command(self, "w") {
            crate::command_result::CommandResult::Success(_) => Ok(()),
            crate::command_result::CommandResult::Error(error) => {
                Err(format!("Cannot write the buffer first: {}", error.error))
            }
        }
    }

    /// File and cursor based commands have nothing to act on in the message
    /// buffer; refuse instead of acting on the file being committed.
    fn refuse_in_commit_buffer(&mut self) -> bool {
        let refuse = self.is_commit_message_buffer();
        if refuse {
            self.set_status_message("Not available in the commit message buffer");
        }
        refuse
    }

    fn git_report<T>(&mut self, result: anyhow::Result<T>, success: impl FnOnce(T) -> String) {
        match result {
            Ok(value) => self.set_status_message(success(value)),
            Err(error) => self.set_status_message(format!("Git: {error:#}")),
        }
    }

    fn current_file_name(&self) -> String {
        self.buffer()
            .file_path()
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string()
    }

    /// `<Space>gs` — stage the change under the cursor.
    pub fn git_stage_hunk(&mut self) {
        if self.refuse_in_commit_buffer() {
            return;
        }
        if let Err(message) = self.write_buffer_for_git() {
            self.set_status_message(message);
            return;
        }
        let anchor = self.git_anchor();
        let line = self.buffer().cursor().line();
        match ops::stage_hunk(&anchor, line) {
            Ok(true) => self.set_status_message("Staged the hunk under the cursor"),
            Ok(false) => self.set_status_message("No unstaged change at the cursor"),
            Err(error) => self.set_status_message(format!("Git: {error:#}")),
        }
    }

    /// `<Space>gu` — unstage the staged change under the cursor.
    pub fn git_unstage_hunk(&mut self) {
        if self.refuse_in_commit_buffer() {
            return;
        }
        if let Err(message) = self.write_buffer_for_git() {
            self.set_status_message(message);
            return;
        }
        let anchor = self.git_anchor();
        let line = self.buffer().cursor().line();
        match ops::unstage_hunk(&anchor, line) {
            Ok(true) => self.set_status_message("Unstaged the hunk under the cursor"),
            Ok(false) => self.set_status_message("No staged change at the cursor"),
            Err(error) => self.set_status_message(format!("Git: {error:#}")),
        }
    }

    /// `<Space>gS` — stage the whole file.
    pub fn git_stage_file(&mut self) {
        if self.refuse_in_commit_buffer() {
            return;
        }
        if let Err(message) = self.write_buffer_for_git() {
            self.set_status_message(message);
            return;
        }
        let anchor = self.git_anchor();
        let name = self.current_file_name();
        let result = ops::stage_file(&anchor);
        self.git_report(result, |()| format!("Staged {name}"));
    }

    /// `<Space>gU` — unstage the whole file.
    pub fn git_unstage_file(&mut self) {
        if self.refuse_in_commit_buffer() {
            return;
        }
        let anchor = self.git_anchor();
        let name = self.current_file_name();
        let result = ops::unstage_file(&anchor);
        self.git_report(result, |()| format!("Unstaged {name}"));
    }

    /// `:GitStageAll` — stage every change in the working tree.
    pub fn git_stage_all(&mut self) {
        if let Err(message) = self.write_buffer_for_git() {
            self.set_status_message(message);
            return;
        }
        let anchor = self.git_anchor();
        let result = ops::stage_all(&anchor);
        self.git_report(result, |()| "Staged all changes".to_string());
    }

    // -----------------------------------------------------------------
    // Status
    // -----------------------------------------------------------------

    fn git_status_rows(&self, anchor: &Path) -> anyhow::Result<(PathBuf, Vec<(String, GitPick)>)> {
        let workdir = ops::workdir_of(anchor)?;
        let rows = ops::status(anchor)?
            .into_iter()
            .map(|entry| {
                let label = match &entry.renamed_from {
                    Some(from) => format!("{from} -> {}", entry.path),
                    None => entry.path.clone(),
                };
                let target = GitTarget::in_root(&workdir, &entry.path);
                let pick = if entry.conflicted {
                    GitPick::Edit(target)
                } else {
                    GitPick::DiffFile(target)
                };
                (format!("{}  {label}", entry.code()), pick)
            })
            .collect();
        Ok((workdir, rows))
    }

    /// `<Space>gg` / `:GitStatus` — changed files; Enter opens the diff,
    /// `Ctrl-T` stages or unstages, `Ctrl-E` edits the file.
    pub fn open_git_status_picker(&mut self) {
        let anchor = self.git_anchor();
        match self.git_status_rows(&anchor) {
            Ok((_, rows)) if rows.is_empty() => self.set_status_message("Working tree clean"),
            Ok((workdir, rows)) => {
                let picker =
                    Picker::new_git(workdir, rows, "Git status").with_role(PickerRole::GitStatus);
                self.set_picker(picker);
                self.set_mode(Mode::Picker);
                self.mark_picker_selection_changed();
            }
            Err(error) => self.set_status_message(format!("Git: {error:#}")),
        }
    }

    fn selected_git_status_target(&self) -> Option<GitTarget> {
        let picker = self.picker()?;
        (picker.role() == Some(PickerRole::GitStatus))
            .then(|| picker.selected_git_pick())
            .flatten()
            .and_then(GitPick::target)
            .cloned()
    }

    /// `Ctrl-T` in the status list: stage the selected file, or unstage it
    /// when everything in it is already staged.
    pub fn git_status_toggle_selected(&mut self) {
        let Some(target) = self.selected_git_status_target() else {
            return;
        };
        let entry = ops::status(&target.root).ok().and_then(|entries| {
            entries
                .into_iter()
                .find(|entry| Path::new(&entry.path) == target.relative)
        });
        let Some(entry) = entry else {
            return;
        };
        let name = entry.path.clone();
        let outcome = if entry.has_unstaged_changes() {
            ops::stage_target(&target).map(|()| format!("Staged {name}"))
        } else {
            // A staged rename is a deletion and an addition; unstaging only
            // the new path would leave the deletion of the old one staged.
            let old = entry
                .renamed_from
                .as_deref()
                .map(|from| GitTarget::in_root(&target.root, from));
            ops::unstage_target(&target)
                .and_then(|()| old.map_or(Ok(()), |old| ops::unstage_target(&old)))
                .map(|()| format!("Unstaged {name}"))
        };
        match outcome {
            Ok(message) => {
                self.set_status_message(message);
                self.refresh_git_status_picker();
            }
            Err(error) => self.set_status_message(format!("Git: {error:#}")),
        }
    }

    /// Rebuilds the status list after it changed under the cursor.
    pub fn refresh_git_status_picker(&mut self) {
        let Some(anchor) = self
            .picker()
            .filter(|picker| picker.role() == Some(PickerRole::GitStatus))
            .map(|picker| picker.base_dir().to_path_buf())
        else {
            return;
        };
        match self.git_status_rows(&anchor) {
            Ok((_, rows)) if rows.is_empty() => {
                self.close_picker();
                self.set_mode(Mode::Normal);
                self.set_status_message("Working tree clean");
            }
            Ok((_, rows)) => {
                if let Some(picker) = self.picker_mut() {
                    picker.replace_git_rows(rows);
                }
            }
            Err(error) => self.set_status_message(format!("Git: {error:#}")),
        }
    }

    /// `Ctrl-E` in the status list: open the file itself.
    pub fn git_status_edit_selected(&mut self) {
        let Some(target) = self.selected_git_status_target() else {
            return;
        };
        self.close_picker();
        self.set_mode(Mode::Normal);
        self.git_edit(&target);
    }

    fn git_edit(&mut self, target: &GitTarget) {
        let path = target.absolute();
        if let Err(error) = self.load_file(&path) {
            self.set_status_message(format!("Failed to open {}: {error}", path.display()));
        }
    }

    /// Performs what Enter on a status / history row stands for.
    pub fn run_git_pick(&mut self, pick: GitPick) {
        let result = match pick {
            GitPick::DiffFile(target) => self.git_show_file_diff(&target),
            GitPick::Edit(target) => {
                self.git_edit(&target);
                Ok(())
            }
            GitPick::Show { root, oid, path } => self.git_show_commit(&root, &oid, path.as_deref()),
            GitPick::DiffHead => self.open_diff_review(Some("HEAD")),
        };
        if let Err(error) = result {
            self.set_status_message(format!("Git: {error:#}"));
        }
    }

    /// Opens the diff review of uncommitted changes, positioned on `target`.
    pub fn git_show_file_diff(&mut self, target: &GitTarget) -> anyhow::Result<()> {
        let relative = target.relative.to_string_lossy().to_string();
        self.open_diff_review(Some("HEAD"))?;
        if !self.diff_review_jump_to_path(&relative) {
            self.set_status_message(format!("{relative} has no changes against HEAD"));
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // Commit
    // -----------------------------------------------------------------

    pub fn is_commit_message_buffer(&self) -> bool {
        self.ui_panels
            .commit
            .as_ref()
            .is_some_and(|session| session.buffer_id == self.buffer().id())
    }

    /// `<Space>gc` / `:GitCommit` (and `<Space>gC` / `:GitAmend`): a message
    /// buffer to write with `:w` / `ZZ`, abort with `:q!`.
    pub fn open_commit_message(&mut self, amend: bool) {
        if self.is_commit_message_buffer() {
            return;
        }
        if let Err(message) = self.write_buffer_for_git() {
            self.set_status_message(message);
            return;
        }
        let anchor = self.git_anchor();
        let context = match ops::commit_context(&anchor) {
            Ok(context) => context,
            Err(error) => {
                self.set_status_message(format!("Git: {error:#}"));
                return;
            }
        };
        if !context.conflicted.is_empty() {
            self.set_status_message(format!(
                "Unresolved conflicts in {}: resolve them and stage the files first",
                context.conflicted.join(", ")
            ));
            return;
        }
        if !amend && context.staged.is_empty() && !context.merging {
            self.set_status_message(
                if context.unstaged.is_empty() && context.untracked.is_empty() {
                    "Nothing to commit: the working tree is clean".to_string()
                } else {
                    "Nothing staged: stage with <Space>gs / <Space>gS, or in :GitStatus (Ctrl-T)"
                        .to_string()
                },
            );
            return;
        }
        if amend && context.head_message.is_none() {
            self.set_status_message("Nothing to amend: there are no commits yet");
            return;
        }
        if let Some(operation) = context.operation.filter(|_| amend) {
            self.set_status_message(format!(
                "You are in the middle of a {operation} -- cannot amend"
            ));
            return;
        }

        let mut text = String::new();
        if amend {
            text.push_str(context.head_message.as_deref().unwrap_or(""));
        } else if let Some(prepared) = &context.prepared_message {
            text.push_str(prepared);
        }
        text.push_str("\n\n# Write the commit message. Lines starting with '#' are ignored.\n");
        text.push_str(&format!(
            "# :w or ZZ {}, :q! aborts.\n#\n",
            if amend {
                "amends the last commit"
            } else {
                "commits"
            }
        ));
        if let Some(branch) = &context.branch {
            text.push_str(&format!("# On branch {branch}\n"));
        }
        match context.operation {
            Some("merge") => text.push_str("# All conflicts fixed but you are still merging.\n"),
            Some(operation) => text.push_str(&format!(
                "# All conflicts fixed but the {operation} is not finished.\n"
            )),
            None => {}
        }
        let section = |title: &str, lines: &[String]| {
            if lines.is_empty() {
                String::new()
            } else {
                let mut out = format!("#\n# {title}:\n");
                for line in lines {
                    out.push_str(&format!("#\t{line}\n"));
                }
                out
            }
        };
        text.push_str(&section("Changes to be committed", &context.staged));
        text.push_str(&section("Changes not staged for commit", &context.unstaged));
        text.push_str(&section("Untracked files", &context.untracked));

        // A fresh tab with an editable scratch buffer (never written to disk,
        // ignored by the quit guard because of its bracketed name).
        self.clear_definition_returns();
        self.sync_current_tab_buffer();
        let mut buffer = crate::buffer::Buffer::new_from_str(&text);
        buffer.set_file_path(COMMIT_BUFFER.to_string());
        let index = self.push_buffer(buffer);
        self.tab_page_manager.new_tab();
        let id = self.buffers[index].id();
        self.tab_page_manager.current_tab_mut().set_buffer_id(id);
        self.current_buffer_index = index;
        self.clear_lsp_state();
        self.lsp.state.needs_lsp_init = false;
        self.buffer_mut()
            .cursor_mut()
            .set_position(0, GraphemeCol(0));
        self.ui_panels.commit = Some(Box::new(CommitSession {
            buffer_id: id,
            amend,
            anchor,
            pending: None,
        }));
        // Start typing right away when the message is empty.
        if !amend {
            self.start_change_building(self.cursor_position());
            self.set_mode(Mode::Insert);
        } else {
            self.set_mode(Mode::Normal);
        }
        self.set_status_message(if amend {
            "Amending: edit the message, :w to amend, :q! to abort"
        } else {
            "Commit message: :w to commit, :q! to abort"
        });
        self.mark_dirty();
    }

    /// Ends the commit message buffer: starts the commit (`commit == true`)
    /// or aborts. The commit runs in the background because hooks can take a
    /// long time; [`Self::poll_git_commit`] finishes it. A failed commit keeps
    /// the buffer open so the message is not lost.
    pub fn finish_commit_message(&mut self, commit: bool) {
        let Some(session) = self.ui_panels.commit.as_ref() else {
            return;
        };
        if session.pending.is_some() {
            self.set_status_message("A commit is already running");
            return;
        }
        if !commit {
            if let Some(session) = self.ui_panels.commit.take() {
                self.close_commit_tab(&session);
            }
            self.set_status_message("Commit aborted");
            return;
        }
        let (anchor, amend) = (session.anchor.clone(), session.amend);
        let text = self.buffer().rope().to_string();
        let prepared = match ops::prepare_commit(&anchor, &text, amend) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.set_status_message(format!("{error:#}"));
                return;
            }
        };
        let (sender, receiver) = channel();
        std::thread::spawn(move || {
            let _ = sender.send(prepared.run().map_err(|error| format!("{error:#}")));
        });
        if let Some(session) = self.ui_panels.commit.as_mut() {
            session.pending = Some(receiver);
        }
        self.set_status_message(if amend {
            "Amending…"
        } else {
            "Committing…"
        });
    }

    /// True while a commit started from the message buffer is running.
    pub fn git_commit_pending(&self) -> bool {
        self.ui_panels
            .commit
            .as_ref()
            .is_some_and(|session| session.pending.is_some())
    }

    /// Finishes a background commit once it ends. Returns true when something
    /// changed.
    pub fn poll_git_commit(&mut self) -> bool {
        let Some(session) = self.ui_panels.commit.as_mut() else {
            return false;
        };
        let Some(receiver) = session.pending.as_ref() else {
            return false;
        };
        let outcome = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("The commit stopped unexpectedly".to_string()),
        };
        session.pending = None;
        match outcome {
            Ok((short, subject)) => {
                if let Some(session) = self.ui_panels.commit.take() {
                    self.close_commit_tab(&session);
                    self.set_status_message(format!(
                        "{} {short}: {subject}",
                        if session.amend {
                            "Amended"
                        } else {
                            "Committed"
                        }
                    ));
                }
                self.refresh_git_after_commit();
            }
            Err(output) => self.report_commit_failure(&output),
        }
        true
    }

    /// Shows what git said when it refused the commit (a hook's output, a
    /// signing failure, ...). The message buffer stays open.
    fn report_commit_failure(&mut self, output: &str) {
        let last = output.lines().rev().find(|line| !line.trim().is_empty());
        self.set_status_message(format!(
            "Commit failed: {}",
            last.unwrap_or("git commit failed")
        ));
        let lines: Vec<&str> = output.lines().collect();
        let tail = lines[lines.len().saturating_sub(12)..].join("\n");
        self.push_toast(
            ToastRequest::new(ToastSource::Git, ToastLevel::Error, tail)
                .with_title("Commit failed"),
        );
    }

    fn close_commit_tab(&mut self, session: &CommitSession) {
        if let Some(index) = self.find_buffer_index_by_id(session.buffer_id) {
            // Empty and clean: it stays in the list as an inert scratch buffer.
            self.buffers[index].replace_content("");
            self.buffers[index].mark_clean();
            self.buffers[index].change_manager_mut().mark_saved();
        }
        // The commit may have outlived the user's stay in its tab.
        let tab = self
            .tab_page_manager
            .tabs()
            .iter()
            .position(|tab| tab.buffer_id() == Some(session.buffer_id));
        let in_commit_tab = tab.is_none_or(|tab| tab == self.current_tab_index());
        match tab {
            Some(tab) if !in_commit_tab && self.tab_count() > 1 => {
                self.sync_current_tab_buffer();
                self.tab_page_manager.close_tab(tab);
            }
            _ if self.tab_count() > 1 => self.close_current_tab(),
            _ => {}
        }
        if in_commit_tab {
            self.set_mode(Mode::Normal);
        }
    }

    fn refresh_git_after_commit(&mut self) {
        if let Some(path) = self.buffer().file_path().map(str::to_string) {
            if !super::buffer_manager::is_scratch_path(&path) {
                self.git_branch = crate::git::branch_name(&path);
                self.spawn_git_refresh(&path, self.options.blame);
            }
        }
        self.mark_dirty();
    }

    // -----------------------------------------------------------------
    // History
    // -----------------------------------------------------------------

    fn history_rows(entries: Vec<LogEntry>, root: &Path) -> Vec<(String, GitPick)> {
        entries
            .into_iter()
            .map(|entry| {
                let pick = if entry.oid.is_empty() {
                    // Uncommitted changes: the working tree diff of the file.
                    match entry.path {
                        Some(path) => GitPick::DiffFile(GitTarget::in_root(root, path)),
                        None => GitPick::DiffHead,
                    }
                } else {
                    GitPick::Show {
                        root: root.to_path_buf(),
                        oid: entry.oid,
                        path: entry.path,
                    }
                };
                let display = format!(
                    "{}  {}  {}  {}",
                    entry.short, entry.date, entry.author, entry.subject
                );
                (display, pick)
            })
            .collect()
    }

    fn show_history(&mut self, title: &str, result: anyhow::Result<Vec<LogEntry>>) {
        let anchor = self.git_anchor();
        match result {
            Ok(entries) if entries.is_empty() => {
                self.set_status_message("No history found (is the file committed?)")
            }
            Ok(entries) => {
                let root = ops::workdir_of(&anchor).unwrap_or_else(|_| self.picker_base_dir());
                let rows = Self::history_rows(entries, &root);
                let picker = Picker::new_git(root, rows, title);
                self.set_picker(picker);
                self.set_mode(Mode::Picker);
                self.mark_picker_selection_changed();
            }
            Err(error) => self.set_status_message(format!("Git: {error:#}")),
        }
    }

    /// `<Space>gl` / `:GitLog` — commits that changed the current file.
    pub fn open_file_history_picker(&mut self) {
        if self.refuse_in_commit_buffer() {
            return;
        }
        let anchor = self.git_anchor();
        let name = self.current_file_name();
        let result = ops::file_history(&anchor, 300);
        self.show_history(&format!("History of {name}"), result);
    }

    /// `:GitLogAll` — the repository's recent commits.
    pub fn open_repo_history_picker(&mut self) {
        let anchor = self.git_anchor();
        let root = ops::workdir_of(&anchor).unwrap_or_else(|_| anchor.clone());
        let result = ops::file_history(&root, 300);
        self.show_history("Commits", result);
    }

    /// `<Space>gL` / `:GitLineLog` — commits that changed the cursor line,
    /// following the line back through edits. Enter shows the commit's diff.
    pub fn open_line_history_picker(&mut self) {
        if self.refuse_in_commit_buffer() {
            return;
        }
        if let Err(message) = self.write_buffer_for_git() {
            self.set_status_message(message);
            return;
        }
        let anchor = self.git_anchor();
        let line = self.buffer().cursor().line();
        let result = ops::line_history(&anchor, line, 50);
        self.show_history(&format!("History of line {}", line + 1), result);
    }

    /// Enter on a history entry: the commit's diff in the review UI, on the
    /// file when known.
    pub fn git_show_commit(
        &mut self,
        root: &Path,
        oid: &str,
        path: Option<&str>,
    ) -> anyhow::Result<()> {
        let repo = git2::Repository::open(root)?;
        let commit = repo.find_commit(git2::Oid::from_str(oid)?)?;
        if commit.parent_count() == 0 {
            // The review compares against a parent; a root commit has none.
            let diff = crate::git::commit_diff(root, oid)?;
            self.open_diff_buffer_in_new_tab(&format!("Commit {}", &oid[..7]), &diff);
            self.buffer_mut()
                .enable_syntax_highlighting_for_path("commit.diff");
            return Ok(());
        }
        let short = &oid[..oid.len().min(10)];
        self.open_diff_review(Some(&format!("{short}^..{short}")))?;
        if let Some(path) = path.filter(|path| !path.is_empty()) {
            self.diff_review_jump_to_path(path);
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // Merge conflicts
    // -----------------------------------------------------------------

    fn buffer_lines(&self) -> Vec<String> {
        let buffer = self.buffer();
        (0..buffer.line_count())
            .map(|line| {
                buffer
                    .line_text(line)
                    .map(|text| text.trim_end_matches(['\n', '\r']).to_string())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Conflict blocks in the current buffer.
    pub fn buffer_conflicts(&self) -> Vec<conflict::Conflict> {
        conflict::find_conflicts(&self.buffer_lines())
    }

    /// `]n` / `[n` — jump to the next / previous conflict marker block.
    pub fn goto_conflict(&mut self, forward: bool) {
        let conflicts = self.buffer_conflicts();
        if conflicts.is_empty() {
            self.set_status_message("No merge conflicts in this buffer");
            return;
        }
        let line = self.buffer().cursor().line();
        let target = if forward {
            conflicts
                .iter()
                .find(|conflict| conflict.start > line)
                .or_else(|| conflicts.first())
        } else {
            conflicts
                .iter()
                .rev()
                .find(|conflict| conflict.end < line || conflict.start < line)
                .or_else(|| conflicts.last())
        };
        if let Some(target) = target {
            let (start, index) = (
                target.start,
                conflicts
                    .iter()
                    .position(|c| c.start == target.start)
                    .unwrap_or(0)
                    + 1,
            );
            self.buffer_mut()
                .cursor_mut()
                .set_position(start, GraphemeCol(0));
            self.buffer_mut().validate_cursor_position();
            self.center_cursor_in_viewport();
            self.set_status_message(format!(
                "Conflict {index} of {}: {} vs {}",
                conflicts.len(),
                conflicts[index - 1].ours_label,
                conflicts[index - 1].theirs_label,
            ));
        }
    }

    /// Resolves the conflict under the cursor (or the next one below it).
    pub fn resolve_conflict(&mut self, resolution: Resolution) {
        let lines = self.buffer_lines();
        let conflicts = conflict::find_conflicts(&lines);
        let cursor = self.buffer().cursor().line();
        let Some(target) = conflict::conflict_at_or_after(&conflicts, cursor).cloned() else {
            self.set_status_message("No merge conflict at or below the cursor");
            return;
        };
        let replacement = conflict::resolved_lines(&lines, &target, resolution);
        let mut text = replacement.join("\n");
        if !replacement.is_empty() {
            text.push('\n');
        }
        let cursor_before = self.cursor_position();
        let ((), edits) = self.buffer_mut().record(|buf| {
            buf.delete_range(target.start, CharCol(0), target.end + 1, CharCol(0));
            buf.insert_text_at(target.start, CharCol(0), &text);
        });
        if !edits.is_empty() {
            self.buffer_mut()
                .cursor_mut()
                .set_position(target.start, GraphemeCol(0));
            self.buffer_mut().validate_cursor_position();
            let cursor_after = self.cursor_position();
            self.push_recorded_undo(edits, cursor_before, cursor_after);
        }
        let remaining = self.buffer_conflicts().len();
        self.set_status_message(format!(
            "Kept {}; {} conflict{} left in this buffer",
            resolution.label(),
            remaining,
            if remaining == 1 { "" } else { "s" }
        ));
        self.mark_dirty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Repo {
        _dir: tempfile::TempDir,
        root: PathBuf,
        repo: git2::Repository,
    }

    fn repo() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let repo = git2::Repository::init(&root).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "t@example.com").unwrap();
        // Commits run through the git binary: keep the developer's global
        // signing and hook configuration out of the tests.
        config.set_bool("commit.gpgsign", false).unwrap();
        config
            .set_str("core.hooksPath", root.join(".git/hooks").to_str().unwrap())
            .unwrap();
        Repo {
            _dir: dir,
            root,
            repo,
        }
    }

    /// Waits for the background commit started by `:w` in the message buffer.
    fn settle_commit(editor: &mut Editor) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while editor.git_commit_pending() {
            editor.poll_git_commit();
            assert!(
                std::time::Instant::now() < deadline,
                "the commit did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn commit_all(repo: &Repo, message: &str) {
        let mut index = repo.repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = git2::Signature::now("Test", "t@example.com").unwrap();
        let parent = repo.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn commit_buffer_commits_on_write_and_keeps_the_message_when_it_fails() {
        let repo = repo();
        fs::write(repo.root.join("a.txt"), "one\n").unwrap();
        commit_all(&repo, "init");
        fs::write(repo.root.join("a.txt"), "two\n").unwrap();
        let mut editor = Editor::default();
        editor.load_file(repo.root.join("a.txt")).unwrap();

        // Nothing staged yet: the buffer does not open.
        editor.open_commit_message(false);
        assert!(!editor.is_commit_message_buffer());
        assert!(editor.status_message().contains("Nothing staged"));

        editor.git_stage_file();
        editor.open_commit_message(false);
        assert!(editor.is_commit_message_buffer());
        let text = editor.buffer().rope().to_string();
        assert!(text.contains("# Changes to be committed:"), "{text}");
        assert!(text.contains("#\tM a.txt"), "{text}");

        // An empty message fails and keeps the buffer.
        editor.finish_commit_message(true);
        assert!(
            editor.is_commit_message_buffer(),
            "still open after a failure"
        );
        assert!(
            editor.status_message().contains("empty"),
            "{}",
            editor.status_message()
        );

        editor
            .buffer_mut()
            .insert_text_at(0, CharCol(0), "Change a");
        editor.finish_commit_message(true);
        assert_eq!(editor.status_message(), "Committing…");
        assert!(editor.git_commit_pending());
        settle_commit(&mut editor);
        assert!(!editor.is_commit_message_buffer());
        assert!(
            editor.status_message().starts_with("Committed"),
            "{}",
            editor.status_message()
        );
        let head = repo.repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message().unwrap(), "Change a\n");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn aborting_the_commit_buffer_leaves_the_repository_alone() {
        let repo = repo();
        fs::write(repo.root.join("a.txt"), "one\n").unwrap();
        commit_all(&repo, "init");
        fs::write(repo.root.join("a.txt"), "two\n").unwrap();
        let mut editor = Editor::default();
        editor.load_file(repo.root.join("a.txt")).unwrap();
        editor.git_stage_file();
        editor.open_commit_message(false);
        editor
            .buffer_mut()
            .insert_text_at(0, CharCol(0), "will not land");
        editor.finish_commit_message(false);
        assert!(!editor.is_commit_message_buffer());
        assert_eq!(editor.status_message(), "Commit aborted");
        assert_eq!(
            repo.repo
                .head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .message()
                .unwrap(),
            "init"
        );
        assert!(editor.buffer().file_path().unwrap().ends_with("a.txt"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn amend_prefills_the_last_message() {
        let repo = repo();
        fs::write(repo.root.join("a.txt"), "one\n").unwrap();
        commit_all(&repo, "First try");
        let mut editor = Editor::default();
        editor.load_file(repo.root.join("a.txt")).unwrap();
        editor.open_commit_message(true);
        assert!(editor.is_commit_message_buffer());
        assert!(editor
            .buffer()
            .rope()
            .to_string()
            .starts_with("First try\n"));
        editor.buffer_mut().replace_content("Second try\n");
        editor.finish_commit_message(true);
        settle_commit(&mut editor);
        assert!(
            editor.status_message().starts_with("Amended"),
            "{}",
            editor.status_message()
        );
        assert_eq!(
            repo.repo
                .head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .message()
                .unwrap(),
            "Second try\n"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn stage_hunk_writes_the_buffer_first_and_stages_only_that_hunk() {
        let repo = repo();
        let ten = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";
        fs::write(repo.root.join("a.txt"), ten).unwrap();
        commit_all(&repo, "init");
        let mut editor = Editor::default();
        editor.load_file(repo.root.join("a.txt")).unwrap();
        // Two unsaved edits in the buffer.
        editor
            .buffer_mut()
            .delete_range(1, CharCol(0), 1, CharCol(2));
        editor.buffer_mut().insert_text_at(1, CharCol(0), "L2");
        editor
            .buffer_mut()
            .delete_range(8, CharCol(0), 8, CharCol(2));
        editor.buffer_mut().insert_text_at(8, CharCol(0), "L9");
        editor.mark_buffer_modified();
        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(8, GraphemeCol(0));

        editor.git_stage_hunk();
        assert_eq!(editor.status_message(), "Staged the hunk under the cursor");
        let statuses = ops::status(&repo.root).unwrap();
        assert_eq!(
            statuses[0].code(),
            "MM",
            "one hunk staged, the other still not"
        );
        let mut index = repo.repo.index().unwrap();
        index.read(true).unwrap();
        let entry = index.get_path(Path::new("a.txt"), 0).unwrap();
        let staged =
            String::from_utf8(repo.repo.find_blob(entry.id).unwrap().content().to_vec()).unwrap();
        assert_eq!(staged, "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nL9\nl10\n");

        editor.git_unstage_hunk();
        assert_eq!(
            editor.status_message(),
            "Unstaged the hunk under the cursor"
        );
        assert_eq!(ops::status(&repo.root).unwrap()[0].code(), " M");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn status_picker_lists_changes_and_ctrl_t_stages_and_unstages() {
        let repo = repo();
        fs::write(repo.root.join("a.txt"), "one\n").unwrap();
        fs::write(repo.root.join("b.txt"), "b\n").unwrap();
        commit_all(&repo, "init");
        fs::write(repo.root.join("a.txt"), "two\n").unwrap();
        fs::write(repo.root.join("new.txt"), "n\n").unwrap();
        let mut editor = Editor::default();
        editor.load_file(repo.root.join("b.txt")).unwrap();

        editor.open_git_status_picker();
        let rows: Vec<String> = editor
            .picker()
            .unwrap()
            .collect_filtered_results(10)
            .into_iter()
            .map(|r| r.display.clone())
            .collect();
        assert_eq!(rows, vec![" M  a.txt", "??  new.txt"]);
        assert_eq!(editor.picker().unwrap().title(), Some("Git status"));

        editor.git_status_toggle_selected();
        let rows: Vec<String> = editor
            .picker()
            .unwrap()
            .collect_filtered_results(10)
            .into_iter()
            .map(|r| r.display.clone())
            .collect();
        assert_eq!(rows[0], "M   a.txt", "staged in place, selection kept");
        editor.git_status_toggle_selected();
        assert_eq!(
            ops::status(&repo.root).unwrap()[0].code(),
            " M",
            "toggled back"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn status_picker_toggles_both_paths_of_a_staged_rename() {
        let repo = repo();
        fs::write(repo.root.join("x.txt"), "some content\nof the file\n").unwrap();
        fs::write(repo.root.join("keep.txt"), "k\n").unwrap();
        commit_all(&repo, "init");
        fs::rename(repo.root.join("x.txt"), repo.root.join("y.txt")).unwrap();
        ops::stage_all(&repo.root).unwrap();
        let mut editor = Editor::default();
        editor.load_file(repo.root.join("keep.txt")).unwrap();

        editor.open_git_status_picker();
        let rows = |editor: &Editor| -> Vec<String> {
            editor
                .picker()
                .unwrap()
                .collect_filtered_results(10)
                .into_iter()
                .map(|r| r.display.clone())
                .collect()
        };
        assert_eq!(rows(&editor), vec!["R   x.txt -> y.txt"]);

        // Unstaging only `y.txt` would leave the deletion of `x.txt` staged.
        editor.git_status_toggle_selected();
        assert_eq!(rows(&editor), vec![" D  x.txt", "??  y.txt"]);
    }

    #[test]
    fn working_tree_renames_are_listed_as_a_deletion_and_an_untracked_file() {
        let repo = repo();
        fs::write(repo.root.join("x.txt"), "some content\nof the file\n").unwrap();
        commit_all(&repo, "init");
        fs::rename(repo.root.join("x.txt"), repo.root.join("y.txt")).unwrap();
        // `git status` does not pair them up either.
        let codes: Vec<String> = ops::status(&repo.root)
            .unwrap()
            .iter()
            .map(|entry| format!("{} {}", entry.code(), entry.path))
            .collect();
        assert_eq!(codes, vec![" D x.txt", "?? y.txt"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn conflict_navigation_and_resolution_edit_the_buffer_and_undo() {
        let repo = repo();
        let text = "top\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> topic\nmid\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> topic\nend\n";
        fs::write(repo.root.join("c.txt"), text).unwrap();
        let mut editor = Editor::default();
        editor.load_file(repo.root.join("c.txt")).unwrap();

        editor.goto_conflict(true);
        assert_eq!(editor.buffer().cursor().line(), 1);
        assert!(
            editor.status_message().starts_with("Conflict 1 of 2"),
            "{}",
            editor.status_message()
        );
        editor.goto_conflict(true);
        assert_eq!(editor.buffer().cursor().line(), 7);
        editor.goto_conflict(true);
        assert_eq!(editor.buffer().cursor().line(), 1, "wraps around");
        editor.goto_conflict(false);
        assert_eq!(editor.buffer().cursor().line(), 7, "previous wraps too");

        editor
            .buffer_mut()
            .cursor_mut()
            .set_position(2, GraphemeCol(0));
        editor.resolve_conflict(Resolution::Theirs);
        assert_eq!(
            editor.buffer().rope().to_string(),
            "top\ntheirs\nmid\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> topic\nend\n"
        );
        editor.resolve_conflict(Resolution::Both);
        assert_eq!(
            editor.buffer().rope().to_string(),
            "top\ntheirs\nmid\na\nb\nend\n"
        );
        assert!(
            editor.status_message().contains("0 conflicts left"),
            "{}",
            editor.status_message()
        );

        editor.undo();
        assert!(editor
            .buffer()
            .rope()
            .to_string()
            .contains("<<<<<<< HEAD\na"));
    }
}
