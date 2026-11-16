[
  (compound_statement)
  (declaration_list)
  (field_declaration_list)
  (enumerator_list)
  (parameter_list)
  (init_declarator)
  (expression_statement)
] @indent

[
  "case"
  "}"
  "]"
  ")"
  (access_specifier)
] @outdent

(if_statement
  consequence: (_) @indent
  (#not-kind-eq? @indent "compound_statement")
  (#set! "scope" "all"))
(while_statement
  body: (_) @indent
  (#not-kind-eq? @indent "compound_statement")
  (#set! "scope" "all"))
(do_statement
  body: (_) @indent
  (#not-kind-eq? @indent "compound_statement")
  (#set! "scope" "all"))
(for_statement
  ")"
  (_) @indent
  (#not-kind-eq? @indent "compound_statement")
  (#set! "scope" "all"))
