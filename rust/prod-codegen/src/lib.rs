//! prod-codegen: renders `prod-ir` modules as Rust source text.
//!
//! This crate is `#![no_std]` (with `alloc`) and host-independent: it renders
//! Rust code as a plain `String`, never as `proc_macro2::TokenStream`, so it
//! can run on wasm32 or inside other hosts. `prod-macros` and `prod-cli` are
//! thin drivers on top of [`generate_module`].
//!
//! # Code generation policy
//!
//! The generated code targets the project's production standard: it must not
//! panic on caller-controlled input, and it must not allocate. Those two rules
//! drive everything below.
//!
//! ## Memory profile: no heap, ever
//!
//! Nothing rendered here can allocate. Lean `List α` is the only type that
//! would naïvely need a heap, so its lowering is position-dependent:
//!
//! - **Parameter position** → `&[α]`. `List.nil` match arms render as the
//!   slice pattern `[]` and `List.cons (h t)` as `[h, t @ ..]`, so structural
//!   recursion passes the tail sub-slice directly — no rebinding, no copying.
//! - **Return position** → a caller-owned output buffer. The signature gains a
//!   trailing `output: &mut [α]` and returns `Result<usize, ComputeError>`,
//!   the length of the initialized prefix. The body is rendered in *builder
//!   mode*: `List.nil` becomes `Ok(0)`; `List.cons h t` splits one element off
//!   the front of the buffer (`split_first_mut`, so exhaustion is an `Err`,
//!   never an index panic), writes the head, recurses the tail into the
//!   remainder, and returns `1 +` the tail's length. `if`/`let`/`cases`
//!   recurse into builder mode; `let`-bound list values (LCNF emits lists in
//!   A-normal form) are resolved through a scoped environment rather than
//!   materialized.
//! - **Zero-argument definitions returning a list** (the golden values) →
//!   `&'static [α]` built from a promoted array literal.
//!
//! A list value that reaches any other position — an intermediate value used
//! as something other than a builder tail, or a list nested inside another
//! type — is an [`Error::UnsupportedList`]: an honest codegen failure rather
//! than a silently allocating fallback. `Type::Vec` is rejected outright as
//! [`Error::HeapType`].
//!
//! ## Error contract: fallibility is precise, not uniform
//!
//! Lean `Nat` maps to bounded `u64`, fixed-width Lean `Int64` maps to `i64`,
//! and mathematical Lean `Int` is rejected as [`Error::UnboundedInt`] rather
//! than silently narrowed. The partial operations report failure instead of panicking: addition, multiplication,
//! shifts, and powers render as `checked_*(..).ok_or(crate::ComputeError::X)?`
//! (with the shift/power exponent narrowed through
//! `u32::try_from(..).map_err(..)?`). Subtraction saturates at zero (Lean Nat
//! subtraction); division by zero returns zero and remainder by zero returns
//! the dividend (Lean Nat's total operations), so neither is fallible.
//! There is no bignum fallback, so this is exact only while values fit in `u64`.
//!
//! A definition returns `Result<T, crate::ComputeError>` **only if it needs
//! to**: if its body contains a checked operation, or calls a definition that
//! is itself fallible, or builds a list into a caller buffer. That is a least
//! fixpoint over the module's call graph ([`Shape`]), so leaf definitions and
//! the zero-argument goldens keep their plain return types. Calls to fallible
//! definitions render as `f(args)?`.
//!
//! ## Other lowerings
//!
//! - **LCNF nodes**:
//!   - `Match` renders as a Rust `match`, with `default` becoming the `_` arm.
//!     The Nat structural-recursion ctors are special-cased: `Nat.zero` renders
//!     as the literal pattern `0`, and `Nat.succ k` as the `_` arm with
//!     `k` bound to `(scrut).saturating_sub(1)` (exact, since the zero arm
//!     matches first). `Bool.true`/`Bool.false` → `true`/`false` patterns, and
//!     `Option.none`/`Option.some v` → `None`/`Some(v)` patterns. The List
//!     ctors use the slice patterns described above.
//!   - `Ctor` renders as tuple-style construction `Name(args...)` (bare `Name`
//!     when there are no args), except `Prod.mk`, which renders as a Rust
//!     tuple `(a, b)` — nested for right-nested pairs — and the Bool/Option
//!     ctors, which render as `true`/`false` and `None`/`Some(x)`.
//!   - `Proj` renders straight through: `(proj "Type" "field" e)` becomes
//!     `e.field` (raw-escaped if `field` is a Rust keyword). The field name
//!     is resolved once, in `Lower.lean`, against Lean's own structure info
//!     — codegen holds no type-keyed lookup table, so there is no second
//!     copy of the declaration that could disagree with the first and swap
//!     fields silently. `crate::Instance` is generated like any other type
//!     and mirrors the Lean structure's own field spelling (`q`, `T`, `O`)
//!     for exactly this reason.
//!
//!   - `Type::Tuple` renders as a Rust tuple type, so
//!     `(Tuple Nat (Tuple Nat Nat))` becomes `(u64, (u64, u64))`.
//!   - `Unreachable` renders as `unreachable!()`.
//!   - **Jp/Jmp policy**: an acyclic join point with one or more `jmp` callers
//!     is inlined independently at every jump site as
//!     `{ let p = arg; ...; <jp body> }`, and the declaration site renders as
//!     `()`. A join point with no callers renders its body in place. Anything
//!     cyclic is rejected as
//!     [`Error::UnsupportedJoinPoint`], because it would need real control
//!     flow. This used to emit a `loop {}` skeleton with a "manual port
//!     required" comment, which did not compile: the join point's parameters
//!     were never bound, and each jump site had type `()` where its arm
//!     needed a value.
//!
//! ## Recursion
//!
//! Generated recursion is structurally bounded by a fuel or data argument (the
//! Lean side must already be terminating for LCNF to emit it), so stack depth
//! is a function of the caller's inputs, not of unbounded search.

#![no_std]

extern crate alloc;

mod c_abi;
mod core_wasm;
mod joins;
mod naming;
mod ownership;
mod package;
mod sdk;
mod text_view;
mod view;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use ownership::{expression_type, pattern_types, LocalTypes};
use prod_ir::{Alt, CtorDecl, Definition, Expr, Module, Type, TypeDecl};

pub use c_abi::{generate_c_bindings, CAbiError, CBindings};
pub use core_wasm::{generate_core_wasm_package, CoreWasmSpec};
pub use package::{
    generate_cargo_package, CargoDependency, CargoPackageSpec, GeneratedPackage, PackageFile,
};
pub use sdk::{generate_sdks, SdkBindings};
pub use text_view::{generate_text_view_v1, TextBrowserAdapterBinding, TextViewV1};
pub use view::{
    generate_holoview_bundle, generate_view_v1, BrowserAdapterBinding, EvaluatedViewV1,
    GeneratedViewV1, ViewOperation,
};

/// Errors that can occur during code generation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Code generation is not possible for an opaque expression
    OpaqueExpr(String),
    /// `(param n)` refers to a parameter index outside the definition's list
    ParamOutOfBounds(usize),
    /// A list value appears somewhere the allocation-free lowering cannot
    /// render it: nested inside another type, or used as an intermediate
    /// value rather than flowing into the output buffer.
    UnsupportedList(String),
    /// A type that would require a heap allocation in generated code.
    HeapType(String),
    /// A type is defined in terms of itself; needs the tier-1 memory profile.
    RecursiveType(String),
    /// A type takes type parameters; needs monomorphization (S5).
    PolymorphicType(String),
    /// A field's type cannot appear in an allocation-free generated type.
    UnsupportedFieldType(String),
    /// Two Lean types share a last name component, so they would collide.
    DuplicateTypeName(String),
    /// A type reached codegen with no rendering.
    OpaqueType(String),
    /// Mathematical Lean `Int` has no exact fixed-width Rust representation.
    UnboundedInt,
    /// The exporter could not resolve a callee to a generated definition.
    UnresolvedCall(String),
    /// A projection names a field the declared type does not have. Catches a
    /// declaration and a projection disagreeing within one IR file.
    UnknownField(String, String),
    /// A join point that jumps to itself. Acyclic join points are duplicated
    /// at their call sites; cycles would need real control flow.
    UnsupportedJoinPoint(String),
    /// Acyclic join expansion exceeds 65,536 expression nodes or 128 nested calls.
    JoinExpansionLimit,
    /// Two simultaneous parameters or pattern fields bind the same name.
    /// Ordinary nested shadowing and sibling name reuse remain supported.
    DuplicateBinding(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OpaqueExpr(s) => write!(f, "cannot generate code for opaque expression: {}", s),
            Error::ParamOutOfBounds(i) => write!(f, "parameter index {} is out of bounds", i),
            Error::UnsupportedList(s) => {
                write!(f, "list value cannot be rendered without allocating: {}", s)
            }
            Error::HeapType(s) => write!(
                f,
                "type would require a heap allocation in generated code: {}",
                s
            ),
            Error::RecursiveType(s) => write!(
                f,
                "recursive type `{}` cannot be rendered allocation-free (needs the tier-1 profile)",
                s
            ),
            Error::PolymorphicType(s) => write!(
                f,
                "type `{}` has type parameters; monomorphization is not implemented",
                s
            ),
            Error::UnsupportedFieldType(s) => {
                write!(f, "field type is not allowed in a generated type: {}", s)
            }
            Error::DuplicateTypeName(s) => {
                write!(f, "two Lean types share the last name component `{}`", s)
            }
            Error::OpaqueType(s) => write!(f, "no Rust rendering for type: {}", s),
            Error::UnboundedInt => write!(
                f,
                "mathematical Lean `Int` is unbounded and cannot be represented by a fixed-width Rust integer"
            ),
            Error::UnresolvedCall(s) => write!(
                f,
                "`{}` is neither @[prod]-tagged nor a whitelisted operator, so there is nothing to call",
                s
            ),
            Error::UnknownField(ty, field) => {
                write!(f, "type `{}` declares no field `{}`", ty, field)
            }
            Error::UnsupportedJoinPoint(name) => write!(
                f,
                "join point `{}` is cyclic or its argument count differs from its parameters",
                name
            ),
            Error::JoinExpansionLimit => write!(
                f,
                "join expansion exceeds 65536 expression nodes or 128 nested calls per definition"
            ),
            Error::DuplicateBinding(name) => write!(
                f,
                "simultaneous parameters or pattern fields repeat binding `{}`",
                name
            ),
        }
    }
}

/// The rejections the generator makes, for the published subset contract
/// (`prod subset`, `specs/lean-for-production.md`). One entry per `Error`
/// variant, in declaration order; keep in step with `Error` above — the
/// contract is rendered from this list, so a variant missing here is a
/// rejection the published contract silently fails to disclose.
pub const REJECTIONS: &[(&str, &str)] = &[
    (
        "OpaqueExpr",
        "an expression with no Rust rendering",
    ),
    (
        "ParamOutOfBounds",
        "a parameter index outside the definition's parameter list",
    ),
    (
        "UnsupportedList",
        "a list value outside supported slice, output-buffer, constant or owned collection positions; computed eager jump arguments in builder/static mode require unsupported intermediate storage",
    ),
    (
        "HeapType",
        "a type that would require a heap allocation in generated code",
    ),
    (
        "RecursiveType",
        "an inductive refers to itself (directly, or through one level of indirection); needs the tier-1 memory profile",
    ),
    (
        "PolymorphicType",
        "an inductive has type parameters; monomorphization is not implemented",
    ),
    (
        "UnsupportedFieldType",
        "a field type not allowed in an allocation-free generated type (e.g. a list or vector field, which would need owned storage)",
    ),
    (
        "DuplicateTypeName",
        "two Lean types share a last name component, so they would collide in Rust",
    ),
    (
        "OpaqueType",
        "a type reached codegen with no Rust rendering",
    ),
    (
        "UnboundedInt",
        "mathematical Lean Int reaches a fixed-width runtime target",
    ),
    (
        "UnresolvedCall",
        "the callee is neither @[prod]-tagged nor a whitelisted operator, so there is nothing to call",
    ),
    (
        "UnknownField",
        "a projection names a field the declared type does not have",
    ),
    (
        "UnsupportedJoinPoint",
        "a cyclic join point or a jump whose argument count differs from its parameters; acyclic continuations are specialized before ownership analysis",
    ),
    (
        "JoinExpansionLimit",
        "acyclic join expansion exceeds 65536 expression nodes or 128 nested continuation calls per definition; checked before materialization, not an application memory or general IR-depth guarantee",
    ),
    (
        "DuplicateBinding",
        "simultaneous parameters or pattern fields repeat a name; nested shadowing and sibling name reuse remain supported",
    ),
];

/// How a generated definition presents itself to its callers.
///
/// Computed for the whole module up front, because a call site cannot know
/// whether to append `?` until the callee's shape is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Plain value: `fn f(..) -> T`.
    Value,
    /// Fallible: `fn f(..) -> Result<T, ComputeError>`; call sites append `?`.
    Fallible,
    /// List builder: `fn f(.., output: &mut [E]) -> Result<usize, ComputeError>`.
    Buffer,
    /// Zero-argument list golden: `fn f() -> &'static [E]`.
    StaticList,
}

/// Definition name → [`Shape`], for one module.
type Signatures<'m> = BTreeMap<&'m str, Shape>;

/// Rust keywords that a Lean field or constructor name may legitimately be.
/// Escaped with the raw-identifier prefix rather than renamed, so the Rust
/// name still matches the Lean name exactly.
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while", "abstract", "become", "box", "do", "final", "macro", "override", "priv",
    "typeof", "unsized", "virtual", "yield", "try", "gen",
];

/// A Lean identifier as a Rust identifier, raw-escaped if it is a keyword.
fn rust_ident(name: &str) -> String {
    if RUST_KEYWORDS.contains(&name) {
        format!("r#{}", name)
    } else {
        String::from(name)
    }
}

/// A Lean binder/local as a Rust local. Path keywords such as `self` cannot
/// be written as raw identifiers, so all keywords receive a stable prefix.
fn rust_local_ident(name: &str) -> String {
    if RUST_KEYWORDS.contains(&name) {
        format!("__prod_{name}")
    } else {
        String::from(name)
    }
}

/// Last dot-separated component of a full Lean name.
fn last_component(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// Full Lean type name → its declaration, for the module being rendered.
type TypeTable<'m> = BTreeMap<&'m str, &'m TypeDecl>;

fn type_table(types: &[TypeDecl]) -> Result<TypeTable<'_>, Error> {
    let mut by_full: TypeTable = BTreeMap::new();
    let mut short_seen: BTreeMap<&str, &str> = BTreeMap::new();
    for decl in types {
        let short = last_component(&decl.name);
        if let Some(previous) = short_seen.insert(short, &decl.name) {
            if previous != decl.name {
                return Err(Error::DuplicateTypeName(String::from(short)));
            }
        }
        by_full.insert(decl.name.as_str(), decl);
    }
    Ok(by_full)
}

/// Render one type declaration: a struct if it has exactly one constructor,
/// otherwise an enum with named-field variants.
///
fn copy_type(ty: &Type, table: &TypeTable, visiting: &mut BTreeSet<String>) -> bool {
    match ty {
        Type::String | Type::Bytes | Type::List(_) | Type::Vec(_) => false,
        Type::Named(name) => {
            if !visiting.insert(name.clone()) {
                return false;
            }
            let result = table.get(name.as_str()).is_some_and(|declaration| {
                declaration.ctors.iter().all(|constructor| {
                    constructor
                        .fields
                        .iter()
                        .all(|(_, field)| copy_type(field, table, visiting))
                })
            });
            visiting.remove(name);
            result
        }
        Type::Option(inner) => copy_type(inner, table, visiting),
        Type::Result { ok, error } => {
            copy_type(ok, table, visiting) && copy_type(error, table, visiting)
        }
        Type::Tuple(items) => items.iter().all(|item| copy_type(item, table, visiting)),
        _ => true,
    }
}

fn generate_type_decl(decl: &TypeDecl, table: &TypeTable) -> Result<String, Error> {
    // The exporter reached this type but could not describe it. It is declared
    // anyway so that the rejection names a reason instead of an unknown type.
    if let Some(reason) = &decl.unsupported {
        return Err(match reason.as_str() {
            "type parameters" => Error::PolymorphicType(decl.name.clone()),
            "recursive" => Error::RecursiveType(decl.name.clone()),
            other => Error::OpaqueType(format!("{} ({})", decl.name, other)),
        });
    }
    for ctor in &decl.ctors {
        for (field, ty) in &ctor.fields {
            check_field_type(ty, &decl.name, field, table)?;
        }
    }

    let is_copy = decl.ctors.iter().all(|constructor| {
        constructor
            .fields
            .iter()
            .all(|(_, field)| copy_type(field, table, &mut BTreeSet::new()))
    });
    let mut out = if is_copy {
        String::from("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n")
    } else {
        String::from("#[derive(Debug, Clone, PartialEq, Eq)]\n")
    };
    let short = last_component(&decl.name);

    if decl.ctors.len() == 1 {
        let ctor = &decl.ctors[0];
        out.push_str(&format!("pub struct {} {{\n", rust_ident(short)));
        for (name, ty) in &ctor.fields {
            out.push_str(&format!(
                "    pub {}: {},\n",
                rust_ident(name),
                type_to_rust(ty)?
            ));
        }
        out.push_str("}\n");
        return Ok(out);
    }

    let fieldless = decl
        .ctors
        .iter()
        .all(|constructor| constructor.fields.is_empty());
    if fieldless && decl.ctors.len() <= (u8::MAX as usize) + 1 {
        out.push_str("#[repr(u8)]\n");
    }
    out.push_str(&format!("pub enum {} {{\n", rust_ident(short)));
    for (index, ctor) in decl.ctors.iter().enumerate() {
        let variant = rust_ident(last_component(&ctor.name));
        if ctor.fields.is_empty() {
            if fieldless && decl.ctors.len() <= (u8::MAX as usize) + 1 {
                out.push_str(&format!("    {} = {},\n", variant, index));
            } else {
                out.push_str(&format!("    {},\n", variant));
            }
            continue;
        }
        let mut fields = Vec::with_capacity(ctor.fields.len());
        for (name, ty) in &ctor.fields {
            fields.push(format!("{}: {}", rust_ident(name), type_to_rust(ty)?));
        }
        out.push_str(&format!("    {} {{ {} }},\n", variant, fields.join(", ")));
    }
    out.push_str("}\n");
    Ok(out)
}

/// A field type must be renderable and must not make the type recursive.
///
/// `owner` and `field` are the Lean constant and field name responsible, and
/// they appear in the rejection message: the point of this milestone is that a
/// failure names the declaration that caused it, and "a list field would need
/// owned storage" on its own leaves the reader to grep for which one.
fn check_field_type(ty: &Type, owner: &str, field: &str, table: &TypeTable) -> Result<(), Error> {
    match ty {
        Type::Named(n) => {
            if n == owner {
                return Err(Error::RecursiveType(String::from(owner)));
            }
            match table.get(n.as_str()) {
                // One level of indirection is enough to catch the mutual case
                // too: B referring back to A makes A reachable from A.
                Some(other) => {
                    for ctor in &other.ctors {
                        for (_, inner) in &ctor.fields {
                            if let Type::Named(m) = inner {
                                if m == owner {
                                    return Err(Error::RecursiveType(String::from(owner)));
                                }
                            }
                        }
                    }
                    Ok(())
                }
                None => Err(Error::OpaqueType(n.clone())),
            }
        }
        Type::List(inner) => check_field_type(inner, owner, field, table),
        Type::Vec(_) => Err(Error::UnsupportedFieldType(format!(
            "`{}.{}`: a vector field would need heap storage",
            owner, field
        ))),
        Type::Tuple(items) => {
            for item in items {
                check_field_type(item, owner, field, table)?;
            }
            Ok(())
        }
        Type::Option(inner) => check_field_type(inner, owner, field, table),
        Type::Result { ok, error } => {
            check_field_type(ok, owner, field, table)?;
            check_field_type(error, owner, field, table)
        }
        _ => Ok(()),
    }
}

/// Render a whole module: one `pub fn` per definition.
pub fn generate_module(module: &Module) -> Result<String, Error> {
    let table = type_table(&module.types)?;
    let shapes = signatures(&module.definitions);
    let mut out = String::new();
    for decl in &module.types {
        out.push_str(&generate_type_decl(decl, &table)?);
        out.push('\n');
    }
    for def in &module.definitions {
        out.push_str(&generate_def_in(def, &module.definitions, &shapes, &table)?);
        out.push('\n');
    }
    Ok(out)
}

/// Render a single definition as a `pub fn`.
///
/// Calls to definitions outside `def` itself are assumed infallible, since
/// there is no module to resolve them against; use [`generate_module`] when
/// cross-definition fallibility matters. With no module, there is no type
/// table either, so any `(named ...)` type in `def`'s signature is opaque.
pub fn generate_def(def: &Definition) -> Result<String, Error> {
    let one = core::slice::from_ref(def);
    let table: TypeTable = BTreeMap::new();
    generate_def_in(def, one, &signatures(one), &table)
}

/// Compute every definition's [`Shape`] as a least fixpoint over the call
/// graph: seed everything infallible, then promote until nothing changes.
/// Monotone (shapes only ever move `Value` → `Fallible`), so it terminates.
fn signatures<'m>(defs: &'m [Definition]) -> Signatures<'m> {
    let mut shapes: Signatures<'m> = defs
        .iter()
        .map(|def| {
            let shape = match &def.ret {
                Type::List(_) if def.params.is_empty() => Shape::StaticList,
                Type::List(_) => Shape::Buffer,
                _ => Shape::Value,
            };
            (def.name.as_str(), shape)
        })
        .collect();

    loop {
        let mut changed = false;
        for def in defs {
            if shapes.get(def.name.as_str()) != Some(&Shape::Value) {
                continue;
            }
            if is_fallible(&def.body, &shapes) {
                shapes.insert(def.name.as_str(), Shape::Fallible);
                changed = true;
            }
        }
        if !changed {
            return shapes;
        }
    }
}

/// Does this expression perform, or reach, an operation that can fail?
fn is_fallible(expr: &Expr, shapes: &Signatures) -> bool {
    let here = match expr {
        Expr::Add(..) | Expr::Mul(..) | Expr::Shl(..) | Expr::Pow(..) => true,
        Expr::Call(name, _) => matches!(
            shapes.get(name.as_str()),
            Some(Shape::Fallible) | Some(Shape::Buffer)
        ),
        _ => false,
    };
    here || expr.children().any(|child| is_fallible(child, shapes))
}

/// Only closed, supported values can be discarded without evaluating them.
/// Absence of ComputeError is not totality: calls can panic, and an unused
/// unsupported expression must still fail code generation.
fn discardable_value(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Nat(_) | Expr::Int(_) | Expr::Bool(_) | Expr::String(_) | Expr::Bytes(_)
    ) || matches!(expr, Expr::Ctor(name, arguments) if name == "Prod.mk" && arguments.is_empty())
}

fn count_var_uses(expr: &Expr, name: &str) -> usize {
    let here = usize::from(matches!(expr, Expr::Var(candidate) if candidate == name));
    here + expr
        .children()
        .map(|child| count_var_uses(child, name))
        .sum::<usize>()
}

/// Alternatives are exclusive: a binder used once in either branch can still
/// transfer its owner without a clone. Sequential uses remain additive.
fn count_path_uses(expr: &Expr, name: &str) -> usize {
    match expr {
        Expr::If(condition, yes, no) => {
            count_path_uses(condition, name)
                + count_path_uses(yes, name).max(count_path_uses(no, name))
        }
        Expr::Match {
            scrut,
            alts,
            default,
        } => {
            count_path_uses(scrut, name)
                + alts
                    .iter()
                    .map(|alt| count_path_uses(&alt.body, name))
                    .chain(default.iter().map(|value| count_path_uses(value, name)))
                    .max()
                    .unwrap_or(0)
        }
        _ => {
            usize::from(matches!(expr, Expr::Var(candidate) if candidate == name))
                + expr
                    .children()
                    .map(|child| count_path_uses(child, name))
                    .sum::<usize>()
        }
    }
}

/// Whether a binding expression produces a known non-`Copy` Rust value.
///
/// This deliberately answers `false` when the type is not recoverable from
/// the typed IR node. The exporter emits constructors, calls, projections,
/// and literals for the shared values this analysis targets; those cases all
/// retain enough type information for an exact answer.
fn expression_is_non_copy(
    expr: &Expr,
    definitions: &[Definition],
    table: &TypeTable<'_>,
    non_copy_locals: &BTreeSet<String>,
) -> bool {
    let non_copy_type = |ty: &Type| !copy_type(ty, table, &mut BTreeSet::new());
    match expr {
        Expr::String(_) | Expr::Bytes(_) => true,
        Expr::Var(name) => non_copy_locals.contains(name),
        Expr::Call(name, _) => definitions
            .iter()
            .find(|definition| definition.name == *name)
            .is_some_and(|definition| non_copy_type(&definition.ret)),
        Expr::Proj(owner, field, _) => table
            .get(owner.as_str())
            .and_then(|declaration| {
                declaration
                    .ctors
                    .iter()
                    .flat_map(|constructor| constructor.fields.iter())
                    .find_map(|(candidate, ty)| (candidate == field).then_some(ty))
            })
            .is_some_and(non_copy_type),
        Expr::Ctor(name, _)
            if matches!(
                name.as_str(),
                "List.nil"
                    | "List.cons"
                    | "Option.none"
                    | "Option.some"
                    | "Except.ok"
                    | "Except.error"
            ) =>
        {
            true
        }
        Expr::Ctor(name, _) => table.values().any(|declaration| {
            declaration
                .ctors
                .iter()
                .any(|constructor| constructor.name == *name)
                && non_copy_type(&Type::Named(declaration.name.clone()))
        }),
        // These operations have an owned string/byte/list-shaped result.
        Expr::Append(..)
        | Expr::Utf8Encode(..)
        | Expr::Utf8Decode(..)
        | Expr::SplitExact(..)
        | Expr::Join(..)
        | Expr::FormatDecimal(..) => true,
        Expr::Let(name, value, body) => {
            let mut nested = non_copy_locals.clone();
            if expression_is_non_copy(value, definitions, table, non_copy_locals) {
                nested.insert(name.clone());
            }
            expression_is_non_copy(body, definitions, table, &nested)
        }
        Expr::If(_, then_value, else_value) => {
            expression_is_non_copy(then_value, definitions, table, non_copy_locals)
                || expression_is_non_copy(else_value, definitions, table, non_copy_locals)
        }
        Expr::Match { alts, default, .. } => {
            alts.iter()
                .any(|alt| expression_is_non_copy(&alt.body, definitions, table, non_copy_locals))
                || default.as_ref().is_some_and(|value| {
                    expression_is_non_copy(value, definitions, table, non_copy_locals)
                })
        }
        _ => false,
    }
}

fn repeated_non_copy_locals(
    expr: &Expr,
    definitions: &[Definition],
    table: &TypeTable<'_>,
    params: &[(String, Type)],
    returns_copy: bool,
    borrowed_bindings: &BTreeSet<String>,
) -> BTreeSet<String> {
    struct Context<'a, 'm> {
        definitions: &'a [Definition],
        table: &'a TypeTable<'m>,
        parameters: &'a [(String, Type)],
        borrowed_bindings: &'a BTreeSet<String>,
    }

    fn walk(
        expr: &Expr,
        context: &Context<'_, '_>,
        non_copy_locals: &BTreeSet<String>,
        local_types: &LocalTypes,
        output: &mut BTreeSet<String>,
    ) {
        let Context {
            definitions,
            table,
            parameters,
            borrowed_bindings,
        } = *context;
        match expr {
            Expr::Let(name, value, body) => {
                walk(value, context, non_copy_locals, local_types, output);
                let mut nested = non_copy_locals.clone();
                nested.remove(name);
                let mut nested_types = local_types.clone();
                let value_type =
                    expression_type(value, definitions, table, local_types, parameters);
                if let Some(ty) = value_type.clone() {
                    nested_types.insert(name.clone(), ty);
                } else {
                    nested_types.remove(name);
                }
                if value_type
                    .as_ref()
                    .map(|ty| !copy_type(ty, table, &mut BTreeSet::new()))
                    .unwrap_or_else(|| {
                        expression_is_non_copy(value, definitions, table, non_copy_locals)
                    })
                {
                    nested.insert(name.clone());
                    // Borrowed projections can be shared; a moved field is
                    // an owned local and needs the usual reuse protection.
                    if !borrowed_bindings.contains(name)
                        && count_path_uses(body, name) > 1
                        && !single_owned_option_match(name, value, body, definitions, table)
                    {
                        output.insert(name.clone());
                    }
                }
                walk(body, context, &nested, &nested_types, output);
            }
            Expr::Match {
                scrut,
                alts,
                default,
            } => {
                walk(scrut, context, non_copy_locals, local_types, output);
                let scrutinee_type =
                    expression_type(scrut, definitions, table, local_types, parameters);
                for alt in alts {
                    let mut nested = non_copy_locals.clone();
                    let mut nested_types = local_types.clone();
                    for name in &alt.binders {
                        nested.remove(name);
                        nested_types.remove(name);
                    }
                    for (name, ty) in pattern_types(scrutinee_type.as_ref(), alt, table) {
                        if !copy_type(&ty, table, &mut BTreeSet::new()) {
                            nested.insert(name.clone());
                            if !borrowed_bindings.contains(&name)
                                && count_path_uses(&alt.body, &name) > 1
                            {
                                output.insert(name.clone());
                            }
                        }
                        nested_types.insert(name, ty);
                    }
                    walk(&alt.body, context, &nested, &nested_types, output);
                }
                if let Some(default) = default {
                    walk(default, context, non_copy_locals, local_types, output);
                }
            }
            Expr::Jp {
                params: join_params,
                body,
                ..
            } => {
                for parameter in join_params {
                    if count_path_uses(body, parameter) > 1 {
                        output.insert(parameter.clone());
                    }
                }
                walk(body, context, non_copy_locals, local_types, output);
            }
            _ => {
                for child in expr.children() {
                    walk(child, context, non_copy_locals, local_types, output);
                }
            }
        }
    }

    let mut output = BTreeSet::new();
    for (name, ty) in params {
        if !internal_borrowed_parameter(ty, table, returns_copy)
            && !copy_type(ty, table, &mut BTreeSet::new())
            && count_path_uses(expr, name) > 1
        {
            output.insert(name.clone());
        }
    }
    let context = Context {
        definitions,
        table,
        parameters: params,
        borrowed_bindings,
    };
    walk(
        expr,
        &context,
        &BTreeSet::new(),
        &params.iter().cloned().collect(),
        &mut output,
    );
    output
}

fn inline_bindings(expr: &Expr) -> BTreeMap<String, &Expr> {
    fn walk<'a>(expr: &'a Expr, output: &mut BTreeMap<String, &'a Expr>) {
        if let Expr::Let(name, value, _) = expr {
            if matches!(value.as_ref(), Expr::String(_) | Expr::Bytes(_))
                || matches!(
                    value.as_ref(),
                    Expr::Ctor(constructor, arguments)
                        if arguments.is_empty()
                            && matches!(constructor.as_str(), "List.nil" | "Option.none")
                )
            {
                // `List.nil` and `Option.none` are polymorphic in Lean. LCNF
                // can CSE one closed value across uses with different element
                // types, but one Rust local cannot have several monomorphic
                // types. String/byte literals are also inlined so borrowed
                // comparison/call positions stay allocation-free while owned
                // record fields still materialize an owned value at their use.
                output.insert(name.clone(), value);
            }
        }
        for child in expr.children() {
            walk(child, output);
        }
    }

    let mut output = BTreeMap::new();
    walk(expr, &mut output);
    output
}

/// Owners used exclusively through distinct field projections can be partially
/// moved. Names must already be lexically normalized. Any whole-owner use,
/// repeated field, or join point retains the conservative borrowing policy.
fn movable_projection_owners(definition: &Definition, table: &TypeTable<'_>) -> BTreeSet<String> {
    fn walk(
        expr: &Expr,
        fields: &mut BTreeMap<String, BTreeSet<String>>,
        retained: &mut BTreeSet<String>,
    ) -> bool {
        match expr {
            Expr::Proj(_, field, owner) => {
                if let Expr::Var(name) = owner.as_ref() {
                    if !fields
                        .entry(name.clone())
                        .or_default()
                        .insert(field.clone())
                    {
                        retained.insert(name.clone());
                    }
                    true
                } else {
                    walk(owner, fields, retained)
                }
            }
            Expr::Var(name) => {
                retained.insert(name.clone());
                true
            }
            // Join-point bodies may be substituted into another lexical
            // context. This pass deliberately does not model that ownership.
            Expr::Jp { .. } | Expr::Jmp { .. } => false,
            _ => expr
                .children()
                .into_iter()
                .all(|child| walk(child, fields, retained)),
        }
    }
    if returns_borrowed_projection(definition, table) {
        return BTreeSet::new();
    }
    let mut fields = BTreeMap::new();
    let mut retained = BTreeSet::new();
    if !walk(&definition.body, &mut fields, &mut retained) {
        return BTreeSet::new();
    }
    fields
        .into_keys()
        .filter(|name| !retained.contains(name))
        .collect()
}

fn projection_moves(expr: &Expr, locals: &BTreeSet<String>, movable: &BTreeSet<String>) -> bool {
    matches!(expr, Expr::Proj(_, _, owner)
        if matches!(owner.as_ref(), Expr::Var(name)
            if movable.contains(name) && !locals.contains(name)))
}

/// Whether value-mode rendering borrows an existing owner. This is separate
/// from non-Copy analysis: an owned constructor must copy a borrowed field even
/// when it uses that field only once, while predicates must keep borrowing.
fn expression_is_borrowed(
    expr: &Expr,
    definitions: &[Definition],
    table: &TypeTable<'_>,
    locals: &BTreeSet<String>,
    movable: &BTreeSet<String>,
) -> bool {
    match expr {
        Expr::Var(name) => locals.contains(name),
        Expr::Proj(..) => {
            !projection_moves(expr, locals, movable)
                && expression_is_non_copy(expr, definitions, table, &BTreeSet::new())
        }
        Expr::Call(name, _) => definitions
            .iter()
            .find(|definition| definition.name == *name)
            .is_some_and(|definition| {
                returns_borrowed_projection(definition, table)
                    || definition.params.is_empty() && matches!(definition.ret, Type::List(_))
            }),
        Expr::Let(_, _, body) => expression_is_borrowed(body, definitions, table, locals, movable),
        Expr::If(_, then_value, else_value) => {
            expression_is_borrowed(then_value, definitions, table, locals, movable)
                && expression_is_borrowed(else_value, definitions, table, locals, movable)
        }
        Expr::Match { alts, default, .. } => {
            (!alts.is_empty() || default.is_some())
                && alts.iter().all(|alt| {
                    expression_is_borrowed(&alt.body, definitions, table, locals, movable)
                })
                && default.as_ref().is_none_or(|value| {
                    expression_is_borrowed(value, definitions, table, locals, movable)
                })
        }
        _ => false,
    }
}

fn list_head_rebound_by_value(
    scrutinee: &Expr,
    params: &[(String, Type)],
    table: &TypeTable<'_>,
) -> bool {
    // Known non-Copy parameter heads stay borrowed. Other heads retain the
    // renderer's existing clone/rebind, including unknown nested/alias types;
    // `true` means rebound, not a claim that an unknown element is Copy.
    let Expr::Var(name) = scrutinee else {
        return true;
    };
    params
        .iter()
        .find_map(|(parameter, ty)| {
            (parameter == name).then_some(ty).and_then(|ty| match ty {
                Type::List(element) => Some(copy_type(element, table, &mut BTreeSet::new())),
                _ => None,
            })
        })
        .unwrap_or(true)
}

#[derive(Default)]
struct BindingOwnership {
    borrowed: BTreeSet<String>,
    copied_patterns: BTreeSet<String>,
}

fn binding_ownership(
    definition: &Definition,
    definitions: &[Definition],
    table: &TypeTable<'_>,
    movable: &BTreeSet<String>,
) -> BindingOwnership {
    fn walk(
        expr: &Expr,
        definitions: &[Definition],
        table: &TypeTable<'_>,
        params: &[(String, Type)],
        local_types: &LocalTypes,
        locals: &mut BindingOwnership,
        movable: &BTreeSet<String>,
    ) {
        match expr {
            Expr::Let(name, value, body) => {
                walk(
                    value,
                    definitions,
                    table,
                    params,
                    local_types,
                    locals,
                    movable,
                );
                let mut nested_types = local_types.clone();
                if let Some(ty) = expression_type(value, definitions, table, local_types, params) {
                    nested_types.insert(name.clone(), ty);
                } else {
                    nested_types.remove(name);
                }
                if expression_is_borrowed(value, definitions, table, &locals.borrowed, movable) {
                    locals.borrowed.insert(name.clone());
                }
                walk(
                    body,
                    definitions,
                    table,
                    params,
                    &nested_types,
                    locals,
                    movable,
                );
            }
            Expr::Match {
                scrut,
                alts,
                default,
            } => {
                walk(
                    scrut,
                    definitions,
                    table,
                    params,
                    local_types,
                    locals,
                    movable,
                );
                let borrowed =
                    expression_is_borrowed(scrut, definitions, table, &locals.borrowed, movable);
                let scrutinee_type =
                    expression_type(scrut, definitions, table, local_types, params);
                for alt in alts {
                    let fields = pattern_types(scrutinee_type.as_ref(), alt, table);
                    let mut nested_types = local_types.clone();
                    for name in &alt.binders {
                        nested_types.remove(name);
                    }
                    nested_types.extend(fields.clone());
                    if alt.ctor == "List.cons" && alt.binders.len() == 2 {
                        // Slice patterns always borrow their tail. Keep the
                        // head classification identical to render_match's
                        // optional by-value rebind, including nested matches.
                        locals.borrowed.insert(alt.binders[1].clone());
                        if !list_head_rebound_by_value(scrut, params, table) {
                            locals.borrowed.insert(alt.binders[0].clone());
                        }
                    }
                    if borrowed {
                        if alt.ctor != "List.cons" {
                            for (name, ty) in &fields {
                                if copy_type(ty, table, &mut BTreeSet::new()) {
                                    // Builtin patterns use match ergonomics too.
                                    // Record positive typed evidence for their
                                    // by-value rebind; an unknown type is not Copy.
                                    locals.copied_patterns.insert(name.clone());
                                } else {
                                    locals.borrowed.insert(name.clone());
                                }
                            }
                        }
                        if let Some(constructor) = table.values().find_map(|declaration| {
                            declaration.ctors.iter().find(|row| row.name == alt.ctor)
                        }) {
                            for ((_, ty), binder) in constructor.fields.iter().zip(&alt.binders) {
                                // Copy fields are rebound by value in render_match.
                                if !copy_type(ty, table, &mut BTreeSet::new()) {
                                    locals.borrowed.insert(binder.clone());
                                }
                            }
                        }
                    }
                    walk(
                        &alt.body,
                        definitions,
                        table,
                        params,
                        &nested_types,
                        locals,
                        movable,
                    );
                }
                if let Some(default) = default {
                    walk(
                        default,
                        definitions,
                        table,
                        params,
                        local_types,
                        locals,
                        movable,
                    );
                }
            }
            _ => {
                for child in expr.children() {
                    walk(
                        child,
                        definitions,
                        table,
                        params,
                        local_types,
                        locals,
                        movable,
                    );
                }
            }
        }
    }
    let returns_copy = copy_type(&definition.ret, table, &mut BTreeSet::new());
    let mut locals = BindingOwnership {
        borrowed: definition
            .params
            .iter()
            .filter(|(_, ty)| internal_borrowed_parameter(ty, table, returns_copy))
            .map(|(name, _)| name.clone())
            .collect(),
        copied_patterns: BTreeSet::new(),
    };
    walk(
        &definition.body,
        definitions,
        table,
        &definition.params,
        &definition.params.iter().cloned().collect(),
        &mut locals,
        movable,
    );
    locals
}

/// Matching None does not move a payload, so that arm can return the original
/// Option without cloning the Some payload. Keep the original typed expression
/// intact: replacing the return with an untyped None can lose type inference.
/// Any other use of the owner retains the existing conservative clone policy.
fn single_owned_option_match(
    name: &str,
    value: &Expr,
    body: &Expr,
    definitions: &[Definition],
    table: &TypeTable<'_>,
) -> bool {
    let Expr::Call(callee, _) = value else {
        return false;
    };
    if !definitions.iter().any(|definition| {
        definition.name == *callee
            && matches!(definition.ret, Type::Option(_))
            && !returns_borrowed_projection(definition, table)
    }) {
        return false;
    }
    let Expr::Match {
        scrut,
        alts,
        default,
    } = body
    else {
        return false;
    };
    if !matches!(scrut.as_ref(), Expr::Var(scrutinee) if scrutinee == name)
        || default.is_some()
        || alts.len() != 2
        || !alts.iter().any(|alt| {
            alt.ctor == "Option.some"
                && alt.binders.len() == 1
                && count_var_uses(&alt.body, name) == 0
        })
    {
        return false;
    }
    alts.iter().any(|alt| {
        alt.ctor == "Option.none"
            && alt.binders.is_empty()
            && matches!(&alt.body, Expr::Var(returned) if returned == name)
    })
}

fn generate_def_in<'m>(
    def: &'m Definition,
    definitions: &'m [Definition],
    shapes: &Signatures<'m>,
    table: &TypeTable<'m>,
) -> Result<String, Error> {
    // Ownership and inline-value tables are keyed by local name. Preserve
    // lexical scopes at the public IR boundary before building those tables.
    let normalized = naming::normalize_definition(
        def,
        &|name| emitted_call_name(name, definitions, table),
        table,
    )?;
    let (normalized, eager_bindings) = if let Some(expanded) = joins::expand(&normalized)? {
        let join_parameters = JpContext::collect(&normalized.body)
            .decls
            .values()
            .flat_map(|(parameters, _)| parameters.iter().cloned())
            .collect();
        naming::normalize_tracking(
            &expanded,
            &|name| emitted_call_name(name, definitions, table),
            table,
            &join_parameters,
        )?
    } else {
        (normalized, BTreeSet::new())
    };
    let def = &normalized;
    let shape = shapes
        .get(def.name.as_str())
        .copied()
        .unwrap_or(Shape::Value);
    let returns_copy = copy_type(&def.ret, table, &mut BTreeSet::new());
    let helper = needs_borrowed_helper(def, table);
    let generated_name = if helper {
        borrowed_helper_name(def, definitions)
    } else {
        def.name.clone()
    };
    let visibility = if helper { "" } else { "pub " };
    let movable_projections = movable_projection_owners(def, table);
    let bindings = binding_ownership(def, definitions, table, &movable_projections);
    let renderer = Renderer {
        shapes,
        definitions,
        params: &def.params,
        ctx: JpContext::collect(&def.body),
        types: table,
        clone_locals: repeated_non_copy_locals(
            &def.body,
            definitions,
            table,
            &def.params,
            returns_copy,
            &bindings.borrowed,
        ),
        inline_values: inline_bindings(&def.body),
        borrowed_locals: bindings.borrowed,
        copied_patterns: bindings.copied_patterns,
        movable_projections,
        eager_bindings,
    };

    let mut params = String::new();
    for (i, (name, ty)) in def.params.iter().enumerate() {
        if i > 0 {
            params.push_str(", ");
        }
        params.push_str(&format!(
            "{}: {}",
            rust_local_ident(name),
            param_type_to_rust(
                ty,
                table,
                internal_borrowed_parameter(ty, table, returns_copy),
            )?
        ));
    }
    check_named_type(&def.ret, table)?;
    let borrowed_return = returns_borrowed_projection(def, table);
    let return_type = if borrowed_return {
        format!("&{}", type_to_rust(&def.ret)?)
    } else {
        type_to_rust(&def.ret)?
    };

    let implementation = match shape {
        Shape::StaticList => {
            let elem = list_element(&def.ret)?;
            if is_fallible(&def.body, shapes) {
                return Err(Error::UnsupportedList(format!(
                    "`{}` computes its list elements, so it cannot be a promoted &'static slice",
                    def.name
                )));
            }
            let mut items = Vec::new();
            renderer.static_list(&def.body, &[], &mut items)?;
            Ok(format!(
                "{visibility}fn {}() -> &'static [{}] {{\n    &[{}]\n}}\n",
                generated_name,
                type_to_rust(elem)?,
                items.join(", ")
            ))
        }
        Shape::Buffer => {
            let elem = list_element(&def.ret)?;
            if !params.is_empty() {
                params.push_str(", ");
            }
            params.push_str(&format!("output: &mut [{}]", type_to_rust(elem)?));
            let body = renderer.render(
                &def.body,
                &Mode::Builder {
                    out: "output",
                    env: &[],
                    depth: 0,
                },
            )?;
            Ok(format!(
                "{visibility}fn {}({}) -> Result<usize, crate::ComputeError> {{\n    {}\n}}\n",
                generated_name, params, body
            ))
        }
        Shape::Fallible => Ok(format!(
            "{visibility}fn {}({}) -> Result<{}, crate::ComputeError> {{\n    Ok({})\n}}\n",
            generated_name,
            params,
            return_type,
            renderer.render_return(&def.body, borrowed_return)?
        )),
        Shape::Value => Ok(format!(
            "{visibility}fn {}({}) -> {} {{\n    {}\n}}\n",
            generated_name,
            params,
            return_type,
            renderer.render_return(&def.body, borrowed_return)?
        )),
    }?;

    if !helper {
        return Ok(implementation);
    }

    let mut public_params = Vec::with_capacity(def.params.len());
    let mut arguments = Vec::with_capacity(def.params.len());
    for (name, ty) in &def.params {
        let local = rust_local_ident(name);
        let public_borrowed = public_borrowed_parameter(ty, table, def.ret == Type::Bool);
        let internal_borrowed = internal_borrowed_parameter(ty, table, returns_copy);
        public_params.push(format!(
            "{local}: {}",
            param_type_to_rust(ty, table, public_borrowed)?
        ));
        arguments.push(if internal_borrowed && !public_borrowed {
            if matches!(ty, Type::String | Type::Bytes) {
                format!("{local}.as_ref()")
            } else {
                format!("&{local}")
            }
        } else {
            local
        });
    }
    let public_return = match shape {
        Shape::Value => return_type,
        Shape::Fallible => format!("Result<{return_type}, crate::ComputeError>"),
        Shape::Buffer | Shape::StaticList => {
            unreachable!("list results do not use borrowed helpers")
        }
    };
    Ok(format!(
        "pub fn {}({}) -> {} {{\n    {}({})\n}}\n\n{}",
        def.name,
        public_params.join(", "),
        public_return,
        generated_name,
        arguments.join(", "),
        implementation
    ))
}

/// Whether a definition is the compiler-generated shape of an accessor for a
/// non-`Copy` field. Returning a borrow preserves the source value without a
/// clone (and therefore without a possible allocation). The receiver must be
/// the unique borrowed input, so its storage outlives the call and ordinary
/// Rust lifetime elision binds the result to that input unambiguously.
fn returns_borrowed_projection(definition: &Definition, table: &TypeTable) -> bool {
    let Expr::Let(name, value, body) = &definition.body else {
        return false;
    };
    if !matches!(body.as_ref(), Expr::Var(result) if result == name) {
        return false;
    }
    let Expr::Proj(owner, field, receiver) = value.as_ref() else {
        return false;
    };
    let parameter = match receiver.as_ref() {
        Expr::Var(name) => definition
            .params
            .iter()
            .find(|(parameter, _)| parameter == name),
        Expr::Param(index) => definition.params.get(*index),
        _ => None,
    };
    if !matches!(parameter, Some((_, Type::Named(name))) if name == owner)
        || definition
            .params
            .iter()
            .filter(|(_, ty)| internal_borrowed_parameter(ty, table, false))
            .count()
            != 1
    {
        return false;
    }
    table
        .get(owner.as_str())
        .and_then(|declaration| {
            declaration
                .ctors
                .iter()
                .flat_map(|constructor| constructor.fields.iter())
                .find_map(|(name, ty)| (name == field).then_some(ty))
        })
        .is_some_and(|field_type| {
            field_type == &definition.ret && !copy_type(field_type, table, &mut BTreeSet::new())
        })
}

/// Rust spelling of a type in an ordinary (owned, by-value) position.
fn type_to_rust(ty: &Type) -> Result<String, Error> {
    Ok(match ty {
        Type::Nat => String::from("u64"),
        Type::Int => return Err(Error::UnboundedInt),
        Type::Int8 => String::from("i8"),
        Type::Int16 => String::from("i16"),
        Type::Int32 => String::from("i32"),
        Type::Int64 => String::from("i64"),
        Type::UInt8 => String::from("u8"),
        Type::UInt16 => String::from("u16"),
        Type::UInt32 => String::from("u32"),
        Type::UInt64 => String::from("u64"),
        Type::String => String::from("alloc::string::String"),
        Type::Bytes => String::from("alloc::vec::Vec<u8>"),
        Type::Ordering => String::from("core::cmp::Ordering"),
        Type::Bool => String::from("bool"),
        Type::Named(n) => format!("crate::{}", rust_ident(last_component(n))),
        Type::Option(inner) => format!("Option<{}>", type_to_rust(inner)?),
        Type::Result { ok, error } => {
            format!("Result<{}, {}>", type_to_rust(ok)?, type_to_rust(error)?)
        }
        Type::Tuple(items) => {
            let mut rendered = Vec::with_capacity(items.len());
            for item in items {
                rendered.push(type_to_rust(item)?);
            }
            format!("({})", rendered.join(", "))
        }
        Type::Opaque(s) => return Err(Error::OpaqueType(s.clone())),
        // A top-level list parameter/return still uses the allocation-free
        // slice/buffer ABI. Nested list values are explicit owned data and
        // therefore use `Vec` in generated Cargo packages.
        Type::List(inner) => format!("alloc::vec::Vec<{}>", type_to_rust(inner)?),
        Type::Vec(inner) => {
            return Err(Error::HeapType(format!(
                "(Vec {})",
                type_to_rust(inner).unwrap_or_else(|_| String::from("_"))
            )))
        }
    })
}

/// Rust spelling of a parameter type: a top-level list borrows as a slice.
///
/// Checks named types against the module's type table first: parameter and
/// return types are not fields, so [`check_field_type`] never sees them, and
/// without this check an undeclared `(named ...)` in a signature would
/// silently render as `crate::Whatever` instead of being rejected.
fn param_type_to_rust(ty: &Type, table: &TypeTable, borrowed: bool) -> Result<String, Error> {
    check_named_type(ty, table)?;
    if borrowed {
        match ty {
            Type::String => Ok(String::from("&str")),
            Type::Bytes => Ok(String::from("&[u8]")),
            Type::List(inner) => Ok(format!("&[{}]", type_to_rust(inner)?)),
            _ => Ok(format!("&{}", type_to_rust(ty)?)),
        }
    } else {
        type_to_rust(ty)
    }
}

fn public_borrowed_parameter(ty: &Type, table: &TypeTable, returns_boolean: bool) -> bool {
    matches!(ty, Type::List(_))
        || matches!(ty, Type::Named(_)) && !copy_type(ty, table, &mut BTreeSet::new())
        || returns_boolean && matches!(ty, Type::String | Type::Bytes)
}

fn internal_borrowed_parameter(ty: &Type, table: &TypeTable, returns_copy: bool) -> bool {
    public_borrowed_parameter(ty, table, false)
        || returns_copy && !copy_type(ty, table, &mut BTreeSet::new())
}

fn needs_borrowed_helper(definition: &Definition, table: &TypeTable) -> bool {
    let returns_copy = copy_type(&definition.ret, table, &mut BTreeSet::new());
    definition.params.iter().any(|(_, ty)| {
        internal_borrowed_parameter(ty, table, returns_copy)
            != public_borrowed_parameter(ty, table, definition.ret == Type::Bool)
    })
}

fn borrowed_helper_name(definition: &Definition, definitions: &[Definition]) -> String {
    let mut candidate = format!("__prod_borrowed_{}", definition.name);
    while definitions.iter().any(|row| row.name == candidate) {
        candidate.push('_');
    }
    candidate
}

fn emitted_call_name(name: &str, definitions: &[Definition], table: &TypeTable<'_>) -> String {
    definitions
        .iter()
        .find(|definition| definition.name == name)
        .filter(|definition| needs_borrowed_helper(definition, table))
        .map(|definition| borrowed_helper_name(definition, definitions))
        .unwrap_or_else(|| String::from(name))
}

/// A `(named ...)` type occurring in a definition's signature must be
/// declared in the module's type table, at any depth (inside `Option`,
/// `List`, `Vec`, or `Tuple`); otherwise it has no known Rust rendering.
fn check_named_type(ty: &Type, table: &TypeTable) -> Result<(), Error> {
    match ty {
        Type::Named(n) => {
            if table.contains_key(n.as_str()) {
                Ok(())
            } else {
                Err(Error::OpaqueType(n.clone()))
            }
        }
        Type::Option(inner) | Type::Vec(inner) | Type::List(inner) => {
            check_named_type(inner, table)
        }
        Type::Result { ok, error } => {
            check_named_type(ok, table)?;
            check_named_type(error, table)
        }
        Type::Tuple(items) => {
            for item in items {
                check_named_type(item, table)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The element type of a list return type.
fn list_element(ty: &Type) -> Result<&Type, Error> {
    match ty {
        Type::List(inner) => Ok(inner),
        _ => Err(Error::UnsupportedList(
            "expected a list return type".to_string(),
        )),
    }
}

/// Join-point analysis for one definition body (two-pass jp/jmp lowering).
struct JpContext<'a> {
    /// name → (params, body) of each `jp` declaration in the body
    decls: BTreeMap<&'a str, (&'a [String], &'a Expr)>,
    /// name → total number of `jmp` sites in the body
    jmp_counts: BTreeMap<&'a str, usize>,
}

impl<'a> JpContext<'a> {
    fn collect(body: &'a Expr) -> Self {
        let mut ctx = JpContext {
            decls: BTreeMap::new(),
            jmp_counts: BTreeMap::new(),
        };
        ctx.walk(body);
        ctx
    }

    fn walk(&mut self, expr: &'a Expr) {
        // Record decls and counts, then recurse into every subexpression.
        match expr {
            Expr::Jp { name, params, body } => {
                self.decls.insert(name.as_str(), (params, body));
            }
            Expr::Jmp(name, _) => {
                *self.jmp_counts.entry(name.as_str()).or_insert(0) += 1;
            }
            _ => {}
        }
        for child in expr.children() {
            self.walk(child);
        }
    }

    fn jmp_count(&self, name: &str) -> usize {
        self.jmp_counts.get(name).copied().unwrap_or(0)
    }

    /// A join point is cyclic if a jump to it occurs inside its own body.
    fn is_cyclic(&self, name: &str) -> bool {
        match self.decls.get(name) {
            Some((_, body)) => count_jmps(body, name) > 0,
            None => false,
        }
    }

    /// Inlineable: at least one caller, and not self-referential.
    ///
    /// LCNF uses multi-caller join points as shared continuations for ordinary
    /// matches. Duplicating an acyclic pure continuation at each jump is
    /// allocation-free and preserves the expression result.
    fn is_inlineable(&self, name: &str) -> bool {
        self.jmp_count(name) > 0 && !self.is_cyclic(name)
    }
}

/// Number of `jmp <name>` sites within `expr`.
fn count_jmps(expr: &Expr, name: &str) -> usize {
    let self_count = match expr {
        Expr::Jmp(n, _) if n == name => 1,
        _ => 0,
    };
    self_count + expr.children().map(|c| count_jmps(c, name)).sum::<usize>()
}

/// Where the expression being rendered will land.
///
/// The two modes share one traversal: control flow (`if`, `let`, `cases`)
/// is rendered identically and simply propagates the mode into its branches,
/// while the leaves differ.
enum Mode<'x, 'm> {
    /// Ordinary value position. The rendered text has the expression's own
    /// Rust type, with `?` embedded wherever an operation can fail.
    Value,
    /// An owned ABI boundary. Convert borrowed results inside each lexical
    /// scope, before any branch-local owners are dropped.
    OwnedValue,
    /// List builder position. The rendered text has type
    /// `Result<usize, crate::ComputeError>` and fills `out`.
    Builder {
        /// The `&mut [T]` expression this list is written into.
        out: &'x str,
        /// `let`-bound list values in scope, innermost last. LCNF emits lists
        /// in A-normal form, so cons cells arrive as chains of `let`s rather
        /// than as one nested expression.
        env: &'x [(&'m str, &'m Expr)],
        /// Nesting depth, used to keep generated temporaries unique.
        depth: usize,
    },
}

struct Renderer<'s, 'm> {
    shapes: &'s Signatures<'m>,
    definitions: &'m [Definition],
    params: &'m [(String, Type)],
    ctx: JpContext<'m>,
    types: &'s TypeTable<'m>,
    /// Non-`Copy` LCNF `let` bindings referenced more than once.
    ///
    /// Lean values are immutable and may be shared freely. Rust record
    /// construction consumes owned fields, so an LCNF common subexpression
    /// such as one `String` used by two fields must be cloned at each use.
    /// Restricting this to repeated, known non-`Copy` locals keeps the
    /// allocation-free scalar/slice profile unchanged.
    clone_locals: BTreeSet<String>,
    /// Closed polymorphic empty constructors cannot share one inferred Rust
    /// local across differently monomorphized uses.
    inline_values: BTreeMap<String, &'m Expr>,
    /// Locals that borrow a field/parameter rather than owning its value.
    borrowed_locals: BTreeSet<String>,
    copied_patterns: BTreeSet<String>,
    /// Syntactically single-use fields; actual movement also requires an owned receiver.
    movable_projections: BTreeSet<String>,
    /// Expanded jump arguments are eager even when a list builder would
    /// otherwise defer ordinary list bindings into its symbolic environment.
    eager_bindings: BTreeSet<String>,
}

impl<'m> Renderer<'_, 'm> {
    fn borrows(&self, expr: &Expr) -> bool {
        expression_is_borrowed(
            expr,
            self.definitions,
            self.types,
            &self.borrowed_locals,
            &self.movable_projections,
        )
    }

    fn value(&self, expr: &'m Expr) -> Result<String, Error> {
        self.render(expr, &Mode::Value)
    }

    /// A read-only operand does not consume a local even when other uses do.
    /// Cloning here can copy an entire parser record merely to read one field.
    /// Non-local expressions retain ordinary evaluation and ownership rules.
    fn read_value(&self, expr: &'m Expr) -> Result<String, Error> {
        match self.resolved_inline(expr) {
            Expr::Var(name) => Ok(rust_local_ident(name)),
            other => self.value(other),
        }
    }

    fn owned_value(&self, expr: &'m Expr) -> Result<String, Error> {
        self.render(expr, &Mode::OwnedValue)
    }

    fn render_return(&self, expr: &'m Expr, borrowed: bool) -> Result<String, Error> {
        if borrowed {
            self.value(expr)
        } else {
            self.owned_value(expr)
        }
    }

    fn owned_leaf(&self, expr: &'m Expr) -> Result<String, Error> {
        let rendered = self.value(expr)?;
        if self.borrows(expr) {
            // Use the fully qualified alloc trait: generated no_std modules
            // need no extra imports. This also turns borrowed str/slices into
            // their owned String/Vec representation at this owned boundary.
            Ok(format!("alloc::borrow::ToOwned::to_owned({rendered})"))
        } else {
            Ok(rendered)
        }
    }

    fn parse_decimal(&self, value: &'m Expr, target: Option<&Type>) -> Result<String, Error> {
        let parse = match target {
            None => String::from("parse()"),
            Some(
                ty @ (Type::Int8
                | Type::Int16
                | Type::Int32
                | Type::Int64
                | Type::UInt8
                | Type::UInt16
                | Type::UInt32
                | Type::UInt64),
            ) => {
                format!("parse::<{}>()", type_to_rust(ty)?)
            }
            Some(Type::Int) => return Err(Error::UnboundedInt),
            Some(ty) => return Err(Error::OpaqueType(format!("parse-decimal-as target {ty:?}"))),
        };
        // Retain an owned temporary for the whole parse while borrowing an
        // exact str view of String, &String, or &str. The explicit target on
        // new IR prevents Rust inference from changing the source width.
        Ok(format!(
            "{{ let __input = &({}); let __text: &str = core::convert::AsRef::<str>::as_ref(__input); __text.{parse}.ok().filter(|__value| alloc::string::ToString::to_string(__value) == __text) }}",
            self.read_value(value)?
        ))
    }

    fn resolved_inline(&self, expr: &'m Expr) -> &'m Expr {
        match expr {
            Expr::Var(name) => self
                .inline_values
                .get(name)
                .map_or(expr, |value| self.resolved_inline(value)),
            _ => expr,
        }
    }

    fn is_empty_list(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Ctor(name, arguments) => name == "List.nil" && arguments.is_empty(),
            Expr::Var(name) => self
                .inline_values
                .get(name)
                .is_some_and(|value| self.is_empty_list(value)),
            _ => false,
        }
    }

    fn list_head_rebound_by_value(&self, scrutinee: &Expr) -> bool {
        list_head_rebound_by_value(scrutinee, self.params, self.types)
    }

    /// The declaration of a constructor, by its full Lean name.
    fn ctor_decl(&self, name: &str) -> Option<(&'m TypeDecl, &'m CtorDecl)> {
        self.types.values().find_map(|decl| {
            decl.ctors
                .iter()
                .find(|c| c.name == name)
                .map(|c| (*decl, c))
        })
    }

    fn shape_of(&self, name: &str) -> Option<Shape> {
        self.shapes.get(name).copied()
    }

    /// Return the declared type of a projected structure field.
    ///
    /// LCNF lowers a structure accessor returning `List T` to a projection in
    /// a `let`, rather than to a cons chain.  The list still has an existing
    /// owner (the structure argument), so it can be copied into the caller's
    /// output buffer without allocating.
    fn projection_field_type(&self, ty: &str, field: &str) -> Option<&'m Type> {
        self.types.get(ty).and_then(|declaration| {
            declaration
                .ctors
                .iter()
                .flat_map(|constructor| constructor.fields.iter())
                .find_map(|(name, ty)| (name == field).then_some(ty))
        })
    }

    fn render_call_args(&self, name: &str, args: &'m [Expr]) -> Result<Vec<String>, Error> {
        let definition = self
            .definitions
            .iter()
            .find(|definition| definition.name == name);
        args.iter()
            .enumerate()
            .map(|(index, argument)| {
                let argument = self.resolved_inline(argument);
                match definition.and_then(|definition| {
                    definition.params.get(index).map(|(_, ty)| {
                        (
                            ty,
                            internal_borrowed_parameter(
                                ty,
                                self.types,
                                copy_type(&definition.ret, self.types, &mut BTreeSet::new()),
                            ),
                        )
                    })
                }) {
                    Some((Type::String, true)) if matches!(argument, Expr::String(_)) => {
                        let Expr::String(value) = argument else {
                            unreachable!()
                        };
                        Ok(format!("{value:?}"))
                    }
                    Some((Type::Bytes, true)) if matches!(argument, Expr::Bytes(_)) => {
                        let Expr::Bytes(value) = argument else {
                            unreachable!()
                        };
                        Ok(format!("&{value:?}"))
                    }
                    Some((Type::String | Type::Bytes, true)) => {
                        let rendered = self.read_value(argument)?;
                        Ok(format!("({rendered}).as_ref()"))
                    }
                    Some((_, true)) => {
                        let rendered = self.read_value(argument)?;
                        Ok(format!("&({rendered})"))
                    }
                    Some((_, false)) => self.owned_value(argument),
                    _ => self.value(argument),
                }
            })
            .collect()
    }

    fn call_name(&self, name: &str) -> String {
        emitted_call_name(name, self.definitions, self.types)
    }

    /// Is this expression a list value (and therefore only renderable in
    /// builder position or as a `let` binding resolved through `env`)?
    fn is_list_valued(&self, expr: &Expr, env: &[(&'m str, &'m Expr)]) -> bool {
        match expr {
            Expr::Ctor(name, _) => name == "List.nil" || name == "List.cons",
            Expr::Call(name, _) => matches!(
                self.shape_of(name),
                Some(Shape::Buffer) | Some(Shape::StaticList)
            ),
            Expr::Var(name) => {
                lookup(env, name).is_some()
                    || self
                        .params
                        .iter()
                        .any(|(parameter, ty)| parameter == name && matches!(ty, Type::List(_)))
            }
            Expr::Proj(ty, field, _) => {
                matches!(self.projection_field_type(ty, field), Some(Type::List(_)))
            }
            _ => false,
        }
    }

    fn reject_eager_list_binding(
        &self,
        name: &str,
        value: &'m Expr,
        env: &[(&'m str, &'m Expr)],
    ) -> Result<(), Error> {
        if !self.eager_bindings.contains(name) {
            return Ok(());
        }
        // Jumps were not supported in builder/static-list positions before
        // ownership specialization. Do not silently add allocating storage or
        // defer a computed argument into the allocation-free builder's env.
        fn closed_list(value: &Expr) -> bool {
            match value {
                Expr::Ctor(name, fields) if name == "List.nil" => fields.is_empty(),
                Expr::Ctor(name, fields) if name == "List.cons" && fields.len() == 2 => {
                    discardable_value(&fields[0]) && closed_list(&fields[1])
                }
                _ => false,
            }
        }
        fn list_result<'m>(
            renderer: &Renderer<'_, 'm>,
            value: &'m Expr,
            env: &[(&'m str, &'m Expr)],
            locals: &LocalTypes,
            possible_lists: &BTreeSet<String>,
        ) -> bool {
            if matches!(value, Expr::Var(name) if possible_lists.contains(name)) {
                return true;
            }
            if renderer.is_list_valued(value, env) {
                return true;
            }
            let value_type = expression_type(
                value,
                renderer.definitions,
                renderer.types,
                locals,
                renderer.params,
            );
            if matches!(value_type, Some(Type::List(_))) {
                return true;
            }
            match value {
                Expr::Append(left, _) => list_result(renderer, left, env, locals, possible_lists),
                // In these previously unsupported builder jump positions,
                // unknown aliases cannot justify intermediate storage.
                Expr::Var(_) => value_type.is_none(),
                Expr::If(_, yes, no) => {
                    list_result(renderer, yes, env, locals, possible_lists)
                        || list_result(renderer, no, env, locals, possible_lists)
                }
                Expr::Let(name, bound, body) => {
                    let mut nested = env.to_vec();
                    let mut nested_types = locals.clone();
                    let mut nested_lists = possible_lists.clone();
                    if let Some(ty) = expression_type(
                        bound,
                        renderer.definitions,
                        renderer.types,
                        locals,
                        renderer.params,
                    ) {
                        nested_types.insert(name.clone(), ty);
                    } else {
                        nested_types.remove(name);
                    }
                    if list_result(renderer, bound, env, locals, possible_lists) {
                        nested.push((name, bound));
                        nested_lists.insert(name.clone());
                    } else {
                        nested_lists.remove(name);
                    }
                    list_result(renderer, body, &nested, &nested_types, &nested_lists)
                }
                Expr::Match {
                    scrut,
                    alts,
                    default,
                } => {
                    let scrutinee_type = expression_type(
                        scrut,
                        renderer.definitions,
                        renderer.types,
                        locals,
                        renderer.params,
                    );
                    alts.iter().any(|alt| {
                        let fields = pattern_types(scrutinee_type.as_ref(), alt, renderer.types);
                        let mut nested_types = locals.clone();
                        let mut nested_lists = possible_lists.clone();
                        for binder in &alt.binders {
                            match fields.get(binder) {
                                Some(ty) => {
                                    nested_types.insert(binder.clone(), ty.clone());
                                    if matches!(ty, Type::List(_)) {
                                        nested_lists.insert(binder.clone());
                                    } else {
                                        nested_lists.remove(binder);
                                    }
                                }
                                None => {
                                    nested_types.remove(binder);
                                    // Unknown pattern values cannot justify
                                    // introducing intermediate list storage.
                                    nested_lists.insert(binder.clone());
                                }
                            }
                        }
                        list_result(renderer, &alt.body, env, &nested_types, &nested_lists)
                    }) || default.as_deref().is_some_and(|value| {
                        list_result(renderer, value, env, locals, possible_lists)
                    })
                }
                _ => false,
            }
        }
        let value = self.resolved_inline(value);
        let mut locals: LocalTypes = self.params.iter().cloned().collect();
        let mut possible_lists = BTreeSet::new();
        for (name, bound) in env {
            if let Some(ty) =
                expression_type(bound, self.definitions, self.types, &locals, self.params)
            {
                locals.insert(String::from(*name), ty);
            }
            if self.is_list_valued(bound, env) {
                possible_lists.insert(String::from(*name));
            }
        }
        if list_result(self, value, env, &locals, &possible_lists) && !closed_list(value) {
            // Validate its supported expression forms before reporting the
            // storage boundary; opaque/extern arguments must never disappear.
            self.value(value)?;
            return Err(Error::UnsupportedList(String::from(
                "computed eager jump arguments require list storage in builder/static mode",
            )));
        }
        Ok(())
    }

    fn render(&self, expr: &'m Expr, mode: &Mode<'_, 'm>) -> Result<String, Error> {
        if matches!(mode, Mode::OwnedValue)
            && !matches!(expr, Expr::Let(..) | Expr::If(..) | Expr::Match { .. })
        {
            return self.owned_leaf(expr);
        }
        if let (Expr::Let(name, value, _), Mode::Builder { env, .. }) = (expr, mode) {
            self.reject_eager_list_binding(name, value, env)?;
        }
        match expr {
            // ---- control flow: identical in both modes ----
            Expr::If(cond, t, f)
                if matches!(mode, Mode::Value) && self.borrows(t) != self.borrows(f) =>
            {
                // Rust branches must agree on ownership. Keep a borrowed
                // result when both arms borrow; a mixed result owns both arms.
                Ok(format!(
                    "if {} {{ {} }} else {{ {} }}",
                    self.value(cond)?,
                    self.owned_value(t)?,
                    self.owned_value(f)?
                ))
            }
            Expr::If(cond, t, f) => Ok(format!(
                "if {} {{ {} }} else {{ {} }}",
                self.value(cond)?,
                self.render(t, mode)?,
                self.render(f, mode)?
            )),
            Expr::Let(name, _, body) if self.inline_values.contains_key(name) => {
                self.render(body, mode)
            }
            Expr::Let(name, value, body)
                if count_var_uses(body, name) == 0 && discardable_value(value) =>
            {
                self.render(body, mode)
            }
            Expr::Let(name, val, body) => match mode {
                Mode::Builder { out, env, depth } if self.is_list_valued(val, env) => {
                    // A list binding has no runtime representation to emit;
                    // record it and resolve uses through the environment.
                    let mut extended = env.to_vec();
                    extended.push((name.as_str(), val));
                    self.render(
                        body,
                        &Mode::Builder {
                            out,
                            env: &extended,
                            depth: *depth,
                        },
                    )
                }
                _ => Ok(format!(
                    "{{ let {} = {}; {} }}",
                    rust_local_ident(name),
                    self.value(val)?,
                    self.render(body, mode)?
                )),
            },
            Expr::Match {
                scrut,
                alts,
                default,
            } => self.render_match(scrut, alts, default.as_deref(), mode),

            // ---- list-shaped leaves ----
            Expr::Ctor(name, args) if name == "List.nil" && args.is_empty() => match mode {
                // Turbofished: an empty list is the one builder leaf that
                // constrains neither type parameter on its own, and it can
                // appear under a `?` (as the tail of a cons).
                Mode::Builder { .. } => Ok(String::from("Ok::<usize, crate::ComputeError>(0)")),
                Mode::Value | Mode::OwnedValue => Ok(String::from("alloc::vec::Vec::new()")),
            },
            Expr::Ctor(name, args) if name == "List.cons" && args.len() == 2 => match mode {
                Mode::Builder { out, env, depth } => {
                    self.render_cons(&args[0], &args[1], out, env, *depth)
                }
                Mode::Value | Mode::OwnedValue if self.is_empty_list(&args[1]) => {
                    Ok(format!("alloc::vec![{}]", self.owned_value(&args[0])?))
                }
                Mode::Value | Mode::OwnedValue => {
                    // Evaluate the head before the tail, then reuse the owned
                    // tail's capacity. Exact growth avoids doubling a full tail
                    // under the Wasm bump allocator. Tuple initialization keeps
                    // both operands outside the temporary's scope and preserves
                    // error order.
                    Ok(format!(
                        "{{ let mut __list = ({}, {}); __list.1.reserve_exact(1); __list.1.insert(0, __list.0); __list.1 }}",
                        self.owned_value(&args[0])?,
                        self.owned_value(&args[1])?
                    ))
                }
            },

            // ---- everything else ----
            Expr::Var(name) if self.inline_values.contains_key(name) => {
                self.render(self.inline_values[name], mode)
            }
            Expr::Var(name) => match mode {
                Mode::Builder { out, env, .. } => match lookup(env, name) {
                    Some(bound) => self.render(bound, mode),
                    None if self.params.iter().any(|(parameter, ty)| {
                        parameter == name && matches!(ty, Type::List(_))
                    }) =>
                    {
                        let source = rust_local_ident(name);
                        Ok(format!(
                            "if {source}.len() > ({out}).len() {{ Err(crate::ComputeError::OutputTooSmall) }} else {{ let __len = {source}.len(); ({out})[..__len].copy_from_slice({source}); Ok(__len) }}"
                        ))
                    }
                    None => Err(Error::UnsupportedList(format!(
                        "`{}` is not a list built in this definition",
                        name
                    ))),
                },
                Mode::Value | Mode::OwnedValue
                    if self.clone_locals.contains(name) && !self.borrowed_locals.contains(name) =>
                {
                    Ok(format!("{}.clone()", rust_local_ident(name)))
                }
                Mode::Value | Mode::OwnedValue => Ok(rust_local_ident(name)),
            },
            Expr::Call(name, args) => {
                let rendered = self.render_call_args(name, args)?;
                let call_name = self.call_name(name);
                match (mode, self.shape_of(name)) {
                    (Mode::Builder { out, .. }, Some(Shape::Buffer)) => {
                        // The callee writes straight into our remaining buffer
                        // and reports how much of it it used.
                        let mut all = rendered;
                        all.push((*out).to_string());
                        Ok(format!("{}({})", call_name, all.join(", ")))
                    }
                    (Mode::Builder { .. }, _) => Err(Error::UnsupportedList(format!(
                        "`{}` does not build its list into a caller buffer",
                        name
                    ))),
                    (Mode::Value | Mode::OwnedValue, Some(Shape::Buffer)) => {
                        Err(Error::UnsupportedList(format!(
                        "`{}` returns a list; its result cannot be used as an intermediate value",
                        name
                    )))
                    }
                    (Mode::Value | Mode::OwnedValue, Some(Shape::Fallible)) => {
                        Ok(format!("{}({})?", call_name, rendered.join(", ")))
                    }
                    (Mode::Value | Mode::OwnedValue, _) => {
                        Ok(format!("{}({})", call_name, rendered.join(", ")))
                    }
                }
            }

            // A list field already has owned storage in its enclosing value.
            // Copy its elements into the caller-provided buffer, just as a
            // top-level borrowed list parameter is copied.  `clone_from_slice`
            // preserves the generic generated-type contract without requiring
            // list elements to be `Copy`, and performs no allocation.
            Expr::Proj(ty, field, _) if matches!(mode, Mode::Builder { .. }) => {
                if !matches!(self.projection_field_type(ty, field), Some(Type::List(_))) {
                    return Err(Error::UnsupportedList(format!(
                        "`{}.{}` is not a list field",
                        ty, field
                    )));
                }
                let Mode::Builder { out, .. } = mode else {
                    unreachable!()
                };
                let source = self.value(expr)?;
                Ok(format!(
                    "{{ let __source = &({source}); if __source.len() > ({out}).len() {{ Err(crate::ComputeError::OutputTooSmall) }} else {{ let __len = __source.len(); ({out})[..__len].clone_from_slice(__source); Ok(__len) }} }}"
                ))
            }

            // An unresolved callee: refuse it outright rather than rendering
            // a call to a function nobody generated, in either mode.
            Expr::Extern(name, _) => Err(Error::UnresolvedCall(name.clone())),

            // Remaining nodes are value-typed; reaching them in builder mode
            // means the IR put a non-list where a list was declared.
            _ => match mode {
                Mode::Builder { .. } => Err(Error::UnsupportedList(
                    "expression does not build a list".to_string(),
                )),
                Mode::Value | Mode::OwnedValue => self.render_value_leaf(expr),
            },
        }
    }

    fn render_value_leaf(&self, expr: &'m Expr) -> Result<String, Error> {
        match expr {
            Expr::Nat(n) => Ok(format!("{}", n)),
            Expr::Int(n) => Ok(format!("{}", n)),
            Expr::String(value) => Ok(format!(
                "alloc::string::String::from({value:?})"
            )),
            // One owned allocation at the existing Bytes ABI boundary; the
            // compiler folds Array literal builders, so no push-chain or
            // intermediate runtime allocations are introduced.
            Expr::Bytes(value) => Ok(format!("alloc::vec!{value:?}")),
            Expr::Bool(b) => Ok(format!("{}", b)),
            Expr::Param(index) => self
                .params
                .get(*index)
                .map(|(name, _)| rust_local_ident(name))
                .ok_or(Error::ParamOutOfBounds(*index)),
            Expr::Add(a, b) => self.checked_binop(a, b, "checked_add", "AddOverflow"),
            Expr::Mul(a, b) => self.checked_binop(a, b, "checked_mul", "MulOverflow"),
            Expr::CheckedAdd(a, b) => Ok(format!(
                "({}).checked_add({})",
                self.value(a)?,
                self.value(b)?
            )),
            Expr::CheckedSub(a, b) => Ok(format!(
                "({}).checked_sub({})",
                self.value(a)?,
                self.value(b)?
            )),
            Expr::CheckedMul(a, b) => Ok(format!(
                "({}).checked_mul({})",
                self.value(a)?,
                self.value(b)?
            )),
            Expr::CheckedNeg(value) => Ok(format!("({}).checked_neg()", self.value(value)?)),
            Expr::CheckedDiv(a, b) => Ok(format!(
                "({}).checked_div({})",
                self.value(a)?,
                self.value(b)?
            )),
            Expr::BitAnd(a, b) => self.binop(a, b, "&"),
            Expr::BitOr(a, b) => self.binop(a, b, "|"),
            Expr::BitXor(a, b) => self.binop(a, b, "^"),
            Expr::BitNot(value) => Ok(format!("!({})", self.value(value)?)),
            Expr::CheckedShl(a, b) => Ok(format!(
                "({}).checked_shl({})",
                self.value(a)?,
                self.value(b)?
            )),
            Expr::CheckedShr(a, b) => Ok(format!(
                "({}).checked_shr({})",
                self.value(a)?,
                self.value(b)?
            )),
            Expr::CheckedConvert(value) => Ok(format!(
                "core::convert::TryFrom::try_from({}).ok()",
                self.value(value)?
            )),
            Expr::Append(left, right) => Ok(format!(
                "{{ let mut __value = {}; __value.extend_from_slice(&{}); __value }}",
                self.owned_value(left)?,
                self.read_value(right)?
            )),
            Expr::Length(value) => Ok(format!("({}).len() as u64", self.read_value(value)?)),
            Expr::Index(value, offset) => Ok(format!(
                "usize::try_from({}).ok().and_then(|__index| ({}).get(__index).cloned())",
                self.value(offset)?,
                self.read_value(value)?
            )),
            Expr::Slice(value, start, count) => Ok(format!(
                "{{ let __start = usize::try_from({}).ok(); let __count = usize::try_from({}).ok(); match (__start, __count) {{ (Some(__start), Some(__count)) => __start.checked_add(__count).and_then(|__end| ({}).get(__start..__end).map(|__slice| __slice.to_vec())), _ => None }} }}",
                self.value(start)?,
                self.value(count)?,
                self.read_value(value)?
            )),
            // Encoding consumes its String. Borrowed parameters and record
            // fields must cross the existing owned boundary first; already
            // owned Strings retain their allocation through `into_bytes`.
            Expr::Utf8Encode(value) => Ok(format!("({}).into_bytes()", self.owned_value(value)?)),
            Expr::Utf8Decode(value) => Ok(format!(
                "alloc::string::String::from_utf8({}).ok()",
                self.value(value)?
            )),
            Expr::CompareBytes(left, right) => Ok(format!(
                "core::convert::AsRef::<[u8]>::as_ref(&({})).cmp(core::convert::AsRef::<[u8]>::as_ref(&({})))",
                self.read_value(left)?,
                self.read_value(right)?
            )),
            Expr::SplitExact(value, delimiter, maximum) => Ok(format!(
                "{{ let __value = {}; let __delimiter = {}; let __limit: u32 = {}; let __maximum = usize::try_from(__limit).ok(); if __delimiter.is_empty() {{ None }} else {{ let __fields: alloc::vec::Vec<alloc::string::String> = __value.split(&__delimiter).map(alloc::string::String::from).collect(); __maximum.filter(|__maximum| __fields.len() <= *__maximum).map(|_| __fields) }} }}",
                self.value(value)?,
                self.value(delimiter)?,
                self.value(maximum)?
            )),
            Expr::Join(values, delimiter) => Ok(format!(
                "({}).join(&{})",
                self.read_value(values)?,
                self.read_value(delimiter)?
            )),
            Expr::ParseDecimal(value) => self.parse_decimal(value, None),
            Expr::ParseDecimalAs(target, value) => self.parse_decimal(value, Some(target)),
            Expr::FormatDecimal(value) => {
                Ok(format!("alloc::format!(\"{{}}\", {})", self.value(value)?))
            }
            Expr::Quotient(left, right, zero) => Ok(format!(
                "if {} == 0 {{ {} }} else {{ {} / {} }}",
                self.value(right)?,
                self.value(zero)?,
                self.value(left)?,
                self.value(right)?
            )),
            Expr::Remainder(left, right, zero) => Ok(format!(
                "if {} == 0 {{ {} }} else {{ {} % {} }}",
                self.value(right)?,
                self.value(zero)?,
                self.value(left)?,
                self.value(right)?
            )),
            Expr::Negate(value) => Ok(format!("-({})", self.value(value)?)),
            Expr::Sub(a, b) => {
                // Lean Nat subtraction truncates at zero, so it is total.
                // See `checked_binop` for the exact Nat receiver type.
                Ok(format!(
                    "core::convert::identity::<u64>({}).saturating_sub({})",
                    self.value(a)?,
                    self.value(b)?
                ))
            }
            Expr::Div(a, b) => self.total_binop(a, b, "/"),
            Expr::Mod(a, b) => self.total_binop(a, b, "%"),
            Expr::Shl(a, b) => self.checked_exponent_op(
                a,
                b,
                "checked_shl",
                "ShiftExponentTooLarge",
                "ShiftOverflow",
            ),
            // Unlike `Shl`, `Nat.shiftRight` is total and infallible: Lean's
            // `Nat` is unbounded, so `a >>> b = 0` for any `b >= 64` once `a`
            // fits `u64`. `checked_shr` already returns `None` exactly there
            // (and for `b >= 2^32`, via the `try_from` fallback to
            // `u32::MAX`), so `unwrap_or(0)` is the exact answer, not a
            // fallback for a real error — there is no `ComputeError` variant
            // for this because none is needed.
            Expr::Shr(a, b) => Ok(format!(
                "core::convert::identity::<u64>({}).checked_shr(u32::try_from(core::convert::identity::<u64>({})).unwrap_or(u32::MAX)).unwrap_or(0)",
                self.value(a)?,
                self.value(b)?
            )),
            Expr::Pow(a, b) => {
                self.checked_exponent_op(a, b, "checked_pow", "PowExponentTooLarge", "PowOverflow")
            }
            Expr::Eq(a, b) => match (
                self.resolved_inline(a.as_ref()),
                self.resolved_inline(b.as_ref()),
            ) {
                (other, Expr::String(value)) | (Expr::String(value), other) => {
                    Ok(format!("{} == {value:?}", self.read_value(other)?))
                }
                (Expr::Bytes(left), Expr::Bytes(right)) => Ok(format!("{}", left == right)),
                (other, Expr::Bytes(value)) | (Expr::Bytes(value), other) => {
                    Ok(format!("core::convert::AsRef::<[u8]>::as_ref(&({})) == &{value:?}", self.read_value(other)?))
                }
                _ => {
                    let borrowed_a = self.borrows(a);
                    let borrowed_b = self.borrows(b);
                    let a = self.read_value(a)?;
                    let b = self.read_value(b)?;
                    // Equality borrows its operands; normalize mixed owned /
                    // borrowed values without cloning either collection.
                    match (borrowed_a, borrowed_b) {
                        (false, true) => Ok(format!("(&({a}) == {b})")),
                        (true, false) => Ok(format!("({a} == &({b}))")),
                        _ => Ok(format!("({a} == {b})")),
                    }
                }
            },
            Expr::Lt(a, b) => self.binop(a, b, "<"),
            Expr::Le(a, b) => self.binop(a, b, "<="),
            Expr::Gt(a, b) => self.binop(a, b, ">"),
            Expr::Ctor(name, args) => {
                let args = args.iter().map(|argument| self.owned_value(argument))
                    .collect::<Result<Vec<_>, _>>()?;
                if name == "Prod.mk" {
                    Ok(format!("({})", args.join(", ")))
                } else if name == "Bool.true" && args.is_empty() {
                    Ok(String::from("true"))
                } else if name == "Bool.false" && args.is_empty() {
                    Ok(String::from("false"))
                } else if name == "Option.none" && args.is_empty() {
                    Ok(String::from("None"))
                } else if name == "Option.some" && args.len() == 1 {
                    Ok(format!("Some({})", args[0]))
                } else if name == "Except.ok" && args.len() == 1 {
                    Ok(format!("Ok({})", args[0]))
                } else if name == "Except.error" && args.len() == 1 {
                    Ok(format!("Err({})", args[0]))
                } else if let Some((decl, cdecl)) = self.ctor_decl(name) {
                    if args.len() != cdecl.fields.len() {
                        return Err(Error::UnsupportedFieldType(format!(
                            "`{}` takes {} field(s) but got {} argument(s)",
                            name,
                            cdecl.fields.len(),
                            args.len()
                        )));
                    }
                    let path = if decl.ctors.len() == 1 {
                        format!("crate::{}", rust_ident(last_component(&decl.name)))
                    } else {
                        format!(
                            "crate::{}::{}",
                            rust_ident(last_component(&decl.name)),
                            rust_ident(last_component(&cdecl.name))
                        )
                    };
                    if cdecl.fields.is_empty() && decl.ctors.len() != 1 {
                        Ok(path)
                    } else {
                        let mut bound = Vec::with_capacity(args.len());
                        for ((field, _), arg) in cdecl.fields.iter().zip(args.iter()) {
                            bound.push(format!("{}: {}", rust_ident(field), arg));
                        }
                        Ok(format!("{} {{ {} }}", path, bound.join(", ")))
                    }
                } else if name.contains('.') {
                    // No declaration for this constructor, and its Lean name
                    // is dotted. The tuple-style fallthrough below would emit
                    // the dots verbatim — `Conformance.NoProp.mk(n, n)` — and
                    // that is not a Rust path in expression position; it
                    // parses as field access on a value named `Conformance`,
                    // so even `syn::parse_str` waves it through and the
                    // failure surfaces as a rustc error about the generated
                    // file. Refuse it here, naming the constructor. The
                    // bare-name form below stays: a dot-free ctor is at least
                    // a syntactically valid path to a type the host may
                    // supply by hand.
                    Err(Error::UnresolvedCall(name.clone()))
                } else if args.is_empty() {
                    Ok(name.clone())
                } else {
                    Ok(format!("{}({})", name, args.join(", ")))
                }
            }
            Expr::Proj(ty, field, e) => {
                let field_type = self.projection_field_type(ty, field);
                if self.types.contains_key(ty.as_str()) && field_type.is_none() {
                    return Err(Error::UnknownField(ty.clone(), field.clone()));
                }
                let projection = format!("({}).{}", self.read_value(e)?, rust_ident(field));
                if !projection_moves(expr, &self.borrowed_locals, &self.movable_projections) && field_type.is_some_and(|field_type| {
                    !copy_type(field_type, self.types, &mut BTreeSet::new())
                }) {
                    Ok(format!("&{projection}"))
                } else {
                    Ok(projection)
                }
            }
            Expr::Jp { name, body, .. } => {
                if self.ctx.jmp_count(name) == 0 {
                    // No jump sites: the declaration is just a block.
                    Ok(format!(
                        "{{ /* jp \"{}\": no jump sites */ {} }}",
                        name,
                        self.value(body)?
                    ))
                } else if self.ctx.is_inlineable(name) {
                    // Inlined at its single jump site; nothing to emit here.
                    Ok(format!("/* jp \"{}\" inlined at its jump site */ ()", name))
                } else {
                    // Cyclic. This used to emit a `loop {}`
                    // skeleton with a "manual port required" comment, which is
                    // not Rust that compiles: the join point's parameters are
                    // never bound, and each jump site has type `()` where the
                    // arm needs a value. Emitting it at exit 0 is exactly the
                    // silently-broken-output failure this crate rejects
                    // everywhere else, so it is a rejection now.
                    Err(Error::UnsupportedJoinPoint(name.clone()))
                }
            }
            Expr::Jmp(name, args) => match self.ctx.decls.get(name.as_str()) {
                Some((jp_params, body)) if self.ctx.is_inlineable(name) => {
                    let mut out = String::from("{ ");
                    for (p, a) in jp_params.iter().zip(args.iter()) {
                        out.push_str(&format!(
                            "let {} = {}; ",
                            rust_local_ident(p),
                            self.value(a)?
                        ));
                    }
                    out.push_str(&self.value(body)?);
                    out.push_str(" }");
                    Ok(out)
                }
                // The declaration site rejects this too; rejecting here as
                // well means the error names the jump the reader can see,
                // whichever of the two codegen reaches first.
                Some(_) => Err(Error::UnsupportedJoinPoint(name.clone())),
                None => Ok(format!(
                    "/* jmp \"{}\": no matching jp declaration */ ()",
                    name
                )),
            },
            Expr::Unreachable => Ok(String::from("unreachable!()")),
            Expr::Opaque(s) => Err(Error::OpaqueExpr(s.clone())),
            // Handled by `render` before it delegates here.
            Expr::If(..)
            | Expr::Let(..)
            | Expr::Match { .. }
            | Expr::Var(_)
            | Expr::Call(..)
            | Expr::Extern(..) => {
                unreachable!("control-flow nodes are rendered by `render`")
            }
        }
    }

    /// `List.cons head tail` in builder position: take one element off the
    /// front of the buffer, write the head, and recurse the tail into what is
    /// left. `split_first_mut` makes exhaustion an `Err` rather than an index
    /// panic, so the generated code has no bounds-check panic path at all.
    fn render_cons(
        &self,
        head: &'m Expr,
        tail: &'m Expr,
        out: &str,
        env: &[(&'m str, &'m Expr)],
        depth: usize,
    ) -> Result<String, Error> {
        let head = self.value(head)?;
        let (slot, rest_buf) = (format!("__head{}", depth), format!("__rest{}", depth));
        let rest = self.render(
            tail,
            &Mode::Builder {
                out: &rest_buf,
                env,
                depth: depth + 1,
            },
        )?;
        Ok(format!(
            "match ({}).split_first_mut() {{ None => Err(crate::ComputeError::OutputTooSmall), Some(({}, {})) => {{ *{} = {}; let __len{} = {}?; Ok(__len{} + 1) }} }}",
            out, slot, rest_buf, slot, head, depth, rest, depth
        ))
    }

    fn render_match(
        &self,
        scrut: &'m Expr,
        alts: &'m [Alt],
        default: Option<&'m Expr>,
        mode: &Mode<'_, 'm>,
    ) -> Result<String, Error> {
        let head_rebound_by_value = self.list_head_rebound_by_value(scrut);
        let scrut_is_borrowed = self.borrows(scrut);
        let branch_borrows = alts
            .iter()
            .map(|alt| &alt.body)
            .chain(default)
            .map(|body| self.borrows(body))
            .collect::<Vec<_>>();
        let normalize_results = matches!(mode, Mode::Value)
            && branch_borrows.iter().any(|borrowed| *borrowed)
            && branch_borrows.iter().any(|borrowed| !borrowed);
        let scrut = self.value(scrut)?;
        let mut out = format!("match {} {{\n", scrut);
        for alt in alts {
            let body = if normalize_results {
                self.owned_value(&alt.body)?
            } else {
                self.render(&alt.body, mode)?
            };
            let arm = match (alt.ctor.as_str(), alt.binders.len()) {
                // LCNF structural recursion on Nat cases: `Nat.zero` is the
                // literal `0`; `Nat.succ k` binds the predecessor. Since the
                // zero arm matches first, the succ arm's scrutinee is ≥ 1 and
                // `saturating_sub(1)` is the exact predecessor (and stays
                // within the crate's bounded-Nat policy).
                ("Nat.zero", 0) => format!("        0 => {},\n", body),
                ("Nat.succ", 1) => format!(
                    "        _ => {{ let {} = ({}).saturating_sub(1); {} }},\n",
                    rust_local_ident(&alt.binders[0]),
                    scrut,
                    body
                ),
                // Lists are slices: the empty and non-empty slice patterns are
                // exhaustive, and the tail binds as a sub-slice at no cost.
                // Match ergonomics bind the head by reference; rebind it by
                // value so arithmetic on it needs no dereference syntax.
                ("List.nil", 0) => format!("        [] => {},\n", body),
                ("List.cons", 2) if head_rebound_by_value => format!(
                    "        [{}, {} @ ..] => {{ let {} = {}.clone(); {} }},\n",
                    rust_local_ident(&alt.binders[0]),
                    rust_local_ident(&alt.binders[1]),
                    rust_local_ident(&alt.binders[0]),
                    rust_local_ident(&alt.binders[0]),
                    body
                ),
                ("List.cons", 2) => format!(
                    "        [{}, {} @ ..] => {},\n",
                    rust_local_ident(&alt.binders[0]),
                    rust_local_ident(&alt.binders[1]),
                    body
                ),
                ("Bool.true", 0) => format!("        true => {},\n", body),
                ("Bool.false", 0) => format!("        false => {},\n", body),
                ("Option.none", 0) => format!("        None => {},\n", body),
                ("Option.some" | "Except.ok" | "Except.error", 1) => {
                    let constructor = match alt.ctor.as_str() {
                        "Option.some" => "Some",
                        "Except.ok" => "Ok",
                        _ => "Err",
                    };
                    let binder = rust_local_ident(&alt.binders[0]);
                    if self.copied_patterns.contains(&alt.binders[0]) {
                        format!(
                            "        {constructor}({binder}) => {{ let {binder} = *{binder}; {body} }},\n"
                        )
                    } else {
                        format!("        {constructor}({binder}) => {body},\n")
                    }
                }
                _ => match self.ctor_decl(&alt.ctor) {
                    Some((decl, cdecl)) if alt.binders.len() == cdecl.fields.len() => {
                        let path = if decl.ctors.len() == 1 {
                            format!("crate::{}", rust_ident(last_component(&decl.name)))
                        } else {
                            format!(
                                "crate::{}::{}",
                                rust_ident(last_component(&decl.name)),
                                rust_ident(last_component(&cdecl.name))
                            )
                        };
                        if cdecl.fields.is_empty() && decl.ctors.len() != 1 {
                            format!("        {} => {},\n", path, body)
                        } else {
                            let mut bound = Vec::with_capacity(alt.binders.len());
                            let mut copies = String::new();
                            for ((field, ty), binder) in cdecl.fields.iter().zip(alt.binders.iter())
                            {
                                bound.push(format!(
                                    "{}: {}",
                                    rust_ident(field),
                                    rust_local_ident(binder)
                                ));
                                if scrut_is_borrowed
                                    && copy_type(ty, self.types, &mut BTreeSet::new())
                                {
                                    let binder = rust_local_ident(binder);
                                    copies.push_str(&format!("let {binder} = {binder}.clone(); "));
                                }
                            }
                            if copies.is_empty() {
                                format!(
                                    "        {} {{ {} }} => {},\n",
                                    path,
                                    bound.join(", "),
                                    body
                                )
                            } else {
                                format!(
                                    "        {} {{ {} }} => {{ {}{} }},\n",
                                    path,
                                    bound.join(", "),
                                    copies,
                                    body
                                )
                            }
                        }
                    }
                    // Declared, but the alt's binder count does not match the
                    // constructor's field count: this must be rejected, not
                    // rendered. Falling through to the positional arms below
                    // would emit a dotted name used as a Rust path with
                    // positional fields — e.g. `M.Shape.circle(r, extra)` —
                    // which does not compile. Symmetric with the arity check
                    // on the construction side.
                    Some((_, cdecl)) => {
                        return Err(Error::UnsupportedFieldType(format!(
                            "`{}` takes {} field(s) but got {} binder(s)",
                            alt.ctor,
                            cdecl.fields.len(),
                            alt.binders.len()
                        )));
                    }
                    None if alt.binders.is_empty() => {
                        format!("        {} => {},\n", alt.ctor, body)
                    }
                    None => format!(
                        "        {}({}) => {},\n",
                        alt.ctor,
                        alt.binders
                            .iter()
                            .map(|binder| rust_local_ident(binder))
                            .collect::<Vec<_>>()
                            .join(", "),
                        body
                    ),
                },
            };
            out.push_str(&arm);
        }
        if let Some(d) = default {
            let body = if normalize_results {
                self.owned_value(d)?
            } else {
                self.render(d, mode)?
            };
            out.push_str(&format!("        _ => {},\n", body));
        }
        out.push_str("    }");
        Ok(out)
    }

    /// Flatten a constant `List.cons`/`List.nil` chain into array elements for
    /// a promoted `&'static [T]`. Only `let`-bound list values are followed;
    /// anything computed belongs in builder mode instead.
    fn static_list(
        &self,
        expr: &'m Expr,
        env: &[(&'m str, &'m Expr)],
        items: &mut Vec<String>,
    ) -> Result<(), Error> {
        if let Expr::Let(name, value, _) = expr {
            self.reject_eager_list_binding(name, value, env)?;
        }
        match expr {
            Expr::Var(name) => match lookup(env, name) {
                Some(bound) => self.static_list(bound, env, items),
                None => Err(Error::UnsupportedList(format!(
                    "`{}` is not a constant list",
                    name
                ))),
            },
            Expr::Let(name, val, body) if self.is_list_valued(val, env) => {
                let mut extended = env.to_vec();
                extended.push((name.as_str(), val));
                self.static_list(body, &extended, items)
            }
            Expr::Ctor(name, args) if name == "List.nil" && args.is_empty() => Ok(()),
            Expr::Ctor(name, args) if name == "List.cons" && args.len() == 2 => {
                items.push(self.value(&args[0])?);
                self.static_list(&args[1], env, items)
            }
            _ => Err(Error::UnsupportedList(
                "zero-argument list definitions must be constant cons chains".to_string(),
            )),
        }
    }

    fn binop(&self, a: &'m Expr, b: &'m Expr, op: &str) -> Result<String, Error> {
        Ok(format!("({} {} {})", self.value(a)?, op, self.value(b)?))
    }

    /// `checked_add`/`checked_mul`: report overflow instead of panicking.
    ///
    /// Exact Nat typing propagates to let-bound literals. A cast leaves their
    /// original type unconstrained (default i32), rejecting large valid Nats;
    /// identity also refuses narrowing from an incompatible operand type.
    fn checked_binop(
        &self,
        a: &'m Expr,
        b: &'m Expr,
        method: &str,
        error: &str,
    ) -> Result<String, Error> {
        Ok(format!(
            "core::convert::identity::<u64>({}).{}({}).ok_or(crate::ComputeError::{})?",
            self.value(a)?,
            method,
            self.value(b)?,
            error
        ))
    }

    /// `checked_shl`/`checked_pow`: the exponent must also narrow to `u32`,
    /// which is a second, distinct failure mode.
    fn checked_exponent_op(
        &self,
        a: &'m Expr,
        b: &'m Expr,
        method: &str,
        exponent_error: &str,
        overflow_error: &str,
    ) -> Result<String, Error> {
        Ok(format!(
            "core::convert::identity::<u64>({}).{}(u32::try_from(core::convert::identity::<u64>({})).map_err(|_| crate::ComputeError::{})?).ok_or(crate::ComputeError::{})?",
            self.value(a)?,
            method,
            self.value(b)?,
            exponent_error,
            overflow_error
        ))
    }

    /// Lean Nat's total operations satisfy `x / 0 = 0` and `x % 0 = x`.
    /// Tuple evaluation is once, left-to-right; match bindings are not in
    /// scope in either operand, even when an operand uses the same name.
    fn total_binop(&self, a: &'m Expr, b: &'m Expr, op: &str) -> Result<String, Error> {
        let (a, b) = (self.value(a)?, self.value(b)?);
        let zero = if op == "%" { "__left" } else { "0" };
        Ok(format!(
            "match (core::convert::identity::<u64>({a}), core::convert::identity::<u64>({b})) {{ (__left, 0) => {zero}, (__left, __right) => __left {op} __right }}"
        ))
    }
}

/// Innermost-first lookup in a builder-mode list environment.
fn lookup<'m>(env: &[(&'m str, &'m Expr)], name: &str) -> Option<&'m Expr> {
    env.iter()
        .rev()
        .find(|(bound, _)| *bound == name)
        .map(|(_, value)| *value)
}

#[cfg(test)]
mod tests;
