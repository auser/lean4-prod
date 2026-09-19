import Prod.LocalFunctions

open Lean Compiler LCNF Prod

namespace Conformance.LocalFunctions

private def ident (name : Name) : FVarId := ⟨name⟩
private def param (name : Name) : Param .pure :=
  ⟨ident name, name, mkConst ``Nat, false⟩
private def value (name : Name) : Code .pure := .return (ident name)
private def invoke (name : Name) (args : Array (Arg .pure)) : Code .pure :=
  .let ⟨ident `result, `result, mkConst ``Nat, .fvar (ident name) args⟩ (value `result)
private def function (name : Name) (body rest : Code .pure) : Code .pure :=
  .fun (.mk (ident name) name #[param `argument]
    (.forallE `argument (mkConst ``Nat) (mkConst ``Nat) .default) body) rest
private def expectError (label : String) (expected : LocalFunctionError)
    (code : Code .pure) : IO Unit := do
  match firstOrderLocalFunctions code with
  | .error actual =>
    unless actual == expected do
      throw (IO.userError s!"{label}: {repr actual}, expected {repr expected}")
  | .ok _ => throw (IO.userError s!"{label}: unexpectedly accepted")

private def chain (count : Nat) : Code .pure := Id.run do
  let name := fun index => Name.num `function index
  let mut code := invoke (name (count - 1)) #[.fvar (ident `input)]
  for index in (List.range count).reverse do
    let body := if index == 0 then value `argument
      else invoke (name (index - 1)) #[.fvar (ident `argument)]
    code := function (name index) body code
  return code

private def wide (count : Nat) : Code .pure :=
  .cases (.mk ``Bool (mkConst ``Nat) (ident `condition)
    (Array.replicate count (.default (value `input))))

private def arguments (count : Nat) : Code .pure :=
  .let ⟨ident `result, `result, mkConst ``Nat,
    .const `callee [] (Array.replicate count (.fvar (ident `input)))⟩ (value `result)

private def parameters (count : Nat) : Code .pure :=
  .jp (.mk (ident `join) `join (Array.replicate count (param `argument))
    (mkConst ``Nat) (value `input)) (value `input)

private def alternativeParameters (count : Nat) : Code .pure :=
  .cases (.mk `Large (mkConst ``Nat) (ident `input)
    #[.alt `Large.mk (Array.replicate count (param `argument)) (value `input)])

#eval do
  expectError "partial application" (.arity `local)
    (function `local (value `argument) (invoke `local #[]))
  expectError "overapplication" (.arity `local)
    (function `local (value `argument)
      (invoke `local #[.fvar (ident `input), .fvar (ident `input)]))
  expectError "erased local argument" .unsupported
    (function `local (value `input) (invoke `local #[.erased]))
  expectError "type local argument" .unsupported
    (function `local (value `input) (invoke `local #[.type (mkConst ``Nat)]))
  expectError "returned closure" (.escaping `local)
    (function `local (value `argument) (value `local))
  expectError "passed closure" (.escaping `local)
    (function `local (value `argument)
      (.let ⟨ident `result, `result, mkConst ``Nat,
        .const `consumer [] #[.fvar (ident `local)]⟩ (value `result)))
  expectError "projected closure" (.escaping `local)
    (function `local (value `argument)
      (.let ⟨ident `result, `result, mkConst ``Nat,
        .proj `Owner 0 (ident `local)⟩ (value `result)))
  expectError "scrutinized closure" (.escaping `local)
    (function `local (value `argument)
      (.cases (.mk ``Bool (mkConst ``Nat) (ident `local) #[])))
  expectError "recursive closure" (.recursive `local)
    (function `local (invoke `local #[.fvar (ident `argument)])
      (invoke `local #[.fvar (ident `input)]))
  expectError "out-of-scope call" (.escaping `inner)
    (function `outer
      (function `inner (value `argument) (invoke `inner #[.fvar (ident `argument)]))
      (invoke `inner #[.fvar (ident `input)]))
  expectError "mutual recursion" (.recursive `outer)
    (function `outer
      (function `inner (invoke `outer #[.fvar (ident `argument)])
        (invoke `inner #[.fvar (ident `argument)]))
      (invoke `outer #[.fvar (ident `input)]))
  for code in [chain localFunctionDepthLimit, wide (localFunctionNodeLimit - 1)] do
    if let .error reason := firstOrderLocalFunctions code then
      throw (IO.userError s!"exact bound rejected: {repr reason}")
  expectError "one more local call" .depthLimit (chain (localFunctionDepthLimit + 1))
  expectError "one more code node" .inputLimit (wide localFunctionNodeLimit)
  for (make, overhead) in [(arguments, 2), (parameters, 3), (alternativeParameters, 2)] do
    if let .error reason := firstOrderLocalFunctions (make (localFunctionNodeLimit - overhead)) then
      throw (IO.userError s!"exact aggregate bound rejected: {repr reason}")
    expectError "one more aggregate element" .inputLimit
      (make (localFunctionNodeLimit - overhead + 1))
  let args : Array (Arg .pure) := Array.replicate (localFunctionNodeLimit / 2) (.fvar (ident `input))
  expectError "individually bounded arrays exceed aggregate" .inputLimit
    (.let ⟨ident `first, `first, mkConst ``Nat, .const `callee [] args⟩
      (.let ⟨ident `second, `second, mkConst ``Nat, .const `callee [] args⟩ (value `second)))
  IO.println "local-function eligibility: 22 cases passed"

end Conformance.LocalFunctions
