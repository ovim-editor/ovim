//! Code folding: nested folds with Vim's `z` semantics.
//!
//! Folds come from two places: the user (`zf`, always closed when created) and
//! automatic sources (LSP `foldingRange`, indentation). Automatic folds are
//! replaced wholesale whenever the source recomputes them, preserving the
//! open/closed state of folds that keep their header line.
//!
//! A closed fold hides `start + 1 ..= end`; its header line stays visible.
//! Nested folds inside a closed fold are hidden with it, and the *outermost*
//! closed fold containing a line decides how that line is displayed.
//!
//! The renderer and the cursor rules ask about closed folds on every key and
//! frame, so those queries are answered from an index of the outermost closed
//! folds that is rebuilt once per [`FoldManager::generation`], never by
//! rescanning every fold.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Where a fold came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldOrigin {
    /// Created by the user (`zf`); survives automatic recomputation.
    Manual,
    /// Produced by LSP `foldingRange` or the indentation fallback.
    Auto,
}

/// One cell of the fold gutter (Vim's `foldcolumn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldGutterMark {
    /// No fold at this nesting level.
    Blank,
    /// The line heads an open fold (`-`).
    OpenStart,
    /// The line heads a closed fold (`+`).
    ClosedStart,
    /// The line is inside an open fold (`|`).
    Inside,
}

impl FoldGutterMark {
    /// The character Vim draws for the mark.
    pub fn glyph(self) -> char {
        match self {
            Self::Blank => ' ',
            Self::OpenStart => '-',
            Self::ClosedStart => '+',
            Self::Inside => '|',
        }
    }
}

/// Which automatic source produced the current `Auto` folds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldSource {
    /// Indentation heuristic (no server ranges, no syntax tree).
    Indent,
    /// The tree-sitter syntax tree (no server ranges).
    Syntax,
    /// The language server's `textDocument/foldingRange`.
    Lsp,
}

/// Represents a text fold (collapsed region)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fold {
    /// Starting line (inclusive) - the header that stays visible
    start_line: usize,
    /// Ending line (inclusive)
    end_line: usize,
    /// Whether this fold is currently open (false = folded/hidden)
    open: bool,
    origin: FoldOrigin,
}

impl Fold {
    /// Creates a new (closed) manual fold
    pub fn new(start_line: usize, end_line: usize) -> Self {
        Self {
            start_line,
            end_line,
            open: false,
            origin: FoldOrigin::Manual,
        }
    }

    fn auto(start_line: usize, end_line: usize, open: bool) -> Self {
        Self {
            start_line,
            end_line,
            open,
            origin: FoldOrigin::Auto,
        }
    }

    pub fn start_line(&self) -> usize {
        self.start_line
    }

    pub fn end_line(&self) -> usize {
        self.end_line
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn origin(&self) -> FoldOrigin {
        self.origin
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Whether this fold contains the given line
    pub fn contains_line(&self, line: usize) -> bool {
        line >= self.start_line && line <= self.end_line
    }

    /// Whether this fold overlaps with another fold
    pub fn overlaps(&self, other: &Fold) -> bool {
        !(self.end_line < other.start_line || self.start_line > other.end_line)
    }

    /// Number of lines a closed fold hides.
    pub fn hidden_line_count(&self) -> usize {
        self.end_line - self.start_line
    }

    fn encloses(&self, other: &Fold) -> bool {
        self.start_line <= other.start_line && other.end_line <= self.end_line
    }
}

/// Source of [`FoldManager::generation`] values. Global, so a freshly created
/// manager (a reloaded buffer) never repeats a generation a cache has seen.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// What the closed folds hide, derived from the folds once per generation.
#[derive(Debug, Clone, Default)]
struct ClosedIndex {
    /// Outermost closed folds as `(start, end)`, ascending and disjoint.
    outer: Vec<(usize, usize)>,
    /// Merged inclusive hidden line ranges (`start + 1 ..= end` of each outer
    /// fold; ranges that touch are joined), ascending.
    hidden: Vec<(usize, usize)>,
    /// Lines hidden by `hidden[..i]`, so lookups need no summing.
    hidden_before: Vec<usize>,
}

impl ClosedIndex {
    fn build(folds: &[Fold]) -> Self {
        // `folds` is sorted by start, so one sweep that tracks the end of the
        // current outermost closed fold finds every outer fold. A closed fold
        // that starts inside it is nested (or, for crossing folds, extends it).
        let mut outer: Vec<(usize, usize)> = Vec::new();
        for fold in folds.iter().filter(|fold| !fold.is_open()) {
            match outer.last_mut() {
                Some(last) if fold.start_line <= last.1 => last.1 = last.1.max(fold.end_line),
                _ => outer.push((fold.start_line, fold.end_line)),
            }
        }
        let mut hidden: Vec<(usize, usize)> = Vec::new();
        for &(start, end) in &outer {
            let (start, end) = (start + 1, end);
            match hidden.last_mut() {
                Some(last) if start <= last.1 + 1 => last.1 = last.1.max(end),
                _ => hidden.push((start, end)),
            }
        }
        let mut total = 0;
        let hidden_before = hidden
            .iter()
            .map(|&(start, end)| {
                let before = total;
                total += end - start + 1;
                before
            })
            .collect();
        Self {
            outer,
            hidden,
            hidden_before,
        }
    }
}

/// Manages folds for a buffer
#[derive(Debug, Clone)]
pub struct FoldManager {
    /// Sorted by start line ascending, then end line descending, so a fold
    /// always precedes the folds nested inside it.
    folds: Vec<Fold>,
    /// `foldenable`: when false nothing is hidden (`zn` / `zN` / `zi`).
    enabled: bool,
    /// Buffer line count the fold ranges were last aligned with.
    synced_line_count: usize,
    /// Buffer version the automatic folds were computed for.
    auto_version: Option<usize>,
    /// Set once the user has issued a fold command for this buffer; folds are
    /// only computed and maintained after that.
    active: bool,
    /// Where the automatic folds came from.
    source: FoldSource,
    /// Vim's `foldlevel`: folds nested deeper than this are closed, the others
    /// open. `None` is "everything open" (a fresh buffer): it counts as the
    /// deepest nesting, so the first `zm` is useful (Vim under
    /// `foldlevelstart=99` needs ~99 `zm` before anything happens).
    foldlevel: Option<usize>,
    /// Changes whenever the folds, their open/closed state or `foldenable`
    /// change; see [`Self::generation`].
    generation: u64,
    /// Closed-fold lookup tables for the current generation.
    closed: OnceLock<ClosedIndex>,
}

impl FoldManager {
    /// Creates a new empty fold manager
    pub fn new() -> Self {
        Self {
            folds: Vec::new(),
            enabled: true,
            synced_line_count: 0,
            auto_version: None,
            active: false,
            source: FoldSource::Indent,
            foldlevel: None,
            generation: next_generation(),
            closed: OnceLock::new(),
        }
    }

    /// An opaque value that differs whenever what the folds show may have
    /// changed (a fold added, moved, opened or closed; `foldenable` toggled)
    /// and is the same otherwise. Unique across managers, so a cache keyed on
    /// it also notices the manager being replaced.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Records that the folds changed: bumps the generation and drops the
    /// lookup tables derived from them.
    fn touch(&mut self) {
        self.generation = next_generation();
        self.closed = OnceLock::new();
    }

    fn closed_index(&self) -> &ClosedIndex {
        self.closed.get_or_init(|| ClosedIndex::build(&self.folds))
    }

    fn sort(&mut self) {
        self.folds.sort_by(|a, b| {
            a.start_line
                .cmp(&b.start_line)
                .then(b.end_line.cmp(&a.end_line))
        });
    }

    fn insert(&mut self, fold: Fold) {
        self.folds.push(fold);
        self.sort();
    }

    /// Indices of the folds containing `line`, outermost first.
    fn chain(&self, line: usize) -> Vec<usize> {
        self.folds
            .iter()
            .enumerate()
            .filter(|(_, fold)| fold.contains_line(line))
            .map(|(index, _)| index)
            .collect()
    }

    /// Creates a (closed) manual fold. Folds may nest but not partially overlap.
    pub fn create_fold(&mut self, start_line: usize, end_line: usize) {
        if start_line >= end_line {
            return;
        }
        let fold = Fold::new(start_line, end_line);
        // An existing fold covering the same lines is replaced.
        self.folds
            .retain(|f| !(f.start_line == start_line && f.end_line == end_line));
        // Partial overlaps cannot be represented as a tree; drop them.
        self.folds
            .retain(|f| !f.overlaps(&fold) || f.encloses(&fold) || fold.encloses(f));
        self.insert(fold);
        self.touch();
    }

    // ----- state queries ---------------------------------------------------

    /// Whether any fold exists at all.
    pub fn is_empty(&self) -> bool {
        self.folds.is_empty()
    }

    pub fn folds(&self) -> &[Fold] {
        &self.folds
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.touch();
        }
    }

    /// True once the user has used a fold command on this buffer.
    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn activate(&mut self) {
        self.active = true;
    }

    pub fn auto_version(&self) -> Option<usize> {
        self.auto_version
    }

    pub fn is_lsp_backed(&self) -> bool {
        self.source == FoldSource::Lsp
    }

    pub fn source(&self) -> FoldSource {
        self.source
    }

    /// The outermost closed fold containing `line`, as `(start, end)`.
    pub fn closed_fold_at(&self, line: usize) -> Option<(usize, usize)> {
        if !self.enabled {
            return None;
        }
        let outer = &self.closed_index().outer;
        let after = outer.partition_point(|&(start, _)| start <= line);
        outer[..after]
            .last()
            .copied()
            .filter(|&(_, end)| line <= end)
    }

    /// Every outermost closed fold as `(header line, hidden line count)`,
    /// ascending: the lines that carry a `⋯ N lines` marker.
    pub fn closed_fold_headers(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let outer: &[(usize, usize)] = if self.enabled {
            &self.closed_index().outer
        } else {
            &[]
        };
        outer.iter().map(|&(start, end)| (start, end - start))
    }

    /// Checks if a line is hidden by a closed fold (the header is not hidden).
    pub fn is_line_hidden(&self, line: usize) -> bool {
        self.closed_fold_at(line)
            .is_some_and(|(start, _)| line > start)
    }

    /// Whether there's a closed fold whose header is `line`.
    pub fn has_closed_fold_at(&self, line: usize) -> bool {
        self.enabled
            && self
                .folds
                .iter()
                .any(|f| f.start_line() == line && !f.is_open())
    }

    /// Gets the (outermost) fold that starts at the given line
    pub fn fold_at(&self, line: usize) -> Option<&Fold> {
        self.folds.iter().find(|f| f.start_line() == line)
    }

    /// Hidden line count of the closed fold whose header is `line`.
    pub fn folded_line_count_at(&self, line: usize) -> Option<usize> {
        match self.closed_fold_at(line) {
            Some((start, end)) if start == line => Some(end - start),
            _ => None,
        }
    }

    /// Marks for the fold gutter of `line`, outermost fold first: one entry
    /// per fold that contains the line (the chain stops at a closed fold,
    /// whose body is hidden anyway).
    pub fn gutter_chain(&self, line: usize) -> Vec<FoldGutterMark> {
        if !self.enabled {
            return Vec::new();
        }
        let mut marks = Vec::new();
        for index in self.chain(line) {
            let fold = &self.folds[index];
            if !fold.is_open() {
                marks.push(FoldGutterMark::ClosedStart);
                break;
            }
            marks.push(if fold.start_line == line {
                FoldGutterMark::OpenStart
            } else {
                FoldGutterMark::Inside
            });
        }
        marks
    }

    /// Merged inclusive ranges of hidden lines, ascending.
    pub fn hidden_ranges(&self) -> &[(usize, usize)] {
        if !self.enabled {
            return &[];
        }
        &self.closed_index().hidden
    }

    /// Number of hidden lines strictly before `line`.
    pub fn hidden_count_before(&self, line: usize) -> usize {
        if !self.enabled {
            return 0;
        }
        let index = self.closed_index();
        // Ranges starting before `line` are hidden in full except the last,
        // which `line` may cut short.
        let before = index.hidden.partition_point(|&(start, _)| start < line);
        match before.checked_sub(1) {
            Some(last) => {
                let (start, end) = index.hidden[last];
                index.hidden_before[last] + end.min(line - 1) - start + 1
            }
            None => 0,
        }
    }

    /// Index of `line` among the visible lines (hidden lines removed).
    pub fn visible_index(&self, line: usize) -> usize {
        line - self.hidden_count_before(line).min(line)
    }

    /// The logical line that is the `index`-th visible line.
    pub fn line_at_visible_index(&self, index: usize) -> usize {
        if !self.enabled {
            return index;
        }
        let ClosedIndex {
            hidden,
            hidden_before,
            ..
        } = self.closed_index();
        // Range `i` begins hiding at visible index `start - hidden_before[i]`
        // (strictly increasing): every range that begins at or before `index`
        // shifts the result by its length.
        let (mut low, mut high) = (0, hidden.len());
        while low < high {
            let mid = (low + high) / 2;
            if hidden[mid].0 - hidden_before[mid] <= index {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        match low.checked_sub(1) {
            Some(last) => {
                let (start, end) = hidden[last];
                index + hidden_before[last] + (end - start + 1)
            }
            None => index,
        }
    }

    /// First visible line at or after `line`, if any before `line_count`.
    pub fn next_visible_line(&self, line: usize, line_count: usize) -> Option<usize> {
        let mut candidate = line;
        while candidate < line_count {
            match self.closed_fold_at(candidate) {
                Some((start, end)) if candidate > start => candidate = end + 1,
                _ => return Some(candidate),
            }
        }
        None
    }

    /// `j` over closed folds: `count` visible lines down from `line`.
    pub fn step_down(&self, line: usize, count: usize, max_line: usize) -> usize {
        let mut current = line;
        for _ in 0..count {
            let after = match self.closed_fold_at(current) {
                Some((_, end)) => end + 1,
                None => current + 1,
            };
            if after > max_line {
                break;
            }
            current = after;
        }
        current
    }

    /// `k` over closed folds: `count` visible lines up from `line`.
    pub fn step_up(&self, line: usize, count: usize) -> usize {
        let mut current = line;
        for _ in 0..count {
            if current == 0 {
                break;
            }
            let before = current - 1;
            current = match self.closed_fold_at(before) {
                Some((start, _)) => start,
                None => before,
            };
        }
        current
    }

    // ----- opening and closing ----------------------------------------------

    /// `zo`: opens the outermost closed fold at `line`.
    pub fn open_one(&mut self, line: usize) -> bool {
        for index in self.chain(line) {
            if !self.folds[index].is_open() {
                self.folds[index].open();
                self.touch();
                return true;
            }
        }
        false
    }

    /// `zc`: closes the deepest open fold above the first closed one.
    pub fn close_one(&mut self, line: usize) -> bool {
        let mut target = None;
        for index in self.chain(line) {
            if !self.folds[index].is_open() {
                break;
            }
            target = Some(index);
        }
        match target {
            Some(index) => {
                self.folds[index].close();
                self.touch();
                true
            }
            None => false,
        }
    }

    /// `za`: open the fold when the line is inside a closed one, else close.
    pub fn toggle_one(&mut self, line: usize) -> bool {
        if self.closed_fold_at(line).is_some() {
            self.open_one(line)
        } else {
            self.close_one(line)
        }
    }

    /// `zO`: opens every fold containing `line`.
    pub fn open_recursive(&mut self, line: usize) -> bool {
        let mut changed = false;
        for index in self.chain(line) {
            if !self.folds[index].is_open() {
                self.folds[index].open();
                changed = true;
            }
        }
        if changed {
            self.touch();
        }
        changed
    }

    /// `zC`: closes every fold containing `line`.
    pub fn close_recursive(&mut self, line: usize) -> bool {
        let mut changed = false;
        for index in self.chain(line) {
            if self.folds[index].is_open() {
                self.folds[index].close();
                changed = true;
            }
        }
        if changed {
            self.touch();
        }
        changed
    }

    /// `zA`: recursive toggle.
    pub fn toggle_recursive(&mut self, line: usize) -> bool {
        if self.closed_fold_at(line).is_some() {
            self.open_recursive(line)
        } else {
            self.close_recursive(line)
        }
    }

    /// `zv`: opens just enough folds for `line` to be visible.
    pub fn reveal(&mut self, line: usize) -> bool {
        let mut changed = false;
        while self.is_line_hidden(line) || self.closed_fold_at(line).is_some() {
            if !self.open_one(line) {
                break;
            }
            changed = true;
        }
        changed
    }

    /// Opens a fold at the given line (fold header)
    pub fn open_fold_at(&mut self, line: usize) {
        self.open_one(line);
    }

    /// Closes a fold at the given line
    pub fn close_fold_at(&mut self, line: usize) {
        self.close_one(line);
    }

    /// Toggles a fold at the given line
    pub fn toggle_fold_at(&mut self, line: usize) {
        self.toggle_one(line);
    }

    /// Nesting level (1 = outermost) of every fold, in `folds` order.
    fn nesting_levels(&self) -> Vec<usize> {
        let mut ends: Vec<usize> = Vec::new();
        self.folds
            .iter()
            .map(|fold| {
                while ends.last().is_some_and(|&end| end < fold.start_line) {
                    ends.pop();
                }
                ends.push(fold.end_line);
                ends.len()
            })
            .collect()
    }

    /// Depth of the deepest fold nesting (`getDeepestNesting`).
    pub fn deepest_nesting(&self) -> usize {
        self.nesting_levels().into_iter().max().unwrap_or(0)
    }

    /// Vim's `foldlevel`.
    pub fn foldlevel(&self) -> usize {
        self.foldlevel.unwrap_or_else(|| self.deepest_nesting())
    }

    /// Forgets every manual open/close: folds deeper than `foldlevel` are
    /// closed, the rest open (what changing 'foldlevel' does in Vim).
    fn apply_foldlevel(&mut self) {
        let level = self.foldlevel();
        let levels = self.nesting_levels();
        let mut changed = false;
        for (fold, depth) in self.folds.iter_mut().zip(levels) {
            changed |= fold.open != (depth <= level);
            fold.open = depth <= level;
        }
        if changed {
            self.touch();
        }
    }

    /// `zR`: opens everything and sets `foldlevel` to the deepest nesting.
    pub fn open_all(&mut self) {
        self.foldlevel = Some(self.deepest_nesting());
        self.apply_foldlevel();
    }

    /// `zM`: closes everything (`foldlevel` 0).
    pub fn close_all(&mut self) {
        self.foldlevel = Some(0);
        self.apply_foldlevel();
    }

    /// `zr`: reduce folding, `foldlevel` += `count` (at most the deepest
    /// nesting).
    pub fn reduce_folding(&mut self, count: usize) {
        let deepest = self.deepest_nesting();
        self.foldlevel = Some((self.foldlevel() + count).min(deepest));
        self.apply_foldlevel();
    }

    /// `zm`: fold more, `foldlevel` -= `count`. Like Vim, nothing happens
    /// when `foldlevel` is already 0.
    pub fn fold_more(&mut self, count: usize) {
        let level = self.foldlevel();
        if level > 0 {
            self.foldlevel = Some(level.saturating_sub(count));
        }
        self.apply_foldlevel();
    }

    /// `zX` (and the first half of `zx`): re-apply `foldlevel`, undoing
    /// every manual `zo`/`zc`.
    pub fn reapply_foldlevel(&mut self) {
        self.apply_foldlevel();
    }

    // ----- deleting ----------------------------------------------------------

    /// `zd`: deletes the innermost fold containing `line`.
    pub fn delete_fold_at(&mut self, line: usize) {
        if let Some(&index) = self.chain(line).last() {
            self.folds.remove(index);
            self.touch();
        }
    }

    /// `zD`: deletes the outermost fold containing `line` with everything
    /// nested inside it.
    pub fn delete_recursive(&mut self, line: usize) {
        if let Some(&outer) = self.chain(line).first() {
            let outer = self.folds[outer].clone();
            self.folds.retain(|f| !outer.encloses(f));
            self.touch();
        }
    }

    /// `zE`
    pub fn delete_all(&mut self) {
        self.folds.clear();
        self.auto_version = None;
        self.touch();
    }

    // ----- motions -------------------------------------------------------------

    /// `zj`: start of the next fold below `line`.
    pub fn next_fold_start(&self, line: usize) -> Option<usize> {
        self.folds
            .iter()
            .map(|f| f.start_line)
            .filter(|&start| start > line)
            .min()
    }

    /// `zk`: end of the previous fold above `line`.
    pub fn previous_fold_end(&self, line: usize) -> Option<usize> {
        self.folds
            .iter()
            .map(|f| f.end_line)
            .filter(|&end| end < line)
            .max()
    }

    /// `[z` / `]z`: line the motion lands on. Only *open* folds count (the
    /// chain of open folds around `line`, outermost first, ends at the first
    /// closed one). `[z` goes to the start of the innermost fold that starts
    /// above `line`, `]z` to the end of the innermost fold that ends below it;
    /// `None` when there is none (the motion fails).
    pub fn fold_edge_target(&self, line: usize, to_end: bool) -> Option<usize> {
        let mut open_chain: Vec<&Fold> = Vec::new();
        for index in self.chain(line) {
            let fold = &self.folds[index];
            if !fold.is_open() {
                break;
            }
            open_chain.push(fold);
        }
        open_chain.iter().rev().find_map(|fold| {
            if to_end {
                (fold.end_line > line).then_some(fold.end_line)
            } else {
                (fold.start_line < line).then_some(fold.start_line)
            }
        })
    }

    // ----- automatic folds -----------------------------------------------------

    /// Replaces the automatic folds with `ranges` (inclusive `(start, end)`
    /// line pairs). Folds that keep their header line keep their open/closed
    /// state; new ones start open. Partially overlapping ranges are dropped.
    pub fn set_auto_folds(
        &mut self,
        ranges: &[(usize, usize)],
        line_count: usize,
        buffer_version: usize,
        from_lsp: bool,
    ) {
        let source = if from_lsp {
            FoldSource::Lsp
        } else {
            FoldSource::Indent
        };
        self.set_auto_folds_from(ranges, line_count, buffer_version, source);
    }

    /// Like [`Self::set_auto_folds`], recording which source produced them.
    pub fn set_auto_folds_from(
        &mut self,
        ranges: &[(usize, usize)],
        line_count: usize,
        buffer_version: usize,
        source: FoldSource,
    ) {
        self.source = source;
        let before = self.folds.clone();
        let (manual, auto): (Vec<Fold>, Vec<Fold>) = std::mem::take(&mut self.folds)
            .into_iter()
            .partition(|f| f.origin == FoldOrigin::Manual);
        self.folds = manual;
        // State of the previous automatic folds, by exact range and (for a
        // fold whose end moved) by header line; `auto` is sorted, so the
        // first fold seen at a header line is its outermost.
        let mut open_by_range: HashMap<(usize, usize), bool> = HashMap::with_capacity(auto.len());
        let mut open_by_start: HashMap<usize, bool> = HashMap::with_capacity(auto.len());
        for fold in &auto {
            open_by_range.insert((fold.start_line, fold.end_line), fold.open);
            open_by_start.entry(fold.start_line).or_insert(fold.open);
        }

        let mut incoming: Vec<(usize, usize)> = ranges
            .iter()
            .copied()
            .filter(|&(start, end)| start < end && end < line_count)
            .collect();
        incoming.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        incoming.dedup();

        let mut accepted: Vec<Fold> = Vec::with_capacity(incoming.len());
        let mut fresh_starts: HashSet<usize> = HashSet::new();
        // Ends of the accepted folds that enclose the current candidate: a
        // nested chain, so the innermost (last) has the smallest end.
        let mut enclosing_ends: Vec<usize> = Vec::new();
        for (start, end) in incoming {
            while enclosing_ends.last().is_some_and(|&top| top < start) {
                enclosing_ends.pop();
            }
            // Candidates arrive by start, so an accepted fold that reaches the
            // candidate's start but not its end crosses it. Manual folds are
            // few and in no order relative to the candidates; check them all.
            let candidate = Fold::auto(start, end, true);
            let crosses_accepted = enclosing_ends.last().is_some_and(|&top| top < end);
            let crosses_manual = self.folds.iter().any(|f| {
                f.overlaps(&candidate) && !f.encloses(&candidate) && !candidate.encloses(f)
            });
            if crosses_accepted || crosses_manual {
                continue;
            }
            let open = open_by_range
                .get(&(start, end))
                .or_else(|| open_by_start.get(&start))
                .copied();
            accepted.push(Fold::auto(start, end, open.unwrap_or(true)));
            enclosing_ends.push(end);
            if open.is_none() {
                fresh_starts.insert(start);
            }
        }
        self.folds.extend(accepted);
        self.sort();
        // A fold nobody has opened or closed follows `foldlevel`.
        if self.foldlevel.is_some() {
            let level = self.foldlevel();
            let levels = self.nesting_levels();
            for (fold, depth) in self.folds.iter_mut().zip(levels) {
                if fold.origin == FoldOrigin::Auto && fresh_starts.contains(&fold.start_line) {
                    fold.open = depth <= level;
                }
            }
        }
        self.synced_line_count = line_count;
        self.auto_version = Some(buffer_version);
        if self.folds != before {
            self.touch();
        }
    }

    /// Keeps fold ranges aligned with the text after an edit that changed the
    /// line count. `pivot` is the line the edit happened at (the cursor line
    /// before the change). A heuristic bridge until the next recompute.
    pub fn adjust_for_line_count(&mut self, new_line_count: usize, pivot: usize) {
        let old = self.synced_line_count;
        self.synced_line_count = new_line_count;
        if old == 0 || old == new_line_count || self.folds.is_empty() {
            return;
        }
        let delta = new_line_count as isize - old as isize;
        let shift = |line: usize| -> usize { (line as isize + delta).max(0) as usize };
        for fold in &mut self.folds {
            if fold.start_line > pivot {
                fold.start_line = shift(fold.start_line);
                fold.end_line = shift(fold.end_line);
            } else if fold.end_line >= pivot {
                fold.end_line = shift(fold.end_line).max(fold.start_line);
            }
        }
        let max_line = new_line_count.saturating_sub(1);
        self.folds.retain_mut(|fold| {
            fold.end_line = fold.end_line.min(max_line);
            fold.start_line < fold.end_line
        });
        self.sort();
        self.touch();
    }

    /// Records the current line count without moving anything.
    pub fn note_line_count(&mut self, line_count: usize) {
        self.synced_line_count = line_count;
    }
}

fn next_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

impl Default for FoldManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Folds derived from indentation: a line whose following non-blank line is
/// indented deeper heads a fold that runs to the last line indented deeper.
/// Blank lines never end a fold.
pub fn indent_fold_ranges(lines: &[String], tab_width: usize) -> Vec<(usize, usize)> {
    let indent = |text: &str| -> Option<usize> {
        if text.trim().is_empty() {
            return None;
        }
        let mut width = 0;
        for c in text.chars() {
            match c {
                ' ' => width += 1,
                '\t' => width += tab_width - width % tab_width,
                _ => break,
            }
        }
        Some(width)
    };
    let indents: Vec<Option<usize>> = lines.iter().map(|l| indent(l)).collect();
    let mut ranges = Vec::new();
    for (start, header) in indents.iter().enumerate() {
        let Some(header) = *header else { continue };
        let mut end = None;
        for (line, indent) in indents.iter().enumerate().skip(start + 1) {
            match indent {
                None => continue,
                Some(width) if *width > header => end = Some(line),
                Some(_) => break,
            }
        }
        if let Some(end) = end {
            ranges.push((start, end));
        }
    }
    ranges
}

/// Node kinds worth folding: bodies, declarations, literals and comments
/// across the tree-sitter grammars ovim ships (`block`, `class_body`,
/// `function_definition`, `object`, `block_comment`, ...).
fn syntax_kind_is_foldable(kind: &str) -> bool {
    const PARTS: &[&str] = &[
        "block",
        "body",
        "declaration",
        "definition",
        "item",
        "literal",
        "list",
        "array",
        "object",
        "dictionary",
        "table",
        "comment",
        "arguments",
        "parameters",
        "statement",
        "expression",
        "clause",
        "class",
        "function",
        "method",
        "struct",
        "enum",
        "impl",
        "trait",
        "interface",
        "module",
        "namespace",
        "element",
        "section",
    ];
    // Single-token-ish or purely structural nodes that repeat their parent.
    const SKIP: &[&str] = &["program", "source_file", "compilation_unit", "document"];
    !SKIP.contains(&kind) && PARTS.iter().any(|part| kind.contains(part))
}

/// Folds derived from a tree-sitter syntax tree: every multi-line foldable
/// node heads a fold from its first line. A last line that only closes the
/// construct (`}`, `)`, `]`, `end`, `</tag>`) stays visible, as in every
/// editor; `line_text` supplies the buffer's lines.
pub fn syntax_fold_ranges(
    tree: &tree_sitter::Tree,
    line_text: &dyn Fn(usize) -> String,
) -> Vec<(usize, usize)> {
    let closes_only = |row: usize| -> bool {
        let text = line_text(row);
        let text = text.trim_start();
        text.starts_with(['}', ')', ']'])
            || text.starts_with("</")
            || matches!(text.trim_end(), "end" | "fi" | "done" | "esac" | "endif")
    };
    let mut ranges = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        let start_row = node.start_position().row;
        let mut end_row = node.end_position().row;
        // A node ending at column 0 of a row does not cover that row.
        if node.end_position().column == 0 && end_row > start_row {
            end_row -= 1;
        }
        if node.is_named() && end_row > start_row && syntax_kind_is_foldable(node.kind()) {
            let end = if closes_only(end_row) && end_row - 1 > start_row {
                end_row - 1
            } else {
                end_row
            };
            if end > start_row {
                ranges.push((start_row, end));
            }
        }
        // Depth-first walk.
        if end_row > start_row && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                ranges.sort_unstable();
                ranges.dedup();
                return ranges;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested() -> FoldManager {
        // 0..9 outer, 2..4 inner, 6..8 second inner
        let mut m = FoldManager::new();
        m.set_auto_folds(&[(0, 9), (2, 4), (6, 8)], 12, 1, true);
        m
    }

    #[test]
    fn closing_hides_the_body_but_not_the_header() {
        let mut m = nested();
        assert!(m.close_one(3));
        assert!(!m.is_line_hidden(2));
        assert!(m.is_line_hidden(3));
        assert!(m.is_line_hidden(4));
        assert!(!m.is_line_hidden(5));
        assert_eq!(m.folded_line_count_at(2), Some(2));
    }

    /// zc closes the innermost open fold, then the enclosing one; zo reopens
    /// the outermost closed fold first (Vim's one-level semantics).
    #[test]
    fn zc_and_zo_walk_one_level_at_a_time() {
        let mut m = nested();
        assert!(m.close_one(3)); // inner
        assert!(m.close_one(3)); // now the outer one
        assert!(m.is_line_hidden(3), "outer closed hides everything below 0");
        assert!(!m.close_one(3), "nothing left to close");
        assert!(m.open_one(3)); // outer opens, inner stays closed
        assert!(m.is_line_hidden(3));
        assert!(!m.is_line_hidden(1));
        assert!(m.open_one(3));
        assert!(!m.is_line_hidden(3));
    }

    #[test]
    fn za_toggles_and_recursive_variants_touch_the_whole_chain() {
        let mut m = nested();
        assert!(m.toggle_one(3));
        assert!(m.is_line_hidden(3));
        assert!(m.toggle_one(2)); // on the closed header: opens
        assert!(!m.is_line_hidden(3));
        assert!(m.close_recursive(3));
        assert!(m.is_line_hidden(1));
        assert!(m.open_recursive(3));
        assert!(!m.is_line_hidden(1) && !m.is_line_hidden(3));
    }

    #[test]
    fn zr_zm_open_and_close_everything() {
        let mut m = nested();
        m.close_all();
        assert_eq!(m.hidden_ranges(), vec![(1, 9)]);
        m.open_all();
        assert!(m.hidden_ranges().is_empty());
    }

    /// The structure of the `nvim` experiments below (tabs as indentation):
    /// 0 a / 1 b / 2 c / 3 d / 4 d2 / 5 c2 / 6 b2 / 7 e / 8 f / 9 a2, which
    /// gives folds (0,8) L1, (1,5) L2, (2,4) L3, (6,8) L2, (7,8) L3.
    fn three_levels() -> FoldManager {
        let mut m = FoldManager::new();
        m.set_auto_folds(&[(0, 8), (1, 5), (2, 4), (6, 8), (7, 8)], 10, 1, false);
        m
    }

    /// `nvim -u NONE` with `foldmethod=indent`, same nesting: after `zM`,
    /// `zr` opens one level at a time (`foldclosed()` per level), `2zr` from
    /// 1 reaches the deepest, `zr` at the deepest changes nothing and `zm`
    /// closes the deepest level first.
    #[test]
    fn zr_and_zm_walk_the_fold_levels_like_vim() {
        let mut m = three_levels();
        assert_eq!(m.deepest_nesting(), 3);
        m.close_all();
        assert_eq!(m.foldlevel(), 0);
        assert_eq!(m.hidden_ranges(), vec![(1, 8)]);
        m.reduce_folding(1);
        assert_eq!(m.foldlevel(), 1);
        assert_eq!(m.hidden_ranges(), vec![(2, 5), (7, 8)], "L2 folds closed");
        m.reduce_folding(1);
        assert_eq!(m.hidden_ranges(), vec![(3, 4), (8, 8)], "only L3 closed");
        m.reduce_folding(5);
        assert_eq!(m.foldlevel(), 3, "zr stops at the deepest nesting");
        assert!(m.hidden_ranges().is_empty());
        m.fold_more(1);
        assert_eq!(m.hidden_ranges(), vec![(3, 4), (8, 8)]);
        m.fold_more(9);
        assert_eq!(m.foldlevel(), 0);
        m.fold_more(1);
        assert_eq!(m.foldlevel(), 0, "zm at foldlevel 0 does nothing");
    }

    /// A buffer that never used a fold level starts fully open with the
    /// deepest nesting as its level: the first `zm` closes the deepest folds
    /// (divergence from Vim, where `foldlevel=99` makes ~96 `zm` invisible).
    #[test]
    fn a_fresh_buffer_counts_as_fully_open_for_zm() {
        let mut m = three_levels();
        assert_eq!(m.foldlevel(), 3);
        m.fold_more(1);
        assert_eq!(m.hidden_ranges(), vec![(3, 4), (8, 8)]);
    }

    /// `nvim`: `zM`, cursor on a line in the nested fold, `zo` then `zx`
    /// re-applies foldlevel 0 (manual `zo` forgotten) and reveals the cursor
    /// line; `zX` re-applies without revealing.
    #[test]
    fn zx_forgets_manual_state_then_reveals_the_cursor_line() {
        let mut m = three_levels();
        m.close_all();
        m.open_one(3);
        assert!(!m.is_line_hidden(1));
        m.reapply_foldlevel();
        assert_eq!(m.hidden_ranges(), vec![(1, 8)], "zX: all closed again");
        m.reveal(3);
        assert!(!m.is_line_hidden(3));
        assert!(m.is_line_hidden(8), "unrelated fold (7,8) stays closed");
    }

    /// A fold that appears after a recompute follows the current foldlevel.
    #[test]
    fn new_folds_follow_the_foldlevel() {
        let mut m = FoldManager::new();
        m.set_auto_folds(&[(0, 4), (1, 3)], 10, 1, false);
        m.close_all();
        m.set_auto_folds(&[(0, 4), (1, 3), (6, 9)], 10, 2, false);
        assert_eq!(m.hidden_ranges(), vec![(1, 4), (7, 9)]);
    }

    /// `nvim`, folds 2-9 / 3-6 / 4-5 / 8-9 (1-based, all open): `[z` from 3 is
    /// 2, from 4 is 3, from 5 is 4, from 6 is 3, from 8 is 2 (already at the
    /// start: the enclosing fold), from 2 stays (no enclosing fold); `]z` from
    /// 4 is 5, from 5 is 6, from 6 is 9, from 9 stays; a closed fold ends the
    /// chain: with 4-5 closed, `[z` on 4 is 3 and with everything closed it
    /// fails.
    #[test]
    fn fold_edge_motions_match_vim() {
        let mut m = FoldManager::new();
        // 0-based versions of the 1-based folds above.
        m.set_auto_folds(&[(1, 8), (2, 5), (3, 4), (7, 8)], 10, 1, false);
        let start = |m: &FoldManager, line1: usize| m.fold_edge_target(line1 - 1, false);
        let end = |m: &FoldManager, line1: usize| m.fold_edge_target(line1 - 1, true);
        assert_eq!(start(&m, 3), Some(1));
        assert_eq!(start(&m, 4), Some(2));
        assert_eq!(start(&m, 5), Some(3));
        assert_eq!(start(&m, 6), Some(2));
        assert_eq!(start(&m, 8), Some(1));
        assert_eq!(start(&m, 2), None);
        assert_eq!(start(&m, 1), None);
        assert_eq!(end(&m, 4), Some(4));
        assert_eq!(end(&m, 5), Some(5));
        assert_eq!(end(&m, 6), Some(8));
        assert_eq!(end(&m, 9), None);
        m.close_one(3); // closes the innermost open fold (4-5) at line 4
        assert_eq!(start(&m, 4), Some(2));
        assert_eq!(end(&m, 4), Some(5));
        m.close_all();
        assert_eq!(start(&m, 4), None);
        assert_eq!(end(&m, 4), None);
    }

    #[test]
    fn hidden_ranges_merge_and_respect_foldenable() {
        let mut m = FoldManager::new();
        m.set_auto_folds(&[(0, 3), (4, 7)], 10, 1, true);
        m.close_all();
        // The second header (line 4) stays visible between the two bodies.
        assert_eq!(m.hidden_ranges(), vec![(1, 3), (5, 7)]);
        let mut touching = FoldManager::new();
        touching.set_auto_folds(&[(0, 3), (3, 7)], 10, 1, true);
        assert!(touching.folds().len() == 1, "crossing ranges keep only one");
        m.set_enabled(false);
        assert!(m.hidden_ranges().is_empty());
        assert!(!m.is_line_hidden(2));
    }

    #[test]
    fn zj_and_zk_jump_between_folds() {
        let m = nested();
        assert_eq!(m.next_fold_start(0), Some(2));
        assert_eq!(m.next_fold_start(2), Some(6));
        assert_eq!(m.next_fold_start(6), None);
        assert_eq!(m.previous_fold_end(10), Some(9));
        assert_eq!(m.previous_fold_end(6), Some(4));
        assert_eq!(m.previous_fold_end(2), None);
    }

    #[test]
    fn recompute_keeps_the_closed_state_of_surviving_folds() {
        let mut m = nested();
        m.close_one(3);
        m.set_auto_folds(&[(0, 10), (2, 4), (6, 8)], 12, 2, true);
        assert!(m.is_line_hidden(3), "inner fold stayed closed");
        assert!(!m.is_line_hidden(1), "outer stayed open");
    }

    #[test]
    fn manual_folds_survive_recompute_and_start_closed() {
        let mut m = FoldManager::new();
        m.create_fold(1, 3);
        assert!(m.is_line_hidden(2));
        m.set_auto_folds(&[(0, 5)], 8, 1, true);
        assert!(m.is_line_hidden(2));
        m.delete_fold_at(2);
        assert!(!m.is_line_hidden(2));
        assert_eq!(m.folds().len(), 1);
    }

    #[test]
    fn crossing_ranges_are_dropped() {
        let mut m = FoldManager::new();
        m.set_auto_folds(&[(0, 5), (3, 8)], 10, 1, true);
        assert_eq!(m.folds().len(), 1);
    }

    #[test]
    fn line_count_changes_shift_folds_below_the_edit() {
        let mut m = nested();
        m.note_line_count(12);
        // A line was inserted at line 1: folds below shift down by one,
        // the enclosing fold grows.
        m.adjust_for_line_count(13, 1);
        let ranges: Vec<_> = m
            .folds()
            .iter()
            .map(|f| (f.start_line(), f.end_line()))
            .collect();
        assert_eq!(ranges, vec![(0, 10), (3, 5), (7, 9)]);
    }

    #[test]
    fn j_and_k_step_over_closed_folds() {
        let mut m = FoldManager::new();
        m.set_auto_folds(&[(2, 5)], 10, 1, true);
        m.close_all();
        assert_eq!(m.step_down(1, 1, 9), 2);
        assert_eq!(
            m.step_down(2, 1, 9),
            6,
            "from the header to the next visible line"
        );
        assert_eq!(m.step_down(1, 3, 9), 7, "1 -> fold header -> 6 -> 7");
        assert_eq!(m.step_up(6, 1), 2, "back onto the header");
        assert_eq!(m.step_up(6, 2), 1);
        assert_eq!(m.step_down(8, 5, 9), 9, "stops at the last line");
    }

    #[test]
    fn visible_index_maps_around_hidden_lines() {
        let mut m = FoldManager::new();
        m.set_auto_folds(&[(2, 5), (8, 9)], 12, 1, true);
        m.close_all();
        // hidden: 3..=5 and 9
        assert_eq!(m.hidden_count_before(6), 3);
        assert_eq!(m.visible_index(6), 3);
        assert_eq!(m.visible_index(10), 6);
        for line in [0, 1, 2, 6, 7, 8, 10, 11] {
            assert_eq!(m.line_at_visible_index(m.visible_index(line)), line);
        }
    }

    #[test]
    fn next_visible_line_skips_a_closed_body() {
        let mut m = FoldManager::new();
        m.set_auto_folds(&[(2, 5)], 10, 1, true);
        m.close_all();
        assert_eq!(m.next_visible_line(3, 10), Some(6));
        assert_eq!(m.next_visible_line(2, 10), Some(2));
        assert_eq!(m.next_visible_line(6, 10), Some(6));
    }

    /// The old per-line bookkeeping rescanned every fold for every query, so
    /// a few thousand folds made each keystroke cost tens of milliseconds.
    /// 20k nested folds: building, closing and 100k queries finish in well
    /// under a second now (they took minutes when quadratic).
    #[test]
    fn thousands_of_folds_cost_a_single_pass() {
        let ranges: Vec<(usize, usize)> = (0..10_000)
            .flat_map(|n| [(n * 10, n * 10 + 8), (n * 10 + 2, n * 10 + 6)])
            .collect();
        let started = std::time::Instant::now();
        let mut m = FoldManager::new();
        m.set_auto_folds(&ranges, 100_000, 1, true);
        m.close_all();
        assert_eq!(m.folds().len(), 20_000);
        let mut hidden = 0;
        for line in 0..100_000 {
            hidden += usize::from(m.is_line_hidden(line));
            m.visible_index(line);
        }
        assert_eq!(hidden, 10_000 * 8);
        assert_eq!(m.closed_fold_headers().count(), 10_000);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn generation_changes_only_when_the_folds_do() {
        let mut m = nested();
        let first = m.generation();
        assert!(!m.open_one(3), "already open: nothing changes");
        assert_eq!(m.generation(), first);
        assert!(m.close_one(3));
        let closed = m.generation();
        assert_ne!(closed, first);
        // An identical recompute keeps the generation (and the state).
        m.set_auto_folds(&[(0, 9), (2, 4), (6, 8)], 12, 2, true);
        assert_eq!(m.generation(), closed);
        m.set_auto_folds(&[(0, 9), (2, 5), (6, 8)], 12, 3, true);
        assert_ne!(m.generation(), closed);
        m.set_enabled(true);
        let before = m.generation();
        m.set_enabled(false);
        assert_ne!(m.generation(), before);
        assert_ne!(
            FoldManager::new().generation(),
            FoldManager::new().generation(),
            "a replaced manager never repeats a generation"
        );
    }

    /// Markers sit on the headers of the outermost closed folds, once each,
    /// counting the lines the fold hides.
    #[test]
    fn closed_fold_headers_are_the_outermost_closed_folds() {
        let mut m = nested();
        assert_eq!(m.closed_fold_headers().count(), 0);
        m.close_one(3);
        assert_eq!(m.closed_fold_headers().collect::<Vec<_>>(), vec![(2, 2)]);
        m.close_one(3);
        assert_eq!(
            m.closed_fold_headers().collect::<Vec<_>>(),
            vec![(0, 9)],
            "the outer fold swallows the nested marker"
        );
        m.set_enabled(false);
        assert_eq!(m.closed_fold_headers().count(), 0);
    }

    /// The indexed lookups agree with a line-by-line walk.
    #[test]
    fn indexed_lookups_match_a_naive_walk() {
        let mut m = FoldManager::new();
        m.set_auto_folds(
            &[
                (0, 3),
                (2, 3),
                (5, 9),
                (6, 7),
                (9, 9 + 1),
                (14, 20),
                (16, 18),
            ],
            30,
            1,
            true,
        );
        m.close_all();
        m.open_one(6); // opens (5,9) only
        let naive_hidden: Vec<bool> = (0..30)
            .map(|line| {
                m.folds()
                    .iter()
                    .any(|f| !f.is_open() && line > f.start_line() && line <= f.end_line())
            })
            .collect();
        for line in 0..30 {
            assert_eq!(m.is_line_hidden(line), naive_hidden[line], "line {line}");
            let before = naive_hidden[..line].iter().filter(|&&h| h).count();
            assert_eq!(m.hidden_count_before(line), before, "before {line}");
            assert_eq!(m.visible_index(line), line - before);
        }
        let visible: Vec<usize> = (0..30).filter(|&line| !naive_hidden[line]).collect();
        for (index, &line) in visible.iter().enumerate() {
            assert_eq!(m.line_at_visible_index(index), line, "index {index}");
        }
    }

    #[test]
    fn indent_folds_follow_indentation_and_ignore_blank_lines() {
        let lines: Vec<String> = [
            "class A {",      // 0
            "    void f() {", // 1
            "        x();",   // 2
            "",               // 3
            "        y();",   // 4
            "    }",          // 5
            "}",              // 6
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(indent_fold_ranges(&lines, 4), vec![(0, 5), (1, 4)]);
    }
}
