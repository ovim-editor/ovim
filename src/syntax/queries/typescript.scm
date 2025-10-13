; Keywords
[
  "abstract"
  "async"
  "await"
  "break"
  "case"
  "catch"
  "class"
  "const"
  "continue"
  "debugger"
  "declare"
  "default"
  "delete"
  "do"
  "else"
  "enum"
  "export"
  "extends"
  "finally"
  "for"
  "from"
  "function"
  "if"
  "implements"
  "import"
  "in"
  "instanceof"
  "interface"
  "let"
  "namespace"
  "new"
  "of"
  "private"
  "protected"
  "public"
  "readonly"
  "return"
  "static"
  "switch"
  "throw"
  "try"
  "type"
  "typeof"
  "var"
  "void"
  "while"
  "with"
  "yield"
] @keyword

; Types
(type_identifier) @type
(predefined_type) @type

; Functions
(function_declaration name: (identifier) @function)
(method_definition name: (property_identifier) @function)
(call_expression function: (identifier) @function)

; Strings
(string) @string
(template_string) @string

; Numbers
(number) @number

; Booleans and constants
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
(required_parameter (identifier) @parameter)
(optional_parameter (identifier) @parameter)
