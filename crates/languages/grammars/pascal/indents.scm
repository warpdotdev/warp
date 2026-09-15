; Warp's indent engine (crates/syntax_tree/src/queries/indent_query.rs) only
; understands `@indent` and `@outdent`: it walks from the node under the cursor up
; through its parents, adding 1 per `@indent` ancestor and subtracting 1 per
; `@outdent`. A node never indents its own opening line.
;
; So the node to capture is the one that *opens* on the earlier line — `(try)`
; rather than its `(statements)` body, `(for)` rather than the statement it runs —
; because only those still span the start of the line being indented.

[
  ; `begin` … `end`, including the outermost one of a program or unit.
  (block)
  (blockTr)
  ; Statements that carry a body. Pascal lets the body be a single statement with
  ; no `begin`, and it is indented either way.
  (try)
  (repeat)
  (for)
  (foreach)
  (while)
  (with)
  (if)
  (ifElse)
  (caseCase)
  (exceptionHandler)
  ; `case` … `end`, and `class`/`record`/`object`/`interface` … `end`.
  (case)
  (declClass)
  (declIntf)
  (declHelper)
  (declEnum)
  (declVariant)
  ; Declaration groups: entries line up one level under `uses`/`var`/`const`/`type`.
  (declUses)
  (declVars)
  (declConsts)
  (declTypes)
  (declLabels)
  (declExports)
  ; Wrapped parameter and argument lists, and structured literals.
  (declArgs)
  (exprArgs)
  (arrInitializer)
  (recInitializer)
  ; `asm` … `end`.
  (asm)
] @indent

; `end`, `end.` and a closing bracket sit at the level of the construct they close,
; so they cancel the indent their parent contributed.
[
  (kEnd)
  (kEndDot)
  ")"
  "]"
] @outdent

; A `begin` … `end` body already supplies its own level, so the statement that owns
; it must not add a second one on top.
[
  (while
    body: (block))
  (for
    body: (block))
  (foreach
    body: (block))
  (with
    body: (block))
  (if
    then: (block))
  (ifElse
    then: (block))
  (caseCase
    body: (block))
  (exceptionHandler
    body: (block))
] @outdent

; `else` belongs to the `if`/`case` that opened the construct, not to the branch
; above it.
(ifElse
  (kElse) @outdent)

(case
  (kElse) @outdent)

; Visibility sections label the class body rather than belonging to it, so they sit
; at the level of the `class` keyword itself.
(declSection
  [
    (kStrict)
    (kPrivate)
    (kProtected)
    (kPublic)
    (kPublished)
    (kRequired)
    (kOptional)
  ] @outdent)
