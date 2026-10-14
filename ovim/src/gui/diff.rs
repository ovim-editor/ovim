//! Structured diff data for the native GUI. The review patch remains owned by
//! core; this projection only replaces the terminal's width-dependent paint.

use serde::{Serialize, Serializer};
use std::collections::BTreeMap;
use std::path::{Component, Path};
use std::sync::Arc;

use crate::buffer::Buffer;
use crate::editor::{DiffLayout, DiffReviewState, Editor, DIFF_REVIEW_TITLE_PREFIX};
use ovim_core::native_diff::{ChangeRef, CustomReview, PatchLineKind, ReviewPatch};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiDiffReview {
    pub comments: Vec<ovim_core::editor::DiffComment>,
    pub title: String,
    pub layout: &'static str,
    pub managed: bool,
    pub custom: bool,
    pub show_checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<GuiDiffProvenance>,
    pub files: Vec<GuiDiffFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay: Option<GuiDiffOverlayState>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub moves: Vec<GuiDiffMove>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub guided_files: Vec<GuiDiffFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiDiffProvenance {
    pub base_label: String,
    pub comparison_base_oid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiDiffOverlayState {
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiDiffMove {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub old: GuiDiffMoveEndpoint,
    pub new: GuiDiffMoveEndpoint,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiDiffMoveEndpoint {
    pub path: String,
    pub start_line: usize,
    pub line_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_line: Option<usize>,
    pub context_windows: Vec<GuiDiffContextWindow>,
    pub context_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiDiffContextWindow {
    pub start_line: usize,
    pub lines: Vec<GuiDiffLine>,
}

/// A cached projection shared by successive snapshots. Cursor movement does
/// not need to clone a multi-megabyte patch just to send the next snapshot.
#[derive(Debug, Clone)]
pub struct GuiDiffDocument(pub Arc<GuiDiffReview>);

impl PartialEq for GuiDiffDocument {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl Serialize for GuiDiffDocument {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_ref().serialize(serializer)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiDiffFile {
    pub id: String,
    pub checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
    pub metadata: Vec<String>,
    pub hunks: Vec<GuiDiffHunk>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiDiffHunk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ovim_core::native_diff::context::DiffContextView>,
    pub header: String,
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_line: Option<usize>,
    pub lines: Vec<GuiDiffLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiDiffLine {
    pub kind: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_line: Option<usize>,
    pub highlights: Vec<GuiDiffHighlight>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiDiffHighlight {
    pub start: usize,
    pub end: usize,
    pub token: String,
}

pub fn project_diff(editor: &Editor, buffer: &Buffer) -> Option<GuiDiffReview> {
    let title = buffer.display_name()?.to_string();
    if let Some(review) = editor
        .diff_review()
        .filter(|state| state.buffer_id == buffer.id())
    {
        let mut projected = if let Some(custom) = review.custom() {
            project_custom_review(&title, review, custom)
        } else {
            project_review_patch(&title, review)
        };
        projected.overlay = editor
            .diff_review_overlay_state()
            .and_then(|state| match state.mode {
                "none" => None,
                mode => Some(GuiDiffOverlayState {
                    mode,
                    title: state.title,
                }),
            });
        return Some(projected);
    }
    let path = title.strip_prefix(DIFF_REVIEW_TITLE_PREFIX)?;
    Some(project_standalone(&title, path, &buffer.rope().to_string()))
}

fn project_review_patch(title: &str, review: &DiffReviewState) -> GuiDiffReview {
    let patch: &ReviewPatch = review.patch();
    let review_lines = review.patch_review_lines();
    let mut files: Vec<GuiDiffFile> = patch
        .files
        .iter()
        .map(|file| GuiDiffFile {
            id: file.path.clone(),
            checked: review
                .checked_items
                .contains(&format!("file:{}", file.path)),
            label: None,
            message: None,
            path: file.path.clone(),
            old_path: file.old_path.clone(),
            status: file.status.clone(),
            additions: file.additions,
            deletions: file.deletions,
            binary: file.binary,
            metadata: Vec::new(),
            hunks: Vec::new(),
        })
        .collect();

    for (index, (text, info)) in patch.text.lines().zip(&patch.lines).enumerate() {
        let Some(file) = info.file.and_then(|index| files.get_mut(index)) else {
            continue;
        };
        match info.kind {
            PatchLineKind::HunkHeader => {
                let (old_start, old_count, new_start, new_count) =
                    parse_hunk_header(text).unwrap_or((0, 0, 0, 0));
                file.hunks.push(GuiDiffHunk {
                    context: review.context_view(&format!("hunk:{index}")),
                    header: text.to_string(),
                    old_start,
                    old_count,
                    new_start,
                    new_count,
                    review_line: review_lines.get(index).copied().flatten(),
                    lines: Vec::new(),
                });
            }
            PatchLineKind::Context | PatchLineKind::Added | PatchLineKind::Removed => {
                let Some(hunk) = file.hunks.last_mut() else {
                    continue;
                };
                hunk.lines.push(GuiDiffLine {
                    kind: kind_name(info.kind),
                    text: text.get(1..).unwrap_or("").to_string(),
                    old_line: info.old_line,
                    new_line: info.new_line,
                    review_line: review_lines.get(index).copied().flatten(),
                    highlights: project_highlights(
                        text.get(1..).unwrap_or(""),
                        review.patch_line_highlights(index),
                    ),
                });
            }
            PatchLineKind::FileHeader | PatchLineKind::Meta => {
                if is_visible_metadata(text) {
                    file.metadata.push(text.to_string());
                }
            }
        }
    }

    GuiDiffReview {
        comments: review.comments().to_vec(),
        title: title.to_string(),
        layout: layout_name(review.layout),
        managed: true,
        custom: false,
        show_checked: review.show_checked,
        provenance: Some(GuiDiffProvenance {
            base_label: patch.base.name.clone(),
            comparison_base_oid: patch.comparison_base_oid.clone(),
            snapshot_id: None,
        }),
        files,
        overlay: None,
        moves: Vec::new(),
        guided_files: Vec::new(),
    }
}

const MOVE_CONTEXT_RADIUS: usize = 200;
const MAX_MOVE_CONTEXT_LINES: usize = 500;
const MAX_MOVE_CONTEXT_BYTES: usize = 128 * 1024;

fn project_custom_review(
    title: &str,
    review: &DiffReviewState,
    custom: &CustomReview,
) -> GuiDiffReview {
    // The main document always follows real files and their canonical hunks.
    // Pairings annotate that document; they do not replace files with sections.
    let mut projected = project_review_patch(title, review);
    projected.custom = true;
    if let Some(provenance) = &mut projected.provenance {
        provenance.snapshot_id = Some(custom.snapshot.id.clone());
    }
    projected.guided_files = project_guided_files(review, custom);
    let review_lines = review.patch_review_lines();
    let patch_lines = custom.snapshot.patch.text.lines().collect::<Vec<_>>();
    projected.moves = custom
        .pairings
        .iter()
        .enumerate()
        .filter_map(|(index, pairing)| {
            let old = project_move_endpoint(
                review,
                custom,
                &review_lines,
                &patch_lines,
                pairing.old.as_ref()?,
                PatchLineKind::Removed,
            )?;
            let new = project_move_endpoint(
                review,
                custom,
                &review_lines,
                &patch_lines,
                pairing.new.as_ref()?,
                PatchLineKind::Added,
            )?;
            Some(GuiDiffMove {
                id: format!("pair_{index}"),
                label: custom
                    .sections
                    .get(index)
                    .and_then(|section| section.label.clone()),
                old,
                new,
            })
        })
        .collect();
    projected
}

fn project_guided_files(review: &DiffReviewState, custom: &CustomReview) -> Vec<GuiDiffFile> {
    let patch = review.patch();
    let review_lines = review.patch_review_lines();
    custom
        .sections
        .iter()
        .map(|section| {
            let path = section
                .new_path
                .as_deref()
                .or(section.old_path.as_deref())
                .unwrap_or("(metadata)")
                .to_string();
            let canonical = patch.files.iter().find(|file| {
                section.new_path.as_deref() == Some(file.path.as_str())
                    || section.old_path.as_deref() == Some(file.path.as_str())
                    || (section.old_path.is_some()
                        && section.old_path.as_deref() == file.old_path.as_deref())
            });
            let additions = section
                .lines
                .iter()
                .filter(|line| line.kind == PatchLineKind::Added)
                .count();
            let deletions = section
                .lines
                .iter()
                .filter(|line| line.kind == PatchLineKind::Removed)
                .count();
            let header = section
                .lines
                .iter()
                .find(|line| line.kind == PatchLineKind::HunkHeader)
                .map(|line| line.text.clone())
                .or_else(|| section.label.clone())
                .unwrap_or_else(|| {
                    format!(
                        "{} → {}",
                        section.old_path.as_deref().unwrap_or("∅"),
                        section.new_path.as_deref().unwrap_or("∅")
                    )
                });
            let old_start = section
                .lines
                .iter()
                .filter_map(|line| line.old_line)
                .min()
                .unwrap_or(0);
            let new_start = section
                .lines
                .iter()
                .filter_map(|line| line.new_line)
                .min()
                .unwrap_or(0);
            let lines: Vec<GuiDiffLine> = section
                .lines
                .iter()
                .filter(|line| {
                    matches!(
                        line.kind,
                        PatchLineKind::Context | PatchLineKind::Added | PatchLineKind::Removed
                    )
                })
                .map(|line| GuiDiffLine {
                    kind: kind_name(line.kind),
                    text: line.text.clone(),
                    old_line: (line.kind != PatchLineKind::Added)
                        .then_some(line.old_line)
                        .flatten(),
                    new_line: (line.kind != PatchLineKind::Removed)
                        .then_some(line.new_line)
                        .flatten(),
                    review_line: review_lines.get(line.source_patch_line).copied().flatten(),
                    highlights: project_highlights(
                        &line.text,
                        review.patch_line_highlights(line.source_patch_line),
                    ),
                })
                .collect();
            let hunks = if lines.is_empty() {
                Vec::new()
            } else {
                vec![GuiDiffHunk {
                    context: review.context_view(&format!("section:{}", section.id)),
                    header,
                    old_start,
                    old_count: lines.iter().filter(|line| line.old_line.is_some()).count(),
                    new_start,
                    new_count: lines.iter().filter(|line| line.new_line.is_some()).count(),
                    review_line: lines.iter().find_map(|line| line.review_line),
                    lines,
                }]
            };
            GuiDiffFile {
                id: section.id.clone(),
                checked: review
                    .checked_items
                    .contains(&format!("section:{}", section.id)),
                label: section.label.clone(),
                message: section.message.clone(),
                path,
                old_path: section.old_path.clone(),
                status: if section.is_reassigned {
                    match (section.old_path.is_some(), section.new_path.is_some()) {
                        (true, false) => "deletion",
                        (false, true) => "addition",
                        _ => "reassigned",
                    }
                    .to_string()
                } else {
                    canonical
                        .map(|file| file.status.clone())
                        .unwrap_or_else(|| "modified".to_string())
                },
                additions,
                deletions,
                binary: canonical.is_some_and(|file| file.binary),
                metadata: section
                    .lines
                    .iter()
                    .filter(|line| is_visible_metadata(&line.text))
                    .map(|line| line.text.clone())
                    .collect(),
                hunks,
            }
        })
        .collect()
}

fn project_move_endpoint(
    review: &DiffReviewState,
    custom: &CustomReview,
    review_lines: &[Option<usize>],
    patch_lines: &[&str],
    reference: &ChangeRef,
    kind: PatchLineKind,
) -> Option<GuiDiffMoveEndpoint> {
    let snapshot = &custom.snapshot;
    let block = snapshot
        .blocks
        .iter()
        .find(|block| block.id == reference.block_id)?;
    if block.kind != kind {
        return None;
    }
    let offset = reference.offset.unwrap_or(0);
    let count = reference
        .count
        .unwrap_or(block.line_count.checked_sub(offset)?);
    let start_line = block.start_line.checked_add(offset)?;
    let end_line = start_line.checked_add(count)?;
    let file = snapshot.patch.files.get(block.file)?;
    let path = if kind == PatchLineKind::Removed {
        file.old_path.as_deref().unwrap_or(&file.path)
    } else {
        &file.path
    };
    let first_patch_line = block.patch_line_start.checked_add(offset)?;
    let review_line = review_lines.get(first_patch_line).copied().flatten();

    // Changed rows always come from the canonical patch. Frozen source windows
    // add only surrounding context, so a partial capture never clips a move.
    let mut lines = BTreeMap::new();
    for index in 0..count {
        let patch_line = first_patch_line + index;
        let info = snapshot.patch.lines.get(patch_line)?;
        if info.kind != kind {
            return None;
        }
        let text = patch_lines.get(patch_line)?.get(1..)?.to_string();
        let number = start_line + index;
        lines.insert(
            number,
            GuiDiffLine {
                kind: kind_name(kind),
                highlights: project_highlights(&text, review.patch_line_highlights(patch_line)),
                text,
                old_line: (kind == PatchLineKind::Removed).then_some(number),
                new_line: (kind == PatchLineKind::Added).then_some(number),
                review_line: review_lines.get(patch_line).copied().flatten(),
            },
        );
    }

    let source = snapshot.source_for(
        path,
        if kind == PatchLineKind::Removed {
            "old"
        } else {
            "new"
        },
    );
    let mut context_cut = false;
    if let Some(source) = source {
        let lower = start_line.saturating_sub(MOVE_CONTEXT_RADIUS).max(1);
        let upper = end_line.saturating_add(MOVE_CONTEXT_RADIUS);
        let mut nearby = source
            .windows
            .iter()
            .flat_map(|window| {
                window.lines.iter().enumerate().filter_map(|(index, text)| {
                    let number = window.start_line + index;
                    (number >= lower
                        && number < upper
                        && (number < start_line || number >= end_line))
                        .then(|| {
                            let distance = if number < start_line {
                                start_line - number
                            } else {
                                number - end_line + 1
                            };
                            (distance, number, text)
                        })
                })
            })
            .collect::<Vec<_>>();
        nearby.sort_by_key(|(distance, number, _)| (*distance, *number));
        let mut remaining_lines = MAX_MOVE_CONTEXT_LINES;
        let mut remaining_bytes = MAX_MOVE_CONTEXT_BYTES;
        for (_, number, text) in nearby {
            if remaining_lines == 0 || text.len() > remaining_bytes {
                context_cut = true;
                if remaining_lines == 0 {
                    break;
                }
                continue;
            }
            remaining_lines -= 1;
            remaining_bytes -= text.len();
            lines.insert(
                number,
                GuiDiffLine {
                    kind: "context",
                    text: text.clone(),
                    old_line: (kind == PatchLineKind::Removed).then_some(number),
                    new_line: (kind == PatchLineKind::Added).then_some(number),
                    review_line: None,
                    highlights: Vec::new(),
                },
            );
        }
    }

    let context_windows = context_windows_from_lines(lines);
    let context_complete = source.is_some_and(|source| {
        let source_end = source.windows.last().map_or(0, |window| {
            window.start_line + window.lines.len().saturating_sub(1)
        });
        source.complete
            && !context_cut
            && context_windows
                .first()
                .is_some_and(|window| window.start_line == 1)
            && context_windows.last().is_some_and(|window| {
                window.start_line + window.lines.len().saturating_sub(1) == source_end
            })
            && context_windows
                .iter()
                .map(|window| window.lines.len())
                .sum::<usize>()
                == source_end
    });
    Some(GuiDiffMoveEndpoint {
        path: path.to_string(),
        start_line,
        line_count: count,
        review_line,
        context_windows,
        context_complete,
    })
}

fn context_windows_from_lines(lines: BTreeMap<usize, GuiDiffLine>) -> Vec<GuiDiffContextWindow> {
    let mut windows: Vec<GuiDiffContextWindow> = Vec::new();
    for (number, line) in lines {
        if let Some(last) = windows.last_mut() {
            if last.start_line + last.lines.len() == number {
                last.lines.push(line);
                continue;
            }
        }
        windows.push(GuiDiffContextWindow {
            start_line: number,
            lines: vec![line],
        });
    }
    windows
}

fn project_standalone(title: &str, path: &str, content: &str) -> GuiDiffReview {
    let mut file = GuiDiffFile {
        checked: false,
        id: path.to_string(),
        label: None,
        message: None,
        path: path.to_string(),
        old_path: None,
        status: "modified".to_string(),
        additions: 0,
        deletions: 0,
        binary: false,
        metadata: Vec::new(),
        hunks: Vec::new(),
    };
    let mut old_line = 0;
    let mut new_line = 0;

    for (index, text) in content.lines().enumerate() {
        if let Some((old_start, old_count, new_start, new_count)) = parse_hunk_header(text) {
            old_line = old_start;
            new_line = new_start;
            file.hunks.push(GuiDiffHunk {
                context: None,
                header: text.to_string(),
                old_start,
                old_count,
                new_start,
                new_count,
                review_line: Some(index),
                lines: Vec::new(),
            });
            continue;
        }
        if text.starts_with("Binary files ") || text.starts_with("GIT binary patch") {
            file.binary = true;
        }
        if is_visible_metadata(text) {
            file.metadata.push(text.to_string());
        }
        if let Some(old_path) = text.strip_prefix("rename from ") {
            file.old_path = Some(old_path.to_string());
            file.status = "renamed".to_string();
        }
        if text.starts_with("deleted file mode ") {
            file.status = "deleted".to_string();
        }
        if text.starts_with("new file mode ") {
            file.status = "added".to_string();
        }
        let Some(hunk) = file.hunks.last_mut() else {
            continue;
        };
        let Some((kind, body)) = text
            .strip_prefix('+')
            .map(|body| ("added", body))
            .or_else(|| text.strip_prefix('-').map(|body| ("removed", body)))
            .or_else(|| text.strip_prefix(' ').map(|body| ("context", body)))
        else {
            continue;
        };
        let old = (kind != "added").then_some(old_line);
        let new = (kind != "removed").then_some(new_line);
        if old.is_some() {
            old_line += 1;
        }
        if new.is_some() {
            new_line += 1;
        }
        file.additions += usize::from(kind == "added");
        file.deletions += usize::from(kind == "removed");
        hunk.lines.push(GuiDiffLine {
            kind,
            text: body.to_string(),
            old_line: old,
            new_line: new,
            review_line: Some(index),
            highlights: Vec::new(),
        });
    }

    GuiDiffReview {
        comments: Vec::new(),
        title: title.to_string(),
        layout: "split",
        managed: false,
        custom: false,
        show_checked: false,
        provenance: None,
        files: vec![file],
        overlay: None,
        moves: Vec::new(),
        guided_files: Vec::new(),
    }
}

fn is_visible_metadata(text: &str) -> bool {
    [
        "old mode ",
        "new mode ",
        "new file mode ",
        "deleted file mode ",
        "rename from ",
        "rename to ",
        "copy from ",
        "copy to ",
        "similarity index ",
        "Binary files ",
        "GIT binary patch",
        "\\ No newline at end of file",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

fn project_highlights(
    text: &str,
    spans: &[(std::ops::Range<usize>, crate::syntax::HighlightGroup)],
) -> Vec<GuiDiffHighlight> {
    spans
        .iter()
        .filter_map(|(range, group)| {
            let start = range.start.min(text.len());
            let end = range.end.min(text.len());
            if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                return None;
            }
            Some(GuiDiffHighlight {
                start: text[..start].encode_utf16().count(),
                end: text[..end].encode_utf16().count(),
                token: super::syntax_name(*group).to_string(),
            })
        })
        .collect()
}

fn kind_name(kind: PatchLineKind) -> &'static str {
    match kind {
        PatchLineKind::Context => "context",
        PatchLineKind::Added => "added",
        PatchLineKind::Removed => "removed",
        _ => unreachable!("only source lines have a line kind"),
    }
}

fn layout_name(layout: DiffLayout) -> &'static str {
    match layout {
        DiffLayout::Split => "split",
        DiffLayout::Unified => "unified",
    }
}

fn parse_hunk_header(header: &str) -> Option<(usize, usize, usize, usize)> {
    let body = header.strip_prefix("@@ -")?;
    let (old, rest) = body.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let parse_range = |value: &str| {
        let (start, count) = value.split_once(',').unwrap_or((value, "1"));
        Some((start.parse().ok()?, count.parse().ok()?))
    };
    let (old_start, old_count) = parse_range(old)?;
    let (new_start, new_count) = parse_range(new)?;
    Some((old_start, old_count, new_start, new_count))
}

fn focus_diff_pane(editor: &mut Editor, pane: usize, buffer_id: u64) -> Result<(), String> {
    if !editor.focus_window(pane) || editor.buffer().id() != buffer_id {
        return Err("The diff pane has changed; try the action again".to_string());
    }
    if !editor.is_diff_review_buffer()
        && !editor
            .buffer()
            .display_name()
            .is_some_and(|name| name.starts_with(DIFF_REVIEW_TITLE_PREFIX))
    {
        return Err("This pane no longer shows a diff".to_string());
    }
    Ok(())
}

pub fn perform_diff_action(
    editor: &mut Editor,
    pane: usize,
    buffer_id: u64,
    action: &str,
) -> Result<(), String> {
    focus_diff_pane(editor, pane, buffer_id)?;
    let managed = editor.is_diff_review_buffer();
    match action {
        "show_checked" if managed => editor.toggle_diff_review_show_checked(),
        action if managed && action.starts_with("check:") => {
            editor
                .toggle_diff_review_check(action.strip_prefix("check:").expect("check action"))?;
        }
        "clear_comments" if managed => editor.clear_diff_comments(),
        action if managed && action.starts_with("remove_comment:") => {
            let id = action[15..].parse().map_err(|_| "Invalid comment id")?;
            editor.remove_diff_comment(id);
        }
        "refresh" if managed => editor.refresh_diff_review(),
        "split" if managed => editor.set_diff_review_layout(DiffLayout::Split),
        "unified" if managed => editor.set_diff_review_layout(DiffLayout::Unified),
        "toggle_overlay" if managed => editor
            .toggle_diff_review_overlay()
            .map_err(|error| format!("{error:#}"))?,
        "open_saved_overlay" if managed => editor
            .open_saved_diff_overlay()
            .map_err(|error| format!("{error:#}"))?,
        "return_to_live_diff" if managed => editor
            .return_to_live_diff_review()
            .map_err(|error| format!("{error:#}"))?,
        action
            if managed
                && (action.starts_with("expand_up:") || action.starts_with("expand_down:")) =>
        {
            let (direction, id) = action.split_once(':').expect("context action");
            if editor
                .diff_review()
                .and_then(|review| review.context_view(id))
                .is_none()
            {
                return Err("Unknown diff context region".to_string());
            }
            editor.expand_diff_context(id, direction == "expand_up");
        }
        "close" if managed => editor.close_diff_review(),
        "close" => editor.close_current_tab(),
        _ => return Err(format!("Unsupported diff action: {action}")),
    }
    Ok(())
}

pub fn goto_diff_definition(
    editor: &mut Editor,
    pane: usize,
    buffer_id: u64,
    path: &str,
    line: usize,
    side: &str,
    column: u32,
) -> Result<(), String> {
    focus_diff_pane(editor, pane, buffer_id)?;
    editor
        .diff_review_goto_definition(path, line, side, column)
        .map_err(|error| format!("{error:#}"))
}

pub fn open_diff_source(
    editor: &mut Editor,
    pane: usize,
    buffer_id: u64,
    path: &str,
    line: usize,
    side: &str,
) -> Result<(), String> {
    focus_diff_pane(editor, pane, buffer_id)?;
    if editor.is_diff_review_buffer() {
        return editor
            .diff_review_open_source(path, line, side)
            .map_err(|error| format!("{error:#}"));
    }

    let expected_path = editor
        .buffer()
        .display_name()
        .and_then(|title| title.strip_prefix(DIFF_REVIEW_TITLE_PREFIX))
        .ok_or_else(|| "This pane no longer shows a file diff".to_string())?;
    if Path::new(path)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("The selected source is outside this diff".to_string());
    }
    let workspace = super::projected_workspace_path(editor)
        .ok_or_else(|| "No Git workspace is open".to_string())?;
    let root =
        ovim_core::native_diff::worktree_root(&workspace).map_err(|error| format!("{error:#}"))?;
    let review = project_diff(editor, editor.buffer())
        .ok_or_else(|| "This pane no longer shows a diff".to_string())?;
    let file = &review.files[0];
    if file.status == "deleted" {
        return Err(format!("{} was deleted in this diff", file.path));
    }
    if path != expected_path && !(side == "old" && file.old_path.as_deref() == Some(path)) {
        return Err("The selected source is outside this diff".to_string());
    }
    let new_line = match side {
        "new" => line,
        "old" => map_old_line(&file.hunks, line)
            .ok_or_else(|| format!("Old line is not in this diff: {path}:{line}"))?,
        _ => return Err(format!("Unknown diff side: {side}")),
    };
    let target = root.join(expected_path);
    editor
        .open_file(&target)
        .map_err(|error| error.to_string())?;
    editor
        .buffer_mut()
        .cursor_mut()
        .set_position(new_line.saturating_sub(1), crate::unicode::GraphemeCol(0));
    editor.buffer_mut().validate_cursor_position();
    editor.center_cursor_in_viewport();
    editor.mark_dirty();
    Ok(())
}

fn map_old_line(hunks: &[GuiDiffHunk], old_line: usize) -> Option<usize> {
    for hunk in hunks {
        let mut current_new = hunk.new_start;
        for line in &hunk.lines {
            if line.old_line == Some(old_line) {
                return Some(line.new_line.unwrap_or(current_new).max(1));
            }
            if let Some(next) = line.new_line {
                current_new = next + 1;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{IndexAddOption, Repository, Signature};
    use ovim_core::native_diff::{review_snapshot, ChangeRef, DiffPairing, ReviewBase};
    use std::fs;

    fn commit(repo: &Repository) {
        let mut index = repo.index().unwrap();
        index.add_all(["*"], IndexAddOption::DEFAULT, None).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = Signature::now("Ovim", "ovim@example.com").unwrap();
        let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "fixture",
            &tree,
            &parents,
        )
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn review_checks_project_to_both_views_and_validate_pane_identity() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let repo = Repository::init(root).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        fs::write(root.join("source.rs"), "fn old() {}\n").unwrap();
        commit(&repo);
        fs::write(root.join("source.rs"), "fn updated() {}\n").unwrap();
        let custom = review_snapshot(root, &ReviewBase::explicit("HEAD"))
            .unwrap()
            .reassign(&[])
            .unwrap();
        let mut editor = Editor::new();
        editor.open_file(root.join("source.rs")).unwrap();
        editor.open_custom_diff_review("Review", custom).unwrap();
        editor.init_window_manager(100, 40);
        let id = editor.buffer().id();
        assert!(perform_diff_action(&mut editor, 0, id + 1, "check:file:source.rs").is_err());
        assert!(perform_diff_action(&mut editor, 0, id, "check:file:unknown").is_err());
        perform_diff_action(&mut editor, 0, id, "check:file:source.rs").unwrap();
        let projected = project_diff(&editor, editor.buffer()).unwrap();
        assert!(projected.files[0].checked);
        assert!(projected.guided_files.iter().all(|file| file.checked));
        assert!(!projected.show_checked);
        assert!(!editor.buffer().rope().to_string().contains("updated()"));
        perform_diff_action(&mut editor, 0, id, "show_checked").unwrap();
        assert!(project_diff(&editor, editor.buffer()).unwrap().show_checked);
        assert!(editor.buffer().rope().to_string().contains("updated()"));
        assert!(goto_diff_definition(&mut editor, 0, id + 1, "source.rs", 1, "new", 4).is_err());
        goto_diff_definition(&mut editor, 0, id, "source.rs", 1, "new", 4).unwrap();
        assert_eq!(editor.buffer().cursor().col().0, 4);
        assert!(!editor.is_diff_review_buffer());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn one_sided_sections_and_copy_references_survive_restart_and_projection() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let repo = Repository::init(root).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        fs::write(root.join("anchor.rs"), "fn anchor() {}\n").unwrap();
        fs::write(root.join("old.rs"), "duplicate_check();\nrespondent();\n").unwrap();
        commit(&repo);
        fs::remove_file(root.join("old.rs")).unwrap();
        fs::write(root.join("new.rs"), "respondent();\nrespondent();\n").unwrap();
        let snapshot = review_snapshot(root, &ReviewBase::explicit("HEAD")).unwrap();
        let old = snapshot
            .blocks
            .iter()
            .find(|b| b.kind == PatchLineKind::Removed)
            .unwrap();
        let new = snapshot
            .blocks
            .iter()
            .find(|b| b.kind == PatchLineKind::Added)
            .unwrap();
        let reference = |id: &str, offset| ChangeRef {
            block_id: id.into(),
            offset: Some(offset),
            count: Some(1),
        };
        let specs = vec![
            DiffPairing {
                message: Some("The duplicate guard is no longer needed.".into()),
                label: Some("Remove duplicate permission check".into()),
                old: Some(reference(&old.id, 0)),
                new: None,
                related_to: None,
            },
            DiffPairing {
                message: None,
                label: Some("Move respondent lookup".into()),
                old: Some(reference(&old.id, 1)),
                new: Some(reference(&new.id, 0)),
                related_to: None,
            },
            DiffPairing {
                message: None,
                label: Some("Additional respondent copy".into()),
                old: None,
                new: Some(reference(&new.id, 1)),
                related_to: Some(reference(&old.id, 1)),
            },
        ];
        let custom = snapshot.reassign(&specs).unwrap();
        let storage = tempfile::tempdir().unwrap();
        let store = ovim_core::native_diff::store::ReviewStore::new(storage.path().to_path_buf());
        let mut editor = Editor::default();
        editor.set_diff_review_store(Some(store.clone()));
        editor.open_file(root.join("anchor.rs")).unwrap();
        editor
            .open_custom_diff_review("Explain deletion and copy", custom)
            .unwrap();
        let opened = project_diff(&editor, editor.buffer()).unwrap();
        assert_eq!(
            opened.overlay.as_ref().map(|overlay| overlay.mode),
            Some("saved")
        );
        assert_eq!(opened.guided_files.len(), 5);
        assert_eq!(opened.guided_files[0].id, "pair_0");
        assert_eq!(opened.guided_files[1].id, "pair_1");
        assert_eq!(opened.guided_files[2].id, "pair_2");
        assert_eq!(
            opened.guided_files[0].message.as_deref(),
            Some("The duplicate guard is no longer needed.")
        );
        assert!(opened.files.iter().all(|file| file.message.is_none()));
        assert_eq!(opened.guided_files[0].hunks[0].lines[0].old_line, Some(1));
        assert_eq!(opened.guided_files[1].hunks[0].lines[0].old_line, Some(2));
        assert_eq!(opened.guided_files[1].hunks[0].lines[1].new_line, Some(1));
        assert_eq!(opened.guided_files[2].hunks[0].lines[0].new_line, Some(2));
        drop(editor);
        let mut editor = Editor::default();
        editor.set_diff_review_store(Some(store));
        editor.open_file(root.join("anchor.rs")).unwrap();
        editor.open_diff_review(Some("HEAD")).unwrap();
        let projected = project_diff(&editor, editor.buffer()).unwrap();
        assert_eq!(
            projected.overlay.as_ref().map(|overlay| overlay.mode),
            Some("active")
        );
        assert_eq!(
            projected.moves.len(),
            1,
            "A reference must not become an equal-filter pairing"
        );
        let guided = &projected.guided_files;
        assert_eq!(guided[0].status, "deletion");
        assert_eq!(guided[0].additions, 0);
        assert_eq!(guided[0].deletions, 1);
        assert_eq!(guided[2].status, "addition");
        assert!(guided[2].old_path.is_none());
        assert!(guided[2]
            .label
            .as_ref()
            .unwrap()
            .contains("Related source (before): old.rs:2"));
        assert_eq!(guided.iter().map(|f| f.additions).sum::<usize>(), 2);
        assert_eq!(guided.iter().map(|f| f.deletions).sum::<usize>(), 2);
        if let Some(path) = std::env::var_os("OVIM_SECTION_QA_FIXTURE") {
            fs::write(path, serde_json::to_vec_pretty(&projected).unwrap()).unwrap();
        }
        editor.toggle_diff_review_equal_changes();
        for _ in 0..2 {
            let text = editor.buffer().rope().to_string();
            assert!(text.contains("Deletion · Remove duplicate permission check"));
            assert!(text.contains("Addition · Additional respondent copy"));
            assert!(text.contains("duplicate_check();"));
            assert_eq!(text.matches("respondent();").count(), 1, "{text}");
            editor
                .diff_review()
                .unwrap()
                .custom()
                .unwrap()
                .validate_coverage()
                .unwrap();
            editor.toggle_diff_review_layout();
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn context_actions_validate_identity_and_project_captured_source() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let repo = Repository::init(root).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let mut lines: Vec<_> = (1..=60)
            .map(|line| format!("source_{line:03}();"))
            .collect();
        fs::write(root.join("source.rs"), lines.join("\n") + "\n").unwrap();
        commit(&repo);
        lines[29] = "changed();".into();
        fs::write(root.join("source.rs"), lines.join("\n") + "\n").unwrap();
        let mut editor = Editor::new();
        editor.open_file(root.join("source.rs")).unwrap();
        editor.open_diff_review(Some("HEAD")).unwrap();
        editor.init_window_manager(100, 40);
        let buffer_id = editor.buffer().id();
        let initial = project_diff(&editor, editor.buffer()).unwrap();
        let context = initial.files[0].hunks[0].context.as_ref().unwrap();
        assert!(context.can_expand_up && context.can_expand_down);
        let action = format!("expand_up:{}", context.id);
        assert!(perform_diff_action(&mut editor, 0, buffer_id + 1, &action).is_err());
        assert!(perform_diff_action(&mut editor, 0, buffer_id, "expand_up:missing").is_err());
        fs::write(root.join("source.rs"), "different worktree\n").unwrap();
        perform_diff_action(&mut editor, 0, buffer_id, &action).unwrap();
        let projected = project_diff(&editor, editor.buffer()).unwrap();
        let hunk = &projected.files[0].hunks[0];
        assert_eq!(
            hunk.lines.iter().map(|line| &line.text).collect::<Vec<_>>(),
            initial.files[0].hunks[0]
                .lines
                .iter()
                .map(|line| &line.text)
                .collect::<Vec<_>>()
        );
        let context = hunk.context.as_ref().unwrap();
        assert_eq!(context.before.old.len(), 10);
        assert_eq!(context.before.old[0].number, 17);
        assert_eq!(context.before.old[0].text, "source_017();");
        open_diff_source(&mut editor, 0, buffer_id, "source.rs", 17, "new").unwrap();
        assert!(editor.buffer().rope().to_string().contains("source_017();"));
        assert!(!editor
            .buffer()
            .rope()
            .to_string()
            .contains("different worktree"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn custom_projection_keeps_real_files_and_embeds_frozen_move_context() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let repo = Repository::init(root).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        fs::write(root.join("anchor.rs"), "fn anchor() {}\n").unwrap();
        fs::write(
            root.join("old.rs"),
            "fn old_parser() {\n    parse_old();\n}\n",
        )
        .unwrap();
        commit(&repo);
        fs::remove_file(root.join("old.rs")).unwrap();
        fs::write(
            root.join("new.rs"),
            "fn new_parser() {\n    parse_new();\n}\n",
        )
        .unwrap();

        let snapshot = review_snapshot(root, &ReviewBase::explicit("HEAD")).unwrap();
        let old = snapshot
            .blocks
            .iter()
            .find(|block| block.kind == PatchLineKind::Removed)
            .unwrap();
        let new = snapshot
            .blocks
            .iter()
            .find(|block| block.kind == PatchLineKind::Added)
            .unwrap();
        let pairings = vec![DiffPairing {
            message: None,
            label: Some("Parser moved".into()),
            old: ChangeRef {
                block_id: old.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            new: ChangeRef {
                block_id: new.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            related_to: None,
        }];
        let custom = snapshot.reassign(&pairings).unwrap();
        let storage = tempfile::tempdir().unwrap();
        let store = ovim_core::native_diff::store::ReviewStore::new(storage.path().to_path_buf());
        let mut editor = Editor::default();
        editor.set_diff_review_store(Some(store.clone()));
        editor.open_file(root.join("anchor.rs")).unwrap();
        editor
            .open_custom_diff_review("Parser move", custom)
            .unwrap();

        drop(editor);
        // A fresh frontend needs neither the old process nor resumed chat history.
        let mut editor = Editor::default();
        editor.set_diff_review_store(Some(store));
        editor.open_file(root.join("anchor.rs")).unwrap();
        editor.open_diff_review(Some("HEAD")).unwrap();
        let projected = project_diff(&editor, editor.buffer()).unwrap();
        if let Some(path) = std::env::var_os("OVIM_DIFF_QA_FIXTURE") {
            fs::write(path, serde_json::to_vec_pretty(&projected).unwrap()).unwrap();
        }
        assert!(projected.custom);
        let provenance = projected.provenance.as_ref().unwrap();
        assert_eq!(provenance.base_label, "HEAD");
        assert!(!provenance.comparison_base_oid.is_empty());
        assert!(provenance
            .snapshot_id
            .as_deref()
            .is_some_and(|id| id.starts_with("diff_")));
        assert_eq!(
            projected.overlay.as_ref().map(|state| state.mode),
            Some("active")
        );
        assert!(projected.files.iter().any(|file| file.path == "old.rs"));
        assert!(projected.files.iter().any(|file| file.path == "new.rs"));
        assert!(projected
            .files
            .iter()
            .all(|file| file.status != "reassigned"));
        assert!(projected
            .guided_files
            .iter()
            .any(|file| file.status == "reassigned"));
        assert_eq!(projected.moves.len(), 1);
        let moved = &projected.moves[0];
        assert_eq!(moved.label.as_deref(), Some("Parser moved"));
        assert_eq!(moved.old.path, "old.rs");
        assert_eq!(moved.new.path, "new.rs");
        assert!(moved
            .old
            .context_windows
            .iter()
            .flat_map(|window| &window.lines)
            .any(|line| line.text.contains("parse_old")));
        assert!(moved
            .new
            .context_windows
            .iter()
            .flat_map(|window| &window.lines)
            .any(|line| line.text.contains("parse_new")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn moved_endpoints_keep_all_changed_lines_when_source_capture_is_partial() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let repo = Repository::init(root).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        fs::write(root.join("anchor.rs"), "fn anchor() {}\n").unwrap();
        let long_old = format!("old_{}", "x".repeat(140_000));
        let long_new = format!("new_{}", "y".repeat(140_000));
        let old_lines = (0..650)
            .map(|index| {
                if index == 300 {
                    long_old.clone()
                } else {
                    format!("old_{index}")
                }
            })
            .collect::<Vec<_>>();
        fs::write(root.join("old.rs"), format!("{}\n", old_lines.join("\n"))).unwrap();
        commit(&repo);
        fs::remove_file(root.join("old.rs")).unwrap();
        let new_lines = (0..650)
            .map(|index| {
                if index == 300 {
                    long_new.clone()
                } else {
                    format!("new_{index}")
                }
            })
            .collect::<Vec<_>>();
        fs::write(root.join("new.rs"), format!("{}\n", new_lines.join("\n"))).unwrap();

        let mut snapshot = review_snapshot(root, &ReviewBase::explicit("HEAD")).unwrap();
        for source in &mut snapshot.sources {
            for side in [&mut source.old, &mut source.new].into_iter().flatten() {
                side.complete = false;
                side.windows.truncate(1);
                for window in &mut side.windows {
                    window.lines.truncate(10);
                }
            }
        }
        let removed = snapshot
            .blocks
            .iter()
            .find(|block| block.kind == PatchLineKind::Removed && block.line_count == 650)
            .unwrap();
        let added = snapshot
            .blocks
            .iter()
            .find(|block| block.kind == PatchLineKind::Added && block.line_count == 650)
            .unwrap();
        let pairings = vec![DiffPairing {
            message: None,
            label: Some("Large move".into()),
            old: ChangeRef {
                block_id: removed.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            new: ChangeRef {
                block_id: added.id.clone(),
                offset: None,
                count: None,
            }
            .into(),
            related_to: None,
        }];
        let custom = snapshot.reassign(&pairings).unwrap();
        let mut editor = Editor::default();
        editor.open_file(root.join("anchor.rs")).unwrap();
        editor
            .open_custom_diff_review("Large move", custom)
            .unwrap();

        let projected = project_diff(&editor, editor.buffer()).unwrap();
        let moved = &projected.moves[0];
        for (endpoint, expected_kind, expected_long) in [
            (&moved.old, "removed", &long_old),
            (&moved.new, "added", &long_new),
        ] {
            let changed = endpoint
                .context_windows
                .iter()
                .flat_map(|window| &window.lines)
                .filter(|line| line.kind == expected_kind)
                .collect::<Vec<_>>();
            assert_eq!(changed.len(), 650);
            assert_eq!(changed[300].text, *expected_long);
            assert!(!endpoint.context_complete);
        }
    }

    #[test]
    fn standalone_patch_projects_compact_hunks_and_source_lines() {
        let review = project_standalone(
            "Diff · src/example.rs",
            "src/example.rs",
            "comparison: HEAD...WORKTREE\nfile: src/example.rs\n\n@@ -8,2 +8,3 @@ fn main()\n old\n-gone\n+new\n+next\n",
        );
        let hunk = &review.files[0].hunks[0];
        assert_eq!((hunk.old_start, hunk.old_count), (8, 2));
        assert_eq!((hunk.new_start, hunk.new_count), (8, 3));
        assert_eq!(hunk.lines[1].old_line, Some(9));
        assert_eq!(hunk.lines[1].new_line, None);
        assert_eq!(hunk.lines[2].new_line, Some(9));
        assert_eq!(hunk.lines[2].review_line, Some(6));
        assert_eq!(map_old_line(&review.files[0].hunks, 9), Some(9));
        let json = serde_json::to_value(&hunk.lines[1]).unwrap();
        assert!(json.get("newLine").is_none());
    }

    #[test]
    fn hunk_header_defaults_omitted_counts_to_one() {
        assert_eq!(parse_hunk_header("@@ -2 +4 @@"), Some((2, 1, 4, 1)));
        assert_eq!(parse_hunk_header("not a hunk"), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn branch_projection_keeps_syntax_and_old_line_source_mapping() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let repo = Repository::init(root).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        fs::write(
            root.join("sample.rs"),
            "fn example() {\n    let name = 1;\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("old.rs"),
            "fn renamed() {\n    let first = 1;\n    let second = 2;\n}\n",
        )
        .unwrap();
        fs::write(root.join("gone.rs"), "fn gone() {}\n").unwrap();
        commit(&repo);
        let main = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &main, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        fs::write(
            root.join("sample.rs"),
            "fn example() {\n    let name = 2;\n}\n",
        )
        .unwrap();
        fs::rename(root.join("old.rs"), root.join("new.rs")).unwrap();
        fs::write(
            root.join("new.rs"),
            "fn renamed() {\n    let first = 3;\n    let second = 2;\n}\n",
        )
        .unwrap();
        fs::remove_file(root.join("gone.rs")).unwrap();
        {
            let mut index = repo.index().unwrap();
            index.remove_path(Path::new("old.rs")).unwrap();
            index.remove_path(Path::new("gone.rs")).unwrap();
            index.write().unwrap();
        }
        commit(&repo);

        let mut editor = Editor::new();
        editor.open_file(root.join("sample.rs")).unwrap();
        editor.open_diff_review(Some("main")).unwrap();
        let review = project_diff(&editor, editor.buffer()).unwrap();
        assert!(review.managed);
        let sample = review
            .files
            .iter()
            .find(|file| file.path == "sample.rs")
            .unwrap();
        let changed = &sample.hunks[0].lines;
        assert!(changed
            .iter()
            .any(|line| line.kind == "removed" && line.old_line == Some(2)));
        assert!(changed
            .iter()
            .any(|line| line.kind == "added" && line.new_line == Some(2)));
        assert!(changed.iter().any(|line| !line.highlights.is_empty()));
        assert!(changed.iter().all(|line| line.review_line.is_some()));

        editor
            .diff_review_open_source("sample.rs", 2, "old")
            .unwrap();
        assert_eq!(editor.buffer().cursor().line(), 1);
        assert_eq!(editor.buffer().display_name(), None);

        editor.toggle_diff_review();
        let review = project_diff(&editor, editor.buffer()).unwrap();
        let renamed = review
            .files
            .iter()
            .find(|file| file.path == "new.rs")
            .unwrap();
        assert_eq!(renamed.old_path.as_deref(), Some("old.rs"));
        assert_eq!(renamed.status, "renamed");
        editor.diff_review_open_source("old.rs", 2, "old").unwrap();
        assert!(editor.buffer().file_path().unwrap().ends_with("new.rs"));
        assert_eq!(editor.buffer().cursor().line(), 1);

        editor.toggle_diff_review();
        let error = editor
            .diff_review_open_source("gone.rs", 1, "old")
            .unwrap_err();
        assert!(error.to_string().contains("deleted"));
    }

    #[test]
    fn standalone_rename_and_deletion_metadata_are_preserved() {
        let rename = project_standalone(
            "Diff · new.rs",
            "new.rs",
            "rename from old.rs\nrename to new.rs\n@@ -3 +3 @@\n-old\n+new\n",
        );
        assert_eq!(rename.files[0].old_path.as_deref(), Some("old.rs"));
        assert_eq!(rename.files[0].status, "renamed");
        assert_eq!(map_old_line(&rename.files[0].hunks, 3), Some(3));

        let deleted = project_standalone(
            "Diff · gone.rs",
            "gone.rs",
            "deleted file mode 100644\n@@ -1 +0,0 @@\n-gone\n",
        );
        assert_eq!(deleted.files[0].status, "deleted");
    }

    #[test]
    fn stale_buffer_identity_cannot_close_a_new_diff() {
        let mut editor = Editor::new();
        editor.open_diff_buffer_in_new_tab("Diff · sample.rs", "@@ -1 +1 @@\n-old\n+new\n");
        let buffer_id = editor.buffer().id();
        let tab_count = editor.tab_count();
        let error = perform_diff_action(&mut editor, 0, buffer_id + 1, "close").unwrap_err();
        assert!(error.contains("pane has changed"));
        assert_eq!(editor.tab_count(), tab_count);
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn user_comments_validate_source_and_round_trip_through_native_projection() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let file = dir.path().join("payment.ts");
        fs::write(
            &file,
            "function acceptPayment(amount: number) {\n  return amount >= 0;\n}\n",
        )
        .unwrap();
        commit(&repo);
        fs::write(
            &file,
            "function acceptPayment(amount: number) {\n  return amount > 0;\n}\n",
        )
        .unwrap();
        let mut editor = Editor::default();
        editor.open_file(&file).unwrap();
        editor.open_diff_review(Some("HEAD")).unwrap();
        editor.init_window_manager(180, 40);
        let buffer_id = editor.buffer().id();
        let initial = project_diff(&editor, editor.buffer()).unwrap();
        let input = |id, text: &str| GuiDiffCommentInput {
            id,
            path: "payment.ts".into(),
            line: 2,
            side: "new".into(),
            before: true,
            text: text.into(),
        };
        assert!(add_comment(&mut editor, 0, buffer_id + 1, input(None, "stale")).is_err());
        let mut invalid = input(None, "outside diff");
        invalid.path = "../secret".into();
        assert!(add_comment(&mut editor, 0, buffer_id, invalid).is_err());
        let mut invalid = input(None, "wrong side");
        invalid.side = "unknown".into();
        assert!(add_comment(&mut editor, 0, buffer_id, invalid).is_err());
        assert!(add_comment(&mut editor, 0, buffer_id, input(None, "   ")).is_err());
        assert!(add_comment(&mut editor, 0, buffer_id, input(None, &"x".repeat(32769))).is_err());
        add_comment(
            &mut editor,
            0,
            buffer_id,
            input(None, "Should zero still be valid?\nHva med refusjoner?"),
        )
        .unwrap();
        let added = project_diff(&editor, editor.buffer()).unwrap();
        assert_eq!(added.comments[0].excerpt, "  return amount > 0;");
        add_comment(
            &mut editor,
            0,
            buffer_id,
            input(
                Some(added.comments[0].id),
                "Why reject zero?\n第二个问题：负数呢？",
            ),
        )
        .unwrap();
        let edited = project_diff(&editor, editor.buffer()).unwrap();
        editor.set_diff_review_layout(DiffLayout::Unified);
        let unified = project_diff(&editor, editor.buffer()).unwrap();
        editor
            .open_ai_chat(ovim_core::ai::chat_types::ChatOpts::default())
            .unwrap();
        let attachment = editor.pending_diff_comments().unwrap().message();
        editor.clear_diff_comments();
        let cleared = project_diff(&editor, editor.buffer()).unwrap();
        assert!(cleared.comments.is_empty());
        assert!(editor.pending_diff_comments().is_none());
        if let Ok(path) = std::env::var("OVIM_COMMENT_QA_FIXTURE") {
            fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({"initial":initial,"added":added,"edited":edited,"unified":unified,"cleared":cleared,"attachment":attachment})).unwrap()).unwrap();
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuiDiffCommentInput {
    pub id: Option<u64>,
    pub path: String,
    pub line: usize,
    pub side: String,
    pub before: bool,
    pub text: String,
}

pub fn add_comment(
    editor: &mut Editor,
    pane: usize,
    buffer_id: u64,
    comment: GuiDiffCommentInput,
) -> Result<(), String> {
    focus_diff_pane(editor, pane, buffer_id)?;
    if let Some(id) = comment.id {
        editor.update_diff_comment(id, &comment.text)
    } else {
        editor.add_diff_comment(
            &comment.path,
            comment.line,
            &comment.side,
            comment.before,
            &comment.text,
        )
    }
}
