use crate::color::Color;
use std::collections::BTreeMap;

/// Where a decoration appears relative to buffer text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecorationPlacement {
    /// Inserted inline at a buffer position. Affects display width,
    /// cursor positioning, and wrap calculation.
    Inline {
        line: usize,
        /// Character index within the line (original text space, pre-expansion).
        char_idx: usize,
    },
    /// Appended after the code text on a line. Does not affect cursor.
    /// Rendered after wrapping (first visual row only).
    EndOfLine { line: usize },
}

impl DecorationPlacement {
    pub fn line(&self) -> usize {
        match self {
            Self::Inline { line, .. } | Self::EndOfLine { line } => *line,
        }
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
    /// Returns true if the set actually changed.
    pub fn replace_source(
        &mut self,
        source: DecorationSource,
        decorations: Vec<Decoration>,
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

        // Insert new decorations.
        for dec in decorations {
            let line = dec.placement.line();
            self.lines.entry(line).or_default().push(dec);
        }

        // Sort each affected line by position then priority.
        for line_decs in self.lines.values_mut() {
            line_decs.sort_by(|a, b| {
                let pos_a = match &a.placement {
                    DecorationPlacement::Inline { char_idx, .. } => (0, *char_idx),
                    DecorationPlacement::EndOfLine { .. } => (1, 0),
                };
                let pos_b = match &b.placement {
                    DecorationPlacement::Inline { char_idx, .. } => (0, *char_idx),
                    DecorationPlacement::EndOfLine { .. } => (1, 0),
                };
                pos_a.cmp(&pos_b).then(a.priority.cmp(&b.priority))
            });
        }

        if any_removed || any_added {
            self.generation = self.generation.wrapping_add(1);
            true
        } else {
            false
        }
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
    pub fn inline_decorations_for_line(&self, line: usize) -> Vec<(usize, usize)> {
        self.for_line(line)
            .iter()
            .filter_map(|d| match &d.placement {
                DecorationPlacement::Inline { char_idx, .. } => Some((*char_idx, d.display_width)),
                _ => None,
            })
            .collect()
    }

    /// Total display width of inline decorations on a line
    /// at or before the given char index.
    pub fn inline_width_before(&self, line: usize, char_idx: usize) -> usize {
        self.for_line(line)
            .iter()
            .filter_map(|d| match &d.placement {
                DecorationPlacement::Inline {
                    char_idx: idx, ..
                } if *idx <= char_idx => Some(d.display_width),
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

}

// ---------------------------------------------------------------------------
// LSP → Decoration conversion helpers
// ---------------------------------------------------------------------------

/// Convert LSP inlay hints into inline decorations.
///
/// `line_text` returns the text of a given line (without trailing newline).
/// Used to convert LSP UTF-16 offsets to char indices at creation time,
/// so every downstream consumer (cursor, WrapMap, renderer) sees char indices.
pub fn decorations_from_inlay_hints<F>(
    hints: &[lsp_types::InlayHint],
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
            // Convert UTF-16 offset → char index at creation time.
            let text_for_line = line_text(line);
            let char_idx = crate::lsp::utf16_to_char_col(&text_for_line, utf16_col);

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
                placement: DecorationPlacement::Inline { line, char_idx },
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

/// Convert LSP diagnostics into end-of-line decorations.
///
/// Only the first (highest-severity) diagnostic per line is converted,
/// matching the existing renderer behavior.
pub fn decorations_from_diagnostics(diagnostics: &[lsp_types::Diagnostic]) -> Vec<Decoration> {
    use std::collections::HashMap;

    // Group by line, keep highest severity per line.
    let mut best_per_line: HashMap<usize, &lsp_types::Diagnostic> = HashMap::new();
    for diag in diagnostics {
        let line = diag.range.start.line as usize;
        let entry = best_per_line.entry(line).or_insert(diag);
        // Lower severity number = higher severity (ERROR=1, WARNING=2, etc.)
        // DiagnosticSeverity is a newtype: ERROR=1, WARNING=2, INFO=3, HINT=4
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

            Decoration {
                placement: DecorationPlacement::EndOfLine { line },
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

    fn inline(line: usize, char_idx: usize, text: &str, source: DecorationSource) -> Decoration {
        let display_width = text.len(); // ASCII for tests
        Decoration {
            placement: DecorationPlacement::Inline { line, char_idx },
            source,
            text: text.to_string(),
            display_width,
            style: DecorationStyle::new(Color::Gray).with_italic(),
            priority: 0,
        }
    }

    fn eol(line: usize, text: &str, source: DecorationSource) -> Decoration {
        let display_width = text.len();
        Decoration {
            placement: DecorationPlacement::EndOfLine { line },
            source,
            text: text.to_string(),
            display_width,
            style: DecorationStyle::new(Color::Red),
            priority: 0,
        }
    }

    #[test]
    fn replace_source_inserts_and_retrieves() {
        let mut map = DecorationMap::new();
        let gen_before = map.generation;

        map.replace_source(
            DecorationSource::InlayHint,
            vec![
                inline(0, 5, ": String", DecorationSource::InlayHint),
                inline(2, 10, "count: ", DecorationSource::InlayHint),
            ],
        );

        assert_eq!(map.for_line(0).len(), 1);
        assert_eq!(map.for_line(0)[0].text, ": String");
        assert_eq!(map.for_line(2).len(), 1);
        assert!(map.for_line(1).is_empty());
        assert!(map.generation > gen_before);
    }

    #[test]
    fn replace_source_replaces_only_same_source() {
        let mut map = DecorationMap::new();

        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline(0, 5, ": String", DecorationSource::InlayHint)],
        );
        map.replace_source(
            DecorationSource::Diagnostic,
            vec![eol(0, "error: unused variable", DecorationSource::Diagnostic)],
        );

        assert_eq!(map.for_line(0).len(), 2);

        // Replace inlay hints with new set
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline(0, 3, ": i32", DecorationSource::InlayHint)],
        );

        assert_eq!(map.for_line(0).len(), 2);
        // Inline (InlayHint) should be first, then EOL (Diagnostic)
        assert_eq!(map.for_line(0)[0].text, ": i32");
        assert_eq!(map.for_line(0)[1].text, "error: unused variable");
    }

    #[test]
    fn replace_source_with_empty_removes_all_from_source() {
        let mut map = DecorationMap::new();

        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline(0, 5, ": String", DecorationSource::InlayHint)],
        );
        let gen_before = map.generation;

        map.replace_source(DecorationSource::InlayHint, vec![]);

        assert!(map.for_line(0).is_empty());
        assert!(map.generation > gen_before);
    }

    #[test]
    fn no_op_replace_does_not_bump_generation() {
        let mut map = DecorationMap::new();
        let gen = map.generation;

        map.replace_source(DecorationSource::Diagnostic, vec![]);

        assert_eq!(map.generation, gen);
    }

    #[test]
    fn inline_width_before() {
        let mut map = DecorationMap::new();
        map.replace_source(
            DecorationSource::InlayHint,
            vec![
                inline(0, 3, ": i32", DecorationSource::InlayHint),   // 5 cols at char 3
                inline(0, 10, ": String", DecorationSource::InlayHint), // 8 cols at char 10
            ],
        );

        // Before any hint
        assert_eq!(map.inline_width_before(0, 2), 0);
        // At the first hint
        assert_eq!(map.inline_width_before(0, 3), 5);
        // Between hints
        assert_eq!(map.inline_width_before(0, 7), 5);
        // At the second hint
        assert_eq!(map.inline_width_before(0, 10), 13); // 5 + 8
        // Different line
        assert_eq!(map.inline_width_before(1, 10), 0);
    }

    #[test]
    fn inline_for_line_and_eol_for_line() {
        let mut map = DecorationMap::new();
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline(0, 5, ": String", DecorationSource::InlayHint)],
        );
        map.replace_source(
            DecorationSource::Diagnostic,
            vec![eol(0, "unused variable", DecorationSource::Diagnostic)],
        );

        assert_eq!(map.inline_for_line(0).len(), 1);
        assert_eq!(map.eol_for_line(0).len(), 1);
    }

    #[test]
    fn sort_order_inline_before_eol_then_by_position() {
        let mut map = DecorationMap::new();
        map.replace_source(
            DecorationSource::Diagnostic,
            vec![eol(0, "error", DecorationSource::Diagnostic)],
        );
        map.replace_source(
            DecorationSource::InlayHint,
            vec![
                inline(0, 10, "b_hint", DecorationSource::InlayHint),
                inline(0, 3, "a_hint", DecorationSource::InlayHint),
            ],
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
        let mut map = DecorationMap::new();
        map.replace_source(
            DecorationSource::InlayHint,
            vec![inline(0, 5, "hint", DecorationSource::InlayHint)],
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
}
