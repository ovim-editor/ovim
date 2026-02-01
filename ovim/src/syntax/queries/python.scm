; Keywords
[
  "and"
  "as"
  "assert"
  "async"
  "await"
  "break"
  "class"
  "continue"
  "def"
  "del"
  "elif"
  "else"
  "except"
  "finally"
  "for"
  "from"
  "global"
  "if"
  "import"
  "in"
  "is"
  "lambda"
  "nonlocal"
  "not"
  "or"
  "pass"
  "raise"
  "return"
  "try"
  "while"
  "with"
  "yield"
] @keyword

; Function definitions
(function_definition name: (identifier) @function)

; Function calls
(call function: (identifier) @function)
(call function: (attribute attribute: (identifier) @function))

; Classes
(class_definition name: (identifier) @type)

; Strings
(string) @string

; Numbers
(integer) @number
(float) @number

; Booleans
[(true) (false)] @constant
(none) @constant

; Comments
(comment) @comment

; Operators
[
  "="
  "+"
  "-"
  "*"
  "/"
  "//"
  "%"
  "**"
  "=="
  "!="
  "<"
  ">"
  "<="
  ">="
  "and"
  "or"
  "not"
  "in"
  "is"
] @operator

; Parameters
(parameters (identifier) @parameter)

; Decorators
(decorator) @macro
