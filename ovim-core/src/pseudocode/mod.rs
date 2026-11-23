//! Source-mapped reading projections. Providers describe syntax-aware edits;
//! this module applies them without changing the source or its line numbering.
mod java;
mod markdown;

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Java,
    Markdown,
}

#[derive(Debug)]
pub struct Projection {
    pub text: String,
    /// One entry per visible line, mapping its UTF-8 bytes to original bytes.
    pub lines: Vec<ProjectedLine>,
    source_lines: Vec<usize>,
}

#[derive(Debug)]
pub struct ProjectedLine {
    pub source_bytes: Vec<usize>,
}

impl Projection {
    pub fn build(source: &str, language: Language) -> anyhow::Result<Self> {
        anyhow::ensure!(
            source.len() <= 2 * 1024 * 1024,
            "Pseudocode supports documents up to 2 MiB"
        );
        let mut edits = Vec::new();
        match language {
            Language::Java => java::edits(source, 0, &mut edits)?,
            Language::Markdown => markdown::edits(source, &mut edits, false)?,
        }
        Ok(apply(source, edits))
    }

    /// Markdown syntax remains available to document renderers, while fenced
    /// Java uses exactly the same simplification as the terminal reading view.
    pub fn formatted_markdown(source: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            source.len() <= 2 * 1024 * 1024,
            "Pseudocode supports documents up to 2 MiB"
        );
        let mut edits = Vec::new();
        markdown::edits(source, &mut edits, true)?;
        Ok(apply(source, edits))
    }

    pub fn markdown_styles(&self, source: &str) -> anyhow::Result<crate::buffer::LineHighlights> {
        let mut by_line = vec![Vec::new(); self.source_lines.len()];
        for (range, group) in markdown::styles(source)? {
            let first = self
                .source_lines
                .partition_point(|start| *start <= range.start)
                .saturating_sub(1);
            for (line, styles) in by_line.iter_mut().enumerate().skip(first) {
                let start = self.source_lines[line];
                if start >= range.end {
                    break;
                }
                let end = self
                    .source_lines
                    .get(line + 1)
                    .copied()
                    .unwrap_or(source.len());
                styles.push((
                    range.start.max(start) - start..range.end.min(end) - start,
                    group,
                ));
            }
        }
        Ok(self.map_highlights(|line| by_line[line].clone()))
    }

    /// Map a displayed byte column to a source line and byte column.
    pub fn source_position(&self, line: usize, byte: usize) -> (usize, usize) {
        let Some(mapping) = self.lines.get(line).or_else(|| self.lines.last()) else {
            return (0, 0);
        };
        let offset = mapping
            .source_bytes
            .get(byte)
            .or_else(|| mapping.source_bytes.last())
            .copied()
            .unwrap_or(0);
        let source_line = self
            .source_lines
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        (source_line, offset - self.source_lines[source_line])
    }

    /// Project original syntax colours through the same byte map as navigation.
    pub fn map_highlights(
        &self,
        highlights: impl Fn(usize) -> Vec<(Range<usize>, crate::syntax::HighlightGroup)>,
    ) -> Vec<Vec<(Range<usize>, crate::syntax::HighlightGroup)>> {
        self.lines
            .iter()
            .enumerate()
            .map(|(line, map)| {
                let bytes = &map.source_bytes[..map.source_bytes.len().saturating_sub(1)];
                let first = self.source_position(line, 0).0;
                let last = self.source_position(line, bytes.len()).0;
                let mut result = Vec::new();
                for source_line in first..=last {
                    for (range, group) in highlights(source_line) {
                        let start = bytes.partition_point(|byte| {
                            *byte < self.source_lines[source_line] + range.start
                        });
                        let end = bytes.partition_point(|byte| {
                            *byte < self.source_lines[source_line] + range.end
                        });
                        if start < end {
                            result.push((start..end, group));
                        }
                    }
                }
                result
            })
            .collect()
    }

    pub fn view_line_for_source(&self, source_line: usize) -> usize {
        let next = self.lines.partition_point(|map| {
            let byte = map.source_bytes.first().copied().unwrap_or(0);
            let line = self
                .source_lines
                .partition_point(|start| *start <= byte)
                .saturating_sub(1);
            line < source_line
        });
        [
            next.saturating_sub(1),
            next.min(self.lines.len().saturating_sub(1)),
        ]
        .into_iter()
        .min_by_key(|line| self.source_position(*line, 0).0.abs_diff(source_line))
        .unwrap_or(0)
    }
}

#[derive(Debug)]
struct Edit {
    range: Range<usize>,
    replacement: Option<String>,
}

fn replace(edits: &mut Vec<Edit>, range: Range<usize>, replacement: &str) {
    edits.push(Edit {
        range,
        replacement: Some(replacement.into()),
    });
}

fn preserve(edits: &mut Vec<Edit>, range: Range<usize>) {
    edits.push(Edit {
        range,
        replacement: None,
    });
}

/// Remove adjacent horizontal spacing too, but never eat a source newline.
fn remove(source: &str, range: Range<usize>, base: usize, edits: &mut Vec<Edit>) {
    let end = range.end
        + source[range.end..]
            .bytes()
            .take_while(|b| matches!(b, b' ' | b'\t'))
            .count();
    replace(edits, range.start + base..end + base, "");
}

fn parse(source: &str, language: tree_sitter::Language) -> anyhow::Result<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language)?;
    parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("Could not parse pseudocode source"))
}

fn apply(source: &str, mut edits: Vec<Edit>) -> Projection {
    let mut protected: Vec<_> = edits
        .iter()
        .filter(|edit| edit.replacement.is_none())
        .map(|edit| edit.range.clone())
        .collect();
    protected.sort_by_key(|range| range.start);
    let mut protected_ranges: Vec<Range<usize>> = Vec::new();
    for range in protected {
        if let Some(last) = protected_ranges
            .last_mut()
            .filter(|last| range.start <= last.end)
        {
            last.end = last.end.max(range.end);
        } else {
            protected_ranges.push(range);
        }
    }
    edits.retain(|edit| edit.replacement.is_some());
    edits.sort_by_key(|edit| (edit.range.start, std::cmp::Reverse(edit.range.end)));
    let mut text = String::new();
    let mut mapping = Vec::new();
    let mut cursor = 0;
    for edit in edits {
        // An enclosing edit owns its range; nested provider edits are redundant.
        if edit.range.end <= cursor || edit.range.end > source.len() {
            continue;
        }
        // Horizontal-space cleanup may overlap the start of the next edit.
        // Retain its remaining replacement rather than leaking punctuation.
        let start = edit.range.start.max(cursor);
        text.push_str(&source[cursor..start]);
        mapping.extend(cursor..start);
        let replacement = edit.replacement.as_deref().unwrap_or("");
        text.push_str(replacement);
        mapping.extend(std::iter::repeat_n(start, replacement.len()));
        cursor = edit.range.end;
    }
    text.push_str(&source[cursor..]);
    mapping.extend(cursor..source.len());
    mapping.push(source.len());
    let source_lines: Vec<usize> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(byte, _)| byte + 1))
        .collect();
    let mut output = String::new();
    let mut lines = Vec::new();
    let mut offset = 0;
    let mut pending_blank = None;
    for raw in text.split_inclusive('\n') {
        let source_byte = mapping[offset];
        let source_line = source_lines
            .partition_point(|start| *start <= source_byte)
            .saturating_sub(1);
        let start = source_lines[source_line];
        let end = source_lines
            .get(source_line + 1)
            .copied()
            .unwrap_or(source.len());
        let protected_index = protected_ranges.partition_point(|range| range.end <= start);
        let verbatim = protected_ranges
            .get(protected_index)
            .is_some_and(|range| range.start < end);
        let line = if verbatim {
            raw.trim_end_matches(['\r', '\n'])
        } else {
            raw.trim_end_matches(['\r', '\n', ' ', '\t'])
        };
        if line.is_empty() && !verbatim {
            // Preserve only original paragraph spacing, not erased syntax lines.
            if source[start..end].trim().is_empty() && !lines.is_empty() {
                pending_blank = Some(source_byte);
            }
        } else {
            if let Some(byte) = pending_blank.take() {
                output.push('\n');
                lines.push(ProjectedLine {
                    source_bytes: vec![byte],
                });
            }
            output.push_str(line);
            output.push('\n');
            lines.push(ProjectedLine {
                source_bytes: mapping[offset..=offset + line.len()].to_vec(),
            });
        }
        offset += raw.len();
    }
    if lines.is_empty() {
        lines.push(ProjectedLine {
            source_bytes: vec![0],
        });
    }
    Projection {
        text: output,
        lines,
        source_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_hides_declaration_noise_and_compacts_empty_lines() {
        let source = "package example;\n\nimport java.util.List;\n\npublic class Demo {\n    @Override\n    public static int total(final List<Integer> values) {\n        int sum = 0;\n\n\n        for (Integer value : values) {\n            sum += value;\n        }\n        return sum;\n    }\n}\n";
        let view = Projection::build(source, Language::Java).unwrap();
        assert_eq!(view.text, "class Demo:\n    total(values):\n        sum = 0\n\n        for value in values:\n            sum += value\n        return sum\n");
        assert_eq!(view.source_position(2, 8), (7, 12));
        assert!(!view.text.contains("\n\n\n"));
    }

    #[test]
    fn literals_comments_arrays_and_for_separators_are_preserved() {
        let source = "class A {\n void go() {\n  String s = \"public int x; {}\";\n  // int x; {}\n  int[] a = {1, 2};\n  for (int i = 0; i < 2; i++) {\n   log(a[i]);\n  }\n  while (ready) {}\n }\n}\n";
        let view = Projection::build(source, Language::Java).unwrap();
        assert!(
            view.text.contains("s = \"public int x; {}\""),
            "{}",
            view.text
        );
        assert!(view.text.contains("// int x; {}"));
        assert!(view.text.contains("a = {1, 2}"));
        assert!(view.text.contains("for i = 0; i < 2; i++"), "{}", view.text);
        assert!(view.text.contains("while ready {}"), "{}", view.text);
    }

    #[test]
    fn markdown_simplifies_prose_and_java_fences_only() {
        let source = "# Example\n\nA **bold** [link](https://example.com) and `literal *code*`.\n\n```java\npublic int size(String name) {\n    return name.length();\n}\n```\n\n```rust\nlet x: i32 = 1;\n```\n";
        let view = Projection::build(source, Language::Markdown).unwrap();
        assert!(
            view.text
                .starts_with("Example\n\nA bold link and literal *code*."),
            "{}",
            view.text
        );
        assert!(
            view.text.contains("size(name):\n    return name.length()"),
            "{}",
            view.text
        );
        assert!(!view.text.contains("```java"));
        assert!(view.text.contains("```rust\nlet x: i32 = 1;\n```"));
        let row = view
            .text
            .lines()
            .position(|line| line.contains("return name"))
            .unwrap();
        assert_eq!(view.source_position(row, 4), (6, 4));
    }

    #[test]
    fn unicode_crlf_and_empty_documents_keep_valid_source_positions() {
        let source = "\r\nclass Café {\r\n String 名 = \"🦀\";\r\n}\r\n\r\n";
        let view = Projection::build(source, Language::Java).unwrap();
        assert_eq!(view.text, "class Café:\n 名 = \"🦀\"\n");
        let row = view.text.lines().nth(1).unwrap();
        let crab = row.find('🦀').unwrap();
        let (line, byte) = view.source_position(1, crab);
        assert_eq!(line, 2);
        assert!(source.lines().nth(line).unwrap()[byte..].starts_with('🦀'));
        for source in ["", "\n\n", "import a.B;\n"] {
            let view = Projection::build(source, Language::Java).unwrap();
            assert!(view.text.is_empty());
            assert_eq!(view.source_position(0, 0), (0, 0));
        }
    }

    #[test]
    fn incomplete_java_is_preserved_instead_of_guessed() {
        let source = "class A {\n void go() {\n  String x = \"unfinished\n";
        let view = Projection::build(source, Language::Java).unwrap();
        assert!(view.text.contains("unfinished"));
        assert!(view.text.contains("String x"), "{}", view.text);
        let unfinished = "class A {\n String text = \"\"\"\nfirst  \n\n\nlast\n";
        let view = Projection::build(unfinished, Language::Java).unwrap();
        assert!(view.text.contains("first  \n\n\nlast"), "{}", view.text);
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;

    #[test]
    fn java_text_blocks_keep_significant_empty_lines_and_trailing_spaces() {
        let source = "class A {\n String text = \"\"\"\n  first  \n\n\n  last\n  \"\"\";\n}\n";
        let view = Projection::build(source, Language::Java).unwrap();
        assert!(
            view.text.contains("  first  \n\n\n  last\n  \"\"\""),
            "{}",
            view.text
        );
    }

    #[test]
    fn foreign_markdown_fences_preserve_whitespace_verbatim() {
        let source = "# Python\n\n```python\nx = '''first  \n\n\nlast'''\n```\n";
        let view = Projection::build(source, Language::Markdown).unwrap();
        assert!(view
            .text
            .contains("```python\nx = '''first  \n\n\nlast'''\n```\n"));
    }

    #[test]
    fn multiline_for_headers_and_empty_loop_bodies_keep_their_meaning() {
        let source = "class A {\n void go() {\n  for (int i = 0;\n       i < 3;\n       i++) { work(); }\n  while (waiting);\n  int[] values = new int[3];\n }\n}\n";
        let view = Projection::build(source, Language::Java).unwrap();
        assert!(
            view.text.contains("for i = 0;\n       i < 3;\n       i++"),
            "{}",
            view.text
        );
        assert!(
            view.text.contains("while waiting{}") || view.text.contains("while waiting {}"),
            "{}",
            view.text
        );
        assert!(view.text.contains("values = array[3]"), "{}", view.text);
    }

    #[test]
    fn mapped_syntax_ranges_stay_inside_unicode_view_lines() {
        let source = "class Café {\n String 名 = \"🦀\";\n}\n";
        let view = Projection::build(source, Language::Java).unwrap();
        let mut highlighter =
            crate::syntax::SyntaxHighlighter::new(crate::syntax::Language::Java).unwrap();
        highlighter.parse(source);
        let originals = highlighter.highlights_for_all_lines(source);
        let highlights =
            view.map_highlights(|line| originals.get(line).cloned().unwrap_or_default());
        assert!(highlights.iter().any(|line| !line.is_empty()));
        for (text, highlights) in view.text.lines().zip(highlights) {
            for (range, _) in highlights {
                assert!(range.end <= text.len());
                assert!(text.is_char_boundary(range.start));
                assert!(text.is_char_boundary(range.end));
            }
        }
    }
}

#[cfg(test)]
mod declaration_tests {
    use super::*;
    #[test]
    fn inheritance_and_throws_do_not_leave_brace_noise_after_overlapping_spacing_edits() {
        let source = "public class Demo<T> extends Base implements Runnable {\n @Override\n public void run() throws Exception {\n  Object thing = new Box<String>();\n  String text = (String) thing;\n }\n}\n";
        let view = Projection::build(source, Language::Java).unwrap();
        assert_eq!(
            view.text,
            "class Demo:\n run():\n  thing = Box()\n  text = thing\n"
        );
    }
}

#[cfg(test)]
mod compact_tests {
    use super::*;
    #[test]
    fn delimiter_concealment_keeps_compact_java_tokens_separate() {
        let source =
            "class A{void f(){if(ready){go();}for(String x:items){use(x);}while(waiting);}}";
        let view = Projection::build(source, Language::Java).unwrap();
        assert!(view.text.contains("if ready{go();}"), "{}", view.text);
        assert!(
            view.text.contains("for x in items{use(x);}"),
            "{}",
            view.text
        );
        assert!(view.text.contains("while waiting {}"), "{}", view.text);
    }
    #[test]
    fn formatted_markdown_preserves_semantics_and_maps_simplified_java() {
        let source = "# Title\n\nA **bold** [reference][ref].  \nNext line.\n\n```java\nimport java.util.List;\npublic int size(String name) {\n    return name.length();\n}\n```\n\n[ref]: https://example.com\n";
        let plain = Projection::build(source, Language::Markdown).unwrap();
        let rich = Projection::formatted_markdown(source).unwrap();
        assert!(rich.text.starts_with("# Title\n"));
        assert!(rich
            .text
            .contains("A **bold** [reference][ref].  \nNext line."));
        assert!(
            rich.text
                .contains("```java\nsize(name):\n    return name.length()\n```"),
            "{}",
            rich.text
        );
        assert!(rich.text.contains("[ref]: https://example.com"));
        let line = rich
            .text
            .lines()
            .position(|line| line.contains("return"))
            .unwrap();
        assert_eq!(rich.source_position(line, 4), (8, 4));
        let mapped = plain.view_line_for_source(rich.source_position(line, 0).0);
        assert_eq!(plain.source_position(mapped, 4), (8, 4));
        let prose = plain
            .text
            .lines()
            .position(|line| line.contains("bold"))
            .unwrap();
        assert!(plain.markdown_styles(source).unwrap()[prose]
            .iter()
            .any(|(range, group)| {
                *group == crate::syntax::HighlightGroup::MarkupBold
                    && &plain.text.lines().nth(prose).unwrap()[range.clone()] == "bold"
            }));
    }

    #[test]
    fn nearest_source_row_uses_earlier_row_for_ties() {
        let view = Projection::build("class A {\n\n\n int x;\n}\n", Language::Java).unwrap();
        for source_line in 0..8 {
            let expected = (0..view.lines.len())
                .min_by_key(|line| view.source_position(*line, 0).0.abs_diff(source_line))
                .unwrap();
            assert_eq!(view.view_line_for_source(source_line), expected);
        }
    }
}
