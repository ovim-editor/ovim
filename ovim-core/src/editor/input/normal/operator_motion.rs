//! Motions after an operator that have no hand-written (operator, motion) arm.
//!
//! A [`Motion`] moves the cursor to its target and says how vim classifies the
//! range it spans ([`Reach`]); [`apply_between`] then applies any operator to that
//! classified range uniformly, so each new motion is one enum variant instead of
//! a handler per operator.

use crate::editor::input::char_motion::{
    apply_charwise_operator, apply_linewise_operator, OperatorRange,
};
use crate::editor::input::{case, helpers};
use crate::editor::{CursorPos, Editor, FindDirection, Motions, Operator};
use crate::motion_range::{MotionRange, Wise};
use crate::repeat_action::CaseTransform;
use crate::unicode::{CharCol, GraphemeCol};
use crate::KeyCode;
use anyhow::Result;

/// How the range from the cursor to a motion's target is classified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor::input) enum Reach {
    /// The target character is not part of the range (`:help exclusive`).
    Exclusive,
    /// The target character is part of the range.
    Inclusive,
    /// Whole lines from the cursor line to the target line.
    Linewise,
}

/// A cursor motion usable as the target of an operator.
#[derive(Clone, Copy, Debug)]
pub(super) enum Motion {
    Left,
    Right,
    Down,
    Up,
    WordForward {
        big: bool,
    },
    WordBackward {
        big: bool,
    },
    WordEnd {
        big: bool,
    },
    WordEndBackward {
        big: bool,
    },
    LineStart,
    FirstNonBlank,
    LineEnd,
    /// `_`: the first non-blank `count - 1` lines down.
    FirstNonBlankDown,
    /// `+` / `-`: the first non-blank `count` lines down / up.
    NextLine,
    PreviousLine,
    /// `|`: column `count`.
    Column,
    ParagraphForward,
    ParagraphBackward,
    SentenceForward,
    SentenceBackward,
    MatchingBracket,
    SearchNext,
    SearchPrevious,
    /// `;` / `,`: repeat the last `f`/`t`/`F`/`T`.
    FindRepeat {
        reverse: bool,
    },
}

impl Motion {
    /// The motion a key stands for after an operator (`g_prefix`: after `g`).
    pub(super) fn from_key(code: KeyCode, g_prefix: bool) -> Option<Self> {
        Some(match (g_prefix, code) {
            (true, KeyCode::Char('e')) => Self::WordEndBackward { big: false },
            (true, KeyCode::Char('E')) => Self::WordEndBackward { big: true },
            (true, _) => return None,
            (false, KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace) => Self::Left,
            (false, KeyCode::Char('l' | ' ') | KeyCode::Right) => Self::Right,
            (false, KeyCode::Char('j') | KeyCode::Down) => Self::Down,
            (false, KeyCode::Char('k') | KeyCode::Up) => Self::Up,
            (false, KeyCode::Char('w')) => Self::WordForward { big: false },
            (false, KeyCode::Char('W')) => Self::WordForward { big: true },
            (false, KeyCode::Char('b')) => Self::WordBackward { big: false },
            (false, KeyCode::Char('B')) => Self::WordBackward { big: true },
            (false, KeyCode::Char('e')) => Self::WordEnd { big: false },
            (false, KeyCode::Char('E')) => Self::WordEnd { big: true },
            (false, KeyCode::Char('0')) => Self::LineStart,
            (false, KeyCode::Char('^')) => Self::FirstNonBlank,
            (false, KeyCode::Char('$')) => Self::LineEnd,
            (false, KeyCode::Char('_')) => Self::FirstNonBlankDown,
            (false, KeyCode::Char('+') | KeyCode::Enter) => Self::NextLine,
            (false, KeyCode::Char('-')) => Self::PreviousLine,
            (false, KeyCode::Char('|')) => Self::Column,
            (false, KeyCode::Char('}')) => Self::ParagraphForward,
            (false, KeyCode::Char('{')) => Self::ParagraphBackward,
            (false, KeyCode::Char(')')) => Self::SentenceForward,
            (false, KeyCode::Char('(')) => Self::SentenceBackward,
            (false, KeyCode::Char('%')) => Self::MatchingBracket,
            (false, KeyCode::Char('n')) => Self::SearchNext,
            (false, KeyCode::Char('N')) => Self::SearchPrevious,
            (false, KeyCode::Char(';')) => Self::FindRepeat { reverse: false },
            (false, KeyCode::Char(',')) => Self::FindRepeat { reverse: true },
            _ => return None,
        })
    }

    fn reach(self, editor: &Editor) -> Reach {
        match self {
            Self::Down
            | Self::Up
            | Self::FirstNonBlankDown
            | Self::NextLine
            | Self::PreviousLine => Reach::Linewise,
            Self::WordEnd { .. }
            | Self::WordEndBackward { .. }
            | Self::LineEnd
            | Self::MatchingBracket => Reach::Inclusive,
            Self::ParagraphForward if Motions::paragraph_end_is_inclusive(editor.buffer()) => {
                Reach::Inclusive
            }
            // The repeated find is inclusive going forward, exclusive going back.
            Self::FindRepeat { reverse } => match editor.get_last_find().map(|find| find.2) {
                Some(direction) if (direction == FindDirection::Forward) != reverse => {
                    Reach::Inclusive
                }
                _ => Reach::Exclusive,
            },
            _ => Reach::Exclusive,
        }
    }

    /// Moves the cursor to the target of the motion; `false` when the motion
    /// fails (vim then cancels the operator).
    fn apply(self, editor: &mut Editor, count: usize) -> bool {
        let before = editor.cursor_position();
        match self {
            Self::Left => {
                let col = before.col.0.saturating_sub(count);
                editor.buffer_mut().cursor_mut().set_col(GraphemeCol(col));
                return col != before.col.0;
            }
            Self::Right => {
                // Past the last character: an operator may take it.
                let len = editor.buffer().line_index(before.line).grapheme_count();
                let col = (before.col.0 + count).min(len);
                editor.buffer_mut().cursor_mut().set_col(GraphemeCol(col));
                return col != before.col.0;
            }
            Self::Down | Self::Up => {
                let last = editor.buffer().line_count().saturating_sub(1);
                let line = if matches!(self, Self::Down) {
                    (before.line + count).min(last)
                } else {
                    before.line.saturating_sub(count)
                };
                editor.buffer_mut().cursor_mut().set_line(line);
                return line != before.line;
            }
            Self::WordForward { big } => {
                // With an operator the last word stops at the end of its line, unless
                // it started on an empty line: then the range reaches the next
                // word and the exclusive rules (`MotionRange`) classify it.
                let buf = editor.buffer_mut();
                let (line, end) = buf.operator_word_forward_end(count, big);
                let natural_line = buf.cursor().line();
                if !(line < natural_line && buf.line_len(line) == 0) {
                    let col = buf.line_index(line).char_to_grapheme(end);
                    buf.cursor_mut().set_position(line, col);
                }
            }
            Self::WordBackward { big } => {
                let buf = editor.buffer_mut();
                if big {
                    Motions::word_backward_big(buf, count)
                } else {
                    Motions::word_backward(buf, count)
                }
            }
            Self::WordEnd { big } => {
                let buf = editor.buffer_mut();
                if big {
                    Motions::word_end_forward_big(buf, count)
                } else {
                    Motions::word_end_forward(buf, count)
                }
            }
            Self::WordEndBackward { big } => {
                let buf = editor.buffer_mut();
                if big {
                    Motions::word_end_backward_big(buf, count)
                } else {
                    Motions::word_end_backward(buf, count)
                }
            }
            Self::LineStart => editor.buffer_mut().cursor_mut().set_col(GraphemeCol::ZERO),
            Self::FirstNonBlank => Motions::first_non_blank(editor.buffer_mut()),
            Self::LineEnd => {
                let last = editor.buffer().line_count().saturating_sub(1);
                let line = (before.line + count - 1).min(last);
                let len = editor.buffer().line_index(line).grapheme_count();
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(line, GraphemeCol(len.saturating_sub(1)));
            }
            Self::FirstNonBlankDown => {
                let last = editor.buffer().line_count().saturating_sub(1);
                if count > 1 && before.line >= last {
                    return false;
                }
                let line = (before.line + count - 1).min(last);
                editor.buffer_mut().cursor_mut().set_line(line);
                Motions::first_non_blank(editor.buffer_mut());
            }
            Self::NextLine => Motions::plus_motion(editor.buffer_mut(), count),
            Self::PreviousLine => Motions::minus_motion(editor.buffer_mut(), count),
            Self::Column => {
                let len = editor.buffer().line_index(before.line).grapheme_count();
                let col = (count - 1).min(len.saturating_sub(1));
                editor.buffer_mut().cursor_mut().set_col(GraphemeCol(col));
            }
            Self::ParagraphForward => {
                if !Motions::paragraph_forward(editor.buffer_mut(), count) {
                    return false;
                }
            }
            Self::ParagraphBackward => {
                if !Motions::paragraph_backward(editor.buffer_mut(), count) {
                    return false;
                }
            }
            Self::SentenceForward => Motions::sentence_forward(editor.buffer_mut(), count),
            Self::SentenceBackward => Motions::sentence_backward(editor.buffer_mut(), count),
            Self::MatchingBracket => {
                return Motions::jump_to_matching_bracket(editor.buffer_mut());
            }
            Self::SearchNext | Self::SearchPrevious => {
                let reverse = matches!(self, Self::SearchPrevious);
                let Some((line, col)) = editor.search_target(reverse, count) else {
                    return false;
                };
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(line, GraphemeCol(col));
            }
            Self::FindRepeat { reverse } => {
                // Reads the repeat count itself.
                editor.set_count(count);
                return editor.repeat_last_find(reverse);
            }
        }
        // These fail (vim beeps) when there is nowhere to go: no word before the
        // first one, no line after the last.
        let fails_in_place = matches!(
            self,
            Self::WordBackward { .. }
                | Self::WordEnd { .. }
                | Self::WordEndBackward { .. }
                | Self::ParagraphBackward
                | Self::SentenceBackward
                | Self::NextLine
                | Self::PreviousLine
        );
        !fails_in_place || editor.cursor_position() != before
    }
}

/// Applies `operator` over the range from the cursor to `motion`'s target. The
/// cursor is left where the operator puts it (the start of the range).
pub(super) fn apply_motion_operator(
    editor: &mut Editor,
    operator: Operator,
    motion: Motion,
    count: usize,
) -> Result<()> {
    let start = editor.cursor_position();
    let moved = motion.apply(editor, count);
    // After the motion: where a `}` ended decides whether it is inclusive.
    let reach = motion.reach(editor);
    let target = editor.cursor_position();
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start.line, start.col);
    editor.clear_count();
    if !moved {
        return Ok(());
    }
    apply_between(editor, operator, start, target, reach)
}

/// Applies `operator` to the range between two positions in `reach` terms.
pub(in crate::editor::input) fn apply_between(
    editor: &mut Editor,
    operator: Operator,
    start: CursorPos,
    target: CursorPos,
    reach: Reach,
) -> Result<()> {
    if reach == Reach::Linewise {
        let (first, last) = (start.line.min(target.line), start.line.max(target.line));
        return apply_lines(editor, operator, first, last, start);
    }

    let ((from_line, from_col), (to_line, mut to_col)) = {
        let a = (start.line, start.col.0);
        let b = (target.line, target.col.0);
        if a <= b {
            (a, b)
        } else {
            (b, a)
        }
    };
    if reach == Reach::Inclusive {
        to_col += 1;
    }

    // Vim's exclusive adjustments (`:help exclusive`) work on character columns.
    let buffer = editor.buffer();
    let char_col = |line: usize, grapheme: usize| {
        buffer
            .line_index(line)
            .grapheme_to_char(GraphemeCol(grapheme))
    };
    let range = MotionRange::from_exclusive(
        buffer,
        (from_line, char_col(from_line, from_col)),
        (to_line, char_col(to_line, to_col)),
    );
    if range.wise == Wise::Linewise {
        return apply_lines(editor, operator, range.start.0, range.end.0, start);
    }
    let grapheme_col =
        |(line, col): (usize, CharCol)| (line, buffer.line_index(line).char_to_grapheme(col).0);
    let (from, to) = (grapheme_col(range.start), grapheme_col(range.end));
    apply_chars(editor, operator, from, to, start)
}

fn apply_chars(
    editor: &mut Editor,
    operator: Operator,
    from: (usize, usize),
    to: (usize, usize),
    cursor_before: CursorPos,
) -> Result<()> {
    match operator {
        Operator::Delete | Operator::Change | Operator::Yank => {
            apply_charwise_operator(
                editor,
                operator,
                cursor_before,
                OperatorRange::exclusive(from, to),
                None,
            );
        }
        Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
            case::change_case_range(editor, case_transform(operator), from, to);
        }
        // Indenting and folding work on the lines the range touches.
        Operator::Indent | Operator::Dedent | Operator::AutoIndent | Operator::Fold => {
            let last = if to.1 == 0 && to.0 > from.0 {
                to.0 - 1
            } else {
                to.0
            };
            apply_lines(editor, operator, from.0, last, cursor_before)?;
        }
    }
    Ok(())
}

/// Applies `operator` to whole lines `first..=last`.
pub(super) fn apply_lines(
    editor: &mut Editor,
    operator: Operator,
    first: usize,
    last: usize,
    cursor_before: CursorPos,
) -> Result<()> {
    match operator {
        Operator::Delete | Operator::Change | Operator::Yank => {
            apply_linewise_operator(editor, operator, first, last);
            if operator == Operator::Yank {
                // A yank leaves the cursor on the first line of the range.
                let col = cursor_before.col.0.min(
                    editor
                        .buffer()
                        .line_index(first)
                        .grapheme_count()
                        .saturating_sub(1),
                );
                editor
                    .buffer_mut()
                    .cursor_mut()
                    .set_position(first, GraphemeCol(col));
            }
        }
        Operator::Indent => {
            helpers::indent_lines_with_tracking(editor, first, last + 1, cursor_before)?
        }
        Operator::Dedent => {
            helpers::dedent_lines_with_tracking(editor, first, last + 1, cursor_before)?
        }
        Operator::AutoIndent => {
            let options = editor.indent_options();
            helpers::auto_indent_lines_with_tracking(editor, first, last + 1, options)?;
            // The cursor ends on the first line of the range, like `>`.
            let first_non_blank = editor.buffer().first_non_blank_col(first);
            editor
                .buffer_mut()
                .set_cursor_char_col(first, first_non_blank);
        }
        Operator::Fold => {
            editor
                .buffer_mut()
                .fold_manager_mut()
                .create_fold(first, last);
        }
        Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
            let len =
                |editor: &Editor, line: usize| editor.buffer().line_index(line).grapheme_count();
            let last_len = len(editor, last);
            case::change_case_range(
                editor,
                case_transform(operator),
                (first, 0),
                (last, last_len),
            );
            let col = cursor_before
                .col
                .0
                .min(len(editor, first).saturating_sub(1));
            editor
                .buffer_mut()
                .cursor_mut()
                .set_position(first, GraphemeCol(col));
        }
    }
    Ok(())
}

fn case_transform(operator: Operator) -> CaseTransform {
    match operator {
        Operator::Lowercase => CaseTransform::Lower,
        Operator::Uppercase => CaseTransform::Upper,
        _ => CaseTransform::Toggle,
    }
}

/// Finishes `d/pat<CR>` and friends: the search has moved the cursor to the
/// match; apply the operator from where the search began.
pub(in crate::editor::input) fn finish_operator_search(
    editor: &mut Editor,
    operator: Operator,
    origin: (usize, usize),
) -> Result<()> {
    let target = editor.cursor_position();
    let start = CursorPos::new(origin.0, GraphemeCol(origin.1));
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(start.line, start.col);
    apply_between(editor, operator, start, target, Reach::Exclusive)
}
