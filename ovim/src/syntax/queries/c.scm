; Keywords
[
  "break"
  "case"
  "const"
  "continue"
  "default"
  "do"
  "else"
  "enum"
  "extern"
  "for"
  "goto"
  "if"
  "inline"
  "register"
  "return"
  "sizeof"
  "static"
  "struct"
  "switch"
  "typedef"
  "union"
  "volatile"
  "while"
] @keyword

; Types
[
  "char"
  "double"
  "float"
  "int"
  "long"
  "short"
  "signed"
  "unsigned"
  "void"
] @type

(type_identifier) @type
(primitive_type) @type

; Functions
(function_declarator declarator: (identifier) @function)
(call_expression function: (identifier) @function)

; Strings
(string_literal) @string
(char_literal) @string

; Numbers
(number_literal) @number

; Booleans and constants
[(true) (false)] @constant
(null) @constant

; Comments
(comment) @comment

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
  "++"
  "--"
  "->"
] @operator

; Preprocessor
(preproc_directive) @macro
(preproc_def) @macro
(preproc_include) @macro

; Parameters
(parameter_declaration declarator: (identifier) @parameter)
