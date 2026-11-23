//! Merge conflict markers in buffer text: find them, and resolve one block by
//! keeping our side, their side, both, or neither.
//!
//! Handles the default style and `diff3` (`|||||||` base section):
//!
//! ```text
//! <<<<<<< HEAD
//! ours
//! ||||||| base
//! the common ancestor
//! =======
//! theirs
//! >>>>>>> feature
//! ```

/// Line indices (0-based) of one conflict block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The `<<<<<<<` line.
    pub start: usize,
    /// The `|||||||` line, in diff3 style.
    pub base: Option<usize>,
    /// The `=======` line.
    pub separator: usize,
    /// The `>>>>>>>` line.
    pub end: usize,
    /// Text after `<<<<<<<` (usually `HEAD`).
    pub ours_label: String,
    /// Text after `>>>>>>>` (usually the merged branch).
    pub theirs_label: String,
}

impl Conflict {
    pub fn contains(&self, line: usize) -> bool {
        line >= self.start && line <= self.end
    }

    /// Line range of our side, `[from, to)`.
    pub fn ours(&self) -> (usize, usize) {
        (self.start + 1, self.base.unwrap_or(self.separator))
    }

    /// Line range of their side, `[from, to)`.
    pub fn theirs(&self) -> (usize, usize) {
        (self.separator + 1, self.end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Ours,
    Theirs,
    Both,
    Neither,
}

impl Resolution {
    pub fn label(self) -> &'static str {
        match self {
            Resolution::Ours => "ours",
            Resolution::Theirs => "theirs",
            Resolution::Both => "both sides",
            Resolution::Neither => "neither side",
        }
    }
}

fn marker(line: &str, symbol: char) -> Option<&str> {
    let rest = line.strip_prefix(&symbol.to_string().repeat(7))?;
    // Exactly seven markers: an eighth marks something else (e.g. a heading).
    if rest.starts_with(symbol) {
        return None;
    }
    if rest.is_empty() {
        Some("")
    } else {
        rest.strip_prefix(' ').map(str::trim)
    }
}

/// Every complete conflict block in `lines`, in order. Unterminated blocks are
/// ignored.
pub fn find_conflicts<S: AsRef<str>>(lines: &[S]) -> Vec<Conflict> {
    let mut conflicts = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let Some(ours_label) = marker(lines[index].as_ref(), '<') else {
            index += 1;
            continue;
        };
        let start = index;
        let mut base = None;
        let mut separator = None;
        let mut end = None;
        let mut theirs_label = String::new();
        let mut cursor = start + 1;
        while cursor < lines.len() {
            let line = lines[cursor].as_ref();
            if marker(line, '<').is_some() {
                // A new block starts before this one ended: this one is broken.
                break;
            }
            if separator.is_none() && base.is_none() && marker(line, '|').is_some() {
                base = Some(cursor);
            } else if separator.is_none() && marker(line, '=').is_some() {
                separator = Some(cursor);
            } else if separator.is_some() {
                if let Some(label) = marker(line, '>') {
                    end = Some(cursor);
                    theirs_label = label.to_string();
                    break;
                }
            }
            cursor += 1;
        }
        match (separator, end) {
            (Some(separator), Some(end)) => {
                conflicts.push(Conflict {
                    start,
                    base,
                    separator,
                    end,
                    ours_label: ours_label.to_string(),
                    theirs_label,
                });
                index = end + 1;
            }
            _ => index = start + 1,
        }
    }
    conflicts
}

/// The lines that replace the whole block for `resolution`.
pub fn resolved_lines<S: AsRef<str>>(
    lines: &[S],
    conflict: &Conflict,
    resolution: Resolution,
) -> Vec<String> {
    let slice = |(from, to): (usize, usize)| {
        lines[from..to]
            .iter()
            .map(|line| line.as_ref().to_string())
            .collect::<Vec<_>>()
    };
    match resolution {
        Resolution::Ours => slice(conflict.ours()),
        Resolution::Theirs => slice(conflict.theirs()),
        Resolution::Both => {
            let mut both = slice(conflict.ours());
            both.extend(slice(conflict.theirs()));
            both
        }
        Resolution::Neither => Vec::new(),
    }
}

/// The conflict containing `line`, else the next one after it, else the first.
pub fn conflict_at_or_after(conflicts: &[Conflict], line: usize) -> Option<&Conflict> {
    conflicts
        .iter()
        .find(|conflict| conflict.contains(line))
        .or_else(|| conflicts.iter().find(|conflict| conflict.start > line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    const SIMPLE: &str = "before
<<<<<<< HEAD
ours 1
ours 2
=======
theirs 1
>>>>>>> feature
after";

    #[test]
    fn finds_a_simple_block_with_labels() {
        let text = lines(SIMPLE);
        let conflicts = find_conflicts(&text);
        assert_eq!(conflicts.len(), 1);
        let conflict = &conflicts[0];
        assert_eq!(
            (conflict.start, conflict.separator, conflict.end),
            (1, 4, 6)
        );
        assert_eq!(conflict.base, None);
        assert_eq!(conflict.ours_label, "HEAD");
        assert_eq!(conflict.theirs_label, "feature");
        assert_eq!(conflict.ours(), (2, 4));
        assert_eq!(conflict.theirs(), (5, 6));
    }

    #[test]
    fn finds_diff3_blocks_and_several_blocks() {
        let text = lines(
            "<<<<<<< HEAD
a
||||||| base
b
=======
c
>>>>>>> topic
mid
<<<<<<< HEAD
x
=======
y
>>>>>>> topic",
        );
        let conflicts = find_conflicts(&text);
        assert_eq!(conflicts.len(), 2);
        assert_eq!(conflicts[0].base, Some(2));
        assert_eq!(conflicts[0].ours(), (1, 2), "the base section is not ours");
        assert_eq!(conflicts[1].start, 8);
    }

    #[test]
    fn resolves_ours_theirs_both_and_neither() {
        let text = lines(SIMPLE);
        let conflict = &find_conflicts(&text)[0];
        assert_eq!(
            resolved_lines(&text, conflict, Resolution::Ours),
            vec!["ours 1", "ours 2"]
        );
        assert_eq!(
            resolved_lines(&text, conflict, Resolution::Theirs),
            vec!["theirs 1"]
        );
        assert_eq!(
            resolved_lines(&text, conflict, Resolution::Both),
            vec!["ours 1", "ours 2", "theirs 1"]
        );
        assert!(resolved_lines(&text, conflict, Resolution::Neither).is_empty());
    }

    #[test]
    fn ignores_incomplete_blocks_and_lookalikes() {
        assert!(find_conflicts(&lines("<<<<<<< HEAD\nonly ours\n")).is_empty());
        assert!(find_conflicts(&lines("========\n<<<<<<<< x\n>>>>>>>>> y")).is_empty());
        // A broken block does not swallow a following good one.
        let text = lines("<<<<<<< a\nx\n<<<<<<< b\ny\n=======\nz\n>>>>>>> c");
        let conflicts = find_conflicts(&text);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].start, 2);
    }

    #[test]
    fn conflict_lookup_prefers_the_block_under_the_cursor_then_the_next() {
        let text = lines(&format!("{SIMPLE}\nmid\n{SIMPLE}"));
        let conflicts = find_conflicts(&text);
        assert_eq!(conflict_at_or_after(&conflicts, 3).unwrap().start, 1);
        assert_eq!(conflict_at_or_after(&conflicts, 7).unwrap().start, 10);
        assert!(conflict_at_or_after(&conflicts, 30).is_none());
    }
}
