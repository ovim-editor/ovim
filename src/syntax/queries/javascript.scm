; Keywords
[
  "async"
  "await"
  "break"
  "case"
  "catch"
  "class"
  "const"
  "continue"
  "debugger"
  "default"
  "delete"
  "do"
  "else"
  "export"
  "extends"
  "finally"
  "for"
  "from"
  "function"
  "if"
  "import"
  "in"
  "instanceof"
  "let"
  "new"
  "of"
  "return"
  "static"
  "switch"
  "throw"
  "try"
  "typeof"
  "var"
  "void"
  "while"
  "with"
  "yield"
] @keyword

; Functions
(function_declaration name: (identifier) @function)
(method_definition name: (property_identifier) @function)
(call_expression function: (identifier) @function)

; Arrow functions and function expressions don't have names to capture directly,
; but we can highlight them through their parent structures

; Strings
(string) @string
(template_string) @string

; Numbers
(number) @number

; Booleans
[(true) (false)] @constant
(null) @constant
(undefined) @constant

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
  "==="
  "!="
  "!=="
  "<"
  ">"
  "<="
  ">="
  "&&"
  "||"
  "!"
  "?"
  ":"
  "=>"
] @operator

; Properties
(property_identifier) @property

; Parameters
(formal_parameters (identifier) @parameter)
