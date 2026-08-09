(comment) @comment

; Contract, interface, and library declarations
(contract_declaration (identifier) @definition.class)
(interface_declaration (identifier) @definition.class)
(library_declaration (identifier) @definition.class)

; Function and modifier declarations
(function_definition (identifier) @definition.fn)
(modifier_definition (identifier) @definition.method)

; Event and error declarations
(event_definition (identifier) @definition.method)
(error_declaration (identifier) @definition.method)

; Struct and enum declarations
(struct_declaration (identifier) @definition.struct)
(enum_declaration (identifier) @definition.enum)

; State variables
(state_variable_declaration (identifier) @definition)
