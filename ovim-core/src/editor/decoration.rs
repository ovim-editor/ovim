use crate::color::Color;
use crate::edit::Edit;
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
    pub fn adjust_for_edits(&mut self, edits: &[Edit], rope: &Rope) {
        if self.lines.is_empty() || edits.is_empty() {
            return;
        }

        // Collect all decorations, adjust offsets, then rebuild index.
        let mut all_decs: Vec<Decoration> = self
            .lines
            .values_mut()
            .flat_map(|v| v.drain(..))
            .collect();
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
pub fn decorations_from_inlay_hints<F>(
    hints: &[lsp_types::InlayHint],
    rope: &Rope,
    line_text: F,
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
pub fn decorations_from_diagnostics(
    diagnostics: &[lsp_types::Diagnostic],
    rope: &Rope,
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
            vec![eol_at(0, "error: unused variable", DecorationSource::Diagnostic)],
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
                inline_at(3, ": i32", DecorationSource::InlayHint),     // 5 cols at char 3
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
        );

        // Decoration should now be at offset 8 (5 + 3)
        let decs = map.for_line(0);
        assert_eq!(decs.len(), 1);
        assert_eq!(decs[0].placement.char_offset(), 8);
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
        );

        // Decoration was inside deleted region — should be gone
        assert!(map.is_empty());
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
        );

        // Decoration should now be on line 1 (offset 6 = 5+1)
        assert!(map.for_line(0).is_empty());
        assert_eq!(map.for_line(1).len(), 1);
        assert_eq!(map.for_line(1)[0].placement.char_offset(), 6);
    }
}
