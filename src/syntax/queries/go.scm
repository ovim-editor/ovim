; Keywords
[
  "break"
  "case"
  "chan"
  "const"
  "continue"
  "default"
  "defer"
  "else"
  "fallthrough"
  "for"
  "func"
  "go"
  "goto"
  "if"
  "import"
  "interface"
  "map"
  "package"
  "range"
  "return"
  "select"
  "struct"
  "switch"
  "type"
  "var"
] @keyword

; Functions
(function_declaration name: (identifier) @function)
(method_declaration name: (field_identifier) @function)
(call_expression function: (identifier) @function)

; Types
(type_identifier) @type

; Strings
(interpreted_string_literal) @string
(raw_string_literal) @string

; Numbers
(int_literal) @number
(float_literal) @number

; Booleans
[(true) (false)] @constant
(nil) @constant

; Comments
(comment) @comment

; Operators
[
  ":="
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
  "<-"
] @operator

; Parameters
(parameter_declaration name: (identifier) @parameter)
