(comment) @comment

; Function declarations
(function_declarator declarator: (identifier) @definition)

; Class declarations
(struct_specifier name: (type_identifier) @definition.class body:(_))
(declaration type: (union_specifier name: (type_identifier) @definition.class))

; Type declarations
(type_definition declarator: (type_identifier) @definition.type)
(enum_specifier name: (type_identifier) @definition.type)
