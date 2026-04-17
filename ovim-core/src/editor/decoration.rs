use crate::color::Color;
use crate::edit::Edit;
use crate::edit_log::EditLog;
use ropey::Rope;
use std::collections::BTreeMap;

/// Where a decoration appears relative to buffer text.
///
/// Positions are stored as **absolute char offsets** into the rope.
/// This allows edits to adjust positions with simple arithmetic
/// (shift forward for inserts, backward for deletes) without any
/// line/col conversion.  The line number and line-relative char index
/// are derived at query time from the rope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecorationPlacement {
    /// Inserted inline at a buffer position. Affects display width,
    /// cursor positioning, and wrap calculation.
    Inline {
        /// Absolute char offset in the rope.
        char_offset: usize,
    },
    /// Appended after the code text on a line. Does not affect cursor.
    /// Rendered after wrapping (first visual row only).
    EndOfLine {
        /// Char offset of the start of the target line in the rope.
        char_offset: usize,
    },
}

impl DecorationPlacement {
    /// Absolute char offset into the rope.
    pub fn char_offset(&self) -> usize {
        match self {
            Self::Inline { char_offset } | Self::EndOfLine { char_offset } => *char_offset,
        }
    }

    /// Mutable reference to the char offset (for edit adjustment).
    fn char_offset_mut(&mut self) -> &mut usize {
        match self {
            Self::Inline { char_offset } | Self::EndOfLine { char_offset } => char_offset,
        }
    }

    /// Derive the line number from the rope.
    pub fn line(&self, rope: &Rope) -> usize {
        let offset = self.char_offset().min(rope.len_chars());
        rope.char_to_line(offset)
    }

    /// Derive the line-relative char index from the rope.
    pub fn char_idx(&self, rope: &Rope) -> usize {
        let offset = self.char_offset().min(rope.len_chars());
        let line = rope.char_to_line(offset);
        offset - rope.line_to_char(line)
    }
}

/// The source that produced a decoration, for filtering and replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecorationSource {
    InlayHint,
    Diagnostic,
}

/// Visual style for a decoration, independent of ratatui.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecorationStyle {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub italic: bool,
    pub bold: bool,
    pub underline: bool,
}

impl DecorationStyle {
    pub fn new(fg: Color) -> Self {
        Self {
            fg: Some(fg),
            bg: None,
            italic: false,
            bold: false,
            underline: false,
        }
    }

    pub fn with_bg(mut self, bg: Color) -> Self {
        self.bg = Some(bg);
        self
    }

    pub fn with_italic(mut self) -> Self {
        self.italic = true;
        self
    }
}

/// A single piece of virtual text attached to a buffer position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoration {
    pub placement: DecorationPlacement,
    pub source: DecorationSource,
    pub text: String,
    /// Display width in terminal columns (precomputed).
    pub display_width: usize,
    pub style: DecorationStyle,
    /// Sort priority within the same position (lower = rendered first).
    pub priority: u8,
    /// Buffer version that this decoration's `char_offset` was computed
    /// against. Populated from the originating LSP request's `buffer_version`
    /// at construction, and bumped to match the post-edit version each time
    /// `adjust_for_edits` shifts the offset.  Phase-05 Step C populates this
    /// field; Step D adds a projection consumer that uses it.
    pub source_version: u64,
}

/// Project a decoration's source-version char offset forward through a sequence
/// of edits, yielding the offset it should occupy after those edits are applied.
///
/// This is the pure, stateless twin of [`DecorationMap::adjust_for_edits`]: same
/// arithmetic, same semantics, no mutation. The Phase-05 roadmap will eventually
/// have the renderer call `project_offset` on each frame instead of the caller
/// mutating `char_offset` in place. Step D introduces the function and validates
/// it against the accumulator. Step E switches consumers over.
///
/// # Semantics
///
/// For each edit, the offset is transformed as follows:
///
/// - `Edit::Insert { offset: X, text }` of char-length `N`:
///   - if `pos >= X`, the anchor was at or after the insertion point — shift forward by `N`.
///   - otherwise unchanged.
///     Note: `pos == X` shifts forward (inclusive-before semantics). This
///     matches [`DecorationMap::adjust_for_edits`] exactly.
/// - `Edit::Delete { offset: X, text }` of char-length `N`, with `end = X + N`:
///   - if `pos >= end`, the anchor was past the deleted region — shift back by `N`.
///   - if `X < pos < end`, the anchor sat strictly inside the deleted region — return `None`
///     (the decoration has been eaten by the deletion).
///   - otherwise (`pos <= X`) unchanged.
///
/// An empty edit slice is a no-op: returns `Some(source_offset)`.
///
/// # Parity with the accumulator
///
/// The transform above is the same one [`DecorationMap::adjust_for_edits`]
/// applies in-place. The unit tests in this module and the integration test at
/// `ovim/tests/decoration_projection_test.rs` verify parity across interactive
/// scenarios (insert-before, insert-after, delete-before, delete-over, undo,
/// rapid typing). Any divergence is a bug in one of the two paths.
pub fn project_offset(source_offset: usize, edits: &[&Edit]) -> Option<usize> {
    let mut pos = source_offset;
    for edit in edits {
        match edit {
            Edit::Insert { offset, text } => {
                let len = text.chars().count();
                if pos >= *offset {
                    pos += len;
                }
            }
            Edit::Delete { offset, text } => {
                let len = text.chars().count();
                let end = *offset + len;
                if pos >= end {
                    pos -= len;
                } else if pos > *offset {
                    // Strictly inside the deleted region — decoration is lost.
                    return None;
                }
                // pos <= offset: unchanged (anchor lived before the delete).
            }
        }
    }
    Some(pos)
}

/// Per-line decoration index for efficient lookup during rendering.
///
/// Decorations are grouped by line and sorted by position within each line.
/// The `generation` counter increments on every mutation, enabling efficient
/// cache invalidation in the renderer (the line cache includes the generation
/// in its key and naturally misses when decorations change).
#[derive(Debug, Clone)]
pub struct DecorationMap {
    lines: BTreeMap<usize, Vec<Decoration>>,
    pub generation: u64,
}

impl Default for DecorationMap {
    fn default() -> Self {
        Self::new()
    }
}

impl DecorationMap {
    pub fn new() -> Self {
        Self {
            lines: BTreeMap::new(),
            generation: 0,
        }
    }

    /// Replace all decorations from a given source.
    /// The rope is needed to derive line numbers for the index.
    /// Returns true if the set actually changed.
    pub fn replace_source(
        &mut self,
        source: DecorationSource,
        decorations: Vec<Decoration>,
        rope: &Rope,
    ) -> bool {
        // Remove old decorations from this source.
        let mut any_removed = false;
        self.lines.retain(|_, line_decs| {
            let before = line_decs.len();
            line_decs.retain(|d| d.source != source);
            if line_decs.len() != before {
                any_removed = true;
            }
            !line_decs.is_empty()
        });

        let any_added = !decorations.is_empty();

        // Insert new decorations, keyed by line derived from char_offset.
        for dec in decorations {
            let line = dec.placement.line(rope);
            self.lines.entry(line).or_default().push(dec);
        }

        // Sort each affected line by position then priority.
        Self::sort_all_lines(&mut self.lines);

        if any_removed || any_added {
            self.generation = self.generation.wrapping_add(1);
            true
        } else {
            false
        }
    }

    /// Adjust decoration char_offsets after buffer edits, then rebuild
    /// the line index.  Edits must be in the order they were applied.
    ///
    /// The rope must be the **post-edit** rope (edits already applied).
    /// The arithmetic adjustment is independent of rope state — it only
    /// uses the edit offsets and lengths.
    ///
    /// `new_version` is the buffer version after the edits land; every
    /// surviving decoration's `source_version` is bumped to match so the
    /// accumulator and the Step-D projection stay in sync while both
    /// systems coexist.
    pub fn adjust_for_edits(&mut self, edits: &[Edit], rope: &Rope, new_version: u64) {
        if self.lines.is_empty() || edits.is_empty() {
            return;
        }

        // Collect all decorations, adjust offsets, then rebuild index.
        let mut all_decs: Vec<Decoration> =
            self.lines.values_mut().flat_map(|v| v.drain(..)).collect();
        self.lines.clear();

        for edit in edits {
            match edit {
                Edit::Insert { offset, text } => {
                    let len = text.chars().count();
                    for dec in &mut all_decs {
                        let off = dec.placement.char_offset_mut();
                        if *off >= *offset {
                            *off += len;
                        }
                    }
                }
                Edit::Delete { offset, text } => {
                    let len = text.chars().count();
                    let end = offset + len;
                    all_decs.retain_mut(|dec| {
                        let off = dec.placement.char_offset_mut();
                        if *off >= end {
                            *off -= len;
                            true
                        } else if *off > *offset {
                            // Inside deleted region — remove decoration.
                            false
                        } else {
                            true
                        }
                    });
                }
            }
        }

        // Clamp offsets to valid range and rebuild line index.
        let max_offset = rope.len_chars();
        for dec in &mut all_decs {
            let off = dec.placement.char_offset_mut();
            if *off > max_offset {
                *off = max_offset;
            }
            // Keep the anchor version in lockstep with the accumulator.  When
            // Step D introduces projection, the projection cache will key on
            // `(source_version, current_version)`; keeping the field current
            // means a freshly-adjusted decoration projects as a no-op.
            dec.source_version = new_version;
        }

        for dec in all_decs {
            let line = dec.placement.line(rope);
            self.lines.entry(line).or_default().push(dec);
        }
        Self::sort_all_lines(&mut self.lines);
        self.generation = self.generation.wrapping_add(1);
    }

    /// Get all decorations for a line, sorted by position then priority.
    pub fn for_line(&self, line: usize) -> &[Decoration] {
        self.lines.get(&line).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Iterate over every decoration in the map, yielding `(line, &Decoration)`
    /// pairs in line-then-position order.
    ///
    /// Useful for callers that need to enumerate the full set — e.g. projecting
    /// the decoration state into a snapshot for the REST API.
    pub fn iter_all(&self) -> impl Iterator<Item = (usize, &Decoration)> {
        self.lines
            .iter()
            .flat_map(|(line, decs)| decs.iter().map(move |d| (*line, d)))
    }

    /// Get only inline decorations for a line.
    pub fn inline_for_line(&self, line: usize) -> Vec<&Decoration> {
        self.for_line(line)
            .iter()
            .filter(|d| matches!(d.placement, DecorationPlacement::Inline { .. }))
            .collect()
    }

    /// Get only end-of-line decorations for a line.
    pub fn eol_for_line(&self, line: usize) -> Vec<&Decoration> {
        self.for_line(line)
            .iter()
            .filter(|d| matches!(d.placement, DecorationPlacement::EndOfLine { .. }))
            .collect()
    }

    /// Returns `(char_idx, display_width)` pairs for inline decorations on a
    /// line, sorted by char_idx.  Used by WrapMap to account for decoration
    /// widths when computing wrap points.
    pub fn inline_decorations_for_line(&self, line: usize, rope: &Rope) -> Vec<(usize, usize)> {
        let line_start = rope.line_to_char(line);
        self.for_line(line)
            .iter()
            .filter_map(|d| match &d.placement {
                DecorationPlacement::Inline { char_offset } => {
                    Some((char_offset.saturating_sub(line_start), d.display_width))
                }
                _ => None,
            })
            .collect()
    }

    /// Total display width of inline decorations on a line
    /// at or before the given char index.
    pub fn inline_width_before(&self, line: usize, char_idx: usize, rope: &Rope) -> usize {
        let line_start = rope.line_to_char(line);
        self.for_line(line)
            .iter()
            .filter_map(|d| match &d.placement {
                DecorationPlacement::Inline { char_offset } => {
                    let idx = char_offset.saturating_sub(line_start);
                    if idx <= char_idx {
                        Some(d.display_width)
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .sum()
    }

    // -----------------------------------------------------------------
    // Step-E projected accessors
    //
    // The renderer routes through these instead of the raw accessors: they
    // compute each decoration's live char offset by projecting its stored
    // `char_offset` forward through `edit_log.edits_since(source_version)`
    // before filtering by line.
    //
    // In steady state — with the accumulator still running in parallel —
    // `source_version` is bumped to the post-edit buffer version every time
    // the accumulator adjusts `char_offset`, so `edits_since(source_version)`
    // is empty and the projected offset equals the stored one. The projection
    // is still the source of truth for rendering: when Step F removes the
    // accumulator, these methods become the only path and nothing else needs
    // to change.
    //
    // The `rope` argument on `..._projected` methods is always the **current**
    // rope (post-edit); line indices are derived by calling `char_to_line` on
    // the projected offset so a decoration whose anchor has crossed a line
    // boundary shows up on the correct line without mutation.
    // -----------------------------------------------------------------

    /// Project a single decoration's stored offset through the edit log.
    ///
    /// Returns `Some(offset)` if projection succeeds, `None` if a delete
    /// engulfed the anchor since `source_version`. If the log has evicted the
    /// history for that version we fall back to the stored offset — the
    /// decoration will be slightly wrong until a fresh LSP response lands,
    /// but this matches the existing "stale is better than blank" policy.
    fn project_decoration(dec: &Decoration, log: &EditLog) -> Option<usize> {
        let stored = dec.placement.char_offset();
        match log.edits_since(dec.source_version) {
            Some(edits) => project_offset(stored, &edits),
            None => Some(stored),
        }
    }

    /// Projected analogue of [`for_line`]. Returns owned `Decoration`s whose
    /// `placement` has the projected char offset applied; the line lookup is
    /// done against the **projected** offset using the current rope.
    ///
    /// Decorations whose anchors were engulfed by a delete since their
    /// `source_version` are filtered out.
    pub fn for_line_projected(&self, line: usize, rope: &Rope, log: &EditLog) -> Vec<Decoration> {
        let mut out = Vec::new();
        for (_, dec) in self.iter_all() {
            let Some(projected) = Self::project_decoration(dec, log) else {
                continue;
            };
            let projected_line = if projected <= rope.len_chars() {
                rope.char_to_line(projected)
            } else {
                rope.char_to_line(rope.len_chars())
            };
            if projected_line == line {
                let mut cloned = dec.clone();
                match &mut cloned.placement {
                    DecorationPlacement::Inline { char_offset }
                    | DecorationPlacement::EndOfLine { char_offset } => {
                        *char_offset = projected;
                    }
                }
                out.push(cloned);
            }
        }
        // Preserve the same sort order as the stored map.
        out.sort_by(|a, b| {
            let pos_a = match &a.placement {
                DecorationPlacement::Inline { char_offset } => (0, *char_offset),
                DecorationPlacement::EndOfLine { .. } => (1, usize::MAX),
            };
            let pos_b = match &b.placement {
                DecorationPlacement::Inline { char_offset } => (0, *char_offset),
                DecorationPlacement::EndOfLine { .. } => (1, usize::MAX),
            };
            pos_a.cmp(&pos_b).then(a.priority.cmp(&b.priority))
        });
        out
    }

    /// Projected analogue of [`inline_decorations_for_line`]. Returns
    /// `(char_idx_in_line, display_width)` pairs for inline decorations on the
    /// given line, computed from projected offsets.
    pub fn inline_decorations_for_line_projected(
        &self,
        line: usize,
        rope: &Rope,
        log: &EditLog,
    ) -> Vec<(usize, usize)> {
        let line_start = rope.line_to_char(line);
        self.for_line_projected(line, rope, log)
            .into_iter()
            .filter_map(|d| match d.placement {
                DecorationPlacement::Inline { char_offset } => {
                    Some((char_offset.saturating_sub(line_start), d.display_width))
                }
                _ => None,
            })
            .collect()
    }

    /// Projected analogue of [`inline_width_before`]. Sums the display width
    /// of inline decorations whose projected char_idx is `<= char_idx`.
    pub fn inline_width_before_projected(
        &self,
        line: usize,
        char_idx: usize,
        rope: &Rope,
        log: &EditLog,
    ) -> usize {
        let line_start = rope.line_to_char(line);
        self.for_line_projected(line, rope, log)
            .into_iter()
            .filter_map(|d| match d.placement {
                DecorationPlacement::Inline { char_offset } => {
                    let idx = char_offset.saturating_sub(line_start);
                    if idx <= char_idx {
                        Some(d.display_width)
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .sum()
    }

    /// Projected analogue of [`eol_for_line`]. Returns owned clones so the
    /// renderer can consume them like the stored slice.
    pub fn eol_for_line_projected(
        &self,
        line: usize,
        rope: &Rope,
        log: &EditLog,
    ) -> Vec<Decoration> {
        self.for_line_projected(line, rope, log)
            .into_iter()
            .filter(|d| matches!(d.placement, DecorationPlacement::EndOfLine { .. }))
            .collect()
    }

    /// Clear all decorations (e.g., on buffer switch).
    pub fn clear(&mut self) {
        if !self.lines.is_empty() {
            self.lines.clear();
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Returns true if there are no decorations at all.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Sort decorations within each line: inline before EOL, then by
    /// char_offset, then by priority.
    fn sort_all_lines(lines: &mut BTreeMap<usize, Vec<Decoration>>) {
        for line_decs in lines.values_mut() {
            line_decs.sort_by(|a, b| {
                let pos_a = match &a.placement {
                    DecorationPlacement::Inline { char_offset } => (0, *char_offset),
                    DecorationPlacement::EndOfLine { .. } => (1, usize::MAX),
                };
                let pos_b = match &b.placement {
                    DecorationPlacement::Inline { char_offset } => (0, *char_offset),
                    DecorationPlacement::EndOfLine { .. } => (1, usize::MAX),
                };
                pos_a.cmp(&pos_b).then(a.priority.cmp(&b.priority))
            });
        }
    }
}

// ---------------------------------------------------------------------------
// LSP → Decoration conversion helpers
// ---------------------------------------------------------------------------

/// Convert LSP inlay hints into inline decorations with rope-anchored offsets.
///
/// `line_text` returns the text of a given line (without trailing newline).
/// Used to convert LSP UTF-16 offsets to char indices.
/// The rope is used to compute absolute char offsets from (line, char_idx).
/// `source_version` is the buffer version the hints were computed against
/// (from the originating LSP request's `buffer_version`); stored on each
/// produced decoration for Step-D projection.
pub fn decorations_from_inlay_hints<F>(
    hints: &[lsp_types::InlayHint],
    rope: &Rope,
    line_text: F,
    source_version: u64,
) -> Vec<Decoration>
where
    F: Fn(usize) -> String,
{
    let hint_style = DecorationStyle::new(Color::Rgb(120, 120, 140)).with_italic();

    hints
        .iter()
        .map(|hint| {
            let line = hint.position.line as usize;
            let utf16_col = hint.position.character as u32;
            // Convert UTF-16 offset → char index.
            let text_for_line = line_text(line);
            let char_idx = crate::lsp::utf16_to_char_col(&text_for_line, utf16_col);
            // Convert to absolute rope char offset.
            let char_offset = if line < rope.len_lines() {
                rope.line_to_char(line) + char_idx
            } else {
                rope.len_chars()
            };

            let label = match &hint.label {
                lsp_types::InlayHintLabel::String(s) => s.clone(),
                lsp_types::InlayHintLabel::LabelParts(parts) => {
                    parts.iter().map(|p| &*p.value).collect::<String>()
                }
            };

            let text = if hint.padding_right.unwrap_or(false) {
                format!("{} ", label)
            } else {
                label
            };

            let display_width = text.chars().count(); // ASCII-safe for typical hints

            Decoration {
                placement: DecorationPlacement::Inline { char_offset },
                source: DecorationSource::InlayHint,
                text,
                display_width,
                style: hint_style.clone(),
                priority: 10,
                source_version,
            }
        })
        .collect()
}

/// Diagnostic severity icon and colors, matching the existing renderer style.
fn diagnostic_style(
    severity: Option<lsp_types::DiagnosticSeverity>,
) -> (&'static str, DecorationStyle) {
    use lsp_types::DiagnosticSeverity;
    match severity {
        Some(DiagnosticSeverity::ERROR) => (
            "",
            DecorationStyle::new(Color::Red)
                .with_bg(Color::Rgb(60, 20, 20))
                .with_italic(),
        ),
        Some(DiagnosticSeverity::WARNING) => (
            "",
            DecorationStyle::new(Color::Yellow)
                .with_bg(Color::Rgb(60, 50, 20))
                .with_italic(),
        ),
        Some(DiagnosticSeverity::INFORMATION) => (
            "",
            DecorationStyle::new(Color::Cyan)
                .with_bg(Color::Rgb(20, 40, 60))
                .with_italic(),
        ),
        Some(DiagnosticSeverity::HINT) | _ => (
            "",
            DecorationStyle::new(Color::Gray)
                .with_bg(Color::Rgb(40, 40, 40))
                .with_italic(),
        ),
    }
}

/// Convert LSP diagnostics into end-of-line decorations with rope-anchored offsets.
///
/// Only the first (highest-severity) diagnostic per line is converted,
/// matching the existing renderer behavior.
/// `source_version` is the buffer version the diagnostics were computed
/// against (from the originating refresh's `buffer_version`); stored on each
/// produced decoration for Step-D projection.
pub fn decorations_from_diagnostics(
    diagnostics: &[lsp_types::Diagnostic],
    rope: &Rope,
    source_version: u64,
) -> Vec<Decoration> {
    use std::collections::HashMap;

    // Group by line, keep highest severity per line.
    let mut best_per_line: HashMap<usize, &lsp_types::Diagnostic> = HashMap::new();
    for diag in diagnostics {
        let line = diag.range.start.line as usize;
        let entry = best_per_line.entry(line).or_insert(diag);
        let severity_ord = |s: Option<lsp_types::DiagnosticSeverity>| -> u32 {
            match s {
                Some(lsp_types::DiagnosticSeverity::ERROR) => 1,
                Some(lsp_types::DiagnosticSeverity::WARNING) => 2,
                Some(lsp_types::DiagnosticSeverity::INFORMATION) => 3,
                Some(lsp_types::DiagnosticSeverity::HINT) => 4,
                _ => 5,
            }
        };
        let entry_sev = severity_ord(entry.severity);
        let this_sev = severity_ord(diag.severity);
        if this_sev < entry_sev {
            *entry = diag;
        }
    }

    best_per_line
        .into_iter()
        .map(|(line, diag)| {
            let (icon, style) = diagnostic_style(diag.severity);
            let msg = diag.message.lines().next().unwrap_or("");
            let text = format!("{} {}", icon, msg);
            let display_width = text.chars().count();

            let priority = match diag.severity {
                Some(lsp_types::DiagnosticSeverity::ERROR) => 0,
                Some(lsp_types::DiagnosticSeverity::WARNING) => 1,
                Some(lsp_types::DiagnosticSeverity::INFORMATION) => 2,
                _ => 3,
            };

            // Anchor to the start of the line in the rope.
            let char_offset = if line < rope.len_lines() {
                rope.line_to_char(line)
            } else {
                rope.len_chars()
            };

            Decoration {
                placement: DecorationPlacement::EndOfLine { char_offset },
                source: DecorationSource::Diagnostic,
                text,
                display_width,
                style,
                priority,
                source_version,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test rope: "let x = 1;\nlet y = 2;\nlet z = 3;\n"
    ///   line 0: chars  0..11  "let x = 1;\n"
    ///   line 1: chars 11..22  "let y = 2;\n"
    ///   line 2: chars 22..33  "let z = 3;\n"
    fn test_rope() -> Rope {
        Rope::from_str("let x = 1;\nlet y = 2;\nlet z = 3;\n")
    }

    fn inline_at(char_offset: usize, text: &str, source: DecorationSource) -> Decoration {
        let display_width = text.len(); // ASCII for tests
        Decoration {
            placement: DecorationPlacement::Inline { char_offset },
            source,
            text: text.to_string(),
            display_width,
            style: DecorationStyle::new(Color::Gray).with_italic(),
            priority: 0,
            source_version: 0,
        }
    }

    fn eol_at(char_offset: usize, text: &str, source: DecorationSource) -> Decoration {
        let display_width = text.len();
        Decoration {
            placement: DecorationPlacement::EndOfLine { char_offset },
            source,
            text: text.to_string(),
            display_width,
            style: DecorationStyle::new(Color::Red),
            priority: 0,
            source_version: 0,
        }
    }

    #[test]
    fn replace_source_inserts_and_retrieves() {
        let rope = test_rope();
        let mut map = DecorationMap::new();
        let gen_before = map.generation;

        // char 5 on line 0 = offset 5; char 10 on line 2 = offset 22+10=32
        map.replace_source(
            DecorationSource::InlayHint,
            vec![
                inline_at(5, ": String", DecorationSource::InlayHint),
                inline_at(32, "count: ", DecorationSource::InlayHint),
            ],
            &rope,
        );

        assert_eq!(map.for_line(0).len(), 1);
        assert_eq!(map.for_line(0)[0].text, ": String");
        assert_eq!(map.for_line(2).len(), 1);
        assert!(map.for_line(1).is_empty());
        assert!(map.generation > gen_before);
    }

    #[test]
    fn replace_source_replaces_only_same_source() {
        let rope = test_rope();
        let mut map = DecorationMap::new();

        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(5, ": String", DecorationSource::InlayHint)],
            &rope,
        );
        map.replace_source(
            DecorationSource::Diagnostic,
            vec![eol_at(
                0,
                "error: unused variable",
                DecorationSource::Diagnostic,
            )],
            &rope,
        );

        assert_eq!(map.for_line(0).len(), 2);

        // Replace inlay hints with new set
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(3, ": i32", DecorationSource::InlayHint)],
            &rope,
        );

        assert_eq!(map.for_line(0).len(), 2);
        // Inline (InlayHint) should be first, then EOL (Diagnostic)
        assert_eq!(map.for_line(0)[0].text, ": i32");
        assert_eq!(map.for_line(0)[1].text, "error: unused variable");
    }

    #[test]
    fn replace_source_with_empty_removes_all_from_source() {
        let rope = test_rope();
        let mut map = DecorationMap::new();

        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(5, ": String", DecorationSource::InlayHint)],
            &rope,
        );
        let gen_before = map.generation;

        map.replace_source(DecorationSource::InlayHint, vec![], &rope);

        assert!(map.for_line(0).is_empty());
        assert!(map.generation > gen_before);
    }

    #[test]
    fn no_op_replace_does_not_bump_generation() {
        let rope = test_rope();
        let mut map = DecorationMap::new();
        let gen = map.generation;

        map.replace_source(DecorationSource::Diagnostic, vec![], &rope);

        assert_eq!(map.generation, gen);
    }

    #[test]
    fn inline_width_before() {
        let rope = test_rope();
        let mut map = DecorationMap::new();
        // line 0 starts at offset 0: char_idx 3 = offset 3, char_idx 10 = offset 10
        map.replace_source(
            DecorationSource::InlayHint,
            vec![
                inline_at(3, ": i32", DecorationSource::InlayHint), // 5 cols at char 3
                inline_at(10, ": String", DecorationSource::InlayHint), // 8 cols at char 10
            ],
            &rope,
        );

        // Before any hint
        assert_eq!(map.inline_width_before(0, 2, &rope), 0);
        // At the first hint
        assert_eq!(map.inline_width_before(0, 3, &rope), 5);
        // Between hints
        assert_eq!(map.inline_width_before(0, 7, &rope), 5);
        // At the second hint
        assert_eq!(map.inline_width_before(0, 10, &rope), 13); // 5 + 8
                                                               // Different line
        assert_eq!(map.inline_width_before(1, 10, &rope), 0);
    }

    #[test]
    fn inline_for_line_and_eol_for_line() {
        let rope = test_rope();
        let mut map = DecorationMap::new();
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(5, ": String", DecorationSource::InlayHint)],
            &rope,
        );
        map.replace_source(
            DecorationSource::Diagnostic,
            vec![eol_at(0, "unused variable", DecorationSource::Diagnostic)],
            &rope,
        );

        assert_eq!(map.inline_for_line(0).len(), 1);
        assert_eq!(map.eol_for_line(0).len(), 1);
    }

    #[test]
    fn sort_order_inline_before_eol_then_by_position() {
        let rope = test_rope();
        let mut map = DecorationMap::new();
        map.replace_source(
            DecorationSource::Diagnostic,
            vec![eol_at(0, "error", DecorationSource::Diagnostic)],
            &rope,
        );
        map.replace_source(
            DecorationSource::InlayHint,
            vec![
                inline_at(10, "b_hint", DecorationSource::InlayHint),
                inline_at(3, "a_hint", DecorationSource::InlayHint),
            ],
            &rope,
        );

        let decs = map.for_line(0);
        assert_eq!(decs.len(), 3);
        // Inline at char 3 first
        assert_eq!(decs[0].text, "a_hint");
        // Inline at char 10 second
        assert_eq!(decs[1].text, "b_hint");
        // EOL last
        assert_eq!(decs[2].text, "error");
    }

    #[test]
    fn clear_empties_and_bumps_generation() {
        let rope = test_rope();
        let mut map = DecorationMap::new();
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(5, "hint", DecorationSource::InlayHint)],
            &rope,
        );
        let gen_before = map.generation;

        map.clear();

        assert!(map.is_empty());
        assert!(map.generation > gen_before);
    }

    #[test]
    fn clear_on_empty_map_is_noop() {
        let mut map = DecorationMap::new();
        let gen = map.generation;

        map.clear();

        assert_eq!(map.generation, gen);
    }

    #[test]
    fn adjust_for_insert_shifts_decorations_forward() {
        let mut rope = Rope::from_str("let x = 1;\n");
        let mut map = DecorationMap::new();
        // Decoration at char offset 5 (the 'x' position)
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(5, ": i32", DecorationSource::InlayHint)],
            &rope,
        );

        // Insert "foo" (3 chars) at offset 0
        rope.insert(0, "foo");
        map.adjust_for_edits(
            &[Edit::Insert {
                offset: 0,
                text: "foo".to_string(),
            }],
            &rope,
            1,
        );

        // Decoration should now be at offset 8 (5 + 3)
        let decs = map.for_line(0);
        assert_eq!(decs.len(), 1);
        assert_eq!(decs[0].placement.char_offset(), 8);
        assert_eq!(
            decs[0].source_version, 1,
            "adjust_for_edits should bump source_version to the new buffer version"
        );
    }

    #[test]
    fn adjust_for_delete_shifts_decorations_back() {
        let mut rope = Rope::from_str("foobar = 1;\n");
        let mut map = DecorationMap::new();
        // Decoration at offset 8 (the '1' position)
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(8, ": i32", DecorationSource::InlayHint)],
            &rope,
        );

        // Delete "foo" (3 chars) at offset 0
        rope.remove(0..3);
        map.adjust_for_edits(
            &[Edit::Delete {
                offset: 0,
                text: "foo".to_string(),
            }],
            &rope,
            1,
        );

        // Decoration should now be at offset 5 (8 - 3)
        let decs = map.for_line(0);
        assert_eq!(decs.len(), 1);
        assert_eq!(decs[0].placement.char_offset(), 5);
    }

    #[test]
    fn adjust_for_delete_removes_decoration_inside_deleted_region() {
        let mut rope = Rope::from_str("let x = 1;\n");
        let mut map = DecorationMap::new();
        // Decoration at offset 5 (inside the region we'll delete)
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(5, ": i32", DecorationSource::InlayHint)],
            &rope,
        );

        // Delete "x = 1" (5 chars) at offset 4
        rope.remove(4..9);
        map.adjust_for_edits(
            &[Edit::Delete {
                offset: 4,
                text: "x = 1".to_string(),
            }],
            &rope,
            1,
        );

        // Decoration was inside deleted region — should be gone
        assert!(map.is_empty());
    }

    #[test]
    fn iter_all_yields_every_decoration_in_line_order() {
        let rope = test_rope();
        let mut map = DecorationMap::new();
        // Two inline hints on line 0, one EOL diagnostic on line 1.
        map.replace_source(
            DecorationSource::InlayHint,
            vec![
                inline_at(3, "a", DecorationSource::InlayHint),
                inline_at(7, "b", DecorationSource::InlayHint),
            ],
            &rope,
        );
        map.replace_source(
            DecorationSource::Diagnostic,
            vec![eol_at(11, "err", DecorationSource::Diagnostic)],
            &rope,
        );

        let collected: Vec<(usize, String)> = map
            .iter_all()
            .map(|(line, d)| (line, d.text.clone()))
            .collect();

        // BTreeMap walks lines in ascending order; within a line, the sort
        // order established by `sort_all_lines` is preserved.
        assert_eq!(collected.len(), 3);
        assert_eq!(collected[0], (0, "a".to_string()));
        assert_eq!(collected[1], (0, "b".to_string()));
        assert_eq!(collected[2], (1, "err".to_string()));
    }

    #[test]
    fn iter_all_empty_map_is_empty() {
        let map = DecorationMap::new();
        assert_eq!(map.iter_all().count(), 0);
    }

    #[test]
    fn adjust_for_newline_insert_moves_decoration_to_new_line() {
        let mut rope = Rope::from_str("let x = 1;\n");
        let mut map = DecorationMap::new();
        // Decoration at offset 5
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(5, ": i32", DecorationSource::InlayHint)],
            &rope,
        );

        // Insert newline at offset 4 (before "x"), splitting the line
        rope.insert(4, "\n");
        map.adjust_for_edits(
            &[Edit::Insert {
                offset: 4,
                text: "\n".to_string(),
            }],
            &rope,
            1,
        );

        // Decoration should now be on line 1 (offset 6 = 5+1)
        assert!(map.for_line(0).is_empty());
        assert_eq!(map.for_line(1).len(), 1);
        assert_eq!(map.for_line(1)[0].placement.char_offset(), 6);
    }

    #[test]
    fn decorations_from_inlay_hints_sets_source_version() {
        let rope = Rope::from_str("let x = 1;\n");
        let hints = vec![lsp_types::InlayHint {
            position: lsp_types::Position::new(0, 5),
            label: lsp_types::InlayHintLabel::String(": i32".to_string()),
            kind: None,
            text_edits: None,
            tooltip: None,
            padding_left: None,
            padding_right: None,
            data: None,
        }];

        let decs = decorations_from_inlay_hints(
            &hints,
            &rope,
            |line_idx| {
                if line_idx < rope.len_lines() {
                    rope.line(line_idx)
                        .to_string()
                        .trim_end_matches('\n')
                        .to_string()
                } else {
                    String::new()
                }
            },
            42,
        );

        assert_eq!(decs.len(), 1);
        assert_eq!(
            decs[0].source_version, 42,
            "hints should carry the request's buffer_version forward"
        );
        assert_eq!(decs[0].placement.char_offset(), 5);
        assert!(matches!(
            decs[0].placement,
            DecorationPlacement::Inline { .. }
        ));
    }

    // -----------------------------------------------------------------
    // project_offset — pure projection, parity with adjust_for_edits
    // -----------------------------------------------------------------

    fn ins(offset: usize, text: &str) -> Edit {
        Edit::Insert {
            offset,
            text: text.to_string(),
        }
    }

    fn del(offset: usize, text: &str) -> Edit {
        Edit::Delete {
            offset,
            text: text.to_string(),
        }
    }

    fn refs(edits: &[Edit]) -> Vec<&Edit> {
        edits.iter().collect()
    }

    #[test]
    fn project_offset_empty_edits_is_identity() {
        assert_eq!(project_offset(5, &[]), Some(5));
        assert_eq!(project_offset(0, &[]), Some(0));
        assert_eq!(project_offset(usize::MAX / 2, &[]), Some(usize::MAX / 2));
    }

    #[test]
    fn project_offset_insert_before_shifts_forward() {
        let edits = [ins(0, "abc")];
        // Anchor at 5, insert 3 chars at 0 → anchor moves to 8.
        assert_eq!(project_offset(5, &refs(&edits)), Some(8));
    }

    #[test]
    fn project_offset_insert_after_anchor_unchanged() {
        let edits = [ins(10, "abc")];
        // Anchor at 5, insert at 10 (strictly after) → unchanged.
        assert_eq!(project_offset(5, &refs(&edits)), Some(5));
    }

    #[test]
    fn project_offset_insert_at_anchor_shifts_forward() {
        // Inclusive-before semantics: insertion at the anchor offset shifts
        // the anchor forward. This matches `adjust_for_edits` (`off >= offset`).
        let edits = [ins(5, "X")];
        assert_eq!(project_offset(5, &refs(&edits)), Some(6));
    }

    #[test]
    fn project_offset_delete_before_shifts_back() {
        let edits = [del(0, "abc")];
        // Anchor at 10, delete 3 chars from [0, 3) → anchor moves to 7.
        assert_eq!(project_offset(10, &refs(&edits)), Some(7));
    }

    #[test]
    fn project_offset_delete_after_anchor_unchanged() {
        let edits = [del(20, "abc")];
        // Anchor at 10, delete at 20 → unchanged.
        assert_eq!(project_offset(10, &refs(&edits)), Some(10));
    }

    #[test]
    fn project_offset_delete_strictly_inside_returns_none() {
        // Delete [4, 9) — anchor at 5, 6, 7, 8 is strictly inside → None.
        let edits = [del(4, "x = 1")];
        assert_eq!(project_offset(5, &refs(&edits)), None);
        assert_eq!(project_offset(6, &refs(&edits)), None);
        assert_eq!(project_offset(8, &refs(&edits)), None);
    }

    #[test]
    fn project_offset_delete_at_start_boundary_unchanged() {
        // Delete [4, 9) — anchor at 4 (equal to start) is treated as "before
        // the delete" and kept unchanged, matching adjust_for_edits' branch
        // ordering (pos >= end false, pos > offset false → else branch).
        let edits = [del(4, "x = 1")];
        assert_eq!(project_offset(4, &refs(&edits)), Some(4));
    }

    #[test]
    fn project_offset_delete_at_end_boundary_shifts_back() {
        // Delete [4, 9) — anchor at 9 (equal to end) passes the `>= end`
        // branch and shifts back by the delete length.
        let edits = [del(4, "x = 1")];
        assert_eq!(project_offset(9, &refs(&edits)), Some(4));
    }

    #[test]
    fn project_offset_multi_edit_composes_left_to_right() {
        // Anchor at 10.
        // 1. Insert "ab" at 0 → anchor at 12.
        // 2. Delete "xy" at [5, 7) → anchor at 12 (past end, shift back by 2) = 10.
        // 3. Insert "c" at 10 → anchor at 11 (inclusive-before).
        let edits = [ins(0, "ab"), del(5, "xy"), ins(10, "c")];
        assert_eq!(project_offset(10, &refs(&edits)), Some(11));
    }

    #[test]
    fn project_offset_multi_edit_drops_when_later_delete_engulfs() {
        // Anchor at 5. First edit leaves it there; second deletes [3, 8) which
        // engulfs it.
        let edits = [ins(20, "tail"), del(3, "abcde")];
        assert_eq!(project_offset(5, &refs(&edits)), None);
    }

    #[test]
    fn project_offset_matches_adjust_for_edits_on_insert() {
        // Build a decoration, run adjust_for_edits, and verify project_offset
        // produces the same offset starting from the pre-edit source offset.
        let mut rope = Rope::from_str("let x = 1;\n");
        let mut map = DecorationMap::new();
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(5, ": i32", DecorationSource::InlayHint)],
            &rope,
        );

        let source_offset = 5usize;
        let edits = vec![Edit::Insert {
            offset: 0,
            text: "foo".to_string(),
        }];

        // Apply to the rope and the accumulator.
        rope.insert(0, "foo");
        map.adjust_for_edits(&edits, &rope, 1);

        let accumulator_offset = map.for_line(0)[0].placement.char_offset();
        let projected = project_offset(source_offset, &refs(&edits)).expect("projection survives");
        assert_eq!(
            projected, accumulator_offset,
            "projection must match accumulator for a forward insert"
        );
    }

    #[test]
    fn project_offset_matches_adjust_for_edits_on_delete_before() {
        let mut rope = Rope::from_str("foobar = 1;\n");
        let mut map = DecorationMap::new();
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(8, ": i32", DecorationSource::InlayHint)],
            &rope,
        );

        let source_offset = 8usize;
        let edits = vec![Edit::Delete {
            offset: 0,
            text: "foo".to_string(),
        }];

        rope.remove(0..3);
        map.adjust_for_edits(&edits, &rope, 1);

        let accumulator_offset = map.for_line(0)[0].placement.char_offset();
        let projected = project_offset(source_offset, &refs(&edits)).expect("projection survives");
        assert_eq!(projected, accumulator_offset);
    }

    #[test]
    fn project_offset_matches_adjust_for_edits_on_delete_over() {
        // Accumulator drops a decoration whose anchor sits strictly inside the
        // deleted region; projection should return None for the same offset.
        let mut rope = Rope::from_str("let x = 1;\n");
        let mut map = DecorationMap::new();
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline_at(5, ": i32", DecorationSource::InlayHint)],
            &rope,
        );

        let source_offset = 5usize;
        let edits = vec![Edit::Delete {
            offset: 4,
            text: "x = 1".to_string(),
        }];

        rope.remove(4..9);
        map.adjust_for_edits(&edits, &rope, 1);

        assert!(map.is_empty(), "accumulator dropped the decoration");
        assert_eq!(
            project_offset(source_offset, &refs(&edits)),
            None,
            "projection agrees: decoration was engulfed"
        );
    }

    #[test]
    fn decorations_from_diagnostics_sets_source_version() {
        let rope = Rope::from_str("let x = 1;\n");
        let diags = vec![lsp_types::Diagnostic {
            range: lsp_types::Range::new(
                lsp_types::Position::new(0, 4),
                lsp_types::Position::new(0, 5),
            ),
            severity: Some(lsp_types::DiagnosticSeverity::ERROR),
            message: "bad".to_string(),
            ..lsp_types::Diagnostic::default()
        }];

        let decs = decorations_from_diagnostics(&diags, &rope, 7);

        assert_eq!(decs.len(), 1);
        assert_eq!(
            decs[0].source_version, 7,
            "diagnostics should carry the refresh's buffer_version forward"
        );
        assert!(matches!(
            decs[0].placement,
            DecorationPlacement::EndOfLine { .. }
        ));
    }
}
