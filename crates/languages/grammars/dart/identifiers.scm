(comment) @comment
(documentation_comment) @comment

; Type definitions
(class_definition name: (identifier) @definition.class)
(enum_declaration name: (identifier) @definition.enum)
(mixin_declaration (identifier) @definition.class)
(extension_declaration name: (identifier) @definition.class)
(extension_type_declaration name: (identifier) @definition.class)

; Function and constructor definitions
(function_signature name: (identifier) @definition.fn)
(getter_signature name: (identifier) @definition.fn)
(setter_signature name: (identifier) @definition.fn)
(constructor_signature name: (identifier) @definition.constructor)
