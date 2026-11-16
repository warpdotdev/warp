[
  (array_creation_expression)
  (parenthesized_expression)
  (compound_statement)
  (declaration_list)
  (member_call_expression)
  (binary_expression)
  (return_statement)
  (arguments)
  (formal_parameters)
  (enum_declaration_list)
  (switch_block)
  (match_block)
  (case_statement)
  (default_statement)
  (property_hook_list)
] @indent.begin

(return_statement
  [
    (object_creation_expression)
    (anonymous_function)
    (arrow_function)
    (match_expression)
  ]) @indent.dedent

[
  ")"
  "}"
  "]"
] @indent.branch

(comment) @indent.auto

(arguments
  ")" @indent.end)

(formal_parameters
  ")" @indent.end)

(compound_statement
  "}" @indent.end)

(declaration_list
  "}" @indent.end)

(enum_declaration_list
  "}" @indent.end)

(return_statement
  ";" @indent.end)

(property_hook_list
  "}" @indent.end)

(ERROR
  "(" @indent.align
  .
  (_)
  (#set! indent.open_delimiter "(")
  (#set! indent.close_delimiter ")"))

(ERROR
  "[" @indent.align
  .
  (_)
  (#set! indent.open_delimiter "[")
  (#set! indent.close_delimiter "]"))
