//! Bounded, frozen source context for one focused walkthrough change.

use super::{FrozenSource, PatchLineKind, ReviewSection, ReviewSectionLine, ReviewSnapshot};

const MAX_CONTEXT_LINES_PER_EDGE: usize = 12;

pub(crate) struct WalkthroughDiff {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub patch: String,
    /// Inclusive, 1-based presentation rows. These are not source coordinates.
    pub focus: (usize, usize),
}

struct Excerpt<'a> {
    path: &'a str,
    source: Option<&'a FrozenSource>,
    selected: Vec<&'a ReviewSectionLine>,
    before: usize,
    after: usize,
    old: bool,
}

impl Excerpt<'_> {
    fn start(&self) -> usize {
        self.number(self.selected[0])
    }

    fn end(&self) -> usize {
        self.number(self.selected[self.selected.len() - 1])
    }

    fn number(&self, line: &ReviewSectionLine) -> usize {
        if self.old {
            line.old_line
        } else {
            line.new_line
        }
        .expect("selected lines have coordinates on their own side")
    }

    fn next_context(&self, up: bool) -> Option<(usize, &str)> {
        let number = if up {
            self.start().checked_sub(self.before + 1)?
        } else {
            self.end().checked_add(self.after + 1)?
        };
        Some((number, self.source?.line(number)?))
    }

    fn grow(&mut self, up: bool) {
        if up {
            self.before += 1;
        } else {
            self.after += 1;
        }
    }

    fn shrink(&mut self, up: bool) {
        if up {
            self.before -= 1;
        } else {
            self.after -= 1;
        }
    }
}

/// Optional context must fit the same measured viewport budget as the selected
/// change. Missing source windows never fall back to the live filesystem.
pub(crate) fn walkthrough_diff(
    snapshot: &ReviewSnapshot,
    section: &ReviewSection,
    fits: impl Fn(&str) -> bool,
) -> WalkthroughDiff {
    let excerpt = |old| {
        let path = if old {
            &section.old_path
        } else {
            &section.new_path
        };
        let kind = if old {
            PatchLineKind::Removed
        } else {
            PatchLineKind::Added
        };
        let selected: Vec<_> = section
            .lines
            .iter()
            .filter(|line| line.kind == kind)
            .collect();
        Some(Excerpt {
            path: path.as_deref()?,
            source: snapshot.source_for(path.as_deref()?, if old { "old" } else { "new" }),
            selected,
            before: 0,
            after: 0,
            old,
        })
        .filter(|side| !side.selected.is_empty())
    };
    let mut old = excerpt(true);
    let mut new = excerpt(false);
    let mut presentation = render(section, old.as_ref(), new.as_ref(), true);
    if !fits(&presentation.patch) {
        return presentation;
    }

    // Only adjacent lines in the canonical patch can share unchanged context.
    // Reassigned, cross-file and partial changes retain independent snapshots.
    let shared = old.as_ref().zip(new.as_ref()).is_some_and(|(old, new)| {
        let last = old.selected.last().unwrap().source_patch_line;
        let first = new.selected[0].source_patch_line;
        last.checked_add(1) == Some(first)
            && snapshot.patch.lines[last].file == snapshot.patch.lines[first].file
    });
    if shared {
        for _ in 0..MAX_CONTEXT_LINES_PER_EDGE {
            let mut grew = false;
            for up in [true, false] {
                let (before, after) = (old.as_mut().unwrap(), new.as_mut().unwrap());
                let available = before.next_context(up).zip(after.next_context(up));
                let Some(((old_number, old_text), (new_number, new_text))) = available else {
                    continue;
                };
                if old_text != new_text
                    || changed(snapshot, before, old_number)
                    || changed(snapshot, after, new_number)
                {
                    continue;
                }
                before.grow(up);
                after.grow(up);
                let candidate = render(section, old.as_ref(), new.as_ref(), true);
                if fits(&candidate.patch) {
                    presentation = candidate;
                    grew = true;
                } else {
                    old.as_mut().unwrap().shrink(up);
                    new.as_mut().unwrap().shrink(up);
                }
            }
            if !grew {
                break;
            }
        }
        if old
            .as_ref()
            .is_some_and(|side| side.before + side.after > 0)
        {
            return presentation;
        }
    }

    // Here a space means surrounding text from the explicitly named before or
    // after snapshot, not a claim that both versions contain the same line.
    for _ in 0..MAX_CONTEXT_LINES_PER_EDGE {
        let mut grew = false;
        for up in [true, false] {
            for is_old in [true, false] {
                let side = if is_old { &mut old } else { &mut new };
                let Some(side) = side.as_mut().filter(|side| side.next_context(up).is_some())
                else {
                    continue;
                };
                side.grow(up);
                let candidate = render(section, old.as_ref(), new.as_ref(), false);
                if fits(&candidate.patch) {
                    presentation = candidate;
                    grew = true;
                } else {
                    let side = if is_old { &mut old } else { &mut new };
                    side.as_mut().unwrap().shrink(up);
                }
            }
        }
        if !grew {
            break;
        }
    }
    presentation
}

fn changed(snapshot: &ReviewSnapshot, side: &Excerpt<'_>, number: usize) -> bool {
    let file = snapshot.patch.lines[side.selected[0].source_patch_line].file;
    snapshot.blocks.iter().any(|block| {
        Some(block.file) == file
            && block.kind == side.selected[0].kind
            && number >= block.start_line
            && number - block.start_line < block.line_count
    })
}

fn render(
    section: &ReviewSection,
    old: Option<&Excerpt<'_>>,
    new: Option<&Excerpt<'_>>,
    shared: bool,
) -> WalkthroughDiff {
    let mut rows = vec![
        format!("--- {}", section.old_path.as_deref().unwrap_or("/dev/null")),
        format!("+++ {}", section.new_path.as_deref().unwrap_or("/dev/null")),
    ];
    let mut focus_start = usize::MAX;
    let mut focus_end = 0;
    let mut selected = |rows: &mut Vec<String>, side: &Excerpt<'_>| {
        focus_start = focus_start.min(rows.len() + 1);
        for line in &side.selected {
            rows.push(format!("{}{}", if side.old { '-' } else { '+' }, line.text));
        }
        focus_end = rows.len();
    };
    if shared {
        let range = |side: Option<&Excerpt<'_>>| {
            side.map_or((0, 0), |side| {
                (
                    side.start() - side.before,
                    side.selected.len() + side.before + side.after,
                )
            })
        };
        let (old_start, old_count) = range(old);
        let (new_start, new_count) = range(new);
        rows.push(format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@"
        ));
        if let Some(side) = old.or(new) {
            append_context(&mut rows, side, true);
        }
        for side in [old, new].into_iter().flatten() {
            selected(&mut rows, side);
        }
        if let Some(side) = old.or(new) {
            append_context(&mut rows, side, false);
        }
    } else {
        for side in [old, new].into_iter().flatten() {
            let start = side.start() - side.before;
            let count = side.selected.len() + side.before + side.after;
            let location = if side.old {
                format!("-{start},{count} +0,0")
            } else {
                format!("-0,0 +{start},{count}")
            };
            rows.push(format!(
                "@@ {location} @@ {} source: {}:{start}–{}",
                if side.old { "before" } else { "after" },
                side.path,
                side.end() + side.after,
            ));
            append_context(&mut rows, side, true);
            selected(&mut rows, side);
            append_context(&mut rows, side, false);
        }
    }
    WalkthroughDiff {
        old_path: section.old_path.clone(),
        new_path: section.new_path.clone(),
        patch: rows.join("\n") + "\n",
        focus: (focus_start, focus_end),
    }
}

fn append_context(rows: &mut Vec<String>, side: &Excerpt<'_>, up: bool) {
    let range = if up {
        side.start() - side.before..side.start()
    } else {
        side.end() + 1..side.end() + side.after + 1
    };
    for number in range {
        let text = side
            .source
            .and_then(|source| source.line(number))
            .expect("captured context");
        rows.push(format!(" {text}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_diff::{review_snapshot, ChangeRef, DiffPairing, ReviewBase};
    use git2::{IndexAddOption, Repository, Signature};
    use std::fs;

    fn snapshot(before: &[(&str, &str)], after: &[(&str, &str)]) -> ReviewSnapshot {
        let directory = tempfile::tempdir().unwrap();
        let repo = Repository::init(directory.path()).unwrap();
        for (path, text) in before {
            fs::write(directory.path().join(path), text).unwrap();
        }
        let mut index = repo.index().unwrap();
        index.add_all(["*"], IndexAddOption::DEFAULT, None).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = Signature::now("Test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "Before", &tree, &[])
            .unwrap();
        for (path, _) in before {
            fs::remove_file(directory.path().join(path)).unwrap();
        }
        for (path, text) in after {
            fs::write(directory.path().join(path), text).unwrap();
        }
        review_snapshot(directory.path(), &ReviewBase::explicit("HEAD")).unwrap()
    }

    fn section(
        snapshot: &ReviewSnapshot,
        old: Option<(usize, usize)>,
        new: Option<(usize, usize)>,
    ) -> ReviewSection {
        let reference = |kind, slice: Option<(usize, usize)>| {
            slice.map(|(offset, count)| ChangeRef {
                block_id: snapshot
                    .blocks
                    .iter()
                    .find(|block| block.kind == kind)
                    .unwrap()
                    .id
                    .clone(),
                offset: Some(offset),
                count: Some(count),
            })
        };
        snapshot
            .reassign(&[DiffPairing {
                label: None,
                message: None,
                old: reference(PatchLineKind::Removed, old),
                new: reference(PatchLineKind::Added, new),
                related_to: None,
            }])
            .unwrap()
            .sections
            .remove(0)
    }

    #[test]
    fn href_replacement_has_shared_frozen_source_and_exact_coordinates() {
        let before = "<nav>\n  <a\n    href=\"/old\"\n  >Home</a>\n</nav>\n";
        let after = before.replace("/old", "/home");
        let snapshot = snapshot(&[("nav.html", before)], &[("nav.html", &after)]);
        let section = section(&snapshot, Some((0, 1)), Some((0, 1)));
        let view = walkthrough_diff(&snapshot, &section, |_| true);
        assert_eq!(view.patch, "--- nav.html\n+++ nav.html\n@@ -1,5 +1,5 @@\n <nav>\n   <a\n-    href=\"/old\"\n+    href=\"/home\"\n   >Home</a>\n </nav>\n");
        assert_eq!(view.focus, (6, 7));
    }

    #[test]
    fn partial_slice_preserves_focus_and_labels_neighboring_snapshot_text() {
        let snapshot = snapshot(
            &[("file.rs", "start\nold one\nold two\nend\n")],
            &[("file.rs", "start\nnew one\nnew two\nend\n")],
        );
        let section = section(&snapshot, Some((0, 1)), Some((0, 1)));
        let view = walkthrough_diff(&snapshot, &section, |_| true);
        assert!(view.patch.contains(
            "@@ -1,4 +0,0 @@ before source: file.rs:1–4\n start\n-old one\n old two\n end"
        ));
        assert!(view.patch.contains(
            "@@ -0,0 +1,4 @@ after source: file.rs:1–4\n start\n+new one\n new two\n end"
        ));
        assert!(!view.patch.contains("-old two"));
        assert!(!view.patch.contains("+new two"));
        let mut highlighter =
            crate::syntax::SyntaxHighlighter::new(crate::syntax::Language::Diff).unwrap();
        highlighter.parse(&view.patch);
        let highlights = highlighter.highlights_for_all_lines(&view.patch);
        for (row, text) in view.patch.lines().enumerate() {
            let expected = if text == "-old one" {
                Some(crate::syntax::HighlightGroup::DiffRemoved)
            } else if text == "+new one" {
                Some(crate::syntax::HighlightGroup::DiffAdded)
            } else {
                None
            };
            if let Some(expected) = expected {
                assert!(highlights[row].iter().any(|(_, group)| *group == expected));
            }
        }
    }

    #[test]
    fn cross_file_pairing_never_claims_shared_unchanged_context() {
        let snapshot = snapshot(
            &[
                ("old.rs", "same\nremoved\nend\n"),
                ("new.rs", "same\nend\n"),
            ],
            &[("old.rs", "same\nend\n"), ("new.rs", "same\nadded\nend\n")],
        );
        let section = section(&snapshot, Some((0, 1)), Some((0, 1)));
        let view = walkthrough_diff(&snapshot, &section, |_| true);
        assert!(view
            .patch
            .contains("@@ -1,3 +0,0 @@ before source: old.rs:1–3"));
        assert!(view
            .patch
            .contains("@@ -0,0 +1,3 @@ after source: new.rs:1–3"));
        assert_eq!(view.patch.matches(" same\n").count(), 2);
    }

    #[test]
    fn addition_and_deletion_pages_have_surrounding_source() {
        let snapshot = snapshot(
            &[("file.rs", "start\nold\nend\n")],
            &[("file.rs", "start\nnew\nend\n")],
        );
        for (old, new, label, marker) in [
            (Some((0, 1)), None, "before", "-old"),
            (None, Some((0, 1)), "after", "+new"),
        ] {
            let section = section(&snapshot, old, new);
            let view = walkthrough_diff(&snapshot, &section, |_| true);
            assert!(view
                .patch
                .contains(&format!("@@ {label} source: file.rs:1–3")));
            assert!(view.patch.contains(&format!(" start\n{marker}\n end\n")));
        }
    }

    #[test]
    fn context_is_optional_and_respects_line_and_visual_row_budgets() {
        let snapshot = snapshot(
            &[(
                "file.rs",
                "a very long context line that wraps\nold\nshort\n",
            )],
            &[(
                "file.rs",
                "a very long context line that wraps\nnew\nshort\n",
            )],
        );
        let section = section(&snapshot, Some((0, 1)), Some((0, 1)));
        let minimal = walkthrough_diff(&snapshot, &section, |patch| patch.lines().count() <= 5);
        assert_eq!(minimal.patch.lines().count(), 5);
        assert!(minimal.patch.contains("@@ -2,1 +2,1 @@"));
        let visual_rows = |patch: &str| {
            patch
                .lines()
                .map(|line| crate::wrap::visual_line_count(line, 24, 4))
                .sum::<usize>()
        };
        let budget = visual_rows(&minimal.patch) + 1;
        let view = walkthrough_diff(&snapshot, &section, |patch| visual_rows(patch) <= budget);
        assert!(view.patch.contains(" short\n"));
        assert!(!view.patch.contains("very long context"));
        assert!(visual_rows(&view.patch) <= budget);
    }

    #[test]
    fn missing_or_gapped_frozen_sources_never_invent_context() {
        let mut snapshot = snapshot(
            &[("file.rs", "uncaptured\nbefore\nold\nafter\n")],
            &[("file.rs", "uncaptured\nbefore\nnew\nafter\n")],
        );
        let section = section(&snapshot, Some((0, 1)), Some((0, 1)));
        for entry in &mut snapshot.sources {
            for source in [&mut entry.old, &mut entry.new].into_iter().flatten() {
                source.windows[0].lines.remove(0);
                source.windows[0].start_line = 2;
                source.complete = false;
            }
        }
        let view = walkthrough_diff(&snapshot, &section, |_| true);
        assert!(view.patch.contains("@@ -2,3 +2,3 @@"));
        assert!(!view.patch.contains("uncaptured"));
        snapshot.sources.clear();
        let view = walkthrough_diff(&snapshot, &section, |_| true);
        assert_eq!(view.patch.lines().count(), 5);
        assert!(view.patch.contains("@@ -3,1 +3,1 @@\n-old\n+new\n"));
    }
}
