(comment) @comment

; Function and method definitions
(function_expression name: (identifier) @definition.function)
(function_declaration name: (identifier) @definition.function)
(method_definition name: (property_identifier) @definition)

; Class declarations
(class name: (_) @definition.class)
(class_declaration name: (_) @definition.class)

; TypeScript specific definitions
(interface_declaration name: (type_identifier) @definition.interface)
(type_alias_declaration name: (type_identifier) @definition.type)
(enum_declaration name: (identifier) @definition.enum)

; JSX component definitions
(jsx_element 
  open_tag: (jsx_opening_element 
    name: (identifier) @definition.component))
(jsx_self_closing_element 
  name: (identifier) @definition.component)

