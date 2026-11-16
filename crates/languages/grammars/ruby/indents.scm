[
  (class)
  (singleton_class)
  (method)
  (singleton_method)
  (module)
  (call)
  (if)
  (block)
  (do_block)
  (hash)
  (array)
  (argument_list)
  (case)
  (while)
  (until)
  (for)
  (begin)
  (unless)
  (assignment)
  (parenthesized_statements)
] @indent.begin

[
  "end"
  ")"
  "}"
  "]"
] @indent.end

[
  "end"
  ")"
  "}"
  "]"
  (when)
  (elsif)
  (else)
  (rescue)
  (ensure)
] @indent.branch

(comment) @indent.ignore
