//! Accepting a completion item.
//!
//! One accept is one undo step: the main edit (`textEdit` / `insertText` /
//! label, snippet-expanded), the item's `additionalTextEdits` (auto-imports
//! and friends) and the cursor placement are planned in *pre-edit* char
//! offsets and applied bottom-to-top, so nothing shifts under an edit that
//! has not been applied yet.

use super::completion::CompletionAnchor;
use super::snippet_session::SnippetSession;
use super::{CursorPos, Editor, InsertEntryMode};
use crate::snippet::{standard_variable, Snippet};
use crate::unicode::{CharCol, GraphemeCol};
use lsp_types::{CompletionItem, CompletionTextEdit, InsertTextFormat, InsertTextMode};

/// How the accept key treats the identifier text right of the cursor when the
/// server offers separate insert / replace ranges (IntelliJ: Enter inserts,
/// Tab replaces).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionAcceptMode {
    Insert,
    Replace,
}

/// A planned edit in pre-edit char offsets.
#[derive(Debug, Clone)]
pub(super) struct PlannedEdit {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) text: String,
}

fn utf16_to_char_in(text: &str, utf16_col: u32) -> usize {
    let mut units = 0u32;
    let mut chars = 0usize;
    for ch in text.chars() {
        if units >= utf16_col || ch == '\n' || ch == '\r' {
            break;
        }
        units += ch.len_utf16() as u32;
        chars += 1;
    }
    chars
}

impl Editor {
    /// Accepts the currently selected completion from the menu (insert range).
    pub fn accept_completion(&mut self) {
        self.accept_completion_with(CompletionAcceptMode::Insert);
    }

    /// Accepts the selected completion; `mode` picks the insert or the
    /// replace range of `InsertReplaceEdit` items.
    pub fn accept_completion_with(&mut self, mode: CompletionAcceptMode) {
        if self.completion_menu.is_snippet_choices() {
            self.accept_snippet_choice();
        } else if let Some(item) = self.completion_menu.selected_item().cloned() {
            if self.completion_context_is_current() {
                self.accept_completion_item(&item, mode);
            }
        }
        self.dismiss_completion();
        // The snippet just expanded may start on a choice stop.
        self.snippet_show_choices();
    }

    /// Whether the cursor is still where the menu was built: on the same line,
    /// right after the prefix it filters by. After a motion or an edit the menu
    /// did not see (a mouse click, a command run from insert mode) accepting
    /// would rewrite text the user never meant to complete.
    fn completion_context_is_current(&self) -> bool {
        let on_anchor_line = self
            .completion_menu
            .anchor()
            .is_none_or(|anchor| anchor.line == self.buffer().cursor().line());
        on_anchor_line
            && self.completion_prefix_from_trigger_col() == self.completion_menu.trigger_prefix()
    }

    /// Accepts a completion by index from available_completions (used by picker)
    pub fn accept_completion_at(&mut self, index: usize) {
        if let Some(item) = self.lsp.state.available_completions.get(index).cloned() {
            self.accept_completion_item(&item, CompletionAcceptMode::Insert);
        }
        self.lsp.state.available_completions.clear();
        self.dismiss_completion();
        self.snippet_show_choices();
    }

    /// Commit characters: typing one of the selected item's `commitCharacters`
    /// accepts the item first (the character is then inserted by the caller).
    /// Only when the user chose the item (moved the selection) or already
    /// typed it out completely, so ordinary typing is never hijacked.
    pub(crate) fn try_commit_completion(&mut self, typed: char) -> bool {
        let Some(item) = self.completion_menu.selected_item() else {
            return false;
        };
        let commits = item
            .commit_characters
            .as_ref()
            .is_some_and(|chars| chars.iter().any(|c| c.starts_with(typed)));
        if !commits {
            return false;
        }
        let typed_in_full = {
            let text = item.filter_text.as_deref().unwrap_or(&item.label);
            text == self.completion_menu.trigger_prefix()
        };
        if !(self.completion_menu.navigated() || typed_in_full) {
            return false;
        }
        self.accept_completion_with(CompletionAcceptMode::Insert);
        true
    }

    /// Maps an LSP position from the request's document snapshot to a
    /// pre-edit char offset in the current buffer. `range_end` tells whether
    /// the position ends its range: text typed at the request position lies
    /// inside a range that ends there, but in front of one that starts there.
    fn completion_offset(
        &self,
        position: lsp_types::Position,
        rebase: Option<(&CompletionAnchor, usize)>,
        range_end: bool,
    ) -> usize {
        let rope = self.buffer().rope();
        let line = (position.line as usize).min(rope.len_lines().saturating_sub(1));
        let mut col = match rebase {
            Some((anchor, _)) if anchor.line == line => {
                utf16_to_char_in(&anchor.line_text, position.character)
            }
            _ => self.utf16_to_col(line, position.character).0,
        };
        if let Some((anchor, shift)) = rebase {
            if anchor.line == line && (col > anchor.col || (col == anchor.col && range_end)) {
                col += shift;
            }
        }
        let line_start = rope.line_to_char(line);
        let content_len = self.buffer().line_content_len(line);
        line_start + col.min(content_len)
    }

    /// Core completion application.
    fn accept_completion_item(&mut self, item: &CompletionItem, mode: CompletionAcceptMode) {
        use crate::editor::completion::CompletionAnchor as Anchor;

        // The item's textEdit ranges index the buffer snapshot the server
        // answered for. If the user typed on since (identifier characters at
        // the request position), shift them; if the buffer changed in any
        // other way they must not be applied verbatim (OV-00327).
        let exact = self
            .completion_menu
            .items_buffer_version()
            .is_some_and(|v| v == self.buffer().version());
        let anchor: Option<Anchor> = self.completion_menu.anchor().cloned();
        let shift: Option<usize> = if exact {
            Some(0)
        } else {
            anchor
                .as_ref()
                .and_then(|anchor| self.completion_typed_since(anchor))
        };
        let rebase = anchor.as_ref().zip(shift);

        let cursor_char = {
            let rope = self.buffer().rope();
            rope.line_to_char(self.buffer().cursor().line()) + self.buffer().cursor_char_col().0
        };

        // ---- main edit ----
        let text_edit = item.text_edit.as_ref().filter(|_| shift.is_some());
        let (main_start, main_end, main_text) = match text_edit {
            Some(edit) => {
                let (range, new_text) = match edit {
                    CompletionTextEdit::Edit(edit) => (edit.range, edit.new_text.clone()),
                    CompletionTextEdit::InsertAndReplace(ir) => {
                        let range = match mode {
                            CompletionAcceptMode::Insert => ir.insert,
                            CompletionAcceptMode::Replace => ir.replace,
                        };
                        (range, ir.new_text.clone())
                    }
                };
                let start = self.completion_offset(range.start, rebase, false);
                let end = self.completion_offset(range.end, rebase, true);
                (start.min(end), start.max(end), new_text)
            }
            None => {
                // Delete trigger..cursor and insert the item's text.
                let cursor_line = self.buffer().cursor().line();
                let line_start = self.buffer().rope().line_to_char(cursor_line);
                let start = (line_start + self.completion_menu.trigger_col()).min(cursor_char);
                let text = item
                    .text_edit
                    .as_ref()
                    .map(|edit| match edit {
                        CompletionTextEdit::Edit(edit) => edit.new_text.clone(),
                        CompletionTextEdit::InsertAndReplace(ir) => ir.new_text.clone(),
                    })
                    .or_else(|| item.insert_text.clone())
                    .unwrap_or_else(|| item.label.clone());
                (start, cursor_char, text)
            }
        };

        // Normalize CR variants in the LSP-supplied insertion text so the
        // rope stays LF-only (OV-00251).
        let main_text = if main_text.contains('\r') {
            crate::buffer::normalize_for_buffer(&main_text).into_owned()
        } else {
            main_text
        };

        // ---- snippet expansion ----
        let is_snippet = item.insert_text_format == Some(InsertTextFormat::SNIPPET);
        let snippet: Snippet = if is_snippet {
            let line_idx = self.buffer().rope().char_to_line(main_start);
            let line_text = self
                .buffer()
                .line_text(line_idx)
                .unwrap_or_default()
                .to_string();
            let word = self.completion_menu.trigger_prefix().to_string();
            let file = self.buffer().file_path().map(|s| s.to_string());
            let mut snippet = Snippet::parse(&main_text, &|name| {
                standard_variable(name, file.as_deref(), &line_text, line_idx, &word)
            });
            if item.insert_text_mode != Some(InsertTextMode::AS_IS) {
                let line_start = self.buffer().rope().line_to_char(line_idx);
                let indent: String = self
                    .buffer()
                    .rope()
                    .slice(line_start..main_start.max(line_start))
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                snippet.indent_continuation_lines(&indent);
            }
            snippet
        } else {
            Snippet::literal(&main_text)
        };

        // ---- additional edits (auto-imports ...) ----
        let mut extra: Vec<PlannedEdit> = Vec::new();
        if shift.is_some() {
            for edit in item.additional_text_edits.iter().flatten() {
                // An insertion at the request position stays in front of the
                // text typed since, so it never swallows that text.
                let insertion = edit.range.start == edit.range.end;
                let start = self.completion_offset(edit.range.start, rebase, false);
                let end = self.completion_offset(edit.range.end, rebase, !insertion);
                let (start, end) = (start.min(end), start.max(end));
                // An edit overlapping the main edit could not be applied
                // consistently; the main edit wins.
                if start < main_end && end > main_start {
                    continue;
                }
                extra.push(PlannedEdit {
                    start,
                    end,
                    text: crate::buffer::normalize_for_buffer(&edit.new_text).into_owned(),
                });
            }
        }

        // ---- apply ----
        let mut edits: Vec<PlannedEdit> = extra.clone();
        edits.push(PlannedEdit {
            start: main_start,
            end: main_end,
            text: snippet.text.clone(),
        });

        // Where the main text lands once every earlier edit has been applied.
        let shift_before_main: isize = extra
            .iter()
            .filter(|edit| edit.start <= main_start)
            .map(|edit| edit.text.chars().count() as isize - (edit.end - edit.start) as isize)
            .sum();
        let base = (main_start as isize + shift_before_main) as usize;

        // Cursor: first interactive stop, else `$0` / end of the text.
        let session = SnippetSession::new(&snippet, base);
        self.apply_offset_edits_as_one_undo(edits, session.initial_cursor());

        // Tab stops: keep navigating after the insertion.
        self.editing.snippet = if snippet.has_interactive_stops() {
            let mut session = session;
            session.finish_start(self.buffer().rope().len_chars());
            Some(Box::new(session))
        } else {
            None
        };

        if let Some(command) = item.command.clone() {
            self.lsp.state.pending_completion_commands.push(command);
        }

        // A method completion leaves the cursor inside `name(|)`: show its
        // parameters right away.
        self.request_signature_help_if_in_call();
    }

    /// Applies `edits` (pre-edit char offsets, any order) as a single undo
    /// step and puts the cursor at `cursor` (a post-edit char offset). The
    /// insert-mode recording is closed first and restarted afterwards so the
    /// edit is its own `Recorded` entry (see the note in
    /// `accept_completion_item`).
    pub(super) fn apply_offset_edits_as_one_undo(
        &mut self,
        mut edits: Vec<PlannedEdit>,
        cursor: usize,
    ) {
        let cursor_before = CursorPos::new(
            self.buffer().cursor().line(),
            GraphemeCol(self.buffer().cursor().col().0),
        );

        // Break the insert-mode undo here, Vim-style: finalize whatever the
        // user typed before accepting completion as its own `Recorded` entry,
        // then restart the session after completion edits land. Pause/resume
        // would retain pre-completion edits whose absolute char offsets no
        // longer match the rope once the completion text is inserted, and
        // concatenating them with post-completion edits in a single Recorded
        // corrupts the buffer on undo (and again on redo).
        let restart_entry_mode: Option<InsertEntryMode> = if self.buffer().is_recording() {
            let entry_mode = self
                .buffer()
                .change_manager()
                .current_builder
                .as_ref()
                .map(|b| b.entry_mode().clone());
            self.finalize_change_building();
            entry_mode
        } else {
            None
        };

        // Bottom-to-top so earlier offsets stay valid.
        edits.sort_by(|a, b| b.start.cmp(&a.start).then(b.end.cmp(&a.end)));
        let ((), recorded) = self.buffer_mut().record(|buf| {
            for edit in &edits {
                if edit.end > edit.start {
                    buf.delete_char_range(edit.start, edit.end);
                }
                if !edit.text.is_empty() {
                    let line = buf.rope().char_to_line(edit.start);
                    let col = edit.start - buf.rope().line_to_char(line);
                    buf.insert_text_at(line, CharCol(col), &edit.text);
                }
            }
        });

        let (line, col) = {
            let rope = self.buffer().rope();
            let target = cursor.min(rope.len_chars());
            let line = rope.char_to_line(target);
            (line, target - rope.line_to_char(line))
        };
        self.buffer_mut().set_cursor_char_col(line, CharCol(col));
        if !recorded.is_empty() {
            let cursor_after = self.cursor_position();
            self.push_recorded_undo(recorded, cursor_before, cursor_after);
        }

        // Restart the insert-mode session so any further typing forms a fresh
        // Recorded entry at post-edit offsets.
        if let Some(entry_mode) = restart_entry_mode {
            let cursor_after = self.cursor_position();
            self.start_change_building(cursor_after);
            self.set_change_entry_mode(entry_mode);
        }
    }
}
