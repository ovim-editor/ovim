//! Tab stops of an expanded snippet, navigable while in insert mode.
//!
//! Vim-friendly by design: it lives entirely inside insert mode. Tab / Shift-Tab
//! jump between stops, typing over a just-entered placeholder replaces it (the
//! select-mode behaviour of LuaSnip / UltiSnips without a select mode), and
//! Esc, or moving the cursor out of the current stop, ends the session.

use super::completion_accept::PlannedEdit;
use super::Editor;
use crate::snippet::Snippet;

#[derive(Debug, Clone)]
struct Stop {
    index: u32,
    /// Absolute char ranges in the buffer; the first is the primary.
    ranges: Vec<(usize, usize)>,
    /// `${1|a,b,c|}` choices; the first is inserted, the chooser offers all.
    choices: Vec<String>,
}

/// Live tab-stop state for one expanded snippet.
#[derive(Debug)]
pub struct SnippetSession {
    stops: Vec<Stop>,
    current: usize,
    /// Buffer length (chars) when the ranges were last brought up to date.
    doc_len: usize,
    /// The current placeholder still holds its default text: typing or
    /// Backspace replaces it.
    select_pending: bool,
}

impl SnippetSession {
    /// Tab stops of `snippet` inserted at absolute char offset `base`.
    pub fn new(snippet: &Snippet, base: usize) -> Self {
        Self {
            stops: snippet
                .tabstops
                .iter()
                .map(|stop| Stop {
                    index: stop.index,
                    ranges: stop
                        .ranges
                        .iter()
                        .map(|(a, b)| (base + a, base + b))
                        .collect(),
                    choices: stop.choices.clone(),
                })
                .collect(),
            current: 0,
            doc_len: 0,
            select_pending: false,
        }
    }

    /// Where the cursor goes right after the snippet is inserted.
    pub fn initial_cursor(&self) -> usize {
        self.stops[0].ranges[0].0
    }

    /// Called once the snippet text is in the buffer.
    pub(crate) fn finish_start(&mut self, doc_len: usize) {
        self.doc_len = doc_len;
        let (start, end) = self.stops[self.current].ranges[0];
        self.select_pending = self.stops[self.current].index != 0 && end > start;
    }

    fn current_range(&self) -> (usize, usize) {
        self.stops[self.current].ranges[0]
    }

    /// Resizes one range to `new_len` chars (its start stays put) and slides
    /// every offset at or after its old end along, so text typed at the end of
    /// a stop belongs to the stop and later stops keep their place.
    fn resize(&mut self, stop: usize, range: usize, new_len: usize) {
        let (start, end) = self.stops[stop].ranges[range];
        let delta = new_len as isize - (end - start) as isize;
        let slide = |p: usize| -> usize {
            if p >= end {
                (p as isize + delta).max(0) as usize
            } else {
                p
            }
        };
        for (stop_index, candidate) in self.stops.iter_mut().enumerate() {
            for (range_index, r) in candidate.ranges.iter_mut().enumerate() {
                if stop_index == stop && range_index == range {
                    r.1 = start + new_len;
                } else {
                    *r = (slide(r.0), slide(r.1));
                }
            }
        }
    }

    /// Drops stops that lie inside the current placeholder (the user is
    /// rewriting the text they were nested in).
    fn drop_nested_in_current(&mut self) {
        let (start, end) = self.current_range();
        let current_index = self.current;
        let mut position = 0;
        let mut new_current = current_index;
        self.stops.retain(|stop| {
            let this = position;
            position += 1;
            if this == current_index {
                return true;
            }
            let (a, b) = stop.ranges[0];
            let nested = a >= start && b <= end && a < end && (a, b) != (start, end);
            if nested && this < current_index {
                new_current -= 1;
            }
            !nested
        });
        self.current = new_current;
    }
}

impl Editor {
    /// Whether tab stops of an expanded snippet are being navigated.
    pub fn snippet_active(&self) -> bool {
        self.editing.snippet.is_some()
    }

    /// Opens the chooser (the completion popup) when the current stop is a
    /// `${1|a,b,c|}` choice stop. Enter/Tab accepts the highlighted choice
    /// over the placeholder, typing filters, Esc leaves the first choice.
    pub(crate) fn snippet_show_choices(&mut self) {
        let Some(session) = self.editing.snippet.as_ref() else {
            return;
        };
        let stop = &session.stops[session.current];
        if stop.choices.len() < 2 || !session.select_pending {
            return;
        }
        let (start, _) = stop.ranges[0];
        let choices = stop.choices.clone();
        let rope = self.buffer().rope();
        let line = rope.char_to_line(start.min(rope.len_chars()));
        let col = start - rope.line_to_char(line);
        let version = self.buffer().version();
        self.completion_menu
            .show_snippet_choices(&choices, col, version);
    }

    /// Enter/Tab in the chooser: the highlighted choice replaces the
    /// placeholder text; the stop stays current (the next Tab moves on).
    pub(crate) fn accept_snippet_choice(&mut self) {
        let Some(choice) = self
            .completion_menu
            .selected_item()
            .map(|item| item.label.clone())
        else {
            return;
        };
        let Some(session) = self.editing.snippet.as_mut() else {
            return;
        };
        let (start, end) = session.current_range();
        let new_len = choice.chars().count();
        let current = session.current;
        session.resize(current, 0, new_len);
        session.select_pending = false;
        session.doc_len = (session.doc_len + new_len).saturating_sub(end - start);
        self.apply_offset_edits_as_one_undo(
            vec![PlannedEdit {
                start,
                end,
                text: choice,
            }],
            start + new_len,
        );
    }

    /// Ends tab-stop navigation (Esc, cursor left the stop, last stop reached).
    pub fn end_snippet_session(&mut self) {
        self.editing.snippet = None;
    }

    fn cursor_char_offset(&self) -> usize {
        let line = self.buffer().cursor().line();
        self.buffer().rope().line_to_char(line) + self.buffer().cursor_char_col().0
    }

    /// Brings the stop ranges up to date after an edit and ends the session
    /// when the cursor left the current stop. Returns whether it survived.
    pub(crate) fn snippet_sync(&mut self) -> bool {
        let len = self.buffer().rope().len_chars();
        let cursor = self.cursor_char_offset();
        let Some(session) = self.editing.snippet.as_mut() else {
            return false;
        };
        let delta = len as isize - session.doc_len as isize;
        if delta != 0 {
            session.drop_nested_in_current();
            let (start, end) = session.current_range();
            let new_len = ((end - start) as isize + delta).max(0) as usize;
            let current = session.current;
            session.resize(current, 0, new_len);
            session.doc_len = len;
        }
        let (start, end) = session.current_range();
        if cursor < start || cursor > end {
            self.editing.snippet = None;
            return false;
        }
        true
    }

    /// After a key that moved or edited: keep the session coherent.
    pub(crate) fn snippet_after_key(&mut self) {
        if self.editing.snippet.is_some() {
            self.snippet_sync();
        }
    }

    /// The placeholder was entered but not touched: the first typed
    /// character or Backspace/Delete replaces its default text.
    pub(crate) fn snippet_replace_pending_placeholder(&mut self) -> bool {
        if !self.snippet_sync() {
            return false;
        }
        let cursor = self.cursor_char_offset();
        let Some(session) = self.editing.snippet.as_mut() else {
            return false;
        };
        if !session.select_pending {
            return false;
        }
        let (start, end) = session.current_range();
        session.select_pending = false;
        if cursor != start || end <= start {
            return false;
        }
        let removed = end - start;
        session.drop_nested_in_current();
        let current = session.current;
        session.resize(current, 0, 0);
        session.doc_len -= removed;
        self.apply_offset_edits_as_one_undo(
            vec![PlannedEdit {
                start,
                end,
                text: String::new(),
            }],
            start,
        );
        true
    }

    /// Any key other than typing/deleting leaves the placeholder as plain text.
    pub(crate) fn snippet_clear_pending(&mut self) {
        if let Some(session) = self.editing.snippet.as_mut() {
            session.select_pending = false;
        }
    }

    /// Tab (`forward`) / Shift-Tab: jump to the next / previous stop.
    /// Returns false when no session is active (the key is then a plain Tab).
    pub(crate) fn snippet_jump(&mut self, forward: bool) -> bool {
        if !self.snippet_sync() {
            return false;
        }
        self.snippet_sync_mirrors();

        let len = self.buffer().rope().len_chars();
        let Some(session) = self.editing.snippet.as_mut() else {
            return false;
        };
        let target = if forward {
            (session.current + 1..session.stops.len())
                .find(|&i| session.stops[i].ranges[0].1 <= len)
        } else {
            (0..session.current)
                .rev()
                .find(|&i| session.stops[i].ranges[0].1 <= len)
        };
        let Some(target) = target else {
            if forward {
                self.editing.snippet = None;
            }
            return true;
        };
        session.current = target;
        let (start, end) = session.current_range();
        let is_final = session.stops[target].index == 0;
        session.select_pending = !is_final && end > start;
        session.doc_len = len;
        if is_final {
            self.editing.snippet = None;
        }
        let (line, col) = {
            let rope = self.buffer().rope();
            let line = rope.char_to_line(start.min(len));
            (line, start.min(len) - rope.line_to_char(line))
        };
        self.buffer_mut()
            .set_cursor_char_col(line, crate::unicode::CharCol(col));
        self.snippet_show_choices();
        true
    }

    /// Mirrors of the stop being left take over its text.
    fn snippet_sync_mirrors(&mut self) {
        let Some(session) = self.editing.snippet.as_ref() else {
            return;
        };
        let stop = &session.stops[session.current];
        if stop.ranges.len() < 2 {
            return;
        }
        let (start, end) = stop.ranges[0];
        let primary: String = self.buffer().rope().slice(start..end).to_string();
        let mirrors: Vec<(usize, usize)> = stop.ranges[1..].to_vec();
        let mut edits: Vec<PlannedEdit> = mirrors
            .iter()
            .filter(|(a, b)| self.buffer().rope().slice(*a..*b) != primary.as_str())
            .map(|&(a, b)| PlannedEdit {
                start: a,
                end: b,
                text: primary.clone(),
            })
            .collect();
        if edits.is_empty() {
            return;
        }
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
        let cursor = self.cursor_char_offset();
        // The cursor sits in the primary; mirrors before it shift it.
        let cursor_shift: isize = edits
            .iter()
            .filter(|e| e.end <= cursor)
            .map(|e| primary.chars().count() as isize - (e.end - e.start) as isize)
            .sum();
        let plan = edits.clone();
        self.apply_offset_edits_as_one_undo(plan, (cursor as isize + cursor_shift) as usize);
        let len = self.buffer().rope().len_chars();
        if let Some(session) = self.editing.snippet.as_mut() {
            let new_len = primary.chars().count();
            let current = session.current;
            // Resize each mirror of the stop just left (descending, like the
            // edits themselves, so earlier offsets stay valid).
            let mut order: Vec<usize> = (1..session.stops[current].ranges.len()).collect();
            order.sort_by_key(|&i| std::cmp::Reverse(session.stops[current].ranges[i].0));
            for range in order {
                let (a, b) = session.stops[current].ranges[range];
                if edits.iter().any(|e| e.start == a && e.end == b) {
                    session.resize(current, range, new_len);
                }
            }
            session.doc_len = len;
        }
    }

    /// The untouched placeholder to draw as selected: `(line, start_col,
    /// end_col)` in chars, clipped to its first line.
    pub fn snippet_placeholder_highlight(&self) -> Option<(usize, usize, usize)> {
        let session = self.editing.snippet.as_ref()?;
        if !session.select_pending {
            return None;
        }
        let (start, end) = session.current_range();
        let rope = self.buffer().rope();
        if end <= start || end > rope.len_chars() {
            return None;
        }
        let line = rope.char_to_line(start);
        let line_start = rope.line_to_char(line);
        let line_end = line_start + self.buffer().line_content_len(line);
        Some((line, start - line_start, end.min(line_end) - line_start))
    }
}
