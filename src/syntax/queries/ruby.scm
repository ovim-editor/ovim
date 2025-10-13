; Keywords
[
  "alias"
  "and"
  "begin"
  "break"
  "case"
  "class"
  "def"
  "defined?"
  "do"
  "else"
  "elsif"
  "end"
  "ensure"
  "for"
  "if"
  "in"
  "module"
  "next"
  "not"
  "or"
  "redo"
  "rescue"
  "retry"
  "return"
  "then"
  "undef"
  "unless"
  "until"
  "when"
  "while"
  "yield"
] @keyword

; Functions
(method name: (identifier) @function)
(call method: (identifier) @function)

; Strings
(string) @string
(heredoc_body) @string
(symbol) @string

; Numbers
(integer) @number
(float) @number

; Booleans and constants
[(true) (false) (nil)] @constant
(constant) @constant

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
  "<<"
  ">>"
  "=>"
  ".."
  "..."
] @operator

; Instance variables
(instance_variable) @variable

; Parameters
(method_parameters (identifier) @parameter)
