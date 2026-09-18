# lean4-prod

Generate real, raw, performant Rust from actual Lean 4 — automatically, with
whole-language coverage inherited from Lean's own compiler.

## How it works

You tag definitions in Lean. Lean's compiler lowers **every** compilable
definition to LCNF (Lean Compiler Normal Form) — a tiny typed IR with seven
constructors. `prod-export` walks that LCNF and emits a compact s-expression
IR. Pure `no_std` Rust libraries parse the IR and generate zero-cost Rust,
consumed through a proc-macro, a CLI, or a WebAssembly playground.

```
Lean 4 project (native — the Lean toolchain cannot run in wasm)
@[prod] def foo ...        theorem bar ...
        │  lake exe prod-export
        ▼
kernel.ir (sexp)   roots.json   coverage.md   goldens.ir
        │
════════ portable half: no_std + alloc, wasm32-clean ════════
        ▼
prod-ir ─────────────► prod-codegen          (pure Rust libraries)
(AST + nom parser     (IR → Rust source,
 + sexp writer)        single source of truth)
    │          │            │
    ▼          ▼            ▼
prod-macros  prod-cli    prod-wasm
(thin host   (native:    (wasm-bindgen shell:
 proc-macro   parse/gen/  browser playground)
 wrapper)     roots)
```

### Why LCNF (whole-language coverage without rebuilding anything)

We do **not** parse or lower Lean's surface syntax or elaborated `Expr`
ourselves. Lean's own compiler performs the hard lowering — typeclass
dictionaries become explicit arguments, recursors and `match` become `cases`,
higher-order functions become closures and join points, proofs and `Prop`s
are erased. We lower the result — seven `Code` cases — so coverage of the
compilable fragment of Lean comes from Lean itself and grows as Lean grows.

The coverage report (`coverage.md`) classifies every constant in the module
using Lean's own `Lean.Compiler.LCNF.shouldGenerateCode`, so "covered" means
what Lean means.

### Generated-code contract

The generated code targets a production standard: **no panic on
caller-controlled input, and no heap allocation.** Concretely:

- **Memory profile — allocation-free.** `prod-core` has no `extern crate
  alloc`, so an allocating generated type could not compile. Lean `List α`
  therefore never becomes an owned list: in parameter position it is `&[α]`
  (matched with the slice patterns `[]` and `[h, t @ ..]`), and in return
  position the signature gains a caller-owned `output: &mut [α]` and returns
  the written prefix length. Zero-argument list goldens are promoted
  `&'static [α]`. A list value anywhere else is a codegen error, not a silently
  allocating fallback. `rust/prod-core/tests/no_alloc.rs` certifies this
  empirically with a counting global allocator (`just no-alloc`).
- **Error contract.** A definition returns `Result<T, ComputeError>` *only if
  it can fail* — if its body performs a checked `Nat` operation, calls a
  definition that does, or fills an output buffer. That is a fixpoint over the
  module's call graph, so leaf definitions and the goldens keep plain return
  types. `ComputeError` is a `Copy` C-like enum whose `Display` writes straight
  into the formatter, so even the error path allocates nothing.
- **Bounded `Nat`.** `Nat` maps to `u64`: addition, multiplication, shifts, and
  powers report overflow as an error (including shift/power exponents that do
  not fit `u32`); subtraction truncates at zero and division/modulo by zero
  return zero, matching Lean's total operations. Arbitrary-precision `Nat` is
  ruled out *by* the no-heap rule, not merely unimplemented — bounded `u64` is
  the deliberate policy.
- **Bounded recursion.** Generated recursion is structurally bounded by a fuel
  or data argument (Lean must already have proved termination for LCNF to emit
  it), so stack depth is a function of the caller's inputs.
- **Guardrails.** `unsafe_code = "forbid"` workspace-wide (except `prod-wasm`,
  where `#[wasm_bindgen]` expands to unsafe code, and the test-only
  `prod-alloc-counter`); `prod-core` additionally denies
  `clippy::{unwrap_used, expect_used, panic}`. `just test-assertions` reruns
  the suite optimized with `debug-assertions`/`overflow-checks` on.

### Honest limits

- Proofs/`Prop`s are erased by design — they are metadata (see `roots.json`),
  not code.
- `noncomputable` definitions, `@[extern]` FFI, and `unsafe`/`partial`
  runtime tricks are exported as opaque signatures and counted in
  `coverage.md`.
- We couple to Lean's internal LCNF API. The toolchain is pinned
  (`leanprover/lean4:v4.32.1`) and CI-gated against API drift.
- Structural recursion on `Nat` works (LCNF `cases` on `Nat.zero`/`Nat.succ`
  → Rust match with predecessor binding); `Option α` and `Bool` map to Rust
  `Option`/`bool`. Decidable `if` guards are rewritten for `<`, `≤`, and `=`
  on Nat (`Nat.decLt`/`decLe`/`decEq` and the `instDecidableEqNat` wrapper);
  other decidable guards would surface as extern calls in `coverage.md`.
- Lists only flow in the two supported directions described above. Building a
  list into an intermediate value, or nesting one inside another type, fails
  codegen rather than allocating.
- Automatic data-parallel codegen is not implemented, but the optional
  `prod-runtime` host crate provides a bounded deterministic driver for
  independent generated calls. Generated functions remain synchronous, pure,
  and allocation-free; async applications should invoke the driver from their
  runtime's blocking pool.
- The wasm package houses the portable half (parse + codegen + roots). The
  Lean extractor itself is native-only.

### Bounded parallel execution

Generated functions stay synchronous so the same code remains usable in
`no_std`, wasm, and FFI targets. Host applications can run independent calls
with the optional `prod-runtime` crate:

```rust
use prod_runtime::parallel_map;

let inputs = [/* caller-owned inputs */];
let mut outputs = [/* caller-owned output slots */];
parallel_map(&inputs, &mut outputs, 4, |input, output| {
    *output = generated_function(*input)?;
    Ok(())
})?;
```

The driver uses at most the requested number of workers, assigns disjoint
contiguous chunks, and joins in input order. A failed worker is returned after
all workers have joined. Async applications should call it from their
executor's blocking pool rather than pretending the CPU-bound function is
non-blocking.

## Quick start

```sh
nix develop              # pinned Lean 4.32.1 + Rust 1.97.1 development shell
just prod                # compiles proof fixtures, exports, then runs cargo tests
```

Lean side:

```lean
@[prod] def stride (inst : Instance) : Nat := inst.T * inst.O
```

```sh
cd lean && lake exe prod-export   # → rust/prod-core/{kernel,goldens}.ir, roots.json, coverage.md
```

External generated packages use the named-root mode; no `@[prod]` attribute,
wrapper module, or source adapter is required:

```sh
lake exe prod-export \
  --module SemanticFixture.Main \
  --root SemanticFixture.Main.allConsecutive \
  --ir-module SemanticFixture \
  --out ./export
```

Roots must be unique and ASCII-sorted. `Prod.exportNames` validates them,
computes the deterministic compilable closure, erases proof-only
dependencies, and rejects unsafe, partial, opaque/noncomputable,
non-code-generating, unresolved, and unsupported runtime content before a
successful export is returned. `just named-export` runs the exact
LexLean-1.1-generated conformance fixture and pins its IR, reports, and Rust.

Rust side:

```rust
prod_macros::prod_defs! { ir = "kernel.ir" }   // typed, zero-cost Rust fns
```

The Rust code-generation APIs, including Cargo and Core-Wasm packages, preserve
lexical parameter, let, match, and join-point scopes. Colliding local names are
normalized deterministically before ownership analysis; already hygienic names
retain their output bytes.
Positional parameters still refer to the original formal parameters under
shadowing. Duplicate names within one parameter or pattern-binding group are
rejected as `DuplicateBinding`, rather than choosing an ambiguous binding.

### C headers and foreign-function calls

The CLI can generate both sides of a small, explicit C ABI: a header for C
callers and Rust `extern "C"` wrappers that invoke the generated definitions.
The first ABI supports scalar `Nat` (`uint64_t`), fixed-width `Int64`
(`int64_t`), and
`Bool` (`uint8_t`, where zero is false). A checked Lean definition returns a
`*_result_t` with a `status` code and `value`; status zero means success.

```sh
just c-headers
```

That uses `rust/prod-core/goldens.ir` and writes
`output/lean4-prod.h` plus `output/lean4-prod_ffi.rs`. For an exported module
of your own:

```sh
just c-headers path/to/kernel.ir kernel
```

The command is a convenience wrapper around `prod header`; it always keeps
the generated artifacts together under `./output`.

For language SDKs over the same compiled native library, run:

```sh
just sdks
```

This writes a bundle under `output/lean4-prod/`:

```text
output/lean4-prod/
├── c/           # header and Rust extern-C adapter
├── rust/        # safe Rust wrapper crate source
├── python/      # ctypes module
├── typescript/  # loader-neutral TypeScript binding
├── kotlin/      # JNA interface and helpers
└── wasm/        # wasm-bindgen package: .wasm, .js, and .d.ts
```

Use another exported module with `just sdks path/to/kernel.ir kernel`.
All SDKs target the same scalar C ABI (`Nat`/`Int`/`Bool`) and the same status
codes, so the compiled Rust library remains the single implementation. The
TypeScript binding accepts a native function loader (for example `koffi` or
`ffi-napi`), Python uses `ctypes`, and Kotlin uses JNA.

Scalar wrappers independently normalize parameter names that collide with a
target language keyword, imported helper, temporary, or callee. One positional
mapping is shared by all six adapters; safe names retain their bytes, and
exported symbols, scalar types, argument order, and status handling are unchanged.
This parameter boundary does not rename global definitions or generated types.

To generate only one language, use a language-specific recipe:

```sh
just sdk-c
just sdk-rust
just sdk-python
just sdk-typescript
just sdk-kotlin
just sdk-wasm
```

These write only the selected language's files under `output/<stem>/`. The
generic form is `just sdk python path/to/kernel.ir kernel lean4_prod` (the
language, IR path, stem, and native library name are positional).
Use `just sdks` when you want the complete bundle.

Run the generated-language fixture checks with:

```sh
just sdk-fixtures
```

The fixture executes every generated SDK. C, Rust, and Python call the same
compiled generated native library; TypeScript and Kotlin execute their adapter
contracts against deterministic native-interface fakes; WebAssembly loads the
generated `.wasm` through its JavaScript glue in Node. Every fixture checks
success values, booleans, and generated error propagation. The Nix development
shell declares all required compilers and runtimes, so CI runs the complete
matrix rather than silently skipping a language.

### Pinned UOR Framework integration

The repository includes an external-package fixture for the UOR Framework
formalization requested as a production input. It pins upstream commit
`51c01382200b0179d6640b07e9c8119364ab69a1`, imports `UOR.Enums`, exports real
UOR computations through the reusable `Prod.exportModule` API, generates the
complete language SDK bundle, and executes its WebAssembly package in Node:

```sh
nix develop path:. --command just uor-fixture
```

Artifacts are generated under `output/uor/`: `kernel.ir`, proof-root and
coverage reports, C, Rust, Python, TypeScript, Kotlin, and WebAssembly SDKs.
The wasm package contains `uor.js`, `uor.d.ts`, and `uor_bg.wasm`. The
executable adapters currently expose
`WittLevel.bitsWidth (WittLevel.new n)` and UOR's primitive-operation
commutativity metadata. They deliberately use the scalar ABI shared by every
generated SDK.

This is an executable integration with the upstream formalization, not a
claim that its entire generated data model is supported. UOR also contains
polymorphic structures and arrays; those require explicit foreign-language
ownership and layout contracts before they can be exported safely. The
fixture therefore keeps that boundary visible instead of inventing an ABI.
Its own `lean-toolchain` selects Lean 4.32.1 so the external declarations pass
through the same compiler frontend as this generator; only the imported
production modules are built, avoiding unrelated upstream test assertions
pinned to UOR's older toolchain.

The WebAssembly SDK is emitted as a consumable wasm-bindgen package under
`output/<stem>/wasm/`. It contains a `.wasm` binary, JavaScript glue, and
TypeScript declarations; import the generated `.js` module and call its
default initializer before using the named exports. The generated package
uses the web target and does not require consumers to compile Rust or install
`wasm-bindgen`. Its exported functions use the same scalar `Nat`/`Int`/`Bool`
ABI as the other SDKs; fallible functions surface a JavaScript error.
Generation itself requires `wasm-pack`; it is included in the repository's
Nix development shell.

### WebAssembly browser demo

Build the fixture SDK and launch an interactive browser demo with:

```sh
nix develop path:. --command just wasm-demo
```

Then open <http://127.0.0.1:8000/demo/wasm/>. The page imports the generated
JavaScript wrapper and `.wasm` binary from `output/fixture/wasm/`, then runs
checked natural-number addition, a Lean comparison, and an intentional
overflow that demonstrates generated errors crossing the WebAssembly boundary.
Use `just wasm-demo 9000` to choose another port. `just wasm-demo-build` builds
and tests the same package without starting a server.

Serve the demo through `just wasm-demo` instead of opening `index.html`
directly: browsers load WebAssembly modules through HTTP and require the
generated binary to use the `application/wasm` content type. Demo source is in
`demo/wasm/`; generated package files stay under the gitignored `output/`
directory and can always be regenerated. `just wasm-sdk-fixture` runs the same
package and demo behavior checks used by CI.

Include the generated wrapper after the proc-macro expansion in the crate
that owns the generated definitions:

```rust
prod_macros::prod_defs! { ir = "kernel.ir" }
include!("../../output/kernel_ffi.rs");
```

Definitions with lists, generated structs or enums, options, tuples, or other
composite values are omitted and named in a comment in the header instead of
getting an invented ABI. If an IR module has no scalar definitions, the
command fails. Those composite values need an explicit buffer/ownership and
layout contract before they can safely cross a C ABI. The header and wrapper
are generated artifacts; do not hand-edit either file.

## Closed byte literals

Typed decimal parsing retains the exact integer result type from LCNF in
`parse-decimal-as`, including when an Option payload is discarded or only
compared with a literal. Legacy `parse-decimal` IR remains accepted. The real
LexLean-generated decimal fixture checks fixed-width bounds and canonical
syntax; mathematical `Int` remains rejected by the production renderer.

UTF-8 encoding preserves the existing ownership boundary: borrowed String
parameters and fields are copied into owned bytes, while owned Strings reuse
their buffer. The ordinary Core-Wasm fixture suite executes borrowed, aliased,
repeated and record-field inputs in native `std`, `no_std + alloc`, and actual
Wasm, including Unicode, embedded NUL and allocation-bound cases.

The portable `Bytes` ABI also accepts closed Lean `ByteArray` literals, including
empty data and non-UTF-8 bytes. The lowerer folds only the typed
`Array UInt8` literal construction chain consumed by `ByteArray.mk` into a
`(bytes 0 128 255)` IR leaf. Arbitrary runtime Arrays and dynamic byte-array
construction remain rejected; this does not widen the Array language subset.
Owned literal results use the existing `Vec<u8>` ABI and materialize once at
that boundary. Borrowed literal comparisons and calls use static byte slices.
`just portable-package` executes literal returns, nested branches, append and
decode-then-reuse in both `std` and `no_std + alloc`, alongside rejection probes.

Composition through byte-length wrappers accepts the typed `ByteArray.size`
builtin. Specialized byte append is recognized only from Lean's complete
retained mono-LCNF body: copy all of the right input to the end of the left
input, with no other computation or control flow. This is not general
`ByteArray.copySlice` support. Altered copy operands, additional work, and
unknown callable runtime helpers fail closed; runtime namespace membership
does not authorize replacing semantic results with erased dictionary values.

## Closed text Views

`prod_codegen::generate_text_view_v1(&TextViewV1, &TextBrowserAdapterBinding)`
projects `prism.text-view/1` into a browser wasm-bindgen adapter and a Hologram
intent View. The original `generate_view_v1` numeric projection retains its
modeled arithmetic and byte protocol; its generated assets also enforce the
initialization privacy contract described below.
The seven modeled strings are title, heading, input label, submit label, output
label, input error and response error; none accepts markup, callbacks or URLs.
Positive `u32` input/output caps count UTF-8 bytes, not characters. Empty request
and response values are permitted. Metadata also binds the model, View model
and generated core SHA-256 identities.

The browser adapter exports `invoke_bytes`, forwards owned bytes to the named
generated `Vec<u8> -> Vec<u8>` core function, and validates UTF-8 and copy limits.
It uses exact js-sys 0.3.99 and wasm-bindgen 0.2.122 dependencies. It does not
parse application documents or implement application semantics. The core's own
execution and allocation limits remain obligations of its authoritative model:
checking a returned byte length cannot prevent an allocation already made by
that core. Likewise, browser/network internals may allocate before delivering
their bounded inputs to the adapter.

Hologram uses the existing `application.invoke` intent envelope and displays its
single text output without domain parsing. Its transport JSON is streamed and
bounded to `6 * max_output_bytes + 256` bytes before parsing, allowing JSON's
worst-case string escaping plus bounded envelope overhead. Browser input rejects
unpaired UTF-16 before encoding; both transports reject malformed UTF-8 rather
than replacing it. A leading U+FEFF remains application data. Output uses
`textContent` and a labeled polite live region. Invalid input receives focus;
Ctrl/Command+Enter submits a multiline request without consuming ordinary Enter
or IME composition. Native form controls retain their normal keyboard behavior.
Native form navigation is never an application transport: both projections emit
an early CSP `form-action 'none'` policy, omit a successful named draft field,
and keep submit disabled until the cancellation handler is installed. The
modeled response-error text is present without JavaScript and is cleared only
after transport initialization succeeds, without erasing newer input errors.
Browser binding modules load through a caught dynamic import; failed module or
Wasm initialization cannot fall back to sending draft content in a URL.

The closed `lean4-prod/text-view-projection/1` manifest hashes both three-file
projections, the exact HOLOVIEW v1 bundle and the complete browser adapter file
set. `lean4-prod/text-browser-adapter/1` separately binds its core dependency,
entrypoint, caps, identities and generated files. Neither changes Holo/1.

`just text-view` (included in `just ci`) checks deterministic generation and exact
manifest/bundle closure, executes generated JavaScript against controlled DOM
and transport ports, then compiles and executes real generated Wasm probes for
echo, invalid UTF-8 output and output overflow. The DOM harness tests focus,
keyboard, concurrency and failures; it is not a browser layout test. The normal
gate also runs actual Chromium over the exact generated assets and compiled
Wasm, checking disabled JavaScript, blocked application/binding modules, native
submission blocked independently of button state, delayed initialization,
keyboard recovery, invalid responses and absence of draft network/navigation
leaks. The compiler devcontainer owns locked Playwright 1.62.1 and its Chromium
revision; missing browser tools fail the gate. These synthetic IR fixtures test
compiler transport behavior, not application proofs.

`just view` also runs eight real Chromium cases over the generated numeric
View and its compiled fixture Wasm. Both numeric projections prohibit native
form navigation with their own early CSP, omit named operand/operation fields,
and enable submit only after cancellation is installed. The modeled input-error
text is the no-JavaScript/initialization-failure fallback; a caught dynamic
binding import lets pending submissions wait locally for Wasm without losing
input. These security changes intentionally change numeric View asset bytes,
not arithmetic or the generated core.

## Roots (proof-graph analysis)

Every theorem is a root. `Roots.lean` exports each root's dependency edges,
proof-term size, kernel depth, and kernel re-check time to `roots.json`. The
CLI computes what the registry claims:

```sh
prod roots check     # DAG acyclicity + coverage (actually computed)
prod roots pareto    # Pareto front over (proof size, kernel depth, check time)
prod roots connect   # hypothesized bridges between roots sharing kernel deps
```

The roots commands operate on the generated registry directly:

```sh
cd rust
cargo run -p prod-cli -- roots check ../roots.json
cargo run -p prod-cli -- roots pareto ../roots.json
cargo run -p prod-cli -- roots connect ../roots.json
```

The Pareto analysis uses three objectives: proof-term size, kernel depth, and
`check_time_ns` — the wall time for re-typechecking the proof term with Lean's
kernel (`Lean.Kernel.check`), taken as the minimum of 16 repetitions by the
exporter to suppress µs-scale noise. The times are machine-dependent and only
meaningful as a relative signal within one export run. Compact theorem IDs may
repeat when Lean generates private helper theorems, and `roots check` reports
those as warnings while using the dependency names to resolve graph edges.

## Layout

- `lean/Prod/` — the extractor: attribute, LCNF extraction, lowering, roots,
  coverage, emit. Generic; not tied to the example.
- `lean/ProofFixtures.lean` — standalone kernel-checked theorem fixtures;
  `just lean-fixtures` compiles them without adding them to production export.
- `lean/Example/` — worked example: the UOR Atlas coordinate kernel with
  machine-checked proofs (no mathlib — `decide`/`omega`/`rfl` discipline).
- `rust/prod-ir`, `rust/prod-codegen` — `no_std` + `alloc` portable core.
- `rust/prod-macros`, `rust/prod-cli` — thin native shells.
- `rust/prod-wasm` — wasm-bindgen API: `generate(ir)` and `roots_pareto(json)`.
- `rust/prod-core` — runtime types + generated definitions + golden tests.

## Roadmap

- TF1-style claim registry: claims typed by the proof that discharges them
  (build / open / definition), executable checks, mutation testing for
  non-vacuity.
- LCNF impure-phase fidelity (unboxed scalars, refcount elision notes) for
  tighter codegen.
