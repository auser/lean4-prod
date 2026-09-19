import Lean

/-! Validate the first-order local-function subset before emitting continuation
IR. This is not runtime closure support: function values cannot escape, calls
must be saturated, and local call graphs must be acyclic. -/

open Lean Compiler LCNF

namespace Prod

inductive LocalFunctionError where
  | inputLimit
  | depthLimit
  | escaping (name : Name)
  | arity (name : Name)
  | recursive (name : Name)
  | unsupported
  deriving BEq, Repr

-- Aggregate Code nodes plus argument/parameter elements, independent of any
-- application capacity. Rust separately preflights fully expanded output.
def localFunctionNodeLimit : Nat := 65536
def localFunctionDepthLimit : Nat := 128

private abbrev LocalArities := Std.HashMap Name Nat
private abbrev LocalEdges := Std.HashMap Name (Array Name)

private def rejectValue (functions : LocalArities) (id : FVarId)
    : Except LocalFunctionError Unit :=
  if functions.contains id.name then .error (.escaping id.name) else .ok ()

private def checkArguments (functions : LocalArities) (args : Array (Arg .pure))
    : Except LocalFunctionError Unit := do
  if args.size > localFunctionNodeLimit then throw .inputLimit
  for arg in args do
    if let .fvar id := arg then rejectValue functions id

private partial def visitLocal (edges : LocalEdges) (name : Name)
    (active : Array Name) (done : Std.HashMap Name Nat)
    : Except LocalFunctionError (Nat × Std.HashMap Name Nat) := do
  if active.contains name then throw (.recursive name)
  if let some depth := done[name]? then return (depth, done)
  if active.size >= localFunctionDepthLimit then throw .depthLimit
  let mut done := done
  let mut depth := 1
  for dependency in edges[name]?.getD #[] do
    let (childDepth, updated) ← visitLocal edges dependency (active.push name) done
    done := updated
    depth := max depth (childDepth + 1)
    if depth > localFunctionDepthLimit then throw .depthLimit
  return (depth, done.insert name depth)

/-- Inspect code iteratively, without expanding any local body. FVarId identity
    preserves lexical captures even when source binder names collide. -/
private abbrev WorkItem := Code .pure × Option Name × Std.HashSet Name

private def reserveInput (used extra : Nat) : Except LocalFunctionError Nat :=
  if used + extra > localFunctionNodeLimit then .error .inputLimit
  else .ok (used + extra)

private def pushNode (nodes : Array WorkItem) (node : WorkItem)
    : Except LocalFunctionError (Array WorkItem) :=
  if nodes.size >= localFunctionNodeLimit then .error .inputLimit
  else .ok (nodes.push node)

def firstOrderLocalFunctions (code : Code .pure)
    : Except LocalFunctionError (Std.HashSet Name) := do
  let mut nodes : Array WorkItem := #[(code, none, {})]
  let mut functions : LocalArities := {}
  let mut cursor := 0
  let mut used := 0
  while cursor < nodes.size do
    if nodes.size > localFunctionNodeLimit then throw .inputLimit
    let (node, owner, visible) := nodes[cursor]!
    cursor := cursor + 1
    used ← reserveInput used 1
    match node with
    | .let decl rest =>
      match decl.value with
      | .fvar _ args | .const _ _ args => used ← reserveInput used args.size
      | _ => pure ()
      nodes ← pushNode nodes (rest, owner, visible)
    | .fun decl rest =>
      if functions.contains decl.fvarId.name then throw .unsupported
      used ← reserveInput used decl.params.size
      functions := functions.insert decl.fvarId.name decl.params.size
      let visible := visible.insert decl.fvarId.name
      nodes ← pushNode nodes (decl.value, some decl.fvarId.name, visible)
      nodes ← pushNode nodes (rest, owner, visible)
    | .jp decl rest =>
      used ← reserveInput used decl.params.size
      nodes ← pushNode nodes (decl.value, owner, visible)
      nodes ← pushNode nodes (rest, owner, visible)
    | .cases cases =>
      for alt in cases.alts do
        match alt with
        | .alt _ params body =>
          used ← reserveInput used params.size
          nodes ← pushNode nodes (body, owner, visible)
        | .default body => nodes ← pushNode nodes (body, owner, visible)
        | _ => throw .unsupported
    | .jmp _ args => used ← reserveInput used args.size
    | .return .. | .unreach .. => pure ()
    | _ => throw .unsupported
  let mut edges : LocalEdges := {}
  for (node, owner, visible) in nodes do
    match node with
    | .let decl _ =>
      match decl.value with
      | .fvar id args =>
        checkArguments functions args
        if let some arity := functions[id.name]? then
          if !visible.contains id.name then throw (.escaping id.name)
          if args.size != arity then throw (.arity id.name)
          -- lowerArgs erases proof/type arguments. This subset requires one
          -- runtime argument per parameter instead of emitting mismatched IR.
          for arg in args do
            match arg with
            | .fvar _ => pure ()
            | .erased | .type .. => throw .unsupported
          if let some owner := owner then
            let previous := edges[owner]?.getD #[]
            if !previous.contains id.name then
              edges := edges.insert owner (previous.push id.name)
      | .const _ _ args => checkArguments functions args
      | .proj _ _ id => rejectValue functions id
      | .lit .. | .erased => pure ()
      | _ => throw .unsupported
    | .return id => rejectValue functions id
    | .cases cases => rejectValue functions cases.discr
    | .jmp id args =>
      rejectValue functions id
      checkArguments functions args
    | .fun .. | .jp .. | .unreach .. => pure ()
    | _ => throw .unsupported
  let mut done : Std.HashMap Name Nat := {}
  let ordered := functions.toArray.qsort fun left right => Name.quickCmp left.1 right.1 == .lt
  for (name, _) in ordered do
    let (_, updated) ← visitLocal edges name #[] done
    done := updated
  return ordered.foldl (fun result pair => result.insert pair.1) {}

end Prod
