; Pascal / Object Pascal / Delphi.
;
; Adapted from the query that ships with Isopod/tree-sitter-pascal (MIT). Warp's
; highlighter applies every capture it gets and does not evaluate query
; predicates, so the upstream `#match?`-gated rules — the exit/break/continue
; heuristic and the "infer a constant from its spelling" rules — are left out
; here. Keeping them would repaint arbitrary identifiers as keywords.

; -- Keywords

[
  (kProgram)
  (kLibrary)
  (kUnit)
  (kUses)

  (kBegin)
  (kEnd)
  (kAsm)

  (kVar)
  (kThreadvar)
  (kConst)
  (kResourcestring)
  (kConstref)
  (kOut)
  (kType)
  (kLabel)
  (kExports)

  (kAbsolute)

  (kProperty)
  (kRead)
  (kWrite)
  (kImplements)
  (kDefault)
  (kNodefault)
  (kStored)
  (kIndex)
  (kDispId)

  (kClass)
  (kInterface)
  (kDispInterface)
  (kObject)
  (kRecord)
  (kObjcclass)
  (kObjccategory)
  (kObjcprotocol)
  (kArray)
  (kFile)
  (kString)
  (kSet)
  (kOf)
  (kHelper)
  (kPacked)

  (kGeneric)
  (kSpecialize)

  (kFunction)
  (kProcedure)
  (kConstructor)
  (kDestructor)
  (kOperator)
  (kReference)

  (kImplementation)
  (kInitialization)
  (kFinalization)

  (kPublished)
  (kPublic)
  (kProtected)
  (kPrivate)
  (kStrict)
  (kRequired)
  (kOptional)

  (kForward)

  (kStatic)
  (kVirtual)
  (kAbstract)
  (kSealed)
  (kDynamic)
  (kOverride)
  (kOverload)
  (kReintroduce)
  (kInherited)
  (kInline)

  (kStdcall)
  (kCdecl)
  (kCppdecl)
  (kPascal)
  (kRegister)
  (kMwpascal)
  (kExternal)
  (kName)
  (kMessage)
  (kDeprecated)
  (kExperimental)
  (kPlatform)
  (kUnimplemented)
  (kCvar)
  (kExport)
  (kFar)
  (kNear)
  (kSafecall)
  (kAssembler)
  (kNostackframe)
  (kInterrupt)
  (kNoreturn)
  (kIocheck)
  (kLocal)
  (kHardfloat)
  (kSoftfloat)
  (kMs_abi_default)
  (kMs_abi_cdecl)
  (kSaveregisters)
  (kSysv_abi_default)
  (kSysv_abi_cdecl)
  (kVectorcall)
  (kVarargs)
  (kWinapi)
  (kAlias)
  (kDelayed)

  (kFor)
  (kTo)
  (kDownto)
  (kIf)
  (kThen)
  (kElse)
  (kDo)
  (kWhile)
  (kRepeat)
  (kUntil)
  (kTry)
  (kExcept)
  (kFinally)
  (kRaise)
  (kOn)
  (kCase)
  (kWith)
  (kGoto)
] @keyword

; -- Punctuation & operators

[
  "("
  ")"
  "["
  "]"
] @punctuation.bracket

[
  ";"
  ","
  ":"
  ".."
  (kEndDot)
] @punctuation.delimiter

[
  (kDot)
  (kAdd)
  (kSub)
  (kMul)
  (kFdiv)
  (kAssign)
  (kAssignAdd)
  (kAssignSub)
  (kAssignMul)
  (kAssignDiv)
  (kEq)
  (kLt)
  (kLte)
  (kGt)
  (kGte)
  (kNeq)
  (kAt)
  (kHat)
] @operator

; Technically operators, but Pascal spells them as words, so they read better as
; reserved words.
[
  (kOr)
  (kXor)
  (kDiv)
  (kMod)
  (kAnd)
  (kShl)
  (kShr)
  (kNot)
  (kIs)
  (kAs)
  (kIn)
] @keyword

; -- Builtin constants

[
  (kTrue)
  (kFalse)
] @constant

; Arguably a constant, but we highlight it as a keyword.
[
  (kNil)
] @keyword

; -- Literals

(literalNumber) @number
(literalString) @string
(literalChar) @string

; -- Comments and compiler directives

(comment) @comment
(pp) @keyword

; -- Type declarations

(declType
  name: (identifier) @type)

(declType
  name: (genericTpl
    entity: (identifier) @type))

; -- Procedure & function declarations

(declProc
  name: (identifier) @function)

(declProc
  name: (genericTpl
    entity: (identifier) @function))

(declProc
  name: (genericDot
    rhs: (identifier) @function))

(declProc
  name: (genericDot
    rhs: (genericTpl
      entity: (identifier) @function)))

; Properties read like functions at the call site, so declare them the same way.
(declProp
  name: (identifier) @function)

; -- Parameters

(declArg
  name: (identifier) @variable.parameter)

; -- Generic parameters

(genericArg
  name: (identifier) @type.parameter)

(genericArg
  type: (typeref) @type)

; Only the qualifier of a `TFoo.Bar` name is a type. Capturing the whole
; `genericDot` here would also repaint `Bar`, and since Warp keeps the last
; capture for a range, that would beat the `@function` the `declProc` rules give
; a method implementation.
(genericDot
  lhs: (identifier) @type)

(genericDot
  lhs: (genericTpl
    entity: (identifier) @type))

; -- Exception parameters

(exceptionHandler
  variable: (identifier) @variable.parameter)

; -- Type usage

(typeref) @type

; -- Labels

[
  (caseLabel)
  (label)
] @constant

; -- Calls
;
; Pascal does not require parentheses for a procedure call, so the parenthesised
; forms below catch only some of them; the bare-identifier statement rules after
; them cover the rest.

(exprCall
  entity: (identifier) @function)

(exprCall
  entity: (exprTpl
    entity: (identifier) @function))

(exprCall
  entity: (exprDot
    rhs: (identifier) @function))

(exprCall
  entity: (exprDot
    rhs: (exprTpl
      entity: (identifier) @function)))

; A statement that consists of nothing but an identifier is a procedure call.
(statement
  (identifier) @function)

(statement
  (exprDot
    rhs: (identifier) @function))

(statement
  (exprTpl
    entity: (identifier) @function))

(statement
  (exprDot
    rhs: (exprTpl
      entity: (identifier) @function)))

; -- Variable & constant declarations

(declVar
  name: (identifier) @variable)

(declField
  name: (identifier) @variable)

(declConst
  name: (identifier) @constant)

(declEnumValue
  name: (identifier) @constant)
