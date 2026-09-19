module
public import Init

namespace Conformance.BadRoots

public theorem theoremRoot : True := True.intro

public unsafe def unsafeRoot (value : Nat) : Nat := value

public partial def partialRoot (value : Nat) : Nat := partialRoot value

public opaque noncomputableRoot : Nat

public def typeValuedRoot : Type := Nat

public def alpha (value : Nat) : Nat := value

public def zeta (value : Nat) : Nat := alpha value

public def belowLimit (value : Nat) : Bool := value < 4

public def matchedCallee : List Nat → Bool
  | [] => true
  | value :: rest => belowLimit value && matchedCallee rest

public structure NestedMember where
  value : Nat

public structure NestedOwner where
  members : List NestedMember

public def nestedOwnerMembers (owner : NestedOwner) : List NestedMember := owner.members

public structure ProjectionLeft where
  id : Nat

public structure ProjectionRight where
  id : Nat

public def sumProjectionIds (left : ProjectionLeft) (right : ProjectionRight) : Nat :=
  left.id + right.id

public inductive SharedOperation where
  | create | inspect | restore | replace

public structure SharedState where
  value : Nat

public def sharedPredicate (left right : Nat) : Bool := left == right

-- Ordinary repeated alternatives cause Lean's base LCNF to share a captured
-- local function. The source itself contains no lambda or higher-order value.
public def sharedEnumMatch (state : SharedState) (operation : SharedOperation)
    (candidate : Nat) : Bool :=
  match operation with
  | .create => sharedPredicate candidate 17
  | .inspect => sharedPredicate candidate state.value
  | .restore => sharedPredicate candidate state.value
  | .replace => sharedPredicate candidate 17

public theorem sharedEnumMatch_spec (state : SharedState) (operation : SharedOperation)
    (candidate : Nat) : sharedEnumMatch state operation candidate =
      (candidate == match operation with
        | .create | .replace => 17
        | .inspect | .restore => state.value) := by
  cases operation <;> rfl

public def sharedEnumScalar (first second : Bool) (state candidate : Nat) : Bool :=
  let operation := match first, second with
    | false, false => SharedOperation.create
    | false, true => SharedOperation.inspect
    | true, false => SharedOperation.restore
    | true, true => SharedOperation.replace
  sharedEnumMatch ⟨state⟩ operation candidate

public def largeNatReceiver (input : Nat) : Nat :=
  let maximum := 4294967295
  maximum - input

public structure CapturedHolder where
  apply : Nat → Nat

public def escapingCapturedFunction (capture : Nat) : CapturedHolder :=
  ⟨fun value => capture + value⟩

end Conformance.BadRoots
