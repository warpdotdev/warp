; Adapted from Helix's runtime/queries/nix/indents.scm, reduced to the plain
; @indent / @outdent captures Warp's indent engine consumes.

; One level per bracketed scope; the matching close token dedents.
[
  (attrset_expression)
  (rec_attrset_expression)
  (let_attrset_expression)
  (list_expression)
  (parenthesized_expression)
  (formals)
] @indent

[
  "}"
  ")"
  "]"
] @outdent

; A binding value carried onto the line(s) after `=`. It shares the `=` line
; with a same-line bracket, so `x = { … }` is not indented twice.
(binding) @indent

; `let … in`: keep the bindings indented; pull the body back to the `let` level.
(let_expression) @indent
(let_expression body: (_) @outdent)

; `if … then … else`: indent each branch.
(if_expression
  consequence: (_) @indent)
(if_expression
  alternative: (_) @indent)

; Function application: arguments carried onto following lines. Nested
; applications share a line, so they collapse to a single level.
(apply_expression) @indent
