//! Decoration rendering shared by both line pipelines: inline decorations
//! (inlay hints), end-of-line virtual text placement, and row width helpers.

use crate::display::grapheme_display_width;
use ovim_core::editor::decoration::{
    Decoration, DecorationPlacement, DecorationStyle as DecStyle, ProjectedDecorations,
};
use ovim_core::editor::ProjectedDiagnostics;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;

/// Per-line render cache fingerprint: the projected EOL/inline decorations
/// plus the line's full diagnostic set (ranges + severities). The underline
/// squiggle is baked into cached rows and the diagnostic set can change
/// without a buffer edit (save → republish), so the decoration hash alone —
/// which only sees the line's single best-severity EOL message — is not
/// enough to invalidate. (OV-00329)
pub(super) fn line_decoration_cache_hash(
    decorations: &ProjectedDecorations,
    diagnostics: &ProjectedDiagnostics,
    line_idx: usize,
) -> u64 {
    decorations
        .line_hash(line_idx)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ diagnostics.line_hash(line_idx)
}

/// Convert a `DecorationStyle` (framework-independent) to a ratatui `Style`.
pub(super) fn decoration_to_ratatui_style(ds: &DecStyle) -> Style {
    let mut style = Style::default();
    if let Some(fg) = &ds.fg {
        style = style.fg(ovim_color_to_ratatui(*fg));
    }
    if let Some(bg) = &ds.bg {
        style = style.bg(ovim_color_to_ratatui(*bg));
    }
    if ds.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if ds.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if ds.underline {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

fn ovim_color_to_ratatui(c: ovim_core::color::Color) -> Color {
    use ovim_core::color::Color as C;
    match c {
        C::Black => Color::Black,
        C::Red => Color::Red,
        C::Green => Color::Green,
        C::Yellow => Color::Yellow,
        C::Blue => Color::Blue,
        C::Magenta => Color::Magenta,
        C::Cyan => Color::Cyan,
        C::White => Color::White,
        C::DarkGray => Color::DarkGray,
        C::LightRed => Color::LightRed,
        C::LightGreen => Color::LightGreen,
        C::LightYellow => Color::LightYellow,
        C::LightBlue => Color::LightBlue,
        C::LightMagenta => Color::LightMagenta,
        C::LightCyan => Color::LightCyan,
        C::Gray => Color::Gray,
        C::Rgb(r, g, b) => Color::Rgb(r, g, b),
        C::Indexed(i) => Color::Indexed(i),
        C::Reset => Color::Reset,
    }
}

/// Truncate a rendered line to `max_width` display columns.
///
/// Walks spans left-to-right, keeping whole characters that fit.  Partial
/// spans at the boundary are split so the total display width is exactly
/// `max_width` (or less if the last character is wide).
pub(super) fn truncate_line_to_width(line: &mut Line<'static>, max_width: usize) {
    let mut total: usize = 0;
    let mut keep_spans = 0;

    // First pass: find the truncation point.
    let mut split_at: Option<(usize, usize)> = None; // (span_index, budget_cols)
    for (i, span) in line.spans.iter().enumerate() {
        let span_width: usize = span
            .content
            .graphemes(true)
            .map(grapheme_display_width)
            .sum();
        if total + span_width <= max_width {
            total += span_width;
            keep_spans += 1;
        } else {
            split_at = Some((i, max_width - total));
            break;
        }
    }

    // Second pass: apply truncation.
    if let Some((span_idx, budget)) = split_at {
        let style = line.spans[span_idx].style;
        let mut kept = String::new();
        let mut used = 0;
        for grapheme in line.spans[span_idx].content.graphemes(true) {
            let w = grapheme_display_width(grapheme);
            if used + w > budget {
                break;
            }
            kept.push_str(grapheme);
            used += w;
        }
        line.spans.truncate(keep_spans);
        if !kept.is_empty() {
            line.spans.push(Span::styled(kept, style));
        }
    }
    // All spans fit — nothing to truncate.
}

/// Apply inline decorations to a rendered line by splicing styled spans.
///
/// Decorations are inserted right-to-left (highest char_idx first) so earlier
/// insertions don't shift the positions of later ones.
pub(super) fn apply_inline_decorations(
    line: &mut Line<'static>,
    decorations: &[&Decoration],
    char_mapping: &[usize],
    h_offset: usize,
    wrap: bool,
    line_start_offset: usize,
) {
    if decorations.is_empty() {
        return;
    }

    // Sort right-to-left by char_offset
    let mut sorted: Vec<&&Decoration> = decorations.iter().collect();
    sorted.sort_by(|a, b| {
        let a_off = a.placement.char_offset();
        let b_off = b.placement.char_offset();
        b_off.cmp(&a_off) // reverse order
    });

    for dec in sorted {
        // Derive line-relative char_idx from absolute char_offset.
        let char_idx = match &dec.placement {
            DecorationPlacement::Inline { char_offset } => {
                char_offset.saturating_sub(line_start_offset)
            }
            _ => continue,
        };

        // Map through char_mapping (handles tab expansion)
        let expanded_col = if char_idx < char_mapping.len() {
            char_mapping[char_idx]
        } else if !char_mapping.is_empty() {
            *char_mapping.last().unwrap() + 1
        } else {
            char_idx
        };

        // Adjust for horizontal scroll in nowrap mode
        let insert_col = if !wrap {
            if expanded_col < h_offset {
                continue;
            }
            expanded_col - h_offset
        } else {
            expanded_col
        };

        let style = decoration_to_ratatui_style(&dec.style);

        // Walk spans to find insertion point, then split and insert
        let mut char_count = 0;
        let mut span_idx = 0;
        let mut found = false;

        while span_idx < line.spans.len() {
            let span_chars: usize = line.spans[span_idx].content.chars().count();
            if char_count + span_chars > insert_col {
                let offset_in_span = insert_col - char_count;
                let content = line.spans[span_idx].content.to_string();
                let span_style = line.spans[span_idx].style;

                let before: String = content.chars().take(offset_in_span).collect();
                let after: String = content.chars().skip(offset_in_span).collect();

                line.spans.remove(span_idx);
                let mut insert_at = span_idx;
                if !before.is_empty() {
                    line.spans
                        .insert(insert_at, Span::styled(before, span_style));
                    insert_at += 1;
                }
                line.spans
                    .insert(insert_at, Span::styled(dec.text.clone(), style));
                insert_at += 1;
                if !after.is_empty() {
                    line.spans
                        .insert(insert_at, Span::styled(after, span_style));
                }
                found = true;
                break;
            } else if char_count + span_chars == insert_col {
                line.spans
                    .insert(span_idx + 1, Span::styled(dec.text.clone(), style));
                found = true;
                break;
            }
            char_count += span_chars;
            span_idx += 1;
        }

        if !found {
            line.spans.push(Span::styled(dec.text.clone(), style));
        }
    }
}

/// Width of the gap (in columns) between rendered code and an EOL diagnostic.
pub(super) const EOL_DIAG_GAP: usize = 2;

/// Truncate `text` to at most `max_chars` characters, appending `...` when
/// truncation happens. If `max_chars` is too small to fit even the ellipsis,
/// returns whatever prefix fits with no marker.
///
/// Note: char count, not display width — a wide-char (CJK, emoji) message
/// can render up to 2x the budget. Pre-existing behavior; sharpening this
/// is a separate concern.
fn fit_with_ellipsis(text: &str, max_chars: usize) -> String {
    if text.graphemes(true).count() <= max_chars {
        return text.to_string();
    }
    if max_chars < 3 {
        return text.graphemes(true).take(max_chars).collect();
    }
    let prefix: String = text.graphemes(true).take(max_chars - 3).collect();
    format!("{prefix}...")
}

/// Total display width of all spans in a Line.
fn line_display_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|s| {
            s.content
                .graphemes(true)
                .map(grapheme_display_width)
                .sum::<usize>()
        })
        .sum()
}

/// Display width of the row's real content: trailing all-space spans with a
/// default style (the padding `split_line_into_rows` appends) are ignored.
/// Placement decisions must use this, not `line_display_width` — under soft
/// wrap every row arrives padded to `text_width`, and measuring the padding
/// made `place_eol_on_line` treat every short line as "full", pushing its
/// diagnostic to the far screen edge instead of next to the code.
pub(super) fn content_display_width(line: &Line<'_>) -> usize {
    let mut spans = line.spans.as_slice();
    while let Some(last) = spans.last() {
        if last.content.chars().all(|c| c == ' ') && last.style == Style::default() {
            spans = &spans[..spans.len() - 1];
        } else {
            break;
        }
    }
    spans
        .iter()
        .map(|s| {
            s.content
                .graphemes(true)
                .map(grapheme_display_width)
                .sum::<usize>()
        })
        .sum()
}

/// Pad the line with trailing space spans until it reaches `target_width`.
/// No-op if the line already meets or exceeds `target_width`.
pub(super) fn pad_line_to(line: &mut Line<'static>, target_width: usize) {
    let width = line_display_width(line);
    if width < target_width {
        line.spans.push(Span::raw(" ".repeat(target_width - width)));
    }
}

/// Pad the line with trailing spaces carrying `color` as their background, so
/// a diff row's tint reaches the right edge of the viewport.
pub(super) fn pad_line_to_styled(line: &mut Line<'static>, target_width: usize, color: Color) {
    let width = line_display_width(line);
    if width < target_width {
        line.spans.push(Span::styled(
            " ".repeat(target_width - width),
            Style::default().bg(color),
        ));
    }
}

/// Where an EOL decoration should be placed within a rendered row.
#[derive(Debug, Clone, Copy)]
pub(super) enum EolPlacement {
    /// Append the diagnostic right after the row's existing content,
    /// pad the row to `text_width`. Used when the line lives inside a
    /// single budget: non-centered mode, single-row no-overflow case.
    Append { text_width: usize },
    /// Clip the row at `code_box_width` (no bleed past the box), anchor
    /// the diagnostic immediately after the rendered code (so it sits
    /// close to the line, not at the far edge), and pad to `render_width`.
    /// Used in centered (textwidth) mode where lines render into a wider
    /// rect than the code-box: code stays inside the code-box, but the
    /// diagnostic is free to extend into the right margin.
    AtBoxEdge {
        code_box_width: usize,
        render_width: usize,
    },
}

/// Apply end-of-line decorations to a rendered row.
///
/// Strips trailing padding, appends each decoration's styled text (with a
/// `EOL_DIAG_GAP`-column gap), truncates to fit, and re-pads. The exact
/// anchor and final width depend on `placement` — see [`EolPlacement`].
///
/// `AtBoxEdge` performs its clip + anchor + pad work even when there are
/// no decorations, so callers can use it to enforce no-bleed geometry on
/// every row of a wrapped line in centered mode. `Append` short-circuits
/// when there's nothing to do (caller has already padded to text_width).
pub(super) fn apply_eol_decorations(
    row: &mut Line<'static>,
    decorations: &[&Decoration],
    placement: EolPlacement,
) {
    // Append with no decorations: caller's padding already handles this.
    if decorations.is_empty() && matches!(placement, EolPlacement::Append { .. }) {
        return;
    }

    // Remove trailing padding spans so we know where the code actually ends.
    while let Some(last) = row.spans.last() {
        if last.content.chars().all(|c| c == ' ') && last.style == Style::default() {
            row.spans.pop();
        } else {
            break;
        }
    }

    // Resolve where the diagnostic anchors and what we pad to. AtBoxEdge
    // also clips any content past the code-box edge so hints/code don't
    // bleed into the diagnostic margin, then anchors the diagnostic right
    // after the (clipped) rendered code — short lines get the diagnostic
    // close, long lines (clipped at code_box_width) get it at the box edge.
    let (diag_start, final_width) = match placement {
        EolPlacement::Append { text_width } => (line_display_width(row), text_width),
        EolPlacement::AtBoxEdge {
            code_box_width,
            render_width,
        } => {
            truncate_line_to_width(row, code_box_width);
            (line_display_width(row), render_width)
        }
    };

    // Pad the row up to the anchor (extends short lines to a consistent column).
    pad_line_to(row, diag_start);

    // Append the diagnostic if we have one and there's room for gap + ≥1 char.
    let remaining = final_width.saturating_sub(diag_start);
    if !decorations.is_empty() && remaining >= EOL_DIAG_GAP + 4 {
        // First decoration wins (already priority-sorted). The message may
        // use everything up to the row edge: the space right of the code is
        // otherwise unused, and diagnostics are exactly what the user wants
        // to read there.
        let dec = &decorations[0];
        let msg = fit_with_ellipsis(&dec.text, remaining - EOL_DIAG_GAP);
        let style = decoration_to_ratatui_style(&dec.style);
        row.spans.push(Span::raw(" ".repeat(EOL_DIAG_GAP)));
        row.spans.push(Span::styled(msg, style));
    }

    pad_line_to(row, final_width);
}

/// Place an EOL decoration on a single rendered line, choosing the right
/// strategy from `(text_width, render_width, line_width, has_decs)`:
///
/// - **Centered (render_width > text_width)** → `AtBoxEdge`, which clips the
///   line at the code-box edge (no bleed into the margin) and anchors the
///   diagnostic immediately after the rendered code. Short lines get the
///   diagnostic close; lines that reach (or exceed) the box edge get the
///   diagnostic at the box edge — the message is free to extend into the
///   right margin in both cases.
/// - **Non-centered, line + hints overflow text_width with decs present**
///   → `overlay_eol_decoration_at_edge`, which steals the rightmost columns
///   so the diagnostic stays visible.
/// - **Non-centered, line fits or no decs** → `Append`, the default
///   "diagnostic floats after code" behavior.
pub(super) fn place_eol_on_line(
    line: &mut Line<'static>,
    eol_decs: &[&Decoration],
    text_width: usize,
    render_width: usize,
) {
    if render_width > text_width {
        apply_eol_decorations(
            line,
            eol_decs,
            EolPlacement::AtBoxEdge {
                code_box_width: text_width,
                render_width,
            },
        );
    } else if !eol_decs.is_empty() && content_display_width(line) >= text_width {
        overlay_eol_decoration_at_edge(line, eol_decs, text_width);
    } else {
        apply_eol_decorations(line, eol_decs, EolPlacement::Append { text_width });
    }
}

/// Place EOL decorations across the visual rows of a wrapped line. The
/// last row gets the diagnostic via [`place_eol_on_line`]; in centered
/// mode every other row is clipped + padded to `render_width` so they
/// don't bleed into the diagnostic margin.
pub(super) fn place_eol_on_visual_rows(
    rows: &mut [Line<'static>],
    eol_decs: &[&Decoration],
    text_width: usize,
    render_width: usize,
) {
    if rows.is_empty() {
        return;
    }
    let centered = render_width > text_width;
    let last = rows.len() - 1;
    for (i, row) in rows.iter_mut().enumerate() {
        if i == last {
            place_eol_on_line(row, eol_decs, text_width, render_width);
        } else if centered {
            apply_eol_decorations(
                row,
                &[],
                EolPlacement::AtBoxEdge {
                    code_box_width: text_width,
                    render_width,
                },
            );
        }
    }
}

/// Overlay an EOL decoration at the right edge of a line that already exceeds
/// `text_width` (typically because inline decorations pushed it beyond the
/// viewport). The diagnostic replaces the rightmost columns of the rendered
/// line so it's always visible without affecting cursor positioning.
fn overlay_eol_decoration_at_edge(
    line: &mut Line<'static>,
    decorations: &[&Decoration],
    text_width: usize,
) {
    if decorations.is_empty() || text_width < EOL_DIAG_GAP + 6 {
        return;
    }

    let dec = &decorations[0];
    let style = decoration_to_ratatui_style(&dec.style);

    // Budget: gap + message at the right edge, message capped at 1/3 of viewport.
    let msg = fit_with_ellipsis(&dec.text, text_width / 3);
    let overlay_width = EOL_DIAG_GAP
        + msg
            .graphemes(true)
            .map(grapheme_display_width)
            .sum::<usize>();

    // Truncate code to make room, pad up to the truncation point, then push
    // gap + styled message + final padding.
    let truncate_to = text_width.saturating_sub(overlay_width);
    truncate_line_to_width(line, truncate_to);
    pad_line_to(line, truncate_to);
    line.spans.push(Span::raw(" ".repeat(EOL_DIAG_GAP)));
    line.spans.push(Span::styled(msg, style));
    pad_line_to(line, text_width);
}
