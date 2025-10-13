; Keywords
[
  "if"
  "then"
  "else"
  "elif"
  "fi"
  "case"
  "esac"
  "for"
  "while"
  "until"
  "do"
  "done"
  "in"
  "function"
  "select"
] @keyword

; Commands
(command_name) @function

; Strings
(string) @string
(raw_string) @string

; Numbers
(number) @number

; Variables
(variable_name) @variable
(simple_expansion) @variable

; Comments
(comment) @comment

; Operators
[
  "="
  "=="
  "!="
  "<"
  ">"
  "&&"
  "||"
  "|"
  ";"
  "&"
] @operator

; Special variables
[
  "$?"
  "$#"
  "$@"
  "$*"
  "$$"
  "$!"
] @constant
