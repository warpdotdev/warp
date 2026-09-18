(comment) @comment

; Procedures, functions, constructors and destructors. `declProc` covers all four,
; so each is matched through the keyword that introduces it. The name is captured
; whole rather than through `rhs:` so that an implementation written as
; `procedure TFoo.Bar;` shows up in the outline as `TFoo.Bar`.
(declProc
  (kProcedure)
  name: [
    (identifier)
    (genericDot)
    (genericTpl)
  ] @definition.procedure)

(declProc
  (kFunction)
  name: [
    (identifier)
    (genericDot)
    (genericTpl)
  ] @definition.function)

(declProc
  (kConstructor)
  name: [
    (identifier)
    (genericDot)
    (genericTpl)
  ] @definition.constructor)

(declProc
  (kDestructor)
  name: [
    (identifier)
    (genericDot)
    (genericTpl)
  ] @definition.destructor)

; Type declarations: classes, records, interfaces and plain aliases all land here.
(declType
  name: [
    (identifier)
    (genericTpl)
  ] @definition.type)
