; tree-sitter-md (block grammar) highlighting queries
; Note: This uses the block grammar which parses block-level structure.
; Inline elements (emphasis, bold, links) are NOT parsed as separate nodes.
; Inline code (`code`) is handled by find_inline_code_spans() in highlighting.rs.

; Headings - capture the inline text content specifically
(atx_heading (inline) @markup.heading)
(setext_heading (paragraph) @markup.heading)

; Heading markers (# ## ### etc)
(atx_h1_marker) @punctuation.special
(atx_h2_marker) @punctuation.special
(atx_h3_marker) @punctuation.special
(atx_h4_marker) @punctuation.special
(atx_h5_marker) @punctuation.special
(atx_h6_marker) @punctuation.special

; Horizontal rules
(thematic_break) @punctuation.special

; Lists
(list_marker_plus) @punctuation.special
(list_marker_minus) @punctuation.special
(list_marker_star) @punctuation.special

; Blockquotes
(block_quote_marker) @punctuation.special

; Fenced code blocks
(fenced_code_block
  (fenced_code_block_delimiter) @punctuation.special)
(fenced_code_block
  (info_string
    (language) @keyword))
(fenced_code_block
  (code_fence_content) @markup.raw)

; Indented code blocks
(indented_code_block) @markup.raw

; Note: Tables (pipe_table) are NOT supported by tree-sitter-md's block grammar.
; The block grammar only parses block-level structure, not GFM extensions.

; Links and images (block-level link definitions)
(link_title) @string
(link_destination) @string.special
