import Lean
import Prod.Attribute

/-!
# LCNF → sexp IR lowering

Lowers pure-phase `Lean.Compiler.LCNF.Decl` values to the s-expression grammar
parsed by the Rust `prod-ir` crate (`prod-ir/src/parser.rs`). One sexp `def`
per LCNF declaration; `let`/`cases`/`jp`/`jmp`/`return`/`unreach` map to the
corresponding IR nodes. Design decisions:

- **Names**: definitions are stripped to their last component
  (`UorAtlas.stride` → `stride`); the full name is recorded by the caller in a
  `;; full:` comment (the parser skips `;;` comments). LCNF fvarIds are
  resolved to sanitized binder names (`_x.1` → `_x_1`, collisions get a
  `_<counter>` suffix) via a per-definition `FVarId → String` map.
- **Erased/type arguments** (`Arg.erased`, `Arg.type`): dropped from calls,
  counted in `LowerState.dropped` so the caller can emit an arity note.
  `let` bindings with `LetValue.erased` values (proofs) register their binder
  name but emit no binding and no opaque marker — proofs are erased by design.
- **Operator whitelist**: `Nat.add/sub/mul/div/mod/shiftLeft/shiftRight/pow`
  map to the IR binary ops (`pow` was added to prod-ir in M3 for `belt`;
  `shiftRight` maps to `shr`, a total/infallible IR node distinct from `shl`
  — `a >>> b = 0` for `b ≥ 64` since `Nat` is unbounded, so there is no
  overflow case to report). `Nat.shiftRight` reaches lowering both from `>>>`
  written directly and from LCNF's `n / 2` peephole (division by a
  power-of-two literal); either way it is just another whitelisted name by
  the time lowering sees it. Any other constant becomes an *unresolved
  call*: if the callee is `@[prod]`-tagged it is `(call <last-component>
  ...)`, otherwise it is recorded for the coverage report and emitted as
  `(extern "Full.Name" ...)` — a node codegen refuses, rather than a `call`
  to a function nobody generated.
- **Constructors** (detected via the environment, e.g. `Prod.mk`) become
  `(ctor "Full.Name" ...)`; structure projections resolve their LCNF index to
  the declared field name here, where the environment is available, and
  become `(proj "Full.TypeName" "fieldName" x)`.
- **Decidable-if rewrite**: `if a < b then T else F` (and the `≤`/`=`
  analogues) compiles to `let c := Nat.decLt/Nat.decLe/Nat.decEq/
  instDecidableEqNat a b` followed by `cases c` over `Decidable.isFalse`/
  `isTrue` (with erased proof-hypothesis binders). Lowered directly to the IR
  `(if (lt|le|eq a b) T F)`; without this rewrite the scrutinee would surface
  as an extern `decLt` call and `Decidable.*` ctor patterns, neither of which
  has a Rust rendering. Only the immediately-bound shape is recognized;
  anything else still lowers as an extern call.
- **Decidable-as-Bool rewrite**: `a < b : Bool` (no surrounding `if`) compiles
  to the same decider call followed by `Decidable.decide c` rather than
  `cases`. Lowered directly to the IR comparison expression `(lt|le|eq a b)`,
  which is valid outside an `if` too. Same immediately-bound-shape caveat as
  above; anything else still lowers as an extern call to `decide`.
- **Closures** (`Code.fun`) lower to `(opaque "<name>-closure")` plus a
  coverage note — closures are phase-2 work. Impure-phase-only constructors
  never occur at the pure phase; wildcard arms keep the matches total.
-/

open Lean Compiler LCNF

namespace Prod

/-- Static lowering configuration. -/
structure LowerCtx where
  /-- `@[prod]`-tagged names: calls to these are internal, not extern. -/
  tagged : Array Name

/-- Per-definition lowering state: name resolution plus coverage facts. -/
structure LowerState where
  names   : Std.HashMap Name String := {}  -- keyed on `FVarId.name`
  used    : Std.HashSet String := {}
  /-- Compiler-generated Nat dictionaries which are semantically operators. -/
  knownOps : Std.HashMap String String := {}
  counter : Nat := 0
  opaques : Array String := #[]            -- opaque markers emitted
  externs : Array String := #[]            -- non-tagged, non-whitelisted calls
  dropped : Nat := 0                       -- erased/type args dropped

abbrev LowerM := ReaderT LowerCtx (StateRefT LowerState CoreM)

/-- Last dot-separated component of a name as a string. -/
def lastComponent : Name → String
  | .str _ s => s
  | .num _ i => toString i
  | .anonymous => "v"

private def isIdentChar (c : Char) : Bool :=
  let n := c.toNat
  (48 ≤ n && n ≤ 57) || (65 ≤ n && n ≤ 90) || (97 ≤ n && n ≤ 122) || n == 95

/-- Sanitize a Lean name into a Rust-ish identifier: keep [A-Za-z0-9_], map
    everything else to `_`, prefix `v` if empty or starting with a digit.
    `_x.1` → `_x_1`. -/
def sanitize (n : Name) : String :=
  let t := String.ofList (n.toString.toList.map fun c => if isIdentChar c then c else '_')
  if t.isEmpty || t.front.isDigit then "v" ++ t else t

/-- Resolve an fvarId to its emitted name, registering (sanitized, deduped)
    on first use. -/
def registerFVar (fvarId : FVarId) (binderName : Name) : LowerM String := do
  let st ← get
  if let some s := st.names[fvarId.name]? then return s
  let base := sanitize binderName
  let mut s := base
  let mut st := st
  while st.used.contains s do
    st := { st with counter := st.counter + 1 }
    s := s!"{base}_{st.counter}"
  set { st with used := st.used.insert s, names := st.names.insert fvarId.name s }
  return s

/-- Look up an already-registered fvarId (falls back to registering its raw
    name, which is still a valid identifier). -/
def lookupFVar (fvarId : FVarId) : LowerM String :=
  registerFVar fvarId fvarId.name

/-- Names used by Lean's Nat typeclass dictionaries in pure LCNF output. -/
def natDictOp : Name → Option String
  | `instAddNat => some "add"
  | `instSubNat => some "sub"
  | `instMulNat => some "mul"
  | `instDiv => some "div"
  | `instMod => some "mod"
  | `instNatPowNat => some "pow"
  | `Nat.add => some "add"
  | `Nat.sub => some "sub"
  | `Nat.mul => some "mul"
  | `Nat.div => some "div"
  | `Nat.mod => some "mod"
  | `Nat.pow => some "pow"
  | _ => none

/-- Lift a Nat dictionary through its overloaded-operation wrapper. -/
def natHDictOp : Name → Option String
  | `instHAdd => some "add"
  | `instHSub => some "sub"
  | `instHMul => some "mul"
  | `instHDiv => some "div"
  | `instHMod => some "mod"
  | `instHPow => some "pow"
  | `instPowNat => some "pow"
  | _ => none

/-- The operation represented by an already-lowered local value. -/
def knownOpOf (v : LetValue .pure) : LowerM (Option String) := do
  let st ← get
  match v with
  | .const n _ args =>
    match natDictOp n with
    | some op => if args.isEmpty then return some op else return none
    | none =>
      match natHDictOp n, args.toList with
      | some op, [.fvar f] =>
        let nm ← lookupFVar f
        match st.knownOps[nm]? with
        | some existing => return some existing
        | none => return some op
      | _, _ => return none
  | .proj _ _ f =>
    let nm ← lookupFVar f
    return st.knownOps[nm]?
  | .fvar f args =>
    if !args.isEmpty then return none
    let nm ← lookupFVar f
    return st.knownOps[nm]?
  | _ => return none

/-- Emit an `(opaque "...")` expression node and record it for coverage. -/
def opaqueNode (what : String) : LowerM String := do
  modify fun st => { st with opaques := st.opaques.push what }
  return s!"(opaque \"{what}\")"

/-- Emit an `(opaque "...")` type node and record it for coverage. -/
def opaqueType (n : Name) : LowerM String := do
  modify fun st => { st with opaques := st.opaques.push s!"type:{n}" }
  return s!"(opaque \"{n}\")"

/-- Lower call arguments, dropping `erased`/`type` args (counted). -/
def lowerArgs (args : Array (Arg .pure)) : LowerM (Array String) := do
  let mut out := #[]
  for a in args do
    match a with
    | .fvar id => out := out.push (← lookupFVar id)
    | _ => modify fun st => { st with dropped := st.dropped + 1 }
  return out

private def spaced (xs : Array String) : String :=
  if xs.isEmpty then "" else " " ++ String.intercalate " " xs.toList

/-- `.const` operator whitelist as an (LCNF constant name, IR binary op)
    association list. Single source of truth for both `opWhitelist` (what the
    lowerer accepts) and `subsetJson` (the published contract, `Prod.Emit`) —
    extracted so the two cannot drift apart. `pow` was added to prod-ir in M3
    for `belt`; `shiftRight` maps to `shr`, a total/infallible IR node
    distinct from `shl` — `a >>> b = 0` for `b ≥ 64` since `Nat` is unbounded,
    so there is no overflow case to report. -/
def natOpNames : List (Name × String) :=
  [ (`Nat.add, "add"), (`Nat.sub, "sub"), (`Nat.mul, "mul"), (`Nat.div, "div"),
    (`Nat.mod, "mod"), (`Nat.shiftLeft, "shl"), (`Nat.shiftRight, "shr"),
    (`Nat.pow, "pow"), (`Nat.beq, "eq"), (`Nat.ble, "le"), (`Nat.blt, "lt") ]

/-- `.const` operator whitelist: Lean kernel Nat ops → IR binary ops. -/
def opWhitelist (n : Name) : Option String :=
  (natOpNames.find? (fun p => p.1 == n)).map (·.2)

/-- LexLean's semantic backend emits these generic runtime calls from closed
    semantic primitive nodes.  They are compiler boundary operations, not
    application helpers: named-closure discovery leaves their implementation
    bodies out of LCNF and this table lowers the call itself to typed IR. -/
def isLexLeanRuntimeName (n : Name) : Bool :=
  (n.toString.splitOn ".").contains "LexLeanRuntime"

def lexLeanPrimitive? (n : Name) : Option (String × Nat) :=
  if !isLexLeanRuntimeName n then none else
  match lastComponent n with
  | "subtract" => some ("sub", 2)
  | "multiply" => some ("mul", 2)
  | "quotient" => some ("quotient", 3)
  | "remainder" => some ("remainder", 3)
  | "negate" => some ("negate", 1)
  | "checkedAdd" => some ("checked-add", 2)
  | "checkedAddInt64" => some ("checked-add", 2)
  | "checkedSubtract" => some ("checked-sub", 2)
  | "checkedSubtractInt64" => some ("checked-sub", 2)
  | "checkedMultiply" => some ("checked-mul", 2)
  | "checkedMultiplyInt64" => some ("checked-mul", 2)
  | "checkedNegate" => some ("checked-neg", 1)
  | "checkedNegateInt64" => some ("checked-neg", 1)
  | "checkedQuotient" => some ("checked-div", 2)
  | "checkedQuotientInt64" => some ("checked-div", 2)
  | "checkedConvert" => some ("checked-convert", 1)
  | "bitAnd" => some ("bit-and", 2)
  | "bitOr" => some ("bit-or", 2)
  | "bitXor" => some ("bit-xor", 2)
  | "bitNot" => some ("bit-not", 1)
  | "shiftLeft" => some ("checked-shl", 2)
  | "shiftRight" => some ("checked-shr", 2)
  | "equal" => some ("eq", 2)
  | "append" => some ("append", 2)
  | "length" => some ("length", 1)
  | "index" => some ("index", 2)
  | "slice" => some ("slice", 3)
  | "utf8Encode" => some ("utf8-encode", 1)
  | "utf8Decode" => some ("utf8-decode", 1)
  | "compareBytes" => some ("compare-bytes", 2)
  | "splitExact" => some ("split-exact", 3)
  | "join" => some ("join", 2)
  | "parseDecimal" => some ("parse-decimal", 1)
  | "formatDecimal" => some ("format-decimal", 1)
  | _ => none

/-- Lean's closed scalar/container heads that have direct prod-IR type nodes
    and therefore must never be re-exported as application inductives. -/
def isPortableBuiltinTypeName (n : Name) : Bool :=
  [``Nat, ``Bool, ``Int, ``Int8, ``Int16, ``Int32, ``Int64,
   ``UInt8, ``UInt16, ``UInt32, ``UInt64, ``String, ``ByteArray,
   ``Ordering, ``Prod, ``List, ``Option, ``Except].contains n

def isErasedPortableDictionary (n : Name) : Bool :=
  let part := lastComponent n
  part.startsWith "instBEq" || part.startsWith "instDecidableEq" ||
    part.startsWith "instToString" || n == ``Int.instSub || n == ``Int.instMul ||
    toString n == "Int.instNeg"

private def isFixedLiteralConstructor (n : Name) : Bool :=
  let owner := n.getPrefix
  (lastComponent n == "ofNat" || lastComponent n == "ofInt") &&
    [``Int8, ``Int16, ``Int32, ``Int64, ``UInt8, ``UInt16, ``UInt32, ``UInt64].contains owner

private def isCtorName (env : Environment) (n : Name) : Bool :=
  match env.find? n with
  | some (.ctorInfo _) => true
  | _ => false

def lowerLetValue (v : LetValue .pure) : LowerM String := do
  match v with
  | .lit (.nat n) => return toString n
  | .lit (.uint8 n) => return toString n
  | .lit (.uint16 n) => return toString n
  | .lit (.uint32 n) => return toString n
  | .lit (.uint64 n) => return toString n
  | .lit (.usize n) => return toString n
  | .lit (.str value) => return s!"(string \"{Lean.Json.escape value}\")"
  | .erased => opaqueNode "erased"
  | .proj typeName idx struct => do
    let s ← lookupFVar struct
    let env ← getEnv
    -- Resolve the index to a field name here, where the environment is
    -- available. Emitting the index instead would force codegen to keep a
    -- parallel table, and a disagreement between the two swaps fields
    -- silently. LCNF projection indices are into the *declared* field list
    -- (including `Prop` fields), which is exactly what `getStructureFields`
    -- returns, so no filtering is applied before indexing. No name-keyed
    -- special cases here, not even for `UorAtlas.Instance`: the declared
    -- spelling passes through unmodified for every structure, so the
    -- `(type ...)` declaration and the `(proj ...)` reference can never
    -- disagree — both are generated from the same `getStructureFields` call.
    let fields := getStructureFields env typeName
    let field := match fields[idx]? with
      | some n => sanitize n
      | none => s!"field_{idx}"
    return s!"(proj \"{typeName}\" \"{field}\" {s})"
  | .const declName _ args => do
    let env ← getEnv
    let args' ← lowerArgs args
    if let some (op, arity) := lexLeanPrimitive? declName then
      if args'.size >= arity then
        let values := args'.extract (args'.size - arity) args'.size
        modify fun st => { st with dropped := st.dropped + (args'.size - arity) }
        return s!"({op}{spaced values})"
      modify fun st => { st with externs := st.externs.push s!"{declName} (wrong semantic primitive arity)" }
      return s!"(extern \"{declName}\"{spaced args'})"
    if isLexLeanRuntimeName declName then
      -- Typeclass dictionaries and helper records are implementation inputs
      -- to a following primitive call. Their semantic effect is captured by
      -- the primitive opcode and fixed operand/result types, so the target IR
      -- deliberately erases the dictionary value.
      modify fun st => { st with dropped := st.dropped + 1 }
      return "0"
    if isErasedPortableDictionary declName then
      modify fun st => { st with dropped := st.dropped + 1 }
      return "0"
    if lastComponent declName == "neg" &&
        [``Int8, ``Int16, ``Int32, ``Int64].contains declName.getPrefix && args'.size == 1 then
      return s!"(negate {args'[0]!})"
    if isFixedLiteralConstructor declName && args'.size == 1 then
      return args'[0]!
    if isCtorName env declName then
      return s!"(ctor \"{declName}\"{spaced args'})"
    match opWhitelist declName with
    | some op =>
      if args'.size == 2 then
        return s!"({op} {args'[0]!} {args'[1]!})"
      -- partial/unusual application of a whitelisted op: not a 2-arg operator
      -- use, and not necessarily `@[prod]`-tagged either, so it is the same
      -- kind of unresolved callee as the `none` case below.
      modify fun st => { st with externs := st.externs.push s!"{declName} (unusual application)" }
      return s!"(extern \"{declName}\"{spaced args'})"
    | none =>
      if (← read).tagged.contains declName then
        -- internal call to another @[prod]-tagged definition
        return s!"(call {lastComponent declName}{spaced args'})"
      modify fun st => { st with externs := st.externs.push (toString declName) }
      -- Emit a distinct node rather than a `call`: codegen must refuse this,
      -- not render a Rust call to a function nobody generated.
      return s!"(extern \"{declName}\"{spaced args'})"
  | .fvar f args => do
    let nm ← lookupFVar f
    let args' ← lowerArgs args
    match (← get).knownOps[nm]? with
    | some op =>
      if args'.size == 2 then return s!"({op} {args'[0]!} {args'[1]!})"
      return s!"(call {nm}{spaced args'})"
    | none => return s!"(call {nm}{spaced args'})"
  | _ => opaqueNode "letvalue"  -- impure-phase-only constructors

/-- The decider constants recognized by `decidableIf?`/`decideOf?`, paired
    with their IR comparison operator. Single source of truth for both
    `deciderOp` and `subsetJson` (`Prod.Emit`), so the published contract
    cannot list a decider the lowerer does not actually accept, or omit one
    it does. `instDecidableEqNat` appears in LCNF when the instance wrapper
    is not unfolded (unlike the arithmetic dictionaries). -/
def deciderNames : List (Name × String) :=
  [ (``Nat.decLt, "lt"), (``Nat.decLe, "le"), (``Nat.decEq, "eq"),
    (``instDecidableEqNat, "eq") ]

/-- The decider constants recognized by `decidableIf?`, mapped to their IR
    comparison operator. -/
def deciderOp (n : Name) : Option String :=
  (deciderNames.find? (fun p => p.1 == n)).map (·.2)

/-- Recognize the LCNF shape of `if a < b then T else F` (and the `≤`/`=`
    analogues): `let c := <decider> a b` immediately followed by `cases c`
    with exactly the `Decidable.isFalse`/`isTrue` alternatives (either
    order). Returns the IR comparison operator, the compared fvars, and the
    (else, then) branch codes. The alternatives' proof-hypothesis binders are
    dropped by the caller — they are proof-irrelevant and never occur in
    computational code. -/
def decidableIf? (decl : LetDecl .pure) (k : Code .pure)
    : Option (String × FVarId × FVarId × Code .pure × Code .pure) := do
  let .const decider _ #[.fvar a, .fvar b] := decl.value | failure
  let op ← deciderOp decider
  let .cases c := k | failure
  guard (c.discr == decl.fvarId)
  if c.alts.size != 2 then failure
  let mut else? : Option (Code .pure) := none
  let mut then? : Option (Code .pure) := none
  for alt in c.alts do
    match alt with
    | .alt ``Decidable.isFalse _ code => else? := some code
    | .alt ``Decidable.isTrue _ code => then? := some code
    | _ => failure
  return (op, a, b, ← else?, ← then?)

/-- Recognize the LCNF shape of a decidable comparison used as a plain `Bool`
    value (as opposed to the `if`-consuming shape `decidableIf?` handles):
    `let c := <decider> a b` immediately followed by `let x := Decidable.decide
    c`. Binds `x` directly to the IR comparison expression, skipping the
    intermediate decider binding — `Eq`/`Lt`/`Le`/`Gt` are already valid IR
    expressions outside an `if`, not just inside one. Returns the operator,
    the compared fvars, the `decide` binding, and its continuation. Only the
    immediately-bound shape is recognized; anything else still lowers as an
    extern call to `decide`. -/
def decideOf? (decl : LetDecl .pure) (k : Code .pure)
    : Option (String × FVarId × FVarId × LetDecl .pure × Code .pure) := do
  let .const decider _ #[.fvar a, .fvar b] := decl.value | failure
  let op ← deciderOp decider
  let .let decl2 k2 := k | failure
  let .const ``Decidable.decide _ #[.erased, .fvar f] := decl2.value | failure
  guard (f == decl.fvarId)
  return (op, a, b, decl2, k2)

partial def lowerCode : Code .pure → LowerM String
  | .let decl k => do
    let nm ← registerFVar decl.fvarId decl.binderName
    match decidableIf? decl k with
    | some (op, a, b, elseCode, thenCode) =>
      let a' ← lookupFVar a
      let b' ← lookupFVar b
      let else' ← lowerCode elseCode
      let then' ← lowerCode thenCode
      return s!"(if ({op} {a'} {b'}) {then'} {else'})"
    | none =>
    match decideOf? decl k with
    | some (op, a, b, decl2, k2) =>
      let nm2 ← registerFVar decl2.fvarId decl2.binderName
      let a' ← lookupFVar a
      let b' ← lookupFVar b
      let body ← lowerCode k2
      return s!"(let {nm2} ({op} {a'} {b'}) {body})"
    | none =>
    match decl.value with
    | .erased =>
      -- proof/irrelevant binding: register the name (it may occur in erased
      -- positions we drop) but emit no binding and no opaque marker
      lowerCode k
    | value =>
      if let some op ← knownOpOf value then
        -- Dictionary construction is pure and has no runtime meaning;
        -- retain only its semantic operator for later applications.
        modify fun st => { st with knownOps := st.knownOps.insert nm op }
        lowerCode k
      else
        let val ← lowerLetValue value
        let body ← lowerCode k
        return s!"(let {nm} {val} {body})"
  | .fun (.mk fid bn _ _ _) k => do
    let nm ← registerFVar fid bn
    let val ← opaqueNode s!"{nm}-closure"
    let body ← lowerCode k
    return s!"(let {nm} {val} {body})"
  | .jp (.mk fid bn ps _ v) k => do
    let nm ← registerFVar fid bn
    let pnames ← ps.mapM fun p => registerFVar p.fvarId p.binderName
    let jpBody ← lowerCode v
    let body ← lowerCode k
    -- The IR `jp` node is an expression with no continuation slot; the LCNF
    -- continuation is preserved as a `let` around the join-point declaration.
    return s!"(let {nm} (jp {nm} ({String.intercalate " " pnames.toList}) {jpBody}) {body})"
  | .jmp f args => do
    let nm ← lookupFVar f
    let args' ← lowerArgs args
    return s!"(jmp {nm}{spaced args'})"
  | .cases (.mk _tn _rt discr alts) => do
    let scrut ← lookupFVar discr
    let mut parts : Array String := #[]
    for a in alts do
      match a with
      | .alt ctorName ps c =>
        let pnames ← ps.mapM fun p => registerFVar p.fvarId p.binderName
        let body ← lowerCode c
        parts := parts.push s!"(alt \"{ctorName}\" ({String.intercalate " " pnames.toList}) {body})"
      | .default c =>
        parts := parts.push s!"(default {← lowerCode c})"
      | _ => parts := parts.push (← opaqueNode "ctorAlt")  -- impure phase only
    return s!"(cases {scrut}{spaced parts})"
  | .return f => lookupFVar f
  | .unreach _ => return "(unreachable)"
  | _ => opaqueNode "impure-code"  -- impure-phase-only constructors

/-- Lower an LCNF type expression to the IR type grammar. -/
partial def lowerType (e : Expr) : LowerM String := do
  match e with
  | .const ``Nat _ => return "Nat"
  | .const ``Bool _ => return "Bool"
  | .const ``Int _ => return "Int"
  | .const ``Int8 _ => return "Int8"
  | .const ``Int16 _ => return "Int16"
  | .const ``Int32 _ => return "Int32"
  | .const ``Int64 _ => return "Int64"
  | .const ``UInt8 _ => return "UInt8"
  | .const ``UInt16 _ => return "UInt16"
  | .const ``UInt32 _ => return "UInt32"
  | .const ``UInt64 _ => return "UInt64"
  | .const ``String _ => return "String"
  | .const ``ByteArray _ => return "Bytes"
  | .const ``Ordering _ => return "Ordering"
  | .const n _ =>
    match (← getEnv).find? n with
    | some (.inductInfo _) => return s!"(named \"{n}\")"
    | _ => opaqueType n
  | .app (.app (.const ``Prod _) a) b =>
    return s!"(Tuple {← lowerType a} {← lowerType b})"
  | .app (.const ``List _) a =>
    return s!"(List {← lowerType a})"
  | .app (.const ``Option _) a =>
    return s!"(Option {← lowerType a})"
  | .app (.app (.const ``Except _) error) ok =>
    return s!"(Result {← lowerType ok} {← lowerType error})"
  | _ =>
    match e.getAppFn with
    | .const n _ =>
      match (← getEnv).find? n with
      | some (.inductInfo _) => return s!"(named \"{n}\")"
      | _ => opaqueType n
    | _ => opaqueNode "type-expr"

/-- Strip exactly `n` leading `∀`-binders: the LCNF `Signature.type` is the
    full telescope, and the result type lies under the declaration's params. -/
def stripForalls : Nat → Expr → Expr
  | 0, e => e
  | n + 1, .forallE _ _ b _ => stripForalls n b
  | _, e => e

/-- Lower one pure-phase LCNF declaration to a sexp `def`, returning the sexp
    and the collected lowering state (opaque/extern/dropped facts). -/
def lowerDecl (ctx : LowerCtx) (d : Decl .pure) : CoreM (String × LowerState) := do
  let go : LowerM String := do
    let mut ps : Array String := #[]
    for p in d.params do
      let nm ← registerFVar p.fvarId p.binderName
      let ty ← lowerType p.type
      ps := ps.push s!"({nm} {ty})"
    let ret ← lowerType (stripForalls d.params.size d.type)
    let body ← match d.value with
      | .code c => lowerCode c
      | .extern _ => opaqueNode "extern"
    return s!"(def {lastComponent d.name} ({String.intercalate " " ps.toList}) {ret}\n  {body})"
  (go.run ctx).run {}

/-- Indent every line of `s` by `n` spaces. -/
def indent (n : Nat) (s : String) : String :=
  let pad := String.ofList (List.replicate n ' ')
  String.intercalate "\n" ((s.splitOn "\n").map (pad ++ ·))

/-- Is this expression a `Prop`? Prop-valued structure fields are erased and
    never reach the IR. Runs in `MetaM` because `isProp` needs the local
    context machinery. -/
def isPropType (e : Expr) : LowerM Bool :=
  liftM (Lean.Meta.MetaM.run' (Lean.Meta.isProp e))

/-- Render one inductive as an IR `(type ...)` declaration, erasing `Prop`
    fields.

    A type outside the supported fragment is still declared, carrying the
    reason: codegen then rejects a reference to it by name ("needs
    monomorphization") instead of reporting a generic unknown type. Returns
    `none` only when the constant is not an inductive at all. -/
def lowerTypeDecl (typeName : Name) : LowerM (Option String) := do
  let env ← getEnv
  let some (.inductInfo iv) := env.find? typeName | return none
  let unsupported? : Option String :=
    if iv.numParams != 0 then some "type parameters"
    else if iv.numIndices != 0 then some "type indices"
    else if iv.all.length != 1 then some "mutual inductive block"
    else if iv.isRec then some "recursive"
    else none
  if let some reason := unsupported? then
    return some s!"(type \"{typeName}\" (unsupported \"{reason}\"))"
  let mut ctorSexps : Array String := #[]
  for ctorName in iv.ctors do
    let some (.ctorInfo cv) := env.find? ctorName | return none
    -- Walk the constructor telescope past the (zero) type params to reach the
    -- value fields, pairing each with its declared name.
    -- `getStructureFields` panics for ordinary inductives.  Constructor
    -- binder names are stable only for structures; enums use deterministic
    -- positional names instead.
    let fieldNames :=
      match getStructureInfo? env typeName with
      | some info => info.fieldNames
      | none => #[]
    let mut fields : Array String := #[]
    let mut ty := cv.type
    let mut i := 0
    while i < cv.numFields do
      match ty with
      | .forallE _ fieldTy rest _ =>
        if !(← isPropType fieldTy) then
          let nm := match fieldNames[i]? with
            | some n => sanitize n
            | none => s!"field_{i}"
          fields := fields.push s!"({nm} {← lowerType fieldTy})"
        ty := rest
        i := i + 1
      | _ => i := cv.numFields
    ctorSexps := ctorSexps.push s!"(ctor \"{ctorName}\"{spaced fields})"
  return some s!"(type \"{typeName}\"{spaced ctorSexps})"

/-- Type names a single LCNF `LetValue` mentions: a constructor application
    names its inductive, a projection names the structure it projects from.
    These are exactly the two places `lowerLetValue` emits a full Lean type or
    constructor name into the IR, so they are exactly the places codegen needs
    a matching `(type ...)` declaration for. -/
def letValueTypeNames (env : Environment) (v : LetValue .pure) : Array Name :=
  match v with
  | .proj typeName _ _ => #[typeName]
  | .const declName _ _ =>
    match env.find? declName with
    | some (.ctorInfo cv) => #[cv.induct]
    | _ => #[]
  | _ => #[]

/-- Every type name constructed or projected anywhere in a definition's body. -/
partial def codeTypeNames (env : Environment) : Code .pure → Array Name
  | .let decl k => letValueTypeNames env decl.value ++ codeTypeNames env k
  | .fun (.mk _ _ _ _ v) k => codeTypeNames env v ++ codeTypeNames env k
  | .jp (.mk _ _ _ _ v) k => codeTypeNames env v ++ codeTypeNames env k
  | .cases (.mk _ _ _ alts) =>
    alts.foldl (init := #[]) fun acc a =>
      match a with
      | .alt _ _ c => acc ++ codeTypeNames env c
      | .default c => acc ++ codeTypeNames env c
  | _ => #[]

/-- Every named type a declaration needs declared: the head constant of each
    parameter and return type, **plus** every type its body constructs or
    projects. Only the head constant matters — parameterised types are out of
    scope.

    The body half is not an optimization. A definition like
    `def f (n : Nat) := (NoProp.mk n n).alpha` mentions `NoProp` nowhere in its
    signature, so a signature-only walk never declares it; codegen then has no
    declaration to resolve `(ctor "Conformance.NoProp.mk" ...)` against and
    used to fall through to emitting the dotted Lean name as if it were a Rust
    path. Declaring body-reachable types is what makes that definition
    generate real Rust. (Codegen refuses the dotted path outright now too —
    that is the backstop for IR this function did not produce.) -/
def declTypeNames (env : Environment) (d : Decl .pure) : Array Name := Id.run do
  let mut out : Array Name := #[]
  for p in d.params do
    if let .const n _ := p.type.getAppFn then out := out.push n
  if let .const n _ := (stripForalls d.params.size d.type).getAppFn then
    out := out.push n
  match d.value with
  | .code c => return out ++ codeTypeNames env c
  | _ => return out

end Prod
