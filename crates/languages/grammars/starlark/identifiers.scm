; Starlark symbol identification
(comment) @comment

; Function definitions
(function_definition name: (identifier) @definition.def)

; Variables assigned at module level (like rules, constants)
(assignment
  left: (identifier) @definition.var)
