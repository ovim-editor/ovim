//! View-independent visual-row geometry for one indexed logical line.
//!
//! The layout owns the immutable [`LineIndex`] snapshot used to measure it.
//! Plain printable ASCII lines without inline virtual text use arithmetic only.
//! Rich lines retain compact row plans whose source text is materialized only
//! when a frontend asks for particular rows.

use crate::display::control_char_caret;
use crate::text_index::LineIndex;
use crate::unicode::GraphemeCol;
use std::ops::Range;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualPosition {
    pub row: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    pub bytes: Range<usize>,
    pub chars: Range<usize>,
    pub graphemes: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutFragmentKind {
    Text,
    Tab,
    Control,
    /// Virtual text identified by its position in the constructor's sorted
    /// `inline_widths` input. `cell_offset` supports decorations split by wrap.
    InlineDecoration {
        index: usize,
        cell_offset: usize,
    },
    Padding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutFragment {
    pub text: String,
    pub cells: usize,
    pub row_column: usize,
    /// Column in the flat composed line. Wide-character wrap padding is not
    /// part of this coordinate space.
    pub display_start: usize,
    pub source: Option<SourceSpan>,
    pub kind: LayoutFragmentKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutRow {
    pub index: usize,
    pub display_start: usize,
    pub display_end: usize,
    pub fragments: Vec<LayoutFragment>,
}

/// Bounded nowrap projection for source text without inline virtual text.
/// The line index seeks to the left display edge, and this function visits
/// only graphemes intersecting the requested window.
pub fn source_fragments_for_display_range(
    line: &LineIndex,
    tab_width: usize,
    requested: Range<usize>,
) -> Vec<LayoutFragment> {
    if requested.start >= requested.end {
        return Vec::new();
    }
    let tab_width = tab_width.max(1);
    let slice = line.visible_slice(requested.start, requested.end - requested.start, tab_width);
    let temporary = LineIndex::from_text(&slice.text);
    let mut output = Vec::new();
    for grapheme in temporary.graphemes_from(GraphemeCol(0)) {
        let char_start = slice.char_start + grapheme.char_start;
        let grapheme_start = slice.grapheme_start.0 + grapheme.grapheme_col.0;
        let display_start = line.char_to_display(char_start, tab_width);
        let source = SourceSpan {
            bytes: slice.byte_start + grapheme.byte_start
                ..slice.byte_start + grapheme.byte_start + grapheme.text.len(),
            chars: char_start..char_start + grapheme.text.chars().count(),
            graphemes: grapheme_start..grapheme_start + 1,
        };
        let (text, cells, kind, atomic) = if grapheme.text == "\t" {
            let full = tab_width - (display_start % tab_width);
            let start = display_start.max(requested.start);
            let end = display_start.saturating_add(full).min(requested.end);
            if start >= end {
                continue;
            }
            (
                " ".repeat(end - start),
                end - start,
                LayoutFragmentKind::Tab,
                false,
            )
        } else if grapheme.text.chars().count() == 1
            && grapheme
                .text
                .chars()
                .next()
                .and_then(control_char_caret)
                .is_some()
        {
            (
                grapheme
                    .text
                    .chars()
                    .next()
                    .and_then(control_char_caret)
                    .unwrap()
                    .into_iter()
                    .collect(),
                2,
                LayoutFragmentKind::Control,
                true,
            )
        } else {
            (
                grapheme.text.into_owned(),
                line.char_to_display(source.chars.end, tab_width) - display_start,
                LayoutFragmentKind::Text,
                true,
            )
        };
        let actual_start = if atomic {
            display_start
        } else {
            display_start.max(requested.start)
        };
        let actual_end = actual_start.saturating_add(cells);
        if (cells > 0 && actual_end <= requested.start)
            || actual_start >= requested.end
            || (cells == 0 && actual_start < requested.start)
        {
            continue;
        }
        push_materialized_fragment(
            &mut output,
            LayoutFragment {
                text,
                cells,
                row_column: actual_start.saturating_sub(requested.start),
                display_start: actual_start,
                source: Some(source),
                kind,
            },
        );
    }
    output
}

#[derive(Debug, Clone)]
struct FragmentPlan {
    cells: usize,
    row_column: usize,
    display_start: usize,
    source: Option<SourceSpan>,
    kind: LayoutFragmentKind,
}

#[derive(Debug, Clone)]
struct RowPlan {
    display_start: usize,
    display_end: usize,
    fragments: Vec<FragmentPlan>,
}

#[derive(Debug, Clone)]
enum Geometry {
    PlainAscii { len: usize },
    Rich { rows: Vec<RowPlan> },
}

#[derive(Debug, Clone)]
struct InlineRun {
    anchor: usize,
    width: usize,
    original_index: usize,
    text: Option<Arc<str>>,
}

/// Cached wrap geometry for one immutable line snapshot and one width tuple.
/// Inline positions are source character columns and must be sorted by column.
#[derive(Debug, Clone)]
pub struct IndexedLineLayout {
    line: Arc<LineIndex>,
    width: usize,
    tab_width: usize,
    inline: Arc<[InlineRun]>,
    geometry: Geometry,
}

impl IndexedLineLayout {
    pub fn new(
        line: Arc<LineIndex>,
        width: usize,
        tab_width: usize,
        inline_widths: Arc<[(usize, usize)]>,
    ) -> Self {
        let width = width.max(1);
        let tab_width = tab_width.max(1);
        let mut inline: Vec<InlineRun> = inline_widths
            .iter()
            .copied()
            .enumerate()
            .map(|(original_index, (anchor, width))| InlineRun {
                anchor,
                width,
                original_index,
                text: None,
            })
            .collect();
        inline.sort_by_key(|run| run.anchor);
        Self::from_runs(line, width, tab_width, inline.into())
    }

    pub fn with_inline_text(
        line: Arc<LineIndex>,
        width: usize,
        tab_width: usize,
        inline_text: Arc<[(usize, Arc<str>)]>,
    ) -> Self {
        let mut inline: Vec<InlineRun> = inline_text
            .iter()
            .enumerate()
            .map(|(original_index, (anchor, text))| InlineRun {
                anchor: *anchor,
                width: text
                    .graphemes(true)
                    .map(crate::display::grapheme_display_width)
                    .sum(),
                original_index,
                text: Some(text.clone()),
            })
            .collect();
        inline.sort_by_key(|run| run.anchor);
        Self::from_runs(line, width, tab_width, inline.into())
    }

    fn from_runs(
        line: Arc<LineIndex>,
        width: usize,
        tab_width: usize,
        inline: Arc<[InlineRun]>,
    ) -> Self {
        let width = width.max(1);
        let tab_width = tab_width.max(1);
        let geometry = if line.is_plain_ascii() && inline.is_empty() {
            Geometry::PlainAscii {
                len: line.len_chars(),
            }
        } else {
            Geometry::Rich {
                rows: build_rich_rows(&line, width, tab_width, &inline),
            }
        };
        Self {
            line,
            width,
            tab_width,
            inline,
            geometry,
        }
    }

    pub fn line(&self) -> &Arc<LineIndex> {
        &self.line
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn tab_width(&self) -> usize {
        self.tab_width
    }

    /// Whether inline virtual text (an inlay hint) is part of this layout.
    pub fn has_inline(&self) -> bool {
        !self.inline.is_empty()
    }

    pub fn row_count(&self) -> usize {
        match &self.geometry {
            Geometry::PlainAscii { len } => (*len).max(1).div_ceil(self.width),
            Geometry::Rich { rows } => rows.len(),
        }
    }

    /// Locate a source character position in wrapped visual coordinates.
    /// Positions inside a grapheme snap according to [`LineIndex`]. Inline
    /// virtual text anchored before the character contributes to the result.
    pub fn position_for_char(&self, char_column: usize) -> VisualPosition {
        let char_column = char_column.min(self.line.len_chars());
        let display = self.line.char_to_display(char_column, self.tab_width)
            + self.inline_width_before(char_column);
        match &self.geometry {
            Geometry::PlainAscii { .. } => VisualPosition {
                row: display / self.width,
                column: display % self.width,
            },
            Geometry::Rich { rows } => {
                let row = rows
                    .partition_point(|candidate| candidate.display_start <= display)
                    .saturating_sub(1);
                let plan = &rows[row];
                let mut column = 0;
                for fragment in &plan.fragments {
                    if fragment.display_start > display {
                        break;
                    }
                    column = fragment.row_column
                        + display
                            .saturating_sub(fragment.display_start)
                            .min(fragment.cells);
                }
                if column >= self.width {
                    VisualPosition {
                        row: row + 1,
                        column: 0,
                    }
                } else {
                    VisualPosition { row, column }
                }
            }
        }
    }

    /// Flat composed display-column range occupied by a visual row. Padding
    /// introduced when a wide grapheme moves to the next row is excluded.
    pub fn display_range_for_row(&self, row: usize) -> Option<Range<usize>> {
        match &self.geometry {
            Geometry::PlainAscii { len } => {
                if row >= self.row_count() {
                    return None;
                }
                let start = row.saturating_mul(self.width);
                Some(start..start.saturating_add(self.width).min(*len))
            }
            Geometry::Rich { rows } => rows
                .get(row)
                .map(|plan| plan.display_start..plan.display_end),
        }
    }

    /// Materialize only the requested visual rows. Inline decoration fragments
    /// carry geometry and an ordinal; the frontend supplies their styled text.
    pub fn row_fragments(&self, requested: Range<usize>) -> Vec<LayoutRow> {
        let end = requested.end.min(self.row_count());
        if requested.start >= end {
            return Vec::new();
        }
        match &self.geometry {
            Geometry::PlainAscii { len } => (requested.start..end)
                .map(|index| {
                    let start = index * self.width;
                    let source_end = (start + self.width).min(*len);
                    let mut fragments = Vec::with_capacity(2);
                    if start < source_end {
                        fragments.push(LayoutFragment {
                            text: self.line.slice_chars(start..source_end),
                            cells: source_end - start,
                            row_column: 0,
                            display_start: start,
                            source: Some(SourceSpan {
                                bytes: self.line.char_to_byte(start)
                                    ..self.line.char_to_byte(source_end),
                                chars: start..source_end,
                                graphemes: start..source_end,
                            }),
                            kind: LayoutFragmentKind::Text,
                        });
                    }
                    push_materialized_padding(
                        &mut fragments,
                        source_end - start,
                        self.width,
                        source_end,
                    );
                    LayoutRow {
                        index,
                        display_start: start,
                        display_end: source_end,
                        fragments,
                    }
                })
                .collect(),
            Geometry::Rich { rows } => rows[requested.start..end]
                .iter()
                .enumerate()
                .map(|(offset, row)| LayoutRow {
                    index: requested.start + offset,
                    display_start: row.display_start,
                    display_end: row.display_end,
                    fragments: row
                        .fragments
                        .iter()
                        .map(|fragment| self.materialize(fragment))
                        .collect(),
                })
                .collect(),
        }
    }

    /// Materialize the fragments intersecting a flat composed-display range.
    /// This is the nowrap viewport path: row padding is ignored and the output
    /// is bounded by the requested window (apart from an atomic grapheme that
    /// straddles an edge).
    pub fn fragments_for_display_range(&self, requested: Range<usize>) -> Vec<LayoutFragment> {
        if requested.start >= requested.end {
            return Vec::new();
        }
        match &self.geometry {
            Geometry::PlainAscii { len } => {
                let start = requested.start.min(*len);
                let end = requested.end.min(*len).max(start);
                if start == end {
                    return Vec::new();
                }
                vec![LayoutFragment {
                    text: self.line.slice_chars(start..end),
                    cells: end - start,
                    row_column: start.saturating_sub(requested.start),
                    display_start: start,
                    source: Some(SourceSpan {
                        bytes: self.line.char_to_byte(start)..self.line.char_to_byte(end),
                        chars: start..end,
                        graphemes: start..end,
                    }),
                    kind: LayoutFragmentKind::Text,
                }]
            }
            Geometry::Rich { rows } => {
                let first = rows
                    .partition_point(|row| row.display_end <= requested.start)
                    .min(rows.len());
                let mut output = Vec::new();
                for row in &rows[first..] {
                    if row.display_start >= requested.end && row.display_end > row.display_start {
                        break;
                    }
                    for plan in &row.fragments {
                        if matches!(plan.kind, LayoutFragmentKind::Padding) {
                            continue;
                        }
                        let plan_end = plan.display_start.saturating_add(plan.cells);
                        if (plan.cells > 0 && plan_end <= requested.start)
                            || plan.display_start >= requested.end
                            || (plan.cells == 0 && plan.display_start < requested.start)
                        {
                            continue;
                        }
                        if let Some(fragment) = self.materialize_clipped(plan, &requested) {
                            output.push(fragment);
                        }
                    }
                }
                output
            }
        }
    }

    fn inline_width_before(&self, char_column: usize) -> usize {
        self.inline
            .iter()
            .filter(|run| {
                run.anchor < char_column
                    || (run.anchor == char_column && run.anchor < self.line.len_chars())
            })
            .map(|run| run.width)
            .sum()
    }

    fn materialize(&self, plan: &FragmentPlan) -> LayoutFragment {
        let text = match &plan.kind {
            LayoutFragmentKind::Text => plan
                .source
                .as_ref()
                .map(|source| self.line.slice_chars(source.chars.clone()))
                .unwrap_or_default(),
            LayoutFragmentKind::Tab | LayoutFragmentKind::Padding => " ".repeat(plan.cells),
            LayoutFragmentKind::Control => plan
                .source
                .as_ref()
                .and_then(|source| self.line.slice_chars(source.chars.clone()).chars().next())
                .and_then(control_char_caret)
                .map(|caret| caret.into_iter().collect())
                .unwrap_or_default(),
            LayoutFragmentKind::InlineDecoration { index, cell_offset } => self
                .inline
                .iter()
                .find(|run| run.original_index == *index)
                .and_then(|run| run.text.as_ref())
                .map(|text| inline_text_cells(text, *cell_offset, plan.cells))
                .unwrap_or_default(),
        };
        LayoutFragment {
            text,
            cells: plan.cells,
            row_column: plan.row_column,
            display_start: plan.display_start,
            source: plan.source.clone(),
            kind: plan.kind.clone(),
        }
    }

    fn materialize_clipped(
        &self,
        plan: &FragmentPlan,
        requested: &Range<usize>,
    ) -> Option<LayoutFragment> {
        let start = plan.display_start.max(requested.start);
        let end = plan
            .display_start
            .saturating_add(plan.cells)
            .min(requested.end);
        if start >= end
            && !(plan.cells == 0
                && matches!(plan.kind, LayoutFragmentKind::Text)
                && requested.contains(&plan.display_start))
        {
            return None;
        }
        let leading = start - plan.display_start;
        let mut fragment = self.materialize(plan);
        if plan.cells == 0 {
            fragment.row_column = plan.display_start.saturating_sub(requested.start);
            return Some(fragment);
        }
        match &mut fragment.kind {
            LayoutFragmentKind::Text => {
                let source = plan.source.as_ref()?;
                let content_start = self
                    .line
                    .char_to_display(source.chars.start, self.tab_width);
                let requested_content_start = content_start.saturating_add(leading);
                let requested_content_end = content_start.saturating_add(end - plan.display_start);
                let char_start = self
                    .line
                    .display_to_char(requested_content_start, self.tab_width)
                    .max(source.chars.start);
                let mut char_end = self
                    .line
                    .display_to_char(requested_content_end, self.tab_width)
                    .min(source.chars.end);
                if char_end < source.chars.end
                    && self.line.char_to_display(char_end, self.tab_width) < requested_content_end
                {
                    let grapheme = self
                        .line
                        .char_to_grapheme(crate::unicode::CharCol(char_end));
                    char_end = self
                        .line
                        .grapheme_to_char(GraphemeCol(grapheme.0 + 1))
                        .0
                        .min(source.chars.end);
                }
                if char_start >= char_end {
                    return None;
                }
                let actual_content_start = self.line.char_to_display(char_start, self.tab_width);
                let actual_content_end = self.line.char_to_display(char_end, self.tab_width);
                fragment.display_start =
                    plan.display_start + actual_content_start.saturating_sub(content_start);
                fragment.row_column = fragment.display_start.saturating_sub(requested.start);
                fragment.cells = actual_content_end.saturating_sub(actual_content_start);
                fragment.text = self.line.slice_chars(char_start..char_end);
                fragment.source = Some(SourceSpan {
                    bytes: self.line.char_to_byte(char_start)..self.line.char_to_byte(char_end),
                    chars: char_start..char_end,
                    graphemes: self
                        .line
                        .char_to_grapheme(crate::unicode::CharCol(char_start))
                        .0
                        ..self
                            .line
                            .char_to_grapheme(crate::unicode::CharCol(char_end))
                            .0,
                });
            }
            LayoutFragmentKind::Tab | LayoutFragmentKind::Padding => {
                fragment.text = " ".repeat(end - start);
                fragment.cells = end - start;
                fragment.display_start = start;
                fragment.row_column = start.saturating_sub(requested.start);
            }
            LayoutFragmentKind::InlineDecoration { index, cell_offset } => {
                let ordinal = *index;
                *cell_offset += leading;
                fragment.cells = end - start;
                fragment.display_start = start;
                fragment.row_column = start.saturating_sub(requested.start);
                fragment.text = self
                    .inline
                    .iter()
                    .find(|run| run.original_index == ordinal)
                    .and_then(|run| run.text.as_ref())
                    .map(|text| inline_text_cells(text, *cell_offset, fragment.cells))
                    .unwrap_or_default();
            }
            LayoutFragmentKind::Control => {
                // Caret notation is one atomic source grapheme, consistent
                // with wrap geometry and cursor placement.
                fragment.row_column = plan.display_start.saturating_sub(requested.start);
            }
        }
        Some(fragment)
    }
}

fn build_rich_rows(
    line: &LineIndex,
    width: usize,
    tab_width: usize,
    inline: &[InlineRun],
) -> Vec<RowPlan> {
    let mut builder = RowBuilder::new(width);
    let mut decoration = 0;
    for grapheme in line.graphemes_from(GraphemeCol(0)) {
        while decoration < inline.len() && inline[decoration].anchor <= grapheme.char_start {
            builder.push_inline(&inline[decoration]);
            decoration += 1;
        }

        let chars = grapheme.text.chars().count();
        let source = SourceSpan {
            bytes: grapheme.byte_start..grapheme.byte_start + grapheme.text.len(),
            chars: grapheme.char_start..grapheme.char_start + chars,
            graphemes: grapheme.grapheme_col.0..grapheme.grapheme_col.0 + 1,
        };
        if grapheme.text == "\t" {
            let cells = tab_width - (builder.content_display % tab_width);
            builder.push_tab(source, cells);
        } else if grapheme.text.chars().count() == 1
            && grapheme
                .text
                .chars()
                .next()
                .and_then(control_char_caret)
                .is_some()
        {
            builder.push_atomic(source, 2, LayoutFragmentKind::Control);
        } else {
            let cells = crate::display::grapheme_display_width(&grapheme.text);
            builder.push_atomic(source, cells, LayoutFragmentKind::Text);
        }
    }
    while decoration < inline.len() {
        builder.push_inline(&inline[decoration]);
        decoration += 1;
    }
    builder.finish()
}

struct RowBuilder {
    width: usize,
    rows: Vec<RowPlan>,
    row_column: usize,
    flat_display: usize,
    content_display: usize,
}

impl RowBuilder {
    fn new(width: usize) -> Self {
        Self {
            width,
            rows: vec![RowPlan {
                display_start: 0,
                display_end: 0,
                fragments: Vec::new(),
            }],
            row_column: 0,
            flat_display: 0,
            content_display: 0,
        }
    }

    fn next_row(&mut self) {
        self.rows.last_mut().unwrap().display_end = self.flat_display;
        self.rows.push(RowPlan {
            display_start: self.flat_display,
            display_end: self.flat_display,
            fragments: Vec::new(),
        });
        self.row_column = 0;
    }

    fn pad_current_row(&mut self) {
        let cells = self.width.saturating_sub(self.row_column);
        if cells > 0 {
            self.push_plan(FragmentPlan {
                cells,
                row_column: self.row_column,
                display_start: self.flat_display,
                source: None,
                kind: LayoutFragmentKind::Padding,
            });
            self.row_column += cells;
        }
    }

    fn push_inline(&mut self, run: &InlineRun) {
        if let Some(text) = &run.text {
            let mut cell_offset = 0;
            for grapheme in text.graphemes(true) {
                let cells = crate::display::grapheme_display_width(grapheme);
                if cells <= 1 {
                    self.push_inline_cells(run.original_index, cell_offset, cells);
                } else {
                    if self.row_column.saturating_add(cells) > self.width {
                        self.pad_current_row();
                        self.next_row();
                    }
                    self.push_plan(FragmentPlan {
                        cells,
                        row_column: self.row_column,
                        display_start: self.flat_display,
                        source: None,
                        kind: LayoutFragmentKind::InlineDecoration {
                            index: run.original_index,
                            cell_offset,
                        },
                    });
                    self.row_column += cells;
                    self.flat_display += cells;
                }
                cell_offset += cells;
            }
        } else {
            self.push_inline_cells(run.original_index, 0, run.width);
        }
    }

    fn push_inline_cells(&mut self, index: usize, mut cell_offset: usize, mut cells: usize) {
        while cells > 0 {
            if self.row_column >= self.width {
                self.next_row();
            }
            let take = cells.min(self.width - self.row_column);
            self.push_plan(FragmentPlan {
                cells: take,
                row_column: self.row_column,
                display_start: self.flat_display,
                source: None,
                kind: LayoutFragmentKind::InlineDecoration { index, cell_offset },
            });
            self.row_column += take;
            self.flat_display += take;
            cell_offset += take;
            cells -= take;
        }
    }

    fn push_tab(&mut self, source: SourceSpan, mut cells: usize) {
        while cells > 0 {
            if self.row_column >= self.width {
                self.next_row();
            }
            let take = cells.min(self.width - self.row_column);
            self.push_plan(FragmentPlan {
                cells: take,
                row_column: self.row_column,
                display_start: self.flat_display,
                source: Some(source.clone()),
                kind: LayoutFragmentKind::Tab,
            });
            self.row_column += take;
            self.flat_display += take;
            self.content_display += take;
            cells -= take;
        }
    }

    fn push_atomic(&mut self, source: SourceSpan, cells: usize, kind: LayoutFragmentKind) {
        // Keep the established wrap oracle for a glyph wider than the entire
        // viewport: it moves to a new row even when the current row is empty.
        if self.row_column.saturating_add(cells) > self.width {
            self.pad_current_row();
            self.next_row();
        }
        self.push_plan(FragmentPlan {
            cells,
            row_column: self.row_column,
            display_start: self.flat_display,
            source: Some(source),
            kind,
        });
        self.row_column += cells;
        self.flat_display += cells;
        self.content_display += cells;
    }

    fn push_plan(&mut self, plan: FragmentPlan) {
        let row = self.rows.last_mut().unwrap();
        if matches!(plan.kind, LayoutFragmentKind::Text)
            && let Some(previous) = row.fragments.last_mut()
            && matches!(previous.kind, LayoutFragmentKind::Text)
            && previous.row_column + previous.cells == plan.row_column
            && previous.display_start + previous.cells == plan.display_start
            && previous
                .source
                .as_ref()
                .zip(plan.source.as_ref())
                .is_some_and(|(a, b)| {
                    a.bytes.end == b.bytes.start
                        && a.chars.end == b.chars.start
                        && a.graphemes.end == b.graphemes.start
                })
        {
            previous.cells += plan.cells;
            let source = previous.source.as_mut().unwrap();
            let extension = plan.source.unwrap();
            source.bytes.end = extension.bytes.end;
            source.chars.end = extension.chars.end;
            source.graphemes.end = extension.graphemes.end;
            return;
        }
        row.fragments.push(plan);
    }

    fn finish(mut self) -> Vec<RowPlan> {
        self.rows.last_mut().unwrap().display_end = self.flat_display;
        self.rows
    }
}

fn push_materialized_padding(
    fragments: &mut Vec<LayoutFragment>,
    row_column: usize,
    width: usize,
    display_start: usize,
) {
    let cells = width.saturating_sub(row_column);
    if cells > 0 {
        fragments.push(LayoutFragment {
            text: " ".repeat(cells),
            cells,
            row_column,
            display_start,
            source: None,
            kind: LayoutFragmentKind::Padding,
        });
    }
}

fn push_materialized_fragment(output: &mut Vec<LayoutFragment>, fragment: LayoutFragment) {
    if matches!(fragment.kind, LayoutFragmentKind::Text)
        && let Some(previous) = output.last_mut()
        && matches!(previous.kind, LayoutFragmentKind::Text)
        && previous.display_start + previous.cells == fragment.display_start
        && previous
            .source
            .as_ref()
            .zip(fragment.source.as_ref())
            .is_some_and(|(a, b)| {
                a.bytes.end == b.bytes.start
                    && a.chars.end == b.chars.start
                    && a.graphemes.end == b.graphemes.start
            })
    {
        previous.text.push_str(&fragment.text);
        previous.cells += fragment.cells;
        let source = previous.source.as_mut().unwrap();
        let extension = fragment.source.unwrap();
        source.bytes.end = extension.bytes.end;
        source.chars.end = extension.chars.end;
        source.graphemes.end = extension.graphemes.end;
        return;
    }
    output.push(fragment);
}

fn inline_text_cells(text: &str, start: usize, cells: usize) -> String {
    let end = start.saturating_add(cells);
    let mut display = 0usize;
    let mut output = String::new();
    for grapheme in text.graphemes(true) {
        let width = crate::display::grapheme_display_width(grapheme);
        let grapheme_end = display.saturating_add(width);
        if grapheme_end > start && display < end {
            output.push_str(grapheme);
        }
        if display >= end {
            break;
        }
        display = grapheme_end;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii_geometry_is_arithmetic_and_rows_are_bounded() {
        let line = LineIndex::from_text("abcdefghij");
        let layout = IndexedLineLayout::new(line, 4, 4, Arc::from([]));
        assert_eq!(layout.row_count(), 3);
        assert_eq!(
            layout.position_for_char(6),
            VisualPosition { row: 1, column: 2 }
        );
        assert_eq!(layout.display_range_for_row(1), Some(4..8));
        let rows = layout.row_fragments(1..2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].fragments[0].text, "efgh");
    }

    #[test]
    fn tabs_split_across_rows_and_wide_graphemes_move_atomically() {
        let line = LineIndex::from_text("aa\t界b");
        let layout = IndexedLineLayout::new(line, 3, 4, Arc::from([]));
        let rows = layout.row_fragments(0..layout.row_count());
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0].fragments[1].kind, LayoutFragmentKind::Tab));
        assert!(matches!(rows[1].fragments[0].kind, LayoutFragmentKind::Tab));
        assert_eq!(
            layout.position_for_char(3),
            VisualPosition { row: 1, column: 1 }
        );
        assert_eq!(rows[1].fragments[1].text, "界");
        assert_eq!(rows[1].fragments[1].row_column, 1);
    }

    #[test]
    fn inline_widths_participate_in_wrap_without_materializing_text() {
        let line = LineIndex::from_text("abcd");
        let layout = IndexedLineLayout::new(line, 4, 4, Arc::from([(2, 3)]));
        assert_eq!(layout.row_count(), 2);
        assert_eq!(
            layout.position_for_char(2),
            VisualPosition { row: 1, column: 1 }
        );
        let rows = layout.row_fragments(0..2);
        assert!(rows.iter().flat_map(|row| &row.fragments).any(|fragment| {
            matches!(
                fragment.kind,
                LayoutFragmentKind::InlineDecoration { index: 0, .. }
            )
        }));
    }

    #[test]
    fn control_notation_keeps_source_coordinates() {
        let line = LineIndex::from_text("a\u{7f}b");
        let layout = IndexedLineLayout::new(line, 8, 4, Arc::from([]));
        let rows = layout.row_fragments(0..1);
        let control = rows[0]
            .fragments
            .iter()
            .find(|fragment| matches!(fragment.kind, LayoutFragmentKind::Control))
            .unwrap();
        assert_eq!(control.text, "^?");
        assert_eq!(control.source.as_ref().unwrap().chars, 1..2);
    }

    #[test]
    fn composed_display_window_materializes_only_intersecting_fragments() {
        let line = LineIndex::from_text("ab界cdef");
        let layout = IndexedLineLayout::new(line, 4, 4, Arc::from([(4, 2)]));
        let fragments = layout.fragments_for_display_range(2..7);
        assert_eq!(
            fragments
                .iter()
                .filter(|fragment| matches!(fragment.kind, LayoutFragmentKind::Text))
                .map(|fragment| fragment.text.as_str())
                .collect::<Vec<_>>(),
            vec!["界", "c"]
        );
        assert!(fragments.iter().any(|fragment| matches!(
            fragment.kind,
            LayoutFragmentKind::InlineDecoration { index: 0, .. }
        )));
    }

    #[test]
    fn source_display_window_snaps_wide_edges_and_clips_tabs() {
        let line = LineIndex::from_text("ab界\txyz");
        let fragments = source_fragments_for_display_range(&line, 4, 3..7);
        assert_eq!(fragments[0].text, "界");
        assert_eq!(fragments[0].display_start, 2);
        assert!(matches!(fragments[1].kind, LayoutFragmentKind::Tab));
        assert_eq!(fragments[1].text, "   ");
        assert_eq!(fragments[1].display_start, 4);
    }

    #[test]
    fn exact_fill_eol_position_advances_to_the_next_visual_row() {
        for text in ["abcd", "a\t", "ab界"] {
            let line = LineIndex::from_text(text);
            let end = line.len_chars();
            let layout = IndexedLineLayout::new(line, 4, 4, Arc::from([]));
            assert_eq!(
                layout.position_for_char(end),
                VisualPosition { row: 1, column: 0 },
                "{text:?}"
            );
        }
    }

    #[test]
    fn zero_width_source_cluster_survives_a_bounded_window() {
        let line = LineIndex::from_text("\u{301}a");
        let fragments = source_fragments_for_display_range(&line, 4, 0..1);
        assert!(fragments.first().is_some_and(|fragment| {
            fragment.text.starts_with('\u{301}')
                && fragment.source.as_ref().unwrap().chars.start == 0
        }));
    }

    #[test]
    fn unicode_inline_text_wraps_atomically_and_materializes() {
        let line = LineIndex::from_text("ab");
        let layout = IndexedLineLayout::with_inline_text(
            line,
            2,
            4,
            Arc::from([(1, Arc::<str>::from("界"))]),
        );
        let rows = layout.row_fragments(0..layout.row_count());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].fragments[0].text, "界");
        assert!(matches!(
            rows[1].fragments[0].kind,
            LayoutFragmentKind::InlineDecoration {
                index: 0,
                cell_offset: 0
            }
        ));
    }
}
