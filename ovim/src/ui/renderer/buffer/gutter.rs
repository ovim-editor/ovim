//! The gutter: blame column, fold column, line numbers and signs
//! (breakpoints, diagnostics, git, walkthrough focus).

use crate::editor::Editor;
use crate::syntax::Theme;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::ui::renderer::layout::{GUTTER_SPACING, SIGN_WIDTH};
use crate::ui::renderer::styles::{
    blame_color_for_hash, blame_style, get_diagnostic_sign_style, get_git_sign_style,
    get_line_number_style,
};

/// Bracket character for blame grouping
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum BlameBracket {
    /// Single-line commit (no bracket)
    None,
    /// First line of a multi-line group
    Top,
    /// Middle line of a multi-line group
    Mid,
    /// Last line of a multi-line group
    Bottom,
}

/// Pre-computes blame bracket characters for visible lines.
/// Returns a vec of (bracket, hash, author, color) for each line in the range.
fn compute_blame_brackets(
    blame: &crate::GitBlame,
    start_line: usize,
    end_line: usize,
    author_width: usize,
) -> Vec<BlameRow> {
    let mut result = Vec::with_capacity(end_line.saturating_sub(start_line));

    for line_idx in start_line..end_line {
        if let Some(info) = blame.get(line_idx) {
            let hash = &info.commit_hash;
            let color = blame_color_for_hash(hash);

            // Check if prev/next lines have the same commit
            let same_as_prev = line_idx > 0
                && blame
                    .get(line_idx - 1)
                    .map(|p| p.commit_hash == *hash)
                    .unwrap_or(false);
            let same_as_next = blame
                .get(line_idx + 1)
                .map(|n| n.commit_hash == *hash)
                .unwrap_or(false);

            let bracket = match (same_as_prev, same_as_next) {
                (false, false) => BlameBracket::None,
                (false, true) => BlameBracket::Top,
                (true, true) => BlameBracket::Mid,
                (true, false) => BlameBracket::Bottom,
            };

            // Truncate author to fit
            let author: String = info.author.chars().take(author_width).collect();

            result.push((bracket, hash.clone(), author, color));
        } else {
            result.push((
                BlameBracket::None,
                String::new(),
                String::new(),
                Color::DarkGray,
            ));
        }
    }

    result
}

/// One visible line's blame gutter entry: bracket, short hash, author, hash colour.
pub(super) type BlameRow = (BlameBracket, String, String, Color);

/// Pre-computes the blame rows for the visible lines, if blame is shown.
pub(super) fn visible_blame_brackets(
    buffer: &crate::buffer::Buffer,
    blame_width: usize,
    start_line: usize,
    end_line: usize,
) -> Option<Vec<BlameRow>> {
    if blame_width > 0 {
        if let Some(blame) = buffer.git_blame() {
            let author_width = blame_width.saturating_sub(1 + 1 + 5 + 1 + 1); // bracket+sp+hash+sp+trailing_sp
            Some(compute_blame_brackets(
                blame,
                start_line,
                end_line,
                author_width,
            ))
        } else {
            None
        }
    } else {
        None
    }
}

/// Invariant context for gutter rendering within a single render pass.
/// Constructed once before the line loop and passed to all `build_gutter_line` calls.
pub(super) struct GutterContext<'a> {
    pub(super) editor: &'a Editor,
    pub(super) buffer: &'a crate::buffer::Buffer,
    pub(super) theme: &'a Theme,
    pub(super) line_num_width: usize,
    pub(super) cursor_line: usize,
    pub(super) blame_width: usize,
    pub(super) fold_width: usize,
    pub(super) walkthrough_range: Option<(usize, usize)>,
}

pub(super) const WALKTHROUGH_SELECTION_BG: Color = Color::Rgb(34, 57, 76);
const WALKTHROUGH_GUTTER_FG: Color = Color::Rgb(96, 176, 255);

pub(super) fn line_is_in_walkthrough(range: Option<(usize, usize)>, line_idx: usize) -> bool {
    range.is_some_and(|(start, end)| line_idx >= start && line_idx <= end)
}

/// Blank gutter for wrap continuation rows; the blame column keeps its colour.
fn continuation_gutter_line(ctx: &GutterContext, blame_info: Option<&BlameRow>) -> Line<'static> {
    let width = ctx.blame_width + ctx.fold_width + SIGN_WIDTH + ctx.line_num_width + GUTTER_SPACING;
    if ctx.blame_width > 0 {
        if let Some((_, _, _, color)) = blame_info {
            return Line::from(vec![
                Span::styled(" ".repeat(ctx.blame_width), blame_style(*color, ctx.theme)),
                Span::raw(" ".repeat(width - ctx.blame_width)),
            ]);
        }
    }
    Line::from(" ".repeat(width))
}

/// Blame column: branch bracket, plus hash and author on the first line of a
/// commit's group.
fn blame_gutter_span(
    blame_width: usize,
    blame_info: Option<&BlameRow>,
    theme: &Theme,
) -> Span<'static> {
    if let Some((bracket, hash, author, color)) = blame_info {
        let bracket_ch = match bracket {
            BlameBracket::None => ' ',
            BlameBracket::Top => '╭',
            BlameBracket::Mid => '│',
            BlameBracket::Bottom => '╰',
        };

        // Show hash+author only on first line of group or single lines
        let show_info = *bracket == BlameBracket::None || *bracket == BlameBracket::Top;
        let content_width = blame_width - 2; // minus bracket + leading space

        let text = if show_info && !hash.is_empty() {
            let info_str = format!("{} {}", hash, author);
            format!(
                "{} {:content_width$}",
                bracket_ch,
                info_str,
                content_width = content_width
            )
        } else {
            format!(
                "{} {:content_width$}",
                bracket_ch,
                "",
                content_width = content_width
            )
        };

        // Truncate to blame_width
        let text: String = text.chars().take(blame_width).collect();
        Span::styled(text, blame_style(*color, theme))
    } else {
        Span::raw(" ".repeat(blame_width))
    }
}

/// Fold column: `-` heads an open fold, `+` a closed one, `|` inside.
fn fold_gutter_span(editor: &Editor, line_idx: usize, fold_width: usize) -> Span<'static> {
    let cells: String = editor
        .fold_gutter_cells(line_idx, fold_width)
        .into_iter()
        .map(|mark| mark.glyph())
        .collect();
    let has_mark = cells.chars().any(|c| c != ' ');
    Span::styled(
        cells,
        Style::default().fg(if has_mark {
            Color::DarkGray
        } else {
            Color::Reset
        }),
    )
}

fn line_number_text(ctx: &GutterContext, line_idx: usize) -> String {
    let editor = ctx.editor;
    let line_num_width = ctx.line_num_width;
    if editor.options.relative_number {
        let rel = if line_idx == ctx.cursor_line {
            line_idx + 1
        } else {
            line_idx.abs_diff(ctx.cursor_line)
        };
        format!("{:>width$} ", rel, width = line_num_width)
    } else if editor.options.number {
        format!("{:>width$} ", line_idx + 1, width = line_num_width)
    } else {
        "  ".to_string()
    }
}

/// Picks the sign glyph and colour for a line.
/// Sign priority: breakpoint+exec > breakpoint > execution line > diagnostics >
/// walkthrough focus > agent edits > git. The walkthrough marker makes the
/// explained block visible even on blank or very short lines.
fn gutter_sign(
    ctx: &GutterContext,
    line_idx: usize,
    line_diagnostics: &[lsp_types::Diagnostic],
) -> (&'static str, Color) {
    let editor = ctx.editor;
    let buffer = ctx.buffer;
    let line_1based = (line_idx + 1) as u64;
    let breakpoint = editor.breakpoint_marker_at(line_1based);
    let has_breakpoint = breakpoint.is_some();
    let is_exec_line = editor.execution_line_in_current_buffer() == Some(line_1based);

    let buffer_id = buffer.id();
    let is_agent_edit = editor
        .ai_chat_state()
        .map(|c| c.agent_edits.is_line_modified(buffer_id, line_idx))
        .unwrap_or(false);
    let is_walkthrough_line = line_is_in_walkthrough(ctx.walkthrough_range, line_idx);

    use ovim_core::editor::BreakpointMarker;
    let (bp_glyph, bp_exec_glyph, bp_color) = match breakpoint {
        Some(BreakpointMarker::Disabled) => ("○ ", "○▶", Color::DarkGray),
        Some(BreakpointMarker::Conditional) => ("◆ ", "◆▶", Color::Red),
        _ => ("● ", "●▶", Color::Red),
    };
    if has_breakpoint && is_exec_line {
        (bp_exec_glyph, bp_color)
    } else if has_breakpoint {
        (bp_glyph, bp_color)
    } else if is_exec_line {
        ("▶ ", Color::Yellow)
    } else if !line_diagnostics.is_empty() {
        let severity = line_diagnostics[0].severity;
        get_diagnostic_sign_style(severity)
    } else if is_walkthrough_line {
        ("▎ ", WALKTHROUGH_GUTTER_FG)
    } else if is_agent_edit {
        ("▎ ", Color::Rgb(82, 139, 255))
    } else {
        let git_status = buffer.git_status().get_line_status(line_idx);
        get_git_sign_style(git_status)
    }
}

/// Builds a gutter line for a logical line (blame, fold column, line number
/// and sign).
/// If `is_continuation` is true, produces a blank gutter row.
/// Diagnostic signs take priority over git signs when both are present.
pub(super) fn build_gutter_line(
    ctx: &GutterContext,
    line_idx: usize,
    is_continuation: bool,
    line_diagnostics: &[lsp_types::Diagnostic],
    blame_info: Option<&BlameRow>,
) -> Line<'static> {
    if is_continuation {
        return continuation_gutter_line(ctx, blame_info);
    }

    let mut spans = Vec::new();

    // Blame column (if active)
    if ctx.blame_width > 0 {
        spans.push(blame_gutter_span(ctx.blame_width, blame_info, ctx.theme));
    }

    if ctx.fold_width > 0 {
        spans.push(fold_gutter_span(ctx.editor, line_idx, ctx.fold_width));
    }

    let line_num_text = line_number_text(ctx, line_idx);
    let (sign_text, sign_color) = gutter_sign(ctx, line_idx, line_diagnostics);
    let line_num_style = get_line_number_style(line_idx == ctx.cursor_line, ctx.theme);

    let sign_span = Span::styled(
        sign_text,
        Style::default().fg(sign_color).add_modifier(Modifier::BOLD),
    );
    let line_num_span = Span::styled(line_num_text, line_num_style);

    spans.push(sign_span);
    spans.push(line_num_span);

    Line::from(spans)
}
