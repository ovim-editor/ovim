; Terraform / HCL (HashiCorp Configuration Language) highlight query
;
; Covers both Terraform (.tf, .tfvars) and generic HCL (.hcl) files.
; Based on tree-sitter-hcl grammar.

; Keywords
[
  "for"
  "in"
  "if"
  "else"
  "endif"
  "endfor"
] @keyword

; Block types (resource, data, variable, output, etc.)
(block
  (identifier) @keyword)

; Block labels (resource types, names)
(block
  (string_lit) @type)

; Attribute names
(attribute
  (identifier) @property)

; Object element keys
(object_elem
  (identifier) @property)

; Function calls
(function_call
  (identifier) @function)

; String literals
(string_lit) @string
(quoted_template) @string
(template_literal) @string

; Heredoc strings
(heredoc_template) @string
(heredoc_identifier) @label

; Numbers
(numeric_lit) @number

; Boolean literals
(bool_lit) @constant

; Null literal
(null_lit) @constant

; Variable references
(variable_expr
  (identifier) @variable)

; Attribute access
(get_attr
  (identifier) @property)

; Splat expressions
(splat) @operator
(attr_splat) @operator
(full_splat) @operator

; Index access
(index) @punctuation.bracket

; Comments
(comment) @comment

; Operators
[
  "="
  "=="
  "!="
  "<"
  ">"
  "<="
  ">="
  "+"
  "-"
  "*"
  "/"
  "%"
  "&&"
  "||"
  "!"
  "?"
  ":"
  "=>"
  "..."
] @operator

; Punctuation
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ";"] @punctuation.delimiter

; Type annotations (in variable blocks)
(type_expr) @type
