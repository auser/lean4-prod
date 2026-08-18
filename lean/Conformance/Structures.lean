-- Probes for the Lean-structure-field → LCNF-projection-index correspondence.
-- Fields carry distinguishable values on purpose: if the mapping were wrong,
-- c_proj_middle_prop would return the fields in the wrong order and the golden
-- would change. See AGENTS.md for the rule these pin down.
import Prod.Attribute

namespace Conformance

/-- Ordinary inductives are not structures. Their export must not call
    `getStructureFields`, which reports a Lean panic even though export can
    otherwise continue and misleadingly exit successfully. -/
inductive SmallEnum where
  | left
  | right (value : Nat)

@[prod] def c_enum_identity (x : SmallEnum) : SmallEnum := x

/-- Prop field in the MIDDLE, not at the end: the case the existing
    `UorAtlas.Instance` (whose proof field is last) does not exercise. -/
structure MidProp where
  first  : Nat
  ok     : first ≥ 0
  second : Nat
  third  : Nat

/-- All-computational structure, as a control. -/
structure NoProp where
  alpha : Nat
  beta  : Nat

@[prod] def c_proj_middle_prop (m : MidProp) : Nat × Nat × Nat :=
  (m.first, m.second, m.third)

@[prod] def c_proj_no_prop (n : NoProp) : Nat × Nat :=
  (n.alpha, n.beta)

/-- A purely intermediate structure: no `@[prod]` definition mentions it in a
    parameter or return type, so it exists in the IR only because a *body*
    constructs and projects it. `MidProp`/`NoProp` above cannot pin this — both
    are parameters of their conformance defs.

    The exporter's type collection (`Prod.declTypeNames`) used to walk
    signatures alone, so this lowered to `(ctor "Conformance.BodyOnly.mk" ...)`
    with no matching `(type ...)` declaration, and codegen rendered the dotted
    Lean name as a Rust path — `Conformance.BodyOnly.mk(a, b)`, which is not
    Rust — and exited 0. The golden must show a `(type "Conformance.BodyOnly"
    ...)` declaration; if the body walk regresses, it disappears and
    `just conformance` fails. -/
structure BodyOnly where
  alpha : Nat
  beta  : Nat

/-- Kept out of the optimizer's reach on purpose. The obvious spelling,
    `(BodyOnly.mk a b).alpha`, is folded by LCNF to `a` and the constructor
    never reaches the IR at all; branching first forces the structure to be a
    real intermediate value. The `match` is on `Nat` rather than a decidable
    comparison so the case stays inside the published subset (a `let`-bound
    `if` lands its `cases` behind a join point, where `decidableIf?` no longer
    recognizes it, and the decider surfaces as an extern).

    NOTE: this definition's own lowering produces a TWO-CALLER join point (both
    match arms feed one continuation), which codegen rejects as
    `UnsupportedJoinPoint`. That is deliberate and it does not weaken the case:
    the conformance harness pins LOWERING, and what this pins is that
    `declTypeNames` reaches a type used only in a body. The codegen half — that
    a body-only type is declared and then renders — is pinned separately by
    `test_ctor_in_a_definition_body_only_renders_when_declared` in
    `rust/prod-codegen/src/tests.rs`. If join points ever gain a real lowering,
    this definition starts generating too, and nothing here needs to change. -/
@[prod] def c_ctor_body_only (fuel a b : Nat) : Nat :=
  let s := match fuel with
    | 0 => BodyOnly.mk a b
    | _ + 1 => BodyOnly.mk b a
  s.alpha + s.beta

end Conformance
