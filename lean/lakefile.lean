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

-- Exact LexLean-generated PrismPM fixture modules. The nested roots are
-- compiled directly: no handwritten Lean driver, wrapper, or attribute
-- adapter participates in named export.
lean_lib PrismPMGeneratedFixture where
  roots := #[
    `PrismPM.Foundation.Application,
    `PrismPM.Foundation.Arch,
    `PrismPM.Foundation.Bytes,
    `PrismPM.Foundation.Codec,
    `PrismPM.Foundation.Core,
    `PrismPM.Foundation.Qual,
    `PrismPM.Foundation.Result,
    `PrismPM.Foundation.Runtime,
    `PrismPM.Foundation.Sec,
    `PrismPM.Foundation.Utf8,
    `PrismPM.Foundation.Holo,
    `PrismPM.Foundation.Holo.V1.Archive,
    `PrismPM.Foundation.Holo.V1.Capability,
    `PrismPM.Foundation.Holo.V1.CoreWasm,
    `PrismPM.Foundation.Holo.V1.Directory,
    `PrismPM.Foundation.Holo.V1.Format,
    `PrismPM.Foundation.Holo.V1.Identity,
    `PrismPM.Foundation.Holo.V1.Manifest,
    `PrismPM.Foundation.Holo.V1.PrismExtension,
    `PrismPM.Foundation.Holo.V1.SourceManifest,
    `PrismPM.Foundation.Holo.V1.View,
    `PrismPM.Foundation.Integer,
    `PrismPM.Foundation.View.V1.Interaction,
    `PrismPM.Foundation.View.V1.Model,
    `PrismPM.Foundation.View.V1.Projection
  ]

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
