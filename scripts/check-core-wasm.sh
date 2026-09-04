#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd -P)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/lean4-prod-core-wasm.XXXXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
wasm_tools=$(command -v wasm-tools || true)
wasmtime=$(command -v wasmtime || true)
test -n "$wasm_tools" || wasm_tools=/home/vscode/.cargo/bin/wasm-tools
test -n "$wasmtime" || wasmtime=/home/vscode/.cargo/bin/wasmtime
test -x "$wasm_tools"
test -x "$wasmtime"

cd "$repo_root/lean"
lake build Conformance.LexLeanPortableRecursion
lake exe prod-export \
  --module Conformance.LexLeanPortableRecursion \
  --root SemanticFixture.PortableRecursion.echoBytes \
  --ir-module LexLeanPortableRecursion \
  --out "$scratch/export"

generate() {
  local output=$1
  cd "$repo_root/rust"
  RUSTC_WRAPPER= cargo run --locked --offline -p prod-cli -- core-wasm \
    "$scratch/export/kernel.ir" \
    --output "$output" \
    --entry echoBytes \
    --export-name holo_run \
    --input-allocation-cap 65536 \
    --output-allocation-cap 65536 \
    --maximum-pages 4 \
    --crate-name lexlean-echo-guest
  cd "$output"
  RUSTC_WRAPPER= cargo build --release --locked --offline
}

generate "$scratch/first"
generate "$scratch/second"
first_wasm="$scratch/first/target/wasm32-unknown-unknown/release/lexlean_echo_guest.wasm"
second_wasm="$scratch/second/target/wasm32-unknown-unknown/release/lexlean_echo_guest.wasm"
cmp "$first_wasm" "$second_wasm"
cmp "$scratch/first/generation-manifest.json" "$scratch/second/generation-manifest.json"

"$wasm_tools" validate "$first_wasm"
"$wasm_tools" print "$first_wasm" >"$scratch/guest.wat"
if rg -q '^  \(import ' "$scratch/guest.wat"; then
  echo "Core-Wasm guest unexpectedly imports a host function or memory" >&2
  exit 1
fi
rg -q '^  \(memory ' "$scratch/guest.wat"
rg -q '\(export "memory" \(memory 0\)\)' "$scratch/guest.wat"
rg -q '\(export "holo_alloc" \(func ' "$scratch/guest.wat"
rg -q '\(export "holo_run" \(func ' "$scratch/guest.wat"
rg -q '\(param i32\) \(result i32\)' "$scratch/guest.wat"
rg -q '\(param i32 i32\) \(result i64\)' "$scratch/guest.wat"

node "$repo_root/scripts/check-core-wasm.mjs" "$first_wasm"
"$wasmtime" run --invoke holo_alloc "$first_wasm" 0 >"$scratch/wasmtime.stdout" 2>"$scratch/wasmtime.stderr"
rg -q '^[0-9]+$' "$scratch/wasmtime.stdout"

if "$wasmtime" run --invoke holo_alloc "$first_wasm" 65537 >"$scratch/trap.stdout" 2>"$scratch/trap.stderr"; then
  echo "over-cap allocation unexpectedly succeeded under Wasmtime" >&2
  exit 1
fi
rg -qi 'trap|unreachable' "$scratch/trap.stderr"

echo "Core-Wasm generation, inspection, Node execution, and Wasmtime trap checks passed"
