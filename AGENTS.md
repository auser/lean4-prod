# lean4-prod — agent handoff / project guide

You are working in `/Users/auser/work/rust/mine/lean4-prod`: a Lean 4 → production
Rust generator. Tag `@[prod]` definitions in Lean; a metaprogram extracts them via
Lean's OWN compiler frontend (LCNF — Lean Compiler Normal Form), emits a sexp IR;
pure `no_std` Rust libraries parse it and generate zero-cost Rust (proc-macro, CLI,
wasm shells). Theorem "roots" metadata (deps, proof-term size, kernel depth,
kernel re-check time) feeds Pareto/connection analysis in the CLI. Full
architecture: see README.md.

## Goal and engineering standard

The goal is to generate production-ready, parallelized, secure code. We
_always_ prefer NOT allocating any heap memory in the generated code. Follow
the Rust best-practices standard (normative MUST/SHOULD guide: memory
profiles, no panic on recoverable conditions, checked arithmetic, lint/CI
discipline):
https://gist.github.com/auser/c3161f55a8393faa8af5ddda68c6befa

The generated code now complies. The alignment work is recorded in
`specs/plans/2026-08-06-best-practices-alignment.md` (implemented 2026-08-08).
What that means in practice, and what you must not regress:

- **No heap in generated code.** `prod-core` has no `extern crate alloc`.
  Lean `List α` is `&[α]` as a parameter and a caller-owned `output: &mut [α]`
  buffer as a return; zero-arg list goldens are promoted `&'static [α]`. Any
  other list position is a deliberate codegen error (`Error::UnsupportedList`),
  and `Type::Vec` is rejected (`Error::HeapType`). Never "fix" one of those by
  reintroducing an owned list type.
- **No panic on caller-controlled input.** Checked `Nat` ops render as
  `.ok_or(crate::ComputeError::X)?`, never `.expect(..)`. `ComputeError` lives
  in `prod-core/src/error.rs` and is a `Copy` payload-free enum so the error
  path allocates nothing either.
- **Fallibility is a fixpoint, not a blanket.** A def gets
  `Result<_, ComputeError>` only if it can actually fail; see `Shape` in
  `prod-codegen`. Do not make it uniform — the goldens must stay infallible.
- **Guardrails.** `unsafe_code = "forbid"` workspace-wide via
  `[workspace.lints.rust]`. Two crates opt out on purpose and say so in their
  manifests: `prod-wasm` (`#[wasm_bindgen]` expands to unsafe) and the
  test-only `prod-alloc-counter` (holds the one `unsafe impl GlobalAlloc`, so
  that `prod-core` can forbid unsafe in *all* its targets). `prod-core` also
  denies `clippy::{unwrap_used, expect_used, panic}`.
- **Parallelism principles.** Generated fns are pure and `Send`/`Sync` by
  construction. The host-only `prod-runtime` crate now provides bounded
  `parallel_map` execution over disjoint caller-owned slices, with deterministic
  chunk-order joins and panic containment. Automatic proof-guided parallel
  codegen is still future work: if it is added, use bounded workers and a
  deterministic merge order, no unbounded queues, and canonical output bytes
  must never depend on thread scheduling.

## Rules (hard)

- NO mathlib. Pure Lean 4 core/Init; `decide`/`omega`/`rfl` discipline.
- NO `sorry`, NO `axiom` in anything claimed as proved.
- Nothing hand-written downstream of Lean: kernel.ir/goldens.ir/roots.json/
  coverage.md/subset.json are generated artifacts (gitignored). Never edit
  them by hand. Two generated artifacts ARE committed, precisely so a
  drifted regeneration shows up as a reviewable diff instead of silently not
  existing: `lean/Conformance/golden.ir` (pinned by `just conformance`,
  rewritten and accepted with `just conformance-bless` — review the diff
  first) and `specs/lean-for-production.md` (pinned by `just subset-check`,
  part of `just prod`; there is no bless step, just rerun `just subset` and
  review+commit the diff). Never hand-edit either.
- The old repo at `~/work/rust/mine/lean-four-prod/` is READ-ONLY reference.
- No `git add`/`git commit`/other git mutations without the user's explicit go-ahead.
- Verify gates below must actually pass before claiming a milestone done.

## Toolchain quirks (IMPORTANT)

- lean/lake are NOT on PATH. Always run via the nix dev shell.
- Flakes only see git-tracked files and this repo has untracked files, so ALWAYS
  use the `path:` prefix:
  - Build Lean: `cd lean && nix develop path:/Users/auser/work/rust/mine/lean4-prod --command lake build`
  - Run exporter: `... --command lake exe prod-export`
- If `nix` is "not found": `/nix/var/nix/profiles/default/bin/nix`.
- cargo/rustc ARE on PATH (Homebrew). wasm32 target: use
  `RUSTC=$(rustup which --toolchain stable rustc) rustup run stable cargo build ... --target wasm32-unknown-unknown`
  (Homebrew rustc can't see rustup targets).
- Lean builds take minutes; use generous timeouts (300s+).

## Status: M0–M6 DONE for the current example pipeline

- M0 skeleton, M1 Rust core (all green: 24 tests, clippy clean, wasm32 check),
  M2 Lean Example (lake build exit 0, no mathlib), M3 LCNF extractor,
  M4 exporter-generated goldens, M5 computed roots analysis, and M6 wasm/CI.
- roots.json root ids are FULL Lean names (unique by kernel construction);
  each root carries an `"auto"` flag marking Lean-generated machinery
  (equation lemmas `*.eq_<n>`, `*_proof_*` certificates, `sizeOf_spec`/`inj`/
  `injEq` lemmas, structure projections like `UorAtlas.Instance.valid`).
  `roots check|pareto|connect` skip `auto: true` roots by default (`--all`
  includes them); a missing `auto` field (older files) parses as `false`.
- Each root carries `check_time_ns`: wall time for re-typechecking its proof
  term with `Lean.Kernel.check` (empty local context), taken as the MINIMUM
  over 16 exporter-side repetitions — single-shot µs timings flip root
  orderings between runs, the min is reproducible. Machine-dependent; a
  relative signal only. `roots pareto` is a three-objective front over
  (proof_term_size, kernel_depth, check_time_ns); a missing `check_time_ns`
  parses as `0`, which reduces dominance to the old two-objective rule when
  uniformly absent.
- The Pareto front of hand-written roots is currently just `digits_lt_stride`:
  it dominates the other two real theorems on all three objectives — smaller
  (776 nodes), shallower (depth 4), and genuinely cheaper to re-check
  (~110µs vs ~130µs/~400µs, stable across runs). A non-trivial front needs a
  theorem that beats it on at least one objective.
- RECURSION works end-to-end (first recursive `@[prod]` def: `UorAtlas.digitCount`,
  fuel-bounded digit count, in `lean/Example/Kernel.lean`). Three latent bugs
  had to die for it:
  1. `Prod.depthOf` (Roots.lean) assumed a DAG and stack-overflowed on the
     self-cycle of a recursive def — now DFS with a `visiting` set; back edges
     contribute 0 (longest chain over the cycle-condensed graph).
  2. `if a < b then … else …` compiles to `Nat.decLt` + `cases` over
     `Decidable.isFalse/isTrue` — unrenderable in Rust. `decidableIf?` in
     Lower.lean rewrites the immediately-bound shape to IR `(if (lt a b) t f)`.
  3. prod-codegen rendered `Nat.zero`/`Nat.succ` alts as Rust enum patterns
     (invalid on u64) and `checked_add/mul`/`saturating_sub` failed method
     resolution (E0689) on let-bound-literal receivers — now `0` / `_` + pred
     via `saturating_sub(1)`, and receivers pinned with `as u64` (same trick
     `Pow` already used).
  digitCount has goldens (2, 3, and the fuel-0 short-circuit) asserted in
  `rust/prod-core/tests/macro_generation.rs`.
- DECIDABLE GUARDS + OPTION work end-to-end (`UorAtlas.sameClass` uses `=`,
  `UorAtlas.smallEnough` uses `≤`, `UorAtlas.tryClassDecode : … → Option (Nat
  × Nat × Nat)`). `decidableIf?` recognizes `Nat.decLt`/`Nat.decLe`/
  `Nat.decEq`/`instDecidableEqNat` (the eq instance wrapper is NOT unfolded by
  the simplifier — match both) → IR `(if (lt|le|eq a b) t f)`. prod-ir gained
  a `le` expr — NOTE: the parser must delimiter-terminate `tag("le")`, bare
  tag prefix-matches `let` and kills the enclosing alt (regression test:
  `test_parse_le_does_not_eat_let`). Codegen maps `Bool.true`/`Bool.false`
  and `Option.none`/`Option.some` ctors+match arms to `true`/`false`/
  `None`/`Some(v)`; `lowerType` handles `Option α`.
- LISTS work end-to-end too (`UorAtlas.digits : … → List Nat`,
  `UorAtlas.digitSum : List Nat → Nat` in Kernel.lean): prod-ir gained a
  `(List type)` form (`Type::List`) and the Lean lowerer maps `List α` to it.
  Since the best-practices alignment, codegen renders it WITHOUT a heap —
  `&[α]` in parameter position, a caller-owned `output: &mut [α]` buffer in
  return position, `&'static [α]` for zero-arg goldens (see below). List
  goldens (`digits 10 43 canonical = [3,5]`, `digitSum = 8`) roundtrip in
  macro_generation.rs.
- BEST-PRACTICES ALIGNMENT done (2026-08-08, plan
  `specs/plans/2026-08-06-best-practices-alignment.md`). Generated code no
  longer panics or allocates; see "Goal and engineering standard" above for
  the invariants. Load-bearing details worth knowing before touching
  `prod-codegen`:
  - `Shape` (Value / Fallible / Buffer / StaticList) is computed per module as
    a least fixpoint over the call graph; call sites append `?` based on the
    callee's shape, so `generate_def` alone cannot see cross-def fallibility
    (it analyzes a one-def module).
  - Builder mode threads an `out` slice expression and a scoped environment of
    `let`-bound list values. That environment is NOT optional: LCNF emits
    lists in A-normal form, so `digits`'s cons cells arrive as chains of
    `let`s, and materializing them would need the heap.
  - Buffer exhaustion uses `split_first_mut`, not indexing — the generated
    code has no bounds-check panic path at all.
  - `Ok::<usize, crate::ComputeError>(0)` for `List.nil` is turbofished on
    purpose: it is the one builder leaf that constrains neither type parameter
    and it can sit under a `?`.
  - `prod-ir`'s `parse_i64` parses the magnitude as `i128` before narrowing;
    the old `digits.parse::<i64>().unwrap()` panicked on `i64::MIN`.

- S0/S1 (coverage roadmap, `specs/designs/2026-08-08-lean-for-production-coverage.md`)
  DONE: the honest boundary and generated types. What changed since M0–M6
  above:
  - `UorAtlas.Instance` is no longer a special-cased IR type; it is an
    ordinary generated type like `Conformance.MidProp`/`NoProp`. `coordinate.rs`
    (the old hand-written `Instance` struct) is deleted; the struct comes
    entirely from `(type ...)` declarations in `kernel.ir`.
  - Structure projections carry the field name (`(proj "Full.TypeName"
    "fieldName" x)`), not a bare index — `Lower.lean` resolves the LCNF
    projection index to the declared field name once, where `getStructureFields`
    is available, so codegen never keeps a second, potentially-disagreeing
    index table.
  - Unresolved calls (`Error::UnresolvedCall`) and opaque types
    (`Error::OpaqueType`) are hard codegen errors now, not silently rendered
    as best-effort calls/opaque markers. A callee that is neither
    `@[prod]`-tagged nor a whitelisted operator, or a type codegen cannot
    describe, fails the build instead of shipping something unreviewed.
  - `Nat.shiftRight` lowers to a real, total/infallible `shr` IR node (not an
    expansion to div/pow); see `Lower.lean`'s module doc comment for why it
    never overflows.
  - `Prod.declTypeNames` collects types from a definition's **body** (ctor
    applications and projections) as well as its signature, so a type used
    only inside a body still gets a `(type ...)` declaration. Pinned by
    `Conformance.c_ctor_body_only`. Codegen independently refuses to render an
    undeclared *dotted* constructor name as a Rust path — `A.B.mk(x)` is
    valid `syn` (field access then call) and invalid Rust, which is exactly
    how it used to escape.
  - `prod_ir::Expr::children()` is the single traversal for every consumer
    (codegen's fallibility fixpoint and jp analysis, `prod-cli`'s extern
    collection). Its match is exhaustive with no wildcard: a new `Expr`
    variant is a compile error, not a silently-unvisited subtree. Do not
    hand-copy it again — the `prod-cli` copy had already drifted past `Shr`.
  - `Expr::Field` is deleted. `Lower.lean` never emitted it, it rendered
    identically to `Proj` while bypassing `Proj`'s `UnknownField` check, and
    its only remaining users were fixtures. Use `(proj "Type" "field" e)`.
  - The published subset contract (`specs/lean-for-production.md`, generated
    by `just subset` from `subset.json` + `prod_codegen::REJECTIONS`) and the
    conformance golden (`lean/Conformance/golden.ir`) are the project's two
    committed generated artifacts — see the "Rules (hard)" section above for
    their bless/regenerate workflows. The operator whitelist
    (`Prod.natOpNames`) and decider list (`Prod.deciderNames`) in `Lower.lean`
    are each a single association list consumed by both the lowerer
    (`opWhitelist`/`deciderOp`) and the exporter (`subsetJson`), so the
    contract cannot list an operator/decider the lowerer does not actually
    accept, or omit one it does.
  - One documented, deliberate gap: `Prop` fields (e.g. `Instance.valid : q
    ≥ 1 ∧ T ≥ 1 ∧ O ≥ 1`) are erased on export, so the generated Rust struct
    does not enforce the invariant its Lean source states — see the "Erased
    invariants" note in `specs/lean-for-production.md`.

Known remaining limitations: typed Lean `Int` semantics is NOT implemented
(generated Nat is u64 with the bounded policy: checked add/mul/shl/pow,
saturating sub, total div/mod-by-zero). Arbitrary-precision Nat is ruled OUT by
the no-heap directive, not merely unimplemented. Saturated, non-escaping,
acyclic first-order local functions with runtime-valued arguments lower to expression-valued continuation
IR; bounded expansion preserves captures and non-tail caller continuations.
General runtime closure values remain unsupported. User-defined inductives now generate real Rust structs/enums,
and `ctor`/`proj` on them resolve against the module's own `(type ...)`
declarations — a CONSTRUCTION whose constructor has no declaration in the
module is rejected (`UnresolvedCall`) rather than rendered as a dotted Lean
name pretending to be a Rust path. That check does NOT yet cover `cases`
PATTERNS: an alt naming an undeclared constructor still renders
`Foo.Bar.left(v) => v`, which rustc rejects as "expected a pattern, found an
expression". Same defect class, same fix shape; not done. Monomorphization is still absent, so a
parameterised inductive is rejected (`PolymorphicType`) rather than lowered.
Automatic data-parallel codegen is not implemented. The host-only
`prod-runtime` crate can run independent generated functions in bounded,
deterministic chunks; it does not make arbitrary closures safe or infer
independence.

## M3 spec — the LCNF extractor (the defensible core)

Create under `lean/Prod/`: `Attribute.lean` (`@[prod]` via `Lean.registerTagAttribute`;
enumerate tagged names by folding the attribute extension state), `Extract.lean`,
`Lower.lean`, `Roots.lean`, `Coverage.lean`, `Emit.lean` (replaces the stub main),
plus root `lean/Prod.lean` importing the Prod submodules (NOT Example — keep ProdLib
example-agnostic; `Prod.Emit` imports Example). Tag the five kernel defs
(`UorAtlas.stride/class_count/belt/classIndex/classDecode`) with `@[prod]`, replacing
their `-- M3:` comments.

Verified Lean 4.32.1 API facts (from leanprover/lean4 v4.32.1 sources — trust these):

- `Lean.Compiler.LCNF.toDecl : Name → CompilerM (Decl .pure)` — runs toLCNF:
  matches/recursors already `cases`, dictionaries explicit, instance wrappers unfolded.
- `Lean.Compiler.LCNF.CompilerM.run (x : CompilerM α) (s : State := {}) (phase : Phase := .base) : CoreM α`.
- Pure-phase shapes (`Lean/Compiler/LCNF/Basic.lean`):
  `Code := let (LetDecl) (Code) | fun (FunDecl) (Code) | jp (FunDecl) (Code) | jmp FVarId (Array Arg) | cases Cases | return FVarId | unreach Expr`
  `LetValue := lit LitValue | erased | proj Name Nat FVarId | const Name (List Level) (Array Arg) | fvar FVarId (Array Arg)`
  `Arg := erased | fvar FVarId | type Expr` — drop `erased`/`type` args, count them.
  `Alt := alt (ctorName) (params) (code) | default (code)`; `Cases := mk typeName resultType discr alts`.
  `Decl extends Signature` with `value : DeclValue .pure` — `#check DeclValue` and handle all constructors.
- `Lean.Compiler.LCNF.shouldGenerateCode (n : Name) : CoreM Bool` — coverage criterion.
- Module's own constants: iterate `env.constants.map₁` (imports live in map₂).
- Theorem deps: `ConstantInfo.value?` → `Expr.getUsedConstants`; size = Expr node
  count (document the counting); kernel_depth = longest chain in the module's own
  dependency graph.
- Structure projection indices: LCNF `.proj typeName idx fvar` indexes into the
  declared field list — verified with `Conformance.MidProp`, whose
  `Prop` field sits in the middle (`Conformance/Structures.lean`). Field names
  come from `getStructureFields env structName` (resolves directly under 4.32,
  returns declared field names in declaration order including `Prop` fields;
  `Lean.getStructureInfo?` corroborates via `.fieldNames`); `Prop` fields are
  retained as an index slot (their projection is simply never emitted/used by
  `@[prod]` code, since no computational code touches a `Prop`). Constructor
  `numFields` also counts the declared (not erased) fields — confirmed
  `Conformance.MidProp.mk` has `numParams=0 numFields=4` for 4 declared fields.
  Getting this wrong swaps struct fields SILENTLY, so any change here must
  re-run that conformance case.

Lowerer requirements:
- Emit sexp matching `rust/prod-ir` grammar EXACTLY — read
  `lean/Conformance/golden.ir`,
  `rust/prod-ir/src/lib.rs`, `rust/prod-ir/src/parser.rs` first. Only extend the
  Rust parser if a needed form is missing; if you do, add tests, keep `cargo test` green.
- Def names: last component (`UorAtlas.stride` → `stride`), full name in a `;;` comment
  above each def (parser skips `;;` comments). Sanitize fvar binderNames to stable
  short idents (per-def counter fine); carry an `FVarId → String` map while descending.
- Operator whitelist (check parser.rs for exact keywords first):
  `Nat.add/sub/mul/div/mod/shiftLeft/shiftRight/pow/ble/blt` → arith/cmp nodes;
  unmapped consts → `(call name ...)` + counted as extern calls in coverage.
  *(HISTORICAL — what M3 built. Superseded in S0/S1: an unmapped const lowers
  to `(extern "Full.Name" ...)`, a distinct IR node that codegen rejects with
  `Error::UnresolvedCall`. It is still counted in coverage, but it is a hard
  build failure, not a rendered call.)*
- `cases`→`cases` node, `proj`→`proj`, `jp/jmp`→`jp`/`jmp`, `return x`→value,
  `unreach`→`unreachable`. Validated first-order `fun`/local calls use
  expression-valued IR `jp`/`jmp`, not a Lean tail-jump rewrite. Reject escaping,
  partial, overapplied or cyclic functions; preserve eager argument evaluation.
- Type lowering: `Nat/Bool/Int`→same, `UorAtlas.Instance`→`Instance`, else opaque-type
  form per parser.
  *(HISTORICAL — what M3 built. Superseded in S0/S1: the `UorAtlas.Instance`
  hard-wiring is deleted. Every user inductive lowers to `(named "Full.Name")`
  plus a `(type ...)` declaration, and codegen generates the struct/enum from
  that declaration; `Instance` is now an ordinary generated type with no
  special case anywhere. Only genuinely undescribable constants reach the
  opaque-type form, and codegen rejects those with `Error::OpaqueType`.)*

Emit defaults (cwd is `lean/`): `../rust/prod-core/kernel.ir`, `../roots.json`,
`../coverage.md`; support `--out DIR`. Hand-rolled JSON with escaping (no deps).
Coverage classes: EXPORTED / EXPORTED-WITH-OPAQUE (list them) / SKIPPED (with
`shouldGenerateCode` reason guess). Include theorems (skipped: prop) and Instance
ctors/projections in coverage.md.

M3 verify gates:
- `lake build` exit 0; `lake exe prod-export` writes the three files.
- `cd rust && cargo run -p prod-cli -- parse ../rust/prod-core/kernel.ir` succeeds.
- All five kernel defs lower with ZERO opaque nodes (they're Nat arith + projections;
  if anything is opaque, find out why — don't accept it).
- `roots.json` has classIndex_bijective, classDecode_encode, digits_lt_stride with
  non-empty deps; validates with `python3 -m json.tool`.
- Semantic eyeball: classIndex = stride*h2 + O*d + l, belt = class_count * 2^(O-1)
  (association may differ from the legacy fixture — M4 tests values, not shapes).

## M4–M6 (after M3)

- M4: prod-core generated defs come ONLY from kernel.ir via `prod_defs!`; exporter
  `--goldens` dumps Lean `#eval` values; Rust tests assert equality
  (classIndex(1,2,3)=43, stride=24, class_count=96, belt=12288, decode∘encode
  roundtrips on instances (4,3,8), (2,2,4), (5,1,3)). `just prod` green. Watch for:
  `proj "UorAtlas.Instance" idx` vs prod-core's named-field struct — either extend
  the codegen field map or generate the struct from Lean; decide then, document it.
- M5: port roots analysis from old repo's `uor-atlas-roots/lib/{graph,pareto,connect}.ml`
  (OCaml, read-only reference) into prod-cli: `prod roots check|pareto|connect` on
  roots.json — actually compute (the old OCaml CLI printed canned strings; don't
  replicate that). Unit tests for dominates/pareto_front/bridge synthesis.
- M6: prod-wasm (wasm-bindgen: `generate(ir)`, `roots_pareto(json)`), CI for
  `just prod` + no_std/wasm32 build check, README final pass.
