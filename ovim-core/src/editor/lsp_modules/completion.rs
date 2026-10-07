//! LSP completion functionality
//!
//! This module handles code completion requests and application.
//! Completions are typically triggered by Ctrl+N or automatically in insert mode.

use super::super::completion::CompletionAnchor;
use super::super::lsp_state::CompletionIntent;
use super::super::Editor;
use crate::lsp::{uri_from_file_path, CompletionTrigger};
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Status shown while a completion request is in flight.
pub(in crate::editor) const REQUESTING_STATUS: &str = "Requesting completions...";

impl Editor {
    /// Request completion at current cursor position
    pub fn request_completion(&mut self) {
        self.lsp.intents.completion_due = None;
        self.lsp.intents.completion = Some(CompletionIntent::Invoked);
    }

    /// Asks for completion for `intent`. Typing-driven requests wait for the
    /// configured `autocompletedelay` so a burst of keystrokes costs one
    /// request; trigger characters are answered immediately.
    fn schedule_completion(&mut self, intent: CompletionIntent) {
        let delay = self.options.autocomplete_delay_ms;
        if delay == 0 || matches!(intent, CompletionIntent::Typed(_)) {
            self.lsp.intents.completion_due = None;
            self.lsp.intents.completion = Some(intent);
        } else {
            self.lsp.intents.completion_due =
                Some((Instant::now() + Duration::from_millis(delay), intent));
        }
    }

    /// Moves a debounced request to the intent queue once it is due. Returns
    /// true when one became due.
    pub(crate) fn promote_due_completion(&mut self) -> bool {
        let Some((due, intent)) = self.lsp.intents.completion_due else {
            return false;
        };
        if Instant::now() < due {
            return false;
        }
        self.lsp.intents.completion_due = None;
        if self.mode() == crate::mode::Mode::Insert {
            self.lsp.intents.completion = Some(intent);
        }
        true
    }

    /// Insert-mode hook, run right after `c` was inserted: keeps an open menu
    /// filtered, or starts one (identifier typing / server trigger character).
    pub(crate) fn completion_after_typed_char(&mut self, c: char) {
        let ident = is_completion_keyword_char(c);
        if self.completion_menu.has_session() {
            if ident {
                let prefix = self.completion_prefix_from_trigger_col();
                self.completion_menu.filter(&prefix);
                if self.completion_menu.is_incomplete() {
                    self.schedule_completion(CompletionIntent::Incomplete);
                }
                return;
            }
            // A character that is not part of the word closes the menu; it
            // may still open a new one (`foo.` after `foo`).
            self.hide_completion_menu();
        }
        if !self.options.autocomplete {
            return;
        }
        if ident {
            // A request already on its way will be answered for the same word;
            // its answer is rebased over what was typed since (see
            // `completion_typed_since`), so asking again would only cancel it.
            if self.lsp.slots.completion.is_pending() {
                return;
            }
            if self.has_completion_trigger_prefix(self.options.autocomplete_min_chars) {
                self.schedule_completion(CompletionIntent::Identifier);
            }
        } else {
            self.schedule_completion(CompletionIntent::Typed(c));
        }
    }

    /// Insert-mode hook, run right after Backspace removed a character.
    pub(crate) fn completion_after_backspace(&mut self) {
        if !self.completion_menu.has_session() {
            return;
        }
        let cursor_col = self.buffer().cursor_char_col().0;
        if cursor_col < self.completion_menu.trigger_col() {
            // Deleted past where the word began: nothing to complete.
            self.hide_completion_menu();
            return;
        }
        let prefix = self.completion_prefix_from_trigger_col();
        self.completion_menu.filter(&prefix);
        if self.completion_menu.is_incomplete() {
            self.schedule_completion(CompletionIntent::Incomplete);
        }
    }

    /// Whether the response of a request made at `anchor` still describes the
    /// text under the cursor: nothing changed, or the user only typed
    /// identifier characters at the request position (returns the count).
    pub(crate) fn completion_typed_since(&self, anchor: &CompletionAnchor) -> Option<usize> {
        let cursor = self.buffer().cursor();
        if cursor.line() != anchor.line || self.buffer().line_count() != anchor.line_count {
            return None;
        }
        let now: Vec<char> = self
            .buffer()
            .line_text(anchor.line)
            .unwrap_or_default()
            .chars()
            .collect();
        let then: Vec<char> = anchor.line_text.chars().collect();
        let cursor_col = self.buffer().cursor_char_col().0;
        if cursor_col < anchor.col || now.len() < then.len() {
            return None;
        }
        let typed = now.len() - then.len();
        if cursor_col != anchor.col + typed
            || now[..anchor.col] != then[..anchor.col]
            || now[anchor.col + typed..] != then[anchor.col..]
            || !now[anchor.col..anchor.col + typed]
                .iter()
                .all(|c| is_completion_keyword_char(*c))
        {
            return None;
        }
        Some(typed)
    }

    pub(crate) fn completion_trigger_context(&self) -> (usize, String) {
        let cursor = self.buffer().cursor();
        let line_idx = cursor.line();
        let cursor_col = cursor.col();

        completion_trigger_context_from_index(&self.buffer().line_index(line_idx), cursor_col.0)
    }

    /// Trigger decisions need only two identifier scalars, not an allocated
    /// prefix extending back to the beginning of a potentially enormous line.
    pub(crate) fn has_completion_trigger_prefix(&self, min_chars: usize) -> bool {
        let cursor = self.buffer().cursor();
        let index = self.buffer().line_index(cursor.line());
        let mut col = cursor.col().0.min(index.grapheme_count());
        let mut chars = 0;
        while col > 0 && chars < min_chars {
            col -= 1;
            let Some(grapheme) = index.grapheme_at(crate::unicode::GraphemeCol(col)) else {
                break;
            };
            if !grapheme.chars().all(is_completion_keyword_char) {
                break;
            }
            chars += grapheme.chars().count();
        }
        chars >= min_chars
    }

    /// Derives the completion prefix from textEdit ranges when available.
    ///
    /// Uses the most common `textEdit.range.start.character` across items
    /// (majority vote) to determine where the completion token starts, then
    /// reads the text from that column to the cursor as the prefix.
    /// Falls back to word-boundary heuristic when no textEdit is present.
    pub(crate) fn derive_completion_prefix(
        &self,
        items: &[lsp_types::CompletionItem],
    ) -> (usize, String) {
        // Try to derive trigger_col from the textEdit ranges.
        // Use majority vote on range.start.character to handle multi-server
        // scenarios where different servers may have different ranges.
        let start_char = text_edit_majority_start(items);
        if let Some(utf16_start) = start_char {
            let line_idx = self.buffer().cursor().line();
            let cursor_col = self.buffer().cursor_char_col();

            // utf16_to_col returns char col — correct for delete_range
            let trigger_col = self.utf16_to_col(line_idx, utf16_start);

            // Sanity: trigger_col must be at or before cursor
            if trigger_col <= cursor_col {
                let prefix = self
                    .buffer()
                    .line_index(line_idx)
                    .slice_chars(trigger_col.0..cursor_col.0);
                return (trigger_col.0, prefix);
            }
        }

        // Fallback: word-boundary heuristic
        self.completion_trigger_context()
    }

    /// Returns the current prefix text from the stored trigger_col to cursor.
    /// Used for ongoing filtering while the completion menu is visible,
    /// so we don't re-derive the trigger column from word boundaries.
    pub(crate) fn completion_prefix_from_trigger_col(&self) -> String {
        let trigger_col = self.completion_menu().trigger_col(); // char col (bare usize)
        let cursor_col = self.buffer().cursor_char_col();

        if trigger_col > cursor_col.0 {
            return String::new();
        }

        self.buffer()
            .line_index(self.buffer().cursor().line())
            .slice_chars(trigger_col..cursor_col.0)
    }

    /// Implementation of completion request
    pub(in crate::editor) async fn completion_impl(
        &mut self,
        intent: CompletionIntent,
    ) -> Result<bool> {
        // Typing-driven requests are speculative: they must never leave a
        // status message behind when there is simply nothing to ask.
        let explicit = intent == CompletionIntent::Invoked;
        let lsp = match &self.lsp.state.lsp_manager {
            Some(lsp) => lsp.clone(),
            None => {
                if explicit {
                    self.set_lsp_status("LSP not available".to_string());
                }
                return Ok(false);
            }
        };

        let Some(file_path) = self.buffer().file_path().map(|s| s.to_string()) else {
            if explicit {
                self.set_lsp_status("Save file first to use completion".to_string());
            }
            return Ok(false);
        };

        let abs_path = if std::path::Path::new(&file_path).is_absolute() {
            file_path.clone()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(&file_path).to_string_lossy().to_string(),
                Err(_) => {
                    if explicit {
                        self.set_lsp_status("Failed to resolve file path".to_string());
                    }
                    return Ok(false);
                }
            }
        };

        let uri = uri_from_file_path(&abs_path).ok_or_else(|| anyhow!("Invalid file path"))?;

        let cursor_line = self.buffer().cursor().line();
        let line = cursor_line as u32;
        let col = self.buffer().cursor().col().0;
        let character = self.col_to_utf16(cursor_line, col);

        let language_id = match self.language_id_for_path(&file_path) {
            Some(id) => id,
            None => {
                if explicit {
                    self.set_lsp_status("Language not supported for LSP".to_string());
                }
                return Ok(false);
            }
        };

        // Resolve the server group responsible for this document.
        let server_ids = lsp.servers_for_document(&language_id, std::path::Path::new(&file_path));

        // No LSP servers registered yet — server may still be initializing.
        if server_ids.is_empty() {
            // Only set "waiting" status if there isn't already a more specific
            // error (e.g., "LSP: rust-analyzer not found in PATH").
            if explicit && !self.lsp_status().starts_with("LSP:") {
                self.set_lsp_status("LSP: waiting for server...".to_string());
            }
            return Err(anyhow!("No LSP servers ready for {}", language_id));
        }

        // A typed non-identifier character only asks when a server says it is
        // a trigger character (advertised, or the per-language fallback for
        // servers that have not reported capabilities yet).
        let trigger = match intent {
            CompletionIntent::Invoked | CompletionIntent::Identifier => CompletionTrigger::Invoked,
            CompletionIntent::Incomplete => CompletionTrigger::Incomplete,
            CompletionIntent::Typed(typed) => {
                let advertised: HashSet<char> = lsp
                    .completion_trigger_characters_for_servers(&server_ids)
                    .await
                    .into_iter()
                    .collect();
                let fallback = crate::lsp::fallback_completion_trigger_characters(&language_id);
                if advertised.contains(&typed)
                    || (fallback.contains(&typed) && self.typed_fallback_trigger_complete(typed))
                {
                    CompletionTrigger::Character(typed)
                } else {
                    return Ok(false);
                }
            }
        };

        // Sync document content to LSP on the main thread before spawning.
        // This avoids the spawned task racing with send_lsp_changes_if_modified()
        // over the debouncer — a source of timing bugs where stale content
        // was sent from the background task, overwriting newer content.
        self.ensure_lsp_document_synced().await;

        let buffer_version_usize = self.buffer().version();
        let anchor = CompletionAnchor {
            line: cursor_line,
            col: self.buffer().cursor_char_col().0,
            line_text: self
                .buffer()
                .line_text(cursor_line)
                .unwrap_or_default()
                .to_string(),
            line_count: self.buffer().line_count(),
        };

        // Spawn completion request in background (non-blocking).
        // Document sync already happened above via ensure_lsp_document_synced().
        // The task only makes the LSP request — no debouncer interaction.
        let (tx, rx) = tokio::sync::oneshot::channel();
        let language_id = language_id.to_string();
        let file_path_for_task = file_path.clone();
        let task = tokio::spawn(async move {
            let result = if server_ids.len() > 1 {
                lsp.completion_multi(&uri, line, character, &server_ids, trigger)
                    .await
            } else {
                lsp.completion(&uri, line, character, &language_id, trigger)
                    .await
            };
            let task_result = result.map(|outcome| crate::editor::lsp_slot::CompletionResult {
                items: outcome.items,
                is_incomplete: outcome.is_incomplete,
                anchor,
                file_path: file_path_for_task,
                buffer_version: buffer_version_usize,
                synced_content: None,
                synced_lsp_version: None,
                sources: outcome.sources,
            });

            let _ = tx.send(task_result);
        });

        self.lsp.slots.completion.fire(task, rx);

        if explicit {
            self.set_lsp_status(REQUESTING_STATUS.to_string());
        }
        Ok(true)
    }

    /// Asks the server to resolve the selected item's lazily-computed fields
    /// (documentation) once per item, when the server supports it.
    pub(in crate::editor) async fn request_completion_resolve(&mut self) {
        let Some(item) = self.completion_menu.selected_item() else {
            return;
        };
        if item.documentation.is_some() {
            return;
        }
        let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
            return;
        };
        let Some(file_path) = self.buffer().file_path().map(|s| s.to_string()) else {
            return;
        };
        let Some(language_id) = self.language_id_for_path(&file_path) else {
            return;
        };
        let mut server_ids =
            lsp.servers_for_document(&language_id, std::path::Path::new(&file_path));
        // The server that produced the item is the one that can resolve it.
        if let Some(origin) = self
            .lsp
            .state
            .completion_sources
            .get(&crate::editor::completion::item_key(item))
        {
            server_ids = vec![origin.clone()];
        }
        if !lsp.any_supports_completion_resolve(&server_ids).await {
            return;
        }
        let generation = self.completion_menu.generation();
        let Some((source_index, item)) = self.completion_menu.take_unresolved_selection() else {
            return;
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = lsp
                .resolve_completion_item(&server_ids, item)
                .await
                .map(|item| crate::editor::lsp_slot::CompletionResolveResult {
                    source_index,
                    menu_generation: generation,
                    item,
                });
            let _ = tx.send(result);
        });
        self.lsp.slots.completion_resolve.fire(task, rx);
    }

    /// Merges a `completionItem/resolve` answer into the open menu.
    pub(in crate::editor) fn poll_completion_resolve_slot(&mut self) -> bool {
        let Some(result) = self
            .lsp
            .slots
            .completion_resolve
            .poll_with_timeout(Duration::from_secs(10))
        else {
            return false;
        };
        let Ok(result) = result else {
            return false;
        };
        if result.menu_generation != self.completion_menu.generation() {
            return false;
        }
        self.completion_menu
            .apply_resolved(result.source_index, result.item);
        self.mark_dirty();
        true
    }

    /// Runs the `command` of accepted completion items: client-side editor
    /// commands locally, everything else through `workspace/executeCommand`.
    pub(in crate::editor) async fn run_pending_completion_command(&mut self) {
        if self.lsp.state.pending_completion_commands.is_empty() {
            return;
        }
        let commands = std::mem::take(&mut self.lsp.state.pending_completion_commands);
        for command in commands {
            match command.command.as_str() {
                "editor.action.triggerParameterHints" => {
                    self.lsp.intents.signature_help = true;
                }
                "editor.action.triggerSuggest" => self.request_completion(),
                _ => {
                    let Some(lsp) = self.lsp.state.lsp_manager.clone() else {
                        continue;
                    };
                    let Some(file_path) = self.buffer().file_path().map(|s| s.to_string()) else {
                        continue;
                    };
                    let Some(language_id) = self.language_id_for_path(&file_path) else {
                        continue;
                    };
                    let Some(uri) = crate::lsp::uri_from_file_path(&file_path) else {
                        continue;
                    };
                    self.ensure_lsp_document_synced().await;
                    tokio::spawn(async move {
                        let _ = lsp
                            .execute_command(command.command, command.arguments, &uri, &language_id)
                            .await;
                    });
                }
            }
        }
    }

    /// Ends the completion session and forgets any request still on its way.
    pub fn dismiss_completion(&mut self) {
        self.completion_menu.hide();
        self.lsp.slots.completion.cancel();
        self.lsp.slots.completion_resolve.cancel();
        self.lsp.intents.completion = None;
        self.lsp.intents.completion_due = None;
    }

    /// Fallback trigger characters are single characters, but `::` and `->`
    /// only mean something as a pair: a lone `:` or `>` is not a trigger.
    fn typed_fallback_trigger_complete(&self, typed: char) -> bool {
        let cursor_col = self.buffer().cursor().col().0;
        let line = self.buffer().line_index(self.buffer().cursor().line());
        let before = |back: usize| {
            cursor_col
                .checked_sub(back)
                .and_then(|col| line.grapheme_at(crate::unicode::GraphemeCol(col)))
        };
        match typed {
            ':' => before(2).as_deref() == Some(":"),
            '>' => before(2).as_deref() == Some("-"),
            _ => true,
        }
    }
}

/// Whether `c` is part of a completion-prefix keyword.
///
/// Looser than the Vim motion-word definition: hyphens count, so a Tailwind
/// class like `w-1/2` is treated as one prefix when filtering completions and
/// the menu doesn't collapse mid-token. Motion code (`dw`, `ciw`, etc.) keeps
/// the strict alnum+`_` rule. Mirrored in `is_completion_ident_char` in
/// `ovim-core/src/editor/input/insert_mode.rs`.
fn is_completion_keyword_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

fn completion_trigger_context_from_index(
    index: &crate::text_index::LineIndex,
    cursor_col: usize,
) -> (usize, String) {
    let mut start = cursor_col.min(index.grapheme_count());
    let end = index.grapheme_to_char(crate::unicode::GraphemeCol(start)).0;
    while start > 0 {
        let Some(grapheme) = index.grapheme_at(crate::unicode::GraphemeCol(start - 1)) else {
            break;
        };
        if !grapheme.chars().all(is_completion_keyword_char) {
            break;
        }
        start -= 1;
    }
    let start = index.grapheme_to_char(crate::unicode::GraphemeCol(start)).0;
    (start, index.slice_chars(start..end))
}

#[cfg(test)]
fn completion_trigger_context_from_line(text: &str, cursor_col: usize) -> (usize, String) {
    completion_trigger_context_from_index(
        &crate::text_index::LineIndex::from_text(text),
        cursor_col,
    )
}

/// Returns the most common `textEdit.range.start.character` (UTF-16) across
/// completion items. Uses majority vote to handle multi-server scenarios.
///
/// The tally is a `BTreeMap` (not a `HashMap`) and the tie-break is explicit:
/// `HashMap` iteration order is randomized per process, so a popularity tie —
/// common when two servers both contribute (e.g. Java + JDTLS extensions) —
/// otherwise produced a non-deterministic trigger column across runs. On a tie
/// we prefer the *lower* start column (the longer replacement range), which is
/// what a single server's edits would do. (OV-00267)
fn text_edit_majority_start(items: &[lsp_types::CompletionItem]) -> Option<u32> {
    use std::collections::BTreeMap;

    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for item in items {
        let start = match &item.text_edit {
            Some(lsp_types::CompletionTextEdit::Edit(edit)) => edit.range.start.character,
            Some(lsp_types::CompletionTextEdit::InsertAndReplace(ir)) => ir.insert.start.character,
            None => continue,
        };
        *counts.entry(start).or_default() += 1;
    }

    counts
        .into_iter()
        .max_by(|(start_a, count_a), (start_b, count_b)| {
            // Higher count wins; on a tie, the lower start column wins.
            count_a.cmp(count_b).then_with(|| start_b.cmp(start_a))
        })
        .map(|(start, _)| start)
}

#[cfg(test)]
mod tests {
    use super::{completion_trigger_context_from_line, text_edit_majority_start};
    use crate::unicode::grapheme_at_index;

    #[test]
    fn completion_trigger_context_basic_word() {
        let (col, prefix) = completion_trigger_context_from_line("foobar", 6);
        assert_eq!(col, 0);
        assert_eq!(prefix, "foobar");
    }

    #[test]
    fn completion_trigger_context_after_dot() {
        let (col, prefix) = completion_trigger_context_from_line("foo.", 4);
        assert_eq!(col, 4);
        assert_eq!(prefix, "");
    }

    #[test]
    fn completion_trigger_context_member_prefix() {
        let (col, prefix) = completion_trigger_context_from_line("foo.bar", 7);
        assert_eq!(col, 4);
        assert_eq!(prefix, "bar");
    }

    #[test]
    fn completion_trigger_context_double_colon() {
        let (col, prefix) = completion_trigger_context_from_line("foo::bar", 8);
        assert_eq!(col, 5);
        assert_eq!(prefix, "bar");
    }

    #[test]
    fn completion_trigger_context_underscore_digits() {
        let (col, prefix) = completion_trigger_context_from_line("__x1", 4);
        assert_eq!(col, 0);
        assert_eq!(prefix, "__x1");
    }

    // Tailwind classes contain hyphens; the fallback scanner must keep them
    // as part of the prefix so filtering matches the LSP's view of the token.
    #[test]
    fn completion_trigger_context_hyphenated_prefix() {
        let (col, prefix) = completion_trigger_context_from_line("class=\"bg-wh", 12);
        assert_eq!(col, 7);
        assert_eq!(prefix, "bg-wh");
    }

    #[test]
    fn completion_trigger_context_trailing_hyphen() {
        let (col, prefix) = completion_trigger_context_from_line("w-", 2);
        assert_eq!(col, 0);
        assert_eq!(prefix, "w-");
    }

    #[test]
    fn trigger_char_detection_dot() {
        let line = "s.";
        assert_eq!(grapheme_at_index(line, 1), Some("."));
    }

    fn item_with_text_edit(
        label: &str,
        start_char: u32,
        end_char: u32,
    ) -> lsp_types::CompletionItem {
        lsp_types::CompletionItem {
            label: label.to_string(),
            text_edit: Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
                range: lsp_types::Range {
                    start: lsp_types::Position {
                        line: 0,
                        character: start_char,
                    },
                    end: lsp_types::Position {
                        line: 0,
                        character: end_char,
                    },
                },
                new_text: label.to_string(),
            })),
            ..Default::default()
        }
    }

    #[test]
    fn majority_start_single_server() {
        let items = vec![
            item_with_text_edit("bg-white", 11, 16),
            item_with_text_edit("bg-black", 11, 16),
            item_with_text_edit("bg-red-500", 11, 16),
        ];
        assert_eq!(text_edit_majority_start(&items), Some(11));
    }

    #[test]
    fn majority_start_multi_server_picks_most_common() {
        // 3 items from Tailwind (start=11), 1 from TypeScript (start=14)
        let items = vec![
            item_with_text_edit("bg-white", 11, 16),
            item_with_text_edit("bg-black", 11, 16),
            item_with_text_edit("bg-red-500", 11, 16),
            item_with_text_edit("white", 14, 16),
        ];
        assert_eq!(text_edit_majority_start(&items), Some(11));
    }

    #[test]
    fn majority_start_tie_prefers_lower_column_deterministically() {
        // Two servers, equal popularity (2 items each) at start=8 and start=11.
        // The result must be stable across runs (no HashMap order dependence)
        // and pick the lower column. (OV-00267)
        let items = vec![
            item_with_text_edit("alpha", 11, 16),
            item_with_text_edit("beta", 8, 16),
            item_with_text_edit("gamma", 11, 16),
            item_with_text_edit("delta", 8, 16),
        ];
        assert_eq!(text_edit_majority_start(&items), Some(8));
        // Reversed input order — same answer.
        let items_rev: Vec<_> = items.into_iter().rev().collect();
        assert_eq!(text_edit_majority_start(&items_rev), Some(8));
    }

    #[test]
    fn majority_start_no_text_edits() {
        let items = vec![lsp_types::CompletionItem {
            label: "foo".to_string(),
            ..Default::default()
        }];
        assert_eq!(text_edit_majority_start(&items), None);
    }

    #[test]
    fn majority_start_empty() {
        assert_eq!(text_edit_majority_start(&[]), None);
    }
}
