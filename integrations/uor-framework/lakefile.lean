import Lake
open Lake DSL

package «uor-prod-integration» where

require «lean4-prod» from "../../lean"
require uor from git
  "https://github.com/UOR-Foundation/UOR-Framework.git" @
  "51c01382200b0179d6640b07e9c8119364ab69a1"

lean_lib UORProd where
  roots := #[`UORProd]
