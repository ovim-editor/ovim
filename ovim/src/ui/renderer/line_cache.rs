//! Per-line rendering cache.
//!
//! Caches the expensive per-line rendering output (tab expansion, horizontal
//! viewport slicing, highlight computation) to avoid recomputation when only
//! the cursor moves. The cache is invalidated when the buffer content changes,
//! the viewport shifts, or the window resizes.

use ratatui::text::Line;
use std::collections::HashMap;

/// Key identifying a cached rendered line.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LineCacheKey {
    /// Buffer identity (current buffer index)
    buffer_id: usize,
    /// Logical line index in the buffer
    line_idx: usize,
    /// Buffer version when this line was rendered
    buffer_version: usize,
    /// Horizontal scroll offset (display columns)
    h_offset: usize,
    /// Available text width (columns)
    text_width: usize,
    /// Whether wrap mode was enabled
    wrap: bool,
    /// Tab width setting
    tab_width: usize,
    /// Whether markdown conceal was active for this render
    markdown_conceal: bool,
    /// Decoration generation — decorations (inlay hints, diagnostic vtext)
    /// are now included in the cached line, so the cache must invalidate
    /// when they change.
    decoration_generation: u64,
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

/// Per-line rendering cache that avoids recomputing expensive rendering
/// for unchanged lines.
///
/// # Invalidation strategy
///
/// - **Buffer edit**: The entire cache is cleared when `buffer_version` changes.
///   Fine-grained per-line invalidation would require tracking which lines
///   shifted, which isn't worth the complexity for a first pass.
/// - **Scroll/resize**: Cleared when `h_offset` or `text_width` changes
///   (detected via the key including these values).
/// - **Cursor move**: Only the cursor line and previous cursor line are
///   excluded from caching (they have transient cursorline highlighting).
/// - **Visual selection / search / yank flash**: Lines with these overlays
///   are rendered fresh each frame (marked `is_stable: false`).
pub struct LineRenderCache {
    entries: HashMap<usize, (LineCacheKey, CachedLine)>,
    /// Buffer version from the last render pass
    last_buffer_version: usize,
    /// Capacity limit to prevent unbounded growth
    max_entries: usize,
    /// Stats: cache hits this frame
    pub hits: usize,
    /// Stats: cache misses this frame
    pub misses: usize,
}

impl Default for LineRenderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl LineRenderCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::with_capacity(256),
            last_buffer_version: usize::MAX, // force miss on first frame
            max_entries: 1024,
            hits: 0,
            misses: 0,
        }
    }

    /// Clear the entire cache (e.g., on buffer edit or resize).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Reset per-frame stats.
    pub fn reset_stats(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }

    /// Check if a rendered line is cached and still valid.
    ///
    /// Returns `None` if:
    /// - The line was never cached
    /// - The buffer version changed since it was cached
    /// - The viewport parameters changed
    /// - The cached entry had transient highlighting
    pub fn get(
        &mut self,
        buffer_id: usize,
        line_idx: usize,
        buffer_version: usize,
        h_offset: usize,
        text_width: usize,
        wrap: bool,
        tab_width: usize,
        markdown_conceal: bool,
        decoration_generation: u64,
    ) -> Option<&Line<'static>> {
        // Fast path: if buffer version changed, invalidate everything
        if buffer_version != self.last_buffer_version {
            self.clear();
            self.last_buffer_version = buffer_version;
            self.misses += 1;
            return None;
        }

        let key = LineCacheKey {
            buffer_id,
            line_idx,
            buffer_version,
            h_offset,
            text_width,
            wrap,
            tab_width,
            markdown_conceal,
            decoration_generation,
        };

        if let Some((cached_key, cached)) = self.entries.get(&line_idx) {
            if *cached_key == key && cached.is_stable {
                self.hits += 1;
                return Some(&cached.line);
            }
        }
        self.misses += 1;
        None
    }

    /// Store a rendered line in the cache.
    ///
    /// `is_stable` should be `false` for lines with transient overlays
    /// (cursor line, visual selection, search highlights, yank flash).
    pub fn put(
        &mut self,
        buffer_id: usize,
        line_idx: usize,
        buffer_version: usize,
        h_offset: usize,
        text_width: usize,
        wrap: bool,
        tab_width: usize,
        markdown_conceal: bool,
        decoration_generation: u64,
        line: Line<'static>,
        is_stable: bool,
    ) {
        // Evict if over capacity — keep entries near the current viewport
        // instead of clearing everything (which causes a full cache-miss storm
        // on the next frame).
        if self.entries.len() >= self.max_entries {
            let center = line_idx;
            let keep_radius = self.max_entries / 2;
            let lo = center.saturating_sub(keep_radius);
            let hi = center.saturating_add(keep_radius);
            self.entries.retain(|&idx, _| idx >= lo && idx <= hi);
        }

        let key = LineCacheKey {
            buffer_id,
            line_idx,
            buffer_version,
            h_offset,
            text_width,
            wrap,
            tab_width,
            markdown_conceal,
            decoration_generation,
        };
        self.entries
            .insert(line_idx, (key, CachedLine { line, is_stable }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    fn make_line(text: &str) -> Line<'static> {
        Line::from(vec![Span::raw(text.to_string())])
    }

    #[test]
    fn cache_hit() {
        let mut cache = LineRenderCache::new();
        cache.last_buffer_version = 1; // sync version
        cache.put(1, 0, 1, 0, 80, false, 4, false, 0, make_line("hello"), true);

        let result = cache.get(1, 0, 1, 0, 80, false, 4, false, 0);
        assert!(result.is_some());
        assert_eq!(cache.hits, 1);
        assert_eq!(cache.misses, 0);
    }

    #[test]
    fn cache_miss_version_change() {
        let mut cache = LineRenderCache::new();
        cache.last_buffer_version = 1;
        cache.put(1, 0, 1, 0, 80, false, 4, false, 0, make_line("hello"), true);

        // Buffer version changed
        let result = cache.get(1, 0, 2, 0, 80, false, 4, false, 0);
        assert!(result.is_none());
        assert_eq!(cache.misses, 1);
    }

    #[test]
    fn cache_miss_viewport_change() {
        let mut cache = LineRenderCache::new();
        cache.last_buffer_version = 1;
        cache.put(1, 0, 1, 0, 80, false, 4, false, 0, make_line("hello"), true);

        // h_offset changed
        let result = cache.get(1, 0, 1, 5, 80, false, 4, false, 0);
        assert!(result.is_none());
    }

    #[test]
    fn unstable_lines_not_cached() {
        let mut cache = LineRenderCache::new();
        cache.last_buffer_version = 1;
        // Store with is_stable=false (e.g., cursor line)
        cache.put(
            1,
            0,
            1,
            0,
            80,
            false,
            4,
            false,
            0,
            make_line("cursor"),
            false,
        );

        let result = cache.get(1, 0, 1, 0, 80, false, 4, false, 0);
        assert!(result.is_none()); // Should not hit
    }

    #[test]
    fn eviction_keeps_nearby_lines() {
        let mut cache = LineRenderCache::new();
        cache.max_entries = 10; // small cap for test
        cache.last_buffer_version = 1;

        // Fill cache with lines 0..10
        for i in 0..10 {
            cache.put(1, i, 1, 0, 80, false, 4, false, 0, make_line("x"), true);
        }
        assert_eq!(cache.entries.len(), 10);

        // Insert line 8 — should evict lines far from 8 (keep 3..13)
        cache.put(1, 8, 1, 0, 80, false, 4, false, 0, make_line("new"), true);

        // Lines near 8 should survive, line 0 should be evicted
        assert!(cache.entries.contains_key(&8));
        assert!(cache.entries.contains_key(&5));
        assert!(!cache.entries.contains_key(&0));
        assert!(!cache.entries.contains_key(&1));
        assert!(!cache.entries.contains_key(&2));
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
