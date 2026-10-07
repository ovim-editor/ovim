use crate::buffer::Buffer;
use crate::change::{InsertEntryMode, TextObjectType};
use crate::edit::Edit;
use crate::indentation::{leading_str, leading_width, IndentOptions};
use crate::textobjects::TextObjects;
use crate::unicode::{CharCol, GraphemeCol};

#[derive(Clone, Copy, Debug)]
pub enum CaseTransform {
    Lower,
    Upper,
    Toggle,
}

impl CaseTransform {
    pub(crate) fn apply_to(self, text: &str) -> String {
        match self {
            Self::Lower => text.to_lowercase(),
            Self::Upper => text.to_uppercase(),
            Self::Toggle => text
                .chars()
                .map(|ch| {
                    if ch.is_lowercase() {
                        ch.to_uppercase().to_string()
                    } else {
                        ch.to_lowercase().to_string()
                    }
                })
                .collect(),
        }
    }
}

/// What a `gu` / `gU` / `g~` operator covers; resolved from the cursor each time,
/// so `.` applies the same motion at the new position.
#[derive(Clone, Copy, Debug)]
pub enum CaseTarget {
    /// `gUU` / `gUgU` / `3gUU`
    Lines { count: usize },
    /// `gUw`
    WordForward { count: usize },
    /// `gUe`
    WordEnd { count: usize },
    /// `gU$`
    ToEndOfLine,
}

/// The extent of a Visual selection relative to its start, so `.` can re-apply
/// an operator to the same amount of text at the cursor.
#[derive(Clone, Copy, Debug)]
pub enum VisualShape {
    /// Characterwise: `line_delta` lines further, ending `offset_col` columns
    /// past the start (single line) or past column 0 (several lines).
    Char {
        line_delta: usize,
        offset_col: usize,
    },
    Line {
        line_count: usize,
    },
    Block {
        line_count: usize,
        width: usize,
    },
}

/// What Visual `u` / `U` / `~` / `r{char}` does to the selected text.
#[derive(Clone, Copy, Debug)]
pub enum VisualTransform {
    Case(CaseTransform),
    /// Every character but line breaks becomes this one.
    Replace(char),
}

impl VisualTransform {
    fn apply_to(self, text: &str) -> String {
        match self {
            Self::Case(transform) => transform.apply_to(text),
            Self::Replace(ch) => text
                .chars()
                .map(|c| if c == '\n' { c } else { ch })
                .collect(),
        }
    }
}

/// Semantic repeat actions for dot-repeat (Pattern B).
///
/// Unlike `Change` (which handles both undo and repeat), `RepeatAction`
/// captures only the intent needed to re-execute an operation at the
/// current cursor position. Undo is handled separately via `Change::Recorded`.
///
/// Use Pattern B for operations where repeat should be semantic at the
/// current cursor position. This includes both normal-mode-only edits and
/// change/open/replace flows that pass through insert mode before finalizing
/// a repeat intent.
/// See the module doc in `change.rs` for the full boundary guide.
#[derive(Clone, Debug)]
pub enum RepeatAction {
    /// J / gJ — join lines
    JoinLines { count: usize, add_space: bool },
    /// >> — indent lines
    IndentLines {
        line_count: usize,
        options: IndentOptions,
    },
    /// << — dedent lines
    DedentLines {
        line_count: usize,
        options: IndentOptions,
    },
    /// ~ — toggle case at cursor
    ToggleCase { count: usize },
    /// guiw / gUiw / g~iw — case transform for text object
    ChangeCaseTextObject {
        object_type: TextObjectType,
        transform: CaseTransform,
    },
    /// guu / gUw / g~$ ... — case transform for a line or motion range
    ChangeCase {
        transform: CaseTransform,
        target: CaseTarget,
    },
    /// Visual u / U / ~ / r{char} on the same amount of text
    TransformVisual {
        shape: VisualShape,
        transform: VisualTransform,
    },
    /// Ctrl-A / Ctrl-X — increment/decrement number
    NumberOperation { delta: i64 },
    /// di" / di( / diw — delete text object
    DeleteTextObject { object_type: TextObjectType },
    /// df / dt / dF / dT — delete to character motion
    DeleteCharMotion {
        target: char,
        forward: bool,
        till: bool,
        count: usize,
    },
    /// x — delete character(s) forward
    DeleteCharForward { count: usize },
    /// X — delete character(s) backward
    DeleteCharBackward { count: usize },
    /// dd — delete line(s)
    DeleteLines { count: usize },
    /// D / d$ — delete to end of line
    DeleteToEndOfLine,
    /// dw — delete word forward
    DeleteWordForward { count: usize },
    /// cw delete phase — ce-like on a non-blank, dw-like on a blank
    DeleteWordChange { count: usize },
    /// cW delete phase — cE-like on a non-blank, dW-like on a blank
    DeleteWordChangeBig { count: usize },
    /// ce delete phase — like DeleteWordEnd but leaves the insert point un-clamped
    ChangeWordEnd { count: usize },
    /// cE delete phase — like DeleteWordEndBig but leaves the insert point un-clamped
    ChangeWordEndBig { count: usize },
    /// c% delete phase — like DeleteToMatchingBracket but leaves the insert point un-clamped
    ChangeToMatchingBracket,
    /// cgn/cgN delete phase — delete the next/previous search match
    DeleteSearchMatch {
        search_pattern: String,
        search_forward: bool,
    },
    /// db — delete word backward
    DeleteWordBackward { count: usize },
    /// de — delete to end of word (inclusive)
    DeleteWordEnd { count: usize },
    /// dB — delete WORD backward
    DeleteWordBackwardBig { count: usize },
    /// dE — delete to end of WORD (inclusive)
    DeleteWordEndBig { count: usize },
    /// dh — delete character left
    DeleteCharLeft { count: usize },
    /// d0 — delete to start of line
    DeleteToStartOfLine,
    /// d^ — delete to first non-blank
    DeleteToFirstNonBlank,
    /// dW — delete WORD forward
    DeleteWordForwardBig { count: usize },
    /// dj — delete current + count lines down
    DeleteLineDown { count: usize },
    /// dk — delete current + count lines up
    DeleteLineUp { count: usize },
    /// d} — delete to paragraph forward
    DeleteParagraphForward { count: usize },
    /// d{ — delete to paragraph backward
    DeleteParagraphBackward { count: usize },
    /// dG — delete to last line (or target line)
    DeleteToLastLine { target_line: usize },
    /// dgg — delete to first line (or target line)
    DeleteToFirstLine { target_line: usize },
    /// d% — delete to matching bracket
    DeleteToMatchingBracket,
    /// r — replace character(s) at cursor
    ReplaceChar { ch: char, count: usize },
    /// R — replace mode replay
    ReplaceMode { replacements: String },
    /// p — paste after cursor
    PasteAfter { count: usize },
    /// P — paste before cursor
    PasteBefore { count: usize },
    /// o/O — open a line below/above, then replay inserted text
    OpenLine {
        above: bool,
        inserted_text: String,
        options: IndentOptions,
    },
    /// Visual-mode character-wise delete (v...d/x)
    DeleteVisualChar {
        line_delta: usize,
        offset_col: usize,
    },
    /// Visual-line delete (V...d/x)
    DeleteVisualLine { line_count: usize },
    /// Visual-block delete (Ctrl-V...d/x)
    DeleteVisualBlock { line_count: usize, width: usize },
    /// Visual-block `I` / `A` / `c`: delete `delete_width` columns at the
    /// cursor on `line_count` lines (`c` only), then insert the text on each
    /// line at `column`, relative to the cursor column.
    VisualBlockInsert {
        line_count: usize,
        delete_width: usize,
        column: BlockColumn,
        inserted_text: String,
    },
    /// Change operator — semantic delete + insert text (cc, C, s, S, cj, ck, etc.)
    Change {
        delete: Box<RepeatAction>,
        inserted_text: String,
        linewise: bool,
    },
    /// Direct insert-mode session (`i` / `a` / `I` / `A`) dot-repeat.
    ///
    /// `origin_offset` is the absolute char offset where the original session
    /// began, after `entry_mode` repositioned the cursor. `edits` are the raw
    /// `Edit`s captured by `buffer.record()` during the session, still with
    /// their original absolute offsets. Replay subtracts `origin_offset` from
    /// each edit's offset and adds the new origin — a single translation per
    /// edit that preserves intra-session geometry, including edits that went
    /// below the origin (e.g., `<BS>` at column 0 joining lines).
    InsertSession {
        count: usize,
        entry_mode: InsertEntryMode,
        origin_offset: usize,
        edits: Vec<Edit>,
    },
}

impl RepeatAction {
    /// Overrides the original command count for `[count].`.
    /// The resulting action becomes the template for subsequent repeats.
    pub fn with_count(mut self, count: usize) -> Self {
        match &mut self {
            Self::JoinLines { count: n, .. }
            | Self::ToggleCase { count: n }
            | Self::DeleteCharMotion { count: n, .. }
            | Self::DeleteCharForward { count: n }
            | Self::DeleteCharBackward { count: n }
            | Self::DeleteLines { count: n }
            | Self::DeleteWordForward { count: n }
            | Self::DeleteWordChange { count: n }
            | Self::DeleteWordChangeBig { count: n }
            | Self::ChangeWordEnd { count: n }
            | Self::ChangeWordEndBig { count: n }
            | Self::DeleteWordBackward { count: n }
            | Self::DeleteWordEnd { count: n }
            | Self::DeleteWordBackwardBig { count: n }
            | Self::DeleteWordEndBig { count: n }
            | Self::DeleteCharLeft { count: n }
            | Self::DeleteWordForwardBig { count: n }
            | Self::DeleteLineDown { count: n }
            | Self::DeleteLineUp { count: n }
            | Self::DeleteParagraphForward { count: n }
            | Self::DeleteParagraphBackward { count: n }
            | Self::ReplaceChar { count: n, .. }
            | Self::PasteAfter { count: n }
            | Self::PasteBefore { count: n }
            | Self::InsertSession { count: n, .. }
            | Self::IndentLines { line_count: n, .. }
            | Self::DedentLines { line_count: n, .. } => *n = count,
            Self::NumberOperation { delta } => *delta = delta.signum() * count as i64,
            Self::Change { delete, .. } => **delete = delete.as_ref().clone().with_count(count),
            Self::DeleteToLastLine { target_line } | Self::DeleteToFirstLine { target_line } => {
                *target_line = count.saturating_sub(1);
            }
            // These actions currently carry geometry or a resolved text object,
            // rather than a command count.
            _ => {}
        }
        self
    }

    /// Extracts the register payload from the recorded deletion phase. Undo
    /// replays the same edits mechanically without changing any registers.
    pub(crate) fn deleted_register(
        &self,
        edits: &[Edit],
        before: &ropey::Rope,
    ) -> Option<(String, crate::editor::RegisterType)> {
        use crate::editor::RegisterType;
        let first = edits
            .iter()
            .find(|edit| matches!(edit, Edit::Delete { .. }))?;
        let register_type = match self {
            Self::Change { linewise: true, .. }
            | Self::DeleteLines { .. }
            | Self::DeleteVisualLine { .. }
            | Self::DeleteLineDown { .. }
            | Self::DeleteLineUp { .. }
            | Self::DeleteToLastLine { .. }
            | Self::DeleteToFirstLine { .. }
            | Self::DeleteTextObject {
                object_type: TextObjectType::Paragraph { .. },
            } => RegisterType::Line,
            Self::Change { delete, .. } => return delete.deleted_register(edits, before),
            Self::DeleteVisualBlock { .. } | Self::VisualBlockInsert { .. } => RegisterType::Block,
            Self::DeleteParagraphForward { .. } | Self::DeleteParagraphBackward { .. } => {
                let starts_line =
                    before.line_to_char(before.char_to_line(first.offset())) == first.offset();
                if starts_line && first.text().ends_with('\n') {
                    RegisterType::Line
                } else {
                    RegisterType::Character
                }
            }
            Self::DeleteCharMotion { .. }
            | Self::DeleteCharForward { .. }
            | Self::DeleteCharBackward { .. }
            | Self::DeleteToEndOfLine
            | Self::DeleteWordForward { .. }
            | Self::DeleteWordChange { .. }
            | Self::DeleteWordChangeBig { .. }
            | Self::ChangeWordEnd { .. }
            | Self::ChangeWordEndBig { .. }
            | Self::ChangeToMatchingBracket
            | Self::DeleteSearchMatch { .. }
            | Self::DeleteWordBackward { .. }
            | Self::DeleteWordEnd { .. }
            | Self::DeleteWordBackwardBig { .. }
            | Self::DeleteWordEndBig { .. }
            | Self::DeleteCharLeft { .. }
            | Self::DeleteToStartOfLine
            | Self::DeleteToFirstNonBlank
            | Self::DeleteWordForwardBig { .. }
            | Self::DeleteToMatchingBracket
            | Self::DeleteVisualChar { .. }
            | Self::DeleteTextObject { .. } => RegisterType::Character,
            _ => return None,
        };
        let text = if register_type == RegisterType::Block {
            edits
                .iter()
                .filter_map(|edit| match edit {
                    Edit::Delete { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            // Change replay may also remove autoindent after an empty insert;
            // only its first deletion belongs in the delete register.
            first.text().to_string()
        };
        Some((text, register_type))
    }

    /// Execute this action at the current cursor position.
    /// Caller is responsible for wrapping in `buffer.record()`.
    pub fn execute(&self, buffer: &mut Buffer) {
        match self {
            Self::JoinLines { count, add_space } => {
                if *add_space {
                    let _ = buffer.join_lines(*count);
                } else {
                    let _ = buffer.join_lines_no_space(*count);
                }
            }
            Self::IndentLines {
                line_count,
                options,
            } => {
                let start = buffer.cursor().line();
                let end = start + line_count;
                buffer.indent_lines_at(start, end, *options);
                buffer.set_cursor_char_col(start, buffer.first_non_blank_col(start));
            }
            Self::DedentLines {
                line_count,
                options,
            } => {
                let start = buffer.cursor().line();
                let end = start + line_count;
                buffer.dedent_lines_at(start, end, *options);
                buffer.set_cursor_char_col(start, buffer.first_non_blank_col(start));
            }
            Self::ToggleCase { count } => {
                for _ in 0..*count {
                    if !buffer.toggle_char_at_cursor() {
                        break;
                    }
                }
            }
            Self::ChangeCaseTextObject {
                object_type,
                transform,
            } => {
                if let Some(range) = object_type.resolve(buffer) {
                    let Ok(original) = TextObjects::yank_range(buffer, range) else {
                        return;
                    };
                    let transformed = transform.apply_to(&original);
                    if transformed != original {
                        buffer.delete_range(
                            range.start_line,
                            range.start_col,
                            range.end_line,
                            range.end_col,
                        );
                        buffer.insert_text_at(range.start_line, range.start_col, &transformed);
                    }
                    // The cursor lands on the start of the text object, changed or not.
                    buffer.set_cursor_char_col(range.start_line, range.start_col);
                }
            }
            Self::ChangeCase { transform, target } => change_case(buffer, *transform, *target),
            Self::TransformVisual { shape, transform } => {
                transform_visual_shape(buffer, *shape, *transform);
            }
            Self::NumberOperation { delta } => {
                buffer.modify_number_at_cursor(*delta);
            }
            Self::DeleteTextObject { object_type } => {
                buffer.delete_text_object(object_type);
            }
            Self::DeleteCharMotion {
                target,
                forward,
                till,
                count,
            } => {
                buffer.delete_char_motion(*target, *forward, *till, *count);
            }
            Self::DeleteCharForward { count } => {
                buffer.delete_chars_forward(*count);
            }
            Self::DeleteCharBackward { count } => {
                buffer.delete_chars_backward(*count);
            }
            Self::DeleteLines { count } => {
                buffer.delete_lines(*count);
            }
            Self::DeleteToEndOfLine => {
                buffer.delete_to_end_of_line();
            }
            Self::DeleteWordForward { count } => {
                buffer.delete_word_forward(*count);
            }
            Self::DeleteWordChange { count } => {
                buffer.change_word_forward(*count);
            }
            Self::DeleteWordChangeBig { count } => {
                buffer.change_word_forward_big(*count);
            }
            Self::ChangeWordEnd { count } => {
                buffer.change_word_end(*count);
            }
            Self::ChangeWordEndBig { count } => {
                buffer.change_word_end_big(*count);
            }
            Self::ChangeToMatchingBracket => {
                buffer.change_to_matching_bracket();
            }
            Self::DeleteSearchMatch {
                search_pattern,
                search_forward,
            } => {
                let line_idx = buffer.cursor().line();
                let grapheme_col = buffer.cursor().col();

                let mut search = crate::search::Search::new_with_options(
                    search_pattern.clone(),
                    *search_forward,
                    true, // ignorecase
                    true, // smartcase
                );

                if let Some((match_line, match_grapheme_col, match_text)) =
                    search.find_next(buffer, line_idx, grapheme_col)
                {
                    // find_next returns grapheme col; delete_range needs char col.
                    // Convert via the matched line's text.
                    let match_col = buffer
                        .line_text(match_line)
                        .map(|line_text| {
                            crate::unicode::grapheme_to_char_col(
                                &line_text,
                                GraphemeCol(match_grapheme_col),
                            )
                        })
                        .unwrap_or(CharCol(match_grapheme_col));
                    let match_len = match_text.chars().count();
                    let match_end_col = match_col + match_len;
                    buffer.delete_range(match_line, match_col, match_line, match_end_col);
                    buffer.set_cursor_char_col(match_line, match_col);
                }
            }
            Self::DeleteWordBackward { count } => {
                buffer.delete_word_backward(*count);
            }
            Self::DeleteWordEnd { count } => {
                buffer.delete_word_end(*count);
            }
            Self::DeleteWordBackwardBig { count } => {
                buffer.delete_word_backward_big(*count);
            }
            Self::DeleteWordEndBig { count } => {
                buffer.delete_word_end_big(*count);
            }
            Self::DeleteCharLeft { count } => {
                buffer.delete_char_left(*count);
            }
            Self::DeleteToStartOfLine => {
                buffer.delete_to_start_of_line();
            }
            Self::DeleteToFirstNonBlank => {
                buffer.delete_to_first_non_blank();
            }
            Self::DeleteWordForwardBig { count } => {
                buffer.delete_word_forward_big(*count);
            }
            Self::DeleteLineDown { count } => {
                buffer.delete_line_down(*count);
            }
            Self::DeleteLineUp { count } => {
                buffer.delete_line_up(*count);
            }
            Self::DeleteParagraphForward { count } => {
                buffer.delete_paragraph_forward(*count);
            }
            Self::DeleteParagraphBackward { count } => {
                buffer.delete_paragraph_backward(*count);
            }
            Self::DeleteToLastLine { target_line } => {
                buffer.delete_to_last_line(*target_line);
            }
            Self::DeleteToFirstLine { target_line } => {
                buffer.delete_to_first_line(*target_line);
            }
            Self::DeleteToMatchingBracket => {
                buffer.delete_to_matching_bracket();
            }
            Self::ReplaceChar { ch, count } => {
                buffer.replace_chars_at_cursor(*ch, *count);
            }
            Self::ReplaceMode { replacements } => {
                let line_idx = buffer.cursor().line();
                let start_grapheme = buffer.cursor().col();
                let replacement_len = replacements.chars().count();

                if let Some(line) = buffer.line_text(line_idx) {
                    let line = line.into_owned();
                    let start_col = crate::unicode::grapheme_to_char_col(&line, start_grapheme);
                    let end_grapheme = GraphemeCol(
                        start_grapheme
                            .0
                            .saturating_add(replacement_len)
                            .min(crate::unicode::grapheme_count(&line)),
                    );
                    let end_col = crate::unicode::grapheme_to_char_col(&line, end_grapheme);

                    if start_col < end_col {
                        buffer.delete_range(line_idx, start_col, line_idx, end_col);
                    }
                    buffer.insert_text_at(line_idx, start_col, replacements);

                    let final_grapheme = GraphemeCol(
                        start_grapheme
                            .0
                            .saturating_add(crate::unicode::grapheme_count(replacements))
                            .saturating_sub(1),
                    );
                    if let Some(updated_line) = buffer.line_text(line_idx) {
                        let final_col =
                            crate::unicode::grapheme_to_char_col(&updated_line, final_grapheme);
                        buffer.set_cursor_char_col(line_idx, final_col);
                    }
                }
            }
            Self::PasteAfter { .. } | Self::PasteBefore { .. } => {
                // Intentional no-op: paste repeat is intercepted in repeat_last_change()
                // before execute() is called, because it needs Editor-level register access.
            }
            Self::OpenLine {
                above,
                inserted_text,
                options,
            } => {
                let options = options.normalized();
                let line_idx = buffer.cursor().line();
                let line_text = buffer.line_text(line_idx).unwrap_or_default();

                let existing_indent = leading_str(&line_text);
                let mut indent_width = leading_width(&line_text, options.tab_width);

                if !*above {
                    // Match `o` behavior: add one extra indent level after opening delimiters.
                    let trimmed =
                        line_text.trim_end_matches(|c: char| c == '\n' || c.is_whitespace());
                    if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
                        indent_width += options.shift_width;
                    }
                }
                let indent = if options.copy_indent
                    && (*above || indent_width == leading_width(&line_text, options.tab_width))
                {
                    existing_indent.to_string()
                } else {
                    options.encode_indent(indent_width)
                };

                if *above {
                    let text = format!("{}\n", indent);
                    buffer.insert_text_at(line_idx, CharCol::ZERO, &text);
                    buffer
                        .cursor_mut()
                        .set_position(line_idx, GraphemeCol(indent.chars().count()));
                } else {
                    // `line_text` strips terminators by design — use the raw
                    // vs content length asymmetry to detect one. Mirrors
                    // `insert_line_below` after the line_text migration.
                    let has_terminator =
                        buffer.line_raw_len(line_idx) > buffer.line_content_len(line_idx);
                    let (insert_pos, text) = if has_terminator {
                        ((line_idx + 1, CharCol::ZERO), format!("{}\n", indent))
                    } else {
                        let line_len = line_text.chars().count();
                        ((line_idx, CharCol(line_len)), format!("\n{}\n", indent))
                    };
                    buffer.insert_text_at(insert_pos.0, insert_pos.1, &text);
                    buffer
                        .cursor_mut()
                        .set_position(line_idx + 1, GraphemeCol(indent.chars().count()));
                }

                if inserted_text.is_empty() {
                    // Match insert-mode exit cleanup for `o/O<Esc>` on whitespace-only lines.
                    let current_line = buffer.cursor().line();
                    if let Some(line) = buffer.line_text(current_line) {
                        let line_wo_nl = line;
                        if !line_wo_nl.is_empty() && line_wo_nl.chars().all(|c| c.is_whitespace()) {
                            let whitespace_len = line_wo_nl.chars().count();
                            buffer.delete_range(
                                current_line,
                                CharCol::ZERO,
                                current_line,
                                CharCol(whitespace_len),
                            );
                            buffer
                                .cursor_mut()
                                .set_position(current_line, GraphemeCol(0));
                        }
                    }
                    return;
                }

                let line = buffer.cursor().line();
                let col = buffer.cursor_char_col();
                buffer.insert_text_at(line, col, inserted_text);

                // Position cursor at end of inserted text - 1 (Vim Esc behavior)
                let mut final_line = line;
                let mut final_col = col;
                for ch in inserted_text.chars() {
                    if ch == '\n' {
                        final_line += 1;
                        final_col = CharCol::ZERO;
                    } else {
                        final_col += 1;
                    }
                }
                final_col = final_col.saturating_sub(1);
                buffer.set_cursor_char_col(final_line, final_col);
            }
            Self::DeleteVisualChar {
                line_delta,
                offset_col,
            } => {
                let start_line = buffer.cursor().line();
                let start_col = buffer.cursor_char_col();
                let end_line = (start_line + line_delta).min(buffer.line_count().saturating_sub(1));
                let end_grapheme = if *line_delta == 0 {
                    buffer.cursor().col().0 + offset_col
                } else {
                    *offset_col
                };
                let end_text = buffer.line_text(end_line).unwrap_or_default();
                let end_col =
                    crate::unicode::grapheme_to_char_col(&end_text, GraphemeCol(end_grapheme));
                buffer.delete_range(start_line, start_col, end_line, end_col);
                buffer.set_cursor_char_col(start_line, start_col);
            }
            Self::DeleteVisualLine { line_count } => {
                let start_line = buffer.cursor().line();
                let end_line_exclusive = start_line + line_count;
                buffer.delete_range(start_line, CharCol::ZERO, end_line_exclusive, CharCol::ZERO);
                let new_line = start_line.min(buffer.line_count().saturating_sub(1));
                buffer.cursor_mut().set_position(new_line, GraphemeCol(0));
            }
            Self::DeleteVisualBlock { line_count, width } => {
                let start_line = buffer.cursor().line();
                let start_col = buffer.cursor_char_col();
                delete_block(buffer, start_line, start_col, *line_count, *width);
                set_cursor_on_char(buffer, start_line, start_col);
            }
            Self::VisualBlockInsert {
                line_count,
                delete_width,
                column,
                inserted_text,
            } => {
                let start_line = buffer.cursor().line();
                let start_col = buffer.cursor_char_col();
                delete_block(buffer, start_line, start_col, *line_count, *delete_width);
                let end_line = (start_line + line_count).min(buffer.line_count());
                column.offset_by(start_col.0).insert_on_lines(
                    buffer,
                    start_line..end_line,
                    inserted_text,
                );

                // vim: `c` leaves the cursor on the last inserted character of
                // the first line, `I` / `A` at the block's top-left corner.
                if *delete_width == 0 || inserted_text.is_empty() {
                    set_cursor_on_char(buffer, start_line, start_col);
                } else {
                    let mut final_line = start_line;
                    let mut final_col = start_col;
                    for ch in inserted_text.chars() {
                        if ch == '\n' {
                            final_line += 1;
                            final_col = CharCol::ZERO;
                        } else {
                            final_col += 1;
                        }
                    }
                    buffer.set_cursor_char_col(final_line, final_col.saturating_sub(1));
                }
            }
            Self::Change {
                delete,
                inserted_text,
                linewise,
            } => {
                match delete.as_ref() {
                    Self::DeleteTextObject { object_type }
                        if object_type.resolve_for_change(buffer).is_none() =>
                    {
                        return
                    }
                    Self::DeleteCharMotion {
                        target,
                        forward,
                        till,
                        count,
                    } if buffer
                        .char_motion_range(*target, *forward, *till, *count)
                        .is_none() =>
                    {
                        return
                    }
                    _ => {}
                }
                let line = buffer.cursor().line();
                let col = buffer.cursor().col();
                if *linewise {
                    // Resolve the range before deleting: normal-mode deletion
                    // clamps a last-line cursor onto the preceding line.
                    let (start, end) = match delete.as_ref() {
                        Self::DeleteLines { count } => (line, line + count),
                        Self::DeleteVisualLine { line_count } => (line, line + line_count),
                        Self::DeleteLineDown { count } => (line, line + count + 1),
                        Self::DeleteLineUp { count } => (line.saturating_sub(*count), line + 1),
                        Self::DeleteToLastLine { target_line }
                        | Self::DeleteToFirstLine { target_line } => {
                            (line.min(*target_line), line.max(*target_line) + 1)
                        }
                        _ => (line, line + 1),
                    };
                    buffer.change_lines(start, end.min(buffer.line_count()));
                } else {
                    let version = buffer.version();
                    if let Self::DeleteTextObject { object_type } = delete.as_ref() {
                        if let Some(range) = object_type.resolve_for_change(buffer) {
                            buffer.delete_range(
                                range.start_line,
                                range.start_col,
                                range.end_line,
                                range.end_col,
                            );
                            buffer.set_cursor_char_col(range.start_line, range.start_col);
                        }
                    } else {
                        delete.execute(buffer);
                    }
                    if version == buffer.version()
                        && matches!(
                            delete.as_ref(),
                            Self::ChangeToMatchingBracket | Self::DeleteSearchMatch { .. }
                        )
                    {
                        return;
                    }
                    // These normal-mode delete actions clamp an EOL insertion
                    // point. All other change actions resolve their own start,
                    // including backward motions and text objects.
                    if matches!(
                        delete.as_ref(),
                        Self::DeleteCharForward { .. } | Self::DeleteToEndOfLine
                    ) {
                        buffer.cursor_mut().set_position(line, col);
                    }
                }

                // Phase 2: Insert the captured text
                if !inserted_text.is_empty() {
                    let line = buffer.cursor().line();
                    let col = buffer.cursor_char_col();
                    buffer.insert_text_at(line, col, inserted_text);

                    // Position cursor at end of inserted text - 1 (Vim Esc behavior)
                    let text_chars: usize = inserted_text.chars().count();
                    if text_chars > 0 {
                        // Calculate final position by walking through inserted text
                        let mut final_line = line;
                        let mut final_col = col;
                        for ch in inserted_text.chars() {
                            if ch == '\n' {
                                final_line += 1;
                                final_col = CharCol::ZERO;
                            } else {
                                final_col += 1;
                            }
                        }
                        // Back up one (Vim positions cursor on last inserted char)
                        final_col = final_col.saturating_sub(1);
                        buffer.set_cursor_char_col(final_line, final_col);
                    }
                } else if *linewise {
                    let line = buffer.cursor().line();
                    let len = buffer.line_len(line);
                    buffer.delete_range(line, CharCol::ZERO, line, CharCol(len));
                    buffer.cursor_mut().set_col(GraphemeCol::ZERO);
                } else if buffer.cursor().col().0 > 0 {
                    buffer.cursor_mut().move_left(1);
                }
            }
            Self::InsertSession {
                count,
                entry_mode,
                origin_offset,
                edits,
            } => {
                // Step 1: reposition cursor per entry_mode, matching the
                // semantics that `Composite.repeat()` has today.
                match entry_mode {
                    InsertEntryMode::Insert => {}
                    InsertEntryMode::Append => {
                        buffer.cursor_mut().move_right(1);
                    }
                    InsertEntryMode::FirstNonBlank => {
                        let line_idx = buffer.cursor().line();
                        if let Some(line) = buffer.line_text(line_idx) {
                            let content = line;
                            let col = content
                                .chars()
                                .position(|c| !c.is_whitespace())
                                .unwrap_or(0);
                            buffer.set_cursor_char_col(line_idx, CharCol(col));
                        }
                    }
                    InsertEntryMode::EndOfLine => {
                        let line_idx = buffer.cursor().line();
                        if let Some(line) = buffer.line_text(line_idx) {
                            let line_len = line.chars().count();
                            buffer.set_cursor_char_col(line_idx, CharCol(line_len));
                        }
                    }
                    // o/O use RepeatAction::OpenLine, not InsertSession.
                    InsertEntryMode::OpenBelow | InsertEntryMode::OpenAbove => {}
                }

                // Step 2: translate each edit by (new_origin - origin_offset).
                // Absolute offsets in `edits` were captured against the
                // original session's rope — the delta re-anchors them to the
                // current rope without assuming anything about the content
                // between origin and edit.
                //
                // After each edit we also advance the cursor the same way
                // `Buffer::insert_text_at_positioning_cursor` /
                // `Buffer::delete_range_positioning_cursor` do during live
                // editing: insert lands cursor at end of inserted text,
                // delete lands cursor at start of range. Session-internal
                // cursor state matters because future edits in the same
                // session target offsets that assume the cursor moved this
                // way.
                for _ in 0..*count {
                    let new_origin_offset = {
                        let line = buffer.cursor().line();
                        let char_col = buffer.cursor_char_col();
                        buffer.rope().line_to_char(line) + char_col.0
                    };
                    let delta = new_origin_offset as i64 - *origin_offset as i64;

                    for edit in edits {
                        let new_offset = (edit.offset() as i64 + delta).max(0) as usize;
                        match edit {
                            Edit::Insert { text, .. } => {
                                Edit::Insert {
                                    offset: new_offset,
                                    text: text.clone(),
                                }
                                .apply(buffer);
                                let end = new_offset + text.chars().count();
                                let end = end.min(buffer.rope().len_chars());
                                let line = buffer.rope().char_to_line(end);
                                let col = end - buffer.rope().line_to_char(line);
                                buffer.set_cursor_char_col(line, CharCol(col));
                            }
                            Edit::Delete { text, .. } => {
                                Edit::Delete {
                                    offset: new_offset,
                                    text: text.clone(),
                                }
                                .apply(buffer);
                                let anchor = new_offset.min(buffer.rope().len_chars());
                                let line = buffer.rope().char_to_line(anchor);
                                let col = anchor - buffer.rope().line_to_char(line);
                                buffer.set_cursor_char_col(line, CharCol(col));
                            }
                        }
                    }
                }

                // Esc positions the cursor on the last inserted grapheme,
                // including single-character insert sessions.
                if buffer.cursor().col().0 > 0 {
                    buffer.cursor_mut().move_left(1);
                }
            }
        }
    }
}

/// Applies `gu` / `gU` / `g~` over `target`, starting at the cursor, and leaves
/// the cursor where it started. The caller wraps this in `buffer.record()`.
pub fn change_case(buffer: &mut Buffer, transform: CaseTransform, target: CaseTarget) {
    let start = (buffer.cursor().line(), buffer.cursor().col());
    match target {
        CaseTarget::Lines { count } => {
            let end_line = (start.0 + count).min(buffer.line_count());
            for line_idx in start.0..end_line {
                transform_line_range(buffer, line_idx, 0, usize::MAX, |text| {
                    transform.apply_to(text)
                });
            }
        }
        CaseTarget::WordForward { count } | CaseTarget::WordEnd { count } => {
            let inclusive = matches!(target, CaseTarget::WordEnd { .. });
            if inclusive {
                crate::editor::Motions::word_end_forward(buffer, count);
            } else {
                crate::editor::Motions::word_forward(buffer, count);
            }
            let end = (buffer.cursor().line(), buffer.cursor().col().0);
            // Inclusive motions land ON the last affected character.
            let end_col = if inclusive { end.1 + 1 } else { end.1 };
            transform_char_range(buffer, start.0, start.1 .0, end.0, end_col, |text| {
                transform.apply_to(text)
            });
        }
        CaseTarget::ToEndOfLine => {
            transform_line_range(buffer, start.0, start.1 .0, usize::MAX, |text| {
                transform.apply_to(text)
            });
        }
    }
    buffer.cursor_mut().set_position(start.0, start.1);
}

/// Rewrites graphemes `[from, to)` of one line (clamped to the line) with
/// `f(old text)`, skipping the edit when nothing changes.
fn transform_line_range(
    buffer: &mut Buffer,
    line_idx: usize,
    from: usize,
    to: usize,
    f: impl Fn(&str) -> String,
) {
    transform_char_range(buffer, line_idx, from, line_idx, to, f);
}

/// Rewrites the text between two grapheme positions (the end is exclusive and
/// clamped to its line) with `f(old text)`, skipping the edit when nothing changes.
fn transform_char_range(
    buffer: &mut Buffer,
    start_line: usize,
    start_grapheme: usize,
    end_line: usize,
    end_grapheme: usize,
    f: impl Fn(&str) -> String,
) {
    let (Some(start_text), Some(end_text)) =
        (buffer.line_text(start_line), buffer.line_text(end_line))
    else {
        return;
    };
    let start_col = crate::unicode::grapheme_to_char_col(
        &start_text,
        GraphemeCol(start_grapheme.min(crate::unicode::grapheme_count(&start_text))),
    );
    let end_col = crate::unicode::grapheme_to_char_col(
        &end_text,
        GraphemeCol(end_grapheme.min(crate::unicode::grapheme_count(&end_text))),
    );
    drop((start_text, end_text));
    let start = buffer.rope().line_to_char(start_line) + start_col.0;
    let end = buffer.rope().line_to_char(end_line) + end_col.0;
    if end <= start {
        return;
    }
    let text = buffer.rope().slice(start..end).to_string();
    let transformed = f(&text);
    if transformed != text {
        buffer.delete_range(start_line, start_col, end_line, end_col);
        buffer.insert_text_at(start_line, start_col, &transformed);
    }
}

/// Applies a Visual `u` / `U` / `~` / `r` to a selection of `shape`, whose
/// start is at the cursor. Leaves the cursor at the start, as vim does.
pub fn transform_visual_shape(buffer: &mut Buffer, shape: VisualShape, transform: VisualTransform) {
    let start_line = buffer.cursor().line();
    let start_grapheme = buffer.cursor().col().0;
    let last_line = buffer.line_count().saturating_sub(1);
    let f = |text: &str| transform.apply_to(text);
    match shape {
        VisualShape::Char {
            line_delta,
            offset_col,
        } => {
            let end_line = (start_line + line_delta).min(last_line);
            let end_grapheme = if line_delta == 0 {
                start_grapheme + offset_col
            } else {
                offset_col
            };
            transform_char_range(
                buffer,
                start_line,
                start_grapheme,
                end_line,
                end_grapheme,
                f,
            );
        }
        VisualShape::Line { line_count } => {
            let end_line = (start_line + line_count).min(buffer.line_count());
            for line_idx in start_line..end_line {
                transform_line_range(buffer, line_idx, 0, usize::MAX, f);
            }
        }
        VisualShape::Block { line_count, width } => {
            let end_line = (start_line + line_count).min(buffer.line_count());
            for line_idx in start_line..end_line {
                let end = start_grapheme.saturating_add(width);
                transform_line_range(buffer, line_idx, start_grapheme, end, f);
            }
        }
    }
    buffer
        .cursor_mut()
        .set_position(start_line, GraphemeCol(start_grapheme));
}

/// Where a visual-block `I` / `A` / `c` puts its text on each block line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockColumn {
    /// `I` / `c`: at this column; lines shorter than it are left alone.
    Insert(usize),
    /// `A`: at this column, padding shorter lines with spaces.
    Append(usize),
    /// `$A`: at the end of each line.
    EndOfLine,
}

impl BlockColumn {
    /// The same placement shifted right by `base` columns.
    pub fn offset_by(self, base: usize) -> Self {
        match self {
            Self::Insert(col) => Self::Insert(col + base),
            Self::Append(col) => Self::Append(col + base),
            Self::EndOfLine => Self::EndOfLine,
        }
    }

    /// Inserts `text` on each of `lines` (char-space columns).
    pub fn insert_on_lines(self, buffer: &mut Buffer, lines: std::ops::Range<usize>, text: &str) {
        if text.is_empty() {
            return;
        }
        for line in lines {
            let Some(len) = buffer.line_text(line).map(|l| l.chars().count()) else {
                continue;
            };
            match self {
                Self::Insert(col) if col <= len => buffer.insert_text_at(line, CharCol(col), text),
                Self::Insert(_) => {}
                Self::Append(col) if col <= len => buffer.insert_text_at(line, CharCol(col), text),
                Self::Append(col) => {
                    let padded = format!("{}{text}", " ".repeat(col - len));
                    buffer.insert_text_at(line, CharCol(len), &padded);
                }
                Self::EndOfLine => buffer.insert_text_at(line, CharCol(len), text),
            }
        }
    }
}

/// Deletes `width` columns from `start_col` on `line_count` lines.
fn delete_block(
    buffer: &mut Buffer,
    start_line: usize,
    start_col: CharCol,
    line_count: usize,
    width: usize,
) {
    if width == 0 {
        return;
    }
    let end_line = (start_line + line_count).min(buffer.line_count());
    for line in start_line..end_line {
        let len = buffer.line_text(line).map_or(0, |l| l.chars().count());
        if start_col.0 < len {
            let end_col = (start_col + width).min_usize(len);
            buffer.delete_range(line, start_col, line, end_col);
        }
    }
}

/// Puts the cursor at `col` on `line`, clamped to the last character.
fn set_cursor_on_char(buffer: &mut Buffer, line: usize, col: CharCol) {
    let len = buffer.line_text(line).map_or(0, |l| l.chars().count());
    buffer.set_cursor_char_col(line, col.min_usize(len.saturating_sub(1)));
}

#[cfg(test)]
mod insert_session_tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::unicode::GraphemeCol;

    fn set_cursor(buf: &mut Buffer, line: usize, col: usize) {
        buf.cursor_mut().set_position(line, GraphemeCol(col));
    }

    fn insert_session(
        entry_mode: InsertEntryMode,
        origin_offset: usize,
        edits: Vec<Edit>,
    ) -> RepeatAction {
        RepeatAction::InsertSession {
            count: 1,
            entry_mode,
            origin_offset,
            edits,
        }
    }

    #[test]
    fn replay_translates_offsets_by_delta() {
        // Session ran at origin 0 with "foo" typed as three single-char inserts.
        let action = insert_session(
            InsertEntryMode::Insert,
            0,
            vec![
                Edit::Insert {
                    offset: 0,
                    text: "f".into(),
                },
                Edit::Insert {
                    offset: 1,
                    text: "o".into(),
                },
                Edit::Insert {
                    offset: 2,
                    text: "o".into(),
                },
            ],
        );

        let mut buf = Buffer::new_from_str("abcde\n");
        set_cursor(&mut buf, 0, 2);
        action.execute(&mut buf);

        assert_eq!(buf.rope().to_string(), "abfoocde\n");
    }

    #[test]
    fn replay_preserves_session_internal_geometry() {
        // Session ran at origin 100 in some large buffer. Replay at 3 in a
        // small buffer: behavior must depend only on the session's internal
        // offsets, not on what was between origin and the edits originally.
        let action = insert_session(
            InsertEntryMode::Insert,
            100,
            vec![
                Edit::Insert {
                    offset: 100,
                    text: "a".into(),
                },
                Edit::Insert {
                    offset: 101,
                    text: "b".into(),
                },
                Edit::Insert {
                    offset: 102,
                    text: "c".into(),
                },
            ],
        );

        let mut buf = Buffer::new_from_str("xxxxxx\n");
        set_cursor(&mut buf, 0, 3);
        action.execute(&mut buf);

        assert_eq!(buf.rope().to_string(), "xxxabcxxx\n");
    }

    #[test]
    fn replay_handles_backspace_across_origin() {
        // Session origin is at start of line 1 (offset 4 in "aaa\nbbb\n").
        // User hit BS, recorded as Delete @ offset 3 (one before origin) of "\n".
        let action = insert_session(
            InsertEntryMode::Insert,
            4,
            vec![Edit::Delete {
                offset: 3,
                text: "\n".into(),
            }],
        );

        let mut buf = Buffer::new_from_str("aaa\nbbb\n");
        set_cursor(&mut buf, 1, 0);
        action.execute(&mut buf);

        // Lines joined.
        assert_eq!(buf.rope().to_string(), "aaabbb\n");
    }

    #[test]
    fn replay_intra_session_cursor_movement_is_correct() {
        // Session: typed "foo", arrow-left twice, typed "x".
        // Recorded edits: Insert@N "f", Insert@N+1 "o", Insert@N+2 "o",
        // Insert@N+1 "x" (after cursor moved back into the word).
        // Net text inserted: "fxoo".
        let action = insert_session(
            InsertEntryMode::Insert,
            0,
            vec![
                Edit::Insert {
                    offset: 0,
                    text: "f".into(),
                },
                Edit::Insert {
                    offset: 1,
                    text: "o".into(),
                },
                Edit::Insert {
                    offset: 2,
                    text: "o".into(),
                },
                Edit::Insert {
                    offset: 1,
                    text: "x".into(),
                },
            ],
        );

        let mut buf = Buffer::new_from_str("[]\n");
        set_cursor(&mut buf, 0, 1);
        action.execute(&mut buf);

        assert_eq!(buf.rope().to_string(), "[fxoo]\n");
    }

    #[test]
    fn replay_insert_then_delete_net_zero() {
        // Session: typed "ab", then BS twice.
        let action = insert_session(
            InsertEntryMode::Insert,
            0,
            vec![
                Edit::Insert {
                    offset: 0,
                    text: "a".into(),
                },
                Edit::Insert {
                    offset: 1,
                    text: "b".into(),
                },
                Edit::Delete {
                    offset: 1,
                    text: "b".into(),
                },
                Edit::Delete {
                    offset: 0,
                    text: "a".into(),
                },
            ],
        );

        let mut buf = Buffer::new_from_str("xyz\n");
        set_cursor(&mut buf, 0, 2);
        action.execute(&mut buf);

        // Net effect: nothing inserted, but cursor positioned as if at origin.
        assert_eq!(buf.rope().to_string(), "xyz\n");
    }

    #[test]
    fn append_entry_mode_moves_cursor_right_before_replay() {
        // Append mode shifts cursor right by 1 before the insert, matching
        // how `a` starts insert mode one character past the cursor.
        let action = insert_session(
            InsertEntryMode::Append,
            3,
            vec![Edit::Insert {
                offset: 3,
                text: "X".into(),
            }],
        );

        let mut buf = Buffer::new_from_str("abcde\n");
        set_cursor(&mut buf, 0, 1); // on 'b'; Append moves to col 2 ('c').
        action.execute(&mut buf);

        assert_eq!(buf.rope().to_string(), "abXcde\n");
    }

    #[test]
    fn first_non_blank_entry_mode_jumps_before_replay() {
        let action = insert_session(
            InsertEntryMode::FirstNonBlank,
            2,
            vec![Edit::Insert {
                offset: 2,
                text: "!".into(),
            }],
        );

        let mut buf = Buffer::new_from_str("    hello\n");
        set_cursor(&mut buf, 0, 7);
        action.execute(&mut buf);

        // FirstNonBlank: cursor jumps to col 4 ('h'). Insert "!" at 4.
        assert_eq!(buf.rope().to_string(), "    !hello\n");
    }

    #[test]
    fn end_of_line_entry_mode_jumps_to_end_before_replay() {
        let action = insert_session(
            InsertEntryMode::EndOfLine,
            3,
            vec![Edit::Insert {
                offset: 3,
                text: "!".into(),
            }],
        );

        let mut buf = Buffer::new_from_str("abc\n");
        set_cursor(&mut buf, 0, 0);
        action.execute(&mut buf);

        // EndOfLine: cursor jumps to col 3 (past 'c'). Insert "!".
        assert_eq!(buf.rope().to_string(), "abc!\n");
    }

    #[test]
    fn multiline_insert_preserves_shape() {
        // Session: typed "a", Enter, "b" — each as a separate edit.
        let action = insert_session(
            InsertEntryMode::Insert,
            0,
            vec![
                Edit::Insert {
                    offset: 0,
                    text: "a".into(),
                },
                Edit::Insert {
                    offset: 1,
                    text: "\n".into(),
                },
                Edit::Insert {
                    offset: 2,
                    text: "b".into(),
                },
            ],
        );

        let mut buf = Buffer::new_from_str("xx\n");
        set_cursor(&mut buf, 0, 1);
        action.execute(&mut buf);

        assert_eq!(buf.rope().to_string(), "xa\nbx\n");
    }
}
