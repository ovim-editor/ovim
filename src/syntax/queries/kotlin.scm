; Keywords
[
  "abstract"
  "actual"
  "annotation"
  "as"
  "as?"
  "break"
  "by"
  "catch"
  "class"
  "companion"
  "const"
  "constructor"
  "continue"
  "crossinline"
  "data"
  "delegate"
  "do"
  "dynamic"
  "else"
  "enum"
  "expect"
  "external"
  "field"
  "file"
  "final"
  "finally"
  "for"
  "fun"
  "get"
  "if"
  "import"
  "in"
  "infix"
  "init"
  "inline"
  "inner"
  "interface"
  "internal"
  "is"
  "lateinit"
  "noinline"
  "null"
  "object"
  "open"
  "operator"
  "out"
  "override"
  "package"
  "param"
  "private"
  "property"
  "protected"
  "public"
  "receiver"
  "reified"
  "return"
  "sealed"
  "set"
  "setparam"
  "super"
  "suspend"
  "tailrec"
  "this"
  "throw"
  "try"
  "typealias"
  "typeof"
  "val"
  "var"
  "vararg"
  "when"
  "where"
  "while"
] @keyword

; Function declarations
(function_declaration
  (simple_identifier) @function)

; Lambda expressions
(lambda_literal) @function

; Function calls
(call_expression
  (simple_identifier) @function)

; Navigation suffixes for method calls
(navigation_expression
  (navigation_suffix
    (simple_identifier) @function))

; Class declarations
(class_declaration
  (type_identifier) @type)

; Object declarations
(object_declaration
  (type_identifier) @type)

; Interface declarations
(interface_declaration
  (type_identifier) @type)

; Type references
(type_identifier) @type
(user_type
  (type_identifier) @type)

; Nullable types
(nullable_type
  (type_identifier) @type)

; Primitive types (not directly available in Kotlin, but often used)
; Kotlin uses object types, but these are commonly used
; If the grammar doesn't define these, remove this section

; Parameters
(parameter
  (simple_identifier) @parameter)

; Properties
(property_declaration
  (variable_declaration
    (simple_identifier) @property))

; String literals
(line_string_literal) @string
(multi_line_string_literal) @string

; String interpolation
(string_content) @string

; Character literals
(character_literal) @string

; Numbers
(integer_literal) @number
(hex_literal) @number
(bin_literal) @number
(real_literal) @number

; Booleans
(boolean_literal) @constant

; Null
"null" @constant

; Comments
(line_comment) @comment
(multiline_comment) @comment

; Annotations
(annotation
  (user_type
    (type_identifier) @decorator))

(single_annotation
  (user_type
    (type_identifier) @decorator))

; This/Super
"this" @keyword
"super" @keyword

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
  "~"
  "<<"
  ">>"
  ">>>"
  "+="
  "-="
  "*="
  "/="
  "%="
  "++"
  "--"
  "?"
  ":"
  "?:"
  "?."
  "!!"
  ".."
  "->"
  "=>"
  "::"
  "!in"
  "!is"
  "as"
  "as?"
  "in"
  "is"
] @operator

; Punctuation
["(" ")" "[" "]" "{" "}" "<" ">"] @punctuation
["." "," ";" ":"] @punctuation

; Package declarations
(package_header
  (identifier) @namespace)

; Import statements
(import_header
  (identifier) @namespace)
