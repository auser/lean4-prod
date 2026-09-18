#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd -P)
cargo build --manifest-path "$repo_root/rust/Cargo.toml" -p prod-codegen \
  --example workspace_view_fixture --locked --offline
node --test --test-concurrency=1 \
  "$repo_root/rust/prod-codegen/tests/fixtures/workspace_view_loader_test.mjs"
