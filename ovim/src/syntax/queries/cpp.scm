; Keywords
[
  "alignas"
  "alignof"
  "break"
  "case"
  "catch"
  "class"
  "const"
  "constexpr"
  "continue"
  "decltype"
  "default"
  "delete"
  "do"
  "else"
  "enum"
  "explicit"
  "extern"
  "for"
  "friend"
  "goto"
  "if"
  "inline"
  "namespace"
  "new"
  "noexcept"
  "nullptr"
  "operator"
  "private"
  "protected"
  "public"
  "register"
  "return"
  "sizeof"
  "static"
  "static_assert"
  "static_cast"
  "struct"
  "switch"
  "template"
  "this"
  "throw"
  "try"
  "typedef"
  "typeid"
  "typename"
  "union"
  "using"
  "virtual"
  "volatile"
  "while"
] @keyword

; Types
[
  "bool"
  "char"
  "double"
  "float"
  "int"
  "long"
  "short"
  "signed"
  "unsigned"
  "void"
  "wchar_t"
] @type

(type_identifier) @type
(primitive_type) @type

; Functions
(function_declarator declarator: (identifier) @function)
(call_expression function: (identifier) @function)

; Strings
(string_literal) @string
(char_literal) @string
(raw_string_literal) @string

; Numbers
(number_literal) @number

; Booleans and constants
[(true) (false)] @constant
(null) @constant
(nullptr) @constant

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
  "::"
] @operator

; Preprocessor
(preproc_directive) @macro
(preproc_def) @macro
(preproc_include) @macro

; Parameters
(parameter_declaration declarator: (identifier) @parameter)
