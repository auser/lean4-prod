#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd -P)
fixture="$repo_root/fixtures/lexlean-1.1"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/lean4-prod-named.XXXXXXXX")
trap 'rm -rf -- "$scratch"' EXIT

first="$scratch/first"
second="$scratch/second"
mkdir -p "$first" "$second"

cd "$repo_root/lean"
lake build Conformance.LexLean11 Conformance.BadRoots
lake env lean Conformance/LocalFunctions.lean

export_once() {
  local out=$1
  lake exe prod-export \
    --module Conformance.LexLean11 \
    --root SemanticFixture.Main.allConsecutive \
    --ir-module SemanticFixture \
    --out "$out"
}

export_once "$first"
export_once "$second"

for artifact in kernel.ir roots.json coverage.json; do
  cmp "$first/$artifact" "$second/$artifact"
  cmp "$first/$artifact" "$fixture/expected/$artifact"
done

cd "$repo_root/rust"
cargo run -p prod-cli -- validate "$first/kernel.ir"
cargo run -p prod-cli -- gen "$first/kernel.ir" --output "$first/generated.rs"
cargo run -p prod-cli -- gen "$second/kernel.ir" --output "$second/generated.rs"
cmp "$first/generated.rs" "$second/generated.rs"
cmp "$first/generated.rs" "$fixture/expected/generated.rs"

expect_failure() {
  local expected=$1
  shift
  local stderr="$scratch/failure.stderr"
  if "$@" >"$scratch/failure.stdout" 2>"$stderr"; then
    echo "expected command to fail: $*" >&2
    exit 1
  fi
  grep -F -- "$expected" "$stderr" >/dev/null
}

cd "$repo_root/lean"
expect_failure "named export requires at least one --root" \
  lake exe prod-export --module Conformance.BadRoots --ir-module Bad --out "$scratch/bad"
expect_failure "duplicate root Conformance.BadRoots.alpha" \
  lake exe prod-export --module Conformance.BadRoots \
    --root Conformance.BadRoots.alpha --root Conformance.BadRoots.alpha \
    --ir-module Bad --out "$scratch/bad"
expect_failure "roots are not strictly sorted" \
  lake exe prod-export --module Conformance.BadRoots \
    --root Conformance.BadRoots.zeta --root Conformance.BadRoots.alpha \
    --ir-module Bad --out "$scratch/bad"
expect_failure "root Conformance.BadRoots.missing is missing" \
  lake exe prod-export --module Conformance.BadRoots \
    --root Conformance.BadRoots.missing --ir-module Bad --out "$scratch/bad"
expect_failure "root Conformance.BadRoots.theoremRoot is theorem-only" \
  lake exe prod-export --module Conformance.BadRoots \
    --root Conformance.BadRoots.theoremRoot --ir-module Bad --out "$scratch/bad"
expect_failure "root Conformance.BadRoots.unsafeRoot is unsafe" \
  lake exe prod-export --module Conformance.BadRoots \
    --root Conformance.BadRoots.unsafeRoot --ir-module Bad --out "$scratch/bad"
expect_failure "root Conformance.BadRoots.partialRoot is opaque, partial, or noncomputable" \
  lake exe prod-export --module Conformance.BadRoots \
    --root Conformance.BadRoots.partialRoot --ir-module Bad --out "$scratch/bad"
expect_failure "root Conformance.BadRoots.noncomputableRoot is opaque, partial, or noncomputable" \
  lake exe prod-export --module Conformance.BadRoots \
    --root Conformance.BadRoots.noncomputableRoot --ir-module Bad --out "$scratch/bad"
expect_failure "root Conformance.BadRoots.typeValuedRoot does not generate code" \
  lake exe prod-export --module Conformance.BadRoots \
    --root Conformance.BadRoots.typeValuedRoot --ir-module Bad --out "$scratch/bad"

expect_failure "unsupported local function in Conformance.BadRoots.escapingCapturedFunction: Prod.LocalFunctionError.escaping" \
  lake exe prod-export --module Conformance.BadRoots \
    --root Conformance.BadRoots.escapingCapturedFunction \
    --ir-module Bad --out "$scratch/bad"

# The kernel body of a pattern-matching definition refers first to a generated
# matcher. Named closure discovery must scan that internal helper so its public
# callees are included, while keeping the matcher itself out of the public IR.
lake exe prod-export --module Conformance.BadRoots \
  --root Conformance.BadRoots.matchedCallee \
  --ir-module MatchedCallee --out "$scratch/matched-callee"
grep -F -- 'Conformance.BadRoots.belowLimit' \
  "$scratch/matched-callee/roots.json" >/dev/null
if grep -F -- 'matchedCallee.match_' "$scratch/matched-callee/roots.json" >/dev/null; then
  echo "generated matcher leaked into named export closure" >&2
  exit 1
fi

# A root's user type can itself contain another user type (including through
# List/Option). The generated IR must carry that complete type graph so the
# Rust generator never receives an undeclared nested field type.
lake exe prod-export --module Conformance.BadRoots \
  --root Conformance.BadRoots.nestedOwnerMembers \
  --ir-module NestedOwner --out "$scratch/nested-owner"
grep -F -- '(type "Conformance.BadRoots.NestedMember"' \
  "$scratch/nested-owner/kernel.ir" >/dev/null
cd "$repo_root/rust"
cargo run -p prod-cli -- validate "$scratch/nested-owner/kernel.ir"
cargo run -p prod-cli -- gen "$scratch/nested-owner/kernel.ir" \
  --output "$scratch/nested-owner/generated.rs"

# Structure projections are expressions plus owner type metadata, never free
# exported functions. Same-spelled fields on distinct structures must compile
# without duplicate Rust items.
cd "$repo_root/lean"
lake exe prod-export --module Conformance.BadRoots \
  --root Conformance.BadRoots.sumProjectionIds \
  --ir-module ProjectionFields --out "$scratch/projection-fields"
if grep -F -- 'Conformance.BadRoots.ProjectionLeft.id' \
  "$scratch/projection-fields/roots.json" >/dev/null; then
  echo "structure projection leaked into named export closure" >&2
  exit 1
fi
cd "$repo_root/rust"
cargo run -p prod-cli -- validate "$scratch/projection-fields/kernel.ir"
cargo run -p prod-cli -- gen "$scratch/projection-fields/kernel.ir" \
  --output "$scratch/projection-fields/generated.rs"
if grep -E -- '^pub fn (id|ProjectionLeft\.id|ProjectionRight\.id)\(' \
  "$scratch/projection-fields/generated.rs" >/dev/null; then
  echo "structure projection rendered as a free Rust function" >&2
  exit 1
fi

# First-order repeated enum alternatives must not become an unsupported
# runtime closure solely because Lean shares two identical branch bodies.
cd "$repo_root/lean"
lake exe prod-export --module Conformance.BadRoots \
  --root Conformance.BadRoots.largeNatReceiver \
  --root Conformance.BadRoots.sharedEnumScalar \
  --ir-module SharedEnum --out "$scratch/shared-enum"
lake exe prod-export --module Conformance.BadRoots \
  --root Conformance.BadRoots.largeNatReceiver \
  --root Conformance.BadRoots.sharedEnumScalar \
  --ir-module SharedEnum --out "$scratch/shared-enum-repeat"
for artifact in kernel.ir roots.json coverage.json; do
  cmp "$scratch/shared-enum/$artifact" "$scratch/shared-enum-repeat/$artifact"
done
cd "$repo_root/rust"
cargo run -p prod-cli -- validate "$scratch/shared-enum/kernel.ir"
cargo run -p prod-cli -- gen "$scratch/shared-enum/kernel.ir" \
  --output "$scratch/shared-enum/generated.rs"
cargo run -p prod-cli -- header "$scratch/shared-enum/kernel.ir" \
  --output "$scratch/shared-enum/fixture.h" \
  --rust-output "$scratch/shared-enum/ffi.rs"
node "$repo_root/scripts/check-local-functions.mjs" "$scratch/shared-enum"

echo "named-export conformance passed"
