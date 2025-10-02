; Keywords - use valid keyword tokens from tree-sitter-rust
[
  "as"
  "async"
  "await"
  "break"
  "const"
  "continue"
  "else"
  "enum"
  "fn"
  "for"
  "if"
  "impl"
  "in"
  "let"
  "loop"
  "match"
  "mod"
  "move"
  "pub"
  "ref"
  "return"
  "static"
  "struct"
  "trait"
  "type"
  "unsafe"
  "use"
  "where"
  "while"
] @keyword

; Mutable specifier
(mutable_specifier) @keyword

; Function definitions
(function_item name: (identifier) @function)
(function_signature_item name: (identifier) @function)

; Function calls
(call_expression function: (identifier) @function)
(call_expression function: (field_expression field: (field_identifier) @function))

; Methods
(call_expression function: (scoped_identifier name: (identifier) @function))

; Types
(type_identifier) @type
(primitive_type) @type
(generic_type type: (type_identifier) @type)

; Strings
(string_literal) @string
(raw_string_literal) @string
(char_literal) @string

; Numbers
(integer_literal) @number
(float_literal) @number

; Booleans
(boolean_literal) @constant

; Comments
(line_comment) @comment
(block_comment) @comment

; Operators
[
  "="
  "+"
  "-"
  "*"
  "/"
  "%"
  "=="
  "!="
  "<"
  ">"
  "<="
  ">="
  "&&"
  "||"
  "!"
  "&"
  "|"
  "^"
  "<<"
  ">>"
  "+="
  "-="
  "*="
  "/="
  "->"
  "=>"
] @operator

; Macros
(macro_invocation macro: (identifier) @macro)

; Parameters
(parameter pattern: (identifier) @parameter)

; Constants
(const_item name: (identifier) @constant)
(static_item name: (identifier) @constant)

; Punctuation
["(" ")" "[" "]" "{" "}" "<" ">"] @punctuation
