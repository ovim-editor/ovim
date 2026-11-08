//! Presentation-only surrounding source. Never changes patch coverage or saved pairings.
use super::{CustomReview, PatchLineKind, ReviewSnapshot};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextLine {
    pub number: usize,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEdge {
    pub old: Vec<ContextLine>,
    pub new: Vec<ContextLine>,
}

/// Source lines revealed per incremental expansion click or keypress.
pub const CONTEXT_STEP: usize = 20;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffContextView {
    pub id: String,
    pub before: ContextEdge,
    pub after: ContextEdge,
    pub can_expand_up: bool,
    pub can_expand_down: bool,
}

#[derive(Clone, Debug)]
struct Side {
    path: String,
    start: usize,
    end: usize,
    before: Vec<ContextLine>,
    after: Vec<ContextLine>,
}

#[derive(Clone, Debug)]
pub struct ContextRegion {
    pub id: String,
    pub guided: bool,
    pub patch_lines: Vec<usize>,
    old: Option<Side>,
    new: Option<Side>,
}

#[derive(Default)]
pub struct DiffContext {
    regions: Vec<ContextRegion>,
}

impl DiffContext {
    pub fn new(snapshot: &ReviewSnapshot, custom: Option<&CustomReview>) -> Self {
        let patch = &snapshot.patch;
        let mut regions = Vec::new();
        for (index, info) in patch.lines.iter().enumerate() {
            if info.kind != PatchLineKind::HunkHeader {
                continue;
            }
            let Some(file) = info.file.and_then(|file| patch.files.get(file)) else {
                continue;
            };
            let lines: Vec<_> = patch
                .lines
                .iter()
                .enumerate()
                .skip(index + 1)
                .take_while(|(_, line)| {
                    !matches!(
                        line.kind,
                        PatchLineKind::HunkHeader | PatchLineKind::FileHeader
                    )
                })
                .filter(|(_, line)| {
                    matches!(
                        line.kind,
                        PatchLineKind::Context | PatchLineKind::Added | PatchLineKind::Removed
                    )
                })
                .map(|(index, line)| (index, line.kind, line.old_line, line.new_line))
                .collect();
            regions.push(region(
                format!("hunk:{index}"),
                false,
                Some(file.old_path.as_deref().unwrap_or(&file.path)),
                Some(&file.path),
                &lines,
            ));
        }
        if let Some(custom) = custom {
            for section in &custom.sections {
                let lines: Vec<_> = section
                    .lines
                    .iter()
                    .filter(|line| {
                        matches!(
                            line.kind,
                            PatchLineKind::Context | PatchLineKind::Added | PatchLineKind::Removed
                        )
                    })
                    .map(|line| {
                        (
                            line.source_patch_line,
                            line.kind,
                            line.old_line,
                            line.new_line,
                        )
                    })
                    .collect();
                regions.push(region(
                    format!("section:{}", section.id),
                    true,
                    section.old_path.as_deref(),
                    section.new_path.as_deref(),
                    &lines,
                ));
            }
        }
        Self { regions }
    }

    pub fn regions(&self, guided: bool) -> impl Iterator<Item = &ContextRegion> {
        self.regions
            .iter()
            .filter(move |region| region.guided == guided)
    }

    pub fn contains_expanded_line(&self, path: &str, number: usize, old: bool) -> bool {
        self.regions.iter().any(|region| {
            let side = if old { &region.old } else { &region.new };
            side.as_ref().is_some_and(|side| {
                side.path == path
                    && side
                        .before
                        .iter()
                        .chain(&side.after)
                        .any(|line| line.number == number)
            })
        })
    }

    pub fn paths(&self, id: &str) -> (Option<&str>, Option<&str>) {
        self.regions
            .iter()
            .find(|region| region.id == id)
            .map(|region| {
                (
                    region.old.as_ref().map(|s| s.path.as_str()),
                    region.new.as_ref().map(|s| s.path.as_str()),
                )
            })
            .unwrap_or_default()
    }

    pub fn view(&self, snapshot: &ReviewSnapshot, id: &str) -> Option<DiffContextView> {
        let index = self.regions.iter().position(|region| region.id == id)?;
        let region = &self.regions[index];
        Some(DiffContextView {
            id: id.to_string(),
            before: ContextEdge {
                old: region
                    .old
                    .as_ref()
                    .map(|s| s.before.clone())
                    .unwrap_or_default(),
                new: region
                    .new
                    .as_ref()
                    .map(|s| s.before.clone())
                    .unwrap_or_default(),
            },
            after: ContextEdge {
                old: region
                    .old
                    .as_ref()
                    .map(|s| s.after.clone())
                    .unwrap_or_default(),
                new: region
                    .new
                    .as_ref()
                    .map(|s| s.after.clone())
                    .unwrap_or_default(),
            },
            can_expand_up: [false, true]
                .into_iter()
                .any(|old| self.next(snapshot, index, old, true).is_some()),
            can_expand_down: [false, true]
                .into_iter()
                .any(|old| self.next(snapshot, index, old, false).is_some()),
        })
    }

    /// Reveal up to `count` source lines per side at one edge of the region.
    pub fn expand(&mut self, snapshot: &ReviewSnapshot, id: &str, up: bool, count: usize) -> bool {
        let Some(index) = self.regions.iter().position(|region| region.id == id) else {
            return false;
        };
        let mut changed = false;
        for old in [true, false] {
            for _ in 0..count {
                let Some(line) = self.next(snapshot, index, old, up) else {
                    break;
                };
                let region = &mut self.regions[index];
                let side = if old {
                    region.old.as_mut()
                } else {
                    region.new.as_mut()
                }
                .expect("available side");
                if up {
                    side.before.insert(0, line);
                } else {
                    side.after.push(line);
                }
                changed = true;
            }
        }
        changed
    }

    fn next(
        &self,
        snapshot: &ReviewSnapshot,
        index: usize,
        old: bool,
        up: bool,
    ) -> Option<ContextLine> {
        let region = &self.regions[index];
        let side = if old { &region.old } else { &region.new }.as_ref()?;
        let number = if up {
            side.before
                .first()
                .map_or(side.start, |l| l.number)
                .checked_sub(1)?
        } else {
            side.after
                .last()
                .map_or(side.end, |l| l.number)
                .checked_add(1)?
        };
        if number == 0 {
            return None;
        }
        // A neighboring displayed interval owns its source lines, even after expansion.
        if self.regions.iter().enumerate().any(|(other_index, other)| {
            if other_index == index || other.guided != region.guided {
                return false;
            }
            let other = if old { &other.old } else { &other.new };
            other.as_ref().is_some_and(|other| {
                other.path == side.path
                    && number >= other.before.first().map_or(other.start, |l| l.number)
                    && number <= other.after.last().map_or(other.end, |l| l.number)
            })
        }) {
            return None;
        }
        let source = snapshot.source_for(&side.path, if old { "old" } else { "new" })?;
        source.windows.iter().find_map(|window| {
            let offset = number.checked_sub(window.start_line)?;
            window.lines.get(offset).map(|text| ContextLine {
                number,
                text: text.clone(),
            })
        })
    }
}

type SourceLine = (usize, PatchLineKind, Option<usize>, Option<usize>);
fn region(
    id: String,
    guided: bool,
    old_path: Option<&str>,
    new_path: Option<&str>,
    lines: &[SourceLine],
) -> ContextRegion {
    let side = |old: bool, path: Option<&str>| {
        let numbers: Vec<_> = lines
            .iter()
            .filter_map(|(_, kind, before, after)| {
                if old {
                    (*kind != PatchLineKind::Added).then_some(*before).flatten()
                } else {
                    (*kind != PatchLineKind::Removed)
                        .then_some(*after)
                        .flatten()
                }
            })
            .collect();
        Some(Side {
            path: path?.to_string(),
            start: *numbers.iter().min()?,
            end: *numbers.iter().max()?,
            before: Vec::new(),
            after: Vec::new(),
        })
    };
    ContextRegion {
        id,
        guided,
        patch_lines: lines.iter().map(|line| line.0).collect(),
        old: side(true, old_path),
        new: side(false, new_path),
    }
}
