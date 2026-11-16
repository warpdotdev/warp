(comment) @comment

; Function declarations and definitions
(function_declarator declarator: (identifier) @definition)
(function_declarator declarator: (qualified_identifier name: (identifier) @definition))
(function_declarator declarator: (field_identifier) @definition)
(template_method name: (field_identifier) @definition)

; Class declarations
(struct_specifier name: (type_identifier) @definition.class body:(_))
(declaration type: (union_specifier name: (type_identifier) @definition.class))
(class_specifier name: (type_identifier) @definition.class)

; Type declarations
(type_definition declarator: (type_identifier) @definition.type)
(enum_specifier name: (type_identifier) @definition.type)
