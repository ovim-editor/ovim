//! A bounded ring buffer of recent edit groups keyed by post-edit buffer version.
//!
//! `EditLog` is a projection-foundation for phase-05: it lets features that
//! anchor decoration positions to a specific buffer version recover the exact
//! mutation history between that version and the current one.
//!
//! # Semantics
//!
//! Every call to `Buffer::record()` that captures a non-empty edit list pushes
//! a single entry `(post_version, edits)` — where `post_version` is the
//! buffer's `version` *after* the edits were applied. Sequential pushes share
//! no ordering assumption beyond `post_version` being monotonically increasing.
//!
//! When a caller asks for `edits_since(base_version)`, the log returns every
//! edit that bumped the buffer version past `base_version`. If `base_version`
//! is older than the oldest retained entry, the log returns `None`: the caller
//! must treat this as "history lost — invalidate derived state".
//!
//! # Capacity
//!
//! The ring holds the 64 most recent groups. Under normal interactive editing
//! this is far more than a single LSP debounce window. Large-scale mutations
//! (replace_all, refactors) call `clear()` explicitly because their edits are
//! not safely replayable against pre-mutation positions.

use crate::edit::Edit;
use std::collections::VecDeque;

/// Default capacity of the ring buffer. 64 entries is comfortably larger than
/// any single LSP debounce/refresh window under interactive editing.
pub const DEFAULT_CAPACITY: usize = 64;

/// A ring buffer of `(post_version, edits)` entries.
#[derive(Debug, Clone)]
pub struct EditLog {
    entries: VecDeque<(u64, Vec<Edit>)>,
    capacity: usize,
}

impl EditLog {
    /// Creates a new `EditLog` with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Creates a new `EditLog` with a custom capacity. `capacity` must be
    /// non-zero; we treat zero as 1 to keep the structure valid.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
        }
    }

    /// Appends a new entry. `post_version` is the buffer version *after* the
    /// edits were applied. When the ring is full the oldest entry is dropped.
    ///
    /// Empty edit lists are ignored: they carry no information and would only
    /// burn ring capacity.
    pub fn push(&mut self, post_version: u64, edits: Vec<Edit>) {
        if edits.is_empty() {
            return;
        }
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((post_version, edits));
    }

    /// Returns every edit that bumped the buffer version past `base_version`,
    /// in application order. Returns `None` if `base_version` is older than
    /// the oldest retained entry — the history has been evicted and the caller
    /// must fall back to a full-refresh path.
    ///
    /// A `base_version` equal to or newer than the most recent post_version
    /// returns `Some(empty)` — no edits have happened since.
    pub fn edits_since(&self, base_version: u64) -> Option<Vec<&Edit>> {
        // If the ring is empty, nothing has been logged yet. Any caller asking
        // for edits since version N is vacuously up-to-date.
        let Some(oldest) = self.entries.front() else {
            return Some(Vec::new());
        };

        // The oldest retained entry has post_version = oldest.0. Its edits
        // took the buffer *to* that version. So edits_since(v) is recoverable
        // when v >= (oldest.0 - edits_in_oldest_entry_count). But we don't
        // know the pre-version of the oldest group without tracking it.
        //
        // The simple, sound check: the caller's base_version must be >= the
        // pre-version of the oldest retained group. We don't retain pre-versions
        // explicitly, so we approximate: if base_version < oldest.0 - 1 we have
        // definitely lost history (the gap is larger than this group). If
        // base_version == oldest.0, the caller is already at or past the oldest
        // post_version, so the slice from "after oldest" is complete.
        //
        // Precise semantics: we return Some(edits) when the requested
        // base_version >= the pre-version of the oldest entry. Since every
        // push() captures one group's worth of edits ending at post_version,
        // any base_version strictly less than (oldest.0 - oldest_group_len)
        // is definitely lost. But we don't need that level of precision — the
        // ring is conservative. For the Step B shape we use a coarser rule:
        // the history is recoverable iff `base_version` is not strictly less
        // than the smallest post_version retained minus that group's own
        // length. Equivalently: the oldest group's pre-version.
        //
        // For simplicity right now: we retain enough headroom that this
        // precise boundary rarely matters. The conservative rule is: if
        // base_version < oldest.0, we MAY have lost the group that produced
        // oldest — so return None unless we can prove otherwise.
        //
        // Proof-of-not-lost: the oldest group's pre-version is
        // `oldest.0 - 1` if the group contained exactly one edit that bumped
        // the version by 1. Each edit inside a `record()` session bumps
        // `buffer.version` independently (insert_text_at / delete_range each
        // increment version). So the pre-version is
        // `oldest.0 - oldest.1.len() as u64`.
        let oldest_post = oldest.0;
        let oldest_group_len = oldest.1.len() as u64;
        let oldest_pre = oldest_post.saturating_sub(oldest_group_len);

        if base_version < oldest_pre {
            return None;
        }

        let mut out = Vec::new();
        for (post_version, edits) in &self.entries {
            if *post_version > base_version {
                for edit in edits {
                    out.push(edit);
                }
            }
        }
        Some(out)
    }

    /// Removes all entries. Use when the projection has become unsound
    /// (e.g., full buffer replacement) and nothing should attempt to replay.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of entries currently in the ring.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the ring is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Capacity of the ring.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the most recent post_version if any.
    pub fn latest_version(&self) -> Option<u64> {
        self.entries.back().map(|(v, _)| *v)
    }
}

impl Default for EditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn push_and_len() {
        let mut log = EditLog::new();
        assert_eq!(log.len(), 0);
        assert!(log.is_empty());

        log.push(1, vec![ins(0, "a")]);
        assert_eq!(log.len(), 1);
        assert!(!log.is_empty());
    }

    #[test]
    fn push_ignores_empty_edits() {
        let mut log = EditLog::new();
        log.push(1, vec![]);
        assert_eq!(log.len(), 0, "empty edits must not consume ring capacity");
    }

    #[test]
    fn ring_overflow_drops_oldest() {
        let mut log = EditLog::with_capacity(3);
        log.push(1, vec![ins(0, "a")]);
        log.push(2, vec![ins(1, "b")]);
        log.push(3, vec![ins(2, "c")]);
        assert_eq!(log.len(), 3);

        log.push(4, vec![ins(3, "d")]);
        assert_eq!(log.len(), 3, "ring must cap at capacity");
        assert_eq!(log.entries.front().unwrap().0, 2, "oldest evicted");
        assert_eq!(log.entries.back().unwrap().0, 4);
    }

    #[test]
    fn edits_since_returns_newer_only() {
        let mut log = EditLog::with_capacity(8);
        log.push(1, vec![ins(0, "a")]);
        log.push(2, vec![ins(1, "b")]);
        log.push(3, vec![del(0, "a")]);

        // base_version=1: return edits from post_versions strictly greater than 1
        let edits = log.edits_since(1).expect("history available");
        assert_eq!(edits.len(), 2);
        assert_eq!(*edits[0], ins(1, "b"));
        assert_eq!(*edits[1], del(0, "a"));
    }

    #[test]
    fn edits_since_at_latest_returns_empty() {
        let mut log = EditLog::new();
        log.push(1, vec![ins(0, "a")]);
        log.push(2, vec![ins(1, "b")]);

        let edits = log.edits_since(2).expect("history available");
        assert!(
            edits.is_empty(),
            "no edits happened since the latest post_version"
        );
    }

    #[test]
    fn edits_since_on_empty_log_is_some_empty() {
        let log = EditLog::new();
        let edits = log.edits_since(0).expect("empty log is vacuously current");
        assert!(edits.is_empty());
    }

    #[test]
    fn edits_since_expired_returns_none() {
        let mut log = EditLog::with_capacity(2);
        // Pre-version of the oldest group after eviction will be 2 (single
        // edit bumped version from 2 → 3). Any base_version < 2 must be lost.
        log.push(1, vec![ins(0, "a")]);
        log.push(2, vec![ins(1, "b")]);
        log.push(3, vec![ins(2, "c")]);
        log.push(4, vec![ins(3, "d")]);
        // Ring now has (3, [c]) and (4, [d]); oldest pre-version is 3-1 = 2.

        // base_version=1 is older than pre-version of oldest retained group
        assert!(
            log.edits_since(1).is_none(),
            "history for v=1 has been evicted"
        );
        // base_version=2 is exactly the oldest pre-version → recoverable
        assert_eq!(
            log.edits_since(2).expect("recoverable").len(),
            2,
            "two edits since v=2"
        );
        // base_version=3 → only the single edit that took us to v=4
        assert_eq!(log.edits_since(3).expect("recoverable").len(), 1);
        // base_version=4 → nothing
        assert!(log.edits_since(4).expect("recoverable").is_empty());
    }

    #[test]
    fn clear_empties_log() {
        let mut log = EditLog::new();
        log.push(1, vec![ins(0, "a")]);
        log.push(2, vec![ins(1, "b")]);
        log.clear();
        assert!(log.is_empty());
        // After clear, any base_version is vacuously current.
        assert!(log.edits_since(0).unwrap().is_empty());
    }

    #[test]
    fn latest_version_tracks_back() {
        let mut log = EditLog::new();
        assert_eq!(log.latest_version(), None);
        log.push(5, vec![ins(0, "a")]);
        assert_eq!(log.latest_version(), Some(5));
        log.push(7, vec![ins(0, "b")]);
        assert_eq!(log.latest_version(), Some(7));
    }

    #[test]
    fn group_with_multiple_edits_single_entry() {
        let mut log = EditLog::new();
        // One record() session with two mutations: version advances by 2 to
        // reach post_version = 3.
        log.push(3, vec![del(0, "a"), ins(0, "b")]);

        let edits = log.edits_since(1).expect("recoverable");
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn preserves_insertion_order_across_groups() {
        let mut log = EditLog::new();
        log.push(1, vec![ins(0, "a")]);
        log.push(2, vec![ins(1, "b")]);
        log.push(3, vec![ins(2, "c")]);

        let edits = log.edits_since(0).expect("recoverable");
        assert_eq!(*edits[0], ins(0, "a"));
        assert_eq!(*edits[1], ins(1, "b"));
        assert_eq!(*edits[2], ins(2, "c"));
    }

    #[test]
    fn with_capacity_clamps_zero_to_one() {
        let mut log = EditLog::with_capacity(0);
        assert_eq!(log.capacity(), 1);
        log.push(1, vec![ins(0, "a")]);
        log.push(2, vec![ins(1, "b")]);
        assert_eq!(log.len(), 1, "capacity=1 retains only one entry");
    }

    #[test]
    fn default_capacity_is_64() {
        let log = EditLog::new();
        assert_eq!(log.capacity(), DEFAULT_CAPACITY);
        assert_eq!(DEFAULT_CAPACITY, 64);
    }
}
