#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd -P)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/lean4-prod-package.XXXXXXXX")
trap 'rm -rf -- "$scratch"' EXIT

cd "$repo_root/lean"
lake build Conformance.LexLeanPortable
lake exe prod-export \
  --module Conformance.LexLeanPortable \
  --root SemanticFixture.Portable.andUInt64 \
  --root SemanticFixture.Portable.appendBytes \
  --root SemanticFixture.Portable.byteAt \
  --root SemanticFixture.Portable.byteLength \
  --root SemanticFixture.Portable.checkedAddInt64 \
  --root SemanticFixture.Portable.checkedMultiplyInt64 \
  --root SemanticFixture.Portable.checkedNegateInt64 \
  --root SemanticFixture.Portable.checkedQuotientInt64 \
  --root SemanticFixture.Portable.checkedSubtractInt64 \
  --root SemanticFixture.Portable.compareByteStrings \
  --root SemanticFixture.Portable.decodeUtf8 \
  --root SemanticFixture.Portable.encodeUtf8 \
  --root SemanticFixture.Portable.formatInt64 \
  --root SemanticFixture.Portable.isZeroInt64 \
  --root SemanticFixture.Portable.joinStrings \
  --root SemanticFixture.Portable.maximumUInt64 \
  --root SemanticFixture.Portable.notUInt64 \
  --root SemanticFixture.Portable.orUInt64 \
  --root SemanticFixture.Portable.parseInt64 \
  --root SemanticFixture.Portable.shiftRightUInt64 \
  --root SemanticFixture.Portable.shiftUInt64 \
  --root SemanticFixture.Portable.sliceBytes \
  --root SemanticFixture.Portable.splitBounded \
  --root SemanticFixture.Portable.unicodeFixture \
  --root SemanticFixture.Portable.xorUInt64 \
  --ir-module PortableExpanded \
  --out "$scratch/export"

generate() {
  local output=$1
  cd "$repo_root/rust"
  RUSTC_WRAPPER= cargo run --locked --offline -p prod-cli -- cargo \
    "$scratch/export/kernel.ir" \
    --output "$output" \
    --name portable-expanded \
    --version 0.1.0 \
    --description "Real LexLean portable-operation fixture" \
    --repository https://example.invalid/portable \
    --homepage https://example.invalid/ \
    --readme "$repo_root/README.md" \
    --license-mit "$repo_root/rust/prod-codegen/tests/fixtures/LICENSE-MIT" \
    --license-apache /usr/share/common-licenses/Apache-2.0
}

generate "$scratch/first"
generate "$scratch/second"
diff -ru --exclude target "$scratch/first" "$scratch/second"
mkdir "$scratch/first/tests"
cp "$repo_root/rust/prod-codegen/tests/fixtures/portable_generated_test.rs" \
  "$scratch/first/tests/portable.rs"

cd "$scratch/first"
RUSTC_WRAPPER= cargo test --locked --offline
RUSTC_WRAPPER= cargo test --locked --offline --no-default-features
RUSTC_WRAPPER= cargo package --locked --offline --allow-dirty

cd "$repo_root/lean"
lake exe prod-export \
  --module Conformance.LexLeanPortable \
  --root SemanticFixture.Portable.subtractInt \
  --ir-module RejectedMathematicalInt \
  --out "$scratch/int-export"
if "$repo_root/rust/target/debug/prod" cargo "$scratch/int-export/kernel.ir" \
    --output "$scratch/int-package" \
    --name rejected-int \
    --version 0.1.0 \
    --description rejected \
    --repository https://example.invalid/rejected \
    --homepage https://example.invalid/ \
    --readme "$repo_root/README.md" \
    --license-mit "$repo_root/rust/prod-codegen/tests/fixtures/LICENSE-MIT" \
    --license-apache /usr/share/common-licenses/Apache-2.0 \
    >"$scratch/int.stdout" 2>"$scratch/int.stderr"; then
  echo "mathematical Int closure unexpectedly generated a fixed-width package" >&2
  exit 1
fi
rg -q 'mathematical Lean `Int` is unbounded and cannot be represented by a fixed-width Rust integer' "$scratch/int.stderr"

echo "real LexLean portable Cargo package and mathematical-Int rejection passed"
