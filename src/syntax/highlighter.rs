use super::theme::HighlightGroup;
use super::languages::{Language, LanguageRegistry};
use tree_sitter::{Parser, Tree, Query, QueryCursor, Point};
use std::ops::Range;

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
        parser.set_language(&ts_language)
            .map_err(|e| format!("Failed to set language: {}", e))?;

        let query = Query::new(&ts_language, query_source)
            .map_err(|e| format!("Failed to create query: {}", e))?;

        let capture_names = query.capture_names().iter().map(|s| s.to_string()).collect();

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

    /// Gets highlights for a specific line
    pub fn highlights_for_line(&self, line_idx: usize, source: &str) -> Vec<(Range<usize>, HighlightGroup)> {
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
        let matches = cursor.matches(
            &self.query,
            tree.root_node(),
            source.as_bytes(),
        );

        for m in matches {
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
                    let col_start = if start_byte >= line_start_byte {
                        start_byte - line_start_byte
                    } else {
                        0
                    };

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
    fn capture_to_highlight_group(name: &str) -> HighlightGroup {
        match name {
            "keyword" => HighlightGroup::Keyword,
            "function" => HighlightGroup::Function,
            "type" => HighlightGroup::Type,
            "string" => HighlightGroup::String,
            "number" => HighlightGroup::Number,
            "comment" => HighlightGroup::Comment,
            "operator" => HighlightGroup::Operator,
            "variable" => HighlightGroup::Variable,
            "macro" => HighlightGroup::Macro,
            "constant" => HighlightGroup::Constant,
            "property" => HighlightGroup::Property,
            "parameter" => HighlightGroup::Parameter,
            "label" => HighlightGroup::Label,
            "punctuation" => HighlightGroup::Punctuation,
            _ => HighlightGroup::Other,
        }
    }

    /// Gets the language
    pub fn language(&self) -> Language {
        self.language
    }
}
