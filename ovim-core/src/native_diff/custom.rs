//! Frozen review snapshots and explicit reassignment of changed lines.
//!
//! A pairing changes only presentation. Every added and removed line still
//! points at its unique line in the canonical Git patch.

use super::{review_patch, DiffFile, PatchLineKind, ReviewBase, ReviewPatch};
use anyhow::{bail, ensure, Context, Result};
use git2::{Oid, Repository, Tree};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path};

const MAX_FROZEN_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_FROZEN_BYTES_PER_SOURCE: usize = 1024 * 1024;
const MAX_SOURCE_READ_BYTES: usize = 32 * 1024 * 1024;
const CONTEXT_RADIUS: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshot {
    pub id: String,
    pub patch: ReviewPatch,
    pub blocks: Vec<ChangeBlock>,
    /// Captured source text for navigation and context on replay.
    #[serde(default)]
    pub sources: Vec<SourceFileSnapshot>,
    /// Full byte identity of every comparison endpoint, independent of excerpt limits.
    /// Missing in old snapshots or when any endpoint cannot be captured safely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFileSnapshot {
    pub file: usize,
    pub old: Option<FrozenSource>,
    pub new: Option<FrozenSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenSource {
    pub path: String,
    /// Gaps between windows are uncaptured source, not unchanged lines.
    pub windows: Vec<SourceWindow>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceWindow {
    /// 1-based line number of the first line in this window.
    pub start_line: usize,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeBlock {
    pub id: String,
    pub kind: PatchLineKind,
    /// Index in the canonical patch's file summary.
    pub file: usize,
    /// 1-based source line on the side described by `kind`.
    pub start_line: usize,
    pub line_count: usize,
    /// Index of the first line in the canonical patch.
    pub patch_line_start: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRef {
    pub block_id: String,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub count: Option<usize>,
}

/// An ordered review section. The historical `pairings` wire name is retained:
/// either side alone owns a deletion/addition, and both sides own a replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffPairing {
    pub label: Option<String>,
    /// Optional explanation shown with this section, never part of the patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<ChangeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<ChangeRef>,
    /// Explanatory cross-reference only; never consumes or duplicates source lines.
    #[serde(
        default,
        rename = "related_to",
        skip_serializing_if = "Option::is_none"
    )]
    pub related_to: Option<ChangeRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSectionLine {
    pub kind: PatchLineKind,
    /// Content without the unified patch marker on code lines.
    pub text: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    /// Canonical patch line identity, even for a moved presentation line.
    pub source_patch_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSection {
    pub id: String,
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub is_reassigned: bool,
    pub lines: Vec<ReviewSectionLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomReview {
    pub snapshot: ReviewSnapshot,
    pub pairings: Vec<DiffPairing>,
    pub sections: Vec<ReviewSection>,
}

pub fn review_snapshot(path: &Path, base: &ReviewBase) -> Result<ReviewSnapshot> {
    let snapshot = review_display_snapshot(path, base)?;
    ensure!(
        !snapshot.patch.truncated,
        "Diff exceeds the 4 MiB review limit; narrow the comparison before creating a custom diff"
    );
    Ok(snapshot)
}

/// Ordinary diffs remain viewable at the patch limit, but a truncated comparison
/// cannot supply context or prove that a saved refinement is still applicable.
pub fn review_display_snapshot(path: &Path, base: &ReviewBase) -> Result<ReviewSnapshot> {
    let patch = review_patch(path, base)?;
    if patch.truncated {
        return Ok(ReviewSnapshot {
            id: String::new(),
            patch,
            blocks: Vec::new(),
            sources: Vec::new(),
            content_fingerprint: None,
        });
    }
    let mut snapshot = ReviewSnapshot::from_patch(patch)?;
    snapshot.capture_sources(path)?;
    snapshot.refresh_id()?;
    Ok(snapshot)
}

impl ReviewSnapshot {
    pub fn source_for(&self, path: &str, side: &str) -> Option<&FrozenSource> {
        self.sources.iter().find_map(|entry| {
            let source = match side {
                "old" => entry.old.as_ref(),
                "new" => entry.new.as_ref(),
                _ => None,
            }?;
            (source.path == path).then_some(source)
        })
    }

    fn refresh_id(&mut self) -> Result<()> {
        let digest = Sha256::digest(serde_json::to_vec(&(
            &self.patch,
            &self.sources,
            &self.content_fingerprint,
        ))?);
        self.id = format!(
            "diff_{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        Ok(())
    }

    fn capture_sources(&mut self, path: &Path) -> Result<()> {
        let repo = Repository::discover(path)?;
        let base_oid = Oid::from_str(&self.patch.comparison_base_oid)?;
        let old_tree = if base_oid.is_zero() {
            // Compared before the first commit: the base is the empty tree.
            repo.find_tree(repo.treebuilder(None)?.write()?)?
        } else {
            repo.find_commit(base_oid)?.tree()?
        };
        let new_tree = comparison_target_tree(&repo, &self.patch.base.spec)?;
        let mut remaining = MAX_FROZEN_SOURCE_BYTES;
        let mut identities = Vec::new();
        let mut complete_identity = true;
        let raw = self.patch.text.lines().collect::<Vec<_>>();
        let mut by_file = vec![Vec::new(); self.patch.files.len()];
        for (index, line) in self.patch.lines.iter().enumerate() {
            if let Some(file) = line.file {
                if let Some(entries) = by_file.get_mut(file) {
                    entries.push(index);
                }
            }
        }

        for (file_index, file) in self.patch.files.iter().enumerate() {
            let mut captured = SourceFileSnapshot {
                file: file_index,
                old: None,
                new: None,
            };
            for is_old in [true, false] {
                let source_path = if is_old {
                    old_path(file)
                } else {
                    file.path.clone()
                };
                if !safe_relative_path(&source_path) {
                    bail!("Git diff path is not a safe relative path: {source_path}");
                }
                let changed = by_file[file_index]
                    .iter()
                    .map(|&index| &self.patch.lines[index])
                    .filter(|line| {
                        line.kind
                            == if is_old {
                                PatchLineKind::Removed
                            } else {
                                PatchLineKind::Added
                            }
                    })
                    .filter_map(|line| if is_old { line.old_line } else { line.new_line })
                    .collect::<Vec<_>>();
                let source = if is_old {
                    capture_from_tree(&repo, &old_tree, &source_path)?
                } else if let Some(tree) = &new_tree {
                    capture_from_tree(&repo, tree, &source_path)?
                } else {
                    capture_from_worktree(&self.patch.root, &source_path)?
                };
                complete_identity &= source.identity.is_some();
                identities.push((file_index, is_old, source_path.clone(), source.identity));
                let Some(bytes) = source.bytes.filter(|_| !file.binary) else {
                    continue;
                };
                let source = source_lines(&bytes);
                verify_changed_lines(
                    &self.patch,
                    &raw,
                    &by_file[file_index],
                    is_old,
                    &source,
                    &source_path,
                )?;
                let budget = remaining.min(MAX_FROZEN_BYTES_PER_SOURCE);
                let mut allowance = budget;
                let frozen = freeze_source(&source_path, &source, &changed, &mut allowance);
                remaining -= budget - allowance;
                if is_old {
                    captured.old = frozen;
                } else {
                    captured.new = frozen;
                }
            }
            if captured.old.is_some() || captured.new.is_some() {
                self.sources.push(captured);
            }
        }
        if complete_identity {
            self.content_fingerprint = Some(format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&identities)?)
            ));
        }
        Ok(())
    }

    pub fn from_patch(patch: ReviewPatch) -> Result<Self> {
        ensure!(
            !patch.truncated && patch.text.len() <= super::MAX_PATCH_BYTES,
            "Diff exceeds the 4 MiB review limit; narrow the comparison before creating a custom diff"
        );
        let raw: Vec<&str> = patch.text.lines().collect();
        ensure!(
            raw.len() == patch.lines.len(),
            "Git patch text and source map have different line counts"
        );

        let mut blocks: Vec<ChangeBlock> = Vec::new();
        let mut additions = vec![0usize; patch.files.len()];
        let mut deletions = vec![0usize; patch.files.len()];
        for (index, line) in patch.lines.iter().enumerate() {
            let (side, source_line) = match line.kind {
                PatchLineKind::Added => (&mut additions, line.new_line),
                PatchLineKind::Removed => (&mut deletions, line.old_line),
                _ => continue,
            };
            let file = line.file.context("Changed Git line has no file identity")?;
            let source_line = source_line.context("Changed Git line has no source line")?;
            let count = side
                .get_mut(file)
                .context("Changed Git line references a missing file")?;
            *count += 1;
            if let Some(last) = blocks.last_mut() {
                if last.kind == line.kind
                    && last.file == file
                    && last.patch_line_start + last.line_count == index
                {
                    last.line_count += 1;
                    continue;
                }
            }
            blocks.push(ChangeBlock {
                id: format!(
                    "{}_{}",
                    if line.kind == PatchLineKind::Added {
                        "added"
                    } else {
                        "removed"
                    },
                    index
                ),
                kind: line.kind,
                file,
                start_line: source_line,
                line_count: 1,
                patch_line_start: index,
            });
        }
        for (index, file) in patch.files.iter().enumerate() {
            ensure!(
                additions[index] == file.additions && deletions[index] == file.deletions,
                "Git patch for {} is incomplete: expected +{}/-{}, found +{}/-{}",
                file.path,
                file.additions,
                file.deletions,
                additions[index],
                deletions[index]
            );
        }
        let digest = Sha256::digest(serde_json::to_vec(&patch)?);
        let id = format!(
            "diff_{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        Ok(Self {
            id,
            patch,
            blocks,
            sources: Vec::new(),
            content_fingerprint: None,
        })
    }

    /// Reassign disjoint line ranges; unassigned lines stay in canonical order.
    pub fn reassign(&self, pairings: &[DiffPairing]) -> Result<CustomReview> {
        let mut assigned = vec![false; self.patch.lines.len()];
        let mut sections = Vec::with_capacity(pairings.len() + self.patch.files.len());
        let raw: Vec<&str> = self.patch.text.lines().collect();

        for (index, pairing) in pairings.iter().enumerate() {
            ensure!(
                pairing.old.is_some() || pairing.new.is_some(),
                "A section must own removed or added lines"
            );
            ensure!(
                pairing
                    .label
                    .as_deref()
                    .is_none_or(|label| !label.contains(['\n', '\r'])),
                "Section labels must be single lines"
            );
            if let Some(message) = &pairing.message {
                ensure!(
                    !message.trim().is_empty()
                        && message.chars().count() <= 2000
                        && !message
                            .chars()
                            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t')),
                    "Section message must be 1–2000 characters of safe, nonblank text"
                );
            }
            let old = pairing
                .old
                .as_ref()
                .map(|reference| self.select(reference, PatchLineKind::Removed, &mut assigned))
                .transpose()?
                .unwrap_or_default();
            let new = pairing
                .new
                .as_ref()
                .map(|reference| self.select(reference, PatchLineKind::Added, &mut assigned))
                .transpose()?
                .unwrap_or_default();
            let old_path = pairing
                .old
                .as_ref()
                .map(|reference| {
                    self.block(&reference.block_id)
                        .map(|block| old_path(&self.patch.files[block.file]))
                })
                .transpose()?;
            let new_path = pairing
                .new
                .as_ref()
                .map(|reference| {
                    self.block(&reference.block_id)
                        .map(|block| self.patch.files[block.file].path.clone())
                })
                .transpose()?;
            let mut label = pairing.label.clone();
            if let Some(reference) = &pairing.related_to {
                let block = self.block(&reference.block_id)?;
                let range = self.reference_range(reference)?;
                // Owned selections are contiguous canonical ranges. Test intervals,
                // rather than comparing every reference line to every owned line.
                let overlaps = |owned: &[usize]| {
                    owned
                        .first()
                        .zip(owned.last())
                        .is_some_and(|(start, end)| range.start <= *end && range.end > *start)
                };
                ensure!(
                    !overlaps(&old) && !overlaps(&new),
                    "Related source must be outside this section's owned lines"
                );
                let source = &self.patch.files[block.file];
                let path = if block.kind == PatchLineKind::Removed {
                    source
                        .old_path
                        .clone()
                        .unwrap_or_else(|| source.path.clone())
                } else {
                    source.path.clone()
                };
                let start = block
                    .start_line
                    .checked_add(reference.offset.unwrap_or(0))
                    .context("Invalid source coordinate")?;
                let end = start
                    .checked_add(range.len() - 1)
                    .context("Invalid source coordinate")?;
                let side = if block.kind == PatchLineKind::Removed {
                    "before"
                } else {
                    "after"
                };
                let location = if start == end {
                    start.to_string()
                } else {
                    format!("{start}–{end}")
                };
                let description = format!(
                    "Related source ({side}): {}:{location}",
                    path.escape_debug()
                );
                label = Some(match label {
                    Some(label) if !label.is_empty() => format!("{label} · {description}"),
                    _ => description,
                });
            }
            let mut lines = Vec::with_capacity(old.len() + new.len());
            for patch_line in old.into_iter().chain(new) {
                lines.push(section_line(&self.patch, &raw, patch_line));
            }
            sections.push(ReviewSection {
                id: format!("pair_{index}"),
                label,
                message: pairing.message.clone(),
                old_path,
                new_path,
                is_reassigned: true,
                lines,
            });
        }

        sections.extend(self.residual_sections(&raw, &assigned));
        let review = CustomReview {
            snapshot: self.clone(),
            pairings: pairings.to_vec(),
            sections,
        };
        review.validate_coverage()?;
        Ok(review)
    }

    fn block(&self, id: &str) -> Result<&ChangeBlock> {
        self.blocks
            .iter()
            .find(|block| block.id == id)
            .with_context(|| format!("Unknown change block '{id}'"))
    }

    fn select(
        &self,
        reference: &ChangeRef,
        expected: PatchLineKind,
        assigned: &mut [bool],
    ) -> Result<Vec<usize>> {
        let block = self.block(&reference.block_id)?;
        ensure!(
            block.kind == expected,
            "Block '{}' is on the wrong side of the section",
            reference.block_id
        );
        let range = self.reference_range(reference)?;
        let start = range.start;
        let end = range.end;
        for slot in &mut assigned[start..end] {
            if *slot {
                bail!("Section reuses a changed line from block '{}'", block.id);
            }
            *slot = true;
        }
        Ok((start..end).collect())
    }

    fn reference_range(&self, reference: &ChangeRef) -> Result<std::ops::Range<usize>> {
        let block = self.block(&reference.block_id)?;
        let offset = reference.offset.unwrap_or(0);
        ensure!(
            offset < block.line_count,
            "Offset exceeds block '{}'; its length is {}",
            block.id,
            block.line_count
        );
        let count = reference.count.unwrap_or(block.line_count - offset);
        ensure!(count > 0, "Source range cannot be empty");
        ensure!(
            count <= block.line_count - offset,
            "Range exceeds block '{}'; its length is {}",
            block.id,
            block.line_count
        );
        let start = block
            .patch_line_start
            .checked_add(offset)
            .context("Invalid block range")?;
        let end = start.checked_add(count).context("Invalid block range")?;
        Ok(start..end)
    }

    fn residual_sections(&self, raw: &[&str], assigned: &[bool]) -> Vec<ReviewSection> {
        let mut sections = Vec::new();
        let mut by_file = vec![Vec::new(); self.patch.files.len()];
        for (index, info) in self.patch.lines.iter().enumerate() {
            if let Some(file) = info.file {
                by_file[file].push(index);
            }
        }
        for (file_index, file) in self.patch.files.iter().enumerate() {
            let mut file_sections = Vec::new();
            let mut lines = Vec::new();
            let mut section_index = 0;
            let mut seen_hunk = false;
            for &patch_line in &by_file[file_index] {
                if assigned[patch_line] {
                    continue;
                }
                let info = &self.patch.lines[patch_line];
                if info.kind == PatchLineKind::HunkHeader && seen_hunk {
                    append_residual(
                        &mut file_sections,
                        file,
                        file_index,
                        section_index,
                        &mut lines,
                    );
                    section_index += 1;
                }
                seen_hunk |= info.kind == PatchLineKind::HunkHeader;
                lines.push(section_line(&self.patch, raw, patch_line));
            }
            let has_file_metadata = file.binary
                || file.status != "modified"
                || file.old_path.is_some()
                || (file.additions == 0 && file.deletions == 0)
                || by_file[file_index].iter().any(|&index| {
                    let kind = self.patch.lines[index].kind;
                    kind == PatchLineKind::Meta
                        || (kind == PatchLineKind::FileHeader && !is_plain_file_header(raw[index]))
                });
            append_residual(
                &mut file_sections,
                file,
                file_index,
                section_index,
                &mut lines,
            );
            if !has_file_metadata {
                file_sections.retain(|section| {
                    section.lines.iter().any(|line| {
                        matches!(line.kind, PatchLineKind::Added | PatchLineKind::Removed)
                    })
                });
            }
            sections.extend(file_sections);
        }
        sections
    }
}

impl FrozenSource {
    pub fn line(&self, number: usize) -> Option<&str> {
        self.windows.iter().find_map(|window| {
            number
                .checked_sub(window.start_line)
                .and_then(|offset| window.lines.get(offset))
                .map(String::as_str)
        })
    }
}

fn comparison_target_tree<'repo>(
    repo: &'repo Repository,
    spec: &str,
) -> Result<Option<Tree<'repo>>> {
    if spec.ends_with("...WORKTREE") || spec.ends_with("..WORKTREE") {
        return Ok(None);
    }
    let (_, target) = spec
        .split_once("...")
        .or_else(|| spec.split_once(".."))
        .context("Unsupported comparison spec for source capture")?;
    let oid = super::resolve_commit_oid(repo, target.trim())?;
    Ok(Some(repo.find_commit(oid)?.tree()?))
}

fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// An absent endpoint has a verifiable identity; an unreadable one does not.
struct CapturedSource {
    bytes: Option<Vec<u8>>,
    identity: Option<String>,
}
impl CapturedSource {
    fn missing() -> Self {
        Self {
            bytes: None,
            identity: Some("absent".into()),
        }
    }
    fn unavailable() -> Self {
        Self {
            bytes: None,
            identity: None,
        }
    }
    fn bytes(bytes: Vec<u8>) -> Self {
        let identity = Some(format!("sha256:{:x}", Sha256::digest(&bytes)));
        Self {
            bytes: Some(bytes),
            identity,
        }
    }
}

fn capture_from_tree(repo: &Repository, tree: &Tree<'_>, path: &str) -> Result<CapturedSource> {
    let entry = match tree.get_path(Path::new(path)) {
        Ok(entry) => entry,
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            return Ok(CapturedSource::missing())
        }
        Err(_) => return Ok(CapturedSource::unavailable()),
    };
    let Ok(blob) = repo.find_blob(entry.id()) else {
        return Ok(CapturedSource::unavailable());
    };
    if blob.size() > MAX_SOURCE_READ_BYTES {
        return Ok(CapturedSource::unavailable());
    }
    Ok(CapturedSource::bytes(blob.content().to_vec()))
}

fn capture_from_worktree(root: &Path, relative: &str) -> Result<CapturedSource> {
    let path = root.join(relative);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CapturedSource::missing())
        }
        Err(_) => return Ok(CapturedSource::unavailable()),
    };
    // Do not follow an intermediate symlink outside this worktree.
    let canonical_root = fs::canonicalize(root)?;
    if !path
        .parent()
        .and_then(|parent| fs::canonicalize(parent).ok())
        .is_some_and(|parent| parent.starts_with(canonical_root))
    {
        return Ok(CapturedSource::unavailable());
    }
    if metadata.file_type().is_symlink() {
        return Ok(match fs::read_link(path) {
            Ok(target) => CapturedSource::bytes(target.as_os_str().as_encoded_bytes().to_vec()),
            Err(_) => CapturedSource::unavailable(),
        });
    }
    if !metadata.file_type().is_file() || metadata.len() > MAX_SOURCE_READ_BYTES as u64 {
        return Ok(CapturedSource::unavailable());
    }
    let Ok(file) = File::open(path) else {
        return Ok(CapturedSource::unavailable());
    };
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_READ_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SOURCE_READ_BYTES {
        return Ok(CapturedSource::unavailable());
    }
    Ok(CapturedSource::bytes(bytes))
}

fn source_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect()
}

fn verify_changed_lines(
    patch: &ReviewPatch,
    raw: &[&str],
    file_lines: &[usize],
    is_old: bool,
    source: &[String],
    path: &str,
) -> Result<()> {
    for &index in file_lines {
        let info = &patch.lines[index];
        let source_line = match (is_old, info.kind) {
            (true, PatchLineKind::Removed | PatchLineKind::Context) => info.old_line,
            (false, PatchLineKind::Added | PatchLineKind::Context) => info.new_line,
            _ => None,
        };
        let Some(source_line) = source_line else {
            continue;
        };
        let expected = raw[index].get(1..).unwrap_or("");
        ensure!(
            source
                .get(source_line.saturating_sub(1))
                .map(String::as_str)
                == Some(expected),
            "{path} changed while capturing the diff; call read_diff again"
        );
    }
    Ok(())
}

fn freeze_source(
    path: &str,
    lines: &[String],
    changed: &[usize],
    remaining: &mut usize,
) -> Option<FrozenSource> {
    let full_bytes = lines.iter().map(|line| line.len() + 1).sum::<usize>();
    if full_bytes <= *remaining {
        *remaining -= full_bytes;
        return Some(FrozenSource {
            path: path.to_string(),
            windows: vec![SourceWindow {
                start_line: 1,
                lines: lines.to_vec(),
            }],
            complete: true,
        });
    }
    if changed.is_empty() || *remaining == 0 {
        return None;
    }

    let mut ranges: Vec<(usize, usize)> = changed
        .iter()
        .map(|&number| {
            (
                number.saturating_sub(CONTEXT_RADIUS + 1),
                (number + CONTEXT_RADIUS).min(lines.len()),
            )
        })
        .collect();
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    let mut windows = Vec::new();
    for (start, end) in merged {
        let mut captured = Vec::new();
        for line in &lines[start..end] {
            let size = line.len() + 1;
            if size > *remaining {
                break;
            }
            captured.push(line.clone());
            *remaining -= size;
        }
        if !captured.is_empty() {
            windows.push(SourceWindow {
                start_line: start + 1,
                lines: captured,
            });
        }
        if *remaining == 0 {
            break;
        }
    }
    (!windows.is_empty()).then(|| FrozenSource {
        path: path.to_string(),
        windows,
        complete: false,
    })
}

fn is_plain_file_header(line: &str) -> bool {
    line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
}

impl ReviewSection {
    /// Presentation-only omissions, keyed by canonical patch line. Never alter
    /// the saved sections: coverage validation and replay still own every edit.
    pub fn equal_change_lines(&self) -> std::collections::HashSet<usize> {
        let mut hidden = std::collections::HashSet::new();
        if self.old_path.is_none() || self.new_path.is_none() {
            return hidden;
        }
        // Compare each replacement independently; context and metadata are hard
        // boundaries. A deadline bounds pathological comparisons conservatively.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
        for run in self
            .lines
            .split(|line| !matches!(line.kind, PatchLineKind::Added | PatchLineKind::Removed))
        {
            let old: Vec<_> = run
                .iter()
                .filter(|line| line.kind == PatchLineKind::Removed)
                .collect();
            let new: Vec<_> = run
                .iter()
                .filter(|line| line.kind == PatchLineKind::Added)
                .collect();
            if old.is_empty() || new.is_empty() {
                continue;
            }
            let normalize = |line: &&ReviewSectionLine| {
                line.text
                    .chars()
                    .filter(|ch| !matches!(ch, ' ' | '\t' | '\r' | '\x0b' | '\x0c'))
                    .collect::<String>()
            };
            let before: Vec<_> = old.iter().map(normalize).collect();
            let after: Vec<_> = new.iter().map(normalize).collect();
            let before: Vec<_> = before.iter().map(String::as_str).collect();
            let after: Vec<_> = after.iter().map(String::as_str).collect();
            let diff = similar::TextDiff::configure()
                .deadline(deadline)
                .diff_slices(&before, &after);
            for change in diff
                .iter_all_changes()
                .filter(|change| change.tag() == similar::ChangeTag::Equal)
            {
                if let (Some(a), Some(b)) = (change.old_index(), change.new_index()) {
                    if before[a] == after[b] {
                        hidden.insert(old[a].source_patch_line);
                        hidden.insert(new[b].source_patch_line);
                    }
                }
            }
        }
        hidden
    }
}

impl CustomReview {
    /// Check that rendered changed lines retain their canonical identity and content.
    pub fn validate_coverage(&self) -> Result<()> {
        let mut counts = vec![0u8; self.snapshot.patch.lines.len()];
        let raw: Vec<&str> = self.snapshot.patch.text.lines().collect();
        for section in &self.sections {
            for line in &section.lines {
                let source = self
                    .snapshot
                    .patch
                    .lines
                    .get(line.source_patch_line)
                    .context("Review section references a missing patch line")?;
                ensure!(
                    source.kind == line.kind
                        && raw.get(line.source_patch_line).is_some()
                        && *line
                            == section_line(&self.snapshot.patch, &raw, line.source_patch_line),
                    "Review section changed a canonical patch line"
                );
                if matches!(line.kind, PatchLineKind::Added | PatchLineKind::Removed) {
                    let file = source
                        .file
                        .and_then(|file| self.snapshot.patch.files.get(file))
                        .context("Changed source has no file")?;
                    let correct_path = if line.kind == PatchLineKind::Removed {
                        section.old_path.as_deref() == Some(old_path(file).as_str())
                    } else {
                        section.new_path.as_deref() == Some(file.path.as_str())
                    };
                    ensure!(
                        correct_path,
                        "Review section changed a canonical source path"
                    );
                    counts[line.source_patch_line] =
                        counts[line.source_patch_line].saturating_add(1);
                }
            }
        }
        for (index, line) in self.snapshot.patch.lines.iter().enumerate() {
            if matches!(line.kind, PatchLineKind::Added | PatchLineKind::Removed) {
                ensure!(
                    counts[index] == 1,
                    "Changed patch line {index} appears {} times in the custom review",
                    counts[index]
                );
            }
        }
        Ok(())
    }
}

fn old_path(file: &DiffFile) -> String {
    file.old_path.clone().unwrap_or_else(|| file.path.clone())
}

fn section_line(patch: &ReviewPatch, raw: &[&str], index: usize) -> ReviewSectionLine {
    let source = patch.lines[index];
    let text = raw[index];
    let text = if matches!(
        source.kind,
        PatchLineKind::Added | PatchLineKind::Removed | PatchLineKind::Context
    ) {
        text.get(1..).unwrap_or("")
    } else {
        text
    };
    ReviewSectionLine {
        kind: source.kind,
        text: text.to_string(),
        old_line: source.old_line,
        new_line: source.new_line,
        source_patch_line: index,
    }
}

fn append_residual(
    sections: &mut Vec<ReviewSection>,
    file: &DiffFile,
    file_index: usize,
    section_index: usize,
    lines: &mut Vec<ReviewSectionLine>,
) {
    if lines.is_empty() {
        return;
    }
    sections.push(ReviewSection {
        id: format!("residual_{file_index}_{section_index}"),
        label: None,
        message: None,
        old_path: Some(old_path(file)),
        new_path: Some(file.path.clone()),
        is_reassigned: false,
        lines: std::mem::take(lines),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_diff::{BaseKind, PatchLine};
    use git2::{IndexAddOption, Repository, Signature};
    use std::fs;
    use std::path::PathBuf;

    fn fixture() -> ReviewPatch {
        let entries = [
            (
                "diff --git a/old.rs b/old.rs",
                PatchLineKind::FileHeader,
                0,
                None,
                None,
            ),
            (
                "@@ -1,2 +0,0 @@",
                PatchLineKind::HunkHeader,
                0,
                Some(1),
                Some(1),
            ),
            ("-alpha", PatchLineKind::Removed, 0, Some(1), Some(1)),
            ("-beta", PatchLineKind::Removed, 0, Some(2), Some(1)),
            (
                "diff --git a/new.rs b/new.rs",
                PatchLineKind::FileHeader,
                1,
                None,
                None,
            ),
            (
                "@@ -0,0 +1,2 @@",
                PatchLineKind::HunkHeader,
                1,
                Some(0),
                Some(1),
            ),
            ("+alpha", PatchLineKind::Added, 1, None, Some(1)),
            ("+beta", PatchLineKind::Added, 1, None, Some(2)),
        ];
        ReviewPatch {
            root: PathBuf::from("/tmp/review"),
            head: "feature".into(),
            base: ReviewBase {
                name: "main".into(),
                spec: "main...WORKTREE".into(),
                kind: BaseKind::Explicit,
                remote: None,
                fetched_ago: None,
                ever_fetched: false,
            },
            merge_base: None,
            comparison_base_oid: "0123456789abcdef0123456789abcdef01234567".into(),
            ahead: 0,
            behind: 0,
            files: vec![
                DiffFile {
                    path: "old.rs".into(),
                    old_path: None,
                    status: "modified".into(),
                    additions: 0,
                    deletions: 2,
                    binary: false,
                },
                DiffFile {
                    path: "new.rs".into(),
                    old_path: None,
                    status: "added".into(),
                    additions: 2,
                    deletions: 0,
                    binary: false,
                },
            ],
            text: entries
                .iter()
                .map(|entry| format!("{}\n", entry.0))
                .collect(),
            lines: entries
                .iter()
                .map(|(_, kind, file, old_line, new_line)| PatchLine {
                    kind: *kind,
                    file: Some(*file),
                    old_line: *old_line,
                    new_line: *new_line,
                })
                .collect(),
            truncated: false,
        }
    }

    fn reference(id: &str, offset: usize, count: usize) -> ChangeRef {
        ChangeRef {
            block_id: id.into(),
            offset: Some(offset),
            count: Some(count),
        }
    }

    #[test]
    fn cross_file_reassignment_preserves_every_changed_line() {
        let snapshot = ReviewSnapshot::from_patch(fixture()).unwrap();
        assert_eq!(snapshot.blocks.len(), 2);
        let pair = DiffPairing {
            message: None,
            label: Some("Moved alpha".into()),
            old: reference(&snapshot.blocks[0].id, 0, 1).into(),
            new: reference(&snapshot.blocks[1].id, 0, 1).into(),
            related_to: None,
        };
        let review = snapshot.reassign(&[pair]).unwrap();
        assert_eq!(review.sections[0].old_path.as_deref(), Some("old.rs"));
        assert_eq!(review.sections[0].new_path.as_deref(), Some("new.rs"));
        assert_eq!(review.sections[0].lines[0].source_patch_line, 2);
        assert_eq!(review.sections[0].lines[1].source_patch_line, 6);
        assert!(review
            .sections
            .iter()
            .any(|section| { section.lines.iter().any(|line| line.source_patch_line == 3) }));
        assert!(review
            .sections
            .iter()
            .any(|section| { section.lines.iter().any(|line| line.source_patch_line == 7) }));
        review.validate_coverage().unwrap();
        let replay: CustomReview =
            serde_json::from_str(&serde_json::to_string(&review).unwrap()).unwrap();
        assert_eq!(replay, review);
        replay.validate_coverage().unwrap();
    }

    #[test]
    fn section_messages_are_metadata_and_survive_round_trip() {
        let snapshot = ReviewSnapshot::from_patch(fixture()).unwrap();
        let mut pairing = DiffPairing {
            label: Some("Explain the move".into()),
            message: Some("This replaces the older path.\nReview the caller too.".into()),
            old: Some(reference(&snapshot.blocks[0].id, 0, 1)),
            new: Some(reference(&snapshot.blocks[1].id, 0, 1)),
            related_to: None,
        };
        let review = snapshot.reassign(&[pairing.clone()]).unwrap();
        assert_eq!(review.sections[0].message, pairing.message);
        assert_eq!(review.snapshot.patch, snapshot.patch);
        let saved: DiffPairing =
            serde_json::from_str(&serde_json::to_string(&pairing).unwrap()).unwrap();
        assert_eq!(saved.message, pairing.message);

        for invalid in ["   ", "unsafe\u{1b}control"] {
            pairing.message = Some(invalid.into());
            assert!(snapshot.reassign(&[pairing.clone()]).is_err());
        }
        pairing.message = Some("x".repeat(2001));
        assert!(snapshot.reassign(&[pairing]).is_err());
        let legacy: DiffPairing =
            serde_json::from_str("{\"label\":null,\"old\":{\"block_id\":\"old\"}}").unwrap();
        assert_eq!(legacy.message, None);
    }

    #[test]
    fn rejects_overlapping_and_invalid_pairings() {
        let snapshot = ReviewSnapshot::from_patch(fixture()).unwrap();
        let old = &snapshot.blocks[0].id;
        let new = &snapshot.blocks[1].id;
        let pair = DiffPairing {
            message: None,
            label: None,
            old: reference(old, 0, 1).into(),
            new: reference(new, 0, 1).into(),
            related_to: None,
        };
        assert!(snapshot.reassign(&[pair.clone(), pair]).is_err());
        assert!(snapshot
            .reassign(&[DiffPairing {
                message: None,
                label: None,
                old: reference(new, 0, 1).into(),
                new: reference(old, 0, 1).into(),

                related_to: None,
            }])
            .is_err());
        assert!(snapshot
            .reassign(&[DiffPairing {
                message: None,
                label: None,
                old: reference(old, 1, 2).into(),
                new: reference(new, 0, 1).into(),

                related_to: None,
            }])
            .is_err());
    }

    #[test]
    fn labelled_slices_and_shared_references_preserve_exact_ownership() {
        let mut patch = fixture();
        patch.text = patch.text.replace("+beta", "+alpha");
        let snapshot = ReviewSnapshot::from_patch(patch).unwrap();
        let old = &snapshot.blocks[0].id;
        let new = &snapshot.blocks[1].id;
        let specs = vec![
            DiffPairing {
                message: None,
                label: Some("Remove duplicate".into()),
                old: Some(reference(old, 1, 1)),
                new: None,
                related_to: Some(reference(old, 0, 1)),
            },
            DiffPairing {
                message: None,
                label: Some("Primary move".into()),
                old: Some(reference(old, 0, 1)),
                new: Some(reference(new, 0, 1)),
                related_to: None,
            },
            DiffPairing {
                message: None,
                label: Some("Additional copy".into()),
                old: None,
                new: Some(reference(new, 1, 1)),
                related_to: Some(reference(old, 0, 1)),
            },
        ];
        let review = snapshot.reassign(&specs).unwrap();
        review.validate_coverage().unwrap();
        let owned = review
            .sections
            .iter()
            .flat_map(|s| &s.lines)
            .filter(|l| matches!(l.kind, PatchLineKind::Added | PatchLineKind::Removed))
            .map(|l| l.source_patch_line)
            .collect::<Vec<_>>();
        assert_eq!(owned, vec![3, 2, 6, 7]);
        assert!(review.sections[0].new_path.is_none());
        assert!(review.sections[2].old_path.is_none());
        assert!(review.sections[0]
            .label
            .as_ref()
            .unwrap()
            .contains("Related source (before): old.rs:1"));
        assert!(review.sections[2]
            .label
            .as_ref()
            .unwrap()
            .contains("Related source (before): old.rs:1"));
        // References never grant eligibility for the equal-pair filter.
        assert!(review.sections[0].equal_change_lines().is_empty());
        assert!(review.sections[2].equal_change_lines().is_empty());
        assert_eq!(review.sections[1].equal_change_lines().len(), 2);
        let saved = crate::native_diff::store::SavedReview {
            title: "Review".into(),
            snapshot,
            pairings: specs,
        };
        let replay: crate::native_diff::store::SavedReview =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(replay.into_custom().unwrap().1, review);
        for change in 0..4 {
            let mut broken = review.clone();
            match change {
                0 => {
                    broken.sections[0].lines.clear();
                }
                1 => {
                    let duplicate = broken.sections[0].lines[0].clone();
                    broken.sections[0].lines.push(duplicate);
                }
                2 => broken.sections[0].lines[0].text = "invented".into(),
                _ => broken.sections[0].old_path = Some("unrelated.rs".into()),
            }
            assert!(broken.validate_coverage().is_err());
        }
    }

    #[test]
    fn one_sided_sections_reject_empty_wrong_side_overlap_and_invalid_references() {
        let snapshot = ReviewSnapshot::from_patch(fixture()).unwrap();
        let old = &snapshot.blocks[0].id;
        let new = &snapshot.blocks[1].id;
        let deletion = DiffPairing {
            message: None,
            label: None,
            old: Some(reference(old, 0, 1)),
            new: None,
            related_to: None,
        };
        for spec in [
            DiffPairing {
                old: None,
                ..deletion.clone()
            },
            DiffPairing {
                old: Some(reference(new, 0, 1)),
                ..deletion.clone()
            },
            DiffPairing {
                old: None,
                new: Some(reference(old, 0, 1)),
                ..deletion.clone()
            },
            DiffPairing {
                related_to: Some(reference(old, 0, 1)),
                ..deletion.clone()
            },
            DiffPairing {
                related_to: Some(reference(new, 2, 1)),
                ..deletion.clone()
            },
            DiffPairing {
                related_to: Some(reference(new, 0, 0)),
                ..deletion.clone()
            },
            DiffPairing {
                related_to: Some(reference("invented", 0, 1)),
                ..deletion.clone()
            },
        ] {
            assert!(snapshot.reassign(&[spec]).is_err());
        }
        assert!(snapshot
            .reassign(&[deletion.clone(), deletion.clone()])
            .is_err());
        let paired = DiffPairing {
            message: None,
            new: Some(reference(new, 0, 1)),
            ..deletion.clone()
        };
        assert!(snapshot.reassign(&[deletion.clone(), paired]).is_err());
        // Leaving everything else unassigned still retains the entire snapshot.
        let review = snapshot.reassign(&[deletion]).unwrap();
        review.validate_coverage().unwrap();
        assert_eq!(
            review
                .sections
                .iter()
                .flat_map(|s| &s.lines)
                .filter(|l| l.kind == PatchLineKind::Added)
                .count(),
            2
        );
    }

    #[test]
    fn refuses_incomplete_snapshots() {
        let mut patch = fixture();
        patch.truncated = true;
        assert!(ReviewSnapshot::from_patch(patch).is_err());
        let mut patch = fixture();
        patch.lines.pop();
        assert!(ReviewSnapshot::from_patch(patch).is_err());
        let mut patch = fixture();
        patch.text = patch.text.replacen(
            "diff --git a/old.rs b/old.rs",
            &format!("diff --git {}", "x".repeat(super::super::MAX_PATCH_BYTES)),
            1,
        );
        assert!(ReviewSnapshot::from_patch(patch).is_err());
    }

    #[test]
    fn retains_mode_metadata_after_all_text_is_reassigned() {
        let mut patch = fixture();
        patch.text = patch.text.replacen(
            "diff --git a/old.rs b/old.rs\n",
            "diff --git a/old.rs b/old.rs\nold mode 100644\n",
            1,
        );
        patch.lines.insert(
            1,
            PatchLine {
                kind: PatchLineKind::FileHeader,
                file: Some(0),
                old_line: None,
                new_line: None,
            },
        );
        let snapshot = ReviewSnapshot::from_patch(patch).unwrap();
        let review = snapshot
            .reassign(&[DiffPairing {
                message: None,
                label: None,
                old: reference(&snapshot.blocks[0].id, 0, 2).into(),
                new: reference(&snapshot.blocks[1].id, 0, 2).into(),

                related_to: None,
            }])
            .unwrap();
        assert!(review.sections.iter().any(|section| {
            section
                .lines
                .iter()
                .any(|line| line.text == "old mode 100644")
        }));
    }

    #[test]
    fn snapshot_matches_git_review_with_staged_unstaged_and_untracked_moves() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let unchanged = (0..20)
            .map(|number| format!("retained {number}\n"))
            .collect::<String>();
        fs::write(
            temp.path().join("old.rs"),
            format!("alpha\nbeta\n{unchanged}"),
        )
        .unwrap();
        let mut index = repo.index().unwrap();
        index.add_all(["*"], IndexAddOption::DEFAULT, None).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = Signature::now("Ovim", "ovim@example.com").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
        drop(tree);

        fs::write(temp.path().join("old.rs"), &unchanged).unwrap();
        fs::write(temp.path().join("new.rs"), "alpha refactored\n").unwrap();
        fs::write(temp.path().join("staged.rs"), "staged change\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("new.rs")).unwrap();
        index.add_path(Path::new("staged.rs")).unwrap();
        index.write().unwrap();
        fs::write(temp.path().join("new.rs"), "alpha refactored\nbeta moved\n").unwrap();
        fs::write(temp.path().join("untracked.rs"), "untracked change\n").unwrap();

        let base = super::super::resolve_pullbase(temp.path(), Some("main")).unwrap();
        let regular = review_patch(temp.path(), &base).unwrap();
        let snapshot = review_snapshot(temp.path(), &base).unwrap();
        assert_eq!(snapshot.patch, regular);
        assert_eq!(snapshot.patch.comparison_base_oid.len(), 40);
        assert_eq!(
            snapshot
                .source_for("old.rs", "old")
                .and_then(|source| source.line(1)),
            Some("alpha")
        );
        assert_eq!(
            snapshot
                .source_for("new.rs", "new")
                .and_then(|source| source.line(2)),
            Some("beta moved")
        );
        fs::write(temp.path().join("new.rs"), "later edit\n").unwrap();
        assert_eq!(
            snapshot
                .source_for("new.rs", "new")
                .and_then(|source| source.line(2)),
            Some("beta moved")
        );
        for name in ["old.rs", "new.rs", "staged.rs", "untracked.rs"] {
            assert!(snapshot.patch.files.iter().any(|file| file.path == name));
        }
        let old = snapshot
            .blocks
            .iter()
            .find(|block| {
                block.kind == PatchLineKind::Removed
                    && snapshot.patch.files[block.file].path == "old.rs"
            })
            .unwrap();
        let new = snapshot
            .blocks
            .iter()
            .find(|block| {
                block.kind == PatchLineKind::Added
                    && snapshot.patch.files[block.file].path == "new.rs"
            })
            .unwrap();
        let review = snapshot
            .reassign(&[DiffPairing {
                message: None,
                label: Some("Move and edit".into()),
                old: reference(&old.id, 0, old.line_count).into(),
                new: reference(&new.id, 0, new.line_count).into(),

                related_to: None,
            }])
            .unwrap();
        review.validate_coverage().unwrap();
        assert_eq!(review.sections[0].old_path.as_deref(), Some("old.rs"));
        assert_eq!(review.sections[0].new_path.as_deref(), Some("new.rs"));
        assert_eq!(review.snapshot.patch.additions(), regular.additions());
        assert_eq!(review.snapshot.patch.deletions(), regular.deletions());
    }

    #[test]
    fn large_source_uses_frozen_windows_with_visible_gaps() {
        let lines = (1..=1000)
            .map(|number| format!("source line {number:04}"))
            .collect::<Vec<_>>();
        let mut budget = 3_000;
        let source = freeze_source("large.rs", &lines, &[500], &mut budget).unwrap();
        assert!(!source.complete);
        assert_eq!(source.line(500), Some("source line 0500"));
        assert_eq!(source.line(1), None);
        assert!(source.windows[0].start_line > 1);
    }

    #[test]
    fn explicit_comparison_captures_right_tree_instead_of_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        fs::write(temp.path().join("file.rs"), "old\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.rs")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = Signature::now("Ovim", "ovim@example.com").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "base", &tree, &[])
            .unwrap();
        drop(tree);
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        fs::write(temp.path().join("file.rs"), "new in commit\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.rs")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "feature",
            &tree,
            &[&head],
        )
        .unwrap();
        drop(tree);
        fs::write(temp.path().join("file.rs"), "uncommitted later edit\n").unwrap();

        let base = ReviewBase::explicit("main..feature");
        let snapshot = review_snapshot(temp.path(), &base).unwrap();
        assert_eq!(
            snapshot
                .source_for("file.rs", "new")
                .and_then(|source| source.line(1)),
            Some("new in commit")
        );
    }
}

#[cfg(test)]
mod equal_change_tests {
    use super::*;

    fn paired(old: &[&str], new: &[&str]) -> ReviewSection {
        ReviewSection {
            message: None,
            id: "pair".into(),
            label: None,
            old_path: Some("src/a.rs".into()),
            new_path: Some("src/a.rs".into()),
            is_reassigned: true,
            lines: old
                .iter()
                .map(|text| (PatchLineKind::Removed, *text))
                .chain(new.iter().map(|text| (PatchLineKind::Added, *text)))
                .enumerate()
                .map(|(index, (kind, text))| ReviewSectionLine {
                    kind,
                    text: text.into(),
                    old_line: None,
                    new_line: None,
                    source_patch_line: index,
                })
                .collect(),
        }
    }

    #[test]
    fn mixed_pairs_keep_real_edits_and_insertions_in_order() {
        let section = paired(
            &["same();", "old();", "end();"],
            &[" same( );", "new();", "extra();", "end();"],
        );
        let hidden = section.equal_change_lines();
        assert_eq!(hidden, [0, 2, 3, 6].into_iter().collect());
        let mut cross_file = section;
        cross_file.new_path = Some("src/b.rs".into());
        assert_eq!(cross_file.equal_change_lines(), hidden);
    }

    #[test]
    fn context_is_a_boundary_and_unpaired_or_reordered_lines_remain_visible() {
        assert!(paired(&[], &["  "]).equal_change_lines().is_empty());
        assert!(paired(&["gone"], &[]).equal_change_lines().is_empty());
        assert!(paired(&["a", "b"], &["b", "a"]).equal_change_lines().len() < 4);
        let mut section = paired(&["same"], &["same"]);
        section.lines.insert(
            1,
            ReviewSectionLine {
                kind: PatchLineKind::Context,
                text: "boundary".into(),
                old_line: None,
                new_line: None,
                source_patch_line: 2,
            },
        );
        assert!(section.equal_change_lines().is_empty());
    }
}
