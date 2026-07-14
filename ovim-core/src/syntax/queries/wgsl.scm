; Literals
(int_literal) @number
(float_literal) @number
(bool_literal) @constant

; Names. Specific captures below override the general identifier capture.
(identifier) @variable

(function_declaration
  (identifier) @function)

(parameter
  (variable_identifier_declaration
    (identifier) @parameter))

(struct_declaration
  (identifier) @type)

(struct_declaration
  (struct_member
    (variable_identifier_declaration
      (identifier) @property)))

(type_declaration) @type

(type_constructor_or_function_call_expression
  (type_declaration) @function)

(attribute
  (identifier) @macro)

; Keywords
[
  "bitcast"
  "break"
  "case"
  "continue"
  "continuing"
  "default"
  "discard"
  "else"
  "enable"
  "fallthrough"
  "fn"
  "for"
  "if"
  "let"
  "loop"
  "override"
  "return"
  "struct"
  "switch"
  "type"
  "var"
  "while"
  (access_mode)
  (address_space)
  (texel_format)
] @keyword

; Bevy shader preprocessor extensions
[
  "#define_import_path"
  "#else"
  "#endif"
  "#ifdef"
  "#import"
] @macro

(import_path) @string

; Comments
(line_comment) @comment
(block_comment) @comment

; Operators
[
  "!"
  "!="
  "%"
  "%="
  "&"
  "&&"
  "&="
  "*"
  "*="
  "+"
  "++"
  "+="
  "-"
  "--"
  "-="
  "->"
  "/"
  "/="
  "<"
  "<<"
  "<="
  "="
  "=="
  ">"
  ">="
  ">>"
  "@"
  "^"
  "^="
  "|"
  "|="
  "||"
  "~"
] @operator

; Delimiters
[
  ","
  "."
  ":"
  ";"
] @punctuation.delimiter

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket
