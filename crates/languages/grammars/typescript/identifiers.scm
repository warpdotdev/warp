(comment) @comment

; Function and method definitions
(function_expression name: (identifier) @definition.function)
(function_declaration name: (identifier) @definition.function)
(method_definition name: (property_identifier) @definition)

; Class declarations
(class name: (_) @definition.class)
(class_declaration name: (_) @definition.class)
