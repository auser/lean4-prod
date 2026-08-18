# Default
default:
    @just --list

# Full pipeline: export from Lean, then verify the Rust build against it.
prod: lean-fixtures prod-export conformance test test-assertions no-alloc roots-check subset-check

# Compile the standalone Lean proof-fixture library. These declarations are
# real kernel-checked proofs, but are not part of the production export target.
lean-fixtures:
    cd lean && lake build ProofFixtures

# Generate a C header and matching Rust FFI wrappers into ./output. Override
# `ir` for a project's exported module and `stem` for the artifact basename.
c-headers ir="rust/prod-core/goldens.ir" stem="lean4-prod":
    mkdir -p output
    RUSTC_WRAPPER= cargo run --manifest-path rust/Cargo.toml -p prod-cli -- header {{ir}} --output output/{{stem}}.h --rust-output output/{{stem}}_ffi.rs

# Generate the complete scalar SDK bundle under ./output/<stem>.
sdks ir="rust/prod-core/goldens.ir" stem="lean4-prod" library_name="lean4_prod":
    mkdir -p output
    RUSTC_WRAPPER= cargo run --manifest-path rust/Cargo.toml -p prod-cli -- sdks {{ir}} --output output --stem {{stem}} --library-name {{library_name}}

# Generate one SDK language under ./output/<stem>. The language-specific
# aliases below are convenient entry points; use this recipe for scripts.
sdk language="rust" ir="rust/prod-core/goldens.ir" stem="lean4-prod" library_name="lean4_prod":
    mkdir -p output
    RUSTC_WRAPPER= cargo run --manifest-path rust/Cargo.toml -p prod-cli -- sdk {{ir}} --language {{language}} --output output --stem {{stem}} --library-name {{library_name}}

sdk-c ir="rust/prod-core/goldens.ir" stem="lean4-prod" library_name="lean4_prod":
    just sdk c {{ir}} {{stem}} {{library_name}}

sdk-rust ir="rust/prod-core/goldens.ir" stem="lean4-prod" library_name="lean4_prod":
    just sdk rust {{ir}} {{stem}} {{library_name}}

sdk-python ir="rust/prod-core/goldens.ir" stem="lean4-prod" library_name="lean4_prod":
    just sdk python {{ir}} {{stem}} {{library_name}}

sdk-typescript ir="rust/prod-core/goldens.ir" stem="lean4-prod" library_name="lean4_prod":
    just sdk typescript {{ir}} {{stem}} {{library_name}}

sdk-kotlin ir="rust/prod-core/goldens.ir" stem="lean4-prod" library_name="lean4_prod":
    just sdk kotlin {{ir}} {{stem}} {{library_name}}

sdk-wasm ir="rust/prod-core/goldens.ir" stem="lean4-prod" library_name="lean4_prod":
    just sdk wasm {{ir}} {{stem}} {{library_name}}

# Build a representative wasm-bindgen package and assert that the public
# artifacts are package files, not the intermediate generated Rust source.
wasm-sdk-fixture:
    RUSTC_WRAPPER= cargo run --manifest-path rust/Cargo.toml -p prod-cli -- sdk rust/prod-codegen/tests/fixtures/sdk_scalar.ir --language wasm --output output --stem fixture --library-name fixture
    test -f output/fixture/wasm/fixture_bg.wasm
    test -f output/fixture/wasm/fixture.js
    test -f output/fixture/wasm/fixture.d.ts
    test ! -e output/fixture/wasm/lib.rs
    node rust/prod-codegen/tests/fixtures/wasm_sdk_test.mjs output/fixture/wasm
    node demo/wasm/demo_test.mjs output/fixture/wasm

# Build the generated package consumed by the browser demo.
wasm-demo-build:
    RUSTC_WRAPPER= cargo run --manifest-path rust/Cargo.toml -p prod-cli -- sdk rust/prod-codegen/tests/fixtures/sdk_scalar.ir --language wasm --output output --stem fixture --library-name fixture
    node demo/wasm/demo_test.mjs output/fixture/wasm

# Build and serve the browser demo from the repository root.
wasm-demo port="8000": wasm-demo-build
    @echo "Open http://127.0.0.1:{{port}}/demo/wasm/"
    python3 -m http.server {{port}} --bind 127.0.0.1

# Execute one representative generated fixture for every SDK language. The Nix
# shell supplies every compiler/runtime; host runs skip tools not installed.
sdk-fixtures: wasm-sdk-fixture
    RUSTC_WRAPPER= cargo test --manifest-path rust/Cargo.toml -p prod-codegen --test sdk_fixtures -- --nocapture

# Export prod from lean
prod-export:
    cd lean && lake exe prod-export

# The conformance golden pins Lean-side lowering. `prod-export` rewrites it; this
# fails if the rewrite changed anything, so lowering changes surface as a diff.
conformance:
    cd lean && lake exe prod-export
    git diff --exit-code lean/Conformance/golden.ir

# Accept the current lowering as the new golden. Review the diff before running.
conformance-bless:
    cd lean && lake exe prod-export
    git add lean/Conformance/golden.ir

# Build rust debug
build:
    cd rust && cargo build

# Build rust production
build-prod:
    cd rust && cargo build --release

# Test rust workspace
test:
    cd rust && cargo test --workspace

# Same tests, optimized, with debug/overflow assertions left on. A release
# build that silently wraps where the debug build panics is a bug we want to
# hear about before shipping.
test-assertions:
    cd rust && cargo test --workspace --profile release-assertions

# Certify that the generated code performs zero heap activity. Serial: the
# counting allocator is process-global.
no-alloc:
    cd rust && cargo test -p prod-core --test no_alloc -- --test-threads=1

# Validate the generated theorem dependency graph.
roots-check:
    cd rust && cargo run -p prod-cli -- roots check ../roots.json

# The published subset contract is generated from the implementation, so it
# cannot describe a fragment the code does not implement.
subset:
    cd rust && cargo run -p prod-cli -- subset ../subset.json --output ../specs/lean-for-production.md

subset-check: subset
    git diff --exit-code specs/lean-for-production.md

# Link rust code
lint:
    cd rust && cargo clippy --all-targets -- -D warnings

# Formatting is gated, not merely done once — otherwise it drifts immediately.
fmt-check:
    cd rust && cargo fmt --all -- --check

# Apply formatting.
fmt:
    cd rust && cargo fmt --all

# Portable half must stay no_std/wasm32-clean.
wasm-check:
    cd rust && RUSTC_WRAPPER= RUSTC=$(rustup which --toolchain stable rustc) rustup run stable cargo build -p prod-ir -p prod-codegen -p prod-wasm --target wasm32-unknown-unknown
