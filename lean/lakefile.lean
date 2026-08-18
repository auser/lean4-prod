import Lake
open Lake DSL

-- No external libraries: load-bearing identities are proved in pure Lean 4 by
-- `decide` / `omega` / `rfl` (same discipline as uor-addr-lean).
package «lean4-prod» where

lean_lib ProdLib where
  roots := #[`Prod]

@[default_target]
lean_lib Example where
  roots := #[`Example]

@[default_target]
lean_lib Conformance where
  roots := #[`Conformance]

-- Test-only Lean definitions and theorem proofs. This library is compiled by
-- `just lean-fixtures` but is deliberately not imported by `Prod.Emit`, so
-- proof fixtures cannot change production IR or committed goldens.
@[default_target]
lean_lib ProofFixtures where
  roots := #[`ProofFixtures]

@[default_target]
lean_exe «prod-export» where
  root := `Prod.Emit
  supportInterpreter := true
