//! Branch diff review.
//!
//! `<Space>gd` (or `:GitDiff`) opens a read-only patch of everything the
//! current branch changed relative to the default branch, including
//! uncommitted work. Each hunk is coloured with the grammar of the file it
//! came from — the way `delta` renders a patch — rather than with a diff
//! grammar, so the review reads like code. Inside the review:
//!
//! - `s` (or a click on the toolbar) switches between the unified and the
//!   side-by-side layout
//! - `]c` / `[c` move between hunks, `]f` / `[f` between files
//! - `Enter` (or `gf`) opens the file at the line under the cursor in the tab
//!   the review was opened from; `<Space>gd` returns to the review, refreshed
//! - `w` hides equal paired changes in curated reviews, ignoring whitespace
//! - `o` toggles saved moves, `O` opens their frozen snapshot
//! - `r` refreshes, `q` closes, `<Space>gf` fetches the base branch
//!
//! The base is chosen by [`crate::native_diff::resolve_base`], which prefers
//! whichever of `main` / `origin/main` has the most recent merge-base with
//! HEAD so a stale local or remote copy never pollutes the review.

mod checks;
mod highlight;
mod render;

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use super::{Editor, ToastLevel, ToastRequest, ToastSource};
use crate::buffer::{Buffer, BufferId};
use crate::native_diff::{self, CustomReview, PatchLineKind, ReviewBase, ReviewPatch};
use crate::syntax::HighlightGroup;
use crate::unicode::{grapheme_index_for_byte, GraphemeCol};

pub use checks::ReviewChecks;
pub use render::{DiffLayout, DIFF_REVIEW_TITLE_PREFIX};

use render::{
    layout_body, summary_message, Rendered, ReviewCell, ReviewRow, Toolbar, DEFAULT_LAYOUT_WIDTH,
};

/// State of the open branch review, if any.
pub struct DiffReviewState {
    item_rows: Vec<Option<String>>,
    pub checked_items: std::collections::BTreeSet<String>,
    pub show_checked: bool,
    /// The read-only buffer showing the patch.
    pub buffer_id: BufferId,
    /// Buffer that was current when the review was (re-)entered. `Enter`
    /// opens files in the tab showing this buffer.
    pub origin_buffer_id: Option<BufferId>,
    /// Tab index the review was (re-)entered from; fallback when
    /// `origin_buffer_id` is no longer shown in any tab.
    pub origin_tab: usize,
    /// User-supplied comparison (`:GitDiff <spec>`); None uses pullbase or auto.
    pub explicit_spec: Option<String>,
    pub layout: DiffLayout,
    /// The patch the buffer was rendered from, kept so switching layout or
    /// re-flowing after a resize costs no Git work.
    patch: ReviewPatch,
    /// A saved reassignment of the patch's changes. Its sections are never
    /// regenerated from the worktree when the review is revisited.
    custom: Option<CustomReview>,
    custom_title: Option<String>,
    context_snapshot: native_diff::ReviewSnapshot,
    context: native_diff::context::DiffContext,
    context_rows: Vec<Option<String>>,
    /// True when the custom view is attached to the current live Git patch.
    live_overlay: bool,
    /// Text width the side-by-side layout was laid out for.
    layout_width: usize,
    /// Buffer-area width that produced `layout_width`. Re-flowing keys off
    /// this rather than off the text width, which also moves when the line
    /// number gutter grows — that would oscillate.
    layout_area_width: usize,
    /// Source mapping per buffer line.
    rows: Vec<ReviewRow>,
    /// `(buffer line, file index)` for each row of the file summary.
    stat_rows: Vec<(usize, usize)>,
    /// Buffer lines of hunk headers.
    hunk_lines: Vec<usize>,
    /// Buffer line of the first header line of each file, indexed by file.
    file_lines: Vec<usize>,
    /// The same lines in buffer order, for `]f` / `[f`.
    file_nav: Vec<usize>,
    /// The clickable layout switch.
    toolbar: Toolbar,
    /// Buffer text split into lines (for refresh anchoring).
    text_lines: Vec<String>,
    /// Byte range of each patch line inside `patch.text`, for mapping a
    /// cursor column back to a column in the source file. Ranges rather than
    /// owned lines: a review can hold a hundred thousand of them.
    patch_line_ranges: Vec<(usize, usize)>,
    code_highlights: Vec<Vec<(Range<usize>, HighlightGroup)>>,
    custom_targets: Vec<Option<render::CustomTarget>>,
}

impl DiffReviewState {
    /// The source patch retained by the review, independent of terminal layout.
    pub fn patch(&self) -> &ReviewPatch {
        &self.patch
    }

    pub fn context_view(&self, id: &str) -> Option<native_diff::context::DiffContextView> {
        self.context.view(&self.context_snapshot, id)
    }

    pub fn custom(&self) -> Option<&CustomReview> {
        self.custom.as_ref()
    }

    /// First rendered row for each patch line, including both sides of a split.
    pub fn patch_review_lines(&self) -> Vec<Option<usize>> {
        let mut result = vec![None; self.patch.lines.len()];
        for (row_index, row) in self.rows.iter().enumerate() {
            for cell in [row.left, row.right].into_iter().flatten() {
                if let Some(line) = result.get_mut(cell.patch_line) {
                    line.get_or_insert(row_index);
                }
            }
        }
        result
    }

    pub fn patch_line_highlights(&self, patch_line: usize) -> &[(Range<usize>, HighlightGroup)] {
        self.code_highlights
            .get(patch_line)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn root(&self) -> &Path {
        &self.patch.root
    }

    pub fn base(&self) -> &ReviewBase {
        &self.patch.base
    }

    fn row(&self, line: usize) -> Option<&ReviewRow> {
        self.rows.get(line)
    }

    fn file_for_stat_row(&self, line: usize) -> Option<usize> {
        self.stat_rows
            .iter()
            .find(|(row, _)| *row == line)
            .map(|(_, file)| *file)
    }

    fn info(&self, line: usize) -> Option<crate::native_diff::PatchLine> {
        self.row(line).and_then(ReviewRow::info)
    }

    /// Best `(file index, 1-based line)` source target for a buffer line.
    fn source_target(&self, line: usize) -> Option<(usize, usize)> {
        self.cell_target(line, self.info(line)?)
    }

    fn cell_target(
        &self,
        line: usize,
        info: crate::native_diff::PatchLine,
    ) -> Option<(usize, usize)> {
        let file = info.file?;
        if let Some(new_line) = info.new_line {
            return Some((file, new_line));
        }
        // File headers and meta lines: use the first hunk that follows.
        let next_hunk = self
            .rows
            .iter()
            .skip(line + 1)
            .map_while(ReviewRow::info)
            .find(|entry| entry.file == Some(file) && entry.kind == PatchLineKind::HunkHeader)
            .and_then(|entry| entry.new_line);
        Some((file, next_hunk.unwrap_or(1)))
    }

    /// Where the cursor is, in source terms, so a re-render can restore it.
    fn anchor(&self, line: usize, text: Option<String>) -> Option<Anchor> {
        let target = self
            .source_target(line)
            .or_else(|| self.file_for_stat_row(line).map(|file| (file, 1)))?;
        let path = self.patch.files.get(target.0)?.path.clone();
        Some(Anchor {
            path,
            new_line: target.1,
            text: text.filter(|_| self.info(line).is_some()),
        })
    }

    /// Buffer line to restore after a re-render: the same patch line if its
    /// text still exists in that file (nearest to the old position), else the
    /// first line at or after the old source line, else the file header.
    fn line_for_anchor(&self, anchor: &Anchor) -> Option<usize> {
        let file = self
            .patch
            .files
            .iter()
            .position(|entry| entry.path == anchor.path)?;
        let candidates = self.rows.iter().enumerate().filter_map(|(line, row)| {
            let entry = row.info()?;
            (entry.file == Some(file) && entry.kind != PatchLineKind::FileHeader)
                .then_some((line, entry.new_line?))
        });

        if let Some(text) = &anchor.text {
            let same_text = candidates
                .clone()
                .filter(|(line, _)| self.line_text(*line) == Some(text.as_str()))
                .min_by_key(|(_, new_line)| new_line.abs_diff(anchor.new_line));
            if let Some((line, _)) = same_text {
                return Some(line);
            }
        }

        candidates
            .filter(|(_, new_line)| *new_line >= anchor.new_line)
            .min_by_key(|(_, new_line)| *new_line)
            .map(|(line, _)| line)
            .or_else(|| {
                self.file_lines
                    .get(file)
                    .copied()
                    .filter(|line| *line != usize::MAX)
            })
    }

    fn line_text(&self, line: usize) -> Option<&str> {
        self.text_lines.get(line).map(String::as_str)
    }

    fn custom_target_at(&self, line: usize, col: usize) -> Option<(String, usize, &'static str)> {
        let target = self.custom_targets.get(line)?.as_ref()?;
        let row = self.row(line)?;
        let right_selected = self.layout == DiffLayout::Split
            && row
                .right
                .is_some_and(|cell| col >= cell.text_col.saturating_sub(2));
        if right_selected {
            target
                .new
                .clone()
                .map(|(path, line)| (path, line, "new"))
                .or_else(|| target.old.clone().map(|(path, line)| (path, line, "old")))
        } else if self.layout == DiffLayout::Unified
            && row
                .left
                .is_some_and(|cell| cell.info.kind == PatchLineKind::Added)
        {
            target.new.clone().map(|(path, line)| (path, line, "new"))
        } else {
            target
                .old
                .clone()
                .map(|(path, line)| (path, line, "old"))
                .or_else(|| target.new.clone().map(|(path, line)| (path, line, "new")))
        }
    }

    /// The `(byte range, added)` tints for a rendered line, so the frontend
    /// can paint the added/removed background the way `delta` does.
    pub fn line_tints(&self, line: usize) -> Vec<(std::ops::Range<usize>, bool)> {
        self.row(line)
            .map(|row| row.tints().collect())
            .unwrap_or_default()
    }

    /// Whether a rendered line's tint runs all the way to its end, so the
    /// frontend can carry the band through the padding to the right edge.
    /// `Some(true)` is an addition, `Some(false)` a removal.
    pub fn line_trailing_tint(&self, line: usize) -> Option<bool> {
        let length = self.line_text(line)?.len();
        if length == 0 {
            return None;
        }
        self.row(line)?
            .tints()
            .find(|(range, _)| range.end >= length)
            .map(|(_, added)| added)
    }

    /// The column in the source file a cursor at `col` refers to.
    fn source_column(&self, cell: &ReviewCell, col: usize, tab_width: usize) -> usize {
        if cell.text_len == 0 {
            return 0;
        }
        let Some(body) = self.patch_body(cell.patch_line) else {
            return 0;
        };
        let glyphs = layout_body(body, tab_width, self.layout == DiffLayout::Split);
        if glyphs.is_empty() {
            return 0;
        }
        let offset = col.saturating_sub(cell.text_col).min(cell.text_len - 1);
        let index = (cell.src_glyph + offset).min(glyphs.len() - 1);
        grapheme_index_for_byte(body, glyphs[index].src_byte)
    }

    /// The text of a patch line with its `+`/`-`/space marker stripped.
    fn patch_body(&self, patch_line: usize) -> Option<&str> {
        let (start, end) = *self.patch_line_ranges.get(patch_line)?;
        let text = self.patch.text.get(start..end)?;
        match self.patch.lines.get(patch_line)?.kind {
            PatchLineKind::Added | PatchLineKind::Removed | PatchLineKind::Context => text.get(1..),
            _ => Some(text),
        }
    }
}

/// Source position of the cursor, captured before a re-render.
struct Anchor {
    path: String,
    new_line: usize,
    /// Text of the patch line under the cursor, when it was a patch line.
    text: Option<String>,
}

/// A background `git fetch` started by `<Space>gf`.
pub struct PendingGitFetch {
    receiver: Receiver<Result<(), String>>,
    target: String,
}

/// Most recent agent arrangement for one worktree in this editor session.
pub struct SavedDiffOverlay {
    title: String,
    review: CustomReview,
    enabled: bool,
    requires_proof: bool,
}

pub struct DiffOverlayViewState {
    pub mode: &'static str,
    pub title: Option<String>,
}

impl SavedDiffOverlay {
    fn matches(&self, live: &native_diff::ReviewSnapshot) -> bool {
        if self.requires_proof || self.review.snapshot.content_fingerprint.is_some() {
            native_diff::store::same_comparison(&self.review.snapshot, live)
        } else {
            // Explicitly opened legacy reviews keep their in-session behavior;
            // loading from disk always requires the stronger comparison proof.
            native_diff::store::same_patch(&self.review.snapshot.patch, &live.patch)
        }
    }
}

impl Editor {
    /// Frontend opt-in follows other durable editor state: bare editors stay isolated.
    pub fn enable_diff_review_persistence(&mut self) {
        match native_diff::store::ReviewStore::discover() {
            Ok(store) => self.set_diff_review_store(Some(store)),
            Err(error) => {
                crate::log_warn!("diff", "Diff review persistence unavailable: {error:#}")
            }
        }
    }

    pub fn set_diff_review_store(&mut self, store: Option<native_diff::store::ReviewStore>) {
        self.ui_panels.diff_review_store = store;
    }

    fn restore_diff_overlay(&mut self, snapshot: &native_diff::ReviewSnapshot) {
        let Some(store) = &self.ui_panels.diff_review_store else {
            return;
        };
        match store.load(snapshot) {
            Ok(Some((title, review))) => {
                let previous = self
                    .ui_panels
                    .diff_review_overlays
                    .get(&snapshot.patch.root);
                let enabled = previous
                    .filter(|saved| saved.review == review && saved.title == title)
                    .is_none_or(|saved| saved.enabled);
                self.ui_panels.diff_review_overlays.insert(
                    snapshot.patch.root.clone(),
                    SavedDiffOverlay {
                        title,
                        review,
                        enabled,
                        requires_proof: true,
                    },
                );
            }
            Ok(None) => {}
            Err(error) => self.review_toast(
                ToastLevel::Warning,
                format!("Could not restore saved diff: {error:#}"),
            ),
        }
    }

    pub fn diff_review(&self) -> Option<&DiffReviewState> {
        self.ui_panels.diff_review.as_deref()
    }

    pub fn diff_review_overlay_state(&self) -> Option<DiffOverlayViewState> {
        let state = self.ui_panels.diff_review.as_ref()?;
        let cached = self.ui_panels.diff_review_overlays.get(&state.patch.root);
        if state.custom.is_some() && !state.live_overlay {
            return Some(DiffOverlayViewState {
                mode: "saved",
                title: state.custom_title.clone(),
            });
        }
        let mode = match cached {
            Some(saved) if !saved.matches(&state.context_snapshot) => "stale",
            Some(saved) if saved.enabled => "active",
            Some(_) => "available",
            None => "none",
        };
        Some(DiffOverlayViewState {
            mode,
            title: cached.map(|saved| saved.title.clone()),
        })
    }

    fn active_overlay_for_patch(
        &self,
        snapshot: &native_diff::ReviewSnapshot,
    ) -> Option<(String, CustomReview)> {
        self.ui_panels
            .diff_review_overlays
            .get(&snapshot.patch.root)
            .filter(|saved| saved.enabled && saved.matches(snapshot))
            .map(|saved| (saved.title.clone(), saved.review.clone()))
    }

    fn live_review_status(&self, snapshot: &native_diff::ReviewSnapshot) -> String {
        match self
            .ui_panels
            .diff_review_overlays
            .get(&snapshot.patch.root)
        {
            Some(saved) if !saved.matches(snapshot) => {
                "Saved moves unverified · press O to view the frozen diff".to_string()
            }
            Some(saved) if saved.enabled => {
                "Saved moves applied · press o to remove overlay".to_string()
            }
            Some(_) => "Saved moves available · press o to apply overlay".to_string(),
            None => summary_message(&snapshot.patch),
        }
    }

    pub fn return_to_live_diff_review(&mut self) -> anyhow::Result<()> {
        self.open_diff_review(None)
    }

    pub fn toggle_diff_review_overlay(&mut self) -> anyhow::Result<()> {
        let Some(state) = self.ui_panels.diff_review.as_ref() else {
            anyhow::bail!("No diff review is open");
        };
        if state.custom.is_some() && !state.live_overlay {
            return self.return_to_live_diff_review();
        }
        let root = state.patch.root.clone();
        if let Some(saved) = self.ui_panels.diff_review_overlays.get_mut(&root) {
            let previous = saved.enabled;
            saved.enabled = !saved.enabled;
            if let Err(error) = self.refresh_live_diff_review() {
                self.ui_panels
                    .diff_review_overlays
                    .get_mut(&root)
                    .expect("saved overlay remains cached")
                    .enabled = previous;
                return Err(error);
            }
            Ok(())
        } else {
            anyhow::bail!("No saved moves for this repository")
        }
    }

    pub fn open_saved_diff_overlay(&mut self) -> anyhow::Result<()> {
        let root = self
            .ui_panels
            .diff_review
            .as_ref()
            .map(|state| state.patch.root.clone())
            .unwrap_or_else(|| self.diff_review_workspace_hint());
        let saved = self
            .ui_panels
            .diff_review_overlays
            .get(&root)
            .ok_or_else(|| anyhow::anyhow!("No saved moves for this repository"))?;
        let title = saved.title.clone();
        let review = saved.review.clone();
        self.open_custom_diff_review_impl(&title, review, false)
    }

    /// Branch reviews and commit/patch buffers share diff viewport affordances.
    pub fn is_diff_buffer(&self) -> bool {
        self.is_diff_review_buffer() || self.buffer().syntax_language_id() == Some("diff")
    }

    /// True when the current buffer is the branch review.
    pub fn is_diff_review_buffer(&self) -> bool {
        self.ui_panels
            .diff_review
            .as_ref()
            .is_some_and(|state| state.buffer_id == self.buffer().id())
    }

    /// `<Space>gd`: open the review, return to it from a file, or leave it.
    pub fn toggle_diff_review(&mut self) {
        if self.is_diff_review_buffer() {
            self.leave_diff_review();
            return;
        }
        if self.review_buffer_index().is_some() {
            if let Err(error) = self.open_diff_review(None) {
                self.review_toast(ToastLevel::Error, format!("Diff review: {error:#}"));
            }
            return;
        }
        if let Err(error) = self.open_diff_review(None) {
            self.review_toast(ToastLevel::Error, format!("Diff review: {error:#}"));
        }
    }

    /// `:GitDiff [spec]`. Reuses the open review buffer when there is one.
    pub fn open_diff_review(&mut self, spec: Option<&str>) -> anyhow::Result<()> {
        self.open_diff_review_in(None, spec)
    }

    /// [`Self::open_diff_review`] for the repository at `root`. A review shows
    /// one repository, so an open review of another one is closed first.
    pub fn open_diff_review_in(
        &mut self,
        root: Option<&Path>,
        spec: Option<&str>,
    ) -> anyhow::Result<()> {
        let root = root.map(|root| std::fs::canonicalize(root).unwrap_or(root.to_path_buf()));
        if let Some(root) = &root {
            let other_repository = self
                .ui_panels
                .diff_review
                .as_ref()
                .is_some_and(|state| &state.patch.root != root);
            if other_repository {
                self.close_diff_review();
            }
        }
        let custom_root = self
            .ui_panels
            .diff_review
            .as_ref()
            .filter(|state| state.custom.is_some())
            .map(|state| state.patch.root.clone());
        if custom_root.is_some() {
            self.close_diff_review();
        }
        let explicit_spec = spec.map(str::trim).filter(|spec| !spec.is_empty());
        if let Some(state) = self.ui_panels.diff_review.as_mut() {
            state.explicit_spec = explicit_spec.map(str::to_string);
        }
        if self.review_buffer_index().is_some() {
            if !self.is_diff_review_buffer() {
                self.enter_diff_review();
            }
            self.refresh_diff_review();
            return Ok(());
        }

        let root_hint = root
            .or(custom_root)
            .unwrap_or_else(|| self.diff_review_workspace_hint());
        let base = match explicit_spec {
            Some(spec) => ReviewBase::explicit(spec),
            None => self.resolve_review_base_for_path(&root_hint)?,
        };
        let context_snapshot = native_diff::review_display_snapshot(&root_hint, &base)?;
        let patch = context_snapshot.patch.clone();

        let layout = self.ui_panels.diff_review_layout;
        let (area_width, width) = self.diff_review_widths();
        self.restore_diff_overlay(&context_snapshot);
        let overlay = self.active_overlay_for_patch(&context_snapshot);
        let context_snapshot = overlay
            .as_ref()
            .map(|(_, custom)| custom.snapshot.clone())
            .unwrap_or(context_snapshot);
        let rendered = if let Some((title, custom)) = &overlay {
            render::render_custom(
                custom,
                title,
                layout,
                width,
                self.diff_review_tab_width(),
                self.ui_panels.diff_review_hide_equal,
                self.ui_panels.diff_review_hide_notes,
            )
        } else {
            render::render(
                &patch,
                layout,
                width,
                self.unsaved_buffer_count(),
                self.diff_review_tab_width(),
            )
        };

        let origin_buffer_id = Some(self.buffer().id());
        let origin_tab = self.current_tab_index();
        self.open_diff_buffer_in_new_tab(&rendered.title, &rendered.text);
        let buffer_id = self.buffer().id();

        self.ui_panels.diff_review = Some(Box::new(DiffReviewState {
            item_rows: Vec::new(),
            checked_items: Default::default(),
            show_checked: false,
            buffer_id,
            origin_buffer_id,
            origin_tab,
            explicit_spec: explicit_spec.map(str::to_string),
            layout,
            layout_width: width,
            layout_area_width: area_width,
            patch_line_ranges: patch_line_ranges(&patch),
            code_highlights: Vec::new(),
            custom_targets: Vec::new(),
            context: native_diff::context::DiffContext::new(
                &context_snapshot,
                overlay.as_ref().map(|(_, custom)| custom),
            ),
            context_snapshot,
            context_rows: Vec::new(),
            custom: overlay.as_ref().map(|(_, custom)| custom.clone()),
            custom_title: overlay.as_ref().map(|(title, _)| title.clone()),
            live_overlay: overlay.is_some(),
            patch,
            rows: Vec::new(),
            stat_rows: Vec::new(),
            hunk_lines: Vec::new(),
            file_lines: Vec::new(),
            file_nav: Vec::new(),
            toolbar: Toolbar::default(),
            text_lines: Vec::new(),
        }));
        self.apply_rendered_review(rendered);

        let message = self
            .ui_panels
            .diff_review
            .as_ref()
            .map(|state| self.live_review_status(&state.context_snapshot));
        if let Some(message) = message {
            self.set_status_message(message);
        }
        self.mark_dirty();
        Ok(())
    }

    /// Opens a saved, agent-arranged comparison. The original patch remains
    /// available for source syntax and accounting; the sections control what
    /// is shown and stay fixed when the worktree changes.
    pub fn open_custom_diff_review(
        &mut self,
        title: &str,
        custom: CustomReview,
    ) -> anyhow::Result<()> {
        self.open_custom_diff_review_impl(title, custom, true)
    }

    fn open_custom_diff_review_impl(
        &mut self,
        title: &str,
        custom: CustomReview,
        remember: bool,
    ) -> anyhow::Result<()> {
        custom.validate_coverage()?;
        if remember {
            if let Some(store) = &self.ui_panels.diff_review_store {
                if let Err(error) = store.save(title, &custom) {
                    self.review_toast(
                        ToastLevel::Warning,
                        format!("Could not save diff for restart: {error:#}"),
                    );
                }
            }
            self.ui_panels.diff_review_overlays.insert(
                custom.snapshot.patch.root.clone(),
                SavedDiffOverlay {
                    title: title.to_string(),
                    review: custom.clone(),
                    enabled: true,
                    requires_proof: false,
                },
            );
        }
        if self.ui_panels.diff_review.is_some() {
            self.close_diff_review();
        }
        let patch = custom.snapshot.patch.clone();
        let layout = self.ui_panels.diff_review_layout;
        let (area_width, width) = self.diff_review_widths();
        let rendered = render::render_custom(
            &custom,
            title,
            layout,
            width,
            self.diff_review_tab_width(),
            self.ui_panels.diff_review_hide_equal,
            self.ui_panels.diff_review_hide_notes,
        );
        if self.mode() == crate::mode::Mode::AiChat {
            self.close_ai_chat();
            self.set_mode(crate::mode::Mode::Normal);
        }
        let origin_buffer_id = Some(self.buffer().id());
        let origin_tab = self.current_tab_index();
        self.open_diff_buffer_in_new_tab(&rendered.title, &rendered.text);
        let buffer_id = self.buffer().id();
        self.ui_panels.diff_review = Some(Box::new(DiffReviewState {
            item_rows: Vec::new(),
            checked_items: Default::default(),
            show_checked: false,
            buffer_id,
            origin_buffer_id,
            origin_tab,
            explicit_spec: None,
            layout,
            patch_line_ranges: patch_line_ranges(&patch),
            patch,
            context: native_diff::context::DiffContext::new(&custom.snapshot, Some(&custom)),
            context_snapshot: custom.snapshot.clone(),
            context_rows: Vec::new(),
            custom: Some(custom),
            custom_title: Some(title.to_string()),
            live_overlay: false,
            layout_width: width,
            layout_area_width: area_width,
            rows: Vec::new(),
            stat_rows: Vec::new(),
            hunk_lines: Vec::new(),
            file_lines: Vec::new(),
            file_nav: Vec::new(),
            toolbar: Toolbar::default(),
            text_lines: Vec::new(),
            code_highlights: Vec::new(),
            custom_targets: Vec::new(),
        }));
        self.apply_rendered_review(rendered);
        self.mark_dirty();
        Ok(())
    }

    /// Recomputes the patch, keeping the cursor on the same source location.
    pub fn refresh_diff_review(&mut self) {
        let Some(index) = self.review_buffer_index() else {
            return;
        };
        if self
            .ui_panels
            .diff_review
            .as_ref()
            .is_some_and(|state| state.custom.is_some() && !state.live_overlay)
        {
            let anchor = self.diff_review_anchor(index, true);
            self.rerender_diff_review(anchor);
            self.set_status_message("Saved diff review");
            return;
        }
        if let Err(error) = self.refresh_live_diff_review() {
            self.review_toast(ToastLevel::Error, format!("Diff review: {error:#}"));
        }
    }

    /// Compute the entire live comparison before replacing the visible review.
    fn refresh_live_diff_review(&mut self) -> anyhow::Result<()> {
        let index = self
            .review_buffer_index()
            .ok_or_else(|| anyhow::anyhow!("No diff review is open"))?;
        let (root, explicit) = {
            let state = self.ui_panels.diff_review.as_ref().expect("review state");
            (state.patch.root.clone(), state.explicit_spec.clone())
        };

        let base = match explicit.as_deref() {
            Some(spec) => Ok(ReviewBase::explicit(spec)),
            None => self.resolve_review_base_for_path(&root),
        }?;
        let context_snapshot = native_diff::review_display_snapshot(&root, &base)?;
        let patch = context_snapshot.patch.clone();

        self.restore_diff_overlay(&context_snapshot);
        let overlay = self.active_overlay_for_patch(&context_snapshot);
        let context_snapshot = overlay
            .as_ref()
            .map(|(_, custom)| custom.snapshot.clone())
            .unwrap_or(context_snapshot);
        let anchor = self.diff_review_anchor(index, true);
        {
            let state = self.ui_panels.diff_review.as_mut().expect("review state");
            state.patch_line_ranges = patch_line_ranges(&patch);
            state.context = native_diff::context::DiffContext::new(
                &context_snapshot,
                overlay.as_ref().map(|(_, custom)| custom),
            );
            state.context_snapshot = context_snapshot;
            state.patch = patch;
            state.custom = overlay.as_ref().map(|(_, custom)| custom.clone());
            state.custom_title = overlay.as_ref().map(|(title, _)| title.clone());
            state.live_overlay = overlay.is_some();
        }
        self.rerender_diff_review(anchor);

        let message = self
            .ui_panels
            .diff_review
            .as_ref()
            .map(|state| self.live_review_status(&state.context_snapshot));
        if let Some(message) = message {
            self.set_status_message(message);
        }
        Ok(())
    }

    /// Reveal ten captured source lines at the selected region's edge.
    pub fn expand_diff_context(&mut self, id: &str, up: bool) -> bool {
        let Some(index) = self.review_buffer_index() else {
            return false;
        };
        let anchor = self.diff_review_anchor(index, false);
        let cursor_line = self.buffer().cursor().line();
        let cursor_col = self.buffer().cursor().col();
        let source_cell = self
            .ui_panels
            .diff_review
            .as_ref()
            .and_then(|state| state.row(cursor_line))
            .and_then(|row| row.cell_at(cursor_col.0));
        let screen_row = self
            .buffer()
            .cursor()
            .line()
            .saturating_sub(self.scroll_offset());
        let state = self.ui_panels.diff_review.as_mut().expect("review state");
        if !state.context.expand(&state.context_snapshot, id, up) {
            return false;
        }
        self.rerender_diff_review(anchor);
        if let Some(cell) = source_cell {
            let row = self.ui_panels.diff_review.as_ref().and_then(|state| {
                state.rows.iter().position(|row| {
                    [row.left, row.right]
                        .into_iter()
                        .flatten()
                        .any(|candidate| {
                            candidate.patch_line == cell.patch_line
                                && candidate.info == cell.info
                                && candidate.src_glyph == cell.src_glyph
                        })
                })
            });
            if let Some(row) = row {
                self.buffer_mut().cursor_mut().set_position(row, cursor_col);
            }
        }
        let offset = self.buffer().cursor().line().saturating_sub(screen_row);
        self.viewport.scroll_offset = offset;
        if let Some(window) = self
            .window_manager
            .as_mut()
            .and_then(|wm| wm.focused_window_mut())
        {
            window.set_scroll_offset(offset);
        }
        self.viewport.preserve_after_input();
        true
    }

    pub fn expand_diff_context_at_cursor(&mut self, up: bool) {
        let line = self.buffer().cursor().line();
        let id = self.ui_panels.diff_review.as_ref().and_then(|state| {
            state
                .context_rows
                .get(line)
                .cloned()
                .flatten()
                .or_else(|| state.context_rows.iter().skip(line).find_map(Clone::clone))
        });
        if !id.is_some_and(|id| self.expand_diff_context(&id, up)) {
            self.set_status_message("No more captured context in this direction");
        }
    }

    /// `w` in a curated review hides equal paired changes in either layout.
    pub fn toggle_diff_review_equal_changes(&mut self) {
        let Some(index) = self.review_buffer_index() else {
            return;
        };
        if self
            .ui_panels
            .diff_review
            .as_ref()
            .is_none_or(|state| state.custom.is_none())
        {
            return;
        }
        let anchor = self.diff_review_anchor(index, false);
        self.ui_panels.diff_review_hide_equal = !self.ui_panels.diff_review_hide_equal;
        self.rerender_diff_review(anchor);
        self.set_status_message(if self.ui_panels.diff_review_hide_equal {
            "Diff review: equal paired changes hidden (ignoring whitespace)"
        } else {
            "Diff review: showing all changes"
        });
    }

    /// `a` shows or hides messages attached to curated sections.
    pub fn toggle_diff_review_notes(&mut self) {
        let Some(index) = self.review_buffer_index() else {
            return;
        };
        if self
            .ui_panels
            .diff_review
            .as_ref()
            .and_then(|state| state.custom.as_ref())
            .is_none_or(|custom| {
                !custom
                    .sections
                    .iter()
                    .any(|section| section.message.is_some())
            })
        {
            return;
        }
        let line = self.buffers[index].cursor().line();
        let col = self.buffers[index].cursor().col();
        let source_line = self.ui_panels.diff_review.as_ref().and_then(|state| {
            state
                .rows
                .get(line)
                .and_then(|row| row.cell_at(col.0))
                .map(|cell| cell.patch_line)
        });
        let section_at_cursor = self.ui_panels.diff_review.as_ref().and_then(|state| {
            (state.rows.get(line).and_then(ReviewRow::info).is_none())
                .then(|| {
                    state
                        .hunk_lines
                        .partition_point(|heading| *heading <= line)
                        .checked_sub(1)
                })
                .flatten()
        });
        let anchor = self.diff_review_anchor(index, false);
        self.ui_panels.diff_review_hide_notes = !self.ui_panels.diff_review_hide_notes;
        self.rerender_diff_review(anchor);
        let restored = self.ui_panels.diff_review.as_ref().and_then(|state| {
            source_line
                .and_then(|source| {
                    state.rows.iter().position(|row| {
                        row.left.is_some_and(|cell| cell.patch_line == source)
                            || row.right.is_some_and(|cell| cell.patch_line == source)
                    })
                })
                .or_else(|| {
                    section_at_cursor.and_then(|section| state.hunk_lines.get(section).copied())
                })
        });
        if let Some(line) = restored {
            self.buffers[index].cursor_mut().set_position(
                line,
                if source_line.is_some() {
                    col
                } else {
                    GraphemeCol(0)
                },
            );
            self.center_cursor_in_viewport();
        }
        self.set_status_message(if self.ui_panels.diff_review_hide_notes {
            "Diff review: agent notes hidden"
        } else {
            "Diff review: agent notes visible"
        });
    }

    /// `s` in the review, or a click on the toolbar.
    pub fn toggle_diff_review_layout(&mut self) {
        let next = self
            .ui_panels
            .diff_review
            .as_ref()
            .map(|state| state.layout)
            .unwrap_or(self.ui_panels.diff_review_layout)
            .toggled();
        self.set_diff_review_layout(next);
    }

    /// Switches the review to `layout`, keeping the cursor on the same change.
    /// The choice sticks for reviews opened later in the session.
    pub fn set_diff_review_layout(&mut self, layout: DiffLayout) {
        self.ui_panels.diff_review_layout = layout;
        let Some(index) = self.review_buffer_index() else {
            return;
        };
        if self
            .ui_panels
            .diff_review
            .as_ref()
            .is_some_and(|state| state.layout == layout)
        {
            return;
        }
        // The rendered line text differs between layouts, so anchor on the
        // source position only.
        let anchor = self.diff_review_anchor(index, false);
        if let Some(state) = self.ui_panels.diff_review.as_mut() {
            state.layout = layout;
        }
        self.rerender_diff_review(anchor);
        self.set_status_message(format!("Diff review: {} view", layout.label()));
    }

    /// Re-flows the side-by-side layout after the window changed width.
    /// Returns true when the buffer was re-rendered.
    pub fn relayout_diff_review(&mut self) -> bool {
        let Some(index) = self.review_buffer_index() else {
            return false;
        };
        let (area_width, width) = self.diff_review_widths();
        // Re-flow when the pane or its text width changes. Split reviews have
        // no outer gutter, so opening one can widen the text area even when
        // the pane itself did not change size.
        let visible = index == self.current_buffer_index;
        let stale = self.ui_panels.diff_review.as_ref().is_some_and(|state| {
            state.layout == DiffLayout::Split
                && (state.layout_area_width != area_width
                    || (visible && width != state.layout_width))
        });
        if !stale {
            return false;
        }
        let anchor = self.diff_review_anchor(index, true);
        if let Some(state) = self.ui_panels.diff_review.as_mut() {
            state.layout_width = width;
            state.layout_area_width = area_width;
        }
        self.rerender_diff_review(anchor);
        true
    }

    /// Closes the review tab and drops its buffer and state.
    pub fn close_diff_review(&mut self) {
        let Some(index) = self.review_buffer_index() else {
            self.ui_panels.diff_review = None;
            return;
        };
        if index == self.current_buffer_index {
            if self.tab_count() > 1 {
                self.close_current_tab();
            } else if self.buffers.len() > 1 {
                let other = if index == 0 { 1 } else { index - 1 };
                self.switch_to_buffer(other);
                self.sync_current_tab_buffer();
            } else {
                self.add_buffer(Buffer::new());
                self.sync_current_tab_buffer();
            }
        }
        self.remove_review_buffer();
        self.ui_panels.diff_review = None;
        self.set_status_message("Diff review closed");
        self.mark_dirty();
    }

    /// A left click inside the review. Returns true when it hit the toolbar
    /// and the caller should not move the cursor.
    pub fn diff_review_click(&mut self, line: usize, col: usize) -> bool {
        if self.is_diff_review_buffer() {
            let item = self.diff_review().and_then(|state| {
                let text = state.line_text(line)?;
                let suffix = if text.ends_with("  [x] checked") {
                    "  [x] checked"
                } else if text.ends_with("  [ ]") {
                    "  [ ]"
                } else {
                    return None;
                };
                let start = crate::unicode::grapheme_count(&text[..text.len() - suffix.len()]) + 2;
                (col >= start && col < start + 3)
                    .then(|| state.item_rows.get(line).cloned().flatten())
                    .flatten()
            });
            if let Some(id) = item {
                let _ = self.toggle_diff_review_check(&id);
                return true;
            }
        }
        let Some(layout) = self
            .ui_panels
            .diff_review
            .as_ref()
            .filter(|state| state.buffer_id == self.buffer().id())
            .and_then(|state| state.toolbar.hit(line, col))
        else {
            return false;
        };
        self.set_diff_review_layout(layout);
        true
    }

    /// `Enter` in the review: open the file at the line under the cursor in
    /// the originating tab.
    pub fn diff_review_open_at_cursor(&mut self) {
        let Some(state) = self.ui_panels.diff_review.as_ref() else {
            return;
        };
        let cursor = self.buffer().cursor();
        let line = cursor.line();
        let col = cursor.col().0;

        if state.custom.is_some() || state.custom_target_at(line, col).is_some() {
            let target = state.custom_target_at(line, col);
            match target {
                Some((path, source_line, side)) => {
                    if let Err(error) = self.open_captured_review_source(&path, source_line, side) {
                        self.review_toast(
                            ToastLevel::Error,
                            format!("Could not open {path}: {error:#}"),
                        );
                    }
                }
                None => {
                    self.set_status_message("Move to a changed line and press Enter to open it")
                }
            }
            return;
        }

        if let Some(file) = state.file_for_stat_row(line) {
            if let Some(&target) = state.file_lines.get(file) {
                self.jump_to_review_line(target);
            }
            return;
        }

        let cell = state.row(line).and_then(|row| row.cell_at(col));
        let target = cell.and_then(|cell| state.cell_target(line, cell.info));
        let Some((file_index, new_line)) = target else {
            self.set_status_message("Move to a changed line and press Enter to open it");
            return;
        };
        let file = &state.patch.files[file_index];
        if file.status == "deleted" {
            let path = file.path.clone();
            self.review_toast(
                ToastLevel::Warning,
                format!("{path} was deleted on this branch"),
            );
            return;
        }
        let tab_width = self.diff_review_tab_width();
        let target_col = cell
            .map(|cell| state.source_column(&cell, col, tab_width))
            .unwrap_or(0);
        let path = state.patch.root.join(&file.path);
        let display_path = file.path.clone();

        self.go_to_review_origin();
        if let Err(error) = self.open_file(&path) {
            self.review_toast(
                ToastLevel::Error,
                format!("Could not open {display_path}: {error}"),
            );
            return;
        }
        self.buffer_mut()
            .cursor_mut()
            .set_position(new_line.saturating_sub(1), GraphemeCol(target_col));
        self.buffer_mut().validate_cursor_position();
        self.center_cursor_in_viewport();
        self.set_status_message(format!(
            "{display_path}:{new_line} · <Space>gd returns to the review"
        ));
        self.mark_dirty();
    }

    /// Opens a projected GUI review line by source identity. This avoids
    /// depending on the terminal row geometry when the GUI flows hunks itself.
    pub fn diff_review_open_source(
        &mut self,
        path: &str,
        line: usize,
        side: &str,
    ) -> anyhow::Result<()> {
        let state = self
            .ui_panels
            .diff_review
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No branch review is open"))?;
        if let Some(custom) = &state.custom {
            let valid = snapshot_saved_line(&custom.snapshot, path, line, side).is_some();
            anyhow::ensure!(valid, "Source is not in this review: {path}:{line}");
            return self.open_captured_review_source(path, line, side);
        }
        if state
            .context
            .contains_expanded_line(path, line, side == "old")
        {
            return self.open_captured_review_source(path, line, side);
        }
        let file = state
            .patch
            .files
            .iter()
            .find(|file| {
                file.path == path || (side == "old" && file.old_path.as_deref() == Some(path))
            })
            .ok_or_else(|| anyhow::anyhow!("File is not in this review: {path}"))?;
        anyhow::ensure!(
            file.status != "deleted",
            "{path} was deleted on this branch"
        );
        let file_index = state
            .patch
            .files
            .iter()
            .position(|entry| std::ptr::eq(entry, file))
            .expect("validated review file");
        let new_line = match side {
            "new" => line,
            "old" => state
                .patch
                .lines
                .iter()
                .find(|entry| {
                    entry.file == Some(file_index)
                        && entry.old_line == Some(line)
                        && matches!(entry.kind, PatchLineKind::Context | PatchLineKind::Removed)
                })
                .and_then(|entry| entry.new_line)
                .ok_or_else(|| anyhow::anyhow!("Old line is not in this review: {path}:{line}"))?,
            _ => anyhow::bail!("Unknown diff side: {side}"),
        }
        .max(1);
        let target = state.patch.root.join(&file.path);
        self.go_to_review_origin();
        self.open_file(&target)?;
        self.buffer_mut()
            .cursor_mut()
            .set_position(new_line - 1, GraphemeCol(0));
        self.buffer_mut().validate_cursor_position();
        self.center_cursor_in_viewport();
        self.set_status_message(format!(
            "{path}:{new_line} · <Space>gd returns to the review"
        ));
        self.mark_dirty();
        Ok(())
    }

    pub fn diff_review_goto_definition_at_cursor(&mut self) {
        let result = (|| {
            let state = self
                .diff_review()
                .ok_or_else(|| anyhow::anyhow!("No diff review"))?;
            let line = self.buffer().cursor().line();
            let col = self.buffer().cursor().col().0;
            let cell = state
                .row(line)
                .and_then(|row| row.cell_at(col))
                .ok_or_else(|| anyhow::anyhow!("Move to a symbol in the diff first"))?;
            let target = state
                .custom_target_at(line, col)
                .or_else(|| {
                    let file = state.patch.files.get(cell.info.file?)?;
                    if cell.info.kind == PatchLineKind::Removed {
                        Some((
                            file.old_path.as_ref().unwrap_or(&file.path).clone(),
                            cell.info.old_line?,
                            "old",
                        ))
                    } else {
                        Some((file.path.clone(), cell.info.new_line?, "new"))
                    }
                })
                .ok_or_else(|| anyhow::anyhow!("Move to a symbol in the diff first"))?;
            let text = snapshot_saved_line(&state.context_snapshot, &target.0, target.1, target.2)
                .ok_or_else(|| anyhow::anyhow!("Source is not in this review"))?;
            let glyphs = layout_body(
                text,
                self.diff_review_tab_width(),
                state.layout == DiffLayout::Split,
            );
            let offset = cell.src_glyph
                + col
                    .saturating_sub(cell.text_col)
                    .min(cell.text_len.saturating_sub(1));
            let source_col = glyphs
                .get(offset)
                .map(|glyph| grapheme_index_for_byte(text, glyph.src_byte))
                .unwrap_or(0);
            let utf16_column = crate::unicode::grapheme_indices(text)
                .take(source_col)
                .map(|(_, glyph)| glyph.encode_utf16().count() as u32)
                .sum();
            self.diff_review_goto_definition(&target.0, target.1, target.2, utf16_column)
        })();
        if let Err(error) = result {
            self.set_status_message(format!("Definition: {error:#}"));
        }
    }

    /// Resolve a symbol selected in either frontend against verified live source.
    /// A frozen/removed line must never send stale coordinates to an LSP server.
    pub fn diff_review_goto_definition(
        &mut self,
        path: &str,
        line: usize,
        side: &str,
        utf16_column: u32,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.is_diff_review_buffer(), "No active branch review");
        let state = self
            .diff_review()
            .ok_or_else(|| anyhow::anyhow!("No diff review is open"))?;
        let saved = snapshot_saved_line(&state.context_snapshot, path, line, side)
            .ok_or_else(|| anyhow::anyhow!("Source is not in this review: {path}:{line}"))?
            .to_string();
        anyhow::ensure!(
            utf16_column < saved.encode_utf16().count() as u32,
            "Select a symbol in the diff first"
        );
        let file = state
            .patch
            .files
            .iter()
            .find(|file| file.path == path || file.old_path.as_deref() == Some(path))
            .ok_or_else(|| anyhow::anyhow!("File is not in this review"))?;
        let target = state.patch.root.join(&file.path);
        let live = std::fs::read_to_string(&target)?;
        anyhow::ensure!(
            live.lines().nth(line.saturating_sub(1)) == Some(saved.as_str()),
            "This saved line differs from the live source; open the source to navigate definitions"
        );
        self.go_to_review_origin();
        self.open_file(target)?;
        anyhow::ensure!(
            self.buffer().line_text(line - 1).as_deref() == Some(saved.as_str()),
            "The source has unsaved changes at this line; select the symbol in the source"
        );
        let col = self.utf16_to_grapheme_col(line - 1, utf16_column);
        self.buffer_mut()
            .cursor_mut()
            .set_position(line - 1, GraphemeCol(col));
        self.buffer_mut().validate_cursor_position();
        self.center_cursor_in_viewport();
        self.request_goto_definition();
        self.mark_dirty();
        Ok(())
    }

    fn open_captured_review_source(
        &mut self,
        path: &str,
        line: usize,
        side: &str,
    ) -> anyhow::Result<()> {
        use std::path::Component;
        anyhow::ensure!(
            Path::new(path)
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
            "Source path is outside the review"
        );
        let state = self
            .ui_panels
            .diff_review
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No diff review is open"))?;
        let root = state.patch.root.clone();
        let snapshot = state.context_snapshot.clone();
        anyhow::ensure!(side == "old" || side == "new", "Unknown diff side: {side}");
        let saved_line = snapshot_saved_line(&snapshot, path, line, side)
            .ok_or_else(|| anyhow::anyhow!("Source is not in this review: {path}:{line}"))?;
        let current_matches = side == "new"
            && std::fs::read_to_string(root.join(path))
                .ok()
                .and_then(|text| text.lines().nth(line.saturating_sub(1)).map(str::to_string))
                .as_deref()
                == Some(saved_line);
        self.go_to_review_origin();
        if !current_matches {
            let (excerpt, target_line) = snapshot_source_excerpt(&snapshot, path, line, side)?;
            let label = if side == "old" { "Before" } else { "After" };
            self.open_diff_buffer_in_new_tab(&format!("{label} excerpt · {path}"), &excerpt);
            self.buffer_mut()
                .cursor_mut()
                .set_position(target_line, GraphemeCol(0));
            self.set_status_message(format!("Saved {label} excerpt · {path}:{line}"));
        } else {
            self.open_file(root.join(path))?;
            self.buffer_mut()
                .cursor_mut()
                .set_position(line.saturating_sub(1), GraphemeCol(0));
            self.set_status_message(format!("{path}:{line} · <Space>gd opens the live diff"));
        }
        self.buffer_mut().validate_cursor_position();
        self.center_cursor_in_viewport();
        self.mark_dirty();
        Ok(())
    }

    /// `]c` / `[c`: next or previous hunk in the review, or next changed
    /// region (git gutter) in an ordinary file.
    pub fn goto_change(&mut self, forward: bool) {
        if self.is_diff_review_buffer() {
            self.diff_review_goto_hunk(forward);
            return;
        }
        let cursor_line = self.buffer().cursor().line();
        let starts = self.buffer().git_status().hunk_starts();
        let target = if forward {
            starts.iter().copied().find(|line| *line > cursor_line)
        } else {
            starts
                .iter()
                .rev()
                .copied()
                .find(|line| *line < cursor_line)
        };
        match target {
            Some(line) => {
                self.buffer_mut()
                    .cursor_mut()
                    .set_position(line, GraphemeCol(0));
                self.buffer_mut().validate_cursor_position();
                self.center_cursor_in_viewport();
            }
            None => self.set_status_message(if forward {
                "No more changes below"
            } else {
                "No more changes above"
            }),
        }
    }

    pub fn diff_review_goto_hunk(&mut self, forward: bool) {
        let Some(state) = self.ui_panels.diff_review.as_ref() else {
            return;
        };
        let cursor_line = self.buffer().cursor().line();
        let target = next_in(&state.hunk_lines, cursor_line, forward);
        match target {
            Some(line) => self.jump_to_review_line(line),
            None => self.set_status_message(if forward { "Last hunk" } else { "First hunk" }),
        }
    }

    /// Moves the open review's cursor to the section of `path` (relative to the
    /// worktree). Returns false when the review has no such file.
    pub fn diff_review_jump_to_path(&mut self, path: &str) -> bool {
        let Some(state) = self.ui_panels.diff_review.as_ref() else {
            return false;
        };
        let Some(line) = state
            .patch
            .files
            .iter()
            .position(|file| file.path == path)
            .and_then(|index| state.file_lines.get(index).copied())
        else {
            return false;
        };
        self.jump_to_review_line(line);
        true
    }

    pub fn diff_review_goto_file(&mut self, forward: bool) {
        let Some(state) = self.ui_panels.diff_review.as_ref() else {
            return;
        };
        let cursor_line = self.buffer().cursor().line();
        let target = next_in(&state.file_nav, cursor_line, forward);
        match target {
            Some(line) => self.jump_to_review_line(line),
            None => self.set_status_message(if forward { "Last file" } else { "First file" }),
        }
    }

    /// `<Space>gf`: fetch the review base's remote branch in the background
    /// and refresh the review when it lands.
    pub fn fetch_review_base(&mut self) {
        if self.ui_panels.pending_git_fetch.is_some() {
            self.review_toast(ToastLevel::Info, "A fetch is already running");
            return;
        }
        let root = self.diff_review_workspace_hint();
        let base = match self.ui_panels.diff_review.as_ref() {
            Some(state) if state.explicit_spec.is_some() => Ok(state.patch.base.clone()),
            _ => self.resolve_review_base_for_path(&root),
        };
        let remote = match base {
            Ok(base) => base.remote,
            Err(error) => {
                self.review_toast(ToastLevel::Error, format!("Git fetch: {error:#}"));
                return;
            }
        };
        let Some((remote, branch)) = remote else {
            self.review_toast(
                ToastLevel::Warning,
                "The review base is not a remote branch; nothing to fetch",
            );
            return;
        };
        let target = format!("{remote}/{branch}");
        let (sender, receiver) = channel();
        std::thread::spawn(move || {
            let result = std::process::Command::new("git")
                .args(["fetch", "--no-tags", &remote, &branch])
                .current_dir(&root)
                .output();
            let outcome = match result {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
                Err(error) => Err(format!("could not run git: {error}")),
            };
            let _ = sender.send(outcome);
        });
        self.ui_panels.pending_git_fetch = Some(PendingGitFetch {
            receiver,
            target: target.clone(),
        });
        self.set_status_message(format!("Fetching {target}…"));
    }

    /// Polls a background fetch. Returns true when something changed.
    pub fn poll_git_fetch(&mut self) -> bool {
        let Some(pending) = self.ui_panels.pending_git_fetch.take() else {
            return false;
        };
        match pending.receiver.try_recv() {
            Ok(Ok(())) => {
                self.refresh_pullbase_gutters();
                let refreshed = self.review_buffer_index().is_some();
                if refreshed {
                    self.refresh_diff_review();
                }
                self.review_toast(
                    ToastLevel::Success,
                    if refreshed {
                        format!("Fetched {} · review refreshed", pending.target)
                    } else {
                        format!("Fetched {}", pending.target)
                    },
                );
                true
            }
            Ok(Err(message)) => {
                let message = if message.is_empty() {
                    "git fetch failed".to_string()
                } else {
                    message
                };
                self.review_toast(
                    ToastLevel::Error,
                    format!("Fetch {} failed: {message}", pending.target),
                );
                true
            }
            Err(TryRecvError::Empty) => {
                self.ui_panels.pending_git_fetch = Some(pending);
                false
            }
            Err(TryRecvError::Disconnected) => {
                self.review_toast(
                    ToastLevel::Error,
                    format!("Fetch {} failed unexpectedly", pending.target),
                );
                true
            }
        }
    }

    // -- internals ---------------------------------------------------------

    fn review_buffer_index(&self) -> Option<usize> {
        let state = self.ui_panels.diff_review.as_ref()?;
        self.find_buffer_index_by_id(state.buffer_id)
    }

    /// Captures where the cursor is, in source terms, before a re-render.
    /// `match_text` anchors on the patch line's text too, which only helps
    /// when the layout stays the same.
    fn diff_review_anchor(&self, index: usize, match_text: bool) -> Option<Anchor> {
        let state = self.ui_panels.diff_review.as_ref()?;
        let cursor_line = self.buffers[index].cursor().line();
        let text = match_text
            .then(|| state.line_text(cursor_line).map(str::to_string))
            .flatten();
        state.anchor(cursor_line, text)
    }

    /// Re-renders the review buffer from the patch already in state.
    fn rerender_diff_review(&mut self, anchor: Option<Anchor>) {
        let Some(index) = self.review_buffer_index() else {
            return;
        };
        let unsaved = self.unsaved_buffer_count();
        let tab_width = self.diff_review_tab_width();
        let rendered = {
            let state = self.ui_panels.diff_review.as_ref().expect("review state");
            match &state.custom {
                Some(custom) => render::render_custom(
                    custom,
                    state.custom_title.as_deref().unwrap_or("Custom diff"),
                    state.layout,
                    state.layout_width,
                    tab_width,
                    self.ui_panels.diff_review_hide_equal,
                    self.ui_panels.diff_review_hide_notes,
                ),
                None => render::render(
                    &state.patch,
                    state.layout,
                    state.layout_width,
                    unsaved,
                    tab_width,
                ),
            }
        };
        self.buffers[index].set_display_name(rendered.title.clone());
        self.apply_rendered_review(rendered);

        if let Some(anchor) = anchor {
            let target = self
                .ui_panels
                .diff_review
                .as_ref()
                .and_then(|state| state.line_for_anchor(&anchor));
            if let Some(line) = target {
                self.buffers[index]
                    .cursor_mut()
                    .set_position(line, GraphemeCol(0));
            }
        }
        if index == self.current_buffer_index {
            self.buffer_mut().validate_cursor_position();
            self.center_cursor_in_viewport();
        }
        self.mark_dirty();
    }

    /// Installs a fresh render into the state and the review buffer.
    fn apply_rendered_review(&mut self, mut rendered: Rendered) {
        let Some(index) = self.review_buffer_index() else {
            return;
        };
        let tab_width = self.diff_review_tab_width();
        let state = self.ui_panels.diff_review.as_mut().expect("review state");
        state.context_rows = render::expand_context(
            &mut rendered,
            &state.context,
            &state.context_snapshot,
            state.custom.is_some(),
            state.layout,
            state.layout_width,
            tab_width,
        );
        let checked = checks::item_checks(state, &self.ui_panels.diff_review_checks);
        state.checked_items = checked;
        state.show_checked = self.ui_panels.diff_review_show_checked;
        checks::filter_rendered(
            &mut rendered,
            &mut state.context_rows,
            &state.checked_items,
            state.show_checked,
        );
        state.item_rows = rendered.item_rows.clone();
        self.buffers[index].replace_content(&rendered.text);
        self.buffers[index].set_forced_highlights(rendered.highlights);
        let state = self.ui_panels.diff_review.as_mut().expect("review state");
        state.rows = rendered.rows;
        state.stat_rows = rendered.stat_rows;
        state.hunk_lines = rendered.hunk_lines;
        state.file_nav = {
            let mut lines = rendered.file_lines.clone();
            lines.retain(|line| *line != usize::MAX);
            lines.sort_unstable();
            lines.dedup();
            lines
        };
        state.file_lines = rendered.file_lines;
        state.toolbar = rendered.toolbar;
        state.text_lines = rendered.text.lines().map(str::to_string).collect();
        state.code_highlights = rendered.code_highlights;
        state.custom_targets = rendered.custom_targets;
    }

    /// `(buffer area width, text width)` the review lays out against.
    fn diff_review_widths(&self) -> (usize, usize) {
        let Some(area) = self.render_cache.last_buffer_area else {
            return (0, DEFAULT_LAYOUT_WIDTH);
        };
        let area_width = area.width as usize;
        let text_width = if self.render_cache.last_text_width > 0 {
            self.render_cache.last_text_width
        } else {
            area_width.saturating_sub(self.render_cache.last_gutter_width)
        };
        // Stop one column short so a full-width row never triggers a soft wrap.
        (area_width, text_width.saturating_sub(1).max(20))
    }

    fn diff_review_tab_width(&self) -> usize {
        self.indent_options().tab_width
    }

    /// Switches to the review tab (or shows the review buffer in a new tab)
    /// and refreshes it.
    fn enter_diff_review(&mut self) {
        let Some(index) = self.review_buffer_index() else {
            return;
        };
        let review_id = self.buffers[index].id();
        let current_id = self.buffer().id();
        let current_tab = self.current_tab_index();
        if let Some(state) = self.ui_panels.diff_review.as_mut() {
            state.origin_buffer_id = Some(current_id);
            state.origin_tab = current_tab;
        }

        let tab = self
            .tab_page_manager()
            .tabs()
            .iter()
            .position(|tab| tab.buffer_id() == Some(review_id));
        match tab {
            Some(tab) => self.goto_tab(tab),
            None => {
                self.sync_current_tab_buffer();
                self.tab_page_manager_mut().new_tab();
                self.tab_page_manager_mut()
                    .current_tab_mut()
                    .set_buffer_id(review_id);
                self.switch_to_buffer(index);
            }
        }
        self.refresh_diff_review();
    }

    fn leave_diff_review(&mut self) {
        self.go_to_review_origin();
        self.mark_dirty();
    }

    /// Switches to the tab the review was entered from.
    fn go_to_review_origin(&mut self) {
        let Some(state) = self.ui_panels.diff_review.as_ref() else {
            return;
        };
        let review_id = state.buffer_id;
        let origin_id = state.origin_buffer_id;
        let origin_tab = state.origin_tab;
        let review_tab = self.current_tab_index();

        let tabs = self.tab_page_manager().tabs();
        let by_buffer = origin_id.and_then(|origin| {
            tabs.iter().position(|tab| {
                tab.buffer_id() == Some(origin) && tab.buffer_id() != Some(review_id)
            })
        });
        let target = by_buffer.or_else(|| {
            (origin_tab < tabs.len() && origin_tab != review_tab).then_some(origin_tab)
        });
        match target {
            Some(tab) => self.goto_tab(tab),
            None => {
                // Nowhere to return to: give the file its own tab and leave
                // the review where it is.
                self.new_tab();
            }
        }
    }

    fn jump_to_review_line(&mut self, line: usize) {
        self.buffer_mut()
            .cursor_mut()
            .set_position(line, GraphemeCol(0));
        self.buffer_mut().validate_cursor_position();
        self.center_cursor_in_viewport();
        self.mark_dirty();
    }

    fn remove_review_buffer(&mut self) {
        let Some(index) = self.review_buffer_index() else {
            return;
        };
        if index == self.current_buffer_index {
            return;
        }
        self.buffers.remove(index);
        if self.current_buffer_index > index {
            self.current_buffer_index -= 1;
        }
    }

    pub fn resolve_review_base_for_path(&self, path: &Path) -> anyhow::Result<ReviewBase> {
        let branch = native_diff::pullbase_for_path(
            path,
            self.options.pullbase.as_deref(),
            &self.options.pullbase_paths,
        )?;
        native_diff::resolve_pullbase(path, branch)
    }

    pub fn diff_review_workspace_hint(&self) -> PathBuf {
        if let Some(state) = self.ui_panels.diff_review.as_ref() {
            return state.patch.root.clone();
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let Some(path) = self.buffer().file_path() else {
            return cwd;
        };
        let path = Path::new(path);
        if path.is_absolute() {
            path.parent().map(Path::to_path_buf).unwrap_or(cwd)
        } else {
            cwd.join(path)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or(cwd)
        }
    }

    fn unsaved_buffer_count(&self) -> usize {
        self.buffers
            .iter()
            .filter(|buffer| buffer.is_modified() && buffer.file_path().is_some())
            .count()
    }

    fn review_toast(&mut self, level: ToastLevel, message: impl Into<String>) {
        self.push_toast(ToastRequest::new(ToastSource::Git, level, message));
    }
}

/// Byte ranges of each line of `patch.text`, in the same order as
/// [`ReviewPatch::lines`].
fn patch_line_ranges(patch: &ReviewPatch) -> Vec<(usize, usize)> {
    let mut ranges = Vec::with_capacity(patch.lines.len());
    let mut start = 0;
    for (offset, byte) in patch.text.bytes().enumerate() {
        if byte == b'\n' {
            ranges.push((start, offset));
            start = offset + 1;
        }
    }
    if start < patch.text.len() {
        ranges.push((start, patch.text.len()));
    }
    ranges
}

/// Finds saved source text without consulting files that may have changed since capture.
fn snapshot_saved_line<'a>(
    snapshot: &'a native_diff::ReviewSnapshot,
    path: &str,
    line: usize,
    side: &str,
) -> Option<&'a str> {
    snapshot
        .source_for(path, side)
        .and_then(|source| source.line(line))
        .or_else(|| {
            let patch = &snapshot.patch;
            patch
                .text
                .lines()
                .zip(&patch.lines)
                .find_map(|(text, entry)| {
                    let file = patch.files.get(entry.file?)?;
                    let matches = match side {
                        "old" => {
                            file.old_path.as_deref().unwrap_or(&file.path) == path
                                && entry.old_line == Some(line)
                                && matches!(
                                    entry.kind,
                                    PatchLineKind::Context | PatchLineKind::Removed
                                )
                        }
                        "new" => {
                            file.path == path
                                && entry.new_line == Some(line)
                                && matches!(
                                    entry.kind,
                                    PatchLineKind::Context | PatchLineKind::Added
                                )
                        }
                        _ => false,
                    };
                    matches.then(|| text.get(1..)).flatten()
                })
        })
}

/// Builds a numbered excerpt from saved source, falling back to the canonical
/// patch for reviews captured before surrounding source was retained.
fn snapshot_source_excerpt(
    snapshot: &native_diff::ReviewSnapshot,
    path: &str,
    requested_line: usize,
    side: &str,
) -> anyhow::Result<(String, usize)> {
    let mut source = BTreeMap::new();
    if let Some(saved) = snapshot.source_for(path, side) {
        for window in &saved.windows {
            for (offset, text) in window.lines.iter().enumerate() {
                source.insert(window.start_line + offset, text.as_str());
            }
        }
    }
    let patch = &snapshot.patch;
    for (text, entry) in patch.text.lines().zip(&patch.lines) {
        let Some(file) = entry.file.and_then(|index| patch.files.get(index)) else {
            continue;
        };
        let number = match side {
            "old"
                if file.old_path.as_deref().unwrap_or(&file.path) == path
                    && matches!(entry.kind, PatchLineKind::Context | PatchLineKind::Removed) =>
            {
                entry.old_line
            }
            "new"
                if file.path == path
                    && matches!(entry.kind, PatchLineKind::Context | PatchLineKind::Added) =>
            {
                entry.new_line
            }
            _ => None,
        };
        if let Some(number) = number {
            source.entry(number).or_insert(text.get(1..).unwrap_or(""));
        }
    }
    anyhow::ensure!(!source.is_empty(), "No saved {side} source for {path}");
    let mut text = format!("# Saved {side} excerpt · {path}\n");
    let mut previous = None;
    let mut target = 0;
    let mut output_line = 1;
    for (number, body) in source {
        if previous.is_some_and(|last: usize| number > last + 1) {
            text.push_str("       ⋮\n");
            output_line += 1;
        }
        if number == requested_line {
            target = output_line;
        }
        text.push_str(&format!("{number:>6} │ {body}\n"));
        previous = Some(number);
        output_line += 1;
    }
    Ok((text, target))
}

fn next_in(lines: &[usize], cursor_line: usize, forward: bool) -> Option<usize> {
    if forward {
        lines.iter().copied().find(|line| *line > cursor_line)
    } else {
        lines.iter().rev().copied().find(|line| *line < cursor_line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_in_moves_relative_to_cursor() {
        let lines = [3, 8, 12];
        assert_eq!(next_in(&lines, 0, true), Some(3));
        assert_eq!(next_in(&lines, 3, true), Some(8));
        assert_eq!(next_in(&lines, 12, true), None);
        assert_eq!(next_in(&lines, 12, false), Some(8));
        assert_eq!(next_in(&lines, 3, false), None);
    }
}
