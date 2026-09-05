; Identifiers and bindings

(positional_parameter (lowercase_identifier) @variable.parameter)
(labelled_parameter (label (lowercase_identifier)) @variable.parameter)
(optional_parameter (optional_label (lowercase_identifier)) @variable.parameter)
(optional_parameter_with_default (label (lowercase_identifier)) @variable.parameter)

(tuple_pattern (lowercase_identifier) @variable)
(constructor_pattern_argument (lowercase_identifier) @variable)
(case_clause (lowercase_identifier) @variable "=>")
(let_expression (lowercase_identifier) @variable)
(let_mut_expression (lowercase_identifier) @variable)
(value_definition (lowercase_identifier) @variable)
(qualified_identifier (lowercase_identifier) @variable)
(qualified_identifier (dot_lowercase_identifier) @variable)

((qualified_identifier (lowercase_identifier) @variable.special)
 (#eq? @variable.special "self"))

(package_identifier) @variable.special

; Types, constructors, fields, and constants

(type_identifier) @type
(qualified_type_identifier) @type

[
  (enum_definition (identifier) @type)
  (extenum_definition (identifier) @type)
  (struct_definition (identifier) @type)
  (tuple_struct_definition (identifier) @type)
  (type_definition (identifier) @type)
  (trait_definition (identifier) @type)
]

((qualified_type_identifier) @type.builtin
 (#any-of? @type.builtin
   "Unit" "Bool" "Byte" "Int16" "UInt16" "Int" "UInt" "Int64"
   "UInt64" "Float" "Double" "FixedArray" "Array" "Bytes" "String"
   "Error" "Self"))

(enum_constructor) @constructor
(constructor_expression (uppercase_identifier) @constructor)
(constructor_expression (dot_uppercase_identifier) @constructor)
(const_definition (uppercase_identifier) @constant)

(struct_field_declaration (lowercase_identifier) @property)
(access_expression (accessor (dot_identifier) @property))
(labeled_expression (lowercase_identifier) @property)
(labeled_expression_pun (lowercase_identifier) @property)

; Functions and methods

(function_definition
  (function_identifier (lowercase_identifier) @function))
(trait_method_declaration (function_identifier) @function)
(impl_definition (function_identifier) @function)
(apply_expression
  (qualified_identifier (lowercase_identifier) @function))
(apply_expression
  (qualified_identifier (dot_lowercase_identifier) @function))
(method_expression (lowercase_identifier) @function)
(dot_apply_expression (dot_identifier) @function)

; Keywords

[
  "struct" "enum" "extenum" "type" "trait" "typealias" "traitalias"
  "suberror"
] @keyword

[
  "pub" "priv" "readonly" "all" "open" "extern" "declare" "mut"
] @keyword

[
  "guard" "let" "letrec" "and" "const" "with" "as" "is" "using"
  "where" "longest" "nobreak" "defer"
] @keyword

[
  "package" "import"
] @keyword

[
  "fn" "test" "impl" "fnalias" "extend"
] @keyword

[
  "while" "loop" "for" "break" "continue" "in" "return"
] @keyword

[
  "if" "else" "match" "try" "raise" "catch" "noraise" "async"
] @keyword

; Operators and punctuation

[
  "+" "-" "*" "/" "%" "<<" ">>" "|" "&" "^" "=" "+=" "-="
  "*=" "/=" "%=" "<" ">" ">=" "<=" "==" "!=" "&&" "||" "|>"
  "<|" "<+" "<?" "=>" "->" "!" "!!" "?"
] @operator

[
  ";" "," ":" "::" "." ".."
] @punctuation.delimiter

[
  "(" ")" "{" "}" "[" "]"
] @punctuation.bracket

; Literals

(boolean_literal) @boolean
(integer_literal) @number
(float_literal) @number
(byte_literal) @string.special
(byte_escape_literal) @string.escape
(char_literal) @string
(string_literal) @string
(bytes_literal) @string
(multiline_string_literal) @string
(escape_sequence) @string.escape
(regex_literal) @string.regex
(string_interpolation) @embedded

; Comments

(comment) @comment
(block_comment) @comment

((comment) @comment.doc
 (#match? @comment.doc "^///"))
