#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd -P)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/lean4-prod-decimal.XXXXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
node "$repo_root/scripts/check-typed-decimal-provenance.mjs"

cd "$repo_root/lean"
lake build Conformance.LexLeanDecimal
roots=(acceptUInt8 entry nonzeroUInt16 parseInt16 parseInt32 parseInt64 parseInt8 parseUint16 parseUint32 parseUint64 parseUint8)
arguments=()
for root in "${roots[@]}"; do arguments+=(--root "DecimalFixture.Main.$root"); done
for output in first second; do
  lake exe prod-export --module Conformance.LexLeanDecimal "${arguments[@]}" \
    --ir-module TypedDecimal --out "$scratch/$output-export"
done
for artifact in kernel.ir roots.json coverage.json; do
  cmp "$scratch/first-export/$artifact" "$scratch/second-export/$artifact"
done
for type in Int8 Int16 Int32 Int64 UInt8 UInt16 UInt32 UInt64; do
  if ! rg -Fq -- "(parse-decimal-as $type " "$scratch/first-export/kernel.ir"; then
    echo "typed decimal export lost its exact $type target" >&2
    exit 1
  fi
done
if rg -q '\(parse-decimal ' "$scratch/first-export/kernel.ir"; then
  echo 'typed decimal export discarded a result type' >&2
  exit 1
fi

generate_native() {
  local output=$1
  cd "$repo_root/rust"
  RUSTC_WRAPPER= cargo run --locked --offline -p prod-cli -- cargo \
    "$scratch/first-export/kernel.ir" --output "$output" \
    --name typed-decimal --version 0.1.0 \
    --description 'Real LexLean typed-decimal compiler fixture' \
    --repository https://github.com/auser/lean4-prod \
    --homepage https://github.com/auser/lean4-prod \
    --readme "$repo_root/fixtures/lexlean-decimal/README.md" \
    --license-mit "$repo_root/rust/prod-codegen/tests/fixtures/LICENSE-MIT" \
    --license-apache /usr/share/common-licenses/Apache-2.0
}
generate_native "$scratch/first"
generate_native "$scratch/second"
diff -ru "$scratch/first" "$scratch/second"
mkdir "$scratch/first/tests"
cp "$repo_root/rust/prod-codegen/tests/fixtures/typed_decimal_generated_test.rs" \
  "$scratch/first/tests/decimal.rs"
cd "$scratch/first"
RUSTC_WRAPPER= cargo test --locked --offline
RUSTC_WRAPPER= cargo test --locked --offline --no-default-features

for output in first-guest second-guest; do
  cd "$repo_root/rust"
  RUSTC_WRAPPER= cargo run --locked --offline -p prod-cli -- core-wasm \
    "$scratch/first-export/kernel.ir" --output "$scratch/$output" \
    --entry entry --export-name holo_run --input-allocation-cap 128 \
    --output-allocation-cap 128 --maximum-pages 4 --crate-name typed-decimal-guest
  cd "$scratch/$output"
  RUSTC_WRAPPER= cargo build --release --locked --offline
  node "$repo_root/rust/prod-codegen/tests/fixtures/typed_decimal_wasm_test.mjs" \
    "$scratch/$output/target/wasm32-unknown-unknown/release/typed_decimal_guest.wasm"
done
cmp "$scratch/first-guest/target/wasm32-unknown-unknown/release/typed_decimal_guest.wasm" \
  "$scratch/second-guest/target/wasm32-unknown-unknown/release/typed_decimal_guest.wasm"
cmp "$scratch/first-guest/generation-manifest.json" "$scratch/second-guest/generation-manifest.json"

# Even a discarded Option Int payload must not acquire a fixed-width meaning.
for root in acceptInt parseInt; do
  cd "$repo_root/lean"
  lake exe prod-export --module Conformance.LexLeanDecimal \
    --root "DecimalFixture.Main.$root" --ir-module MathematicalInt \
    --out "$scratch/$root-export"
  rg -Fq '(parse-decimal-as Int ' "$scratch/$root-export/kernel.ir"
  if "$repo_root/rust/target/debug/prod" cargo "$scratch/$root-export/kernel.ir" \
      --output "$scratch/$root-package" --name rejected-int --version 0.1.0 \
      --description rejected --repository https://github.com/auser/lean4-prod \
      --homepage https://github.com/auser/lean4-prod \
      --readme "$repo_root/fixtures/lexlean-decimal/README.md" \
      --license-mit "$repo_root/rust/prod-codegen/tests/fixtures/LICENSE-MIT" \
      --license-apache /usr/share/common-licenses/Apache-2.0 \
      >"$scratch/$root.stdout" 2>"$scratch/$root.stderr"; then
    echo "mathematical Int decimal parsing unexpectedly accepted: $root" >&2
    exit 1
  fi
  rg -Fq 'mathematical Lean `Int` is unbounded and cannot be represented by a fixed-width Rust integer' "$scratch/$root.stderr"
  test ! -e "$scratch/$root-package"
done
echo 'real typed-decimal export, native/std/no_std, deterministic Wasm and mathematical-Int rejection passed'
