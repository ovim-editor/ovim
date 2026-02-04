; Tree-sitter query files (commonly stored as `.scm`).
;
; We do not yet ship a dedicated tree-sitter grammar for the query language.
; Until then, `Language::TreeSitterQuery` uses the Bash grammar as a best-effort
; fallback so the highlighter can still be constructed.
;
; This query is intentionally conservative and only references Bash node/token
; names so it compiles with the fallback grammar.

; Strings / numbers / variables (Bash fallback)
(string) @string
(raw_string) @string
(number) @number
(variable_name) @variable
(simple_expansion) @variable

; Bash comments (won't match `;` query comments, but keeps fallback valid)
(comment) @comment

; Highlight semicolons so leading `;` in query comments stands out a bit.
";" @comment

