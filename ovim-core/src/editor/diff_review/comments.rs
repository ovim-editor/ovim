//! User annotations are separate from the immutable patch and its source maps.
use super::{DiffReviewState, Editor, Rendered};

use crate::mode::Mode;
use crate::unicode::GraphemeCol;
use crate::{KeyCode, KeyEvent, Modifiers};
use serde::Serialize;

const MAX_COMMENT_BYTES: usize = 32_768;

fn comment_text(text: &str) -> Result<String, String> {
    let normalized = crate::buffer::normalize_for_buffer(text);
    let text = normalized.trim();
    if text.is_empty() || text.len() > MAX_COMMENT_BYTES {
        return Err("Write a comment of 1–32768 bytes".into());
    }
    Ok(text.into())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffComment {
    pub id: u64,
    pub path: String,
    pub line: usize,
    pub side: String,
    pub before: bool,
    pub text: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffCommentsAttachment {
    pub review_key: String,
    pub comparison: String,
    pub comments: Vec<DiffComment>,
}

impl DiffCommentsAttachment {
    pub fn label(&self) -> String {
        format!(
            "{} diff comment{}",
            self.comments.len(),
            if self.comments.len() == 1 { "" } else { "s" }
        )
    }
    pub fn message(&self) -> String {
        let mut text = format!("Diff review comments ({})\nComparison: {}\nThese comments refer to the captured source below; line numbers may have changed.\n", self.comments.len(), self.comparison);
        for comment in &self.comments {
            text.push_str(&format!(
                "\n{}:{} ({} side, {} line)\nSource: {}\nComment: {}\n",
                comment.path,
                comment.line,
                comment.side,
                if comment.before { "before" } else { "after" },
                comment.excerpt,
                comment.text
            ));
        }
        text
    }
}

pub(super) struct CommentDraft {
    id: u64,
    original: Option<String>,
    text: String,
    cursor: usize,
}

pub(super) fn review_identity(patch: &crate::native_diff::ReviewPatch) -> String {
    format!(
        "{}\0{}\0{}",
        patch.root.display(),
        patch.head,
        patch.comparison_base_oid
    )
}

impl DiffReviewState {
    pub(super) fn comment_identity(&self) -> String {
        review_identity(&self.patch)
    }

    pub fn comments(&self) -> &[DiffComment] {
        &self.comments
    }

    fn comment_target(&self, row: usize, col: usize) -> Option<(String, usize, String)> {
        if let Some(target) = self.custom_target_at(row, col) {
            return Some((target.0, target.1, target.2.into()));
        }
        let cell = self.rows.get(row)?.cell_at(col)?;
        let info = cell.info;
        let file = self.patch.files.get(info.file?)?;
        if let Some(line) = info.old_line.filter(|_| {
            info.new_line.is_none()
                || self.rows[row].right.is_some()
                    && self.rows[row]
                        .left
                        .is_some_and(|left| left.text_col == cell.text_col)
        }) {
            Some((
                file.old_path.clone().unwrap_or_else(|| file.path.clone()),
                line,
                "old".into(),
            ))
        } else {
            Some((
                file.path.clone(),
                info.new_line.or(info.old_line)?,
                "new".into(),
            ))
        }
    }
}

impl Editor {
    pub(super) fn archive_diff_comments(&mut self) {
        if let Some(state) = self.ui_panels.diff_review.as_mut() {
            let comments = std::mem::take(&mut state.comments);
            self.ui_panels
                .closed_diff_comments
                .insert(state.comment_identity(), (state.next_comment_id, comments));
        }
    }
    pub(super) fn restore_diff_comments(&mut self) {
        if let Some(state) = self.ui_panels.diff_review.as_mut() {
            if let Some((next, comments)) = self
                .ui_panels
                .closed_diff_comments
                .remove(&state.comment_identity())
            {
                state.next_comment_id = next;
                state.comments = comments;
            }
        }
    }
    fn sync_diff_comments_attachment(&mut self) {
        if self.pending_diff_comments().is_some_and(|a| {
            self.diff_review()
                .is_some_and(|s| a.review_key == s.comment_identity())
        }) {
            if let Some(chat) = self.ai_state.chat.as_mut() {
                chat.pending_diff_comments = None;
            }
            self.attach_diff_comments_to_chat();
        }
    }

    pub fn is_editing_diff_comment(&self) -> bool {
        self.is_diff_review_buffer()
            && self
                .diff_review()
                .is_some_and(|s| s.comment_draft.is_some())
    }

    pub fn update_diff_comment(&mut self, id: u64, text: &str) -> Result<(), String> {
        if !self.is_diff_review_buffer() {
            return Err("No active diff review".into());
        }
        let text = comment_text(text)?;
        let comment = self
            .ui_panels
            .diff_review
            .as_mut()
            .and_then(|s| s.comments.iter_mut().find(|c| c.id == id))
            .ok_or("Comment no longer exists")?;
        comment.text = text;
        self.rerender_diff_review(None);
        self.sync_diff_comments_attachment();
        Ok(())
    }

    pub fn begin_diff_comment(&mut self, before: bool) {
        if !self.is_diff_review_buffer() {
            return;
        }
        let cursor = self.buffer().cursor();
        let row = cursor.line();
        let col = cursor.col().0;
        let state = self.diff_review().unwrap();
        if let Some(id) = state.comment_rows.get(row).copied().flatten() {
            self.edit_diff_comment(id);
            return;
        }
        let Some((path, line, side)) = state.comment_target(row, col) else {
            self.set_status_message("Move to a source line to add a comment · o below · O above");
            return;
        };
        let Some(excerpt) = super::snapshot_saved_line(&state.context_snapshot, &path, line, &side)
            .map(str::to_string)
        else {
            self.set_status_message(
                "Captured source is unavailable; refresh the diff before commenting",
            );
            return;
        };
        let state = self.ui_panels.diff_review.as_mut().unwrap();
        let id = state.next_comment_id;
        state.next_comment_id += 1;
        state.comments.push(DiffComment {
            id,
            path,
            line,
            side,
            before,
            text: String::new(),
            excerpt,
        });
        state.comment_draft = Some(CommentDraft {
            id,
            original: None,
            text: String::new(),
            cursor: 0,
        });
        self.set_mode(Mode::Insert);
        self.render_comment_draft();
        self.set_status_message(
            "Comment · Esc saves · Ctrl-C cancels · Enter adds a line · D deletes a saved comment",
        );
    }

    pub fn edit_diff_comment(&mut self, id: u64) {
        let Some(state) = self.ui_panels.diff_review.as_mut() else {
            return;
        };
        let Some(comment) = state.comments.iter().find(|comment| comment.id == id) else {
            return;
        };
        state.comment_draft = Some(CommentDraft {
            id,
            original: Some(comment.text.clone()),
            text: comment.text.clone(),
            cursor: comment.text.len(),
        });
        self.set_mode(Mode::Insert);
        self.render_comment_draft();
    }

    fn render_comment_draft(&mut self) {
        let scroll = self.scroll_offset();
        self.rerender_diff_review(None);
        self.viewport.scroll_offset = scroll;
        if let Some(window) = self
            .window_manager
            .as_mut()
            .and_then(|wm| wm.focused_window_mut())
        {
            window.set_scroll_offset(scroll);
        }
        let Some(state) = self.diff_review() else {
            return;
        };
        let Some(draft) = &state.comment_draft else {
            return;
        };
        let Some(row) = state
            .comment_rows
            .iter()
            .position(|id| *id == Some(draft.id))
        else {
            return;
        };
        let before = &draft.text[..draft.cursor];
        let row = row + 1 + before.bytes().filter(|b| *b == b'\n').count();
        let col =
            4 + crate::unicode::grapheme_count(before.rsplit('\n').next().unwrap_or_default());
        self.buffer_mut()
            .cursor_mut()
            .set_position(row, GraphemeCol(col));
        self.update_scroll_offset();
    }

    pub fn handle_diff_comment_key(&mut self, event: KeyEvent) -> bool {
        if self.mode() != Mode::Insert
            || !self.is_diff_review_buffer()
            || self.diff_review().is_none_or(|s| s.comment_draft.is_none())
        {
            return false;
        }
        if event.code == KeyCode::Esc
            || event.code == KeyCode::Char('c') && event.modifiers.contains(Modifiers::CONTROL)
        {
            let cancel = event.code == KeyCode::Char('c');
            let state = self.ui_panels.diff_review.as_mut().unwrap();
            let draft = state.comment_draft.take().unwrap();

            if cancel {
                if let Some(original) = draft.original {
                    if let Some(c) = state.comments.iter_mut().find(|c| c.id == draft.id) {
                        c.text = original;
                    }
                } else {
                    state.comments.retain(|c| c.id != draft.id);
                }
            } else {
                state.comments.retain(|c| !c.text.trim().is_empty());
            }
            self.set_mode(Mode::Normal);
            self.rerender_diff_review(None);
            self.sync_diff_comments_attachment();
            return true;
        }
        let state = self.ui_panels.diff_review.as_mut().unwrap();
        let draft = state.comment_draft.as_mut().unwrap();
        use unicode_segmentation::UnicodeSegmentation;
        let previous = draft.text[..draft.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i);
        let next = draft.cursor
            + draft.text[draft.cursor..]
                .graphemes(true)
                .next()
                .map_or(0, str::len);
        match event.code {
            KeyCode::Char('u') if event.modifiers.contains(Modifiers::CONTROL) => {
                let start = draft.text[..draft.cursor].rfind('\n').map_or(0, |i| i + 1);
                draft.text.drain(start..draft.cursor);
                draft.cursor = start;
            }
            KeyCode::Char('w') if event.modifiers.contains(Modifiers::CONTROL) => {
                let start = crate::editor::ai_chat_input::previous_chat_input_word_boundary(
                    &draft.text,
                    draft.cursor,
                );
                draft.text.drain(start..draft.cursor);
                draft.cursor = start;
            }
            KeyCode::Enter if draft.text.len() < MAX_COMMENT_BYTES => {
                draft.text.insert(draft.cursor, '\n');
                draft.cursor += 1;
            }
            KeyCode::Char(c)
                if !event
                    .modifiers
                    .intersects(Modifiers::CONTROL | Modifiers::ALT)
                    && draft.text.len() + c.len_utf8() <= MAX_COMMENT_BYTES =>
            {
                draft.text.insert(draft.cursor, c);
                draft.cursor += c.len_utf8();
            }
            KeyCode::Backspace => {
                draft.text.drain(previous..draft.cursor);
                draft.cursor = previous;
            }
            KeyCode::Delete => {
                draft.text.drain(draft.cursor..next);
            }
            KeyCode::Left => {
                draft.cursor = previous;
            }
            KeyCode::Right => {
                draft.cursor = next;
            }
            KeyCode::Up | KeyCode::Down => {
                let rows = crate::editor::ai_chat_input::wrap_chat_input_rows(
                    &draft.text,
                    usize::MAX / 2,
                    4,
                );
                if let Some(cursor) = crate::editor::ai_chat_input::move_chat_input_cursor_vertical(
                    &draft.text,
                    draft.cursor,
                    &rows,
                    if event.code == KeyCode::Up { -1 } else { 1 },
                    4,
                ) {
                    draft.cursor = cursor;
                }
            }
            KeyCode::Home => {
                draft.cursor = draft.text[..draft.cursor].rfind('\n').map_or(0, |i| i + 1);
            }
            KeyCode::End => {
                draft.cursor += draft.text[draft.cursor..]
                    .find('\n')
                    .unwrap_or(draft.text.len() - draft.cursor);
            }
            _ => {}
        }
        if let Some(comment) = state.comments.iter_mut().find(|c| c.id == draft.id) {
            comment.text = draft.text.clone();
        }
        self.render_comment_draft();
        true
    }

    pub fn paste_diff_comment(&mut self, text: &str) -> bool {
        if self.mode() != Mode::Insert
            || !self.is_diff_review_buffer()
            || self.diff_review().is_none_or(|s| s.comment_draft.is_none())
        {
            return false;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let state = self.ui_panels.diff_review.as_mut().unwrap();
        let draft = state.comment_draft.as_mut().unwrap();
        if draft.text.len() + text.len() > MAX_COMMENT_BYTES {
            self.set_status_message("Comment is too long (maximum 32768 bytes)");
            return true;
        }
        draft.text.insert_str(draft.cursor, &text);
        draft.cursor += text.len();
        if let Some(comment) = state.comments.iter_mut().find(|c| c.id == draft.id) {
            comment.text = draft.text.clone();
        }
        self.render_comment_draft();
        true
    }

    pub fn delete_diff_comment_at_cursor(&mut self) -> bool {
        let id = self.diff_review().and_then(|s| {
            s.comment_rows
                .get(self.buffer().cursor().line())
                .copied()
                .flatten()
        });
        if let Some(id) = id {
            self.remove_diff_comment(id);
            true
        } else {
            false
        }
    }

    pub fn remove_diff_comment(&mut self, id: u64) {
        let editing = self.is_editing_diff_comment();
        if let Some(state) = self.ui_panels.diff_review.as_mut() {
            state.comments.retain(|c| c.id != id);
            if state
                .comment_draft
                .as_ref()
                .is_some_and(|draft| draft.id == id)
            {
                state.comment_draft = None;
            }
        }
        if editing && !self.is_editing_diff_comment() {
            self.set_mode(Mode::Normal);
        }
        self.rerender_diff_review(None);
        self.sync_diff_comments_attachment();
    }

    pub fn clear_diff_comments(&mut self) {
        let editing = self.is_editing_diff_comment();
        let key = if self.is_diff_review_buffer() {
            self.diff_review().map(DiffReviewState::comment_identity)
        } else {
            self.pending_diff_comments().map(|a| a.review_key.clone())
        };
        let Some(key) = key else {
            return;
        };
        if let Some(state) = self
            .ui_panels
            .diff_review
            .as_mut()
            .filter(|s| s.comment_identity() == key)
        {
            state.comments.clear();
            state.comment_draft = None;
        }
        if let Some((_, comments)) = self.ui_panels.closed_diff_comments.get_mut(&key) {
            comments.clear();
        }
        if let Some(chat) = self.ai_state.chat.as_mut() {
            if chat
                .pending_diff_comments
                .as_ref()
                .is_some_and(|a| a.review_key == key)
            {
                chat.pending_diff_comments = None;
            }
        }
        self.rerender_diff_review(None);
        if editing {
            self.set_mode(Mode::Normal);
        }
        self.set_status_message("Diff comments cleared");
    }

    pub fn add_diff_comment(
        &mut self,
        path: &str,
        line: usize,
        side: &str,
        before: bool,
        text: &str,
    ) -> Result<(), String> {
        if !self.is_diff_review_buffer() {
            return Err("No active diff review".into());
        }
        let text = comment_text(text)?;
        let state = self.diff_review().unwrap();
        let _found = (0..state.rows.len())
            .find_map(|row| {
                [0, usize::MAX].into_iter().find_map(|col| {
                    (state.comment_target(row, col) == Some((path.into(), line, side.into())))
                        .then_some(row)
                })
            })
            .ok_or("The source line is no longer in this diff; select it again")?;
        let excerpt = super::snapshot_saved_line(&state.context_snapshot, path, line, side)
            .ok_or("Captured source is unavailable; refresh the diff before commenting")?
            .to_string();
        let state = self.ui_panels.diff_review.as_mut().unwrap();
        let id = state.next_comment_id;
        state.next_comment_id += 1;
        state.comments.push(DiffComment {
            id,
            path: path.into(),
            line,
            side: side.into(),
            before,
            text,
            excerpt,
        });
        self.rerender_diff_review(None);
        self.sync_diff_comments_attachment();
        Ok(())
    }

    pub fn attach_diff_comments_to_chat(&mut self) {
        if !self.is_diff_review_buffer() {
            return;
        }
        let Some(state) = self.diff_review() else {
            return;
        };
        let comments: Vec<_> = state
            .comments
            .iter()
            .filter(|c| {
                !c.text.trim().is_empty()
                    && !self.ai_state.chat.as_ref().is_some_and(|chat| {
                        chat.queued_inputs
                            .iter()
                            .filter_map(|q| q.diff_comments.as_ref())
                            .any(|a| {
                                a.review_key == state.comment_identity() && a.comments.contains(c)
                            })
                    })
            })
            .cloned()
            .collect();
        if comments.is_empty() {
            return;
        }
        let attachment = DiffCommentsAttachment {
            review_key: state.comment_identity(),
            comparison: format!(
                "{} · repository {} · base {}",
                state.text_lines.first().cloned().unwrap_or_default(),
                state.patch.root.display(),
                state.patch.comparison_base_oid
            ),
            comments,
        };
        if let Some(chat) = self.ai_state.chat.as_mut() {
            chat.pending_diff_comments = Some(attachment);
        }
    }

    pub fn pending_diff_comments(&self) -> Option<&DiffCommentsAttachment> {
        self.ai_state.chat.as_ref()?.pending_diff_comments.as_ref()
    }
    pub fn remove_diff_comments_attachment(&mut self) -> bool {
        self.ai_state
            .chat
            .as_mut()
            .and_then(|c| c.pending_diff_comments.take())
            .is_some()
    }

    pub(crate) fn consume_diff_comments(&mut self, attachment: &DiffCommentsAttachment) {
        if let Some((_, comments)) = self
            .ui_panels
            .closed_diff_comments
            .get_mut(&attachment.review_key)
        {
            comments.retain(|c| !attachment.comments.contains(c));
        }
        if let Some(state) = self
            .ui_panels
            .diff_review
            .as_mut()
            .filter(|s| s.comment_identity() == attachment.review_key)
        {
            let editing_id = state.comment_draft.as_ref().map(|draft| draft.id);
            state
                .comments
                .retain(|c| Some(c.id) == editing_id || !attachment.comments.contains(c));
            self.rerender_diff_review(None);
        }
    }
}

/// Insert annotation rows without changing patch indices or source mappings.
pub(super) fn insert_comments(state: &mut DiffReviewState, rendered: &mut Rendered) {
    state.comment_rows.clear();
    if state.comments.is_empty() {
        return;
    }
    let mut lines: Vec<String> = rendered.text.lines().map(str::to_string).collect();
    state.comment_rows = vec![None; lines.len()];
    // Resolve against the fresh render, not the old width-dependent row numbers.
    let mut additions = Vec::new();
    for comment in &state.comments {
        let mut found = None;
        for (index, row) in rendered.rows.iter().enumerate() {
            let custom = rendered.custom_targets.get(index).and_then(Option::as_ref);
            let target = custom.and_then(|t| {
                if comment.side == "old" {
                    t.old.as_ref()
                } else {
                    t.new.as_ref()
                }
            });
            let matches = target
                .is_some_and(|(path, line)| path == &comment.path && *line == comment.line)
                || [row.left, row.right].into_iter().flatten().any(|cell| {
                    let info = cell.info;
                    state
                        .patch
                        .files
                        .get(info.file.unwrap_or(usize::MAX))
                        .is_some_and(|file| {
                            let (path, line) = if comment.side == "old" {
                                (file.old_path.as_ref().unwrap_or(&file.path), info.old_line)
                            } else {
                                (&file.path, info.new_line)
                            };
                            path == &comment.path && line == Some(comment.line)
                        })
                });
            if matches
                && super::snapshot_saved_line(
                    &state.context_snapshot,
                    &comment.path,
                    comment.line,
                    &comment.side,
                ) == Some(comment.excerpt.as_str())
            {
                found = Some(index + usize::from(!comment.before));
                if comment.before {
                    break;
                }
            }
        }
        // Hidden or refreshed source still keeps the comment visible at the end.
        let at = found.unwrap_or(lines.len());
        additions.push((at, comment, found.is_none()));
    }
    additions.sort_by_key(|(at, c, _)| (*at, c.id));
    for (at, comment, orphan) in additions.into_iter().rev() {
        let text = if let Some(draft) = state.comment_draft.as_ref().filter(|d| d.id == comment.id)
        {
            &draft.text
        } else {
            &comment.text
        };
        let mut rows: Vec<_> = text.split('\n').map(|line| format!("  # {line}")).collect();
        rows.insert(
            0,
            format!(
                "  Comment · {}:{} · {} side{} · o edit · D delete",
                comment.path,
                comment.line,
                comment.side,
                if orphan {
                    " · captured source changed or hidden"
                } else {
                    ""
                }
            ),
        );
        let count = rows.len();
        lines.splice(at..at, rows.iter().cloned());
        rendered
            .rows
            .splice(at..at, vec![Default::default(); count]);
        rendered.highlights.splice(
            at..at,
            rows.iter()
                .map(|line| vec![(0..line.len(), crate::syntax::HighlightGroup::Comment)]),
        );
        rendered.custom_targets.splice(at..at, vec![None; count]);
        rendered.item_rows.splice(at..at, vec![None; count]);
        state.context_rows.splice(at..at, vec![None; count]);
        state
            .comment_rows
            .splice(at..at, vec![Some(comment.id); count]);
        let shift = |line: &mut usize| {
            if *line != usize::MAX && *line >= at {
                *line += count;
            }
        };
        rendered.hunk_lines.iter_mut().for_each(shift);
        rendered.file_lines.iter_mut().for_each(shift);
        for (line, _) in &mut rendered.stat_rows {
            shift(line);
        }
        shift(&mut rendered.toolbar.line);
    }
    rendered.text = lines.join("\n") + "\n";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::ChatOpts;

    fn fixture() -> (tempfile::TempDir, Editor) {
        let directory = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(directory.path()).unwrap();
        let path = directory.path().join("source.rs");
        std::fs::write(&path, "let value = 1;\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("source.rs")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = git2::Signature::now("Test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
        std::fs::write(&path, "let value = 42;\n").unwrap();
        let mut editor = Editor::default();
        editor.open_file(&path).unwrap();
        editor.open_diff_review(Some("HEAD")).unwrap();
        editor
            .add_diff_comment("source.rs", 1, "new", false, "Why 42?\nIs zero valid?")
            .unwrap();
        editor
            .open_ai_chat(ChatOpts {
                profile: Some(crate::ai::PROFILE_LOCAL.into()),
                ..Default::default()
            })
            .unwrap();
        (directory, editor)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn comments_reach_the_provider_with_the_question_and_clear_after_submission() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_directory, mut editor) = fixture();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        editor
            .ai_state
            .config
            .profiles
            .get_mut(crate::ai::PROFILE_LOCAL)
            .unwrap()
            .base_url = Some(format!("http://{}", listener.local_addr().unwrap()));
        let request = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let body = loop {
                let mut chunk = [0; 4096];
                let size = socket.read(&mut chunk).await.unwrap();
                assert!(size > 0);
                bytes.extend_from_slice(&chunk[..size]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|h| {
                            h.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break bytes[end + 4..end + 4 + length].to_vec();
                    }
                }
            };
            let reply = "{\"message\":{\"role\":\"assistant\",\"content\":\"Captured comments received.\"},\"done\":true}\n";
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes()).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()
        });
        editor.ai_state.chat.as_mut().unwrap().input = "Explain these changes".into();
        editor.submit_ai_chat_message().unwrap();
        assert!(editor.pending_diff_comments().is_none());
        assert!(editor.diff_review().unwrap().comments().is_empty());
        let sent = tokio::time::timeout(std::time::Duration::from_secs(10), request)
            .await
            .unwrap()
            .unwrap();
        let message = sent["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "user")
            .unwrap()["content"]
            .as_str()
            .unwrap();
        assert!(message.contains("source.rs:1 (new side"), "{message}");
        assert!(message.contains("Source: let value = 42;"));
        assert!(message.contains("Why 42?\nIs zero valid?"));
        assert!(message.ends_with("Explain these changes"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn failed_turn_start_keeps_comments_and_attachment() {
        let (_directory, mut editor) = fixture();
        let _turn = editor.begin_ai_runtime_turn("already active").unwrap();
        editor.submit_ai_chat_message().unwrap();
        assert!(editor.pending_diff_comments().is_some());
        assert_eq!(editor.diff_review().unwrap().comments().len(), 1);
        assert!(editor.status_message().contains("Unable to start AI turn"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn missing_provider_keeps_comments_and_attachment() {
        let (_directory, mut editor) = fixture();
        editor.ai_state.config.profiles.clear();
        editor.submit_ai_chat_message().unwrap();
        assert!(editor.pending_diff_comments().is_some());
        assert_eq!(editor.diff_review().unwrap().comments().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn queued_comments_are_not_reattached_and_failed_queue_start_preserves_both_drafts() {
        let (_directory, mut editor) = fixture();
        editor.queue_current_ai_chat_input(
            crate::editor::ai_chat_state::QueuedChatInputKind::FollowUp,
        );
        assert!(editor.pending_diff_comments().is_none());
        editor.attach_diff_comments_to_chat();
        assert!(editor.pending_diff_comments().is_none());
        let _turn = editor.begin_ai_runtime_turn("already active").unwrap();
        editor.ai_state.chat.as_mut().unwrap().input = "draft for later".into();
        assert!(!editor.start_next_queued_ai_chat_input().unwrap());
        assert_eq!(editor.ai_chat_input(), "draft for later");
        assert_eq!(editor.ai_chat_queued_inputs().count(), 1);
        assert_eq!(editor.diff_review().unwrap().comments().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn sending_a_snapshot_does_not_clear_later_edits_and_closed_comments_survive() {
        let (_directory, mut editor) = fixture();
        let sent = editor.pending_diff_comments().unwrap().clone();
        editor.close_ai_chat();
        editor.edit_diff_comment(sent.comments[0].id);
        editor.consume_diff_comments(&sent);
        assert_eq!(editor.diff_review().unwrap().comments().len(), 1);
        assert!(editor.is_editing_diff_comment());
        editor.handle_diff_comment_key(KeyEvent::new(KeyCode::Esc, Modifiers::empty()));
        editor
            .update_diff_comment(sent.comments[0].id, "Updated question")
            .unwrap();
        editor.consume_diff_comments(&sent);
        assert_eq!(
            editor.diff_review().unwrap().comments()[0].text,
            "Updated question"
        );
        editor.close_diff_review();
        editor.open_diff_review(Some("HEAD")).unwrap();
        assert_eq!(
            editor.diff_review().unwrap().comments()[0].text,
            "Updated question"
        );
        editor
            .open_ai_chat(ChatOpts {
                profile: Some(crate::ai::PROFILE_LOCAL.into()),
                ..Default::default()
            })
            .unwrap();
        let sent = editor.pending_diff_comments().unwrap().clone();
        editor.close_ai_chat();
        editor.close_diff_review();
        editor.consume_diff_comments(&sent);
        editor.open_diff_review(Some("HEAD")).unwrap();
        assert!(editor.diff_review().unwrap().comments().is_empty());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn refreshing_changed_source_keeps_the_captured_comment_separate() {
        let (directory, mut editor) = fixture();
        editor.close_ai_chat();
        std::fs::write(directory.path().join("source.rs"), "let value = 99;\n").unwrap();
        editor.refresh_diff_review();
        let comment = &editor.diff_review().unwrap().comments()[0];
        assert_eq!(comment.excerpt, "let value = 42;");
        let rendered = editor.buffer().rope().to_string();
        assert!(rendered.contains("captured source changed or hidden"));
        assert!(
            rendered
                .find("let value = 99;")
                .unwrap_or_else(|| panic!("{rendered}"))
                < rendered.find("Comment · source.rs:1").unwrap()
        );
        editor
            .open_ai_chat(ChatOpts {
                profile: Some(crate::ai::PROFILE_LOCAL.into()),
                ..Default::default()
            })
            .unwrap();
        assert!(editor
            .pending_diff_comments()
            .unwrap()
            .message()
            .contains("Source: let value = 42;"));
    }
}
