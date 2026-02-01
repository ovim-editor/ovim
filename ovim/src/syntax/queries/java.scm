; Keywords
[
  "abstract"
  "assert"
  "break"
  "case"
  "catch"
  "class"
  "continue"
  "default"
  "do"
  "else"
  "enum"
  "exports"
  "extends"
  "final"
  "finally"
  "for"
  "if"
  "implements"
  "import"
  "instanceof"
  "interface"
  "module"
  "native"
  "new"
  "open"
  "opens"
  "package"
  "private"
  "protected"
  "provides"
  "public"
  "requires"
  "return"
  "static"
  "strictfp"
  "switch"
  "synchronized"
  "throw"
  "throws"
  "to"
  "transient"
  "transitive"
  "try"
  "uses"
  "volatile"
  "while"
  "with"
] @keyword

; Modifiers
(modifiers) @keyword

; Method declarations
(method_declaration
  name: (identifier) @function)

; Method invocations
(method_invocation
  name: (identifier) @function)

; Constructor declarations
(constructor_declaration
  name: (identifier) @function)

; Class declarations
(class_declaration
  name: (identifier) @type)

; Interface declarations
(interface_declaration
  name: (identifier) @type)

; Enum declarations
(enum_declaration
  name: (identifier) @type)

; Annotation type declarations
(annotation_type_declaration
  name: (identifier) @type)

; Type references
(type_identifier) @type

; Generic types
(generic_type
  (type_identifier) @type)

; Array types
(array_type
  element: (type_identifier) @type)

; Primitive types
[
  (integral_type)
  (floating_point_type)
  (boolean_type)
  (void_type)
] @type

; Field declarations
(field_declaration
  declarator: (variable_declarator
    name: (identifier) @property))

; Field access
(field_access
  field: (identifier) @property)

; Parameters
(formal_parameter
  name: (identifier) @parameter)

; Catch parameters
(catch_formal_parameter
  name: (identifier) @parameter)

; Lambda parameters
(inferred_parameters
  (identifier) @parameter)

; String literals
(string_literal) @string
(character_literal) @string

; Numbers
(decimal_integer_literal) @number
(hex_integer_literal) @number
(octal_integer_literal) @number
(binary_integer_literal) @number
(decimal_floating_point_literal) @number
(hex_floating_point_literal) @number

; Booleans
(true) @constant
(false) @constant
(null_literal) @constant

; Comments
(line_comment) @comment
(block_comment) @comment

; Annotations
(annotation
  name: (identifier) @decorator)
(marker_annotation
  name: (identifier) @decorator)

; This/Super
(this) @keyword
(super) @keyword

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
  "&="
  "|="
  "^="
  "<<="
  ">>="
  ">>>="
  "++"
  "--"
  "?"
  ":"
  "->"
  "::"
] @operator

; Punctuation
["(" ")" "[" "]" "{" "}" "<" ">"] @punctuation
["." "," ";" ":"] @punctuation

; Package and import statements
(package_declaration
  (scoped_identifier) @namespace)

(import_declaration
  (scoped_identifier) @namespace)

; Constants (static final fields)
(field_declaration
  (modifiers
    "static"
    "final")?
  declarator: (variable_declarator
    name: (identifier) @constant))
