; TSX (TypeScript + JSX) syntax highlighting

; ============================================================================
; Keywords
; ============================================================================
[
  "abstract"
  "as"
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
  "get"
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
  "satisfies"
  "set"
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

; ============================================================================
; Types
; ============================================================================
; Custom type identifiers
(type_identifier) @type

; Built-in/predefined types (string, number, boolean, etc.)
(predefined_type) @type.builtin

; Generic type parameters
(type_parameter
  name: (type_identifier) @type)

; Type annotations
(type_annotation
  (type_identifier) @type)

; Interface and type alias names
(interface_declaration
  name: (type_identifier) @type)
(type_alias_declaration
  name: (type_identifier) @type)

; ============================================================================
; Functions
; ============================================================================
(function_declaration
  name: (identifier) @function)
(method_definition
  name: (property_identifier) @function)
(call_expression
  function: (identifier) @function)
(call_expression
  function: (member_expression
    property: (property_identifier) @function))

; Arrow functions with explicit names
(variable_declarator
  name: (identifier) @function
  value: (arrow_function))

; ============================================================================
; Variables
; ============================================================================
; Built-in variables/globals
(this) @variable.builtin

; Regular identifiers (lowest priority, will be overridden by more specific matches)
(identifier) @variable

; ============================================================================
; Properties
; ============================================================================
(property_identifier) @property
(shorthand_property_identifier) @property
(shorthand_property_identifier_pattern) @property

; Object keys
(pair
  key: (property_identifier) @property)
(pair
  key: (string) @property)

; ============================================================================
; Parameters
; ============================================================================
(required_parameter
  pattern: (identifier) @parameter)
(optional_parameter
  pattern: (identifier) @parameter)
(rest_pattern
  (identifier) @parameter)

; Destructuring parameters
(required_parameter
  pattern: (object_pattern
    (shorthand_property_identifier_pattern) @parameter))
(required_parameter
  pattern: (array_pattern
    (identifier) @parameter))

; ============================================================================
; Strings
; ============================================================================
(string) @string
(template_string) @string

; Template string interpolation delimiters
(template_substitution
  "${" @punctuation.delimiter
  "}" @punctuation.delimiter)

; Regex
(regex) @string

; ============================================================================
; Numbers
; ============================================================================
(number) @number

; ============================================================================
; Constants and literals
; ============================================================================
[(true) (false)] @constant
(null) @constant
(undefined) @constant

; ============================================================================
; Comments
; ============================================================================
(comment) @comment

; ============================================================================
; Operators
; ============================================================================
[
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "**="
  "<<="
  ">>="
  ">>>="
  "&="
  "^="
  "|="
  "&&="
  "||="
  "??="
  "+"
  "-"
  "*"
  "/"
  "%"
  "**"
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
  "??"
  "!"
  "~"
  "&"
  "|"
  "^"
  "<<"
  ">>"
  ">>>"
  "++"
  "--"
  "?"
  ":"
  "=>"
  "..."
] @operator

; ============================================================================
; Punctuation
; ============================================================================
["(" ")" "[" "]" "{" "}"] @punctuation
["," "." ";"] @punctuation

; ============================================================================
; JSX Elements
; ============================================================================
; JSX tag names
(jsx_opening_element
  name: (identifier) @tag)
(jsx_closing_element
  name: (identifier) @tag)
(jsx_self_closing_element
  name: (identifier) @tag)

; JSX tag delimiters - make them stand out
(jsx_opening_element
  "<" @tag.delimiter)
(jsx_opening_element
  ">" @tag.delimiter)
(jsx_closing_element
  "</" @tag.delimiter)
(jsx_closing_element
  ">" @tag.delimiter)
(jsx_self_closing_element
  "<" @tag.delimiter)
(jsx_self_closing_element
  "/>" @tag.delimiter)

; JSX attributes
(jsx_attribute
  (property_identifier) @property)

; JSX expression braces
(jsx_expression
  "{" @punctuation.delimiter
  "}" @punctuation.delimiter)

; JSX text content
(jsx_text) @string
