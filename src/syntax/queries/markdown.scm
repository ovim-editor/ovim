; Headings
(atx_heading) @markup.heading
(setext_heading) @markup.heading

; Horizontal rules
(thematic_break) @punctuation.special

; Lists and blockquotes
(list_marker_plus) @punctuation.special
(list_marker_minus) @punctuation.special
(list_marker_star) @punctuation.special
(block_quote_marker) @punctuation.special

; Code
(fenced_code_block
  (info_string) @keyword)
(fenced_code_block
  (code_fence_content) @markup.raw)
(indented_code_block) @markup.raw
(code_span) @markup.raw

; Links and images
(link_title) @string
(link_destination) @string.special
(link_text) @string.special
(autolink) @string.special
(image
  (link_text)? @string.special
  (link_destination)? @string.special)

; Emphasis (italic) - *text* or _text_
; Note: These require tree-sitter-md's inline grammar for full support
(emphasis) @markup.italic

; Strong emphasis (bold) - **text** or __text__
(strong_emphasis) @markup.bold

; Strikethrough - ~~text~~
(strikethrough) @markup.strikethrough
