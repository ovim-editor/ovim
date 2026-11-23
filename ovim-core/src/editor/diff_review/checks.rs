//! Review progress is separate from the patch: checking never edits Git state.
use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::{render::Rendered, DiffReviewState, Editor};
use crate::native_diff::PatchLineKind;

/// Checks belong to canonical changed lines, not a particular arrangement.
/// A file's full patch scopes its local line offsets, so checks survive section
/// reordering, overlay removal and other files changing, but never new content.
pub type ReviewChecks = BTreeMap<String, BTreeSet<usize>>;

struct CheckIdentity {
    files: Vec<String>,
    lines: Vec<Option<(usize, usize)>>,
}

impl CheckIdentity {
    fn new(state: &DiffReviewState) -> Self {
        let patch = &state.patch;
        let mut hashes: Vec<_> = patch
            .files
            .iter()
            .map(|file| {
                let mut hash = Sha256::new();
                for part in [
                    patch.root.to_string_lossy().as_ref(),
                    &patch.comparison_base_oid,
                    &serde_json::to_string(file).expect("file serializes"),
                ] {
                    hash.update(part.len().to_le_bytes());
                    hash.update(part.as_bytes());
                }
                if file.binary {
                    hash.update(
                        state
                            .context_snapshot
                            .content_fingerprint
                            .as_deref()
                            .unwrap_or("")
                            .as_bytes(),
                    );
                }
                hash
            })
            .collect();
        let mut offsets = vec![0; hashes.len()];
        let lines = patch
            .text
            .lines()
            .zip(&patch.lines)
            .map(|(text, line)| {
                let file = line.file?;
                let hash = hashes.get_mut(file)?;
                hash.update(text.len().to_le_bytes());
                hash.update(text.as_bytes());
                let offset = offsets[file];
                offsets[file] += 1;
                Some((file, offset))
            })
            .collect();
        Self {
            files: hashes
                .into_iter()
                .map(|hash| format!("{:x}", hash.finalize()))
                .collect(),
            lines,
        }
    }

    fn token(&self, line: usize) -> Option<(&str, usize)> {
        let (file, offset) = self.lines.get(line).copied().flatten()?;
        Some((&self.files[file], offset))
    }

    fn is_checked(&self, checks: &ReviewChecks, line: usize) -> bool {
        self.token(line).is_some_and(|(key, offset)| {
            checks.get(key).is_some_and(|lines| lines.contains(&offset))
        })
    }
}

pub(super) fn item_checks(state: &DiffReviewState, checks: &ReviewChecks) -> BTreeSet<String> {
    let identity = CheckIdentity::new(state);
    item_coverage(state)
        .into_iter()
        .filter(|(_, lines)| {
            !lines.is_empty() && lines.iter().all(|&line| identity.is_checked(checks, line))
        })
        .map(|(id, _)| id)
        .collect()
}

/// Canonical changed-line ownership keeps file and guided views consistent.
fn item_coverage(state: &DiffReviewState) -> BTreeMap<String, BTreeSet<usize>> {
    let mut coverage: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let patch = &state.patch;
    let mut changes = vec![BTreeSet::new(); patch.files.len()];
    let mut all = vec![BTreeSet::new(); patch.files.len()];
    for (index, line) in patch.lines.iter().enumerate() {
        if let Some(file) = line.file.filter(|&file| file < patch.files.len()) {
            all[file].insert(index);
            if matches!(line.kind, PatchLineKind::Added | PatchLineKind::Removed) {
                changes[file].insert(index);
            }
        }
    }
    for ((file, changes), all) in patch.files.iter().zip(changes).zip(all) {
        coverage.insert(
            format!("file:{}", file.path),
            if changes.is_empty() { all } else { changes },
        );
    }
    if let Some(custom) = &state.custom {
        for section in &custom.sections {
            let changes: BTreeSet<_> = section
                .lines
                .iter()
                .filter(|line| matches!(line.kind, PatchLineKind::Added | PatchLineKind::Removed))
                .map(|line| line.source_patch_line)
                .collect();
            let lines = if changes.is_empty() {
                section
                    .lines
                    .iter()
                    .map(|line| line.source_patch_line)
                    .collect()
            } else {
                changes
            };
            coverage.insert(format!("section:{}", section.id), lines);
        }
    }
    coverage
}

impl Editor {
    pub fn toggle_diff_review_check(&mut self, id: &str) -> Result<(), String> {
        if !self.is_diff_review_buffer() {
            return Err("No active diff review".into());
        }
        let state = self.diff_review().expect("active review");
        let cursor = self.buffer().cursor().line();
        let next_item = state
            .item_rows
            .iter()
            .skip(cursor)
            .flatten()
            .find(|candidate| candidate.as_str() != id)
            .cloned()
            .or_else(|| {
                state
                    .item_rows
                    .iter()
                    .take(cursor)
                    .rev()
                    .flatten()
                    .find(|candidate| candidate.as_str() != id)
                    .cloned()
            });
        let coverage = item_coverage(state);
        let lines = coverage.get(id).ok_or("Unknown diff review item")?;
        let identity = CheckIdentity::new(state);
        let uncheck = state.checked_items.contains(id);
        for &line in lines {
            if let Some((key, offset)) = identity.token(line) {
                if uncheck {
                    if let Some(offsets) = self.ui_panels.diff_review_checks.get_mut(key) {
                        offsets.remove(&offset);
                        if offsets.is_empty() {
                            self.ui_panels.diff_review_checks.remove(key);
                        }
                    }
                } else {
                    self.ui_panels
                        .diff_review_checks
                        .entry(key.to_string())
                        .or_default()
                        .insert(offset);
                }
            }
        }
        self.rerender_diff_review(None);
        if !self.ui_panels.diff_review_show_checked {
            if let Some(line) = next_item.and_then(|next| {
                self.diff_review()?
                    .item_rows
                    .iter()
                    .position(|item| item.as_ref() == Some(&next))
            }) {
                self.jump_to_review_line(line);
            }
        }
        Ok(())
    }

    pub fn toggle_diff_review_check_at_cursor(&mut self) {
        let id = self
            .diff_review()
            .and_then(|state| state.item_rows.get(self.buffer().cursor().line()))
            .cloned()
            .flatten();
        if let Some(id) = id {
            let _ = self.toggle_diff_review_check(&id);
        } else {
            self.set_status_message("Move to a file or change to check it · X shows checked items");
        }
    }

    pub fn toggle_diff_review_show_checked(&mut self) {
        if !self.is_diff_review_buffer() {
            return;
        }
        self.ui_panels.diff_review_show_checked = !self.ui_panels.diff_review_show_checked;
        let anchor = self
            .review_buffer_index()
            .and_then(|index| self.diff_review_anchor(index, true));
        self.rerender_diff_review(anchor);
    }
}

/// Compact every parallel row mapping together, after context expansion.
/// Source patch indices stay untouched; hidden navigation targets disappear.
pub(super) fn filter_rendered(
    rendered: &mut Rendered,
    context: &mut Vec<Option<String>>,
    checked: &BTreeSet<String>,
    show: bool,
) {
    let original: Vec<_> = rendered.text.lines().collect();
    let mut mapping = vec![usize::MAX; original.len()];
    let mut text = String::new();
    let mut rows = Vec::new();
    let mut highlights = Vec::new();
    let mut targets = Vec::new();
    let mut items = Vec::new();
    let mut contexts = Vec::new();
    let mut previous = None;
    for (index, line) in original.iter().enumerate() {
        let item = rendered.item_rows.get(index).cloned().flatten();
        let is_checked = item.as_ref().is_some_and(|id| checked.contains(id));
        if is_checked && !show {
            continue;
        }
        mapping[index] = rows.len();
        let heading = item.is_some()
            && (item != previous || rendered.stat_rows.iter().any(|(row, _)| *row == index));
        text.push_str(line);
        let mut spans = rendered.highlights[index].clone();
        if heading {
            let start = line.len();
            let suffix = if is_checked { "  [x] checked" } else { "  [ ]" };
            text.push_str(suffix);
            spans.push((
                start..start + suffix.len(),
                crate::syntax::HighlightGroup::Comment,
            ));
        }
        text.push('\n');
        rows.push(rendered.rows[index].clone());
        highlights.push(spans);
        targets.push(rendered.custom_targets.get(index).cloned().flatten());
        contexts.push(context.get(index).cloned().flatten());
        previous = item.clone();
        items.push(item);
    }
    let visible_ids: BTreeSet<_> = rendered.item_rows.iter().flatten().collect();
    let count = visible_ids
        .iter()
        .filter(|id| checked.contains(**id))
        .count();
    // Always show progress, including when the final item has just disappeared.
    text.push_str(&format!(
        "\n{count}/{} checked · x check/uncheck · X {} checked\n",
        visible_ids.len(),
        if show { "hide" } else { "show" }
    ));
    if items.iter().all(Option::is_none) && count > 0 {
        text.push_str("All visible changes checked · X to review them again\n");
    }
    while rows.len() < text.lines().count() {
        rows.push(Default::default());
        highlights.push(Vec::new());
        targets.push(None);
        items.push(None);
        contexts.push(None);
    }
    let remap = |line: usize| mapping.get(line).copied().unwrap_or(usize::MAX);
    rendered.hunk_lines = rendered
        .hunk_lines
        .iter()
        .map(|&line| remap(line))
        .filter(|&line| line != usize::MAX)
        .collect();
    rendered
        .file_lines
        .iter_mut()
        .for_each(|line| *line = remap(*line));
    rendered.stat_rows = rendered
        .stat_rows
        .iter()
        .filter_map(|&(line, file)| (remap(line) != usize::MAX).then_some((remap(line), file)))
        .collect();
    rendered.toolbar.line = remap(rendered.toolbar.line);
    rendered.text = text;
    rendered.rows = rows;
    rendered.highlights = highlights;
    rendered.custom_targets = targets;
    rendered.item_rows = items;
    *context = contexts;
}
