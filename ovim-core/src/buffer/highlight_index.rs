use crate::syntax::HighlightGroup;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

type Highlight = (Range<usize>, HighlightGroup);

#[derive(Debug)]
struct IntervalNode {
    center: usize,
    /// Original highlight ordinals crossing `center`, sorted for directional
    /// early exits. Query results are sorted back into original order.
    by_start: Vec<usize>,
    by_end: Vec<usize>,
    left: Option<Box<IntervalNode>>,
    right: Option<Box<IntervalNode>>,
}

impl IntervalNode {
    fn build(highlights: &[Highlight], ordinals: Vec<usize>) -> Option<Box<Self>> {
        if ordinals.is_empty() {
            return None;
        }
        let mut points = Vec::with_capacity(ordinals.len());
        for &index in &ordinals {
            points.push(highlights[index].0.start);
        }
        points.sort_unstable();
        let center = points[points.len() / 2];
        let mut left = Vec::new();
        let mut crossing = Vec::new();
        let mut right = Vec::new();
        for index in ordinals {
            let range = &highlights[index].0;
            if range.end <= center {
                left.push(index);
            } else if range.start > center {
                right.push(index);
            } else {
                crossing.push(index);
            }
        }
        let mut by_start = crossing.clone();
        by_start.sort_unstable_by_key(|&index| highlights[index].0.start);
        let mut by_end = crossing;
        by_end.sort_unstable_by_key(|&index| std::cmp::Reverse(highlights[index].0.end));
        Some(Box::new(Self {
            center,
            by_start,
            by_end,
            left: Self::build(highlights, left),
            right: Self::build(highlights, right),
        }))
    }

    fn query(&self, highlights: &[Highlight], range: &Range<usize>, output: &mut Vec<usize>) {
        if range.end <= self.center {
            for &index in &self.by_start {
                if highlights[index].0.start >= range.end {
                    break;
                }
                if highlights[index].0.end > range.start {
                    output.push(index);
                }
            }
            if let Some(left) = &self.left {
                left.query(highlights, range, output);
            }
        } else if range.start > self.center {
            for &index in &self.by_end {
                if highlights[index].0.end <= range.start {
                    break;
                }
                if highlights[index].0.start < range.end {
                    output.push(index);
                }
            }
            if let Some(right) = &self.right {
                right.query(highlights, range, output);
            }
        } else {
            output.extend(self.by_start.iter().copied());
            if let Some(left) = &self.left {
                left.query(highlights, range, output);
            }
            if let Some(right) = &self.right {
                right.query(highlights, range, output);
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct LineHighlightIndex {
    highlights: Arc<[Highlight]>,
    root: Option<Box<IntervalNode>>,
}

impl LineHighlightIndex {
    fn new(highlights: Vec<Highlight>) -> Self {
        let highlights: Arc<[Highlight]> = highlights.into();
        let ordinals = highlights
            .iter()
            .enumerate()
            .filter_map(|(index, (range, _))| (range.start < range.end).then_some(index))
            .collect();
        let root = IntervalNode::build(&highlights, ordinals);
        Self { highlights, root }
    }

    fn query(&self, range: Range<usize>) -> Vec<Highlight> {
        if range.start >= range.end {
            return Vec::new();
        }
        let mut ordinals = Vec::new();
        if let Some(root) = &self.root {
            root.query(&self.highlights, &range, &mut ordinals);
        }
        ordinals.sort_unstable();
        ordinals.dedup();
        ordinals
            .into_iter()
            .map(|index| self.highlights[index].clone())
            .collect()
    }
}

#[derive(Debug, Default)]
pub(super) struct HighlightRangeCache {
    generation: Option<(u64, u64)>,
    lines: BTreeMap<usize, ((u64, u64), Arc<LineHighlightIndex>)>,
}

impl HighlightRangeCache {
    fn sync_generation(&mut self, generation: (u64, u64)) {
        if self.generation != Some(generation) {
            self.lines.clear();
            self.generation = Some(generation);
        }
    }

    pub(super) fn get(
        &mut self,
        line: usize,
        generation: (u64, u64),
    ) -> Option<Arc<LineHighlightIndex>> {
        self.sync_generation(generation);
        self.lines
            .get(&line)
            .filter(|(cached_generation, _)| *cached_generation == generation)
            .map(|(_, index)| index.clone())
    }

    pub(super) fn insert(
        &mut self,
        line: usize,
        generation: (u64, u64),
        highlights: Vec<Highlight>,
    ) -> Arc<LineHighlightIndex> {
        self.sync_generation(generation);
        let index = Arc::new(LineHighlightIndex::new(highlights));
        self.lines.insert(line, (generation, index.clone()));
        index
    }
}

pub(super) fn query_cached_index(
    index: &LineHighlightIndex,
    range: Range<usize>,
) -> Vec<Highlight> {
    index.query(range)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_query_is_bounded_and_preserves_original_precedence_order() {
        let index = LineHighlightIndex::new(vec![
            (0..100, HighlightGroup::String),
            (80..90, HighlightGroup::Comment),
            (20..30, HighlightGroup::Keyword),
            (25..85, HighlightGroup::Type),
        ]);
        assert_eq!(
            index.query(82..84),
            vec![
                (0..100, HighlightGroup::String),
                (80..90, HighlightGroup::Comment),
                (25..85, HighlightGroup::Type),
            ]
        );
    }
}
