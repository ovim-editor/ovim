use super::languages::{Language, LanguageRegistry};
use super::theme::HighlightGroup;
use std::ops::Range;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor, Tree};

/// Syntax highlighter using tree-sitter
pub struct SyntaxHighlighter {
    language: Language,
    parser: Parser,
    tree: Option<Tree>,
    query: Query,
    capture_names: Vec<String>,
}

impl SyntaxHighlighter {
    /// Creates a new syntax highlighter for the given language
    pub fn new(language: Language) -> Result<Self, String> {
        let ts_language = LanguageRegistry::get_tree_sitter_language(language);
        let query_source = LanguageRegistry::get_highlight_query(language);

        let mut parser = Parser::new();
        parser
            .set_language(&ts_language)
            .map_err(|e| format!("Failed to set language: {}", e))?;

        let query = Query::new(&ts_language, query_source)
            .map_err(|e| format!("Failed to create query: {}", e))?;

        let capture_names = query
            .capture_names()
            .iter()
            .map(|s| s.to_string())
            .collect();

        Ok(Self {
            language,
            parser,
            tree: None,
            query,
            capture_names,
        })
    }

    /// Parses the given source code
    pub fn parse(&mut self, source: &str) {
        self.tree = self.parser.parse(source, None);
    }

    /// Updates the syntax tree after an edit
    pub fn update(&mut self, edit: tree_sitter::InputEdit, source: &str) {
        if let Some(ref mut tree) = self.tree {
            tree.edit(&edit);
            self.tree = self.parser.parse(source, Some(tree));
        } else {
            self.parse(source);
        }
    }

    /// Gets highlights for all lines at once (more efficient than per-line)
    /// This queries the syntax tree ONCE and distributes highlights to lines
    pub fn highlights_for_all_lines(
        &self,
        source: &str,
    ) -> Vec<Vec<(Range<usize>, HighlightGroup)>> {
        let Some(ref tree) = self.tree else {
            return Vec::new();
        };

        let lines: Vec<&str> = source.lines().collect();
        let mut line_highlights: Vec<Vec<(Range<usize>, HighlightGroup)>> =
            vec![Vec::new(); lines.len()];

        // Calculate line byte offsets once
        let mut line_start_bytes = Vec::with_capacity(lines.len());
        let mut offset = 0;
        for line in &lines {
            line_start_bytes.push(offset);
            offset += line.len() + 1; // +1 for newline
        }

        // Query the tree ONCE for all matches
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());

        // Precompute line end bytes for binary search
        let line_end_bytes: Vec<usize> = lines
            .iter()
            .zip(line_start_bytes.iter())
            .map(|(line, &start)| start + line.len())
            .collect();

        // Distribute captures to lines using binary search
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let start_byte = node.start_byte();
                let end_byte = node.end_byte();

                // Get capture name and highlight group
                let capture_name = &self.capture_names[capture.index as usize];
                let group = Self::capture_to_highlight_group(capture_name);

                // Binary search to find the first line that could overlap.
                // A line overlaps if its end byte > start_byte.
                let first_line = line_end_bytes
                    .partition_point(|&line_end| line_end <= start_byte);

                // Walk forward from there until lines no longer overlap
                for line_idx in first_line..lines.len() {
                    let line_start_byte = line_start_bytes[line_idx];

                    // Past the capture — no more lines can overlap
                    if line_start_byte >= end_byte {
                        break;
                    }

                    // Convert to column range relative to line start
                    let col_start = start_byte.saturating_sub(line_start_byte);
                    let line_len = lines[line_idx].len();
                    let line_end_byte = line_start_byte + line_len;
                    let col_end = if end_byte <= line_end_byte {
                        end_byte - line_start_byte
                    } else {
                        line_len
                    };

                    line_highlights[line_idx].push((col_start..col_end, group));
                }
            }
        }

        // Sort each line's highlights by start position
        for highlights in &mut line_highlights {
            highlights.sort_by_key(|(range, _)| range.start);
        }

        line_highlights
    }

    /// Gets highlights for a range of lines (viewport-aware query)
    /// Uses `set_point_range()` to restrict tree-sitter to only the visible lines.
    /// Returns a Vec indexed from 0 = start_line, with length (end_line - start_line).
    pub fn highlights_for_line_range(
        &self,
        source: &str,
        start_line: usize,
        end_line: usize,
    ) -> Vec<Vec<(Range<usize>, HighlightGroup)>> {
        let Some(ref tree) = self.tree else {
            return Vec::new();
        };

        let range_len = end_line.saturating_sub(start_line);
        if range_len == 0 {
            return Vec::new();
        }

        let lines: Vec<&str> = source.lines().collect();
        let actual_end = end_line.min(lines.len());
        let actual_len = actual_end.saturating_sub(start_line);
        if actual_len == 0 {
            return Vec::new();
        }

        let mut line_highlights: Vec<Vec<(Range<usize>, HighlightGroup)>> =
            vec![Vec::new(); actual_len];

        // Calculate byte offsets for lines in range
        let mut line_start_bytes = Vec::with_capacity(lines.len());
        let mut offset = 0;
        for line in &lines {
            line_start_bytes.push(offset);
            offset += line.len() + 1; // +1 for newline
        }

        // Restrict query cursor to the viewport line range
        let mut cursor = QueryCursor::new();
        cursor.set_point_range(
            tree_sitter::Point {
                row: start_line,
                column: 0,
            }..tree_sitter::Point {
                row: actual_end,
                column: 0,
            },
        );

        // Precompute line end bytes for the range (binary search target)
        let line_end_bytes: Vec<usize> = (start_line..actual_end)
            .map(|i| line_start_bytes[i] + lines[i].len())
            .collect();

        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let start_byte = node.start_byte();
                let end_byte = node.end_byte();

                let capture_name = &self.capture_names[capture.index as usize];
                let group = Self::capture_to_highlight_group(capture_name);

                // Binary search within the viewport range to find first overlapping line.
                // A line overlaps if its end byte > start_byte.
                let first_rel = line_end_bytes
                    .partition_point(|&line_end| line_end <= start_byte);

                for rel_idx in first_rel..actual_len {
                    let line_idx = start_line + rel_idx;
                    let line_start_byte = line_start_bytes[line_idx];

                    if line_start_byte >= end_byte {
                        break;
                    }

                    let col_start = start_byte.saturating_sub(line_start_byte);
                    let line_len = lines[line_idx].len();
                    let line_end_byte = line_start_byte + line_len;
                    let col_end = if end_byte <= line_end_byte {
                        end_byte - line_start_byte
                    } else {
                        line_len
                    };

                    line_highlights[rel_idx].push((col_start..col_end, group));
                }
            }
        }

        // Sort each line's highlights by start position
        for highlights in &mut line_highlights {
            highlights.sort_by_key(|(range, _)| range.start);
        }

        line_highlights
    }

    /// Gets highlights for a specific line
    /// Note: For bulk operations, use highlights_for_all_lines() which is much faster
    pub fn highlights_for_line(
        &self,
        line_idx: usize,
        source: &str,
    ) -> Vec<(Range<usize>, HighlightGroup)> {
        let Some(ref tree) = self.tree else {
            return Vec::new();
        };

        let mut highlights = Vec::new();
        let mut cursor = QueryCursor::new();

        // Calculate line byte range by scanning bytes directly (avoids collecting all lines)
        let mut line_start_byte = 0;
        for (i, line) in source.lines().enumerate() {
            if i == line_idx {
                let line_end_byte = line_start_byte + line.len();

                // Restrict query to just this line
                cursor.set_point_range(
                    tree_sitter::Point {
                        row: line_idx,
                        column: 0,
                    }..tree_sitter::Point {
                        row: line_idx + 1,
                        column: 0,
                    },
                );

                let mut matches =
                    cursor.matches(&self.query, tree.root_node(), source.as_bytes());

                while let Some(m) = matches.next() {
                    for capture in m.captures {
                        let node = capture.node;
                        let start_byte = node.start_byte();
                        let end_byte = node.end_byte();

                        if start_byte < line_end_byte && end_byte > line_start_byte {
                            let capture_name = &self.capture_names[capture.index as usize];
                            let group = Self::capture_to_highlight_group(capture_name);

                            let col_start = start_byte.saturating_sub(line_start_byte);
                            let col_end = if end_byte <= line_end_byte {
                                end_byte - line_start_byte
                            } else {
                                line.len()
                            };

                            highlights.push((col_start..col_end, group));
                        }
                    }
                }

                highlights.sort_by_key(|(range, _)| range.start);
                return highlights;
            }
            line_start_byte += line.len() + 1; // +1 for newline
        }

        // line_idx out of bounds
        highlights
    }

    /// Converts a tree-sitter capture name to a highlight group
    /// Supports hierarchical names with specific handling (e.g., "type.builtin" -> TypeBuiltin)
    fn capture_to_highlight_group(name: &str) -> HighlightGroup {
        // Try exact match first (most common cases)
        match name {
            "keyword" => return HighlightGroup::Keyword,
            "function" => return HighlightGroup::Function,
            "type" => return HighlightGroup::Type,
            "type.builtin" => return HighlightGroup::TypeBuiltin,
            "string" => return HighlightGroup::String,
            "number" => return HighlightGroup::Number,
            "comment" => return HighlightGroup::Comment,
            "operator" => return HighlightGroup::Operator,
            "variable" => return HighlightGroup::Variable,
            "variable.builtin" => return HighlightGroup::VariableBuiltin,
            "macro" => return HighlightGroup::Macro,
            "constant" => return HighlightGroup::Constant,
            "property" => return HighlightGroup::Property,
            "parameter" => return HighlightGroup::Parameter,
            "label" => return HighlightGroup::Label,
            "punctuation" => return HighlightGroup::Punctuation,
            "punctuation.delimiter" => return HighlightGroup::PunctuationDelimiter,
            "punctuation.bracket" => return HighlightGroup::Punctuation,
            "tag" => return HighlightGroup::Tag,
            "tag.delimiter" => return HighlightGroup::PunctuationDelimiter,
            "constructor" => return HighlightGroup::Constructor,
            // Markup-specific captures for markdown
            "markup.italic" => return HighlightGroup::MarkupItalic,
            "markup.bold" => return HighlightGroup::MarkupBold,
            "markup.heading" => return HighlightGroup::MarkupHeading,
            "markup.raw" => return HighlightGroup::MarkupRaw,
            _ => {}
        }

        // If no exact match, try hierarchical fallback
        // For "comment.documentation", try "comment"
        if let Some(base) = name.split('.').next() {
            match base {
                "keyword" => return HighlightGroup::Keyword,
                "function" => return HighlightGroup::Function,
                "type" => return HighlightGroup::Type,
                "string" => return HighlightGroup::String,
                "number" => return HighlightGroup::Number,
                "comment" => return HighlightGroup::Comment,
                "operator" => return HighlightGroup::Operator,
                "variable" => return HighlightGroup::Variable,
                "macro" => return HighlightGroup::Macro,
                "constant" => return HighlightGroup::Constant,
                "property" => return HighlightGroup::Property,
                "parameter" => return HighlightGroup::Parameter,
                "label" => return HighlightGroup::Label,
                "punctuation" => return HighlightGroup::Punctuation,
                "tag" => return HighlightGroup::Tag,
                "constructor" => return HighlightGroup::Constructor,
                // Markup fallback - e.g., "markup.emphasis" -> markup
                "markup" => return HighlightGroup::MarkupItalic,
                _ => {}
            }
        }

        HighlightGroup::Other
    }

    /// Gets the language
    pub fn language(&self) -> Language {
        self.language
    }

    /// Gets the parse tree (if parsed)
    /// Used for extracting code blocks from markdown
    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tsx_highlighter() {
        let mut h =
            SyntaxHighlighter::new(Language::Tsx).expect("TSX highlighter should be created");

        let tsx_code = r#"const Button = ({ onClick }: Props) => {
  return (
    <button className="btn" onClick={onClick}>
      Click me
    </button>
  );
};"#;

        h.parse(tsx_code);
        let highlights = h.highlights_for_all_lines(tsx_code);

        assert!(!highlights.is_empty(), "Should have highlights");
        // Check that we have highlights on multiple lines
        let non_empty_lines = highlights.iter().filter(|h| !h.is_empty()).count();
        assert!(non_empty_lines >= 3, "Should highlight multiple lines");
    }

    #[test]
    fn test_typescript_highlighter() {
        let mut h = SyntaxHighlighter::new(Language::TypeScript)
            .expect("TypeScript highlighter should be created");

        let ts_code = r#"interface User {
  name: string;
  age: number;
}

const user: User = { name: "Alice", age: 30 };"#;

        h.parse(ts_code);
        let highlights = h.highlights_for_all_lines(ts_code);

        assert!(!highlights.is_empty(), "Should have highlights");
    }

    #[test]
    fn test_jsx_comment_brace_not_comment_colored() {
        // Regression: JSX comment braces {/* */} should not appear as comments.
        // The `}` was highlighted as PunctuationDelimiter which shared Comment's color.
        let mut h =
            SyntaxHighlighter::new(Language::Tsx).expect("TSX highlighter should be created");

        let code = "<div>\n  {/* comment */}\n</div>";

        h.parse(code);
        let highlights = h.highlights_for_all_lines(code);

        let line1_text = code.lines().nth(1).unwrap();
        let line1 = &highlights[1];

        // The closing } should be Punctuation (not PunctuationDelimiter, which is comment-colored)
        let closing_brace_byte = line1_text.len() - 1;
        let brace_highlight = line1
            .iter()
            .filter(|(range, _)| range.contains(&closing_brace_byte))
            .min_by_key(|(range, _)| range.end - range.start)
            .map(|(_, group)| *group);
        assert_eq!(
            brace_highlight,
            Some(HighlightGroup::Punctuation),
            "Closing brace of JSX comment should be Punctuation, not PunctuationDelimiter"
        );

        // Opening { too
        let opening_brace_byte = 2; // "  {" -> byte 2
        let brace_highlight = line1
            .iter()
            .filter(|(range, _)| range.contains(&opening_brace_byte))
            .min_by_key(|(range, _)| range.end - range.start)
            .map(|(_, group)| *group);
        assert_eq!(
            brace_highlight,
            Some(HighlightGroup::Punctuation),
            "Opening brace of JSX comment should be Punctuation, not PunctuationDelimiter"
        );
    }

    #[test]
    fn test_javascript_highlighter() {
        let mut h = SyntaxHighlighter::new(Language::JavaScript)
            .expect("JavaScript highlighter should be created");

        let js_code = r#"function greet(name) {
  return `Hello, ${name}!`;
}

const result = greet("World");"#;

        h.parse(js_code);
        let highlights = h.highlights_for_all_lines(js_code);

        assert!(!highlights.is_empty(), "Should have highlights");
    }

    #[test]
    fn test_line_range_matches_all_lines() {
        // Verify highlights_for_line_range produces the same results as the
        // corresponding slice of highlights_for_all_lines.
        let mut h =
            SyntaxHighlighter::new(Language::Rust).expect("Rust highlighter should be created");

        let source = "fn foo() {\n    let x = 42;\n    let y = \"hello\";\n    x + y\n}\n";
        h.parse(source);

        let all = h.highlights_for_all_lines(source);

        // Query a middle range
        let range = h.highlights_for_line_range(source, 1, 4);
        assert_eq!(range.len(), 3, "Should cover lines 1..4");
        for i in 0..3 {
            assert_eq!(
                range[i], all[1 + i],
                "Line range[{}] should match all_lines[{}]",
                i,
                1 + i
            );
        }
    }

    #[test]
    fn test_per_line_matches_all_lines() {
        // Verify highlights_for_line produces the same results as
        // highlights_for_all_lines for each line.
        let mut h =
            SyntaxHighlighter::new(Language::Rust).expect("Rust highlighter should be created");

        let source = "use std::io;\nfn main() {\n    println!(\"hi\");\n}\n";
        h.parse(source);

        let all = h.highlights_for_all_lines(source);
        for (i, expected) in all.iter().enumerate() {
            let per_line = h.highlights_for_line(i, source);
            assert_eq!(
                &per_line, expected,
                "highlights_for_line({}) should match highlights_for_all_lines",
                i
            );
        }
    }

    #[test]
    fn test_multiline_capture_distributed_correctly() {
        // A multi-line string literal should produce highlights on every
        // line it spans — this exercises the binary search path.
        let mut h =
            SyntaxHighlighter::new(Language::Rust).expect("Rust highlighter should be created");

        let source = "let s = \"\nline2\nline3\n\";\n";
        h.parse(source);

        let all = h.highlights_for_all_lines(source);
        // The string spans lines 0-3. Each should have at least one highlight.
        for (i, line_h) in all.iter().enumerate().take(4) {
            assert!(
                !line_h.is_empty(),
                "Line {} should have highlights from the multi-line string",
                i
            );
        }
    }

    #[test]
    fn test_out_of_bounds_line_returns_empty() {
        let mut h =
            SyntaxHighlighter::new(Language::Rust).expect("Rust highlighter should be created");
        let source = "fn main() {}";
        h.parse(source);

        assert!(h.highlights_for_line(999, source).is_empty());
        assert!(h.highlights_for_line_range(source, 5, 10).is_empty());
    }
}
