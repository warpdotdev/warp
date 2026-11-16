(comment) @comment

; Function definitions
(function_declaration name: (identifier) @definition.func)
(method_declaration name: (field_identifier) @function.func)

; Type declarations
(type_spec name: (type_identifier) @definition.type)
