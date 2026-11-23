//! Pointer selection in buffer coordinates.
//!
//! Frontends that already know the buffer position under the pointer (the TUI
//! after hit-testing, the GUI from its DOM) drive these three entry points.
//! Columns are grapheme columns; display-column mapping stays with the caller.
//! The behaviour follows Vim with `mouse=a` (checked in `nvim --clean`):
//!
//! - a press places the cursor; dragging from it starts charwise Visual mode
//!   anchored at the press, but only once the pointer reaches another cell;
//! - a double-click selects the word (or whitespace / punctuation run) under
//!   the pointer, and a drag after it extends word by word;
//! - a triple-click selects the line (linewise Visual), a drag extends by line;
//! - shift-press starts Visual mode at the old cursor in Normal/Insert mode, and
//!   in Visual mode moves the selection end *nearest* the click, keeping the other.
//!
//! Starting a selection from Insert mode ends the insert session first, as any
//! other cursor movement out of it does.

use crate::editor::{Editor, PointerGranularity};
use crate::mode::Mode;
use crate::textobjects::TextObjects;
use crate::unicode::{char_to_grapheme_col, CharCol, GraphemeCol};

impl Editor {
    /// Ends the insert session a pointer gesture interrupts (like Ctrl-O: no
    /// cursor step-back, no `count` repeat) and dismisses its popups.
    fn end_insert_for_pointer(&mut self) {
        if self.mode() == Mode::Insert {
            self.dismiss_completion();
            super::insert_mode::finish_insert_mode(self, true);
        }
    }

    /// Clamps a pointer position to a cell a Normal/Visual cursor can occupy.
    fn clamp_pointer_cell(&self, line: usize, col: usize) -> (usize, usize) {
        let line = line.min(self.buffer().line_count().saturating_sub(1));
        let len = self.buffer().line_grapheme_count(line);
        (line, col.min(len.saturating_sub(1)))
    }

    fn set_pointer_cursor(&mut self, line: usize, col: usize) {
        self.buffer_mut()
            .cursor_mut()
            .set_position(line, GraphemeCol(col));
    }

    /// First and last grapheme column of the word, whitespace run or
    /// punctuation run under the cursor (`iw`); the cell itself on an empty line.
    fn word_span_at_cursor(&self) -> (usize, usize) {
        let line = self.buffer().cursor().line();
        let col = self.buffer().cursor().col().0;
        let Some(range) = TextObjects::inner_word(self.buffer()) else {
            return (col, col);
        };
        let text = crate::display::line_content(self.buffer().rope(), line).to_string();
        let first = char_to_grapheme_col(&text, range.start_col).0;
        let last = char_to_grapheme_col(&text, CharCol(range.end_col.0.saturating_sub(1))).0;
        (first, last.max(first))
    }

    /// Primary-button press at a buffer position.
    ///
    /// `click_count` is the consecutive-click count (1 click, 2 word, 3 line;
    /// it wraps like Vim's click cycle). `shift` extends the selection instead
    /// of starting one. The caller leaves Command/Search-style modes first.
    pub fn pointer_press(&mut self, line: usize, col: usize, click_count: u8, shift: bool) {
        let count = ((click_count.max(1) - 1) % 3) + 1;
        let pointer = &mut self.render_cache.mouse_state;
        pointer.is_dragging = true;
        pointer.granularity = PointerGranularity::Char;
        pointer.word_origin = None;

        if shift && count == 1 {
            self.pointer_extend(line, col);
            return;
        }
        if self.mode().is_visual() {
            self.set_mode(Mode::Normal);
        }
        if count == 1 {
            self.place_cursor_from_pointer(line, col);
            let cursor = self.cursor_position();
            self.render_cache.mouse_state.drag_origin = Some((cursor.line, cursor.col.0));
            return;
        }

        self.end_insert_for_pointer();
        let (line, col) = self.clamp_pointer_cell(line, col);
        self.set_pointer_cursor(line, col);
        self.render_cache.mouse_state.drag_origin = Some((line, col));
        if count == 2 {
            let (first, last) = self.word_span_at_cursor();
            self.set_visual_start(line, first);
            self.set_pointer_cursor(line, last);
            self.set_mode(Mode::Visual);
            let pointer = &mut self.render_cache.mouse_state;
            pointer.granularity = PointerGranularity::Word;
            pointer.word_origin = Some((line, first, last));
        } else {
            self.set_visual_start(line, col);
            self.set_mode(Mode::VisualLine);
            self.render_cache.mouse_state.granularity = PointerGranularity::Line;
        }
    }

    /// Shift-press: Vim's "extend selection".
    fn pointer_extend(&mut self, line: usize, col: usize) {
        self.end_insert_for_pointer();
        let target = self.clamp_pointer_cell(line, col);
        let here = self.clamp_pointer_cell(
            self.buffer().cursor().line(),
            self.buffer().cursor().col().0,
        );
        let anchor = match (self.mode(), self.visual_start()) {
            (mode, Some(anchor)) if mode.is_visual() => {
                // The end farthest from the click stays put as the anchor.
                let anchor = self.clamp_pointer_cell(anchor.0, anchor.1);
                let linewise = mode == Mode::VisualLine;
                let key = |cell: (usize, usize)| if linewise { (cell.0, 0) } else { cell };
                let (lo, hi) = if key(anchor) <= key(here) {
                    (anchor, here)
                } else {
                    (here, anchor)
                };
                let t = key(target);
                let lo_is_farthest = if t < key(lo) {
                    false
                } else if t > key(hi) {
                    true
                } else {
                    let to_lo = (t.0 - key(lo).0, t.1.abs_diff(key(lo).1));
                    let to_hi = (key(hi).0 - t.0, key(hi).1.abs_diff(t.1));
                    to_lo > to_hi
                };
                if lo_is_farthest {
                    lo
                } else {
                    hi
                }
            }
            _ => here,
        };
        self.set_visual_start(anchor.0, anchor.1);
        if !self.mode().is_visual() {
            self.set_mode(Mode::Visual);
        }
        self.set_pointer_cursor(target.0, target.1);
        self.render_cache.mouse_state.drag_origin = Some(anchor);
    }

    /// Pointer moved (primary button held) to a buffer position.
    pub fn pointer_drag(&mut self, line: usize, col: usize) {
        let state = &self.render_cache.mouse_state;
        if !state.is_dragging {
            return;
        }
        if !matches!(
            self.mode(),
            Mode::Normal | Mode::Insert | Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        ) {
            return;
        }
        let (granularity, origin, word) = (state.granularity, state.drag_origin, state.word_origin);
        let Some(origin) = origin else { return };
        let origin = self.clamp_pointer_cell(origin.0, origin.1);
        let target = self.clamp_pointer_cell(line, col);

        match granularity {
            PointerGranularity::Char => {
                if !self.mode().is_visual() {
                    if target == origin {
                        return;
                    }
                    self.end_insert_for_pointer();
                    self.set_visual_start(origin.0, origin.1);
                    self.set_mode(Mode::Visual);
                }
                self.set_pointer_cursor(target.0, target.1);
            }
            PointerGranularity::Word => {
                let Some((word_line, first, last)) = word else {
                    return;
                };
                self.set_pointer_cursor(target.0, target.1);
                let (target_first, target_last) = self.word_span_at_cursor();
                let (anchor, head) = if target < origin {
                    ((word_line, last), target_first)
                } else {
                    ((word_line, first), target_last)
                };
                self.set_visual_start(anchor.0, anchor.1);
                if !self.mode().is_visual() {
                    self.set_mode(Mode::Visual);
                }
                self.set_pointer_cursor(target.0, head);
            }
            PointerGranularity::Line => {
                self.set_visual_start(origin.0, origin.1);
                if self.mode() != Mode::VisualLine {
                    self.set_mode(Mode::VisualLine);
                }
                self.set_pointer_cursor(target.0, target.1);
            }
        }
    }

    /// Primary button released: the selection (if any) stays as Visual mode.
    pub fn pointer_release(&mut self) {
        let pointer = &mut self.render_cache.mouse_state;
        pointer.is_dragging = false;
        pointer.granularity = PointerGranularity::Char;
        pointer.word_origin = None;
    }
}

#[cfg(test)]
#[path = "mouse_select_tests.rs"]
mod tests;
