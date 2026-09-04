#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd -P)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/lean4-prod-view.XXXXXXXX")
trap 'rm -rf -- "$scratch"' EXIT

first="$scratch/first"
second="$scratch/second"
core="$scratch/core"
fixture="$repo_root/rust/prod-codegen/tests/fixtures"

cd "$repo_root"
cargo run --manifest-path rust/Cargo.toml -p prod-cli -- cargo \
  "$fixture/view_core.ir" --output "$core" --name fixture-core --version 0.1.0 \
  --description "Generated View adapter fixture core" \
  --repository https://example.invalid/fixture-core \
  --homepage https://example.invalid/fixture-core/ \
  --readme README.md --license-mit "$fixture/LICENSE-MIT" \
  --license-apache /usr/share/common-licenses/Apache-2.0
cargo run --manifest-path rust/Cargo.toml -p prod-cli -- view \
  "$fixture/view_v1.json" --output "$first"
cargo run --manifest-path rust/Cargo.toml -p prod-cli -- view \
  "$fixture/view_v1.json" --output "$second"
diff -ru "$first" "$second"

test "$(find "$first/hologram" -maxdepth 1 -type f -printf '%f\n' | sort | tr '\n' ' ')" = \
  "app.css app.js index.html view.holoview "
test "$(find "$first/browser" -maxdepth 1 -type f -printf '%f\n' | sort | tr '\n' ' ')" = \
  "app.css app.js index.html "
head -c 10 "$first/hologram/view.holoview" | od -An -tx1 | tr -d ' \n' | \
  grep -Fx '484f4c4f564945570001'
grep -F "name:'application.invoke'" "$first/hologram/app.js" >/dev/null
grep -F "const MIN=-9223372036854775808n" "$first/browser/app.js" >/dev/null
grep -F 'fixture-core = "=0.1.0"' "$first/browser-adapter/Cargo.toml" >/dev/null
! grep -E '(path|git)[[:space:]]*=' "$first/browser-adapter/Cargo.toml"

adapter="$first/browser-adapter"
patch="patch.crates-io.fixture-core.path='$core'"
# A clean devcontainer has not resolved the generated adapter's registry
# dependencies yet. Resolve and fetch the exact lock once, then require every
# compilation step below to succeed offline from that locked dependency set.
cargo generate-lockfile --manifest-path "$adapter/Cargo.toml" --config "$patch"
cargo fetch --manifest-path "$adapter/Cargo.toml" --target wasm32-unknown-unknown \
  --locked --config "$patch"
cargo check --manifest-path "$adapter/Cargo.toml" --target wasm32-unknown-unknown \
  --locked --offline --config "$patch"
cargo build --manifest-path "$adapter/Cargo.toml" --target wasm32-unknown-unknown \
  --release --locked --offline --config "$patch"
wasm="$adapter/target/wasm32-unknown-unknown/release/fixture_browser_adapter.wasm"
"$CARGO_HOME/bin/wasm-tools" validate "$wasm"
"$CARGO_HOME/bin/wasm-bindgen" --target web --out-dir "$scratch/web" --out-name fixture "$wasm"
test -f "$scratch/web/fixture.js"
test -f "$scratch/web/fixture_bg.wasm"
grep -F 'export function calculate' "$scratch/web/fixture.js" >/dev/null

echo "Foundation.View.V1 generation, reproducibility, adapter, and Wasm checks passed"
