use super::Buffer;
use crate::syntax::{
    CodeBlockCache, HighlightGroup, Language, LanguageRegistry, SyntaxHighlighter,
};
use std::ops::Range;

/// Finds inline code spans (single backtick `code`) in a line of text.
/// Returns ranges covering the content between backticks (including the backticks).
/// Handles escaped backticks and double-backtick (`` ` ``) spans.
fn find_inline_code_spans(line: &str) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    // Build (char, byte_offset) pairs to track both char and byte positions
    let indexed: Vec<(usize, char)> = line.char_indices().collect();
    let len = indexed.len();
    let mut i = 0;

    while i < len {
        let (_, ch) = indexed[i];
        // Skip escaped backticks
        if ch == '\\' {
            i += 2;
            continue;
        }

        if ch == '`' {
            // Count consecutive backticks for the opening delimiter
            let open_byte_start = indexed[i].0;
            let mut backtick_count = 0;
            while i < len && indexed[i].1 == '`' {
                backtick_count += 1;
                i += 1;
            }

            // Find matching closing delimiter (same number of backticks)
            let content_start = i;
            let mut found_close = false;
            while i < len {
                if indexed[i].1 == '`' {
                    let mut close_count = 0;
                    while i < len && indexed[i].1 == '`' {
                        close_count += 1;
                        i += 1;
                    }
                    if close_count == backtick_count {
                        // Found matching close - span covers open backticks through close backticks
                        // Use byte position after the last closing backtick
                        let close_byte_end = if i < len { indexed[i].0 } else { line.len() };
                        spans.push(open_byte_start..close_byte_end);
                        found_close = true;
                        break;
                    }
                    // Not matching, continue searching
                } else {
                    i += 1;
                }
            }

            if !found_close {
                // No closing delimiter found, the backticks are literal
                i = content_start;
            }
        } else {
            i += 1;
        }
    }

    spans
}

/// Per-line syntax highlights: maps character ranges to highlight groups
pub type LineHighlights = Vec<Vec<(Range<usize>, HighlightGroup)>>;

/// Large file threshold in lines - files above this disable expensive features
const LARGE_FILE_LINES: usize = 50_000;

/// Large file threshold in bytes - files above this disable expensive features
const LARGE_FILE_BYTES: usize = 5 * 1024 * 1024; // 5MB

impl Buffer {
    /// Checks if this is a large file (exceeds line or byte threshold)
    pub fn is_large_file(&self) -> bool {
        let line_count = self.line_count();
        let byte_count = self.rope().len_bytes();

        line_count > LARGE_FILE_LINES || byte_count > LARGE_FILE_BYTES
    }

    /// Gets the large file threshold for lines
    pub fn large_file_line_threshold() -> usize {
        LARGE_FILE_LINES
    }

    /// Gets the large file threshold for bytes
    pub fn large_file_byte_threshold() -> usize {
        LARGE_FILE_BYTES
    }

    /// Enables syntax highlighting for this buffer based on file path
    /// Automatically skips large files for performance
    pub fn enable_syntax_highlighting(&mut self) {
        // Don't enable syntax for large files
        if self.is_large_file() {
            // Note: Syntax highlighting disabled for large files - don't print to stderr
            // This avoids interrupting user output
            return;
        }

        if let Some(ref path) = self.file_path {
            if let Some(lang) = LanguageRegistry::detect_from_path(path) {
                if let Ok(mut highlighter) = SyntaxHighlighter::new(lang) {
                    let source = self.rope.to_string();

                    highlighter.parse(&source);

                    // Build initial highlight cache
                    self.build_highlight_cache(&highlighter, &source);

                    self.syntax = Some(highlighter);
                    self.version += 1;
                }
            }
        }
    }

    /// Checks if syntax highlighting should be initialized (lazy loading)
    /// Returns true if the buffer has a file path with supported language but no syntax yet
    pub fn should_init_syntax(&self) -> bool {
        // Don't initialize syntax for large files
        if self.is_large_file() {
            return false;
        }

        // Already has syntax or a background task is computing it
        if self.syntax.is_some() || self.syntax_loading {
            return false;
        }

        if let Some(ref path) = self.file_path {
            LanguageRegistry::detect_from_path(path).is_some()
        } else {
            false
        }
    }

    /// Marks that a background task is computing initial syntax highlights.
    /// Prevents `should_init_syntax()` from returning true again until the task completes.
    pub fn mark_syntax_loading(&mut self) {
        self.syntax_loading = true;
    }

    /// Clears the background syntax-loading flag without applying highlights.
    pub fn clear_syntax_loading(&mut self) {
        self.syntax_loading = false;
    }

    /// Returns the current highlight version counter.
    /// Used to check if the buffer has been edited since a background parse started.
    pub fn highlight_version(&self) -> u64 {
        self.highlight_version
    }

    /// Applies pre-computed syntax highlights from a background task.
    /// Returns true if the highlights were applied (version matched).
    ///
    /// After installing the cached highlights (so the next render is styled),
    /// creates a SyntaxHighlighter on the main thread for future incremental updates.
    pub fn apply_background_syntax(
        &mut self,
        lang: Language,
        highlights: LineHighlights,
        version: u64,
    ) -> bool {
        self.syntax_loading = false;

        // Only apply if buffer hasn't been edited since the background parse started
        if self.highlight_version != version {
            return false;
        }

        // Install cached highlights so the next render shows styled text
        self.cached_highlights = Some(highlights);

        // Create a SyntaxHighlighter on the main thread for future incremental updates.
        // This parses the current content, which is fast (~5ms for typical files) and
        // doesn't cause FOUC since highlights are already cached above.
        if let Ok(mut highlighter) = SyntaxHighlighter::new(lang) {
            let source = self.rope.to_string();
            highlighter.parse(&source);

            // For markdown files, also build code block cache
            if lang == Language::Markdown {
                if let Some(tree) = highlighter.tree() {
                    let mut cache = crate::syntax::CodeBlockCache::new();
                    cache.update_from_tree(tree, &source, self.highlight_version);
                    self.code_block_cache = Some(cache);
                }
            }

            self.syntax = Some(highlighter);
            self.pending_rehighlight = false;
            self.version += 1;
        }

        true
    }

    /// Builds the highlight cache for all lines
    pub(super) fn build_highlight_cache(&mut self, highlighter: &SyntaxHighlighter, source: &str) {
        // Use the efficient single-pass method that queries the tree once
        self.cached_highlights = Some(highlighter.highlights_for_all_lines(source));

        // For markdown files, also build code block cache for language-specific highlighting
        if highlighter.language() == Language::Markdown {
            if let Some(tree) = highlighter.tree() {
                let mut cache = CodeBlockCache::new();
                cache.update_from_tree(tree, source, self.highlight_version);
                self.code_block_cache = Some(cache);
            }
        }
    }

    /// Shifts highlights after an insertion
    pub(super) fn shift_highlights_for_insertion(&mut self, line: usize, col: usize, text: &str) {
        let Some(ref mut cache) = self.cached_highlights else {
            return; // No cache to shift
        };

        if line >= cache.len() {
            return;
        }

        // Check if insertion contains newlines
        let newline_count = text.matches('\n').count();

        if newline_count == 0 {
            // Single-line insertion: shift highlights on the same line
            // TODO: Use grapheme cluster library for proper multi-codepoint emoji handling
            // Currently chars().count() splits multi-codepoint emojis incorrectly
            let char_count = text.chars().count();

            for (range, _) in &mut cache[line] {
                if range.start >= col {
                    // Highlight starts after insertion point: shift right
                    range.start += char_count;
                    range.end += char_count;
                } else if range.end > col {
                    // Highlight contains insertion point: extend end
                    range.end += char_count;
                }
            }
        } else {
            // Multi-line insertion: handle line splits and shifts
            let lines: Vec<&str> = text.split('\n').collect();
            // TODO: Use grapheme cluster library for proper multi-codepoint emoji handling
            // Currently chars().count() splits multi-codepoint emojis incorrectly
            let last_line_len = lines.last().map(|s| s.chars().count()).unwrap_or(0);

            // Split the current line's highlights at the insertion point
            let current_line_highlights = cache[line].clone();
            let mut before_insert = Vec::new();
            let mut after_insert = Vec::new();

            for (range, group) in current_line_highlights {
                if range.end <= col {
                    // Entirely before insertion
                    before_insert.push((range, group));
                } else if range.start >= col {
                    // Entirely after insertion: will move to new line
                    // Adjust column position (relative to start of new line)
                    let new_start = range.start - col + last_line_len;
                    let new_end = range.end - col + last_line_len;
                    after_insert.push((new_start..new_end, group));
                } else {
                    // Spans insertion point: keep the before part only
                    before_insert.push((range.start..col, group));
                    // The after part would be on the new line, but it's cut off
                    // (We can't split highlights perfectly without re-parsing)
                }
            }

            // Update current line with highlights before insertion
            cache[line] = before_insert;

            // Insert new empty lines for the newlines in the inserted text
            for _ in 0..newline_count {
                cache.insert(line + 1, Vec::new());
            }

            // The last new line gets the highlights that were after the insertion
            if line + newline_count < cache.len() {
                cache[line + newline_count] = after_insert;
            }
        }
    }

    /// Creates a tree-sitter InputEdit for an insertion operation
    /// This enables incremental parsing instead of full re-parse
    pub(super) fn create_ts_insert_edit(
        &self,
        line: usize,
        col: usize,
        text: &str,
    ) -> Option<tree_sitter::InputEdit> {
        let line_start = self.rope.line_to_char(line);
        let insert_pos = (line_start + col).min(self.rope.len_chars());

        let start_byte = self.rope.char_to_byte(insert_pos);
        let old_end_byte = start_byte;
        let new_end_byte = start_byte + text.len();

        // Calculate positions (row, column) for tree-sitter
        let start_position = tree_sitter::Point {
            row: line,
            column: col,
        };

        // For insertions, old_end == start
        let old_end_position = start_position;

        // Calculate new_end position based on newlines in inserted text
        let newline_count = text.matches('\n').count();
        let new_end_position = if newline_count == 0 {
            // Single-line insertion
            tree_sitter::Point {
                row: line,
                column: col + text.chars().count(),
            }
        } else {
            // Multi-line insertion
            let last_line = text.split('\n').next_back().unwrap_or("");
            tree_sitter::Point {
                row: line + newline_count,
                column: last_line.chars().count(),
            }
        };

        Some(tree_sitter::InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position,
            old_end_position,
            new_end_position,
        })
    }

    /// Creates a tree-sitter InputEdit for a deletion operation
    /// This enables incremental parsing instead of full re-parse
    pub(super) fn create_ts_delete_edit(
        &self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
        deleted_text: &str,
    ) -> Option<tree_sitter::InputEdit> {
        let start_line_char = self.rope.line_to_char(start_line);
        let start_pos = (start_line_char + start_col).min(self.rope.len_chars());

        let start_byte = self.rope.char_to_byte(start_pos);
        let old_end_byte = start_byte + deleted_text.len();
        let new_end_byte = start_byte;

        let start_position = tree_sitter::Point {
            row: start_line,
            column: start_col,
        };

        let old_end_position = tree_sitter::Point {
            row: end_line,
            column: end_col,
        };

        // For deletions, new_end == start
        let new_end_position = start_position;

        Some(tree_sitter::InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position,
            old_end_position,
            new_end_position,
        })
    }

    /// Applies an incremental tree-sitter edit to the syntax highlighter
    /// This is much faster than full re-parse for small edits
    pub(super) fn apply_incremental_syntax_edit(&mut self, edit: tree_sitter::InputEdit) {
        if let Some(ref mut syntax) = self.syntax {
            let source = self.rope.to_string();
            syntax.update(edit, &source);

            // Keep stale highlights until new ones are calculated
            // This prevents flashing (no highlights) during typing
            // The pending_rehighlight flag ensures fresh highlights will be computed
        }
    }

    /// Shifts highlights after a deletion
    pub(super) fn shift_highlights_for_deletion(
        &mut self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) {
        let Some(ref mut cache) = self.cached_highlights else {
            return; // No cache to shift
        };

        if start_line >= cache.len() {
            return;
        }

        if start_line == end_line {
            // Single-line deletion
            if start_line >= cache.len() {
                return;
            }

            let deleted_chars = end_col.saturating_sub(start_col);
            let highlights = &mut cache[start_line];

            // Filter and adjust highlights
            highlights.retain_mut(|(range, _)| {
                if range.end <= start_col {
                    // Before deletion: keep as-is
                    true
                } else if range.start >= end_col {
                    // After deletion: shift left
                    range.start = range.start.saturating_sub(deleted_chars);
                    range.end = range.end.saturating_sub(deleted_chars);
                    true
                } else if range.start >= start_col && range.end <= end_col {
                    // Entirely within deletion: remove
                    false
                } else if range.start < start_col && range.end > end_col {
                    // Contains deletion: shrink
                    range.end = start_col + (range.end - end_col);
                    true
                } else if range.start < start_col {
                    // Starts before, ends within deletion
                    range.end = start_col;
                    true
                } else {
                    // Starts within, ends after deletion
                    range.start = start_col;
                    range.end = start_col + (range.end - end_col);
                    true
                }
            });
        } else {
            // Multi-line deletion
            let deleted_lines = end_line - start_line;

            // Get highlights from end of deletion range that survive
            let surviving_highlights = if end_line < cache.len() {
                cache[end_line]
                    .iter()
                    .filter_map(|(range, group)| {
                        if range.start >= end_col {
                            // After deletion point: shift to start line
                            let new_start = start_col + (range.start - end_col);
                            let new_end = start_col + (range.end - end_col);
                            Some((new_start..new_end, *group))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };

            // Trim start line highlights
            if start_line < cache.len() {
                cache[start_line].retain(|(range, _)| range.end <= start_col);
                // Add surviving highlights from end line
                cache[start_line].extend(surviving_highlights);
            }

            // Remove deleted lines
            if start_line + 1 < cache.len() {
                let end = (start_line + deleted_lines + 1).min(cache.len());
                cache.drain(start_line + 1..end);
            }
        }
    }

    /// Gets syntax highlights for a specific line
    /// Returns a list of (column_range, highlight_group) tuples
    ///
    /// Priority order:
    /// 1. Code block cache (for markdown code fences with known languages)
    /// 2. Semantic highlights (from LSP)
    /// 3. Tree-sitter cached highlights
    /// 4. Inline code overlay (for markdown: backtick `code` spans)
    pub fn highlights_for_line(&self, line_idx: usize) -> Vec<(Range<usize>, HighlightGroup)> {
        // For markdown: check code block cache first (language-specific highlighting)
        if let Some(ref code_cache) = self.code_block_cache {
            if let Some(highlights) = code_cache.highlights_for_line(line_idx) {
                return highlights.clone();
            }
        }

        // Prefer semantic highlights from LSP if available
        if let Some(ref semantic) = self.semantic_highlights {
            if line_idx < semantic.len() && !semantic[line_idx].is_empty() {
                return semantic[line_idx].clone();
            }
        }

        // Get base highlights from tree-sitter cache
        let mut highlights = if let Some(ref cache) = self.cached_highlights {
            if line_idx < cache.len() {
                cache[line_idx].clone()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // For markdown files: overlay inline code spans (backtick `code`)
        // Only for lines NOT inside fenced code blocks
        if self.code_block_cache.is_some() {
            let in_code_block = self
                .code_block_cache
                .as_ref()
                .map_or(false, |c| c.is_line_in_code_block(line_idx));

            if !in_code_block {
                if let Some(line_text) = self.rope.line(line_idx).as_str() {
                    let inline_spans = find_inline_code_spans(line_text);
                    for span in inline_spans {
                        highlights.push((span, HighlightGroup::MarkupRaw));
                    }
                } else {
                    // Fallback for lines that cross chunk boundaries
                    let line_text: String = self.rope.line(line_idx).chars().collect();
                    let inline_spans = find_inline_code_spans(&line_text);
                    for span in inline_spans {
                        highlights.push((span, HighlightGroup::MarkupRaw));
                    }
                }
            }
        }

        highlights
    }

    /// Sets semantic highlights decoded from LSP semantic tokens
    pub fn set_semantic_highlights(
        &mut self,
        highlights: Vec<Vec<(Range<usize>, HighlightGroup)>>,
    ) {
        self.semantic_highlights = Some(highlights);
    }

    /// Clears semantic highlights (e.g., when LSP disconnects)
    pub fn clear_semantic_highlights(&mut self) {
        self.semantic_highlights = None;
    }

    /// Checks if semantic highlights are available
    pub fn has_semantic_highlights(&self) -> bool {
        self.semantic_highlights.is_some()
    }

    /// Decodes LSP semantic tokens into highlight spans
    /// The legend provides the mapping from token type indices to names
    pub fn decode_semantic_tokens(
        &mut self,
        tokens: &lsp_types::SemanticTokens,
        legend: &lsp_types::SemanticTokensLegend,
    ) {
        let line_count = self.line_count();
        let mut highlights: Vec<Vec<(Range<usize>, HighlightGroup)>> = vec![Vec::new(); line_count];

        // Semantic tokens use relative positions (delta encoding)
        let mut current_line: u32 = 0;
        let mut current_char: u32 = 0;

        for token in &tokens.data {
            // Update position based on deltas
            if token.delta_line > 0 {
                current_line += token.delta_line;
                current_char = token.delta_start;
            } else {
                current_char += token.delta_start;
            }

            let line = current_line as usize;
            if line >= line_count {
                break;
            }

            let start_col = current_char as usize;
            let end_col = start_col + token.length as usize;
            let token_type = token.token_type as usize;

            // Map token type to HighlightGroup
            let highlight_group = if token_type < legend.token_types.len() {
                Self::lsp_token_type_to_highlight_group(legend.token_types[token_type].as_str())
            } else {
                HighlightGroup::Other
            };

            highlights[line].push((start_col..end_col, highlight_group));
        }

        self.semantic_highlights = Some(highlights);
    }

    /// Maps LSP semantic token type names to HighlightGroup
    fn lsp_token_type_to_highlight_group(token_type: &str) -> HighlightGroup {
        match token_type {
            "namespace" | "module" => HighlightGroup::Type,
            "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" => {
                HighlightGroup::Type
            }
            "parameter" => HighlightGroup::Parameter,
            "variable" | "property" | "enumMember" => HighlightGroup::Variable,
            "function" | "method" | "member" => HighlightGroup::Function,
            "macro" | "decorator" => HighlightGroup::Macro,
            "keyword" | "modifier" => HighlightGroup::Keyword,
            "comment" => HighlightGroup::Comment,
            "string" | "regexp" => HighlightGroup::String,
            "number" => HighlightGroup::Number,
            "operator" => HighlightGroup::Operator,
            "label" | "event" => HighlightGroup::Label,
            _ => HighlightGroup::Other,
        }
    }

    /// Checks if syntax highlighting is enabled
    pub fn has_syntax_highlighting(&self) -> bool {
        self.syntax.is_some()
    }

    /// Returns a reference to the treesitter syntax tree, if available.
    pub fn syntax_tree(&self) -> Option<&tree_sitter::Tree> {
        self.syntax.as_ref().and_then(|s| s.tree())
    }

    /// Checks if re-highlighting is needed
    pub fn needs_rehighlight(&self) -> bool {
        self.pending_rehighlight && self.syntax.is_some()
    }

    /// Gets data needed for re-highlighting (content, version, language)
    pub fn get_rehighlight_data(&self) -> Option<(String, u64, Language)> {
        if !self.needs_rehighlight() {
            return None;
        }

        let syntax = self.syntax.as_ref()?;
        let content = self.rope.to_string();
        let version = self.highlight_version;
        let language = syntax.language();

        Some((content, version, language))
    }

    /// Rebuilds highlight cache from the existing syntax highlighter
    /// This uses the incrementally-updated parse tree, so it's fast!
    /// The tree-sitter parse tree was already updated incrementally via `update()`,
    /// so this just queries it for highlights (no re-parsing needed).
    pub fn rebuild_highlight_cache(&mut self) -> Option<u64> {
        if !self.needs_rehighlight() {
            return None;
        }

        let syntax = self.syntax.as_ref()?;
        let content = self.rope.to_string();
        let version = self.highlight_version;

        // Query the incrementally-updated parse tree for highlights
        // This is fast because tree-sitter already updated the tree via InputEdit
        let highlights = syntax.highlights_for_all_lines(&content);

        self.cached_highlights = Some(highlights);

        // For markdown files, also rebuild code block cache
        if syntax.language() == Language::Markdown {
            if let Some(tree) = syntax.tree() {
                let mut cache = CodeBlockCache::new();
                cache.update_from_tree(tree, &content, version);
                self.code_block_cache = Some(cache);
            }
        }

        self.pending_rehighlight = false;

        Some(version)
    }

    /// Rebuilds highlight cache for only the viewport lines (start_line..end_line).
    /// Does NOT clear pending_rehighlight — the full rebuild is still needed for off-screen content.
    pub fn rebuild_viewport_highlight_cache(&mut self, start_line: usize, end_line: usize) {
        let syntax = match self.syntax.as_ref() {
            Some(s) => s,
            None => return,
        };

        let content = self.rope.to_string();
        let line_count = self.line_count();
        let actual_end = end_line.min(line_count);
        if start_line >= actual_end {
            return;
        }

        let viewport_highlights =
            syntax.highlights_for_line_range(&content, start_line, actual_end);

        // Ensure the cache exists and is the right size
        let cache = self
            .cached_highlights
            .get_or_insert_with(|| vec![Vec::new(); line_count]);
        // Resize if needed (buffer may have grown/shrunk)
        cache.resize_with(line_count, Vec::new);

        // Overwrite only the viewport portion
        for (i, highlights) in viewport_highlights.into_iter().enumerate() {
            let line_idx = start_line + i;
            if line_idx < cache.len() {
                cache[line_idx] = highlights;
            }
        }
    }

    /// Applies re-highlighted results if version matches
    pub fn apply_highlights(
        &mut self,
        highlights: Vec<Vec<(Range<usize>, HighlightGroup)>>,
        version: u64,
    ) -> bool {
        // Only apply if version matches (buffer hasn't changed since re-parse started)
        if self.highlight_version == version {
            self.cached_highlights = Some(highlights);
            self.pending_rehighlight = false;
            true
        } else {
            false
        }
    }
}
