import Prod
import UOR.Enums

/-!
Scalar production adapters over the pinned UOR Framework formalization.

UOR's complete generated model includes polymorphic structures and arrays,
which intentionally remain outside the current foreign-language scalar ABI.
These definitions exercise real UOR code while exposing values every SDK can
represent without inventing an ownership or layout contract.
-/

namespace UORProd

@[prod] def wittBits (n : Nat) : Nat :=
  WittLevel.bitsWidth (WittLevel.new n)

@[prod] def wittBytes (n : Nat) : Nat :=
  WittLevel.bitsWidth (WittLevel.new n) / 8

@[prod] def addIsCommutative : Bool :=
  PrimitiveOp.isCommutative .add

end UORProd
