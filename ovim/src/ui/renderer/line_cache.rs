//! Per-line rendering cache.
//!
//! Caches the expensive per-line rendering output (tab expansion, horizontal
//! viewport slicing, highlight computation) to avoid recomputation when only
//! the cursor moves. Entries are grouped by the frame they were rendered under
//! (buffer, version, width, scroll...), so a change to any of those makes the
//! old entries unreachable, while the panes of a split each keep their own.

use ratatui::text::Line;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

#[derive(Clone)]
pub(crate) struct IndexedRenderLine {
    pub layout: Arc<ovim_core::line_layout::IndexedLineLayout>,
    pub transform: Option<Arc<ovim_core::markdown_conceal::LineTransform>>,
    pub links: Arc<[ovim_core::markdown_conceal::ConcealedLink]>,
}

struct IndexedCacheEntry {
    source: Weak<ovim_core::text_index::LineIndex>,
    width: usize,
    tab_width: usize,
    conceal: bool,
    decorations: u64,
    rendered: IndexedRenderLine,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ChatBubbleCacheKey {
    pub conversation_id: u64,
    pub node_id: u64,
    pub panel_width: usize,
    pub selected: bool,
    pub allow_edits: bool,
    pub thinking_expanded: bool,
    pub child_count: usize,
    pub branch_position: Option<(usize, usize)>,
    pub theme_hash: u64,
    pub terminal_image_support: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct CachedChatImage {
    pub row: usize,
    pub x: u16,
    pub width: u16,
    pub height: u16,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct CachedChatBubble {
    pub lines: Vec<Line<'static>>,
    pub images: Vec<CachedChatImage>,
}

/// Per-frame inputs every cached line's validity depends on. Built once
/// per render pass; `key()` adds the per-line parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineCacheFrame {
    /// Stable buffer identity (`Buffer::id`). Must NOT be the buffer index:
    /// two different buffers can occupy the same index over time (e.g. a
    /// walkthrough replacing its presentation buffer), and both start at
    /// version 0, so index+version cannot tell them apart.
    pub buffer_id: u64,
    /// `Buffer::version`, which moves on text edits only.
    pub buffer_version: usize,
    /// `Buffer::highlight_projection_generation`. Highlights can change
    /// without a text edit (background syntax, LSP semantic tokens,
    /// debounced rehighlight); those bump this generation but not
    /// `buffer_version`. Without it, such frames hit the cache unchanged
    /// and the terminal diff repaints nothing until the next edit.
    pub highlight_generation: u64,
    /// Horizontal scroll offset (display columns)
    pub h_offset: usize,
    /// Available text width (columns)
    pub text_width: usize,
    /// Whether wrap mode is enabled
    pub wrap: bool,
    /// Tab width setting
    pub tab_width: usize,
    /// Whether markdown conceal is active
    pub markdown_conceal: bool,
}

impl LineCacheFrame {
    /// Key for one line of this frame. `decoration_hash` is the per-line
    /// decoration fingerprint from `ProjectedDecorations::line_hash`, so an
    /// LSP push that touches one line invalidates only that line.
    pub fn key(self, line_idx: usize, decoration_hash: u64) -> LineCacheKey {
        LineCacheKey {
            frame: self,
            line_idx,
            decoration_hash,
        }
    }
}

/// Key identifying a cached rendered line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineCacheKey {
    frame: LineCacheFrame,
    line_idx: usize,
    decoration_hash: u64,
}

/// A cached rendered line (before soft-wrap splitting).
#[derive(Debug, Clone)]
struct CachedLine {
    /// The rendered styled line (pre-wrap)
    line: Line<'static>,
    /// Whether this line had any special highlighting when cached
    /// (visual selection, search, cursorline, yank flash).
    /// Lines with transient highlighting are NOT cached because
    /// they change every frame.
    is_stable: bool,
}

/// The lines one frame (typically one pane) rendered.
struct FrameLines {
    frame: LineCacheFrame,
    lines: HashMap<usize, (LineCacheKey, CachedLine)>,
    /// `LineRenderCache::tick` when the frame was last begun, for eviction.
    last_used: u64,
}

/// Frames kept at once: enough for every pane of a busy split layout, plus
/// the frames of the last few edits. Older ones are dropped, least recently
/// used first.
const MAX_FRAMES: usize = 8;

/// Per-line rendering cache that avoids recomputing expensive rendering
/// for unchanged lines.
///
/// # Invalidation strategy
///
/// - **Buffer edit / highlight arrival / scroll / resize**: Entries belong to
///   the `LineCacheFrame` they were rendered under; when any field changes the
///   old entries can no longer be reached. Fine-grained per-line invalidation
///   would require tracking which lines shifted, which isn't worth the
///   complexity.
/// - **Split panes**: each pane renders under its own frame (buffer, width,
///   scroll), and the cache keeps a few frames at once, so a pane's lines
///   survive the other panes being rendered in between.
/// - **Cursor move**: Only the cursor line and previous cursor line are
///   excluded from caching (they have transient cursorline highlighting).
/// - **Visual selection / search / yank flash**: Lines with these overlays
///   are rendered fresh each frame (marked `is_stable: false`).
pub struct LineRenderCache {
    /// Lines per frame, at most [`MAX_FRAMES`].
    frames: Vec<FrameLines>,
    /// Index into `frames` of the frame begun last.
    current: usize,
    /// Counts `begin_frame` calls.
    tick: u64,
    indexed: HashMap<(u64, usize), IndexedCacheEntry>,
    chat_bubbles: HashMap<ChatBubbleCacheKey, CachedChatBubble>,
    /// Capacity limit per frame to prevent unbounded growth
    max_entries: usize,
    /// Stats: cache hits this frame
    pub hits: usize,
    /// Stats: cache misses this frame
    pub misses: usize,
    pub chat_hits: usize,
    pub chat_misses: usize,
}

impl Default for LineRenderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl LineRenderCache {
    pub fn new() -> Self {
        Self {
            frames: Vec::new(),
            current: 0,
            tick: 0,
            indexed: HashMap::new(),
            chat_bubbles: HashMap::with_capacity(128),
            max_entries: 1024,
            hits: 0,
            misses: 0,
            chat_hits: 0,
            chat_misses: 0,
        }
    }

    /// Clear the entire cache (e.g., on buffer edit or resize).
    pub fn clear(&mut self) {
        self.frames.clear();
        self.current = 0;
    }

    /// The lines of the frame begun last.
    fn current_lines(&mut self) -> Option<&mut HashMap<usize, (LineCacheKey, CachedLine)>> {
        self.frames
            .get_mut(self.current)
            .map(|frame| &mut frame.lines)
    }

    /// Retain core geometry for nowrap viewports, where no window WrapMap
    /// owns it. Source identity preserves unrelated rows across buffer edits.
    pub(crate) fn indexed_line(
        &mut self,
        buffer_id: u64,
        line: usize,
        source: Arc<ovim_core::text_index::LineIndex>,
        width: usize,
        tab_width: usize,
        conceal: bool,
        decorations: u64,
        inline: &[&ovim_core::editor::decoration::Decoration],
        line_start_char: usize,
    ) -> IndexedRenderLine {
        let key = (buffer_id, line);
        if let Some(entry) = self.indexed.get(&key) {
            if entry
                .source
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, &source))
                && entry.width == width
                && entry.tab_width == tab_width
                && entry.conceal == conceal
                && entry.decorations == decorations
            {
                return entry.rendered.clone();
            }
        }
        let mut transform = None;
        let mut links: Arc<[ovim_core::markdown_conceal::ConcealedLink]> = Arc::from([]);
        let index = if conceal {
            let raw = source.slice_chars(0..source.len_chars());
            let spans = ovim_core::markdown_conceal::scan_markdown_conceal(&raw);
            if spans.is_empty() {
                source.clone()
            } else {
                let mapped = ovim_core::markdown_conceal::apply_conceal(&raw, &spans);
                links =
                    ovim_core::markdown_conceal::extract_concealed_links(&spans, &mapped).into();
                let index = ovim_core::text_index::LineIndex::from_text(&mapped.text);
                transform = Some(Arc::new(mapped));
                index
            }
        } else {
            source.clone()
        };
        let inline_text = inline
            .iter()
            .map(|decoration| {
                let anchor = decoration
                    .placement
                    .char_offset()
                    .saturating_sub(line_start_char);
                let anchor = if let Some(mapped) = &transform {
                    let byte = source.char_to_byte(anchor);
                    mapped
                        .src_to_view
                        .get(byte)
                        .copied()
                        .unwrap_or(index.len_chars())
                } else {
                    anchor
                };
                (anchor, Arc::<str>::from(decoration.text.as_str()))
            })
            .collect::<Vec<_>>()
            .into();
        let rendered = IndexedRenderLine {
            layout: Arc::new(ovim_core::line_layout::IndexedLineLayout::with_inline_text(
                index,
                width,
                tab_width,
                inline_text,
            )),
            transform,
            links,
        };
        if self.indexed.len() >= self.max_entries {
            self.indexed.clear();
        }
        self.indexed.insert(
            key,
            IndexedCacheEntry {
                source: Arc::downgrade(&source),
                width,
                tab_width,
                conceal,
                decorations,
                rendered: rendered.clone(),
            },
        );
        rendered
    }

    /// Reset per-frame stats.
    pub fn reset_stats(&mut self) {
        self.hits = 0;
        self.misses = 0;
        self.chat_hits = 0;
        self.chat_misses = 0;
    }

    pub(crate) fn get_chat_bubble(&mut self, key: &ChatBubbleCacheKey) -> Option<CachedChatBubble> {
        let cached = self.chat_bubbles.get(key).cloned();
        if cached.is_some() {
            self.chat_hits += 1;
        } else {
            self.chat_misses += 1;
        }
        cached
    }

    pub(crate) fn insert_chat_bubble(&mut self, key: ChatBubbleCacheKey, bubble: CachedChatBubble) {
        const MAX_CHAT_BUBBLES: usize = 512;
        if self.chat_bubbles.len() >= MAX_CHAT_BUBBLES {
            self.chat_bubbles.clear();
        }
        self.chat_bubbles.insert(key, bubble);
    }

    /// Start a render pass under `frame`: lookups and stores address the
    /// lines rendered under exactly this frame, which stay available across
    /// passes of other frames (the other panes of a split). Resets the
    /// hit/miss stats. Call once per frame before any line lookup.
    pub fn begin_frame(&mut self, frame: LineCacheFrame) {
        self.tick += 1;
        let index = match self.frames.iter().position(|known| known.frame == frame) {
            Some(index) => index,
            None => {
                if self.frames.len() >= MAX_FRAMES {
                    let oldest = self
                        .frames
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, known)| known.last_used)
                        .map(|(index, _)| index)
                        .unwrap_or(0);
                    self.frames.swap_remove(oldest);
                }
                self.frames.push(FrameLines {
                    frame,
                    lines: HashMap::with_capacity(256),
                    last_used: 0,
                });
                self.frames.len() - 1
            }
        };
        self.frames[index].last_used = self.tick;
        self.current = index;
        self.reset_stats();
    }

    /// Check if a rendered line is cached and still valid.
    ///
    /// Returns `None` if the line was never cached, its key no longer
    /// matches (frame or decorations changed), or the cached entry had
    /// transient highlighting.
    pub fn get(&mut self, key: &LineCacheKey) -> Option<&Line<'static>> {
        let Self {
            frames,
            current,
            hits,
            misses,
            ..
        } = self;
        let cached = frames
            .get(*current)
            .and_then(|frame| frame.lines.get(&key.line_idx))
            .filter(|(cached_key, cached)| cached_key == key && cached.is_stable);
        match cached {
            Some((_, cached)) => {
                *hits += 1;
                Some(&cached.line)
            }
            None => {
                *misses += 1;
                None
            }
        }
    }

    /// Store a rendered line in the cache.
    ///
    /// `is_stable` should be `false` for lines with transient overlays
    /// (cursor line, visual selection, search highlights, yank flash).
    pub fn put(&mut self, key: LineCacheKey, line: Line<'static>, is_stable: bool) {
        // Evict if over capacity — keep entries near the current viewport
        // instead of clearing everything (which causes a full cache-miss storm
        // on the next frame).
        let max_entries = self.max_entries;
        let Some(entries) = self.current_lines() else {
            return;
        };
        if entries.len() >= max_entries {
            let center = key.line_idx;
            let keep_radius = max_entries / 2;
            let lo = center.saturating_sub(keep_radius);
            let hi = center.saturating_add(keep_radius);
            entries.retain(|&idx, _| idx >= lo && idx <= hi);
        }

        entries.insert(key.line_idx, (key, CachedLine { line, is_stable }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    fn make_line(text: &str) -> Line<'static> {
        Line::from(vec![Span::raw(text.to_string())])
    }

    fn test_frame(buffer_id: u64, buffer_version: usize) -> LineCacheFrame {
        LineCacheFrame {
            buffer_id,
            buffer_version,
            highlight_generation: 0,
            h_offset: 0,
            text_width: 80,
            wrap: false,
            tab_width: 4,
            markdown_conceal: false,
        }
    }

    /// A cache with one render pass begun under `frame(1, 1)`.
    fn cache_in_frame() -> (LineRenderCache, LineCacheFrame) {
        let mut cache = LineRenderCache::new();
        let frame = test_frame(1, 1);
        cache.begin_frame(frame);
        (cache, frame)
    }

    #[test]
    fn cache_hit() {
        let (mut cache, frame) = cache_in_frame();
        cache.put(frame.key(0, 0), make_line("hello"), true);

        assert!(cache.get(&frame.key(0, 0)).is_some());
        assert_eq!(cache.hits, 1);
        assert_eq!(cache.misses, 0);
    }

    #[test]
    fn cache_miss_highlight_generation_change() {
        // Highlights arriving without a text edit (background syntax, LSP
        // semantic tokens) leave buffer_version unchanged; the frame must
        // still re-render stable lines or the screen never repaints.
        let (mut cache, frame) = cache_in_frame();
        cache.put(frame.key(0, 0), make_line("plain"), true);
        assert!(cache.get(&frame.key(0, 0)).is_some());

        let highlighted = LineCacheFrame {
            highlight_generation: frame.highlight_generation + 1,
            ..frame
        };
        cache.begin_frame(highlighted);
        assert!(cache.get(&highlighted.key(0, 0)).is_none());

        // Same frame again: no spurious invalidation.
        cache.put(highlighted.key(0, 0), make_line("styled"), true);
        cache.begin_frame(highlighted);
        assert!(cache.get(&highlighted.key(0, 0)).is_some());
    }

    #[test]
    fn cache_miss_version_change() {
        let (mut cache, frame) = cache_in_frame();
        cache.put(frame.key(0, 0), make_line("hello"), true);

        let edited = test_frame(1, 2);
        cache.begin_frame(edited);
        assert!(cache.get(&edited.key(0, 0)).is_none());
        assert_eq!(cache.misses, 1);
    }

    #[test]
    fn cache_miss_buffer_identity_change() {
        // Two fresh buffers swapped at the same index share version 0; the
        // stable buffer id must still invalidate the cache between them.
        let mut cache = LineRenderCache::new();
        let first = test_frame(1, 0);
        cache.begin_frame(first);
        cache.put(first.key(0, 0), make_line("stale"), true);

        let second = test_frame(2, 0);
        cache.begin_frame(second);
        assert!(cache.get(&second.key(0, 0)).is_none());
        assert!(cache.get(&second.key(0, 0)).is_none());
    }

    #[test]
    fn cache_miss_viewport_change() {
        let (mut cache, frame) = cache_in_frame();
        cache.put(frame.key(0, 0), make_line("hello"), true);

        let scrolled = LineCacheFrame {
            h_offset: 5,
            ..frame
        };
        cache.begin_frame(scrolled);
        assert!(cache.get(&scrolled.key(0, 0)).is_none());
    }

    #[test]
    fn unstable_lines_not_cached() {
        let (mut cache, frame) = cache_in_frame();
        // Store with is_stable=false (e.g., cursor line)
        cache.put(frame.key(0, 0), make_line("cursor"), false);

        assert!(cache.get(&frame.key(0, 0)).is_none());
    }

    #[test]
    fn eviction_keeps_nearby_lines() {
        let (mut cache, frame) = cache_in_frame();
        cache.max_entries = 10; // small cap for test

        // Fill cache with lines 0..10
        for i in 0..10 {
            cache.put(frame.key(i, 0), make_line("x"), true);
        }
        assert_eq!(cache.current_lines().unwrap().len(), 10);

        // Insert line 8 — should evict lines far from 8 (keep 3..13)
        cache.put(frame.key(8, 0), make_line("new"), true);

        // Lines near 8 should survive, line 0 should be evicted
        let entries = cache.current_lines().unwrap();
        assert!(entries.contains_key(&8));
        assert!(entries.contains_key(&5));
        assert!(!entries.contains_key(&0));
        assert!(!entries.contains_key(&1));
        assert!(!entries.contains_key(&2));
    }

    #[test]
    fn per_line_decoration_hash_isolates_invalidation() {
        // Two lines cached. Only line 0's decoration hash changes — line 1
        // should still hit. Demonstrates that an LSP push touching one line
        // no longer invalidates the entire cache (the regression Fix #6
        // addressed).
        let (mut cache, frame) = cache_in_frame();
        cache.put(frame.key(0, 100), make_line("a"), true);
        cache.put(frame.key(1, 200), make_line("b"), true);

        // Line 0's hash changed (e.g. new diagnostic).
        assert!(cache.get(&frame.key(0, 101)).is_none());
        // Line 1 is untouched — should still hit with its original hash.
        assert!(cache.get(&frame.key(1, 200)).is_some());
    }

    #[test]
    fn cache_miss_decoration_hash_change() {
        let (mut cache, frame) = cache_in_frame();
        cache.put(frame.key(0, 42), make_line("x"), true);

        // Same line, different decoration hash — should miss.
        assert!(cache.get(&frame.key(0, 43)).is_none());
        // Same line, same hash — should hit.
        assert!(cache.get(&frame.key(0, 42)).is_some());
    }

    /// The panes of a split render one after the other every frame; each
    /// pane's lines used to be thrown away by the next pane's `begin_frame`.
    #[test]
    fn split_panes_keep_their_lines_across_frames() {
        let mut cache = LineRenderCache::new();
        let left = test_frame(1, 5);
        let right = LineCacheFrame {
            text_width: 40,
            ..test_frame(2, 9)
        };
        for _ in 0..3 {
            for pane in [left, right] {
                cache.begin_frame(pane);
                for line in 0..20 {
                    if cache.get(&pane.key(line, 0)).is_none() {
                        cache.put(pane.key(line, 0), make_line("x"), true);
                    }
                }
            }
        }
        // Only the first frame of each pane missed; the next two were all hits.
        cache.begin_frame(left);
        for line in 0..20 {
            assert!(cache.get(&left.key(line, 0)).is_some());
        }
        assert_eq!((cache.hits, cache.misses), (20, 0));
        cache.begin_frame(right);
        assert!(cache.get(&right.key(0, 0)).is_some());
        // A pane's lines are not visible to the other pane's frame.
        assert!(cache.get(&left.key(0, 0)).is_none());
    }

    #[test]
    fn old_frames_are_dropped_least_recently_used_first() {
        let mut cache = LineRenderCache::new();
        let pinned = test_frame(1, 0);
        cache.begin_frame(pinned);
        cache.put(pinned.key(0, 0), make_line("kept"), true);
        // Typing: a new version every frame, with the pane still rendered.
        for version in 1..=(MAX_FRAMES * 2) {
            cache.begin_frame(pinned);
            let edited = test_frame(2, version);
            cache.begin_frame(edited);
            cache.put(edited.key(0, 0), make_line("x"), true);
        }
        assert!(cache.frames.len() <= MAX_FRAMES);
        cache.begin_frame(pinned);
        assert!(
            cache.get(&pinned.key(0, 0)).is_some(),
            "a frame in use survives the churn"
        );
        let first_edit = test_frame(2, 1);
        cache.begin_frame(first_edit);
        assert!(
            cache.get(&first_edit.key(0, 0)).is_none(),
            "stale frames go"
        );
    }

    #[test]
    fn stats_reset() {
        let mut cache = LineRenderCache::new();
        cache.hits = 10;
        cache.misses = 5;
        cache.reset_stats();
        assert_eq!(cache.hits, 0);
        assert_eq!(cache.misses, 0);
    }
}
