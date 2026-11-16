(comment) @comment
(block_comment) @comment

; Class declarations
(class_definition name: (identifier) @definition.class)

; Object declarations
(object_definition name: (identifier) @definition.class)

; Trait declarations
(trait_definition name: (identifier) @definition.class)

; Enum declarations
(enum_definition name: (identifier) @definition.class)

; Function/method declarations
(function_declaration name: (identifier) @definition)
(function_definition name: (identifier) @definition)

; Variable definitions
(val_definition pattern: (identifier) @definition)
(var_definition pattern: (identifier) @definition)
(val_declaration name: (identifier) @definition)
(var_declaration name: (identifier) @definition)

; Parameters
(parameter name: (identifier) @definition)
(binding name: (identifier) @definition)
(class_parameter name: (identifier) @definition)

; Type definitions
(type_definition name: (type_identifier) @definition.type)
