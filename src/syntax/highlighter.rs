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

        // Distribute captures to lines in a single pass
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let start_byte = node.start_byte();
                let end_byte = node.end_byte();

                // Get capture name and highlight group
                let capture_name = &self.capture_names[capture.index as usize];
                let group = Self::capture_to_highlight_group(capture_name);

                // Find which lines this capture spans
                for (line_idx, &line_start_byte) in line_start_bytes.iter().enumerate() {
                    if line_idx >= lines.len() {
                        break;
                    }

                    let line_end_byte = line_start_byte + lines[line_idx].len();

                    // Check if this capture overlaps with this line
                    if start_byte < line_end_byte && end_byte > line_start_byte {
                        // Convert to column range relative to line start
                        // Cap col_start at 0 (can't be before line start)
                        let col_start = start_byte.saturating_sub(line_start_byte);

                        // Cap col_end at line length (can't extend past line end)
                        let line_len = lines[line_idx].len();
                        let col_end = if end_byte <= line_end_byte {
                            end_byte - line_start_byte
                        } else {
                            line_len
                        };

                        line_highlights[line_idx].push((col_start..col_end, group));
                    }
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

        // Calculate line byte range
        let lines: Vec<&str> = source.lines().collect();
        if line_idx >= lines.len() {
            return highlights;
        }

        let line_start_byte: usize = lines.iter().take(line_idx).map(|l| l.len() + 1).sum();
        let line_end_byte = line_start_byte + lines[line_idx].len();

        // Query captures in this line
        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let start_byte = node.start_byte();
                let end_byte = node.end_byte();

                // Check if this capture overlaps with our line
                if start_byte < line_end_byte && end_byte > line_start_byte {
                    // Get capture name (e.g., "keyword", "function")
                    let capture_name = &self.capture_names[capture.index as usize];
                    let group = Self::capture_to_highlight_group(capture_name);

                    // Convert to column range relative to line start
                    let col_start = start_byte.saturating_sub(line_start_byte);

                    let col_end = if end_byte <= line_end_byte {
                        end_byte - line_start_byte
                    } else {
                        line_end_byte - line_start_byte
                    };

                    highlights.push((col_start..col_end, group));
                }
            }
        }

        // Sort by start position
        highlights.sort_by_key(|(range, _)| range.start);

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
        let mut h = SyntaxHighlighter::new(Language::Tsx)
            .expect("TSX highlighter should be created");

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
}
