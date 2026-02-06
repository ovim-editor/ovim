; C# highlight query
;
; Based on tree-sitter-c-sharp grammar which supports C# 1 through 13.0.

; Keywords
[
  "abstract"
  "add"
  "alias"
  "as"
  "async"
  "await"
  "base"
  "break"
  "case"
  "catch"
  "checked"
  "class"
  "const"
  "continue"
  "default"
  "delegate"
  "do"
  "else"
  "enum"
  "event"
  "explicit"
  "extern"
  "finally"
  "fixed"
  "for"
  "foreach"
  "get"
  "global"
  "goto"
  "if"
  "implicit"
  "in"
  "init"
  "interface"
  "internal"
  "is"
  "lock"
  "namespace"
  "new"
  "operator"
  "out"
  "override"
  "params"
  "partial"
  "private"
  "protected"
  "public"
  "readonly"
  "record"
  "ref"
  "remove"
  "required"
  "return"
  "scoped"
  "sealed"
  "set"
  "sizeof"
  "stackalloc"
  "static"
  "struct"
  "switch"
  "this"
  "throw"
  "try"
  "typeof"
  "unchecked"
  "unsafe"
  "using"
  "value"
  "var"
  "virtual"
  "void"
  "volatile"
  "when"
  "where"
  "while"
  "with"
  "yield"
] @keyword

; Type declarations
(class_declaration
  name: (identifier) @type)

(interface_declaration
  name: (identifier) @type)

(struct_declaration
  name: (identifier) @type)

(enum_declaration
  name: (identifier) @type)

(record_declaration
  name: (identifier) @type)

(delegate_declaration
  name: (identifier) @type)

; Type references
(type_identifier) @type
(predefined_type) @type
(nullable_type) @type
(generic_name) @type

; Namespace
(namespace_declaration
  name: (identifier) @namespace)

(qualified_name) @namespace

; Method declarations
(method_declaration
  name: (identifier) @function)

(local_function_statement
  name: (identifier) @function)

(constructor_declaration
  name: (identifier) @function)

(destructor_declaration
  name: (identifier) @function)

; Method invocations
(invocation_expression
  function: (identifier) @function)

(invocation_expression
  function: (member_access_expression
    name: (identifier) @function))

; Property declarations
(property_declaration
  name: (identifier) @property)

; Field declarations
(field_declaration
  (variable_declaration
    (variable_declarator
      (identifier) @property)))

; Event declarations
(event_declaration
  name: (identifier) @property)

; Parameter declarations
(parameter
  name: (identifier) @parameter)

; Attribute
(attribute) @attribute

; String literals
(string_literal) @string
(verbatim_string_literal) @string
(raw_string_literal) @string
(interpolated_string_expression) @string
(character_literal) @string

; Numbers
(integer_literal) @number
(real_literal) @number

; Boolean literals
(boolean_literal) @constant

; Null literal
(null_literal) @constant

; Comments
(comment) @comment

; XML doc comments
(xml_comment) @comment

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
  "??"
  "??="
  "=>"
  "?"
  ":"
  "::"
  "->"
] @operator

; Punctuation
["(" ")" "[" "]" "{" "}" "<" ">"] @punctuation.bracket
["." "," ";" ":"] @punctuation.delimiter

; LINQ keywords
[
  "from"
  "join"
  "on"
  "equals"
  "into"
  "let"
  "orderby"
  "ascending"
  "descending"
  "select"
  "group"
  "by"
] @keyword

; Pattern matching
(switch_expression) @keyword
(switch_expression_arm) @keyword

; Lambda expressions
(lambda_expression) @function

; using directive
(using_directive
  (identifier) @namespace)
