(line_comment) @comment
(line_outer_doc_comment) @comment
(block_outer_doc_comment) @comment

; Function definitions
(function_item (identifier) @definition.fn)
(function_signature_item (identifier) @definition.fn)

; Struct declarations
(struct_item name: (type_identifier) @definition.struct)

; Enum declarations
(enum_item name: (type_identifier) @definition.enum)

; Trait declarations
(trait_item name: (type_identifier) @definition.trait)

; Type alias declarations
(type_item name: (type_identifier) @definition.type)
