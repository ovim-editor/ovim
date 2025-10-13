; Keys
(block_mapping_pair key: (flow_node) @property)
(block_mapping_pair key: (_) @property)

; Strings
(string_scalar) @string
(double_quote_scalar) @string
(single_quote_scalar) @string

; Numbers
(integer_scalar) @number
(float_scalar) @number

; Booleans and constants
(boolean_scalar) @constant
(null_scalar) @constant

; Comments
(comment) @comment

; Tags
(tag) @type
