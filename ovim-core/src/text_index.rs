//! Shared, rope-backed coordinates and bounded line reads.
//!
//! Ordinary ASCII runs need no per-character index. Unicode lines keep sparse
//! grapheme checkpoints, and display metadata records only tabs and clusters
//! whose cell width differs from their scalar count. Edits to cached ASCII
//! lines update that metadata without walking unchanged text. Unicode edits
//! retain the unaffected checkpoint prefix and rebuild the changed suffix;
//! a pathological edit near the beginning can therefore still scan a line.

use crate::display::{grapheme_display_width, trim_line_terminator};
use crate::unicode::{CharCol, GraphemeCol};
use ropey::{Rope, RopeSlice};
use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::ops::Range;
use std::sync::{Arc, Mutex};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

const CHECKPOINT_STRIDE: usize = 256;
const CHANGE_HISTORY: usize = 256;

#[derive(Clone, Copy, Debug, Default)]
struct Checkpoint {
    byte: usize,
    char_col: usize,
    grapheme: usize,
}

#[derive(Clone, Copy, Debug)]
struct DisplaySpecial {
    start: usize,
    end: usize,
    /// None denotes a tab, whose width depends on the incoming content column.
    width: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
struct DisplayCheckpoint {
    start: usize,
    end: usize,
    display_start: usize,
    display_end: usize,
}

/// A stable index of one visible logical line, excluding its terminator.
/// The rope snapshot shares storage with the buffer; it never copies a long
/// line into a String. Held snapshots remain valid after subsequent edits.
#[derive(Debug)]
pub struct LineIndex {
    text: Rope,
    ascii: bool,
    checkpoints: Vec<Checkpoint>,
    specials: Vec<DisplaySpecial>,
    graphemes: usize,
    displays: Mutex<BTreeMap<usize, Arc<Vec<DisplayCheckpoint>>>>,
}

/// A viewport-sized string and its origins in the original logical line.
/// The first cluster may begin before the requested display column (a tab or
/// wide glyph straddling the left edge); callers clip that leading cell area.
#[derive(Debug, Clone)]
pub struct LineSlice {
    pub text: String,
    pub byte_start: usize,
    pub char_start: usize,
    pub grapheme_start: GraphemeCol,
    pub display_start: usize,
}

#[derive(Debug)]
pub struct IndexedGrapheme<'a> {
    pub text: Cow<'a, str>,
    pub byte_start: usize,
    pub char_start: usize,
    pub grapheme_col: GraphemeCol,
}

/// Grapheme streaming across rope chunks. GraphemeCursor can ask for arbitrary
/// preceding context (RI runs, ZWJ sequences, Indic conjuncts); it is always
/// supplied from the full line, never from independently segmented chunks.
pub struct IndexedGraphemes<'a> {
    text: RopeSlice<'a>,
    cursor: GraphemeCursor,
    chunk: &'a str,
    chunk_start: usize,
    position: Checkpoint,
    ascii: bool,
}

impl<'a> IndexedGraphemes<'a> {
    fn new(text: RopeSlice<'a>, position: Checkpoint, ascii: bool) -> Self {
        let (chunk, chunk_start, _, _) = text.chunk_at_byte(position.byte);
        Self {
            text,
            cursor: GraphemeCursor::new(position.byte, text.len_bytes(), true),
            chunk,
            chunk_start,
            position,
            ascii,
        }
    }

    fn next_boundary(&mut self) -> Option<usize> {
        loop {
            match self.cursor.next_boundary(self.chunk, self.chunk_start) {
                Ok(boundary) => return boundary,
                Err(GraphemeIncomplete::NextChunk) => {
                    let (chunk, start, _, _) = self.text.chunk_at_byte(self.cursor.cur_cursor());
                    self.chunk = chunk;
                    self.chunk_start = start;
                }
                Err(GraphemeIncomplete::PreContext(end)) => {
                    let (chunk, start, _, _) = self.text.chunk_at_byte(end - 1);
                    self.cursor.provide_context(&chunk[..end - start], start);
                }
                Err(error) => panic!("invalid forward rope grapheme cursor: {error:?}"),
            }
        }
    }
}

impl<'a> Iterator for IndexedGraphemes<'a> {
    type Item = IndexedGrapheme<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.position;
        if start.byte >= self.text.len_bytes() {
            return None;
        }
        if self.ascii && start.byte >= self.chunk_start + self.chunk.len() {
            let (chunk, chunk_start, _, _) = self.text.chunk_at_byte(start.byte);
            self.chunk = chunk;
            self.chunk_start = chunk_start;
        }
        let source_chunk = self.chunk;
        let source_chunk_start = self.chunk_start;
        let end = if self.ascii {
            start.byte + 1
        } else {
            self.next_boundary()?
        };
        // The cursor already holds the current leaf. Walking the rope from
        // its root for every tiny grapheme turns a streaming pass into
        // O(graphemes * tree height), especially visible in debug builds.
        let text: Cow<'a, str> = if start.byte >= source_chunk_start
            && end <= source_chunk_start + source_chunk.len()
        {
            Cow::Borrowed(&source_chunk[start.byte - source_chunk_start..end - source_chunk_start])
        } else if start.byte >= self.chunk_start && end <= self.chunk_start + self.chunk.len() {
            Cow::Borrowed(&self.chunk[start.byte - self.chunk_start..end - self.chunk_start])
        } else {
            self.text.byte_slice(start.byte..end).into()
        };
        self.position.byte = end;
        self.position.char_col += if self.ascii { 1 } else { text.chars().count() };
        self.position.grapheme += 1;
        Some(IndexedGrapheme {
            text,
            byte_start: start.byte,
            char_start: start.char_col,
            grapheme_col: GraphemeCol(start.grapheme),
        })
    }
}

/// Grapheme streaming backward across rope chunks: the reverse of
/// [`IndexedGraphemes`], yielding each grapheme's column and leading scalar.
/// One backward pass costs O(graphemes read), unlike a random-access lookup per
/// column, which re-walks up to a checkpoint stride each time.
pub struct ReverseGraphemes<'a> {
    text: RopeSlice<'a>,
    cursor: GraphemeCursor,
    chunk: &'a str,
    chunk_start: usize,
    /// Byte offset the next grapheme read ends at.
    byte: usize,
    /// Number of graphemes before `byte`.
    grapheme: usize,
    ascii: bool,
}

impl<'a> ReverseGraphemes<'a> {
    fn new(text: RopeSlice<'a>, position: Checkpoint, ascii: bool) -> Self {
        let (chunk, chunk_start, _, _) = text.chunk_at_byte(position.byte);
        Self {
            text,
            cursor: GraphemeCursor::new(position.byte, text.len_bytes(), true),
            chunk,
            chunk_start,
            byte: position.byte,
            grapheme: position.grapheme,
            ascii,
        }
    }

    fn prev_boundary(&mut self) -> Option<usize> {
        loop {
            match self.cursor.prev_boundary(self.chunk, self.chunk_start) {
                Ok(boundary) => return boundary,
                Err(GraphemeIncomplete::PrevChunk) => {
                    let (chunk, start, _, _) = self.text.chunk_at_byte(self.chunk_start - 1);
                    self.chunk = chunk;
                    self.chunk_start = start;
                }
                Err(GraphemeIncomplete::PreContext(end)) => {
                    let (chunk, start, _, _) = self.text.chunk_at_byte(end - 1);
                    self.cursor.provide_context(&chunk[..end - start], start);
                }
                Err(error) => panic!("invalid backward rope grapheme cursor: {error:?}"),
            }
        }
    }
}

impl Iterator for ReverseGraphemes<'_> {
    type Item = (GraphemeCol, char);

    fn next(&mut self) -> Option<Self::Item> {
        if self.byte == 0 {
            return None;
        }
        let start = if self.ascii {
            self.byte - 1
        } else {
            self.prev_boundary()?
        };
        if start < self.chunk_start || start >= self.chunk_start + self.chunk.len() {
            let (chunk, chunk_start, _, _) = self.text.chunk_at_byte(start);
            self.chunk = chunk;
            self.chunk_start = chunk_start;
        }
        let first_char = self.chunk[start - self.chunk_start..].chars().next()?;
        self.byte = start;
        self.grapheme -= 1;
        Some((GraphemeCol(self.grapheme), first_char))
    }
}

impl LineIndex {
    pub fn from_text(text: impl AsRef<str>) -> Arc<Self> {
        Arc::new(Self::new(RopeSlice::from(text.as_ref())))
    }

    pub fn new(text: RopeSlice<'_>) -> Self {
        let text = Rope::from(trim_line_terminator(text));
        let ascii = text.len_bytes() == text.len_chars();
        let mut index = Self {
            text,
            ascii,
            checkpoints: vec![Checkpoint::default()],
            specials: Vec::new(),
            graphemes: 0,
            displays: Mutex::new(BTreeMap::new()),
        };
        if ascii {
            index.graphemes = index.text.len_chars();
            for (col, ch) in index.text.chars().enumerate() {
                if ch.is_ascii_control() || crate::display::char_display_width(ch) != 1 {
                    index.specials.push(DisplaySpecial {
                        start: col,
                        end: col + 1,
                        width: (ch != '\t').then(|| crate::display::char_display_width(ch)),
                    });
                }
            }
        } else {
            index.build_suffix(Checkpoint::default());
        }
        index
    }

    fn build_suffix(&mut self, start: Checkpoint) {
        let mut iter = IndexedGraphemes::new(self.text.slice(..), start, self.ascii);
        while let Some(grapheme) = iter.next() {
            let end = iter.position;
            let chars = end.char_col - grapheme.char_start;
            let width = grapheme_display_width(&grapheme.text);
            if grapheme.text == "\t" || width != chars {
                self.specials.push(DisplaySpecial {
                    start: grapheme.char_start,
                    end: end.char_col,
                    width: (grapheme.text != "\t").then_some(width),
                });
            }
            if end.grapheme.is_multiple_of(CHECKPOINT_STRIDE) {
                self.checkpoints.push(end);
            }
        }
        self.graphemes = iter.position.grapheme;
    }

    fn edited(&self, text: RopeSlice<'_>, range: Range<usize>, inserted: &str) -> Self {
        let text = Rope::from(trim_line_terminator(text));
        let ascii = text.len_bytes() == text.len_chars();
        let mut result = Self {
            text,
            ascii,
            checkpoints: vec![Checkpoint::default()],
            specials: Vec::new(),
            graphemes: 0,
            displays: Mutex::new(BTreeMap::new()),
        };
        if self.ascii && ascii {
            // Printable ASCII occupies one char, byte, grapheme and cell. Only
            // sparse tabs/control characters need shifting or inspection.
            let inserted_len = inserted.len();
            let removed = range.end - range.start;
            result.specials.extend(
                self.specials
                    .iter()
                    .copied()
                    .filter(|s| s.end <= range.start),
            );
            for (offset, ch) in inserted.chars().enumerate() {
                if ch.is_ascii_control() || crate::display::char_display_width(ch) != 1 {
                    result.specials.push(DisplaySpecial {
                        start: range.start + offset,
                        end: range.start + offset + 1,
                        width: (ch != '\t').then(|| crate::display::char_display_width(ch)),
                    });
                }
            }
            result
                .specials
                .extend(
                    self.specials
                        .iter()
                        .filter(|s| s.start >= range.end)
                        .map(|s| DisplaySpecial {
                            start: s.start - removed + inserted_len,
                            end: s.end - removed + inserted_len,
                            width: s.width,
                        }),
                );
            result.graphemes = result.text.len_chars();
        } else {
            // Restart before the changed cluster: inserting a combining mark
            // can merge with its predecessor. Supply full rope pre-context at
            // chunk seams, including arbitrarily long RI/ZWJ context.
            let start = if self.ascii {
                let col = range.start.saturating_sub(1);
                Checkpoint {
                    byte: col,
                    char_col: col,
                    grapheme: col,
                }
            } else {
                let pos = self
                    .checkpoints
                    .partition_point(|p| p.char_col < range.start);
                self.checkpoints[pos.saturating_sub(1)]
            };
            result.checkpoints = if self.ascii {
                // The formerly implicit ASCII prefix needs explicit sparse
                // checkpoints once the line contains Unicode. Its coordinates
                // are arithmetic; constructing them never rereads the prefix.
                (0..start.char_col)
                    .step_by(CHECKPOINT_STRIDE)
                    .map(|col| Checkpoint {
                        byte: col,
                        char_col: col,
                        grapheme: col,
                    })
                    .collect()
            } else {
                self.checkpoints
                    .iter()
                    .copied()
                    .take_while(|p| p.char_col < start.char_col)
                    .collect()
            };
            result.checkpoints.push(start);
            result.specials = self
                .specials
                .iter()
                .copied()
                .take_while(|s| s.end <= start.char_col)
                .collect();
            result.build_suffix(start);
        }
        result
    }

    pub fn len_chars(&self) -> usize {
        self.text.len_chars()
    }
    pub fn len_bytes(&self) -> usize {
        self.text.len_bytes()
    }
    pub fn grapheme_count(&self) -> usize {
        self.graphemes
    }
    pub fn is_empty(&self) -> bool {
        self.text.len_chars() == 0
    }
    pub fn is_plain_ascii(&self) -> bool {
        self.ascii && self.specials.is_empty()
    }
    pub fn simple_display_width(&self) -> Option<usize> {
        self.is_plain_ascii().then_some(self.len_chars())
    }
    pub fn char_to_byte(&self, col: usize) -> usize {
        self.text.char_to_byte(col.min(self.len_chars()))
    }
    pub fn byte_to_char(&self, byte: usize) -> usize {
        self.text.byte_to_char(byte.min(self.len_bytes()))
    }

    pub fn utf16_to_char(&self, column: usize) -> usize {
        self.text
            .utf16_cu_to_char(column.min(self.text.len_utf16_cu()))
    }

    fn position_for_grapheme(&self, col: GraphemeCol) -> Checkpoint {
        let col = col.0.min(self.graphemes);
        if self.ascii {
            return Checkpoint {
                byte: col,
                char_col: col,
                grapheme: col,
            };
        }
        let cp = self.checkpoints[self
            .checkpoints
            .partition_point(|p| p.grapheme <= col)
            .saturating_sub(1)];
        let mut iter = IndexedGraphemes::new(self.text.slice(..), cp, false);
        while iter.position.grapheme < col {
            if iter.next().is_none() {
                break;
            }
        }
        iter.position
    }

    pub fn grapheme_to_char(&self, col: GraphemeCol) -> CharCol {
        CharCol(self.position_for_grapheme(col).char_col)
    }

    pub fn char_to_grapheme(&self, col: CharCol) -> GraphemeCol {
        let col = col.0.min(self.len_chars());
        if self.ascii {
            return GraphemeCol(col);
        }
        let cp = self.checkpoints[self
            .checkpoints
            .partition_point(|p| p.char_col <= col)
            .saturating_sub(1)];
        let mut iter = IndexedGraphemes::new(self.text.slice(..), cp, false);
        while let Some(g) = iter.next() {
            if col < iter.position.char_col {
                return g.grapheme_col;
            }
        }
        GraphemeCol(self.graphemes)
    }

    pub fn graphemes_from(&self, col: GraphemeCol) -> IndexedGraphemes<'_> {
        IndexedGraphemes::new(
            self.text.slice(..),
            self.position_for_grapheme(col),
            self.ascii,
        )
    }

    /// The graphemes before `col`, nearest first (`col - 1`, `col - 2`, ...).
    pub fn graphemes_before(&self, col: GraphemeCol) -> ReverseGraphemes<'_> {
        ReverseGraphemes::new(
            self.text.slice(..),
            self.position_for_grapheme(col),
            self.ascii,
        )
    }

    pub fn grapheme_at(&self, col: GraphemeCol) -> Option<String> {
        self.graphemes_from(col).next().map(|g| g.text.into_owned())
    }

    /// Returns the leading scalar of a grapheme without materializing it.
    ///
    /// Motion classification only needs this scalar, so keeping this path
    /// allocation-free matters for long ASCII words.
    pub fn grapheme_first_char(&self, col: GraphemeCol) -> Option<char> {
        let char_col = self.grapheme_to_char(col).0;
        (char_col < self.len_chars()).then(|| self.text.char(char_col))
    }

    pub fn slice_chars(&self, range: Range<usize>) -> String {
        let start = range.start.min(self.len_chars());
        self.text
            .slice(start..range.end.min(self.len_chars()).max(start))
            .to_string()
    }

    fn display_checkpoints(&self, tab_width: usize) -> Arc<Vec<DisplayCheckpoint>> {
        let tab_width = tab_width.max(1);
        let mut cache = self.displays.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(points) = cache.get(&tab_width) {
            return points.clone();
        }
        let mut points = Vec::with_capacity(self.specials.len());
        let mut chars = 0;
        let mut display = 0;
        for special in &self.specials {
            display += special.start - chars;
            let width = special
                .width
                .unwrap_or_else(|| tab_width - display % tab_width);
            points.push(DisplayCheckpoint {
                start: special.start,
                end: special.end,
                display_start: display,
                display_end: display + width,
            });
            display += width;
            chars = special.end;
        }
        let points = Arc::new(points);
        // Bound per-line settings history across tabstop changes.
        if cache.len() >= 4 {
            cache.clear();
        }
        cache.insert(tab_width, points.clone());
        points
    }

    /// Like display::char_col_to_display_col, an interior scalar position
    /// counts the containing grapheme's complete width.
    pub fn char_to_display(&self, col: usize, tab_width: usize) -> usize {
        let mut col = col.min(self.len_chars());
        if !self.ascii {
            let g = self.char_to_grapheme(CharCol(col));
            if self.grapheme_to_char(g).0 != col {
                col = self.grapheme_to_char(GraphemeCol(g.0 + 1)).0;
            }
        }
        let points = self.display_checkpoints(tab_width);
        let pos = points.partition_point(|p| p.end <= col);
        if pos == 0 {
            col
        } else {
            let p = points[pos - 1];
            p.display_end + col - p.end
        }
    }

    pub fn display_to_char(&self, col: usize, tab_width: usize) -> usize {
        let points = self.display_checkpoints(tab_width);
        // A zero-width cluster at exactly the requested column belongs to
        // that boundary (matching the existing string helper).
        let pos = points.partition_point(|p| {
            p.display_end < col || (p.display_end == col && p.display_start < col)
        });
        if let Some(p) = points.get(pos) {
            if col >= p.display_start {
                return p.start;
            }
        }
        let result = if pos == 0 {
            col
        } else {
            let p = points[pos - 1];
            p.end + col.saturating_sub(p.display_end)
        };
        let result = result.min(self.len_chars());
        // Multi-scalar clusters can have width equal to scalar count; they
        // need no display exception but must still remain atomic.
        self.grapheme_to_char(self.char_to_grapheme(CharCol(result)))
            .0
    }

    pub fn display_width(&self, tab_width: usize) -> usize {
        self.char_to_display(self.len_chars(), tab_width)
    }

    pub fn visible_slice(&self, start: usize, width: usize, tab_width: usize) -> LineSlice {
        let char_start = self.display_to_char(start, tab_width);
        let grapheme_start = self.char_to_grapheme(CharCol(char_start));
        let display_start = self.char_to_display(char_start, tab_width);
        let edge = start.saturating_add(width);
        let mut char_end = self.display_to_char(edge, tab_width);
        if width > 0
            && char_end < self.len_chars()
            && self.char_to_display(char_end, tab_width) < edge
        {
            let g = self.char_to_grapheme(CharCol(char_end));
            char_end = self.grapheme_to_char(GraphemeCol(g.0 + 1)).0;
        }
        if width == 0 {
            char_end = char_start;
        }
        LineSlice {
            text: self.slice_chars(char_start..char_end),
            byte_start: self.char_to_byte(char_start),
            char_start,
            grapheme_start,
            display_start,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use unicode_segmentation::UnicodeSegmentation;

    fn assert_index_matches(index: &LineIndex, expected: &str) {
        assert_eq!(index.len_bytes(), expected.len());
        assert_eq!(index.len_chars(), expected.chars().count());
        assert_eq!(index.grapheme_count(), expected.graphemes(true).count());
        assert_eq!(index.slice_chars(0..usize::MAX), expected);
        for g in 0..=index.grapheme_count() + 1 {
            assert_eq!(
                index.grapheme_to_char(GraphemeCol(g)),
                crate::unicode::grapheme_to_char_col(expected, GraphemeCol(g)),
                "grapheme {g}"
            );
        }
        for c in 0..=index.len_chars() + 1 {
            assert_eq!(
                index.char_to_grapheme(CharCol(c)),
                crate::unicode::char_to_grapheme_col(expected, CharCol(c)),
                "char {c}"
            );
            for tab in [1, 4, 8] {
                assert_eq!(
                    index.char_to_display(c, tab),
                    crate::display::char_col_to_display_col(expected, c, tab),
                    "display for char {c}, tab {tab}"
                );
            }
        }
        for tab in [1, 4, 8] {
            for d in 0..=crate::display::display_width(expected, tab) + 2 {
                assert_eq!(
                    index.display_to_char(d, tab),
                    crate::display::display_col_to_char_col(expected, d, tab),
                    "char for display {d}, tab {tab}"
                );
            }
        }
    }

    #[test]
    fn coordinates_match_existing_unicode_and_display_semantics() {
        for text in [
            "",
            "hello",
            "\ta\t\x01z",
            "é中e\u{301}x",
            "❤️👨‍👩‍👧‍👦🇺🇸🇳🇴z",
            "\u{200b}a\u{200b}",
            "क्‍षx",
        ] {
            assert_index_matches(&LineIndex::new(RopeSlice::from(text)), text);
        }
    }

    #[test]
    fn rope_chunk_seams_preserve_arbitrary_grapheme_context() {
        let text = format!(
            "{}{}{}{}",
            "x".repeat(997),
            "🇳".repeat(601),
            "a\u{301}".repeat(400),
            "👩‍👩‍👧‍👧".repeat(120)
        );
        let rope = Rope::from_str(&text);
        assert!(rope.chunks().count() > 4);
        let index = LineIndex::new(rope.slice(..));
        let actual: Vec<_> = index
            .graphemes_from(GraphemeCol::ZERO)
            .map(|g| g.text.into_owned())
            .collect();
        assert_eq!(actual, text.graphemes(true).collect::<Vec<_>>());
        for g in [0, 255, 256, 511, 1000, 1250, index.grapheme_count()] {
            assert_eq!(
                index.grapheme_to_char(GraphemeCol(g)),
                crate::unicode::grapheme_to_char_col(&text, GraphemeCol(g))
            );
        }
    }

    #[test]
    fn local_ascii_edits_preserve_snapshots_and_other_line_indexes() {
        let mut buffer = Buffer::new_from_str(&format!("{}\tend\nother\n", "a".repeat(100_000)));
        let old = buffer.line_index(0);
        let other = buffer.line_index(1);
        buffer.insert_text_at(0, CharCol(1), "X\t");
        let updated = buffer.line_index(0);
        assert_eq!(old.len_chars(), 100_004);
        assert_eq!(updated.len_chars(), 100_006);
        assert_eq!(updated.specials.len(), 2);
        assert_eq!(
            updated.checkpoints.len(),
            1,
            "ASCII edits require no grapheme checkpoint scan"
        );
        assert_eq!(updated.slice_chars(0..5), "aX\taa");
        assert!(Arc::ptr_eq(&other, &buffer.line_index(1)));
        buffer.delete_range(0, CharCol(1), 0, CharCol(3));
        assert_eq!(buffer.line_index(0).display_width(4), old.display_width(4));
        assert_eq!(
            buffer.line_changes_since(0).unwrap(),
            vec![
                LineChange {
                    version: 1,
                    start_line: 0,
                    old_line_count: 1,
                    new_line_count: 1
                },
                LineChange {
                    version: 2,
                    start_line: 0,
                    old_line_count: 1,
                    new_line_count: 1
                },
            ]
        );
    }

    #[test]
    fn unicode_suffix_edits_resegment_combining_and_regional_runs() {
        let mut buffer =
            Buffer::new_from_str(&format!("{}a{}x\n", "é".repeat(260), "🇳".repeat(600)));
        buffer.line_index(0);
        buffer.insert_text_at(0, CharCol(261), "\u{301}🇳");
        let text = buffer.line_text(0).unwrap().into_owned();
        let index = buffer.line_index(0);
        assert_eq!(index.grapheme_count(), text.graphemes(true).count());
        assert_eq!(
            index
                .graphemes_from(GraphemeCol::ZERO)
                .map(|g| g.text.into_owned())
                .collect::<Vec<_>>(),
            text.graphemes(true).collect::<Vec<_>>()
        );
        buffer.delete_range(0, CharCol(260), 0, CharCol(264));
        let text = buffer.line_text(0).unwrap().into_owned();
        let index = buffer.line_index(0);
        assert_eq!(index.grapheme_count(), text.graphemes(true).count());
        assert_eq!(
            index.grapheme_to_char(GraphemeCol(400)),
            crate::unicode::grapheme_to_char_col(&text, GraphemeCol(400))
        );
    }

    #[test]
    fn introducing_unicode_retains_bounded_checkpoints_in_the_ascii_prefix() {
        let mut buffer = Buffer::new_from_str(&"a".repeat(20_000));
        buffer.line_index(0);
        buffer.insert_text_at(0, CharCol(19_999), "\u{301}界");
        let index = buffer.line_index(0);
        assert!(index
            .checkpoints
            .windows(2)
            .all(|pair| pair[1].grapheme - pair[0].grapheme <= CHECKPOINT_STRIDE));
        assert_eq!(index.grapheme_to_char(GraphemeCol(10_000)), CharCol(10_000));
        assert_eq!(index.grapheme_count(), 20_001);
    }

    #[test]
    fn structural_splices_shift_cache_and_reset_history_safely() {
        let mut buffer = Buffer::new_from_str("abc\ndef\nxyz\n");
        let last = buffer.line_index(2);
        buffer.insert_text_at(0, CharCol(1), "q\nr\n");
        assert!(Arc::ptr_eq(&last, &buffer.line_index(4)));
        assert_eq!(
            buffer.line_changes_since(0).unwrap()[0],
            LineChange {
                version: 1,
                start_line: 0,
                old_line_count: 1,
                new_line_count: 3
            }
        );
        buffer.delete_range(0, CharCol(1), 2, CharCol(0));
        assert!(Arc::ptr_eq(&last, &buffer.line_index(2)));
        assert_eq!(buffer.line_index(0).slice_chars(0..usize::MAX), "abc");
        let version = buffer.version();
        buffer.replace_all("different\n");
        assert!(buffer.line_changes_since(version).is_none());
        let version = buffer.version();
        buffer.line_index(0);
        buffer.rope_mut().insert(0, "new ");
        assert!(buffer.line_changes_since(version).is_none());
        assert_eq!(
            buffer.line_index(0).slice_chars(0..usize::MAX),
            "new different"
        );
    }

    #[test]
    fn bounded_visible_slice_reports_source_origins_and_partial_cells() {
        let index = LineIndex::from_text("a\t中e\u{301}z");
        let slice = index.visible_slice(2, 3, 4);
        assert_eq!(slice.text, "\t中");
        assert_eq!(slice.char_start, 1);
        assert_eq!(slice.byte_start, 1);
        assert_eq!(slice.grapheme_start, GraphemeCol(1));
        assert_eq!(slice.display_start, 1);
        assert_eq!(index.visible_slice(100, 10, 4).text, "");
    }

    #[test]
    fn terminators_and_phantom_lines_are_not_indexed_as_content() {
        for text in ["abc\n", "abc\r\n", "abc\r"] {
            let buffer = Buffer::new_from_str(text);
            assert_eq!(buffer.line_index(0).slice_chars(0..usize::MAX), "abc");
            assert_eq!(
                buffer.line_index(buffer.raw_line_count() - 1).len_chars(),
                0
            );
        }
    }

    #[test]
    fn journal_eviction_never_returns_partial_history() {
        let mut buffer = Buffer::new();
        for _ in 0..CHANGE_HISTORY + 1 {
            buffer.insert_text_at(0, CharCol(0), "a");
        }
        assert!(buffer.line_changes_since(0).is_none());
        assert_eq!(buffer.line_changes_since(1).unwrap().len(), CHANGE_HISTORY);
    }
}

/// Replace `old_line_count` rows at `start_line` with `new_line_count` rows.
/// Coordinates describe the rope immediately before this edit. Counts include
/// the touched boundary line, even when no newline was inserted or removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineChange {
    pub version: usize,
    pub start_line: usize,
    pub old_line_count: usize,
    pub new_line_count: usize,
}

#[derive(Default)]
pub(crate) struct TextIndexCache {
    lines: BTreeMap<usize, Arc<LineIndex>>,
}

#[derive(Default)]
pub(crate) struct LineChangeLog {
    changes: VecDeque<LineChange>,
    base_version: usize,
}

impl LineChangeLog {
    pub(crate) fn since(&self, version: usize, current: usize) -> Option<Vec<LineChange>> {
        if version < self.base_version || version > current {
            return None;
        }
        Some(
            self.changes
                .iter()
                .copied()
                .filter(|change| change.version > version)
                .collect(),
        )
    }
    pub(crate) fn reset(&mut self, version: usize) {
        self.changes.clear();
        self.base_version = version;
    }
    pub(crate) fn push(&mut self, change: LineChange) {
        if self.changes.len() == CHANGE_HISTORY {
            self.base_version = self.changes.pop_front().unwrap().version;
        }
        self.changes.push_back(change);
    }
}

pub(crate) struct PendingTextEdit {
    start_line: usize,
    end_line: usize,
    start_col: usize,
    end_col: usize,
    old_lines: usize,
}

impl PendingTextEdit {
    pub(crate) fn new(rope: &Rope, start: usize, end: usize) -> Self {
        let start_line = rope.char_to_line(start);
        let end_line = rope.char_to_line(end);
        Self {
            start_line,
            end_line,
            start_col: start - rope.line_to_char(start_line),
            end_col: end - rope.line_to_char(end_line),
            old_lines: rope.len_lines(),
        }
    }
}

impl TextIndexCache {
    pub(crate) fn line(&mut self, rope: &Rope, line: usize) -> Arc<LineIndex> {
        self.lines
            .entry(line)
            .or_insert_with(|| {
                Arc::new(LineIndex::new(if line < rope.len_lines() {
                    rope.line(line)
                } else {
                    RopeSlice::from("")
                }))
            })
            .clone()
    }

    pub(crate) fn clear(&mut self) {
        self.lines.clear();
    }

    pub(crate) fn edited(
        &mut self,
        rope: &Rope,
        edit: PendingTextEdit,
        inserted: &str,
        version: usize,
    ) -> LineChange {
        let old_line_count = edit.end_line - edit.start_line + 1;
        // Equivalent to old_line_count + (new total - old total), using
        // subtraction only after the nonnegative row count is established.
        let new_line_count = if rope.len_lines() >= edit.old_lines {
            old_line_count + (rope.len_lines() - edit.old_lines)
        } else {
            old_line_count - (edit.old_lines - rope.len_lines())
        };
        let old = self.lines.remove(&edit.start_line);
        if old_line_count == 1 && new_line_count == 1 {
            if let Some(old) = old {
                if edit.end_col <= old.len_chars() && !inserted.contains(['\r', '\n']) {
                    let updated = old.edited(
                        rope.line(edit.start_line),
                        edit.start_col..edit.end_col,
                        inserted,
                    );
                    self.lines.insert(edit.start_line, Arc::new(updated));
                }
            }
        } else {
            // Structural edits may shift the cached row keys; ordinary local
            // typing never visits the rest of this map.
            let tail = self.lines.split_off(&edit.start_line);
            for (line, index) in tail {
                if line > edit.end_line {
                    self.lines
                        .insert(line - old_line_count + new_line_count, index);
                }
            }
        }
        LineChange {
            version,
            start_line: edit.start_line,
            old_line_count,
            new_line_count,
        }
    }
}
